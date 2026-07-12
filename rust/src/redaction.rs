use serde_json::Value;
use std::collections::HashSet;

// ═══════════════════════════════════════════
// Public API: Output Formatters
// ═══════════════════════════════════════════

/// Redaction policy. Convert with `.into()` to [`Redactor`] or [`OutputOptions`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedactionPolicy {
    /// Redact only inside top-level `trace`.
    RedactionTraceOnly,
    /// Do not redact any fields.
    RedactionNone,
}

/// Rendering style for YAML and plain output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputStyle {
    /// Human-readable AFDATA rendering: strip suffixes and format values.
    #[default]
    Readable,
    /// Schema-preserving rendering: keep keys and values unchanged after redaction.
    Raw,
}

/// Configurable redaction builder for secrets and legacy field names.
///
/// `Redactor` encapsulates redaction policy and custom secret field names.
/// Build with [`Redactor::new()`], configure via builder methods, then pass to
/// redaction functions like [`redacted_value`] or [`redact_url_secrets`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Redactor {
    policy: Option<RedactionPolicy>,
    secret_names: Vec<String>,
}

impl Redactor {
    /// Create a new default redactor (full redaction, no custom secret names).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set custom field names to treat as secrets in addition to `_secret` suffixes.
    ///
    /// Matching is exact field-name equality at any nesting level. The same
    /// list also matches URL query-parameter names inside `_url` fields.
    /// Builder style: returns `self`.
    pub fn secret_names<I: IntoIterator<Item = S>, S: Into<String>>(mut self, names: I) -> Self {
        self.secret_names = names.into_iter().map(|s| s.into()).collect();
        self
    }

    /// Set the redaction policy (default: full redaction).
    /// Builder style: returns `self`.
    pub fn policy(mut self, policy: RedactionPolicy) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Redact a JSON value copy using this redactor's policy and secret names.
    pub fn value(&self, value: &Value) -> Value {
        let mut v = value.clone();
        self.redact_in_place(&mut v);
        v
    }

    /// Redact secret components of a URL string using this redactor's settings.
    ///
    /// A query parameter is redacted iff its (form-decoded) name ends in
    /// `_secret`/`_SECRET` or matches an exact entry in `secret_names`. The
    /// userinfo password (`scheme://user:pass@host`) is always redacted as a
    /// structural rule. Only the secret spans are replaced with `***`; every
    /// other byte is preserved. A string that is not a single, whitespace-free,
    /// scheme-prefixed URL (including a URL embedded in surrounding prose) is
    /// returned unchanged.
    pub fn url(&self, url: &str) -> String {
        let context = RedactionContext::from_redactor(self);
        redact_url_in_str(url, &context).unwrap_or_else(|| url.to_string())
    }

    pub(crate) fn redact_in_place(&self, value: &mut Value) {
        let context = RedactionContext::from_redactor(self);
        apply_redaction_policy_with_context(value, self.policy, &context);
    }
}

impl From<RedactionPolicy> for Redactor {
    fn from(policy: RedactionPolicy) -> Self {
        Self {
            policy: Some(policy),
            secret_names: Vec::new(),
        }
    }
}

/// Output options combining redaction and rendering style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputOptions {
    /// Redactor applied before rendering.
    pub redaction: Redactor,
    /// Rendering style for YAML and plain output.
    pub style: OutputStyle,
}

impl From<RedactionPolicy> for OutputOptions {
    fn from(policy: RedactionPolicy) -> Self {
        Self {
            redaction: Redactor::from(policy),
            style: OutputStyle::default(),
        }
    }
}

// ═══════════════════════════════════════════
// Public API: Redaction & Utility
// ═══════════════════════════════════════════

/// Return a JSON value copy with default `_secret` redaction applied.
pub fn redacted_value(value: &Value) -> Value {
    Redactor::new().value(value)
}

/// Redact secret components of a single URL string, using default options.
///
/// Returns `url` with its userinfo password and any `_secret`-suffixed query
/// parameter values replaced by `***`.
pub fn redact_url_secrets(url: &str) -> String {
    Redactor::new().url(url)
}

/// Parse a human-readable size string into bytes.
///
/// Accepts a number followed by an explicit unit. Decimal units are
/// `B`, `kB`, `MB`, `GB`, `TB`; binary units are `KiB`, `MiB`, `GiB`, `TiB`.

// ═══════════════════════════════════════════
// Secret Redaction
// ═══════════════════════════════════════════

#[derive(Default)]
pub(crate) struct RedactionContext {
    secret_names: HashSet<String>,
}

impl RedactionContext {
    fn from_redactor(redactor: &Redactor) -> Self {
        let secret_names = redactor.secret_names.iter().cloned().collect();
        Self { secret_names }
    }

    fn is_secret_key(&self, key: &str) -> bool {
        key_has_secret_suffix(key) || self.secret_names.contains(key)
    }
}

fn key_has_secret_suffix(key: &str) -> bool {
    key.ends_with("_secret") || key.ends_with("_SECRET")
}

fn key_has_url_suffix(key: &str) -> bool {
    key.ends_with("_url") || key.ends_with("_URL")
}

#[cfg(feature = "cli-help")]
pub(crate) fn is_secret_flag_name(flag_name: &str, context: &RedactionContext) -> bool {
    let normalized = flag_name.replace('-', "_");
    context.is_secret_key(&normalized) || context.is_secret_key(flag_name)
}

