//! CLI-facing value construction for `set`/`add` (`cli-shell-config-todo.md`
//! §3, which supersedes the earlier `cli-design-review-todo.md` D3): a bare
//! VALUE/FIELD=VALUE is always [`Value::String`] with zero coercion — no
//! shape-guessing, no type prefixes. An exact type is requested explicitly
//! via [`ValueType`] (`set`'s `--value-type` flag); [`guard_bare_overwrite`]
//! implements the "异型覆盖守卫" (heterogeneous-overwrite guard) that turns a
//! bare VALUE silently changing an existing scalar's type into an argument
//! error instead.

use crate::document::{DocumentError, DocumentResult, Value};

/// The exact type an explicit `--value-type` requests for a `set` VALUE.
///
/// `String` is also the *implicit* type of a bare VALUE (zero coercion) —
/// see [`guard_bare_overwrite`] for the rule that keeps that implicit choice
/// from silently overwriting a differently-typed existing scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    String,
    Number,
    Bool,
    Null,
    Json,
}

impl ValueType {
    /// Parse a `--value-type` flag value. `None` for anything else.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "string" => Some(Self::String),
            "number" => Some(Self::Number),
            "bool" => Some(Self::Bool),
            "null" => Some(Self::Null),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// The flag spelling this variant was parsed from (used in error
    /// messages and the heterogeneous-overwrite guard's escape hatches).
    pub fn name(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Null => "null",
            Self::Json => "json",
        }
    }
}

/// Construct a `Value` from a CLI string per an explicit [`ValueType`].
///
/// `raw` is the VALUE positional; it must be `None` for [`ValueType::Null`]
/// (null takes no payload) and `Some` for every other type. `number`/`bool`
/// parse strictly (an invalid literal is a [`DocumentError::ParseError`],
/// not a silent fallback to string); `json` is the only entry point for
/// arrays, objects, and an "exact-type scalar" (`--value-type json` value
/// `"8080"` writes the *string* `"8080"`, not the number). `number` is
/// literal-faithful: an oversized integer or high-precision float is
/// preserved digit for digit via [`Value::Number`] — see
/// `cli-shell-config-todo.md` §4.
pub fn value_from_type(value_type: ValueType, raw: Option<&str>) -> DocumentResult<Value> {
    match value_type {
        ValueType::Null => match raw {
            None => Ok(Value::Null),
            Some(_) => Err(DocumentError::ParseError {
                format: "value".to_string(),
                detail: "--value-type null takes no VALUE".to_string(),
            }),
        },
        ValueType::String => Ok(Value::String(require_value(raw, value_type)?.to_string())),
        ValueType::Bool => {
            let raw = require_value(raw, value_type)?;
            parse_bool(raw).map(Value::Bool).ok_or_else(|| {
                DocumentError::ParseError {
                    format: "boolean".to_string(),
                    detail: format!(
                        "invalid --value-type bool value `{raw}`; expected true/false, yes/no, on/off, or 1/0"
                    ),
                }
            })
        }
        ValueType::Number => parse_number_literal(require_value(raw, value_type)?),
        ValueType::Json => {
            let raw = require_value(raw, value_type)?;
            serde_json::from_str::<serde_json::Value>(raw)
                .map(Value::from)
                .map_err(|error| DocumentError::ParseError {
                    format: "JSON".to_string(),
                    detail: error.to_string(),
                })
        }
    }
}

fn require_value(raw: Option<&str>, value_type: ValueType) -> DocumentResult<&str> {
    raw.ok_or_else(|| DocumentError::ParseError {
        format: "value".to_string(),
        detail: format!("--value-type {} requires a VALUE", value_type.name()),
    })
}

/// Parse a `--value-type number` literal strictly as a JSON number (no
/// leading zeros, no `+` sign, no `Infinity`/`NaN` — the same grammar the
/// document layer's JSON reader accepts), preserving its exact digits via
/// [`Value::Number`] when it does not fit `Integer`/`Unsigned`/`Float`
/// exactly. Reuses `serde_json`'s `arbitrary_precision` parsing so the
/// validation and the literal-fidelity capture are the same code path as
/// the JSON document reader (`format::json::load`).
fn parse_number_literal(text: &str) -> DocumentResult<Value> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .filter(serde_json::Value::is_number)
        .map(Value::from)
        .ok_or_else(|| DocumentError::ParseError {
            format: "number".to_string(),
            detail: format!("invalid --value-type number literal `{text}`"),
        })
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// The four AFDATA scalar "kinds" the heterogeneous-overwrite guard
/// distinguishes. `Number` covers `Integer`/`Unsigned`/`Float`/
/// [`Value::Number`] — the guard cares whether VALUE would change a scalar
/// *from* one of these kinds *to* a bare string, not which numeric
/// representation was in play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarKind {
    Null,
    Bool,
    Number,
    String,
}

impl ScalarKind {
    /// The `--value-type` spelling that keeps a value of this kind.
    pub fn value_type_name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool => "bool",
            Self::Number => "number",
            Self::String => "string",
        }
    }
}

