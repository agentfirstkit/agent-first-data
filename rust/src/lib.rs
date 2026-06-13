//! Agent-First Data (AFDATA) output formatting and protocol templates.
//!
//! Public APIs include:
//! - 3 protocol builders: [`build_json_ok`], [`build_json_error`], [`build_json`]
//! - 3 value-copy redactors: [`redacted_value`], [`redacted_value_with`], [`redacted_value_with_options`]
//! - 7 output formatters: [`output_json`], [`output_json_with`], [`output_json_with_options`],
//!   [`output_yaml`], [`output_yaml_with_options`], [`output_plain`], [`output_plain_with_options`]
//! - 2 in-place value redactors: [`redact_secrets_in_place`], [`redact_secrets_in_place_with_options`]
//!   (these redact `_secret` and `_url` fields in a JSON value)
//! - 2 URL-string redactors: [`redact_url_secrets`], [`redact_url_secrets_with_options`]
//!   (operate on one URL string; the value redactors above apply these to `_url` fields)
//! - 4 parse utilities: [`parse_size`], [`normalize_utc_offset`],
//!   [`is_valid_rfc3339_date`], [`is_valid_rfc3339_time`]
//! - CLI helpers: [`cli_parse_output`], [`cli_parse_log_filters`], [`cli_output`],
//!   [`cli_output_with_options`], [`build_cli_error`], [`build_cli_version`],
//!   [`cli_render_version`], [`cli_handle_version_or_continue`]
//! - 6 types: [`OutputFormat`], [`VersionConfig`], [`RedactionPolicy`],
//!   [`RedactionOptions`], [`OutputStyle`], [`OutputOptions`]
//! - (feature `cli-help`): configurable clap help rendering via [`cli_render_help_with_options`]
//!   and [`cli_handle_help_or_continue`]
//! - (feature `cli-help-markdown`): [`cli_render_help_markdown`] — recursive Markdown help
//! - (feature `skill-admin`): [`skill::run_skill_admin`] — install/uninstall/status a spore's
//!   embedded Agent Skill across Codex, Claude Code, and opencode; returns a typed
//!   [`skill::SkillReport`]
//! - (feature `tracing`): [`afdata_tracing::try_init_json`] / `try_init_plain` /
//!   `try_init_yaml` initialize an AFDATA stdout logging layer and report initialization failures

#[cfg(feature = "tracing")]
pub mod afdata_tracing;

#[cfg(feature = "skill-admin")]
pub mod skill;

use serde_json::Value;
use std::collections::HashSet;

// ═══════════════════════════════════════════
// Public API: Protocol Builders
// ═══════════════════════════════════════════

/// Build `{code: "ok", result: ..., trace?: ...}`.
pub fn build_json_ok(result: Value, trace: Option<Value>) -> Value {
    match trace {
        Some(t) => serde_json::json!({"code": "ok", "result": result, "trace": t}),
        None => serde_json::json!({"code": "ok", "result": result}),
    }
}

/// Build `{code: "error", error: message, hint?: ..., trace?: ...}`.
pub fn build_json_error(message: &str, hint: Option<&str>, trace: Option<Value>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("code".to_string(), Value::String("error".to_string()));
    obj.insert("error".to_string(), Value::String(message.to_string()));
    if let Some(h) = hint {
        obj.insert("hint".to_string(), Value::String(h.to_string()));
    }
    if let Some(t) = trace {
        obj.insert("trace".to_string(), t);
    }
    Value::Object(obj)
}

/// Build `{code: "<custom>", ...fields, trace?: ...}`.
pub fn build_json(code: &str, fields: Value, trace: Option<Value>) -> Value {
    let mut obj = match fields {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    obj.insert("code".to_string(), Value::String(code.to_string()));
    if let Some(t) = trace {
        obj.insert("trace".to_string(), t);
    }
    Value::Object(obj)
}

// ═══════════════════════════════════════════
// Public API: Output Formatters
// ═══════════════════════════════════════════

/// Redaction policy for [`output_json_with`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedactionPolicy {
    /// Redact only inside top-level `trace`.
    RedactionTraceOnly,
    /// Do not redact any fields.
    RedactionNone,
}

/// Redaction options for legacy secret field names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RedactionOptions {
    /// Optional scoped policy. `None` means default full redaction.
    pub policy: Option<RedactionPolicy>,
    /// Field names to treat as secrets in addition to `_secret` suffixes.
    ///
    /// Matching is exact field-name equality at any nesting level. The same
    /// list also matches URL query-parameter names inside `_url` fields (see
    /// [`redact_url_secrets`]).
    pub secret_names: Vec<String>,
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

/// Output options combining redaction and rendering style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutputOptions {
    /// Redaction options applied before rendering.
    pub redaction: RedactionOptions,
    /// Rendering style for YAML and plain output.
    pub style: OutputStyle,
}

/// Format as single-line JSON with full `_secret` redaction.
pub fn output_json(value: &Value) -> String {
    serialize_json_output(&redacted_value(value))
}

/// Format as single-line JSON with configurable redaction policy.
pub fn output_json_with(value: &Value, redaction_policy: RedactionPolicy) -> String {
    serialize_json_output(&redacted_value_with(value, redaction_policy))
}

/// Format as single-line JSON with configurable output options.
///
/// JSON output ignores [`OutputStyle`] and always preserves original keys and values after
/// redaction.
pub fn output_json_with_options(value: &Value, output_options: &OutputOptions) -> String {
    serialize_json_output(&redacted_value_with_options(
        value,
        &output_options.redaction,
    ))
}

fn serialize_json_output(value: &Value) -> String {
    match serde_json::to_string(value) {
        Ok(s) => s,
        Err(err) => serde_json::json!({
            "error": "output_json_failed",
            "detail": err.to_string(),
        })
        .to_string(),
    }
}

/// Format as multi-line YAML. Keys stripped, values formatted, secrets redacted.
pub fn output_yaml(value: &Value) -> String {
    output_yaml_with_options(value, &OutputOptions::default())
}

/// Format as multi-line YAML with configurable output options.
pub fn output_yaml_with_options(value: &Value, output_options: &OutputOptions) -> String {
    let mut lines = vec!["---".to_string()];
    let v = redacted_value_with_options(value, &output_options.redaction);
    match output_options.style {
        OutputStyle::Readable => render_yaml_processed(&v, 0, &mut lines),
        OutputStyle::Raw => render_yaml_raw(&v, 0, &mut lines),
    }
    lines.join("\n")
}

/// Format as single-line logfmt. Keys stripped, values formatted, secrets redacted.
pub fn output_plain(value: &Value) -> String {
    output_plain_with_options(value, &OutputOptions::default())
}

/// Format as single-line logfmt with configurable output options.
pub fn output_plain_with_options(value: &Value, output_options: &OutputOptions) -> String {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let v = redacted_value_with_options(value, &output_options.redaction);
    match output_options.style {
        OutputStyle::Readable => collect_plain_pairs(&v, "", &mut pairs),
        OutputStyle::Raw => collect_plain_pairs_raw(&v, "", &mut pairs),
    }
    pairs.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
    pairs
        .into_iter()
        .map(|(k, v)| format!("{}={}", quote_logfmt_key(&k), quote_logfmt_value(&v)))
        .collect::<Vec<_>>()
        .join(" ")
}

// ═══════════════════════════════════════════
// Public API: Redaction & Utility
// ═══════════════════════════════════════════

/// Redact `_secret` fields in-place.
pub fn redact_secrets_in_place(value: &mut Value) {
    redact_secrets(value);
}

/// Redact secret fields in-place using configurable redaction options.
pub fn redact_secrets_in_place_with_options(
    value: &mut Value,
    redaction_options: &RedactionOptions,
) {
    apply_redaction_options(value, redaction_options);
}

/// Return a JSON value copy with default `_secret` redaction applied.
pub fn redacted_value(value: &Value) -> Value {
    let mut v = value.clone();
    redact_secrets(&mut v);
    v
}

/// Return a JSON value copy with an explicit redaction policy applied.
pub fn redacted_value_with(value: &Value, redaction_policy: RedactionPolicy) -> Value {
    let mut v = value.clone();
    apply_redaction_policy(&mut v, redaction_policy);
    v
}

/// Return a JSON value copy with configurable redaction options applied.
pub fn redacted_value_with_options(value: &Value, redaction_options: &RedactionOptions) -> Value {
    let mut v = value.clone();
    apply_redaction_options(&mut v, redaction_options);
    v
}

