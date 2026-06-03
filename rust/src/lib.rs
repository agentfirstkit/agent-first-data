//! Agent-First Data (AFDATA) output formatting and protocol templates.
//!
//! 23 public APIs and 5 types (+ 2 optional help renderers):
//! - 3 protocol builders: [`build_json_ok`], [`build_json_error`], [`build_json`]
//! - 3 value-copy redactors: [`redacted_value`], [`redacted_value_with`], [`redacted_value_with_options`]
//! - 7 output formatters: [`output_json`], [`output_json_with`], [`output_json_with_options`],
//!   [`output_yaml`], [`output_yaml_with_options`], [`output_plain`], [`output_plain_with_options`]
//! - 2 in-place value redactors: [`internal_redact_secrets`], [`internal_redact_secrets_with_options`]
//!   (these redact `_secret` and `_url` fields in a JSON value)
//! - 2 URL-string redactors: [`redact_url_secrets`], [`redact_url_secrets_with_options`]
//!   (operate on one URL string; the value redactors above apply these to `_url` fields)
//! - 1 parse utility: [`parse_size`]
//! - 5 CLI helpers: [`cli_parse_output`], [`cli_parse_log_filters`], [`cli_output`],
//!   [`cli_output_with_options`], [`build_cli_error`]
//! - 5 types: [`OutputFormat`], [`RedactionPolicy`], [`RedactionOptions`],
//!   [`OutputStyle`], [`OutputOptions`]
//! - (feature `cli-help`): [`cli_render_help`] — recursive plain-text help for clap commands
//! - (feature `cli-help-markdown`): [`cli_render_help_markdown`] — recursive Markdown help
//! - (feature `skill-admin`): [`skill::run_skill_admin`] — install/uninstall/status a spore's
//!   embedded Agent Skill across Codex, Claude Code, and opencode; returns a typed
//!   [`skill::SkillReport`]

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
    /// Replace every `_secret` subtree with `"***"`.
    RedactionStrict,
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
        .map(|(k, v)| format!("{}={}", k, quote_logfmt_value(&v)))
        .collect::<Vec<_>>()
        .join(" ")
}

// ═══════════════════════════════════════════
// Public API: Redaction & Utility
// ═══════════════════════════════════════════

/// Redact `_secret` fields in-place.
pub fn internal_redact_secrets(value: &mut Value) {
    redact_secrets(value);
}