/// Whether `value` satisfies `expected` for a typed read — the GET-side
/// counterpart to [`value_from_type`]. `Json` matches any value; `Number`
/// matches every numeric representation; the rest match their one kind. A
/// consumer that asks for a type gets a value back only when it actually is
/// that type, so a wrong-typed leaf is a caught error rather than a surprise.
pub fn value_matches_type(value: &Value, expected: ValueType) -> bool {
    match expected {
        ValueType::Json => true,
        ValueType::String => matches!(value, Value::String(_)),
        ValueType::Bool => matches!(value, Value::Bool(_)),
        ValueType::Null => matches!(value, Value::Null),
        ValueType::Number => matches!(
            value,
            Value::Integer(_) | Value::Unsigned(_) | Value::Float(_) | Value::Number(_)
        ),
    }
}

/// The scalar kind of `value`, or `None` for an array/object (the guard
/// does not apply to containers — see `cli-shell-config-todo.md` §3).
pub fn scalar_kind(value: &Value) -> Option<ScalarKind> {
    match value {
        Value::Null => Some(ScalarKind::Null),
        Value::Bool(_) => Some(ScalarKind::Bool),
        Value::Integer(_) | Value::Unsigned(_) | Value::Float(_) | Value::Number(_) => {
            Some(ScalarKind::Number)
        }
        Value::String(_) => Some(ScalarKind::String),
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// The §3 "异型覆盖守卫" (heterogeneous-overwrite guard, closing design rule
/// 4 — "no silent type rewrites"): a bare VALUE (implicit `--value-type
/// string`) is always a string, so overwriting an *existing scalar of a
/// different kind* would silently change its type. That is an argument
/// error, not a coercion decision. Returns the existing kind so the caller
/// can build a message with the two escape hatches (`--value-type <kind>`
/// to keep the type, or `--value-type string` to convert explicitly).
///
/// `Ok(())` when there is nothing to guard: the target is absent (a new
/// key), already a string (no type change), or a container (out of this
/// guard's scope). Never called when `--value-type` was passed explicitly —
/// an explicit type is a deliberate declaration, not a silent rewrite.
pub fn guard_bare_overwrite(existing: Option<&Value>) -> Result<(), ScalarKind> {
    match existing.and_then(scalar_kind) {
        Some(ScalarKind::String) | None => Ok(()),
        Some(other) => Err(other),
    }
}

/// Coerce a CLI string toward the type already present at `existing`, for a
/// consumer that *knows* the target type from the value it is replacing (a
/// config setter reading typed leaves out of a serialized document).
///
/// This is the library counterpart to the CLI's explicit `--value-type`: the
/// generic `afdata` CLI must ask the user for the type because it cannot know
/// it, but a consumer that does — because it holds the existing typed leaf, or
/// its own schema — should neither add a flag nor re-parse. It is **type
/// directed, not shape guessing**: the existing leaf's [`scalar_kind`] selects
/// the parse; a `bool`/`number` literal that does not match falls back to a
/// string; a string, `null`, or absent leaf yields a string; a container leaf
/// is replaced with a JSON literal.
pub fn coerce_toward(raw: &str, existing: Option<&Value>) -> DocumentResult<Value> {
    match existing.and_then(scalar_kind) {
        Some(ScalarKind::Bool) => Ok(value_from_type(ValueType::Bool, Some(raw))
            .unwrap_or_else(|_| Value::String(raw.to_string()))),
        Some(ScalarKind::Number) => Ok(value_from_type(ValueType::Number, Some(raw))
            .unwrap_or_else(|_| Value::String(raw.to_string()))),
        Some(ScalarKind::String | ScalarKind::Null) => Ok(Value::String(raw.to_string())),
        // A container leaf (array/object) or a brand-new key with no type to
        // aim at: a structured literal (`[`/`{`) is parsed as JSON so the value
        // round-trips, while a bare scalar stays a string — no scalar
        // shape-guessing (`007` never becomes `7`).
        None => coerce_structured_or_string(raw),
    }
}

/// A structured literal (`[`/`{`) parses as a JSON array/object; anything else
/// is taken verbatim as a string. Used where no scalar type is known, so a
/// bare value is never shape-guessed into a number/bool.
fn coerce_structured_or_string(raw: &str) -> DocumentResult<Value> {
    let trimmed = raw.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        value_from_type(ValueType::Json, Some(raw))
    } else {
        Ok(Value::String(raw.to_string()))
    }
}

/// Coerce a CLI value slice toward the type at `existing` via [`coerce_toward`]:
/// one value becomes a scalar, several become an array whose elements are each
/// coerced toward the existing array's element type. An empty slice is an error.
pub fn coerce_values_toward(values: &[String], existing: Option<&Value>) -> DocumentResult<Value> {
    match values {
        [] => Err(DocumentError::EmptyValues),
        [one] => coerce_toward(one, existing),
        many => {
            let element = existing
                .and_then(Value::as_array)
                .and_then(|array| array.first());
            Ok(Value::Array(
                many.iter()
                    .map(|value| coerce_toward(value, element))
                    .collect::<DocumentResult<Vec<_>>>()?,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    #[test]
    fn bare_string_is_zero_coercion() {
        for raw in ["007", "1.0", "true", "null", "3e10"] {
            assert_eq!(
                value_from_type(ValueType::String, Some(raw)).unwrap(),
                Value::String(raw.to_string())
            );
        }
    }

    #[test]
    fn value_type_number_is_literal_faithful() {
        assert_eq!(
            value_from_type(ValueType::Number, Some("18446744073709551615")).unwrap(),
            Value::Unsigned(u64::MAX)
        );
        let huge = "123456789012345678901234567890";
        assert_eq!(
            value_from_type(ValueType::Number, Some(huge)).unwrap(),
            Value::Number(huge.to_string())
        );
        let precise = "0.1000000000000000055511151231257827";
        assert_eq!(
            value_from_type(ValueType::Number, Some(precise)).unwrap(),
            Value::Number(precise.to_string())
        );
    }

    #[test]
    fn value_type_number_rejects_leading_zero_and_non_numeric() {
        assert!(value_from_type(ValueType::Number, Some("007")).is_err());
        assert!(value_from_type(ValueType::Number, Some("abc")).is_err());
        assert!(value_from_type(ValueType::Number, Some("+5")).is_err());
    }

    #[test]
    fn value_type_bool_is_lenient() {
        assert_eq!(
            value_from_type(ValueType::Bool, Some("yes")).unwrap(),
            Value::Bool(true)
        );
        assert!(value_from_type(ValueType::Bool, Some("nope")).is_err());
    }

    #[test]
    fn value_type_null_takes_no_value() {
        assert_eq!(value_from_type(ValueType::Null, None).unwrap(), Value::Null);
        assert!(value_from_type(ValueType::String, None).is_err());
        assert!(value_from_type(ValueType::Null, Some("x")).is_err());
    }

    #[test]
    fn value_type_json_is_the_only_container_entry_point() {
        let value = value_from_type(ValueType::Json, Some(r#"["a","b"]"#)).unwrap();
        assert_eq!(
            value,
            Value::Array(vec![
                Value::String("a".to_string()),
                Value::String("b".to_string())
            ])
        );
        // An exact-type scalar via --value-type json: the string "8080", not
        // the number 8080.
        assert_eq!(
            value_from_type(ValueType::Json, Some("\"8080\"")).unwrap(),
            Value::String("8080".to_string())
        );
    }

    #[test]
    fn guard_fires_only_for_bare_overwrite_of_a_differently_kinded_scalar() {
        assert_eq!(guard_bare_overwrite(None), Ok(()));
        assert_eq!(
            guard_bare_overwrite(Some(&Value::String("x".to_string()))),
            Ok(())
        );
        assert_eq!(guard_bare_overwrite(Some(&Value::Array(vec![]))), Ok(()));
        assert_eq!(
            guard_bare_overwrite(Some(&Value::Integer(8080))),
            Err(ScalarKind::Number)
        );
        assert_eq!(
            guard_bare_overwrite(Some(&Value::Bool(true))),
            Err(ScalarKind::Bool)
        );
        assert_eq!(
            guard_bare_overwrite(Some(&Value::Null)),
            Err(ScalarKind::Null)
        );
    }

    #[test]
    fn coerce_toward_is_type_directed_not_shape_guessing() {
        // Toward an existing scalar's kind: bool/number parse toward it, a
        // non-matching literal falls back to a string, a string leaf stays a
        // string.
        assert_eq!(
            coerce_toward("false", Some(&Value::Bool(true))).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            coerce_toward("5432", Some(&Value::Integer(1))).unwrap(),
            Value::from(serde_json::json!(5432))
        );
        assert_eq!(
            coerce_toward("not-a-number", Some(&Value::Integer(1))).unwrap(),
            Value::String("not-a-number".to_string())
        );
        assert_eq!(
            coerce_toward("007", Some(&Value::String("x".to_string()))).unwrap(),
            Value::String("007".to_string())
        );
        // No existing type: a bare scalar is a string (no `007` -> `7`), but a
        // structured literal parses as JSON.
        assert_eq!(
            coerce_toward("007", None).unwrap(),
            Value::String("007".to_string())
        );
        assert!(matches!(
            coerce_toward("[]", None).unwrap(),
            Value::Array(_)
        ));
        assert!(matches!(
            coerce_toward("{\"a\":1}", Some(&Value::Object(Default::default()))).unwrap(),
            Value::Object(_)
        ));
    }

    #[test]
    fn coerce_values_toward_scalar_vs_array() {
        assert!(matches!(
            coerce_values_toward(&[], None),
            Err(DocumentError::EmptyValues)
        ));
        assert_eq!(
            coerce_values_toward(&["x".to_string()], None).unwrap(),
            Value::String("x".to_string())
        );
        // Several values become an array, each coerced toward the existing
        // array's element type.
        let existing = Value::Array(vec![Value::Bool(true)]);
        assert_eq!(
            coerce_values_toward(&["false".to_string(), "true".to_string()], Some(&existing))
                .unwrap(),
            Value::Array(vec![Value::Bool(false), Value::Bool(true)])
        );
    }
}
