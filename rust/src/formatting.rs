use crate::cli::OutputFormat;
use crate::redaction::{OutputOptions, PlainStyle};
use serde_json::Value;

/// Render a value as a string in the given format with the given options.
///
/// The single `value × format × options → String` entry point. JSON and YAML
/// are structure-preserving and ignore [`PlainStyle`]; plain honors it. All
/// three redact through `options.redaction` before rendering.
pub fn render(value: &Value, format: OutputFormat, options: &OutputOptions) -> String {
    match format {
        OutputFormat::Json => serialize_json_output(&options.redaction.value(value)),
        OutputFormat::Yaml => render_yaml(value, options),
        OutputFormat::Plain => render_plain(value, options),
    }
}

pub(crate) fn serialize_json_output(value: &Value) -> String {
    match serde_json::to_string(value) {
        Ok(s) => s,
        Err(err) => serde_json::json!({
            "error": "output_json_failed",
            "detail": err.to_string(),
        })
        .to_string(),
    }
}

/// Format as multi-line YAML with the given output options.
///
/// YAML output ignores [`PlainStyle`] and always preserves original keys and values after
/// redaction, the same structure-preserving semantics as JSON.
pub(crate) fn render_yaml(value: &Value, output_options: &OutputOptions) -> String {
    let mut lines = vec!["---".to_string()];
    let v = output_options.redaction.value(value);
    render_yaml_raw(&v, 0, &mut lines);
    lines.join("\n")
}

/// Format as single-line logfmt with the given output options.
pub(crate) fn render_plain(value: &Value, output_options: &OutputOptions) -> String {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let v = output_options.redaction.value(value);
    match output_options.style {
        PlainStyle::Readable => collect_plain_pairs(&v, "", &mut pairs),
        PlainStyle::Raw => collect_plain_pairs_raw(&v, "", &mut pairs),
    }
    pairs.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
    pairs
        .into_iter()
        .map(|(k, v)| format!("{}={}", quote_logfmt_key(&k), quote_logfmt_value(&v)))
        .collect::<Vec<_>>()
        .join(" ")
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

/// Extract currency code from `_{code}_micro` / `_{CODE}_MICRO` pattern.
fn try_strip_generic_micro(key: &str) -> Option<(String, String)> {
    let code = extract_currency_code_micro(key)?;
    let suffix_len = code.len() + "_micro".len() + 1; // _{code}_micro
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

fn integer_text(value: &Value) -> Option<String> {
    match value {
        Value::Number(_) => as_int(value).map(|n| n.to_string()),
        Value::String(s) if is_decimal_integer_string(s) => Some(s.clone()),
        _ => None,
    }
}

fn is_decimal_integer_string(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

fn epoch_ns_to_ms(value: &Value) -> Option<i64> {
    let ns = match value {
        Value::Number(_) => i128::from(as_int(value)?),
        Value::String(s) if is_decimal_integer_string(s) => s.parse::<i128>().ok()?,
        _ => return None,
    };
    ns.div_euclid(1_000_000).try_into().ok()
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
        return epoch_ns_to_ms(value)
            .and_then(|ms| format_rfc3339_ms(ms).map(|formatted| (stripped, formatted)));
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
    if let Some((stripped, code)) = try_strip_generic_micro(key) {
        return as_uint(value).map(|n| {
            (
                stripped,
                format!(
                    "{}.{:06} {}",
                    n / 1_000_000,
                    n % 1_000_000,
                    code.to_uppercase()
                ),
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
        return integer_text(value).map(|n| (stripped, format!("{n}msats")));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_sats") {
        return integer_text(value).map(|n| (stripped, format!("{n}sats")));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_bytes") {
        return as_uint(value).map(|n| (stripped, format_bytes_human(n)));
    }
    if let Some(stripped) = strip_suffix_ci(key, "_percent") {
        return value
            .is_number()
            .then(|| (stripped, format!("{}%", number_str(value))));
    }
    // Group 5: short suffixes (last to avoid false positives)
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
    if n.is_f64()
        && let Some(f) = n.as_f64()
        && f.is_finite()
        && f.fract() == 0.0
        && f.abs() < 1e21
    {
        return format!("{f:.0}");
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

/// Format non-negative bytes as human-readable binary size.
pub(crate) fn format_bytes_human(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    const TIB: f64 = GIB * 1024.0;

    let b = bytes as f64;
    if b >= TIB {
        format!("{:.1}TiB", b / TIB)
    } else if b >= GIB {
        format!("{:.1}GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1}MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1}KiB", b / KIB)
    } else {
        format!("{bytes}B")
    }
}

/// Format a number with thousands separators.
pub(crate) fn format_with_commas(n: u64) -> String {
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
pub(crate) fn extract_currency_code(key: &str) -> Option<&str> {
    let without_cents = key
        .strip_suffix("_cents")
        .or_else(|| key.strip_suffix("_CENTS"))?;
    extract_currency_code_from_stem(without_cents)
}

/// Extract currency code from a `_{code}_micro` / `_{CODE}_MICRO` suffix.
fn extract_currency_code_micro(key: &str) -> Option<&str> {
    let without_micro = key
        .strip_suffix("_micro")
        .or_else(|| key.strip_suffix("_MICRO"))?;
    extract_currency_code_from_stem(without_micro)
}

fn extract_currency_code_from_stem(stem: &str) -> Option<&str> {
    let last_underscore = stem.rfind('_')?;
    let code = &stem[last_underscore + 1..];
    if code.is_empty()
        || !(3..=4).contains(&code.len())
        || !code.bytes().all(|b| b.is_ascii_alphabetic())
    {
        return None;
    }
    Some(code)
}

// ═══════════════════════════════════════════
// YAML Rendering (structure-preserving)
// ═══════════════════════════════════════════

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