/// Redact secret components of a single URL string, using default options.
///
/// Returns `url` with its userinfo password and any `_secret`-suffixed query
/// parameter values replaced by `***`. See [`redact_url_secrets_with_options`].
pub fn redact_url_secrets(url: &str) -> String {
    redact_url_secrets_with_options(url, &RedactionOptions::default())
}

/// Redact secret components of a single URL string.
///
/// A query parameter is redacted iff its (form-decoded) name ends in
/// `_secret`/`_SECRET` or matches an exact entry in `secret_names`. The
/// userinfo password (`scheme://user:pass@host`) is always redacted as a
/// structural rule. Only the secret spans are replaced with `***`; every other
/// byte is preserved. A string that is not a single, whitespace-free,
/// scheme-prefixed URL (including a URL embedded in surrounding prose) is
/// returned unchanged.
pub fn redact_url_secrets_with_options(url: &str, redaction_options: &RedactionOptions) -> String {
    let context = RedactionContext::from_options(redaction_options);
    redact_url_in_str(url, &context).unwrap_or_else(|| url.to_string())
}

/// Parse a human-readable size string into bytes.
///
/// Accepts bare number, or number followed by unit letter
/// (`B`, `K`, `M`, `G`, `T`). Case-insensitive. Trims whitespace.
/// Returns `None` for invalid or negative input.
pub fn parse_size(s: &str) -> Option<u64> {
    const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let last = *s.as_bytes().last()?;
    let (num_str, mult) = match last {
        b'B' | b'b' => (&s[..s.len() - 1], 1u64),
        b'K' | b'k' => (&s[..s.len() - 1], 1024),
        b'M' | b'm' => (&s[..s.len() - 1], 1024 * 1024),
        b'G' | b'g' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        b'T' | b't' => (&s[..s.len() - 1], 1024u64 * 1024 * 1024 * 1024),
        b'0'..=b'9' | b'.' => (s, 1),
        _ => return None,
    };
    if num_str.is_empty() || !is_decimal_number(num_str) {
        return None;
    }
    if let Ok(n) = num_str.parse::<u64>() {
        let result = n.checked_mul(mult)?;
        return (result <= MAX_SAFE_INTEGER).then_some(result);
    }
    // Integer overflow must not silently fall back to float parsing.
    if !num_str.contains('.') && !num_str.contains('e') && !num_str.contains('E') {
        return None;
    }
    let f: f64 = num_str.parse().ok()?;
    if f < 0.0 || f.is_nan() || f.is_infinite() {
        return None;
    }
    let result = f * mult as f64;
    if result > MAX_SAFE_INTEGER as f64 {
        return None;
    }
    Some(result as u64)
}

fn is_decimal_number(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut digits = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
        i += 1;
        if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
            i += 1;
        }
        let exp_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return false;
        }
    }
    i == bytes.len()
}

/// Normalize a fixed UTC offset string to AFDATA canonical form.
///
/// Returns `"UTC"` for zero offset. Non-zero offsets return `+HH:MM` or
/// `-HH:MM`. This helper handles fixed offsets only; IANA timezone names and
/// DST rules are intentionally out of scope.
pub fn normalize_utc_offset(s: &str) -> Option<String> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("utc") || s.eq_ignore_ascii_case("z") {
        return Some("UTC".to_string());
    }
    let sign = match s.as_bytes().first()? {
        b'+' => '+',
        b'-' => '-',
        _ => return None,
    };
    let body = &s[1..];
    let (hours, minutes) = parse_utc_offset_body(body)?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    if hours == 0 && minutes == 0 {
        return Some("UTC".to_string());
    }
    Some(format!("{sign}{hours:02}:{minutes:02}"))
}

/// Return true when `s` is an RFC 3339 `full-date` (`YYYY-MM-DD`).
pub fn is_valid_rfc3339_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let Some(year) = parse_ascii_u16_bytes(&bytes[0..4]) else {
        return false;
    };
    let Some(month) = parse_ascii_u8_bytes(&bytes[5..7]) else {
        return false;
    };
    let Some(day) = parse_ascii_u8_bytes(&bytes[8..10]) else {
        return false;
    };
    (1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day)
}

/// Return true when `s` is an RFC 3339 `partial-time` (`HH:MM:SS[.fraction]`).
///
/// AFDATA intentionally rejects `Z`/offset suffixes here: time-only fields are
/// not instants and cannot be resolved through timezone rules without a date.
pub fn is_valid_rfc3339_time(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 8 || bytes[2] != b':' || bytes[5] != b':' {
        return false;
    }
    let Some(hour) = parse_ascii_u8_bytes(&bytes[0..2]) else {
        return false;
    };
    let Some(minute) = parse_ascii_u8_bytes(&bytes[3..5]) else {
        return false;
    };
    let Some(second) = parse_ascii_u8_bytes(&bytes[6..8]) else {
        return false;
    };
    if hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    if bytes.len() == 8 {
        return true;
    }
    bytes[8] == b'.' && bytes.len() > 9 && bytes[9..].iter().all(u8::is_ascii_digit)
}

fn parse_utc_offset_body(body: &str) -> Option<(u8, u8)> {
    if body.is_empty() {
        return None;
    }
    if let Some((hours, minutes)) = body.split_once(':') {
        if hours.is_empty() || hours.len() > 2 || minutes.len() != 2 {
            return None;
        }
        return Some((parse_ascii_u8(hours)?, parse_ascii_u8(minutes)?));
    }
    if !body.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match body.len() {
        1 | 2 => Some((parse_ascii_u8(body)?, 0)),
        4 => Some((parse_ascii_u8(&body[..2])?, parse_ascii_u8(&body[2..])?)),
        _ => None,
    }
}

fn parse_ascii_u8(s: &str) -> Option<u8> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn parse_ascii_u8_bytes(bytes: &[u8]) -> Option<u8> {
    let n = parse_ascii_u16_bytes(bytes)?;
    u8::try_from(n).ok()
}

fn parse_ascii_u16_bytes(bytes: &[u8]) -> Option<u16> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut value = 0u16;
    for byte in bytes {
        value = value.checked_mul(10)?;
        value = value.checked_add(u16::from(byte - b'0'))?;
    }
    Some(value)
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    let year = u32::from(year);
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

// ═══════════════════════════════════════════
// Public API: CLI Helpers
// ═══════════════════════════════════════════

/// Output format for CLI and pipe/MCP modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Yaml,
    Plain,
}

/// Configuration for pre-parser `--version` handling.
///
/// This helper scans raw argv before the application's argument parser so
/// `--version --output json` can return an AFDATA event instead of letting
/// clap or another parser print conventional plain text and exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionConfig {
    /// Format used for `--version` when no explicit output flag is present.
    ///
    /// `Some(format)` renders an AFDATA `{code:"version", ...}` event in that
    /// format. `None` preserves conventional CLI output: `<name> <version>`.
    pub default_output: Option<OutputFormat>,
    /// Optional long output flag to read, for example `--output`.
    pub output_flag: Option<&'static str>,
    /// Optional short output flag to read, for example `-o`.
    pub output_short: Option<char>,
    /// Whether an explicit output flag can override `default_output`.
    pub allow_output_format: bool,
}

impl VersionConfig {
    /// Construct a custom version handler configuration.
    pub const fn new(default_output: Option<OutputFormat>) -> Self {
        Self {
            default_output,
            output_flag: None,
            output_short: None,
            allow_output_format: false,
        }
    }

    /// Structured bare-version preset.
    ///
    /// A bare `--version` is a JSON AFDATA event. Most CLIs should prefer
    /// [`Self::conventional_default`] so human `--version` stays familiar while
    /// explicit `--output json|yaml|plain` remains structured.
    pub const fn agent_cli_default() -> Self {
        Self {
            default_output: Some(OutputFormat::Json),
            output_flag: Some("--output"),
            output_short: None,
            allow_output_format: true,
        }
    }

    /// Recommended preset: keep conventional bare version text while still
    /// honoring explicit `--output json|yaml|plain`.
    pub const fn conventional_default() -> Self {
        Self {
            default_output: None,
            output_flag: Some("--output"),
            output_short: None,
            allow_output_format: true,
        }
    }

    /// Return a copy with a different default output.
    pub const fn with_default_output(mut self, default_output: Option<OutputFormat>) -> Self {
        self.default_output = default_output;
        self
    }

    /// Return a copy with a different long output flag.
    pub const fn with_output_flag(mut self, flag: Option<&'static str>) -> Self {
        self.output_flag = flag;
        self
    }

    /// Return a copy with a different short output flag.
    pub const fn with_output_short(mut self, flag: Option<char>) -> Self {
        self.output_short = flag;
        self
    }

