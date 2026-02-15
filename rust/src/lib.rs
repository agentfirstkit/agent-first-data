//! Agent-First Data (AFD) output formatting and protocol templates.
//!
//! Implements the AFD output convention. JSON is the canonical lossless format.
//! YAML preserves structure with quoted strings. Plain applies suffix-driven
//! formatting for human readability.
//!
//! ```text
//! --output json|yaml|plain
//! ```

use serde_json::Value;

/// Output format for CLI and API responses.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum OutputFormat {
    #[default]
    Json,
    Yaml,
    Plain,
}

impl OutputFormat {
    /// Format a JSON value as a single compact line (JSONL-compatible).
    pub fn format(&self, value: &Value) -> String {
        match self {
            Self::Json => serde_json::to_string(value).unwrap_or_default(),
            Self::Yaml => to_yaml(value),
            Self::Plain => to_plain(value),
        }
    }

    /// Format a JSON value with pretty printing (JSON only; yaml/plain unchanged).
    pub fn format_pretty(&self, value: &Value) -> String {
        match self {
            Self::Json => serde_json::to_string_pretty(value).unwrap_or_default(),
            Self::Yaml => to_yaml(value),
            Self::Plain => to_plain(value),
        }
    }
}

// ═══════════════════════════════════════════
// YAML
// ═══════════════════════════════════════════

/// Convert a JSON Value into a YAML document.
///
/// Strings are always quoted to avoid YAML pitfalls (`no` → `false`, `3.0` → float).
/// Values are preserved as-is — no suffix-driven transformation.
/// Starts with `---` for multi-document streaming compatibility.
pub fn to_yaml(value: &Value) -> String {
    let mut lines = vec!["---".to_string()];
    render_yaml(value, 0, &mut lines);
    lines.join("\n")
}

fn render_yaml(value: &Value, indent: usize, lines: &mut Vec<String>) {
    let prefix = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            for (k, v) in jcs_sorted(map) {
                match v {
                    Value::Object(inner) if !inner.is_empty() => {
                        lines.push(format!("{}{}:", prefix, k));
                        render_yaml(v, indent + 1, lines);
                    }
                    Value::Object(_) => {
                        lines.push(format!("{}{}: {{}}", prefix, k));
                    }
                    Value::Array(arr) => {
                        if arr.is_empty() {
                            lines.push(format!("{}{}: []", prefix, k));
                        } else {
                            lines.push(format!("{}{}:", prefix, k));
                            for item in arr {
                                if item.is_object() {
                                    lines.push(format!("{}  -", prefix));
                                    render_yaml(item, indent + 2, lines);
                                } else {
                                    lines.push(format!("{}  - {}", prefix, yaml_scalar(item)));
                                }
                            }
                        }
                    }
                    _ => {
                        lines.push(format!("{}{}: {}", prefix, k, yaml_scalar(v)));
                    }
                }
            }
        }
        _ => {
            lines.push(format!("{}{}", prefix, yaml_scalar(value)));
        }
    }
}

/// Sort map entries by UTF-16 code unit order (JCS, RFC 8785).
fn jcs_sorted(map: &serde_json::Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
    entries
}

fn yaml_scalar(value: &Value) -> String {
    match value {
        Value::String(s) => {
            let escaped = s
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            format!("\"{}\"", escaped)
        }
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => format!("\"{}\"", other.to_string().replace('"', "\\\"")),
    }
}

// ═══════════════════════════════════════════
// Plain
// ═══════════════════════════════════════════

/// Convert a JSON Value into human-readable plain text.
///
/// Applies agent-first-data suffix-driven formatting:
/// - `_ms` → append `ms`, or convert to seconds if ≥ 1000
/// - `_bytes` → human-readable (`446.1KB`)
/// - `_epoch_ms` → RFC 3339
/// - `_secret` → `***`
/// - Currency suffixes → formatted amounts
pub fn to_plain(value: &Value) -> String {
    let mut lines = Vec::new();
    render_plain(value, 0, &mut lines);
    lines.join("\n")
}

fn render_plain(value: &Value, indent: usize, lines: &mut Vec<String>) {
    let prefix = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            for (k, v) in jcs_sorted(map) {
                match v {
                    Value::Object(_) => {
                        lines.push(format!("{}{}:", prefix, k));
                        render_plain(v, indent + 1, lines);
                    }
                    Value::Array(arr) => {
                        if arr.is_empty() {
                            lines.push(format!("{}{}: []", prefix, k));
                        } else if arr.iter().all(|v| !v.is_object() && !v.is_array()) {
                            lines.push(format!("{}{}:", prefix, k));
                            for item in arr {
                                lines.push(format!("{}  - {}", prefix, plain_scalar(item)));
                            }
                        } else {
                            lines.push(format!("{}{}:", prefix, k));
                            for item in arr {
                                if item.is_object() {
                                    lines.push(format!("{}  -", prefix));
                                    render_plain(item, indent + 2, lines);
                                } else {
                                    lines.push(format!("{}  - {}", prefix, plain_scalar(item)));
                                }
                            }
                        }
                    }
                    _ => {
                        lines.push(format!("{}{}: {}", prefix, k, format_plain_field(k, v)));
                    }
                }
            }
        }
        _ => {
            lines.push(format!("{}{}", prefix, plain_scalar(value)));
        }
    }
}

