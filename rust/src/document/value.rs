//! Custom Value type — zero external format dependencies.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Custom Value IR independent of any format crate.
/// Supports all formats: JSON, TOML, YAML, dotenv, and INI.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Float(f64),
    /// A numeric literal that would lose digits if forced through
    /// [`Value::Integer`]/[`Value::Unsigned`]/[`Value::Float`]: an integer
    /// outside `u64::MAX`, or any literal written with a fractional part or
    /// exponent (`3.14`, `1e-10`, `-0.0`, …). Holds the exact source text
    /// verbatim — never parsed and reserialized through `f64` — so a 30-digit
    /// integer or a high-precision decimal round-trips digit for digit.
    ///
    /// Produced by the JSON backend's reader (`format::json::load`), which is
    /// the format whose number grammar is genuinely arbitrary-precision;
    /// TOML integers are grammar-limited to `i64` and TOML/YAML floats are
    /// canonically `f64` at the format level, so those backends keep using
    /// `Integer`/`Unsigned`/`Float` for values in range. `set --value-type
    /// number` also produces this variant, preserving the CLI argument's
    /// literal spelling the same way.
    Number(String),
    String(String),
    Array(Vec<Value>),
    /// Object map. Keys are stored sorted (BTreeMap), not in insertion order.
    Object(BTreeMap<String, Value>),
}

impl Value {
    /// Stable value-kind label for diagnostics and machine-readable metadata.
    ///
    /// Signed and unsigned fixed-width integers both report `integer`; exact
    /// numeric literals stored in [`Value::Number`] report `number`.
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Integer(_) | Self::Unsigned(_) => "integer",
            Self::Float(_) => "float",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// True for `Integer`/`Unsigned`, and for a [`Value::Number`] literal
    /// with no fractional part or exponent — a big integer that only missed
    /// `Integer`/`Unsigned` because it overflows `i64`/`u64`, not because it
    /// is actually fractional.
    pub fn is_integer(&self) -> bool {
        match self {
            Value::Integer(_) | Value::Unsigned(_) => true,
            Value::Number(text) => is_integer_literal(text),
            _ => false,
        }
    }

    /// Exact `i64` value, when this holds one. `None` for a [`Value::Number`]
    /// literal even when it is integral — by construction that variant only
    /// exists because the literal does *not* fit `i64`/`u64`; read the exact
    /// digits via [`Value::as_number_literal`] instead.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_unsigned(&self) -> Option<u64> {
        match self {
            Value::Unsigned(value) => Some(*value),
            _ => None,
        }
    }

    /// True for `Float`, and for a [`Value::Number`] literal with a
    /// fractional part or exponent.
    pub fn is_float(&self) -> bool {
        match self {
            Value::Float(_) => true,
            Value::Number(text) => !is_integer_literal(text),
            _ => false,
        }
    }

    /// Best-effort numeric magnitude as `f64` — lossy for a
    /// [`Value::Number`] literal outside `f64`'s exact range, but never
    /// fails for a well-formed literal. Callers that need the exact digits
    /// (not just the magnitude) must read [`Value::as_number_literal`]
    /// instead; this accessor exists for magnitude-only consumers (numeric
    /// comparisons, lint checks) that cannot use arbitrary precision anyway.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Number(text) => text.parse::<f64>().ok(),
            _ => None,
        }
    }

    /// True for a [`Value::Number`] literal (a number outside
    /// `Integer`/`Unsigned`/`Float`'s exact range, preserved verbatim).
    pub fn is_number_literal(&self) -> bool {
        matches!(self, Value::Number(_))
    }

    /// The exact source text of a [`Value::Number`] literal, digit for
    /// digit. `None` for every other variant, including `Integer`/
    /// `Unsigned`/`Float` — those already round-trip exactly through their
    /// own `Display`, so this accessor is specifically for the literal-only
    /// case where that would corrupt the value.
    pub fn as_number_literal(&self) -> Option<&str> {
        match self {
            Value::Number(text) => Some(text),
            _ => None,
        }
    }

    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }

    pub fn as_array(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    pub fn as_object(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut BTreeMap<String, Value>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object().and_then(|o| o.get(key))
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.as_object_mut().and_then(|o| o.get_mut(key))
    }
}

