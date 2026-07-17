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
    String(String),
    Array(Vec<Value>),
    /// Object map. Keys are stored sorted (BTreeMap), not in insertion order.
    Object(BTreeMap<String, Value>),
}

impl Value {
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

    pub fn is_integer(&self) -> bool {
        matches!(self, Value::Integer(_) | Value::Unsigned(_))
    }

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

    pub fn is_float(&self) -> bool {
        matches!(self, Value::Float(_))
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
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

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Null => write!(f, "null"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Integer(i) => write!(f, "{}", i),
            Value::Unsigned(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
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
                formatter.write_str("a configuration value")
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
                    if let Some(i) = n.as_i64() {
                        Value::Integer(i)
                    } else if let Some(u) = n.as_u64() {
                        Value::Unsigned(u)
                    } else {
                        Value::Float(n.as_f64().unwrap_or(0.0))
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