/// Redact secret fields in-place using configurable redaction options.
pub fn internal_redact_secrets_with_options(
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
    if num_str.is_empty() {
        return None;
    }
    if let Ok(n) = num_str.parse::<u64>() {
        return n.checked_mul(mult);
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
    if result >= u64::MAX as f64 {
        return None;
    }
    Some(result as u64)
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

/// Build a standard CLI parse error value.
///
/// Use when `Cli::try_parse()` fails or a flag value is invalid.
/// Print with [`output_json`] and exit with code 2.
///
/// ```
/// let err = agent_first_data::build_cli_error("--output: invalid value 'xml'", None);
/// assert_eq!(err["code"], "error");
/// assert_eq!(err["error_code"], "invalid_request");
/// assert_eq!(err["retryable"], false);
/// ```
pub fn build_cli_error(message: &str, hint: Option<&str>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("code".to_string(), Value::String("error".to_string()));
    obj.insert(
        "error_code".to_string(),
        Value::String("invalid_request".to_string()),
    );
    obj.insert("error".to_string(), Value::String(message.to_string()));
    if let Some(h) = hint {
        obj.insert("hint".to_string(), Value::String(h.to_string()));
    }
    obj.insert("retryable".to_string(), Value::Bool(false));
    obj.insert("trace".to_string(), serde_json::json!({"duration_ms": 0}));
    Value::Object(obj)
}

// ═══════════════════════════════════════════
// Public API: CLI Help Rendering (optional)
// ═══════════════════════════════════════════

/// Render recursive plain-text help for a clap command tree.
///
/// Walks to the subcommand identified by `subcommand_path` (empty = root),
/// then recursively expands all descendant subcommands into a single output.
/// Agents read `--help` once and get the complete CLI interface.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
pub fn cli_render_help(cmd: &clap::Command, subcommand_path: &[&str]) -> String {
    let target = walk_to_subcommand(cmd, subcommand_path);
    let mut buf = String::new();
    render_help_recursive(target, &[], &mut buf, true);
    buf
}

/// Render recursive Markdown help for a clap command tree.
///
/// Same tree walk as [`cli_render_help`], but outputs Markdown suitable for
/// documentation generation (`myapp --help-markdown > docs/cli.md`).
///
/// Requires the `cli-help-markdown` feature.
#[cfg(feature = "cli-help-markdown")]
pub fn cli_render_help_markdown(cmd: &clap::Command, subcommand_path: &[&str]) -> String {
    let target = walk_to_subcommand(cmd, subcommand_path);
    let md = clap_markdown::help_markdown_command(target);
    // Strip the clap-markdown footer (<hr/> + <small>...</small>)
    md.rfind("\n<hr/>")
        .map_or(md.clone(), |pos| md[..pos].to_string())
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
fn render_help_recursive(
    cmd: &clap::Command,
    parent_path: &[&str],
    buf: &mut String,
    is_root: bool,
) {
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

    // Render clap's built-in help for this command (usage, args, options)
    let styled = cmd.clone().render_long_help();
    let help_text = styled.to_string();

    // In root command, insert --help-markdown after the "Print help" line
    if is_root {
        let mut found_help = false;
        for line in help_text.lines() {
            let _ = writeln!(buf, "{line}");
            if line.trim_start().starts_with("-h, --help") {
                found_help = true;
            } else if found_help && line.contains("Print help") {
                let _ = writeln!(buf, "      --help-markdown");
                let _ = writeln!(
                    buf,
                    "          Output help as Markdown (for documentation generation)"
                );
                found_help = false;
            } else {
                found_help = false;
            }
        }
    } else {
        let _ = write!(buf, "{help_text}");
    }

    // Recurse into visible subcommands
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" {
            continue; // skip clap's auto-generated "help" subcommand
        }
        render_help_recursive(sub, &cmd_path, buf, false);
    }
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

fn redact_secrets(value: &mut Value) {
    let context = RedactionContext::default();
    redact_secrets_with_context(value, &context);
}

fn redact_secrets_with_context(value: &mut Value, context: &RedactionContext) {
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if context.is_secret_key(&key) {
                    match map.get(&key) {
                        Some(Value::Object(_)) | Some(Value::Array(_)) => {
                            // Traverse containers, don't replace
                        }
                        _ => {
                            map.insert(key.clone(), Value::String("***".into()));
                            continue;
                        }
                    }
                } else if key_has_url_suffix(&key) {
                    if let Some(Value::String(s)) = map.get_mut(&key) {
                        if let Some(redacted) = redact_url_in_str(s, context) {
                            *s = redacted;
                        }
                        continue;
                    }
                }
                if let Some(v) = map.get_mut(&key) {
                    redact_secrets_with_context(v, context);
                }
            }
        }
        Value::Array(arr) => {
            for v in arr {
                redact_secrets_with_context(v, context);
            }
        }
        _ => {}
    }
}

fn redact_secrets_strict_with_context(value: &mut Value, context: &RedactionContext) {
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if context.is_secret_key(&key) {
                    map.insert(key, Value::String("***".into()));
                } else if key_has_url_suffix(&key) {
                    if let Some(Value::String(s)) = map.get_mut(&key) {
                        if let Some(redacted) = redact_url_in_str(s, context) {
                            *s = redacted;
                        }
                    } else if let Some(v) = map.get_mut(&key) {
                        redact_secrets_strict_with_context(v, context);
                    }
                } else if let Some(v) = map.get_mut(&key) {
                    redact_secrets_strict_with_context(v, context);
                }
            }
        }
        Value::Array(arr) => {
            for v in arr {
                redact_secrets_strict_with_context(v, context);
            }
        }
        _ => {}
    }
}

