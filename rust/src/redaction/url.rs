//! URL span detection and credential redaction, preserving untouched bytes.
use super::{REDACTED_MARKER, RedactionContext};
pub(super) fn redact_url_in_str(s: &str, context: &RedactionContext) -> Option<String> {
    redact_url_in_str_depth(s, context, 0)
}

fn redact_url_in_str_depth(s: &str, context: &RedactionContext, depth: usize) -> Option<String> {
    // Precondition (spec): a single, whitespace-free, scheme-prefixed URL.
    // The gate is scheme + no-whitespace only — NOT "parses as a URL library
    // object". Span location below is purely byte-wise, so we never re-serialize
    // the URL; adding a `url::Url::parse` gate here would diverge across
    // languages (e.g. ports > 65535 or empty hosts that one library rejects and
    // another accepts) and silently leak secrets in the values it rejects.
    if !s.contains("://") || !is_single_url(s) {
        return None;
    }
    // Bound recursive redirect inspection, failing closed before stack growth.
    if depth >= 16 {
        return Some(REDACTED_MARKER.to_string());
    }
    let scheme_sep = s.find("://")?;
    let scheme = &s[..scheme_sep];
    let rest = &s[scheme_sep + 3..];

    // Authority runs from after "://" to the first '/', '?', or '#'.
    let mut auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    // An unencoded '?' or '#' inside a password ends that authority early and
    // leaves the rest of the password in the query or fragment. When a ':'
    // precedes that early end and an '@' follows it before the first '/', the
    // userinfo runs to that '@' — how libpq reads such a connection string.
    let first_slash = rest.find('/').unwrap_or(rest.len());
    if rest[..auth_end].contains(':')
        && let Some(at) = rest[..first_slash].rfind('@')
        && at > auth_end
    {
        auth_end = rest[at..]
            .find(['/', '?', '#'])
            .map_or(rest.len(), |end| at + end);
    }
    let authority = &rest[..auth_end];
    let remainder = &rest[auth_end..];

    // An unencoded '/' inside a password ends the authority at a "port" that
    // is not a number, with the '@' somewhere after it. Where the password
    // ends cannot be known, so the whole value is replaced.
    if !authority.contains('@') && has_non_numeric_port(authority) && remainder.contains('@') {
        return Some(REDACTED_MARKER.to_string());
    }

    let new_authority = redact_userinfo_password(authority);

    // `remainder` is `path[?query][#fragment]`; '#' ends the query, so split the
    // fragment off first and the query out of what is left.
    let (before_fragment, fragment) = match remainder.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (remainder, None),
    };
    let new_before_fragment = match before_fragment.split_once('?') {
        Some((path, query)) => format!("{path}?{}", redact_query(query, context, depth)),
        None => before_fragment.to_string(),
    };
    // A fragment gets the same treatment as the query: `k=v&k=v` after the '#'
    // is exactly how an OAuth implicit-flow response hands back a token, so a
    // secret-named fragment parameter must not survive where the identically
    // named query parameter would not. A fragment that is not in that shape has
    // no '=' in its segments and passes through byte-for-byte.
    // A hash route (`#/cb?k=v`) carries its parameters after its first '?',
    // while a form reader keeps that '?' inside a value; redact both readings:
    // the fragment as parameters first, so a secret value absorbs its '?',
    // then whatever follows the first '?' that survives.
    let new_fragment = match fragment {
        Some(fragment) => {
            let params = redact_query(fragment, context, depth);
            match params.split_once('?') {
                Some((route, query)) => {
                    format!("#{route}?{}", redact_query(query, context, depth))
                }
                None => format!("#{params}"),
            }
        }
        None => String::new(),
    };

    Some(format!(
        "{scheme}://{new_authority}{new_before_fragment}{new_fragment}"
    ))
}

pub(super) fn redact_urls_in_text_with_context(text: &str, context: &RedactionContext) -> String {
    let mut output = String::with_capacity(text.len());
    let mut copied_through = 0;
    let mut search_from = 0;
    let mut found = false;
    while let Some((start, end)) = next_scheme_url_span(text, search_from) {
        found = true;
        output.push_str(&text[copied_through..start]);
        let url = &text[start..end];
        output.push_str(&redact_url_in_str(url, context).unwrap_or_else(|| url.to_string()));
        copied_through = end;
        search_from = end;
    }
    if !found {
        return text.to_string();
    }
    output.push_str(&text[copied_through..]);
    output
}

