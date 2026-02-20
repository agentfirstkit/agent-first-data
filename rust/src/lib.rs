//! Agent-First Data (AFDATA) output formatting and protocol templates.
//!
//! 12 public APIs and 1 type:
//! - 3 protocol builders: [`build_json_ok`], [`build_json_error`], [`build_json`]
//! - 3 output formatters: [`output_json`], [`output_yaml`], [`output_plain`]
//! - 1 redaction utility: [`internal_redact_secrets`]
//! - 1 parse utility: [`parse_size`]
//! - 4 CLI helpers: [`cli_parse_output`], [`cli_parse_log_filters`], [`cli_output`], [`build_cli_error`]
//! - 1 type: [`OutputFormat`]

#[cfg(feature = "tracing")]
pub mod afdata_tracing;

use serde_json::Value;

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

/// Build `{code: "error", error: message, trace?: ...}`.
pub fn build_json_error(message: &str, trace: Option<Value>) -> Value {
    match trace {
        Some(t) => serde_json::json!({"code": "error", "error": message, "trace": t}),
        None => serde_json::json!({"code": "error", "error": message}),
    }
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

/// Format as single-line JSON. Secrets redacted, original keys, raw values.
pub fn output_json(value: &Value) -> String {
    let mut v = value.clone();
    redact_secrets(&mut v);
    serde_json::to_string(&v).unwrap_or_default()
}

/// Format as multi-line YAML. Keys stripped, values formatted, secrets redacted.
pub fn output_yaml(value: &Value) -> String {
    let mut lines = vec!["---".to_string()];
    render_yaml_processed(value, 0, &mut lines);
    lines.join("\n")
}

/// Format as single-line logfmt. Keys stripped, values formatted, secrets redacted.
pub fn output_plain(value: &Value) -> String {
    let mut pairs: Vec<(String, String)> = Vec::new();
    collect_plain_pairs(value, "", &mut pairs);
    pairs.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
    pairs
        .into_iter()
        .map(|(k, v)| {
            if v.contains(' ') {
                format!("{}=\"{}\"", k, v)
            } else {
                format!("{}={}", k, v)
            }
        })
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
    let f: f64 = num_str.parse().ok()?;
    if f < 0.0 || f.is_nan() || f.is_infinite() {
        return None;
    }
    let result = f * mult as f64;
    if result > u64::MAX as f64 {
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

/// Build a standard CLI parse error value.
///
/// Use when `Cli::try_parse()` fails or a flag value is invalid.
/// Print with [`output_json`] and exit with code 2.
///
/// ```
/// let err = agent_first_data::build_cli_error("--output: invalid value 'xml'");
/// assert_eq!(err["code"], "error");
/// assert_eq!(err["error_code"], "invalid_request");
/// assert_eq!(err["retryable"], false);
/// ```
pub fn build_cli_error(message: &str) -> Value {
    serde_json::json!({
        "code": "error",
        "error_code": "invalid_request",
        "error": message,
        "retryable": false,
        "trace": {"duration_ms": 0}
    })
}

// ═══════════════════════════════════════════
// Secret Redaction
// ═══════════════════════════════════════════

fn redact_secrets(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if key.ends_with("_secret") || key.ends_with("_SECRET") {
                    match map.get(&key) {
                        Some(Value::Object(_)) | Some(Value::Array(_)) => {
                            // Traverse containers, don't replace
                        }
                        _ => {
                            map.insert(key.clone(), Value::String("***".into()));
                            continue;
                        }
                    }
                }
                if let Some(v) = map.get_mut(&key) {
                    redact_secrets(v);
                }
            }
        }
        Value::Array(arr) => {
            for v in arr {
                redact_secrets(v);
            }
        }
        _ => {}
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
    let suffix_upper: String = suffix_lower.chars().map(|c| c.to_ascii_uppercase()).collect();
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
        return value.as_i64().map(|s| (stripped, format_rfc3339_ms(s * 1000)));
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
            if counts.get(&stripped).copied().unwrap_or(0) > 1
                && original != stripped.as_str()
            {
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
                                        lines.push(format!(
                                            "{}  - {}",
                                            prefix,
                                            yaml_scalar(item)
                                        ));
                                    }
                                }
                            }
                        }
                        _ => {
                            lines.push(format!(
                                "{}{}: {}",
                                prefix,
                                display_key,
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

fn plain_scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests;