/// Redact secret components of a single URL string, returning `Some(redacted)`
/// when `s` is a processable URL, or `None` when it is not (so callers can keep
/// the original). Only secret spans change; all other bytes are preserved.
fn redact_url_in_str(s: &str, context: &RedactionContext) -> Option<String> {
    // Fast path + precondition: a single, whitespace-free, scheme-prefixed URL.
    if !s.contains("://") || !is_single_url(s) || url::Url::parse(s).is_err() {
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

/// Replace the userinfo password (`user:pass@`) with `***`, preserving the
/// username. Authority without `@`, or userinfo without `:`, is unchanged.
fn redact_userinfo_password(authority: &str) -> String {
    let Some(at) = authority.find('@') else {
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
        Some(RedactionPolicy::RedactionStrict) => {
            redact_secrets_strict_with_context(value, context)
        }
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
fn try_process_field(key: &str, value: &Value) -> Option<(String, String)> {
    // Group 1: compound timestamp suffixes
    if let Some(stripped) = strip_suffix_ci(key, "_epoch_ms") {
        return value.as_i64().map(|ms| (stripped, format_rfc3339_ms(ms)));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_epoch_s") {
        return value
            .as_i64()
            .map(|s| (stripped, format_rfc3339_ms(s * 1000)));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_epoch_ns") {
        return value
            .as_i64()
            .map(|ns| (stripped, format_rfc3339_ms(ns.div_euclid(1_000_000))));
    }

    // Group 2: compound currency suffixes
    if let Some(stripped) = strip_suffix_ci(key, "_usd_cents") {
        return value
            .as_u64()
            .map(|n| (stripped, format!("${}.{:02}", n / 100, n % 100)));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_eur_cents") {
        return value
            .as_u64()
            .map(|n| (stripped, format!("€{}.{:02}", n / 100, n % 100)));
    }
    if let Some((stripped, code)) = try_strip_generic_cents(key) {
        return value.as_u64().map(|n| {
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
        return value.as_i64().map(|n| (stripped, format_bytes_human(n)));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_percent") {
        return value
            .is_number()
            .then(|| (stripped, format!("{}%", number_str(value))));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_secret") {
        return Some((stripped, "***".to_string()));
    }

    // Group 5: short suffixes (last to avoid false positives)
    if let Some(stripped) = strip_suffix_ci(key, "_btc") {
        return value
            .is_number()
            .then(|| (stripped, format!("{} BTC", number_str(value))));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_jpy") {
        return value
            .as_u64()
            .map(|n| (stripped, format!("¥{}", format_with_commas(n))));
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
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
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
fn format_rfc3339_ms(ms: i64) -> String {
    use chrono::{DateTime, Utc};
    let secs = ms.div_euclid(1000);
    let nanos = (ms.rem_euclid(1000) * 1_000_000) as u32;
    match DateTime::from_timestamp(secs, nanos) {
        Some(dt) => dt
            .with_timezone(&Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        None => ms.to_string(),
    }
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
    if code.is_empty() {
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
                        display_key,
                        escape_yaml_str(&fv)
                    ));
                } else {
                    match v {
                        Value::Object(inner) if !inner.is_empty() => {
                            lines.push(format!("{}{}:", prefix, display_key));
                            render_yaml_processed(v, indent + 1, lines);
                        }
                        Value::Object(_) => {
                            lines.push(format!("{}{}: {{}}", prefix, display_key));
                        }
                        Value::Array(arr) => {
                            if arr.is_empty() {
                                lines.push(format!("{}{}: []", prefix, display_key));
                            } else {
                                lines.push(format!("{}{}:", prefix, display_key));
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
                            lines.push(format!("{}{}: {}", prefix, display_key, yaml_scalar(v)));
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
            for (key, v) in map {
                render_yaml_field_raw(&prefix, key, v, indent, lines);
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
            lines.push(format!("{}{}:", prefix, key));
            render_yaml_raw(value, indent + 1, lines);
        }
        Value::Object(_) => {
            lines.push(format!("{}{}: {{}}", prefix, key));
        }
        Value::Array(arr) => {
            if arr.is_empty() {
                lines.push(format!("{}{}: []", prefix, key));
            } else {
                lines.push(format!("{}{}:", prefix, key));
                render_yaml_array_raw(arr, indent + 1, lines);
            }
        }
        _ => {
            lines.push(format!("{}{}: {}", prefix, key, yaml_scalar(value)));
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
}

fn yaml_scalar(value: &Value) -> String {
    match value {
        Value::String(s) => {
            format!("\"{}\"", escape_yaml_str(s))
        }
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => format!("\"{}\"", other.to_string().replace('"', "\\\"")),
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
        for (key, v) in map {
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
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
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
        .replace('\t', "\\t");
    format!("\"{}\"", escaped)
}

#[cfg(test)]
mod tests;