    /// Return a copy that enables or disables explicit output overrides.
    pub const fn with_output_format_override(mut self, enabled: bool) -> Self {
        self.allow_output_format = enabled;
        self
    }
}

/// Parse `--output` flag value into [`OutputFormat`].
///
/// Returns `Err` with a message suitable for passing to [`build_cli_error`] on unknown values.
///
/// ```
/// use agent_first_data::{cli_parse_output, OutputFormat};
/// assert!(matches!(cli_parse_output("json"), Ok(OutputFormat::Json)));
/// assert!(cli_parse_output("xml").is_err());
/// ```
pub fn cli_parse_output(s: &str) -> Result<OutputFormat, String> {
    match s {
        "json" => Ok(OutputFormat::Json),
        "yaml" => Ok(OutputFormat::Yaml),
        "plain" => Ok(OutputFormat::Plain),
        _ => Err(format!(
            "invalid --output format '{s}': expected json, yaml, or plain"
        )),
    }
}

/// Normalize `--log` flag entries: trim, lowercase, deduplicate, remove empty.
///
/// Accepts pre-split entries as produced by clap's `value_delimiter = ','`.
///
/// ```
/// use agent_first_data::cli_parse_log_filters;
/// let f = cli_parse_log_filters(&["Query", " error ", "query"]);
/// assert_eq!(f, vec!["query", "error"]);
/// ```
pub fn cli_parse_log_filters<S: AsRef<str>>(entries: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in entries {
        let s = entry.as_ref().trim().to_ascii_lowercase();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Dispatch output formatting by [`OutputFormat`].
///
/// Equivalent to calling [`output_json`], [`output_yaml`], or [`output_plain`] directly.
///
/// ```
/// use agent_first_data::{cli_output, OutputFormat};
/// let v = serde_json::json!({"code": "ok"});
/// let s = cli_output(&v, OutputFormat::Plain);
/// assert!(s.contains("code=ok"));
/// ```
pub fn cli_output(value: &Value, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output_json(value),
        OutputFormat::Yaml => output_yaml(value),
        OutputFormat::Plain => output_plain(value),
    }
}

/// Dispatch output formatting by [`OutputFormat`] with configurable output options.
///
/// JSON output ignores [`OutputStyle`] and always preserves original keys and values after
/// redaction. YAML and plain output use the requested style.
pub fn cli_output_with_options(
    value: &Value,
    format: OutputFormat,
    output_options: &OutputOptions,
) -> String {
    match format {
        OutputFormat::Json => output_json_with_options(value, output_options),
        OutputFormat::Yaml => output_yaml_with_options(value, output_options),
        OutputFormat::Plain => output_plain_with_options(value, output_options),
    }
}

/// Build a standard CLI version value.
pub fn build_cli_version(version: &str) -> Value {
    build_json("version", serde_json::json!({ "version": version }), None)
}

/// Render a CLI version response.
///
/// Pass `Some(format)` for an AFDATA event in JSON/YAML/plain. Pass `None` to
/// preserve conventional `<name> <version>` output.
pub fn cli_render_version(name: &str, version: &str, format: Option<OutputFormat>) -> String {
    let mut rendered = match format {
        Some(format) => cli_output(&build_cli_version(version), format),
        None => format!("{name} {version}"),
    };
    while rendered.ends_with('\n') {
        rendered.pop();
    }
    rendered.push('\n');
    rendered
}

/// Render version output from raw argv if `--version` or `-V` is present.
///
/// `raw_args` should be the full argv vector, including argv[0], as produced by
/// `std::env::args()`. The helper intentionally runs before clap or another
/// parser so explicit `--output json|yaml|plain` is honored instead of being
/// bypassed by built-in version handling.
///
/// Returns a standard [`build_cli_error`] value when the version request is
/// malformed, for example `--version --output xml`.
pub fn cli_handle_version_or_continue(
    raw_args: &[String],
    name: &str,
    version: &str,
    config: &VersionConfig,
) -> Result<Option<String>, Value> {
    let parsed = parse_version_request(raw_args, config);
    if !parsed.version_requested {
        return Ok(None);
    }
    if let Some(error) = parsed.output_error {
        return Err(build_cli_error(
            &error,
            Some("valid version output formats: json, yaml, plain"),
        ));
    }
    let format = if config.allow_output_format {
        parsed.output_format.or(config.default_output)
    } else {
        config.default_output
    };
    Ok(Some(cli_render_version(name, version, format)))
}

struct ParsedVersionRequest {
    version_requested: bool,
    output_format: Option<OutputFormat>,
    output_error: Option<String>,
}

fn parse_version_request(raw_args: &[String], config: &VersionConfig) -> ParsedVersionRequest {
    let args = raw_args.get(1..).unwrap_or(&[]);
    let mut version_requested = false;
    let mut output_format = None;
    let mut output_error = None;
    let output_flag = config.output_flag.map(normalize_long_flag);

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            break;
        }

        let (flag_name, inline_value) = split_flag(arg);
        if matches!(arg, "--version" | "-V") {
            version_requested = true;
            i += 1;
            continue;
        }

        if config.allow_output_format
            && version_output_flag_matches(flag_name, output_flag, config.output_short)
        {
            let value = inline_value.or_else(|| {
                args.get(i + 1)
                    .map(String::as_str)
                    .filter(|next| !next.starts_with('-'))
            });
            if let Some(value) = value {
                match cli_parse_output(value) {
                    Ok(format) => output_format = Some(format),
                    Err(err) => output_error = Some(err),
                }
            } else {
                output_error = Some(format!(
                    "missing value for --{}: expected json, yaml, or plain",
                    output_flag.unwrap_or("output")
                ));
            }
            i += if inline_value.is_some() || value.is_none() {
                1
            } else {
                2
            };
            continue;
        }
        i += 1;
    }

    ParsedVersionRequest {
        version_requested,
        output_format,
        output_error,
    }
}

fn version_output_flag_matches(
    flag_name: Option<&str>,
    output_flag: Option<&str>,
    output_short: Option<char>,
) -> bool {
    let Some(seen) = flag_name else {
        return false;
    };
    output_flag.is_some_and(|expected| seen == expected)
        || output_short.is_some_and(|short| {
            let mut chars = seen.chars();
            chars.next().is_some_and(|seen_short| seen_short == short) && chars.next().is_none()
        })
}

/// Build a standard CLI parse error value.
///
/// Use when `Cli::try_parse()` fails or a flag value is invalid.
/// Print with [`output_json`] and exit with code 2.
///
/// ```
/// let err = agent_first_data::build_cli_error("--output: invalid value 'xml'", None);
/// assert_eq!(err["code"], "error");
/// assert_eq!(err["error"], "--output: invalid value 'xml'");
/// assert!(err.get("error_code").is_none());
/// ```
pub fn build_cli_error(message: &str, hint: Option<&str>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("code".to_string(), Value::String("error".to_string()));
    obj.insert("error".to_string(), Value::String(message.to_string()));
    if let Some(h) = hint {
        obj.insert("hint".to_string(), Value::String(h.to_string()));
    }
    Value::Object(obj)
}

// ═══════════════════════════════════════════
// Public API: CLI Help Rendering (optional)
// ═══════════════════════════════════════════

/// How much of a command tree a help request should render.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpScope {
    /// Render only the selected command's own clap-style help.
    ///
    /// Clap's normal help still lists direct subcommands in the "Commands"
    /// section, but descendant command detail is not expanded.
    OneLevel,
    /// Render the selected command and all visible descendant subcommands.
    Recursive,
}

/// Output format for help rendering.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpFormat {
    Plain,
    Markdown,
    Json,
    Yaml,
}

#[cfg(feature = "cli-help")]
impl HelpFormat {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "plain" => Some(Self::Plain),
            "markdown" => Some(Self::Markdown),
            "json" => Some(Self::Json),
            "yaml" => Some(Self::Yaml),
            _ => None,
        }
    }
}

/// Options for rendering CLI help.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpOptions {
    pub scope: HelpScope,
    pub format: HelpFormat,
}

#[cfg(feature = "cli-help")]
impl HelpOptions {
    /// Human-friendly current-level plain help.
    pub const fn one_level_plain() -> Self {
        Self {
            scope: HelpScope::OneLevel,
            format: HelpFormat::Plain,
        }
    }

    /// Agent/doc-friendly recursive plain help.
    pub const fn recursive_plain() -> Self {
        Self {
            scope: HelpScope::Recursive,
            format: HelpFormat::Plain,
        }
    }
}

