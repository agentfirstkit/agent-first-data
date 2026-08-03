use serde_json::Value;
use std::collections::HashSet;

// ═══════════════════════════════════════════
// Public API: Output Formatters
// ═══════════════════════════════════════════

/// Which fields a [`Redactor`] scrubs. The default is [`RedactionPolicy::All`].
///
/// The policy selects a *scope* inside a structured value. A command line
/// ([`Redactor::argv`]) and a bare URL string ([`Redactor::url`]) have no
/// `result`/`trace` split to scope to, so on those two paths `TraceOnly`
/// redacts in full like `All`, and only [`RedactionPolicy::Off`] turns
/// redaction off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RedactionPolicy {
    /// Redact every secret field anywhere in the value (the default).
    #[default]
    All,
    /// Redact only inside the top-level `trace` object.
    TraceOnly,
    /// Do not redact anything.
    Off,
}

impl RedactionPolicy {
    /// Whether this policy redacts an input that carries no scope of its own —
    /// a command line or a single URL string, as opposed to a JSON value with a
    /// `trace` object to narrow to.
    ///
    /// `TraceOnly` narrows *where* redaction applies within a value; it does not
    /// weaken redaction. A standalone argv or URL has no non-`trace` half to
    /// leave alone — and both are diagnostic material by construction, the very
    /// thing `TraceOnly` scrubs — so `TraceOnly` redacts them in full, exactly
    /// like `All`. Only `Off`, the caller explicitly asking for raw output,
    /// disables redaction, and it does so on all three paths alike.
    fn redacts_unscoped_input(self) -> bool {
        !matches!(self, RedactionPolicy::Off)
    }
}

/// Rendering style for plain (logfmt) output only. JSON and YAML are always
/// structure-preserving and ignore this.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlainStyle {
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
    policy: RedactionPolicy,
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
        self.policy = policy;
        self
    }

    /// Redact a JSON value copy using this redactor's policy and secret names.
    ///
    /// Clones `value` first; for a large payload you already own and can
    /// mutate, prefer [`Redactor::redact_in_place`] to avoid the copy.
    pub fn value(&self, value: &Value) -> Value {
        let mut v = value.clone();
        self.redact_in_place(&mut v);
        v
    }

    /// Redact secret components of a URL string using this redactor's settings.
    ///
    /// A query parameter is redacted iff its (form-decoded) name ends in
    /// `_secret`/`_SECRET` or matches an exact entry in `secret_names`. A
    /// fragment written in the same `k=v&k=v` shape — how an OAuth implicit-flow
    /// response carries its token — is redacted by that same rule; any other
    /// fragment passes through byte-for-byte. The userinfo password
    /// (`scheme://user:pass@host`) is always redacted as a structural rule.
    /// Only the secret spans are replaced with `***`; every other byte is
    /// preserved. A string that is not a single, whitespace-free,
    /// scheme-prefixed URL (including a URL embedded in surrounding prose) is
    /// returned unchanged.
    ///
    /// A URL is unscoped input: `RedactionPolicy::Off` returns it unchanged,
    /// every other policy redacts it in full.
    pub fn url(&self, url: &str) -> String {
        if !self.policy.redacts_unscoped_input() {
            return url.to_string();
        }
        let context = RedactionContext::from_redactor(self);
        redact_url_in_str(url, &context).unwrap_or_else(|| url.to_string())
    }

    /// Redact `value` in place, using this redactor's policy and secret names.
    ///
    /// The zero-copy counterpart of [`Redactor::value`] — use it on a large
    /// payload you already own to avoid cloning.
    pub fn redact_in_place(&self, value: &mut Value) {
        let context = RedactionContext::from_redactor(self);
        apply_redaction_policy_with_context(value, self.policy, &context);
    }

    /// Redact secret *values* out of a command line, using this redactor's
    /// policy and secret names.
    ///
    /// A long flag whose name is secret by AFDATA naming (`--api-key-secret`,
    /// or an exact `secret_names` entry) has its value replaced by `***`, in
    /// both `--flag=value` and `--flag value` spellings. Everything else is
    /// preserved byte-for-byte.
    ///
    /// Free text is deliberately never scanned: a bare `api_key_secret=sk-live`
    /// positional, or a secret-looking token after a non-secret flag, is left
    /// alone. AFDATA decides sensitivity from the *field name*, and argv is no
    /// exception — rename the flag rather than pattern-matching values. A flag
    /// with no value (end of argv, or followed by another flag) is likewise
    /// left inspectable.
    ///
    /// Only long (`--`) flags are recognized, matching the convention's
    /// long-flags-only rule.
    ///
    /// A command line is unscoped input: `RedactionPolicy::Off` returns `args`
    /// unchanged, every other policy redacts in full.
    pub fn argv<S: AsRef<str>>(&self, args: &[S]) -> Vec<String> {
        if !self.policy.redacts_unscoped_input() {
            return args.iter().map(|arg| arg.as_ref().to_string()).collect();
        }
        let context = RedactionContext::from_redactor(self);
        let mut out = Vec::with_capacity(args.len());
        let mut redact_next = false;
        for arg in args {
            let arg = arg.as_ref();
            if redact_next {
                redact_next = false;
                if !arg.starts_with('-') {
                    out.push(REDACTED_MARKER.to_string());
                    continue;
                }
            }
            if let Some(rest) = arg.strip_prefix("--") {
                if let Some((name, _)) = rest.split_once('=') {
                    if is_secret_flag_name(name, &context) {
                        out.push(format!("--{name}={REDACTED_MARKER}"));
                        continue;
                    }
                } else if is_secret_flag_name(rest, &context) {
                    redact_next = true;
                }
            }
            out.push(arg.to_string());
        }
        out
    }

    /// True when `name` would be treated as a secret field name by this
    /// redactor: an exact `_secret`/`_SECRET` suffix, or an exact match
    /// against a configured `secret_names` entry.
    ///
    /// Exposed for callers that must gate on a single *targeted* field name
    /// (for example a CLI dot-path leaf) rather than redact a whole value —
    /// [`Redactor::value`] only rewrites fields it finds while walking an
    /// object, so a bare scalar pulled out from under its field name needs
    /// this explicit check instead.
    pub fn is_secret_name(&self, name: &str) -> bool {
        RedactionContext::from_redactor(self).is_secret_key(name)
    }
}