/// Format a scalar value for plain output, applying suffix-driven rules.
///
/// Suffix priority (most specific first):
/// 1. `_secret` → `***`
/// 2. `_epoch_ms` / `_epoch_s` / `_epoch_ns` → RFC 3339
/// 3. `_rfc3339` → pass through
/// 4. `_bytes` → human-readable size
/// 5. Currency: `_msats`, `_sats`, `_btc`, `_usd_cents`, `_eur_cents`, `_cents`, `_jpy`
/// 6. Duration: `_minutes`, `_hours`, `_days`, `_ms`, `_ns`, `_us`, `_s`
fn format_plain_field(key: &str, value: &Value) -> String {
    let lower = key.to_ascii_lowercase();

    // Secret — always redact
    if lower.ends_with("_secret") {
        return "***".to_string();
    }

    // Timestamps → RFC 3339
    if lower.ends_with("_epoch_ms") {
        if let Some(ms) = value.as_i64() {
            return format_rfc3339_ms(ms);
        }
    }
    if lower.ends_with("_epoch_s") {
        if let Some(s) = value.as_i64() {
            return format_rfc3339_ms(s * 1000);
        }
    }
    if lower.ends_with("_epoch_ns") {
        if let Some(ns) = value.as_i64() {
            return format_rfc3339_ms(ns.div_euclid(1_000_000));
        }
    }
    if lower.ends_with("_rfc3339") {
        return plain_scalar(value);
    }

    // Size
    if lower.ends_with("_bytes") {
        if let Some(n) = value.as_i64() {
            return format_bytes_human(n);
        }
    }

    // Percentage
    if lower.ends_with("_percent") {
        if value.is_number() {
            return format!("{}%", plain_scalar(value));
        }
    }

    // Currency — Bitcoin
    if lower.ends_with("_msats") {
        if value.is_number() {
            return format!("{}msats", plain_scalar(value));
        }
    }
    if lower.ends_with("_sats") {
        if value.is_number() {
            return format!("{}sats", plain_scalar(value));
        }
    }
    if lower.ends_with("_btc") {
        if value.is_number() {
            return format!("{} BTC", plain_scalar(value));
        }
    }

    // Currency — Fiat with symbol
    if lower.ends_with("_usd_cents") {
        if let Some(n) = value.as_u64() {
            return format!("${}.{:02}", n / 100, n % 100);
        }
    }
    if lower.ends_with("_eur_cents") {
        if let Some(n) = value.as_u64() {
            return format!("€{}.{:02}", n / 100, n % 100);
        }
    }
    if lower.ends_with("_jpy") {
        if let Some(n) = value.as_u64() {
            return format!("¥{}", format_with_commas(n));
        }
    }
    // Currency — Generic _{code}_cents
    if lower.ends_with("_cents") {
        if let Some(code) = extract_currency_code(&lower) {
            if let Some(n) = value.as_u64() {
                return format!("{}.{:02} {}", n / 100, n % 100, code.to_uppercase());
            }
        }
    }

    // Duration — long units (check before short)
    if lower.ends_with("_minutes") {
        if value.is_number() {
            return format!("{} minutes", plain_scalar(value));
        }
    }
    if lower.ends_with("_hours") {
        if value.is_number() {
            return format!("{} hours", plain_scalar(value));
        }
    }
    if lower.ends_with("_days") {
        if value.is_number() {
            return format!("{} days", plain_scalar(value));
        }
    }

    // Duration — ms (with ≥1000 → seconds conversion)
    if lower.ends_with("_ms") && !lower.ends_with("_epoch_ms") {
        if let Some(n) = value.as_u64() {
            return if n >= 1000 {
                format!("{:.2}s", n as f64 / 1000.0)
            } else {
                format!("{}ms", n)
            };
        }
        if let Some(n) = value.as_f64() {
            return if n >= 1000.0 {
                format!("{:.2}s", n / 1000.0)
            } else {
                format!("{}ms", plain_scalar(value))
            };
        }
    }

    // Duration — ns, us, s
    if lower.ends_with("_ns") && !lower.ends_with("_epoch_ns") {
        if value.is_number() {
            return format!("{}ns", plain_scalar(value));
        }
    }
    if lower.ends_with("_us") {
        if value.is_number() {
            return format!("{}μs", plain_scalar(value));
        }
    }
    if lower.ends_with("_s") && !lower.ends_with("_epoch_s") {
        if value.is_number() {
            return format!("{}s", plain_scalar(value));
        }
    }

    // Default — no transformation
    plain_scalar(value)
}