/// Configuration for pre-clap help handling.
///
/// The handler scans raw argv before `Cli::try_parse()` so applications can
/// support requests such as `--help --output markdown` without clap exiting
/// early with `DisplayHelp`.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpConfig {
    /// Scope used for `--help` / `-h` when neither `--recursive` nor a
    /// configured `recursive_flag` is present.
    pub default_scope: HelpScope,
    /// Format used for help when no explicit output flag is present.
    pub default_format: HelpFormat,
    /// Optional extra alias for the built-in `--recursive` scope modifier.
    ///
    /// `--recursive` is always recognized; set this only to accept an
    /// additional custom flag name (for example `--full`). Like `--recursive`,
    /// the alias is a *modifier* that selects recursive scope when `--help` is
    /// present; on its own it does not trigger help.
    pub recursive_flag: Option<&'static str>,
    /// Optional output flag to read help format from, for example `--output`.
    pub output_flag: Option<&'static str>,
    /// Whether an explicit output flag can override `default_format`.
    pub allow_output_format: bool,
}

#[cfg(feature = "cli-help")]
impl HelpConfig {
    /// Construct a custom help handler configuration.
    pub const fn new(default_scope: HelpScope, default_format: HelpFormat) -> Self {
        Self {
            default_scope,
            default_format,
            recursive_flag: None,
            output_flag: None,
            allow_output_format: false,
        }
    }

    /// Recommended preset for human-facing CLIs.
    ///
    /// `--help` renders one-level plain help by default. Scope and format are
    /// orthogonal: `--recursive` expands the selected command subtree, while
    /// `--output json|yaml|markdown` picks the format. So `--help --recursive`
    /// is recursive plain text and `--help --recursive --output markdown` is a
    /// recursive Markdown export.
    pub const fn human_cli_default() -> Self {
        Self {
            default_scope: HelpScope::OneLevel,
            default_format: HelpFormat::Plain,
            recursive_flag: None,
            output_flag: Some("--output"),
            allow_output_format: true,
        }
    }

    /// Recommended preset for agent-first CLIs that want full surface help by default.
    pub const fn agent_cli_default() -> Self {
        Self {
            default_scope: HelpScope::Recursive,
            default_format: HelpFormat::Plain,
            recursive_flag: None,
            output_flag: Some("--output"),
            allow_output_format: true,
        }
    }

    /// Return a copy with a different default scope.
    pub const fn with_default_scope(mut self, scope: HelpScope) -> Self {
        self.default_scope = scope;
        self
    }

    /// Return a copy with a different default format.
    pub const fn with_default_format(mut self, format: HelpFormat) -> Self {
        self.default_format = format;
        self
    }

    /// Return a copy with a different recursive-help flag.
    pub const fn with_recursive_flag(mut self, flag: Option<&'static str>) -> Self {
        self.recursive_flag = flag;
        self
    }

    /// Return a copy with a different output flag.
    pub const fn with_output_flag(mut self, flag: Option<&'static str>) -> Self {
        self.output_flag = flag;
        self
    }

    /// Return a copy that enables or disables help format overrides.
    pub const fn with_output_format_override(mut self, enabled: bool) -> Self {
        self.allow_output_format = enabled;
        self
    }
}

/// Render help for a clap command tree with explicit scope and format.
///
/// Walks to the subcommand identified by `subcommand_path` (empty = root),
/// then renders either the selected command only (`OneLevel`) or the selected
/// command and all descendants (`Recursive`).
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
pub fn cli_render_help_with_options(
    cmd: &clap::Command,
    subcommand_path: &[&str],
    options: &HelpOptions,
) -> String {
    let target = walk_to_subcommand(cmd, subcommand_path);
    let mut rendered = match options.format {
        HelpFormat::Plain => match options.scope {
            HelpScope::OneLevel => render_help_one_level_plain(target),
            HelpScope::Recursive => {
                let mut buf = String::new();
                render_help_recursive_plain(target, &[], &mut buf);
                buf
            }
        },
        HelpFormat::Markdown => render_help_markdown(cmd, subcommand_path, options.scope),
        HelpFormat::Json => {
            serialize_json_output(&build_help_schema(cmd, subcommand_path, options.scope))
        }
        HelpFormat::Yaml => output_yaml_with_options(
            &build_help_schema(cmd, subcommand_path, options.scope),
            &OutputOptions {
                redaction: RedactionOptions {
                    policy: Some(RedactionPolicy::RedactionNone),
                    secret_names: Vec::new(),
                },
                style: OutputStyle::Raw,
            },
        ),
    };
    // Every format ends with exactly one trailing newline so `print!`-ing the
    // result is clean across plain/markdown/json/yaml (JSON and raw YAML would
    // otherwise have none).
    while rendered.ends_with('\n') {
        rendered.pop();
    }
    rendered.push('\n');
    rendered
}

/// Render recursive plain-text help for a clap command tree.
///
/// Walks to the subcommand identified by `subcommand_path` (empty = root),
/// then recursively expands all descendant subcommands into a single output.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
pub fn cli_render_help(cmd: &clap::Command, subcommand_path: &[&str]) -> String {
    cli_render_help_with_options(cmd, subcommand_path, &HelpOptions::recursive_plain())
}

/// Render recursive Markdown help for a clap command tree.
///
/// Same tree walk as [`cli_render_help`], but outputs Markdown suitable for
/// documentation generation (`myapp --help --recursive --output markdown > docs/cli.md`).
///
/// Requires the `cli-help-markdown` feature.
#[cfg(feature = "cli-help-markdown")]
pub fn cli_render_help_markdown(cmd: &clap::Command, subcommand_path: &[&str]) -> String {
    cli_render_help_with_options(
        cmd,
        subcommand_path,
        &HelpOptions {
            scope: HelpScope::Recursive,
            format: HelpFormat::Markdown,
        },
    )
}

/// Render help from raw argv if a help flag is present; otherwise return `None`.
///
/// `raw_args` should be the full argv vector, including argv[0], as produced by
/// `std::env::args()`. The helper intentionally runs before clap parsing so
/// `--help --recursive` and `--help --output markdown` can select scope and
/// format instead of being consumed by clap's built-in help handling. Scope
/// (`--recursive`) and format (`--output`) are orthogonal.
///
/// A bare `--recursive` without `--help` is treated as a non-help request
/// (`Ok(None)`), leaving the flag for the application's own parser.
///
/// Returns a standard [`build_cli_error`] value when the help request is
/// malformed, for example `--help --output xml`.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
pub fn cli_handle_help_or_continue(
    raw_args: &[String],
    cmd: &clap::Command,
    config: &HelpConfig,
) -> Result<Option<String>, Value> {
    let parsed = parse_help_request(raw_args, cmd, config);
    if !parsed.help_requested {
        return Ok(None);
    }
    if let Some(error) = parsed.output_error {
        return Err(build_cli_error(
            &error,
            Some("valid help output formats: plain, markdown, json, yaml"),
        ));
    }

    let (scope, format) = resolve_help_options(&parsed, config);
    let path: Vec<&str> = parsed.subcommand_path.iter().map(String::as_str).collect();
    Ok(Some(cli_render_help_with_options(
        cmd,
        &path,
        &HelpOptions { scope, format },
    )))
}

#[cfg(feature = "cli-help")]
fn resolve_help_options(
    parsed: &ParsedHelpRequest,
    config: &HelpConfig,
) -> (HelpScope, HelpFormat) {
    // Scope and format are orthogonal: `--recursive` (or the configured
    // recursive flag, or a recursive default_scope) decides one-level vs
    // recursive, while `--output` independently decides the format.
    let scope = if parsed.recursive_requested {
        HelpScope::Recursive
    } else {
        config.default_scope
    };
    let format = if config.allow_output_format {
        parsed.output_format.unwrap_or(config.default_format)
    } else {
        config.default_format
    };
    (scope, format)
}

#[cfg(feature = "cli-help")]
fn walk_to_subcommand<'a>(cmd: &'a clap::Command, path: &[&str]) -> &'a clap::Command {
    let mut current = cmd;
    for name in path {
        current = current.find_subcommand(name).unwrap_or(current);
    }
    current
}

#[cfg(feature = "cli-help")]
fn walk_to_subcommand_with_names<'a>(
    cmd: &'a clap::Command,
    path: &[&str],
) -> (&'a clap::Command, Vec<String>) {
    let mut current = cmd;
    let mut names = vec![cmd.get_name().to_string()];
    for name in path {
        if let Some(next) = current.find_subcommand(name) {
            current = next;
            names.push(next.get_name().to_string());
        } else {
            break;
        }
    }
    (current, names)
}

