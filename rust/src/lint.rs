//! In-process AFDATA naming and serialized-output checks.

use crate::{
    ProtocolViolation, is_valid_bcp47, is_valid_rfc3339, is_valid_rfc3339_date,
    is_valid_rfc3339_time, normalize_utc_offset, validate_protocol_event,
};
use serde::Serialize;
use serde_json::{Number, Value};
use std::fmt;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Finding severity, ordered from advisory to failing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LintSeverity {
    Warning,
    Error,
}

impl LintSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for LintSeverity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One deterministic AFDATA lint finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LintFinding {
    pub rule_id: String,
    pub severity: LintSeverity,
    pub pointer: String,
    pub message: String,
}

impl LintFinding {
    pub fn error(rule_id: &str, pointer: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.to_string(),
            severity: LintSeverity::Error,
            pointer: pointer.into(),
            message: message.into(),
        }
    }

    pub fn warning(rule_id: &str, pointer: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.to_string(),
            severity: LintSeverity::Warning,
            pointer: pointer.into(),
            message: message.into(),
        }
    }

    pub const fn is_error(&self) -> bool {
        matches!(self.severity, LintSeverity::Error)
    }

    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "rule_id": self.rule_id,
            "severity": self.severity,
            "pointer": self.pointer,
            "message": self.message,
        })
    }
}

/// Controls which deterministic findings are returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LintOptions {
    minimum_severity: LintSeverity,
}

impl LintOptions {
    /// Include warnings and errors.
    pub const fn new() -> Self {
        Self {
            minimum_severity: LintSeverity::Warning,
        }
    }

    /// Return only findings at or above `minimum_severity`.
    pub const fn minimum_severity(mut self, minimum_severity: LintSeverity) -> Self {
        self.minimum_severity = minimum_severity;
        self
    }

    pub const fn errors_only() -> Self {
        Self::new().minimum_severity(LintSeverity::Error)
    }
}

impl Default for LintOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Lint an actual serialized JSON value in-process.
pub fn lint_value(value: &Value, options: LintOptions) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    lint_value_at(value, "", &mut findings);
    findings.retain(|finding| finding.severity >= options.minimum_severity);
    findings
}

/// Assert the recommended strict protocol profile without spawning the CLI.
pub fn assert_strict_event(value: &Value) -> Result<(), ProtocolViolation> {
    validate_protocol_event(value, true)
}

/// Return every default lint finding as `Err`; useful in unit tests.
pub fn assert_no_lint_findings(value: &Value) -> Result<(), Vec<LintFinding>> {
    assert_no_lint_findings_with_options(value, LintOptions::default())
}

/// As [`assert_no_lint_findings`], with explicit severity filtering.
pub fn assert_no_lint_findings_with_options(
    value: &Value,
    options: LintOptions,
) -> Result<(), Vec<LintFinding>> {
    let findings = lint_value(value, options);
    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings)
    }
}

/// Failure from [`assert_redaction_canary_absent`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RedactionCanaryError {
    EmptyCanary,
    Exposed,
}

impl fmt::Display for RedactionCanaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyCanary => formatter.write_str("redaction canary must not be empty"),
            Self::Exposed => formatter.write_str("redaction canary remains in serialized output"),
        }
    }
}

impl std::error::Error for RedactionCanaryError {}

/// Verify that a unique test canary is absent from final serialized output.
///
/// Pass the string returned by [`crate::render`] or an HTTP/body serializer,
/// not the pre-redaction input value.
///
/// Every renderer escapes a string before it reaches the stream, so a canary
/// can be present verbatim yet unfindable by a raw substring search: a PEM key
/// carries newlines, a password may carry `"`, a Windows path carries `\`, and
/// a canary inside a `_url` value is percent-encoded. This checks the raw text
/// *and* a decoded copy of it, so the escaping a renderer applies cannot hide a
/// leak. Decoding the haystack rather than enumerating the escaped spellings of
/// the canary keeps the check from drifting when a renderer changes, and biases
/// it the safe way: an over-eager decode can only raise a false alarm, never
/// pass a real leak.
pub fn assert_redaction_canary_absent(
    serialized: &str,
    canary: &str,
) -> Result<(), RedactionCanaryError> {
    if canary.is_empty() {
        return Err(RedactionCanaryError::EmptyCanary);
    }
    if serialized.contains(canary) || decode_output_escapes(serialized).contains(canary) {
        Err(RedactionCanaryError::Exposed)
    } else {
        Ok(())
    }
}