fn next_scheme_url_span(text: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut start = from;
    while start < bytes.len() {
        // Only consider the head of a maximal scheme-byte run: a scheme cannot
        // begin mid-run, so a position whose predecessor is a scheme byte was
        // already covered by the run that contains it.
        if start == 0 || !is_scheme_byte(bytes[start - 1]) {
            let mut scheme_end = start;
            while scheme_end < bytes.len() && is_scheme_byte(bytes[scheme_end]) {
                scheme_end += 1;
            }
            // A scheme must start with a letter, but the run need not: prose
            // like `2https://h/?token_secret=x` glues a digit onto the URL. Retry
            // from the first letter *inside* the run instead of abandoning the
            // span — giving up there fails open and leaks the whole query.
            let scheme_start = (start..scheme_end).find(|&i| bytes[i].is_ascii_alphabetic());
            if let Some(scheme_start) = scheme_start
                && bytes
                    .get(scheme_end..scheme_end.saturating_add(3))
                    .is_some_and(|separator| separator == b"://")
            {
                let mut end = scheme_end + 3;
                while end < bytes.len() {
                    let Some(character) = text[end..].chars().next() else {
                        break;
                    };
                    if is_span_terminator_whitespace(character) || is_text_url_delimiter(character)
                    {
                        break;
                    }
                    end += character.len_utf8();
                }
                while end > scheme_end + 3 {
                    let Some(character) = text[..end].chars().next_back() else {
                        break;
                    };
                    if !is_trailing_text_url_punctuation(character) {
                        break;
                    }
                    end -= character.len_utf8();
                }
                if end > scheme_end + 3 {
                    return Some((scheme_start, end));
                }
            }
        }
        start += 1;
    }
    None
}

fn is_scheme_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')
}