#[cfg(feature = "cli-help")]
fn render_help_one_level_plain(cmd: &clap::Command) -> String {
    enriched_help_command(cmd).render_long_help().to_string()
}

/// Clone `cmd` and fold the afdata-handled help modifiers into clap's own
/// `-h, --help` description.
///
/// Help is rendered by clap, which has no knowledge of the `--recursive` scope
/// modifier or the `--output` help formats (afdata consumes both before clap
/// parses). Rather than appending a separate section, we patch the description
/// of the existing help flag so the help surface is documented in place — in
/// every format, since plain/markdown render this flag and the JSON/YAML schema
/// reads it. Commands with subcommands advertise `--recursive`; leaf commands
/// only advertise the `--output` formats (they have nothing to expand).
#[cfg(feature = "cli-help")]
fn enriched_help_command(cmd: &clap::Command) -> clap::Command {
    let cmd = cmd.clone();
    let description = if visible_subcommands(&cmd).next().is_some() {
        HELP_FLAG_WITH_SUBCOMMANDS
    } else {
        HELP_FLAG_LEAF
    };
    // clap auto-generates `-h, --help` lazily during build, so `mut_arg` cannot
    // reach it yet. Replace it with an explicit flag carrying the enriched
    // description. This command is only rendered, never parsed (afdata handles
    // `--help` before clap), so the action is immaterial.
    cmd.disable_help_flag(true).arg(
        clap::Arg::new("help")
            .short('h')
            .long("help")
            .help(description)
            .long_help(description)
            .action(clap::ArgAction::Help),
    )
}

/// Description for the `-h, --help` flag on commands that have subcommands.
#[cfg(feature = "cli-help")]
const HELP_FLAG_WITH_SUBCOMMANDS: &str =
    "Print help. Add --recursive to expand every nested subcommand; \
     add --output json|yaml|markdown to render this help in another format.";

/// Description for the `-h, --help` flag on leaf commands (no subcommands).
#[cfg(feature = "cli-help")]
const HELP_FLAG_LEAF: &str =
    "Print help. Add --output json|yaml|markdown to render this help in another format.";

#[cfg(feature = "cli-help")]
fn render_help_recursive_plain(cmd: &clap::Command, parent_path: &[&str], buf: &mut String) {
    use std::fmt::Write;

    // Build the full command path (e.g. "myapp service start")
    let mut cmd_path = parent_path.to_vec();
    cmd_path.push(cmd.get_name());
    let path_str = cmd_path.join(" ");

    // Separator between commands (skip for the first one)
    if !buf.is_empty() {
        let _ = writeln!(buf);
        let _ = writeln!(buf, "{}", "═".repeat(60));
    }

    // Header: "myapp service start — description"
    if let Some(about) = cmd.get_about() {
        let _ = writeln!(buf, "{path_str} — {about}");
    } else {
        let _ = writeln!(buf, "{path_str}");
    }
    let _ = writeln!(buf);

    // Render clap's built-in help for this command (usage, args, options).
    // Only the target command (top of the recursion) advertises the help
    // modifiers; repeating them on every descendant block would be pure noise.
    let is_target = parent_path.is_empty();
    let styled = if is_target {
        enriched_help_command(cmd).render_long_help()
    } else {
        cmd.clone().render_long_help()
    };
    let help_text = styled.to_string();
    let _ = write!(buf, "{help_text}");

    // Recurse into visible subcommands
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" || sub.is_hide_set() {
            continue; // skip clap's auto-generated "help" subcommand
        }
        render_help_recursive_plain(sub, &cmd_path, buf);
    }
}

#[cfg(feature = "cli-help")]
fn render_help_markdown(cmd: &clap::Command, subcommand_path: &[&str], scope: HelpScope) -> String {
    let (target, names) = walk_to_subcommand_with_names(cmd, subcommand_path);
    let mut buf = String::new();
    render_markdown_command(target, &names, &mut buf, 1, true);
    if matches!(scope, HelpScope::Recursive) {
        render_markdown_descendants(target, &names, &mut buf, 2);
    }
    buf
}

#[cfg(feature = "cli-help")]
fn render_markdown_descendants(
    cmd: &clap::Command,
    parent_names: &[String],
    buf: &mut String,
    level: usize,
) {
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" || sub.is_hide_set() {
            continue;
        }
        let mut names = parent_names.to_vec();
        names.push(sub.get_name().to_string());
        render_markdown_command(sub, &names, buf, level, false);
        render_markdown_descendants(sub, &names, buf, level.saturating_add(1));
    }
}

#[cfg(feature = "cli-help")]
fn render_markdown_command(
    cmd: &clap::Command,
    names: &[String],
    buf: &mut String,
    level: usize,
    enrich: bool,
) {
    use std::fmt::Write;

    if !buf.is_empty() {
        let _ = writeln!(buf);
    }
    let heading_level = "#".repeat(level.max(1));
    let path = names.join(" ");
    if let Some(about) = cmd.get_about() {
        let _ = writeln!(buf, "{heading_level} {path} - {about}");
    } else {
        let _ = writeln!(buf, "{heading_level} {path}");
    }
    if let Some(long_about) = cmd.get_long_about() {
        let _ = writeln!(buf);
        let _ = writeln!(buf, "{long_about}");
    }
    let _ = writeln!(buf);
    let _ = writeln!(buf, "```text");
    let help = if enrich {
        enriched_help_command(cmd).render_long_help()
    } else {
        cmd.clone().render_long_help()
    };
    write_trimmed_help(buf, &help.to_string());
    if !buf.ends_with('\n') {
        let _ = writeln!(buf);
    }
    let _ = writeln!(buf, "```");
}

#[cfg(feature = "cli-help")]
fn write_trimmed_help(buf: &mut String, help: &str) {
    use std::fmt::Write;

    for line in help.lines() {
        let _ = writeln!(buf, "{}", line.trim_end());
    }
}

#[cfg(feature = "cli-help")]
struct ParsedHelpRequest {
    help_requested: bool,
    recursive_requested: bool,
    output_format: Option<HelpFormat>,
    output_error: Option<String>,
    subcommand_path: Vec<String>,
}

#[cfg(feature = "cli-help")]
fn parse_help_request(
    raw_args: &[String],
    cmd: &clap::Command,
    config: &HelpConfig,
) -> ParsedHelpRequest {
    let args = match raw_args.first() {
        Some(first) if first.starts_with('-') || cmd.find_subcommand(first).is_some() => raw_args,
        _ => raw_args.get(1..).unwrap_or(&[]),
    };
    let mut help_requested = false;
    let mut recursive_requested = false;
    let mut output_format = None;
    let mut output_error = None;
    let mut subcommand_path = Vec::new();
    let mut current = cmd;
    let output_flag = config.output_flag.map(normalize_long_flag);
    let recursive_flag = config.recursive_flag.map(normalize_long_flag);

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            break;
        }

        let (flag_name, inline_value) = split_flag(arg);
        if matches!(arg, "--help" | "-h") {
            help_requested = true;
            i += 1;
            continue;
        }
        // `--recursive` is a help *modifier*, not a help trigger: it only
        // selects recursive scope when `--help` is also present. A bare
        // `--recursive` leaves help_requested false so the full argv falls
        // through to the application's own parser untouched.
        if arg == "--recursive"
            || flag_name
                .zip(recursive_flag)
                .is_some_and(|(seen, expected)| seen == expected)
        {
            recursive_requested = true;
            i += 1;
            continue;
        }
        if config.allow_output_format
            && flag_name
                .zip(output_flag)
                .is_some_and(|(seen, expected)| seen == expected)
        {
            let value = inline_value.or_else(|| {
                args.get(i + 1)
                    .map(String::as_str)
                    .filter(|next| !next.starts_with('-'))
            });
            if let Some(value) = value {
                match HelpFormat::parse(value) {
                    Some(format) => output_format = Some(format),
                    None => {
                        output_error = Some(format!(
                            "invalid --{} format '{}': expected plain, json, yaml, or markdown",
                            output_flag.unwrap_or("output"),
                            value
                        ));
                    }
                }
            } else {
                output_error = Some(format!(
                    "missing value for --{}: expected plain, json, yaml, or markdown",
                    output_flag.unwrap_or("output")
                ));
            }
            i += if inline_value.is_some() || value.is_none() {
                1
            } else {
                2
            };
            continue;
        }
        if arg.starts_with('-') {
            i += if inline_value.is_none() && flag_takes_value(current, arg) {
                2
            } else {
                1
            };
            continue;
        }
        if let Some(sub) = current.find_subcommand(arg) {
            if sub.get_name() != "help" && !sub.is_hide_set() {
                subcommand_path.push(sub.get_name().to_string());
                current = sub;
            }
        }
        i += 1;
    }

    ParsedHelpRequest {
        help_requested,
        recursive_requested,
        output_format,
        output_error,
        subcommand_path,
    }
}