impl From<RedactionPolicy> for Redactor {
    fn from(policy: RedactionPolicy) -> Self {
        Self {
            policy,
            secret_names: Vec::new(),
        }
    }
}

/// Output options combining redaction and rendering style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputOptions {
    /// Redactor applied before rendering.
    pub redaction: Redactor,
    /// Rendering style for plain output only.
    pub style: PlainStyle,
}

impl From<RedactionPolicy> for OutputOptions {
    fn from(policy: RedactionPolicy) -> Self {
        Self {
            redaction: Redactor::from(policy),
            style: PlainStyle::default(),
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

/// Redact secret values out of a command line, using default options.
///
/// Returns `args` with the value of every `_secret`-suffixed long flag replaced
/// by `***`, covering both `--flag=value` and `--flag value`. Use
/// [`Redactor::argv`] for custom `secret_names` or a non-default policy.
///
/// Intended for CLIs that record their own invocation — startup diagnostics,
/// audit trails, crash reports — where writing argv verbatim would put a
/// credential in the log.
pub fn redact_argv<S: AsRef<str>>(args: &[S]) -> Vec<String> {
    Redactor::new().argv(args)
}

/// Redact secret components of a single URL string, using default options.
///
/// Returns `url` with its userinfo password and any `_secret`-suffixed query
/// parameter values replaced by `***`.
pub fn redact_url_secrets(url: &str) -> String {
    Redactor::new().url(url)
}

// ═══════════════════════════════════════════
// Secret Redaction
// ═══════════════════════════════════════════

/// The scalar every redacted span, value, and subtree is replaced with. Also
/// the signal plain rendering reads to tell a hidden field from a live one.
pub(crate) const REDACTED_MARKER: &str = "***";

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

/// Whether a long flag's name is secret, normalizing the kebab-case flag
/// spelling to the snake_case field spelling the convention is defined in.
///
/// Ungated: core argv redaction relies on it.
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
                    // A null secret is an *absent* secret. Masking it would
                    // manufacture the appearance of a configured credential:
                    // readers cannot tell `"***"`-because-set from
                    // `"***"`-because-null, so a tool showing its own config
                    // would report every unset secret as configured. Redaction
                    // hides a value that exists; it does not invent one.
                    if !map.get(&key).is_some_and(Value::is_null) {
                        map.insert(key, Value::String(REDACTED_MARKER.into()));
                    }
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

    // `remainder` is `path[?query][#fragment]`; '#' ends the query, so split the
    // fragment off first and the query out of what is left.
    let (before_fragment, fragment) = match remainder.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (remainder, None),
    };
    let new_before_fragment = match before_fragment.split_once('?') {
        Some((path, query)) => format!("{path}?{}", redact_query(query, context)),
        None => before_fragment.to_string(),
    };
    // A fragment gets the same treatment as the query: `k=v&k=v` after the '#'
    // is exactly how an OAuth implicit-flow response hands back a token, so a
    // secret-named fragment parameter must not survive where the identically
    // named query parameter would not. A fragment that is not in that shape has
    // no '=' in its segments and passes through byte-for-byte.
    let new_fragment = match fragment {
        Some(fragment) => format!("#{}", redact_query(fragment, context)),
        None => String::new(),
    };

    Some(format!(
        "{scheme}://{new_authority}{new_before_fragment}{new_fragment}"
    ))
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
                format!("{raw_key}={REDACTED_MARKER}")
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
    redaction_policy: RedactionPolicy,
    context: &RedactionContext,
) {
    match redaction_policy {
        RedactionPolicy::All => redact_secrets_with_context(value, context),
        RedactionPolicy::TraceOnly => {
            if let Value::Object(map) = value
                && let Some(trace) = map.get_mut("trace")
            {
                redact_secrets_with_context(trace, context);
            }
        }
        RedactionPolicy::Off => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── URL fragments carry secrets too ──────────

    #[test]
    fn url_fragment_params_redacted_like_query_params() {
        assert_eq!(
            redact_url_secrets("https://h/p?token_secret=QUERYLEAK#token_secret=FRAGLEAK"),
            "https://h/p?token_secret=***#token_secret=***"
        );
    }

    #[test]
    fn url_fragment_params_redacted_without_a_query() {
        // The OAuth implicit-flow shape: the credential exists only after '#'.
        assert_eq!(
            redact_url_secrets("https://h/cb#access_token_secret=abc&state=xyz"),
            "https://h/cb#access_token_secret=***&state=xyz"
        );
    }

    #[test]
    fn url_fragment_without_params_is_preserved() {
        for url in [
            "https://h/p?a=1#section",
            "https://h/p#",
            "https://h/p#a/b?c",
        ] {
            assert_eq!(redact_url_secrets(url), url);
        }
    }

    #[test]
    fn url_fragment_honors_secret_names() {
        let redactor = Redactor::new().secret_names(vec!["token".to_string()]);
        assert_eq!(
            redactor.url("https://h/cb#token=abc&page=2"),
            "https://h/cb#token=***&page=2"
        );
    }

    // ── One policy meaning across value, argv, url ──────────

    fn scoped_value() -> Value {
        json!({
            "result": {"api_key_secret": "sk-result"},
            "trace": {"api_key_secret": "sk-trace"}
        })
    }

    fn argv() -> Vec<String> {
        vec!["tool".to_string(), "--api-key-secret=sk-live".to_string()]
    }

    #[test]
    fn all_policy_redacts_every_path() {
        let redactor = Redactor::new().policy(RedactionPolicy::All);
        assert_eq!(
            redactor.value(&scoped_value()),
            json!({
                "result": {"api_key_secret": "***"},
                "trace": {"api_key_secret": "***"}
            })
        );
        assert_eq!(redactor.argv(&argv()), vec!["tool", "--api-key-secret=***"]);
        assert_eq!(
            redactor.url("https://u:pw@h/cb?token_secret=abc"),
            "https://u:***@h/cb?token_secret=***"
        );
    }

    #[test]
    fn trace_only_scopes_a_value_but_redacts_argv_and_url_in_full() {
        let redactor = Redactor::new().policy(RedactionPolicy::TraceOnly);
        // Scoped input: only the `trace` half is scrubbed.
        assert_eq!(
            redactor.value(&scoped_value()),
            json!({
                "result": {"api_key_secret": "sk-result"},
                "trace": {"api_key_secret": "***"}
            })
        );
        // Unscoped input: a command line and a bare URL have no non-`trace`
        // half to leave alone, so they are redacted like `All`.
        assert_eq!(redactor.argv(&argv()), vec!["tool", "--api-key-secret=***"]);
        assert_eq!(
            redactor.url("https://u:pw@h/cb?token_secret=abc"),
            "https://u:***@h/cb?token_secret=***"
        );
    }

    #[test]
    fn off_policy_disables_every_path() {
        let redactor = Redactor::new().policy(RedactionPolicy::Off);
        assert_eq!(redactor.value(&scoped_value()), scoped_value());
        assert_eq!(redactor.argv(&argv()), argv());
        assert_eq!(
            redactor.url("https://u:pw@h/cb?token_secret=abc"),
            "https://u:pw@h/cb?token_secret=abc"
        );
    }
}