/// Plain scalar: no quotes, raw value.
fn plain_scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

// ═══════════════════════════════════════════
// Secret redaction
// ═══════════════════════════════════════════

/// Walk a JSON Value tree and redact any field ending in `_secret`.
///
/// Applies the AFD convention: `_secret` suffix signals sensitive data.
/// String values are replaced with `"***"`. Call this before serializing
/// config or log output in any format (JSON, YAML, plain).
pub fn redact_secrets(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let secret_keys: Vec<String> = map
                .keys()
                .filter(|k| k.to_ascii_lowercase().ends_with("_secret"))
                .cloned()
                .collect();
            for key in secret_keys {
                if let Some(Value::String(s)) = map.get_mut(&key) {
                    *s = "***".into();
                }
            }
            for v in map.values_mut() {
                redact_secrets(v);
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
// AFD Protocol templates
// ═══════════════════════════════════════════

/// Build `{code: "ok", result: ...}`.
pub fn ok(result: Value) -> Value {
    serde_json::json!({"code": "ok", "result": result})
}

/// Build `{code: "ok", result: ..., trace: ...}`.
pub fn ok_trace(result: Value, trace: Value) -> Value {
    serde_json::json!({"code": "ok", "result": result, "trace": trace})
}

/// Build `{code: "error", error: "message"}`.
pub fn error(message: &str) -> Value {
    serde_json::json!({"code": "error", "error": message})
}

/// Build `{code: "error", error: "message", trace: ...}`.
pub fn error_trace(message: &str, trace: Value) -> Value {
    serde_json::json!({"code": "error", "error": message, "trace": trace})
}

/// Build `{code: "startup", config: ..., args: ..., env: ...}`.
pub fn startup(config: Value, args: Value, env: Value) -> Value {
    serde_json::json!({"code": "startup", "config": config, "args": args, "env": env})
}

/// Build `{code: "<custom>", ...fields}` — tool-defined status line.
pub fn status(code: &str, fields: Value) -> Value {
    let mut obj = match fields {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    obj.insert("code".to_string(), Value::String(code.to_string()));
    Value::Object(obj)
}

// ═══════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════

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

/// Extract currency code from a `_{code}_cents` suffix.
/// e.g., "fare_thb_cents" → Some("thb")
fn extract_currency_code(key: &str) -> Option<&str> {
    let without_cents = key.strip_suffix("_cents")?;
    let last_underscore = without_cents.rfind('_')?;
    Some(&without_cents[last_underscore + 1..])
}

// ═══════════════════════════════════════════
// Size parsing
// ═══════════════════════════════════════════

/// Parse a human-readable size string into bytes.
///
/// Accepts `_size` config values: bare number, or number followed by unit letter
/// (`B`, `K`, `M`, `G`, `T`). Case-insensitive. Trims whitespace.
/// Returns `None` for invalid or negative input.
///
/// ```text
/// "10M"  → 10_485_760
/// "1.5K" → 1_536
/// "512B" → 512
/// "1024" → 1_024
/// ```
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
// Tests
// ═══════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../spec/fixtures");

    fn load_fixture(name: &str) -> Value {
        let path = format!("{}/{}", FIXTURES_DIR, name);
        let data = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", path, e));
        serde_json::from_str(&data)
            .unwrap_or_else(|e| panic!("failed to parse {}: {}", path, e))
    }

    #[test]
    fn test_plain_fixtures() {
        let cases = load_fixture("plain.json");
        for case in cases.as_array().expect("plain.json must be an array") {
            let name = case["name"].as_str().expect("missing name");
            let input = &case["input"];
            let plain = to_plain(input);
            for expected in case["contains"].as_array().expect("missing contains") {
                let s = expected.as_str().expect("contains must be strings");
                assert!(plain.contains(s), "[plain/{name}] expected {s:?} in {plain:?}");
            }
            if let Some(not_contains) = case.get("not_contains") {
                for nc in not_contains.as_array().expect("not_contains must be array") {
                    let s = nc.as_str().expect("not_contains must be strings");
                    assert!(!plain.contains(s), "[plain/{name}] unexpected {s:?} in {plain:?}");
                }
            }
        }
    }

    #[test]
    fn test_yaml_fixtures() {
        let cases = load_fixture("yaml.json");
        for case in cases.as_array().expect("yaml.json must be an array") {
            let name = case["name"].as_str().expect("missing name");
            let input = &case["input"];
            let yaml = to_yaml(input);
            if let Some(prefix) = case.get("starts_with") {
                let s = prefix.as_str().expect("starts_with must be string");
                assert!(yaml.starts_with(s), "[yaml/{name}] expected starts_with {s:?} in {yaml:?}");
            }
            if let Some(contains) = case.get("contains") {
                for expected in contains.as_array().expect("contains must be array") {
                    let s = expected.as_str().expect("contains must be strings");
                    assert!(yaml.contains(s), "[yaml/{name}] expected {s:?} in {yaml:?}");
                }
            }
        }
    }

    #[test]
    fn test_redact_fixtures() {
        let cases = load_fixture("redact.json");
        for case in cases.as_array().expect("redact.json must be an array") {
            let name = case["name"].as_str().expect("missing name");
            let mut input = case["input"].clone();
            let expected = &case["expected"];
            redact_secrets(&mut input);
            assert_eq!(&input, expected, "[redact/{name}]");
        }
    }

    #[test]
    fn test_protocol_fixtures() {
        let cases = load_fixture("protocol.json");
        for case in cases.as_array().expect("protocol.json must be an array") {
            let name = case["name"].as_str().expect("missing name");
            let typ = case["type"].as_str().expect("missing type");
            let args = &case["args"];
            let result = match typ {
                "ok" => ok(args["result"].clone()),
                "ok_trace" => ok_trace(args["result"].clone(), args["trace"].clone()),
                "error" => error(args["message"].as_str().expect("missing message")),
                "error_trace" => error_trace(
                    args["message"].as_str().expect("missing message"),
                    args["trace"].clone(),
                ),
                "startup" => startup(
                    args["config"].clone(),
                    args["args"].clone(),
                    args["env"].clone(),
                ),
                "status" => {
                    let code = args["code"].as_str().expect("missing code");
                    let fields = args["fields"].clone();
                    status(code, fields)
                }
                other => panic!("unknown protocol type: {other}"),
            };
            if let Some(expected) = case.get("expected") {
                assert_eq!(&result, expected, "[protocol/{name}]");
            }
            if let Some(expected_contains) = case.get("expected_contains") {
                let ec = expected_contains.as_object().expect("expected_contains must be object");
                let ro = result.as_object().expect("result must be object");
                for (k, v) in ec {
                    assert_eq!(ro.get(k).unwrap_or(&Value::Null), v, "[protocol/{name}] key {k}");
                }
            }
        }
    }

    #[test]
    fn test_exact_fixtures() {
        let cases = load_fixture("exact.json");
        for case in cases.as_array().expect("exact.json must be an array") {
            let name = case["name"].as_str().expect("missing name");
            let format = case["format"].as_str().expect("missing format");
            let input = &case["input"];
            let expected = case["expected"].as_str().expect("missing expected");
            let got = match format {
                "plain" => to_plain(input),
                "yaml" => to_yaml(input),
                other => panic!("unknown format: {other}"),
            };
            assert_eq!(got, expected, "[exact/{name}]");
        }
    }

    #[test]
    fn test_helper_fixtures() {
        let cases = load_fixture("helpers.json");
        for case in cases.as_array().expect("helpers.json must be an array") {
            let name = case["name"].as_str().expect("missing name");
            let test_cases = case["cases"].as_array().expect("missing cases");
            match name {
                "format_bytes_human" => {
                    for tc in test_cases {
                        let arr = tc.as_array().expect("case must be [input, expected]");
                        let input = arr[0].as_i64().expect("input must be i64");
                        let expected = arr[1].as_str().expect("expected must be string");
                        assert_eq!(format_bytes_human(input), expected, "[helpers/format_bytes_human({input})]");
                    }
                }
                "format_with_commas" => {
                    for tc in test_cases {
                        let arr = tc.as_array().expect("case must be [input, expected]");
                        let input = arr[0].as_u64().expect("input must be u64");
                        let expected = arr[1].as_str().expect("expected must be string");
                        assert_eq!(format_with_commas(input), expected, "[helpers/format_with_commas({input})]");
                    }
                }
                "extract_currency_code" => {
                    for tc in test_cases {
                        let arr = tc.as_array().expect("case must be [input, expected]");
                        let input = arr[0].as_str().expect("input must be string");
                        let expected = if arr[1].is_null() { None } else { arr[1].as_str() };
                        assert_eq!(extract_currency_code(input), expected, "[helpers/extract_currency_code({input})]");
                    }
                }
                "parse_size" => {
                    for tc in test_cases {
                        let arr = tc.as_array().expect("case must be [input, expected]");
                        let input = arr[0].as_str().expect("input must be string");
                        let expected = if arr[1].is_null() { None } else { arr[1].as_u64() };
                        assert_eq!(parse_size(input), expected, "[helpers/parse_size({input:?})]");
                    }
                }
                other => panic!("unknown helper: {other}"),
            }
        }
    }
}