fn normalize_long_flag(flag: &str) -> &str {
    flag.trim_start_matches('-')
}

fn split_flag(arg: &str) -> (Option<&str>, Option<&str>) {
    if let Some(stripped) = arg.strip_prefix("--") {
        if let Some((name, value)) = stripped.split_once('=') {
            (Some(name), Some(value))
        } else {
            (Some(stripped), None)
        }
    } else if let Some(stripped) = arg.strip_prefix('-') {
        (Some(stripped), None)
    } else {
        (None, None)
    }
}

#[cfg(feature = "cli-help")]
fn flag_takes_value(cmd: &clap::Command, raw_flag: &str) -> bool {
    let Some(flag) = raw_flag.strip_prefix('-') else {
        return false;
    };
    let name = flag.trim_start_matches('-');
    cmd.get_arguments().any(|arg| {
        let long_matches = arg.get_long().is_some_and(|long| long == name);
        let short_matches =
            name.len() == 1 && arg.get_short().is_some_and(|short| name.starts_with(short));
        (long_matches || short_matches)
            && matches!(
                arg.get_action(),
                clap::ArgAction::Set | clap::ArgAction::Append
            )
    })
}

#[cfg(feature = "cli-help")]
fn build_help_schema(cmd: &clap::Command, subcommand_path: &[&str], scope: HelpScope) -> Value {
    let (target, names) = walk_to_subcommand_with_names(cmd, subcommand_path);
    let mut schema = command_schema(target, &names, matches!(scope, HelpScope::Recursive), true);
    if let Value::Object(map) = &mut schema {
        map.insert("code".to_string(), Value::String("help".to_string()));
        map.insert(
            "scope".to_string(),
            Value::String(help_scope_tag(scope).to_string()),
        );
    }
    schema
}

#[cfg(feature = "cli-help")]
fn help_scope_tag(scope: HelpScope) -> &'static str {
    match scope {
        HelpScope::OneLevel => "one_level",
        HelpScope::Recursive => "recursive",
    }
}

#[cfg(feature = "cli-help")]
fn command_schema(cmd: &clap::Command, names: &[String], recursive: bool, enrich: bool) -> Value {
    let subcommands: Vec<Value> = visible_subcommands(cmd)
        .map(|sub| {
            let mut child_names = names.to_vec();
            child_names.push(sub.get_name().to_string());
            if recursive {
                // Descendants never re-advertise the help modifiers (enrich=false).
                command_schema(sub, &child_names, true, false)
            } else {
                command_summary_schema(sub, &child_names)
            }
        })
        .collect();

    serde_json::json!({
        "name": cmd.get_name(),
        "command_path": names.join(" "),
        "path": names,
        "about": styled_to_value(cmd.get_about()),
        "long_about": styled_to_value(cmd.get_long_about()),
        "usage": cmd.clone().render_usage().to_string(),
        "arguments": command_arguments_schema(cmd, enrich),
        "subcommands": subcommands,
    })
}

#[cfg(feature = "cli-help")]
fn command_summary_schema(cmd: &clap::Command, names: &[String]) -> Value {
    serde_json::json!({
        "name": cmd.get_name(),
        "command_path": names.join(" "),
        "path": names,
        "about": styled_to_value(cmd.get_about()),
        "long_about": styled_to_value(cmd.get_long_about()),
        "usage": Value::Null,
        "arguments": [],
        "subcommands": [],
    })
}

#[cfg(feature = "cli-help")]
fn visible_subcommands(cmd: &clap::Command) -> impl Iterator<Item = &clap::Command> {
    cmd.get_subcommands()
        .filter(|sub| sub.get_name() != "help" && !sub.is_hide_set())
}

#[cfg(feature = "cli-help")]
fn command_arguments_schema(cmd: &clap::Command, enrich: bool) -> Vec<Value> {
    // For the target command, render through the enriched clone so the schema
    // documents the `-h, --help` modifiers (`--recursive`, `--output`) just like
    // the plain and markdown formats do (clap adds `--help` lazily during build,
    // so the raw command would omit it). Descendants stay un-enriched to avoid
    // repeating the same modifier doc on every command in a recursive dump.
    let owned = enrich.then(|| enriched_help_command(cmd));
    let source = owned.as_ref().unwrap_or(cmd);
    source
        .get_arguments()
        .filter(|arg| !arg.is_hide_set())
        .map(argument_schema)
        .collect()
}

#[cfg(feature = "cli-help")]
fn argument_schema(arg: &clap::Arg) -> Value {
    let value_names: Vec<String> = arg
        .get_value_names()
        .map(|names| names.iter().map(ToString::to_string).collect())
        .unwrap_or_default();
    let default_values: Vec<String> = arg
        .get_default_values()
        .iter()
        .map(|value| value.to_string_lossy().to_string())
        .collect();
    serde_json::json!({
        "id": arg.get_id().to_string(),
        "kind": if arg.get_long().is_some() || arg.get_short().is_some() { "option" } else { "argument" },
        "long": arg.get_long(),
        "short": arg.get_short().map(|c| c.to_string()),
        "help": styled_to_value(arg.get_help()),
        "long_help": styled_to_value(arg.get_long_help()),
        "required": arg.is_required_set(),
        "action": format!("{:?}", arg.get_action()),
        "value_names": value_names,
        "default_values": default_values,
    })
}

#[cfg(feature = "cli-help")]
fn styled_to_value(value: Option<&clap::builder::StyledStr>) -> Value {
    value.map_or(Value::Null, |s| Value::String(s.to_string()))
}

// ═══════════════════════════════════════════
// Secret Redaction
// ═══════════════════════════════════════════

#[derive(Default)]
struct RedactionContext {
    secret_names: HashSet<String>,
}