/// True when a numeric literal's text has no fractional part or exponent —
/// a plain (optionally negative) run of decimal digits.
fn is_integer_literal(text: &str) -> bool {
    !text.contains(['.', 'e', 'E'])
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Integer(i) => write!(f, "{}", i),
            Value::Unsigned(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::Number(text) => write!(f, "{text}"),
            Value::String(s) => write!(f, "\"{}\"", s.escape_default()),
            Value::Array(_) => write!(f, "[...]"),
            Value::Object(_) => write!(f, "{{...}}"),
        }
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => serializer.serialize_none(),
            Value::Bool(value) => serializer.serialize_bool(*value),
            Value::Integer(value) => serializer.serialize_i64(*value),
            Value::Unsigned(value) => serializer.serialize_u64(*value),
            Value::Float(value) => serializer.serialize_f64(*value),
            // Generic serde data model has no arbitrary-precision number
            // primitive, so this is a best-effort numeric fallback (used only
            // by callers serializing a `Value` through some *other* serde
            // format directly — the crate's own JSON path goes through
            // `From<Value> for serde_json::Value` below, which preserves the
            // literal exactly via serde_json's `arbitrary_precision`).
            Value::Number(text) => {
                if let Ok(i) = text.parse::<i64>() {
                    serializer.serialize_i64(i)
                } else if let Ok(u) = text.parse::<u64>() {
                    serializer.serialize_u64(u)
                } else {
                    serializer.serialize_f64(text.parse::<f64>().unwrap_or(0.0))
                }
            }
            Value::String(value) => serializer.serialize_str(value),
            Value::Array(values) => values.serialize(serializer),
            Value::Object(values) => values.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ValueVisitor;

        impl<'de> de::Visitor<'de> for ValueVisitor {
            type Value = Value;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a document value")
            }

            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Value::Null)
            }

            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(Value::Null)
            }

            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(Value::Bool(value))
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(Value::Integer(value))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(Value::Unsigned(value))
            }

            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(Value::Float(value))
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(Value::String(value.to_string()))
            }

            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(Value::String(value))
            }

            fn visit_seq<A: de::SeqAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = access.next_element()? {
                    values.push(value);
                }
                Ok(Value::Array(values))
            }

            fn visit_map<A: de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                while let Some((key, value)) = access.next_entry()? {
                    values.insert(key, value);
                }
                Ok(Value::Object(values))
            }
        }

        deserializer.deserialize_any(ValueVisitor)
    }
}

mod json_convert {
    use super::*;
    use serde_json::json;

    impl From<serde_json::Value> for Value {
        fn from(v: serde_json::Value) -> Self {
            match v {
                serde_json::Value::Null => Value::Null,
                serde_json::Value::Bool(b) => Value::Bool(b),
                serde_json::Value::Number(n) => {
                    // With the crate's `arbitrary_precision` feature enabled,
                    // `n.as_str()` is the exact source literal — `as_i64`/
                    // `as_u64` only succeed for a literal with no fractional
                    // part or exponent that also fits the target width, so a
                    // plain in-range integer still takes the exact
                    // `Integer`/`Unsigned` path (unchanged from before);
                    // everything else (every float literal, and integers
                    // beyond `u64::MAX`) is preserved digit for digit as
                    // `Value::Number` instead of being forced through `f64`.
                    if let Some(i) = n.as_i64() {
                        Value::Integer(i)
                    } else if let Some(u) = n.as_u64() {
                        Value::Unsigned(u)
                    } else {
                        Value::Number(n.as_str().to_string())
                    }
                }
                serde_json::Value::String(s) => Value::String(s),
                serde_json::Value::Array(a) => {
                    Value::Array(a.into_iter().map(Value::from).collect())
                }
                serde_json::Value::Object(o) => {
                    let map = o.into_iter().map(|(k, v)| (k, Value::from(v))).collect();
                    Value::Object(map)
                }
            }
        }
    }

    impl From<Value> for serde_json::Value {
        fn from(v: Value) -> Self {
            match v {
                Value::Null => serde_json::Value::Null,
                Value::Bool(b) => json!(b),
                Value::Integer(i) => json!(i),
                Value::Unsigned(u) => json!(u),
                Value::Float(f) => {
                    json!(f)
                }
                // Re-parse the exact literal text as a JSON number so
                // `serde_json`'s `arbitrary_precision` `Number` retains it
                // verbatim (`Number::as_str()` round-trips exactly — see the
                // stdlib doctest on that method). Our own writers only ever
                // construct `Value::Number` from text that already passed
                // JSON-number syntax validation (the JSON reader below, or
                // `set --value-type number`'s literal check), so the
                // fallback is unreachable in practice; it stays total rather
                // than panicking on a hypothetically malformed literal.
                Value::Number(text) => match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(number @ serde_json::Value::Number(_)) => number,
                    _ => json!(text.parse::<f64>().unwrap_or(0.0)),
                },
                Value::String(s) => json!(s),
                Value::Array(a) => {
                    let arr: Vec<serde_json::Value> = a.into_iter().map(|v| v.into()).collect();
                    serde_json::Value::Array(arr)
                }
                Value::Object(o) => {
                    let mut map = serde_json::Map::new();
                    for (k, v) in o {
                        map.insert(k, serde_json::Value::from(v));
                    }
                    serde_json::Value::Object(map)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Value;
    use std::collections::BTreeMap;

    #[test]
    fn value_kind_names_are_stable() {
        let cases = [
            (Value::Null, "null"),
            (Value::Bool(true), "boolean"),
            (Value::Integer(-1), "integer"),
            (Value::Unsigned(1), "integer"),
            (Value::Float(1.5), "float"),
            (Value::Number("1e1000".to_string()), "number"),
            (Value::String("value".to_string()), "string"),
            (Value::Array(Vec::new()), "array"),
            (Value::Object(BTreeMap::new()), "object"),
        ];

        for (value, expected) in cases {
            assert_eq!(value.kind_name(), expected);
        }
    }
}