/// Whitespace that ends a URL span in prose: the Unicode `White_Space`
/// property, written out here instead of delegated to `char::is_whitespace`.
///
/// The four AFDATA implementations must cut the span at the same byte, and each
/// language's native predicate covers a different set — JS `\s` counts U+FEFF
/// but not U+0085, Python's `str.isspace()` counts U+001C–U+001F, Rust and Go
/// count neither. Divergence is a defect in both directions: ending a span early
/// leaves the rest of the query readable, and ending it late pulls the following
/// prose into the URL, where redacting the parameter it lands in deletes it. So
/// the set is enumerated, and it is generous — a redaction helper exists to keep
/// a log line readable, so anything a human reads as a space must end the URL.
///
/// Two exclusions are deliberate, not oversights:
/// * **U+FEFF** (zero-width no-break space) is not `White_Space`. Treating it as
///   one would cut `https://h/x\u{feff}?token_secret=leak` short and leave the
///   secret in the clear — the exact leak JS `\s` used to cause here.
/// * **U+001C–U+001F** (the file/group/record/unit separators) are not
///   `White_Space` either; only Python's `str.isspace()` counted them.
///
/// This is a different question from [`is_single_url`], which asks whether a
/// whole string is one URL and stays ASCII-only. A span this scanner produces
/// contains no whitespace at all, so it satisfies that stricter gate either way.
fn is_span_terminator_whitespace(character: char) -> bool {
    matches!(
        character,
        // TAB, LF, VT, FF, CR, SPACE
        '\u{0009}'..='\u{000d}' | '\u{0020}'
            // NEL, NBSP, OGHAM SPACE MARK
            | '\u{0085}' | '\u{00a0}' | '\u{1680}'
            // EN QUAD .. HAIR SPACE
            | '\u{2000}'..='\u{200a}'
            // LINE SEPARATOR, PARAGRAPH SEPARATOR, NARROW NBSP,
            // MEDIUM MATHEMATICAL SPACE, IDEOGRAPHIC SPACE
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

fn is_text_url_delimiter(character: char) -> bool {
    matches!(
        character,
        '"' | '\''
            | '<'
            | '>'
            | '`'
            | '，'
            | '。'
            | '；'
            | '！'
            | '？'
            | '）'
            | '》'
            | '】'
            | '」'
            | '』'
    )
}

fn is_trailing_text_url_punctuation(character: char) -> bool {
    matches!(
        character,
        '.' | ',' | ';' | '!' | ')' | ']' | '}' | '，' | '。' | '；' | '！' | '）' | '》' | '】'
    )
}

pub(super) fn redact_url_field_value(s: &str, context: &RedactionContext) -> String {
    if let Some(redacted) = redact_url_in_str(s, context) {
        return redacted;
    }
    let trimmed = s.trim_matches(is_span_terminator_whitespace);
    if trimmed != s
        && let Some(redacted) = redact_url_in_str(trimmed, context)
    {
        return redacted;
    }
    // Fail closed: a `_url` value we could not parse as a clean scheme-prefixed
    // URL, yet which carries a credential sigil (`@` userinfo), internal
    // whitespace, or a `://` scheme separator, is redacted wholesale rather than
    // passed through. A schemeless connection string like `user:pass@host/db`
    // has no scheme anchor for the surgical span logic above, and a URL behind
    // a non-whitespace prefix (a BOM, a zero-width space, a label) has one the
    // span logic cannot reach, so blanket redaction is the safe default.
    if s.chars().any(is_span_terminator_whitespace) || s.contains('@') || s.contains("://") {
        return REDACTED_MARKER.to_string();
    }
    s.to_string()
}

/// Replace the userinfo password (`user:pass@`) with `***`, preserving the
/// username. Authority without `@`, or userinfo without `:`, is unchanged.
fn redact_userinfo_password(authority: &str) -> String {
    let Some(at) = authority.rfind('@') else {
        return authority.to_string();
    };
    let userinfo = &authority[..at];
    match userinfo.find(':') {
        Some(colon) => format!(
            "{}:{REDACTED_MARKER}{}",
            &authority[..colon],
            &authority[at..]
        ),
        None => authority.to_string(),
    }
}

/// True when the authority's text after its last ':' is not a port number.
/// A bracketed IPv6 literal without a port ends in ']' and has none.
fn has_non_numeric_port(authority: &str) -> bool {
    match authority.rfind(':') {
        Some(colon) if !authority.ends_with(']') => {
            let port = &authority[colon + 1..];
            !port.bytes().all(|b| b.is_ascii_digit())
        }
        _ => false,
    }
}

/// Redact the values of secret-named query parameters, preserving raw bytes of
/// every other segment (keys, benign values, encoding, ordering, separators).
fn redact_query(query: &str, context: &RedactionContext, depth: usize) -> String {
    query
        .split('&')
        .map(|segment| {
            let Some(eq) = segment.find('=') else {
                return segment.to_string();
            };
            let raw_key = &segment[..eq];
            // Form-decode the name (`+` → space, percent-decode) for the check.
            let (name, value) = url::form_urlencoded::parse(segment.as_bytes())
                .next()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .unwrap_or_default();
            if context.is_secret_key(&name) {
                return format!("{raw_key}={REDACTED_MARKER}");
            }
            // A redirect target carries its own parameters: redact a nested
            // URL in place, or the whole value when it arrived encoded.
            let raw_value = &segment[eq + 1..];
            match redact_url_in_str_depth(&value, context, depth + 1) {
                Some(redacted) if redacted != value && value == raw_value => {
                    format!("{raw_key}={redacted}")
                }
                Some(redacted) if redacted != value => format!("{raw_key}={REDACTED_MARKER}"),
                _ => segment.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// True when `s` begins with a URL scheme (`ALPHA *(ALPHA / DIGIT / "+" / "-" /
/// ".") "://"`) and contains no ASCII whitespace — i.e. a single bare URL, not
/// a URL embedded in prose.
fn is_single_url(s: &str) -> bool {
    // `u8::is_ascii_whitespace` omits VT (U+000B); the gate counts it, so a
    // value split by one is not a single URL.
    if s.bytes().any(|b| b.is_ascii_whitespace() || b == 0x0b) {
        return false;
    }
    let bytes = s.as_bytes();
    if !bytes.first().is_some_and(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.') {
            i += 1;
        } else {
            break;
        }
    }
    s[i..].starts_with("://")
}