impl RedactionContext {
    fn from_options(redaction_options: &RedactionOptions) -> Self {
        let secret_names = redaction_options.secret_names.iter().cloned().collect();
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

const MAX_DEPTH: usize = 256;

fn redact_secrets(value: &mut Value) {
    let context = RedactionContext::default();
    redact_secrets_with_context(value, &context);
}

fn redact_secrets_with_context(value: &mut Value, context: &RedactionContext) {
    redact_secrets_with_context_depth(value, context, 0);
}

fn redact_secrets_with_context_depth(value: &mut Value, context: &RedactionContext, depth: usize) {
    if depth >= MAX_DEPTH {
        *value = Value::String("***".into());
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
    if trimmed != s {
        if let Some(redacted) = redact_url_in_str(trimmed, context) {
            return redacted;
        }
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

fn apply_redaction_policy(value: &mut Value, redaction_policy: RedactionPolicy) {
    let context = RedactionContext::default();
    apply_redaction_policy_with_context(value, Some(redaction_policy), &context);
}

fn apply_redaction_options(value: &mut Value, redaction_options: &RedactionOptions) {
    let context = RedactionContext::from_options(redaction_options);
    apply_redaction_policy_with_context(value, redaction_options.policy, &context);
}

fn apply_redaction_policy_with_context(
    value: &mut Value,
    redaction_policy: Option<RedactionPolicy>,
    context: &RedactionContext,
) {
    match redaction_policy {
        Some(RedactionPolicy::RedactionTraceOnly) => {
            if let Value::Object(map) = value {
                if let Some(trace) = map.get_mut("trace") {
                    redact_secrets_with_context(trace, context);
                }
            }
        }
        Some(RedactionPolicy::RedactionNone) => {}
        None => redact_secrets_with_context(value, context),
    }
}

// ═══════════════════════════════════════════
// Suffix Processing
// ═══════════════════════════════════════════

/// Strip a suffix matching exact lowercase or exact uppercase only.
fn strip_suffix_ci(key: &str, suffix_lower: &str) -> Option<String> {
    if let Some(s) = key.strip_suffix(suffix_lower) {
        return Some(s.to_string());
    }
    let suffix_upper: String = suffix_lower
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if let Some(s) = key.strip_suffix(&suffix_upper) {
        return Some(s.to_string());
    }
    None
}

/// Extract currency code from `_{code}_cents` / `_{CODE}_CENTS` pattern.
fn try_strip_generic_cents(key: &str) -> Option<(String, String)> {
    let code = extract_currency_code(key)?;
    let suffix_len = code.len() + "_cents".len() + 1; // _{code}_cents
    let stripped = &key[..key.len() - suffix_len];
    if stripped.is_empty() {
        return None;
    }
    Some((stripped.to_string(), code.to_string()))
}

/// Try suffix-driven processing. Returns Some((stripped_key, formatted_value))
/// when suffix matches and type is valid. None for no match or type mismatch.
/// Accept an integer value, including an integral-valued float (`3.0` → `3`).
/// Non-integral floats and out-of-range values return `None`. This keeps the
/// four language implementations consistent: JS/TS cannot distinguish `3` from
/// `3.0` after JSON parsing, so the value's integrality — not its lexical form —
/// decides whether an integer-required suffix applies.
fn as_int(value: &Value) -> Option<i64> {
    if let Some(i) = value.as_i64() {
        return Some(i);
    }
    let f = value.as_f64()?;
    if f.is_finite() && f.fract() == 0.0 && (i64::MIN as f64..=i64::MAX as f64).contains(&f) {
        return Some(f as i64);
    }
    None
}

/// Like [`as_int`] but for non-negative integers (rejects negatives).
fn as_uint(value: &Value) -> Option<u64> {
    if let Some(u) = value.as_u64() {
        return Some(u);
    }
    let f = value.as_f64()?;
    if f.is_finite() && f.fract() == 0.0 && (0.0..=u64::MAX as f64).contains(&f) {
        return Some(f as u64);
    }
    None
}

fn try_process_field(key: &str, value: &Value) -> Option<(String, String)> {
    // Group 1: compound timestamp suffixes
    if let Some(stripped) = strip_suffix_ci(key, "_epoch_ms") {
        return as_int(value)
            .and_then(|ms| format_rfc3339_ms(ms).map(|formatted| (stripped, formatted)));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_epoch_s") {
        return as_int(value)
            .and_then(|s| s.checked_mul(1000))
            .and_then(|ms| format_rfc3339_ms(ms).map(|formatted| (stripped, formatted)));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_epoch_ns") {
        return as_int(value).and_then(|ns| {
            format_rfc3339_ms(ns.div_euclid(1_000_000)).map(|formatted| (stripped, formatted))
        });
    }

    // Group 2: compound currency suffixes
    if let Some(stripped) = strip_suffix_ci(key, "_usd_cents") {
        return as_uint(value).map(|n| (stripped, format!("${}.{:02}", n / 100, n % 100)));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_eur_cents") {
        return as_uint(value).map(|n| (stripped, format!("€{}.{:02}", n / 100, n % 100)));
    }
    if let Some((stripped, code)) = try_strip_generic_cents(key) {
        return as_uint(value).map(|n| {
            (
                stripped,
                format!("{}.{:02} {}", n / 100, n % 100, code.to_uppercase()),
            )
        });
    }

    // Group 3: multi-char suffixes
    if let Some(stripped) = strip_suffix_ci(key, "_rfc3339") {
        return value.as_str().map(|s| (stripped, s.to_string()));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_minutes") {
        return value
            .is_number()
            .then(|| (stripped, format!("{} minutes", number_str(value))));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_hours") {
        return value
            .is_number()
            .then(|| (stripped, format!("{} hours", number_str(value))));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_days") {
        return value
            .is_number()
            .then(|| (stripped, format!("{} days", number_str(value))));
    }

    // Group 4: single-unit suffixes
    if let Some(stripped) = strip_suffix_ci(key, "_msats") {
        return value
            .is_number()
            .then(|| (stripped, format!("{}msats", number_str(value))));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_sats") {
        return value
            .is_number()
            .then(|| (stripped, format!("{}sats", number_str(value))));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_bytes") {
        return as_int(value).map(|n| (stripped, format_bytes_human(n)));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_percent") {
        return value
            .is_number()
            .then(|| (stripped, format!("{}%", number_str(value))));
    }
    // Group 5: short suffixes (last to avoid false positives)
    if let Some(stripped) = strip_suffix_ci(key, "_btc") {
        return value
            .is_number()
            .then(|| (stripped, format!("{} BTC", number_str(value))));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_jpy") {
        return as_uint(value).map(|n| (stripped, format!("¥{}", format_with_commas(n))));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_ns") {
        return value
            .is_number()
            .then(|| (stripped, format!("{}ns", number_str(value))));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_us") {
        return value
            .is_number()
            .then(|| (stripped, format!("{}μs", number_str(value))));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_ms") {
        return format_ms_value(value).map(|v| (stripped, v));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_s") {
        return value
            .is_number()
            .then(|| (stripped, format!("{}s", number_str(value))));
    }

    None
}

/// Process object fields: strip keys, format values, detect collisions.
fn process_object_fields<'a>(
    map: &'a serde_json::Map<String, Value>,
) -> Vec<(String, &'a Value, Option<String>)> {
    let mut entries: Vec<(String, &'a str, &'a Value, Option<String>)> = Vec::new();
    for (key, value) in map {
        if let Some(stripped) = strip_suffix_ci(key, "_secret") {
            entries.push((stripped, key.as_str(), value, None));
            continue;
        }
        match try_process_field(key, value) {
            Some((stripped, formatted)) => {
                entries.push((stripped, key.as_str(), value, Some(formatted)));
            }
            None => {
                entries.push((key.clone(), key.as_str(), value, None));
            }
        }
    }

    // Detect collisions
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (stripped, _, _, _) in &entries {
        *counts.entry(stripped.clone()).or_insert(0) += 1;
    }

    // Resolve collisions: revert both key and formatted value
    let mut result: Vec<(String, &'a Value, Option<String>)> = entries
        .into_iter()
        .map(|(stripped, original, value, formatted)| {
            if counts.get(&stripped).copied().unwrap_or(0) > 1 && original != stripped.as_str() {
                (original.to_string(), value, None)
            } else {
                (stripped, value, formatted)
            }
        })
        .collect();

    result.sort_by(|(a, _, _), (b, _, _)| a.encode_utf16().cmp(b.encode_utf16()));
    result
}

// ═══════════════════════════════════════════
// Formatting Helpers
// ═══════════════════════════════════════════

fn number_str(value: &Value) -> String {
    match value {
        Value::Number(n) => format_number(n),
        _ => String::new(),
    }
}

/// Render a JSON number canonically for YAML/plain output: an integral-valued
/// float drops its trailing `.0` so `3.0` and `3` both render as `3`. This
/// matches Go (`strconv.FormatFloat(_, 'f', -1, 64)`), TypeScript
/// (`Number.prototype.toString`), and Python (`int(v)` for integral floats),
/// keeping the four implementations byte-identical.
fn format_number(n: &serde_json::Number) -> String {
    if n.is_f64() {
        if let Some(f) = n.as_f64() {
            if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e21 {
                return format!("{f:.0}");
            }
        }
    }
    normalize_exponent(&n.to_string())
}

fn normalize_exponent(s: &str) -> String {
    let Some(e) = s.find(['e', 'E']) else {
        return s.to_string();
    };
    let mantissa = &s[..e];
    let mut exp = &s[e + 1..];
    let mut sign = "";
    if exp.starts_with(['+', '-']) {
        sign = &exp[..1];
        exp = &exp[1..];
    }
    let exp = exp.trim_start_matches('0');
    let exp = if exp.is_empty() { "0" } else { exp };
    format!("{mantissa}e{sign}{exp}")
}

/// Format ms as seconds: 3 decimal places, trim trailing zeros, min 1 decimal.
fn format_ms_as_seconds(ms: f64) -> String {
    let formatted = format!("{:.3}", ms / 1000.0);
    let trimmed = formatted.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{}0s", trimmed)
    } else {
        format!("{}s", trimmed)
    }
}

/// Format `_ms` value: < 1000 → `{n}ms`, ≥ 1000 → seconds.
fn format_ms_value(value: &Value) -> Option<String> {
    let n = value.as_f64()?;
    if n.abs() >= 1000.0 {
        Some(format_ms_as_seconds(n))
    } else if let Some(i) = value.as_i64() {
        Some(format!("{}ms", i))
    } else {
        Some(format!("{}ms", number_str(value)))
    }
}

/// Convert unix milliseconds (signed) to RFC 3339 with UTC timezone.
const MIN_RFC3339_MS: i64 = -62135596800000;
const MAX_RFC3339_MS: i64 = 253402300799999;

fn format_rfc3339_ms(ms: i64) -> Option<String> {
    use chrono::{DateTime, Utc};
    if !(MIN_RFC3339_MS..=MAX_RFC3339_MS).contains(&ms) {
        return None;
    }
    let secs = ms.div_euclid(1000);
    let nanos = (ms.rem_euclid(1000) * 1_000_000) as u32;
    DateTime::from_timestamp(secs, nanos).map(|dt| {
        dt.with_timezone(&Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    })
}

/// Format bytes as human-readable size (binary units). Handles negative values.
fn format_bytes_human(bytes: i64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;

    let sign = if bytes < 0 { "-" } else { "" };
    let b = (bytes as f64).abs();
    if b >= TB {
        format!("{sign}{:.1}TB", b / TB)
    } else if b >= GB {
        format!("{sign}{:.1}GB", b / GB)
    } else if b >= MB {
        format!("{sign}{:.1}MB", b / MB)
    } else if b >= KB {
        format!("{sign}{:.1}KB", b / KB)
    } else {
        format!("{bytes}B")
    }
}

/// Format a number with thousands separators.
fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

/// Extract currency code from a `_{code}_cents` / `_{CODE}_CENTS` suffix.
fn extract_currency_code(key: &str) -> Option<&str> {
    let without_cents = key
        .strip_suffix("_cents")
        .or_else(|| key.strip_suffix("_CENTS"))?;
    let last_underscore = without_cents.rfind('_')?;
    let code = &without_cents[last_underscore + 1..];
    if code.is_empty()
        || !(3..=4).contains(&code.len())
        || !code.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return None;
    }
    Some(code)
}

// ═══════════════════════════════════════════
// YAML Rendering
// ═══════════════════════════════════════════

fn render_yaml_processed(value: &Value, indent: usize, lines: &mut Vec<String>) {
    let prefix = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            let processed = process_object_fields(map);
            for (display_key, v, formatted) in processed {
                if let Some(fv) = formatted {
                    lines.push(format!(
                        "{}{}: \"{}\"",
                        prefix,
                        yaml_key(&display_key),
                        escape_yaml_str(&fv)
                    ));
                } else {
                    match v {
                        Value::Object(inner) if !inner.is_empty() => {
                            lines.push(format!("{}{}:", prefix, yaml_key(&display_key)));
                            render_yaml_processed(v, indent + 1, lines);
                        }
                        Value::Object(_) => {
                            lines.push(format!("{}{}: {{}}", prefix, yaml_key(&display_key)));
                        }
                        Value::Array(arr) => {
                            if arr.is_empty() {
                                lines.push(format!("{}{}: []", prefix, yaml_key(&display_key)));
                            } else {
                                lines.push(format!("{}{}:", prefix, yaml_key(&display_key)));
                                for item in arr {
                                    if item.is_object() {
                                        lines.push(format!("{}  -", prefix));
                                        render_yaml_processed(item, indent + 2, lines);
                                    } else {
                                        lines.push(format!("{}  - {}", prefix, yaml_scalar(item)));
                                    }
                                }
                            }
                        }
                        _ => {
                            lines.push(format!(
                                "{}{}: {}",
                                prefix,
                                yaml_key(&display_key),
                                yaml_scalar(v)
                            ));
                        }
                    }
                }
            }
        }
        _ => {
            lines.push(format!("{}{}", prefix, yaml_scalar(value)));
        }
    }
}

fn render_yaml_raw(value: &Value, indent: usize, lines: &mut Vec<String>) {
    let prefix = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            for key in sorted_value_keys(map) {
                render_yaml_field_raw(&prefix, &key, &map[&key], indent, lines);
            }
        }
        Value::Array(arr) => {
            render_yaml_array_raw(arr, indent, lines);
        }
        _ => {
            lines.push(format!("{}{}", prefix, yaml_scalar(value)));
        }
    }
}