/// Undo every escape AFDATA output can introduce: JSON and YAML string escapes,
/// logfmt quoting, and percent-encoding inside a URL.
///
/// Only used to widen the canary search, so an unrecognized escape is left
/// alone rather than guessed at.
fn decode_output_escapes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => match decode_backslash_escape(&bytes[index + 1..]) {
                Some((decoded, consumed)) => {
                    out.extend_from_slice(decoded.as_bytes());
                    index += 1 + consumed;
                }
                None => {
                    out.push(bytes[index]);
                    index += 1;
                }
            },
            b'%' => match percent_byte(&bytes[index + 1..]) {
                Some(byte) => {
                    out.push(byte);
                    index += 3;
                }
                None => {
                    out.push(bytes[index]);
                    index += 1;
                }
            },
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decode one escape body (the bytes after a `\`), returning it with the number
/// of bytes consumed.
fn decode_backslash_escape(rest: &[u8]) -> Option<(String, usize)> {
    let simple = |character: char| Some((character.to_string(), 1));
    match *rest.first()? {
        b'n' => simple('\n'),
        b'r' => simple('\r'),
        b't' => simple('\t'),
        b'b' => simple('\u{0008}'),
        b'f' => simple('\u{000c}'),
        b'v' => simple('\u{000b}'),
        b'0' => simple('\0'),
        b'\\' => simple('\\'),
        b'"' => simple('"'),
        b'/' => simple('/'),
        b'u' => {
            let first = hex4(rest.get(1..)?)?;
            // A non-BMP character is escaped as a surrogate pair; decoding the
            // halves separately would lose the character entirely.
            if (0xd800..0xdc00).contains(&first) {
                let low = rest
                    .get(5..7)
                    .filter(|marker| *marker == b"\\u".as_slice())
                    .and_then(|_| rest.get(7..))
                    .and_then(hex4)
                    .filter(|value| (0xdc00..0xe000).contains(value))?;
                let combined = 0x10000 + ((first - 0xd800) << 10) + (low - 0xdc00);
                return char::from_u32(combined).map(|character| (character.to_string(), 11));
            }
            char::from_u32(first).map(|character| (character.to_string(), 5))
        }
        _ => None,
    }
}

fn hex4(bytes: &[u8]) -> Option<u32> {
    let digits = bytes.get(..4)?;
    std::str::from_utf8(digits)
        .ok()
        .and_then(|text| u32::from_str_radix(text, 16).ok())
}

fn percent_byte(rest: &[u8]) -> Option<u8> {
    let digits = rest.get(..2)?;
    std::str::from_utf8(digits)
        .ok()
        .and_then(|text| u8::from_str_radix(text, 16).ok())
}

fn lint_value_at(value: &Value, pointer: &str, findings: &mut Vec<LintFinding>) {
    lint_unsafe_integer(value, pointer, findings);
    match value {
        Value::Object(map) => {
            if let Some(Value::Object(properties)) = map.get("properties") {
                for (name, schema) in properties {
                    lint_schema_property(
                        name,
                        schema,
                        &join_pointer(pointer, "properties"),
                        findings,
                    );
                }
            }
            for (key, child) in map {
                // JSON Schema property descriptors are not runtime values.
                if key == "properties" && child.is_object() {
                    continue;
                }
                let child_pointer = join_pointer(pointer, key);
                lint_suffix_type(key, child, &child_pointer, findings);
                lint_missing_suffix(key, child, &child_pointer, findings);
                lint_value_at(child, &child_pointer, findings);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                lint_value_at(item, &join_pointer(pointer, &index.to_string()), findings);
            }
        }
        _ => {}
    }
}

fn lint_schema_property(
    name: &str,
    schema: &Value,
    properties_pointer: &str,
    findings: &mut Vec<LintFinding>,
) {
    let property_pointer = join_pointer(properties_pointer, name);
    let normalized_name = name.to_ascii_lowercase();
    if !matches!(schema, Value::Object(_) | Value::Bool(_)) {
        lint_suffix_type(name, schema, &property_pointer, findings);
        lint_value_at(schema, &property_pointer, findings);
        return;
    }
    if normalized_name.ends_with("_secret")
        && let Some(object) = schema.as_object()
    {
        for field in ["default", "example"] {
            if let Some(value) = object.get(field)
                && !is_redacted_secret_literal(value)
            {
                findings.push(LintFinding::error(
                    "secret_schema_value_exposed",
                    join_pointer(&property_pointer, field),
                    format!("schema property {name:?} exposes secret {field}"),
                ));
            }
        }
        if let Some(Value::Array(examples)) = object.get("examples") {
            for (index, value) in examples.iter().enumerate() {
                if !is_redacted_secret_literal(value) {
                    findings.push(LintFinding::error(
                        "secret_schema_value_exposed",
                        join_pointer(
                            &join_pointer(&property_pointer, "examples"),
                            &index.to_string(),
                        ),
                        format!("schema property {name:?} exposes secret example"),
                    ));
                }
            }
        }
    }
    lint_schema_suffix_type(name, schema, &property_pointer, findings);
    lint_value_at(schema, &property_pointer, findings);
}

fn lint_schema_suffix_type(
    name: &str,
    schema: &Value,
    pointer: &str,
    findings: &mut Vec<LintFinding>,
) {
    let normalized = name.to_ascii_lowercase();
    let (expected, description): (&[&str], &str) = if normalized.ends_with("_bytes") {
        (&["integer"], "an integer byte count")
    } else if normalized.ends_with("_epoch_s") || normalized.ends_with("_epoch_ms") {
        (&["integer"], "an integer epoch timestamp")
    } else if normalized.ends_with("_epoch_ns") {
        (&["string"], "a decimal integer string")
    } else if normalized.ends_with("_sats") || normalized.ends_with("_msats") {
        (
            &["integer", "string"],
            "an integer or decimal integer string",
        )
    } else if normalized.ends_with("_percent") || is_duration_suffix(&normalized) {
        (&["integer", "number"], "a numeric value")
    } else if is_currency_minor_unit_suffix(&normalized) {
        (&["integer"], "an integer currency amount")
    } else if normalized.ends_with("_rfc3339")
        || normalized.ends_with("_url")
        || normalized.ends_with("_bcp47")
        || normalized.ends_with("_rfc3339_date")
        || normalized.ends_with("_rfc3339_time")
        || normalized.ends_with("_utc_offset")
    {
        (&["string"], "a string")
    } else {
        return;
    };

    if !schema_accepts_any_type(schema, expected) {
        findings.push(LintFinding::error(
            "suffix_type_mismatch",
            join_pointer(pointer, "type"),
            format!("schema property {name:?} must allow {description}"),
        ));
    }
}

fn schema_accepts_any_type(schema: &Value, expected: &[&str]) -> bool {
    let Some(object) = schema.as_object() else {
        return true;
    };

    if let Some(schema_type) = object.get("type") {
        return match schema_type {
            Value::String(value) => expected.contains(&value.as_str()),
            Value::Array(values) => values.iter().any(|value| {
                value
                    .as_str()
                    .is_some_and(|value| expected.contains(&value))
            }),
            _ => true,
        };
    }

    for keyword in ["anyOf", "oneOf"] {
        if let Some(Value::Array(branches)) = object.get(keyword) {
            return branches
                .iter()
                .any(|branch| schema_accepts_any_type(branch, expected));
        }
    }
    if let Some(Value::Array(branches)) = object.get("allOf") {
        return branches
            .iter()
            .all(|branch| schema_accepts_any_type(branch, expected));
    }
    true
}

fn is_redacted_secret_literal(value: &Value) -> bool {
    matches!(value, Value::Null) || matches!(value, Value::String(text) if text == "***")
}

const REGISTERED_SUFFIXES: &[(&str, &[&str])] = &[
    (
        "duration",
        &["_ns", "_us", "_ms", "_s", "_minutes", "_hours", "_days"],
    ),
    (
        "timestamp",
        &["_epoch_s", "_epoch_ms", "_epoch_ns", "_rfc3339"],
    ),
    (
        "strict_string",
        &["_rfc3339_date", "_rfc3339_time", "_bcp47", "_utc_offset"],
    ),
    ("size", &["_bytes"]),
    ("percentage", &["_percent"]),
    (
        "currency",
        &[
            "_msats",
            "_sats",
            "_usd_cents",
            "_eur_cents",
            "_jpy",
            "_{code}_cents",
            "_{code}_micro",
        ],
    ),
    ("sensitive", &["_secret", "_url"]),
];

const UNSUFFIXED_STEMS: &[(&str, &str)] = &[
    ("timeout", "duration"),
    ("elapsed", "duration"),
    ("duration", "duration"),
    ("ttl", "duration"),
    ("interval", "duration"),
    ("latency", "duration"),
    ("delay", "duration"),
    ("uptime", "duration"),
    ("price", "currency"),
    ("amount", "currency"),
    ("cost", "currency"),
    ("fee", "currency"),
    ("balance", "currency"),
    ("subtotal", "currency"),
    ("revenue", "currency"),
    ("created", "timestamp"),
    ("updated", "timestamp"),
    ("modified", "timestamp"),
    ("expires", "timestamp"),
    ("issued", "timestamp"),
    ("timestamp", "timestamp"),
    ("apikey", "sensitive"),
    ("api_key", "sensitive"),
    ("token", "sensitive"),
    ("password", "sensitive"),
    ("passwd", "sensitive"),
    ("secret", "sensitive"),
    ("credential", "sensitive"),
    ("credentials", "sensitive"),
];

/// Whether a key already carries a suffix the renderer and redactor act on.
///
/// Matching is the convention's own rule — the suffix spelled all-lowercase or
/// all-uppercase, never mixed — because this decides whether the linter stays
/// quiet. Accepting `api_Secret` here would report a field as marked while
/// `redaction.rs` leaves it in the clear, which is the one mistake a linter
/// used as a leak guard must not make.
fn has_registered_suffix(key: &str) -> bool {
    REGISTERED_SUFFIXES
        .iter()
        .flat_map(|(_, suffixes)| suffixes.iter())
        .any(|suffix| match suffix.strip_prefix("_{code}") {
            Some(tail) => crate::formatting::strip_suffix_ci(key, tail)
                .as_deref()
                .and_then(|rest| rest.rsplit_once('_').map(|(_, code)| code.to_string()))
                .is_some_and(|code| {
                    (3..=4).contains(&code.len())
                        && code
                            .chars()
                            .all(|character| character.is_ascii_alphabetic())
                }),
            None => crate::formatting::has_suffix_ci(key, suffix),
        })
}

fn lint_missing_suffix(key: &str, value: &Value, pointer: &str, findings: &mut Vec<LintFinding>) {
    if value.is_null() || has_registered_suffix(key) {
        return;
    }
    let lower = key.to_ascii_lowercase();
    let category = if lower.ends_with("_at") {
        Some("timestamp")
    } else {
        UNSUFFIXED_STEMS
            .iter()
            .find(|(stem, _)| lower == *stem || lower.ends_with(&format!("_{stem}")))
            .map(|(_, category)| *category)
    };
    let Some(category) = category else {
        return;
    };
    let plausible = match category {
        "duration" | "currency" | "size" | "percentage" => value.is_number(),
        "timestamp" => value.is_number() || value.is_string(),
        _ => value.is_string(),
    };
    if !plausible {
        return;
    }
    let suffixes = REGISTERED_SUFFIXES
        .iter()
        .find(|(name, _)| *name == category)
        .map(|(_, suffixes)| suffixes.join(", "))
        .unwrap_or_default();
    let message = if category == "sensitive" {
        format!(
            "`{key}` looks like a credential but is not marked, so it is printed and logged in \
             the clear. Rename it with one of: {suffixes}"
        )
    } else {
        format!(
            "`{key}` names a {category} but carries no unit, so a reader cannot tell what it \
             means without asking. Rename it with one of: {suffixes}"
        )
    };
    findings.push(LintFinding::warning(
        "missing_suffix",
        pointer.to_string(),
        message,
    ));
}

fn lint_suffix_type(key: &str, value: &Value, pointer: &str, findings: &mut Vec<LintFinding>) {
    if value.is_null() {
        return;
    }
    // Suffix recognition uses the convention's own matching rule, not a
    // case-folded approximation: a key the renderer treats as plain must not be
    // held to a suffix's type contract, or the fix the message asks for leaves
    // the field just as untreated as before.
    let has = |suffix: &str| crate::formatting::has_suffix_ci(key, suffix);
    let message = if has("_bytes") {
        (!is_non_negative_integer(value))
            .then(|| format!("{key:?} must be a non-negative integer byte count"))
    } else if has("_epoch_s") || has("_epoch_ms") {
        (!is_integer(value)).then(|| format!("{key:?} must be an integer epoch timestamp"))
    } else if has("_epoch_ns") {
        (!is_decimal_integer_string(value))
            .then(|| format!("{key:?} must be a decimal integer string"))
    } else if has("_sats") || has("_msats") {
        (!(is_integer(value) || is_decimal_integer_string(value)))
            .then(|| format!("{key:?} must be an integer or decimal integer string"))
    } else if has("_percent") {
        (!value.is_number()).then(|| format!("{key:?} must be numeric"))
    } else if is_duration_suffix(key) {
        (!value.is_number()).then(|| format!("{key:?} must be a numeric duration"))
    } else if is_currency_minor_unit_suffix(key) {
        (!is_integer(value)).then(|| format!("{key:?} must be an integer currency amount"))
    } else if has("_rfc3339") {
        if value.as_str().is_some_and(is_valid_rfc3339) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 date-time with a mandatory offset (e.g. \
                 2026-02-14T10:30:00Z)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if has("_url") {
        // Redaction walks a URL-marked collection into its string leaves, so a
        // collection is a shape the convention supports, not a type error. Only
        // the leaves are held to the single-URL rule.
        match url_field_violation(value) {
            UrlFieldCheck::Ok => None,
            UrlFieldCheck::NotAUrl => Some(format!(
                "{key:?} must be a single URL (no internal whitespace or bare credentials)"
            )),
            UrlFieldCheck::NotAString => Some(format!(
                "{key:?} must be a URL string, or a collection whose leaves are URL strings"
            )),
        }
    } else if has("_bcp47") {
        if value.as_str().is_some_and(is_valid_bcp47) {
            None
        } else if value.is_string() {
            Some(format!("{key:?} must be a well-formed BCP 47 language tag"))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if has("_rfc3339_date") {
        if value.as_str().is_some_and(is_valid_rfc3339_date) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 full-date (YYYY-MM-DD)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if has("_rfc3339_time") {
        if value.as_str().is_some_and(is_valid_rfc3339_time) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 partial-time (HH:MM:SS[.fraction], no Z or offset)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if has("_utc_offset") {
        if value.as_str().and_then(normalize_utc_offset).is_some() {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be a fixed UTC offset (\"UTC\" or ±HH:MM)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else {
        None
    };
    if let Some(message) = message {
        findings.push(LintFinding::error(
            "suffix_type_mismatch",
            pointer.to_string(),
            message,
        ));
    }
}

fn lint_unsafe_integer(value: &Value, pointer: &str, findings: &mut Vec<LintFinding>) {
    let Value::Number(number) = value else {
        return;
    };
    if !number_is_integer_literal(number) {
        return;
    }
    let exceeds_safe_range = if let Some(value) = number.as_i128() {
        value.unsigned_abs() > u128::from(MAX_SAFE_INTEGER)
    } else if let Some(value) = number.as_u128() {
        value > u128::from(MAX_SAFE_INTEGER)
    } else if let Some(value) = number.as_f64() {
        value.abs() > MAX_SAFE_INTEGER as f64
    } else {
        // An exact integer too large even for a finite f64 is necessarily
        // outside JavaScript's safe-integer range.
        true
    };
    if exceeds_safe_range {
        findings.push(unsafe_integer_finding(pointer));
    }
}

fn unsafe_integer_finding(pointer: &str) -> LintFinding {
    LintFinding::error(
        "unsafe_integer",
        pointer.to_string(),
        "integer exceeds JavaScript safe integer range ±(2^53-1)".to_string(),
    )
}

fn is_integer(value: &Value) -> bool {
    matches!(value, Value::Number(number) if number_is_integer(number))
}

fn is_non_negative_integer(value: &Value) -> bool {
    let Value::Number(number) = value else {
        return false;
    };
    if !number_is_integer(number) {
        return false;
    }
    let text = number.to_string();
    !text.starts_with('-') || decimal_number_is_zero(&text)
}

/// Whether a JSON number's exact mathematical value is an integer.
///
/// `serde_json/arbitrary_precision` preserves the source decimal, so this
/// handles integral-valued decimals and exponents without rounding through
/// f64, while also accepting integers larger than u128.
fn number_is_integer(number: &Number) -> bool {
    decimal_number_is_integer(&number.to_string())
}

/// Whether the producer wrote this number as an integer literal.
///
/// The unsafe-integer rule is about an integer that cannot survive a JavaScript
/// round trip, so it keys off the written form rather than the mathematical
/// value. `1.5e300` and `1.0e17` are float literals: already doubles by
/// construction, carrying no exactness promise to break. `100000000000000000`
/// does carry one, and breaks it.
fn number_is_integer_literal(number: &Number) -> bool {
    crate::formatting::number_is_integer_literal(number)
}

fn decimal_number_is_integer(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let exponent_index = unsigned.find(['e', 'E']);
    let (mantissa, exponent_text) = exponent_index.map_or((unsigned, None), |index| {
        (&unsigned[..index], Some(&unsigned[index + 1..]))
    });
    if mantissa
        .bytes()
        .filter(|byte| *byte != b'.')
        .all(|byte| byte == b'0')
    {
        return true;
    }

    let fraction_digits = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let exponent = exponent_text.map_or(0, |value| {
        value.parse::<i128>().unwrap_or_else(|_| {
            if value.starts_with('-') {
                i128::MIN
            } else {
                i128::MAX
            }
        })
    });
    let scale = exponent.saturating_sub(i128::try_from(fraction_digits).unwrap_or(i128::MAX));
    if scale >= 0 {
        return true;
    }

    let required_trailing_zeroes = scale.unsigned_abs();
    let digit_count = mantissa.bytes().filter(|byte| *byte != b'.').count();
    if required_trailing_zeroes > digit_count as u128 {
        return false;
    }
    mantissa
        .bytes()
        .filter(|byte| *byte != b'.')
        .rev()
        .take(required_trailing_zeroes as usize)
        .all(|byte| byte == b'0')
}

fn decimal_number_is_zero(text: &str) -> bool {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let mantissa = unsigned
        .split_once(['e', 'E'])
        .map_or(unsigned, |(mantissa, _)| mantissa);
    mantissa
        .bytes()
        .filter(|byte| *byte != b'.')
        .all(|byte| byte == b'0')
}

fn is_decimal_integer_string(value: &Value) -> bool {
    let Value::String(text) = value else {
        return false;
    };
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
}

fn is_duration_suffix(key: &str) -> bool {
    ["_ns", "_us", "_ms", "_s", "_minutes", "_hours", "_days"]
        .iter()
        .any(|suffix| crate::formatting::has_suffix_ci(key, suffix))
}

/// Whether a key names a currency minor unit the formatter actually formats.
///
/// `_cents` and `_micro` carry a currency code (`_usd_cents`); a bare
/// `total_cents` is not a registered suffix, and holding it to the integer rule
/// would demand a fix that changes nothing about how the field is rendered.
fn is_currency_minor_unit_suffix(key: &str) -> bool {
    crate::formatting::extract_currency_code(key).is_some()
        || crate::formatting::extract_currency_code_micro(key).is_some()
        || crate::formatting::has_suffix_ci(key, "_jpy")
}

/// How a `_url`-marked value compares to the shapes redaction handles.
enum UrlFieldCheck {
    Ok,
    NotAUrl,
    NotAString,
}

/// Redaction gives URL treatment to string leaves and walks collections to
/// reach them, so the linter accepts the same shapes and checks the leaves.
fn url_field_violation(value: &Value) -> UrlFieldCheck {
    match value {
        Value::Null => UrlFieldCheck::Ok,
        Value::String(text) => {
            if is_wellformed_url_field(text) {
                UrlFieldCheck::Ok
            } else {
                UrlFieldCheck::NotAUrl
            }
        }
        Value::Array(items) => first_url_violation(items.iter()),
        Value::Object(entries) => first_url_violation(entries.values()),
        _ => UrlFieldCheck::NotAString,
    }
}

fn first_url_violation<'value>(values: impl Iterator<Item = &'value Value>) -> UrlFieldCheck {
    for value in values {
        match url_field_violation(value) {
            UrlFieldCheck::Ok => {}
            violation => return violation,
        }
    }
    UrlFieldCheck::Ok
}

fn is_wellformed_url_field(value: &str) -> bool {
    if is_scheme_prefixed_url(value) || is_scheme_prefixed_url(value.trim()) {
        return true;
    }
    !value.chars().any(char::is_whitespace) && !value.contains('@')
}

fn is_scheme_prefixed_url(value: &str) -> bool {
    if value.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return false;
    }
    let bytes = value.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }
    let mut index = 1;
    while index < bytes.len() {
        let character = bytes[index];
        if character.is_ascii_alphanumeric() || matches!(character, b'+' | b'-' | b'.') {
            index += 1;
        } else {
            break;
        }
    }
    value[index..].starts_with("://")
}

fn join_pointer(base: &str, token: &str) -> String {
    let escaped = token.replace('~', "~0").replace('/', "~1");
    if base.is_empty() {
        format!("/{escaped}")
    } else {
        format!("{base}/{escaped}")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LintOptions, LintSeverity, REGISTERED_SUFFIXES, RedactionCanaryError,
        assert_no_lint_findings, assert_redaction_canary_absent, assert_strict_event, lint_value,
    };
    use serde_json::{Value, json};

    #[test]
    fn public_lint_api_reports_and_filters_findings() {
        let value = json!({
            "timeout": 5,
            "size_bytes": "large",
            "created_rfc3339": "not-a-time"
        });
        let findings = lint_value(&value, LintOptions::default());
        assert_eq!(findings.len(), 3);
        assert!(
            findings
                .iter()
                .any(|finding| finding.severity == LintSeverity::Warning)
        );
        let errors = lint_value(&value, LintOptions::errors_only());
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn integer_suffixes_accept_integral_decimals_and_signed_currency() {
        let value: Value = serde_json::from_str(
            r#"{
                "price_usd_cents": -1,
                "refund_eur_cents": -3.0,
                "size_bytes": 3.0,
                "created_epoch_ms": 3e0
            }"#,
        )
        .unwrap_or_else(|error| panic!("{error}"));

        assert!(lint_value(&value, LintOptions::errors_only()).is_empty());
    }

    #[test]
    fn registered_currency_suffixes_accept_three_or_four_letter_codes() {
        assert!(super::has_registered_suffix("fare_thb_cents"));
        assert!(super::has_registered_suffix("deposit_usdt_cents"));
        assert!(!super::has_registered_suffix("total_amount_cents"));
        assert!(!super::has_registered_suffix("price_usdtx_cents"));
    }

    #[test]
    fn integer_suffixes_reject_fractional_values_and_negative_bytes() {
        let value: Value = serde_json::from_str(
            r#"{
                "price_usd_cents": 3.5,
                "size_bytes": -1
            }"#,
        )
        .unwrap_or_else(|error| panic!("{error}"));
        let findings = lint_value(&value, LintOptions::errors_only());

        assert_eq!(
            findings
                .iter()
                .filter(|finding| finding.rule_id == "suffix_type_mismatch")
                .count(),
            2
        );
    }

    #[test]
    fn unsafe_integer_finds_values_beyond_u128() {
        let value: Value =
            serde_json::from_str(r#"{"huge_count":340282366920938463463374607431768211456}"#)
                .unwrap_or_else(|error| panic!("{error}"));
        let findings = lint_value(&value, LintOptions::errors_only());

        assert!(
            findings
                .iter()
                .any(|finding| finding.rule_id == "unsafe_integer"),
            "{findings:?}"
        );
    }

    #[test]
    fn assertion_helpers_cover_protocol_lint_and_redaction() {
        let event = json!({
            "kind": "result",
            "result": {"code": "ready"},
            "trace": {}
        });
        assert!(assert_strict_event(&event).is_ok());
        assert!(assert_no_lint_findings(&event).is_ok());
        assert!(assert_redaction_canary_absent("{\"secret\":\"***\"}", "canary-42").is_ok());
        assert_eq!(
            assert_redaction_canary_absent("contains canary-42", "canary-42"),
            Err(RedactionCanaryError::Exposed)
        );
    }

    /// The missing-suffix rule reads intent from the name, so what it stays
    /// quiet about matters as much as what it reports. Driven through the
    /// public entry point rather than the private helper, so a later move of
    /// the rule cannot take the pin with it.
    #[test]
    fn missing_suffix_reads_intent_from_the_name() {
        for (label, value) in [
            ("bare dimension name", json!({"timeout": 5000})),
            ("suffixed dimension name", json!({"request_timeout": 1})),
            ("_at timestamp", json!({"expires_at": 1})),
        ] {
            let findings = lint_value(&json!(value), LintOptions::default());
            assert!(
                findings.iter().any(|f| f.rule_id == "missing_suffix"),
                "{label}: expected a missing_suffix warning, got {findings:?}"
            );
        }

        for (label, value) in [
            ("already labelled duration", json!({"timeout_ms": 5000})),
            ("already labelled currency", json!({"price_gbp_cents": 1})),
            ("already labelled secret", json!({"api_key_secret": "sk"})),
            // A dimension name over a container is a config block, not a bare
            // number. This is the guard that has no other test.
            (
                "dimension name over a container",
                json!({"timeout": {"connect": 1}}),
            ),
            ("credentials block", json!({"credentials": {"user": "x"}})),
            ("unrelated name", json!({"name": "demo"})),
        ] {
            let findings = lint_value(&json!(value), LintOptions::default());
            assert!(
                findings.is_empty(),
                "{label}: expected no findings, got {findings:?}"
            );
        }
    }

    /// A canary is only useful if escaping cannot hide it. Every case here is a
    /// canary sitting verbatim in the output while a raw substring search
    /// misses it, which is how a leak used to pass this assertion.
    #[test]
    fn redaction_canary_survives_every_output_escape() {
        for (label, canary) in [
            (
                "pem newlines",
                "-----BEGIN KEY-----\nabc\n-----END KEY-----",
            ),
            ("double quote", "p@ss\"word"),
            ("backslash path", "C:\\Users\\me\\key"),
            ("tab", "a\tb"),
            ("non-bmp", "k\u{1f511}ey"),
        ] {
            let rendered = crate::render(
                &json!({ "note": canary }),
                crate::OutputFormat::Json,
                &crate::OutputOptions::default(),
            );
            assert!(
                rendered.contains("note"),
                "{label}: expected the field to survive rendering"
            );
            assert_eq!(
                assert_redaction_canary_absent(&rendered, canary),
                Err(RedactionCanaryError::Exposed),
                "{label}: escaped canary must still be reported, output was {rendered}"
            );
        }
    }

    /// The percent-encoded spelling counts too: a canary can reach the stream
    /// through a URL without ever appearing literally.
    #[test]
    fn redaction_canary_is_found_percent_encoded() {
        assert_eq!(
            assert_redaction_canary_absent("{\"note\":\"https://h/p?q=a%20b%20c\"}", "a b c"),
            Err(RedactionCanaryError::Exposed)
        );
    }

    /// The widened search must not cry wolf: a genuinely redacted secret
    /// reports clean in every format, whatever characters it contained.
    #[test]
    fn redaction_canary_stays_quiet_when_the_value_was_redacted() {
        for canary in [
            "-----BEGIN KEY-----\nabc\n-----END KEY-----",
            "p@ss\"word",
            "C:\\Users\\me\\key",
            "k\u{1f511}ey",
        ] {
            for format in [
                crate::OutputFormat::Json,
                crate::OutputFormat::Yaml,
                crate::OutputFormat::Plain,
            ] {
                let rendered = crate::render(
                    &json!({ "note_secret": canary }),
                    format,
                    &crate::OutputOptions::default(),
                );
                assert_eq!(
                    assert_redaction_canary_absent(&rendered, canary),
                    Ok(()),
                    "redacted output must not report a leak, output was {rendered}"
                );
            }
        }
    }

    #[test]
    fn registry_suffixes_match_the_lint_table() {
        const REGISTRY: &str =
            include_str!("../../skills/agent-first-data/references/registry.json");
        let registry: Value =
            serde_json::from_str(REGISTRY).unwrap_or_else(|error| panic!("{error}"));
        let suffixes = registry["suffixes"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mut from_registry: Vec<(String, String)> = suffixes
            .iter()
            .filter_map(|entry| {
                Some((
                    entry["category"].as_str()?.to_string(),
                    entry["suffix"].as_str()?.to_string(),
                ))
            })
            .collect();
        let mut from_table: Vec<(String, String)> = REGISTERED_SUFFIXES
            .iter()
            .flat_map(|(category, suffixes)| {
                suffixes
                    .iter()
                    .map(move |suffix| ((*category).to_string(), (*suffix).to_string()))
            })
            .collect();
        from_registry.sort();
        from_table.sort();
        assert_eq!(from_table, from_registry);
    }
}