const MAX_DEPTH: usize = 256;
const MAX_DEPTH_MARKER: &str = "<afdata:max-depth>";

fn redact_secrets_with_context(value: &mut Value, context: &RedactionContext) {
    redact_secrets_with_context_depth(value, context, 0);
}

fn redact_secrets_with_context_depth(value: &mut Value, context: &RedactionContext, depth: usize) {
    if depth >= MAX_DEPTH {
        *value = Value::String(MAX_DEPTH_MARKER.into());
        return;
    }
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if context.is_secret_key(&key) {
                    map.insert(key, Value::String("***".into()));
                } else if key_has_url_suffix(&key) {
                    if let Some(Value::String(s)) = map.get_mut(&key) {
                        *s = redact_url_field_value(s, context);
                    } else if let Some(v) = map.get_mut(&key) {
                        redact_secrets_with_context_depth(v, context, depth + 1);
                    }
                } else if let Some(v) = map.get_mut(&key) {
                    redact_secrets_with_context_depth(v, context, depth + 1);
                }
            }
        }
        Value::Array(arr) => {
            for v in arr {
                redact_secrets_with_context_depth(v, context, depth + 1);
            }
        }
        _ => {}
    }
}

/// Redact secret components of a single URL string, returning `Some(redacted)`
/// when `s` is a processable URL, or `None` when it is not (so callers can keep
/// the original). Only secret spans change; all other bytes are preserved.
fn redact_url_in_str(s: &str, context: &RedactionContext) -> Option<String> {
    // Precondition (spec): a single, whitespace-free, scheme-prefixed URL.
    // The gate is scheme + no-whitespace only — NOT "parses as a URL library
    // object". Span location below is purely byte-wise, so we never re-serialize
    // the URL; adding a `url::Url::parse` gate here would diverge across
    // languages (e.g. ports > 65535 or empty hosts that one library rejects and
    // another accepts) and silently leak secrets in the values it rejects.
    if !s.contains("://") || !is_single_url(s) {
        return None;
    }
    let scheme_sep = s.find("://")?;
    let scheme = &s[..scheme_sep];
    let rest = &s[scheme_sep + 3..];

    // Authority runs from after "://" to the first '/', '?', or '#'.
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    let remainder = &rest[auth_end..];

    let new_authority = redact_userinfo_password(authority);

    // Query runs from the first '?' to the first '#' (or end).
    let new_remainder = match remainder.find('?') {
        Some(q) => {
            let (path, q_onwards) = remainder.split_at(q);
            let query_body = &q_onwards[1..];
            let (query, fragment) = match query_body.find('#') {
                Some(h) => (&query_body[..h], &query_body[h..]),
                None => (query_body, ""),
            };
            format!("{path}?{}{fragment}", redact_query(query, context))
        }
        None => remainder.to_string(),
    };

    Some(format!("{scheme}://{new_authority}{new_remainder}"))
}

fn redact_url_field_value(s: &str, context: &RedactionContext) -> String {
    if let Some(redacted) = redact_url_in_str(s, context) {
        return redacted;
    }
    let trimmed = s.trim();
    if trimmed != s
        && let Some(redacted) = redact_url_in_str(trimmed, context)
    {
        return redacted;
    }
    // Fail closed: a `_url` value we could not parse as a clean scheme-prefixed
    // URL, yet which carries a credential sigil (`@` userinfo) or internal
    // whitespace, is redacted wholesale rather than passed through. A schemeless
    // connection string like `user:pass@host/db` has no scheme anchor for the
    // surgical span logic above, so blanket redaction is the safe default.
    if s.chars().any(char::is_whitespace) || s.contains('@') {
        return "***".to_string();
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
        Some(colon) => format!("{}:***{}", &authority[..colon], &authority[at..]),
        None => authority.to_string(),
    }
}

/// Redact the values of secret-named query parameters, preserving raw bytes of
/// every other segment (keys, benign values, encoding, ordering, separators).
fn redact_query(query: &str, context: &RedactionContext) -> String {
    query
        .split('&')
        .map(|segment| {
            let Some(eq) = segment.find('=') else {
                return segment.to_string();
            };
            let raw_key = &segment[..eq];
            // Form-decode the name (`+` → space, percent-decode) for the check.
            let name = url::form_urlencoded::parse(segment.as_bytes())
                .next()
                .map(|(k, _)| k.into_owned())
                .unwrap_or_default();
            if context.is_secret_key(&name) {
                format!("{raw_key}=***")
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// True when `s` begins with a URL scheme (`ALPHA *(ALPHA / DIGIT / "+" / "-" /
/// ".") "://"`) and contains no ASCII whitespace — i.e. a single bare URL, not
/// a URL embedded in prose.
fn is_single_url(s: &str) -> bool {
    if s.bytes().any(|b| b.is_ascii_whitespace()) {
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

fn apply_redaction_policy_with_context(
    value: &mut Value,
    redaction_policy: Option<RedactionPolicy>,
    context: &RedactionContext,
) {
    match redaction_policy {
        Some(RedactionPolicy::RedactionTraceOnly) => {
            if let Value::Object(map) = value
                && let Some(trace) = map.get_mut("trace")
            {
                redact_secrets_with_context(trace, context);
            }
        }
        Some(RedactionPolicy::RedactionNone) => {}
        None => redact_secrets_with_context(value, context),
    }
}