fn render_yaml_field_raw(
    prefix: &str,
    key: &str,
    value: &Value,
    indent: usize,
    lines: &mut Vec<String>,
) {
    match value {
        Value::Object(inner) if !inner.is_empty() => {
            lines.push(format!("{}{}:", prefix, yaml_key(key)));
            render_yaml_raw(value, indent + 1, lines);
        }
        Value::Object(_) => {
            lines.push(format!("{}{}: {{}}", prefix, yaml_key(key)));
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                lines.push(format!("{}{}: []", prefix, yaml_key(key)));
            } else {
                lines.push(format!("{}{}:", prefix, yaml_key(key)));
                render_yaml_array_raw(arr, indent + 1, lines);
            }
        }
        _ => {
            lines.push(format!(
                "{}{}: {}",
                prefix,
                yaml_key(key),
                yaml_scalar(value)
            ));
        }
    }
}

fn render_yaml_array_raw(arr: &[Value], indent: usize, lines: &mut Vec<String>) {
    let prefix = "  ".repeat(indent);
    for item in arr {
        match item {
            Value::Object(inner) if !inner.is_empty() => {
                lines.push(format!("{}-", prefix));
                render_yaml_raw(item, indent + 1, lines);
            }
            Value::Array(nested) if !nested.is_empty() => {
                lines.push(format!("{}-", prefix));
                render_yaml_array_raw(nested, indent + 1, lines);
            }
            Value::Object(_) => {
                lines.push(format!("{}- {{}}", prefix));
            }
            Value::Array(_) => {
                lines.push(format!("{}- []", prefix));
            }
            _ => {
                lines.push(format!("{}- {}", prefix, yaml_scalar(item)));
            }
        }
    }
}

fn escape_yaml_str(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('\x0c', "\\f")
        .replace('\x0b', "\\v")
}

fn yaml_key(key: &str) -> String {
    if is_safe_key(key) {
        key.to_string()
    } else {
        format!("\"{}\"", escape_yaml_str(key))
    }
}

fn quote_logfmt_key(key: &str) -> String {
    if is_safe_key(key) {
        key.to_string()
    } else {
        quote_logfmt_value(key)
    }
}

fn is_safe_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

fn yaml_scalar(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{}\"", escape_yaml_str(s)),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(n),
        Value::Object(_) | Value::Array(_) => {
            format!("\"{}\"", escape_yaml_str(&canonical_json(value)))
        }
    }
}

// ═══════════════════════════════════════════
// Plain Rendering (logfmt)
// ═══════════════════════════════════════════

fn collect_plain_pairs(value: &Value, prefix: &str, pairs: &mut Vec<(String, String)>) {
    if let Value::Object(map) = value {
        let processed = process_object_fields(map);
        for (display_key, v, formatted) in processed {
            let full_key = if prefix.is_empty() {
                display_key
            } else {
                format!("{}.{}", prefix, display_key)
            };
            if let Some(fv) = formatted {
                pairs.push((full_key, fv));
            } else {
                match v {
                    Value::Object(_) => collect_plain_pairs(v, &full_key, pairs),
                    Value::Array(arr) => {
                        let joined = arr.iter().map(plain_scalar).collect::<Vec<_>>().join(",");
                        pairs.push((full_key, joined));
                    }
                    Value::Null => pairs.push((full_key, String::new())),
                    _ => pairs.push((full_key, plain_scalar(v))),
                }
            }
        }
    }
}

fn collect_plain_pairs_raw(value: &Value, prefix: &str, pairs: &mut Vec<(String, String)>) {
    if let Value::Object(map) = value {
        for key in sorted_value_keys(map) {
            let v = &map[&key];
            let full_key = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{}.{}", prefix, key)
            };
            match v {
                Value::Object(_) => collect_plain_pairs_raw(v, &full_key, pairs),
                Value::Array(arr) => {
                    let joined = arr.iter().map(plain_scalar).collect::<Vec<_>>().join(",");
                    pairs.push((full_key, joined));
                }
                Value::Null => pairs.push((full_key, String::new())),
                _ => pairs.push((full_key, plain_scalar(v))),
            }
        }
    }
}

fn plain_scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(n),
        Value::Object(_) | Value::Array(_) => canonical_json(value),
    }
}

fn quote_logfmt_value(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if !value
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '=' | '"' | '\\'))
    {
        return value.to_string();
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('\x0c', "\\f")
        .replace('\x0b', "\\v");
    format!("\"{}\"", escaped)
}

fn canonical_json(value: &Value) -> String {
    serde_json::to_string(&sort_json_value(value))
        .unwrap_or_else(|_| "<unsupported:json>".to_string())
}

fn sort_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for key in sorted_value_keys(map) {
                if let Some(v) = map.get(&key) {
                    out.insert(key, sort_json_value(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sort_json_value).collect()),
        _ => value.clone(),
    }
}

fn sorted_value_keys(map: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = map.keys().cloned().collect();
    keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    keys
}

#[cfg(test)]
mod tests;
