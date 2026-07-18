//! Fallible serde adapters over the format-independent [`Value`] IR.

#![allow(clippy::items_after_test_module)]

use crate::document::{DocumentError, DocumentResult, Value};
use serde::{
    Deserializer, Serialize,
    de::{self, DeserializeOwned},
    ser,
};

/// Deserialize a typed configuration from a value view.
pub fn from_value<T: DeserializeOwned>(value: &Value, path: &str) -> DocumentResult<T> {
    T::deserialize(ValueDeserializer(value)).map_err(|error| DocumentError::from_serde(path, error))
}

/// Serialize a typed configuration into the format-independent value view.
pub fn to_value<T: Serialize>(value: &T) -> DocumentResult<Value> {
    value
        .serialize(ValueSerializer)
        .map_err(|error| DocumentError::ParseError {
            format: "serde".to_string(),
            detail: error.to_string(),
        })
}

struct ValueSerializer;

#[derive(Debug)]
struct SerializeError(String);

impl std::fmt::Display for SerializeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SerializeError {}

impl serde::ser::Error for SerializeError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

type Result<T, E = SerializeError> = std::result::Result<T, E>;

impl serde::Serializer for ValueSerializer {
    type Ok = Value;
    type Error = SerializeError;
    type SerializeSeq = SequenceSerializer;
    type SerializeTuple = SequenceSerializer;
    type SerializeTupleStruct = SequenceSerializer;
    type SerializeTupleVariant = VariantSequenceSerializer;
    type SerializeMap = MapSerializer;
    type SerializeStruct = MapSerializer;
    type SerializeStructVariant = VariantMapSerializer;

    fn serialize_bool(self, value: bool) -> Result<Value> {
        Ok(Value::Bool(value))
    }
    fn serialize_i8(self, value: i8) -> Result<Value> {
        Ok(Value::Integer(i64::from(value)))
    }
    fn serialize_i16(self, value: i16) -> Result<Value> {
        Ok(Value::Integer(i64::from(value)))
    }
    fn serialize_i32(self, value: i32) -> Result<Value> {
        Ok(Value::Integer(i64::from(value)))
    }
    fn serialize_i64(self, value: i64) -> Result<Value> {
        Ok(Value::Integer(value))
    }
    fn serialize_u8(self, value: u8) -> Result<Value> {
        Ok(Value::Unsigned(u64::from(value)))
    }
    fn serialize_u16(self, value: u16) -> Result<Value> {
        Ok(Value::Unsigned(u64::from(value)))
    }
    fn serialize_u32(self, value: u32) -> Result<Value> {
        Ok(Value::Unsigned(u64::from(value)))
    }
    fn serialize_u64(self, value: u64) -> Result<Value> {
        Ok(Value::Unsigned(value))
    }
    fn serialize_f32(self, value: f32) -> Result<Value> {
        self.serialize_f64(f64::from(value))
    }
    fn serialize_f64(self, value: f64) -> Result<Value> {
        if value.is_finite() {
            Ok(Value::Float(value))
        } else {
            Err(SerializeError(
                "non-finite float is not a document value".to_string(),
            ))
        }
    }
    fn serialize_char(self, value: char) -> Result<Value> {
        Ok(Value::String(value.to_string()))
    }
    fn serialize_str(self, value: &str) -> Result<Value> {
        Ok(Value::String(value.to_string()))
    }
    fn serialize_bytes(self, _value: &[u8]) -> Result<Value> {
        Err(SerializeError(
            "bytes require an explicit string or sequence representation".to_string(),
        ))
    }
    fn serialize_none(self) -> Result<Value> {
        Ok(Value::Null)
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Value> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<Value> {
        Ok(Value::Null)
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Value> {
        Ok(Value::Null)
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Value> {
        Ok(Value::String(variant.to_string()))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Value> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Value> {
        let mut map = BTreeMap::new();
        map.insert(variant.to_string(), value.serialize(ValueSerializer)?);
        Ok(Value::Object(map))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<SequenceSerializer> {
        Ok(SequenceSerializer {
            values: Vec::with_capacity(len.unwrap_or(0)),
        })
    }
    fn serialize_tuple(self, len: usize) -> Result<SequenceSerializer> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<SequenceSerializer> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<VariantSequenceSerializer> {
        Ok(VariantSequenceSerializer {
            variant: variant.to_string(),
            values: Vec::with_capacity(len),
        })
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<MapSerializer> {
        Ok(MapSerializer {
            values: BTreeMap::new(),
            next_key: None,
        })
    }
    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<MapSerializer> {
        self.serialize_map(Some(len))
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<VariantMapSerializer> {
        Ok(VariantMapSerializer {
            variant: variant.to_string(),
            map: MapSerializer {
                values: BTreeMap::new(),
                next_key: None,
            },
        })
    }
}

use serde::ser::{
    SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};
use std::collections::BTreeMap;

struct SequenceSerializer {
    values: Vec<Value>,
}
impl SerializeSeq for SequenceSerializer {
    type Ok = Value;
    type Error = SerializeError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.values.push(value.serialize(ValueSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Value> {
        Ok(Value::Array(self.values))
    }
}
impl SerializeTuple for SequenceSerializer {
    type Ok = Value;
    type Error = SerializeError;
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.values.push(value.serialize(ValueSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Value> {
        Ok(Value::Array(self.values))
    }
}
impl SerializeTupleStruct for SequenceSerializer {
    type Ok = Value;
    type Error = SerializeError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.values.push(value.serialize(ValueSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Value> {
        Ok(Value::Array(self.values))
    }
}

struct VariantSequenceSerializer {
    variant: String,
    values: Vec<Value>,
}
impl SerializeTupleVariant for VariantSequenceSerializer {
    type Ok = Value;
    type Error = SerializeError;
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.values.push(value.serialize(ValueSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Value> {
        let mut map = BTreeMap::new();
        map.insert(self.variant, Value::Array(self.values));
        Ok(Value::Object(map))
    }
}

struct MapSerializer {
    values: BTreeMap<String, Value>,
    next_key: Option<String>,
}
impl SerializeMap for MapSerializer {
    type Ok = Value;
    type Error = SerializeError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<()> {
        let Value::String(key) = key.serialize(ValueKeySerializer)? else {
            return Err(SerializeError("map keys must be strings".to_string()));
        };
        self.next_key = Some(key);
        Ok(())
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        let key = self
            .next_key
            .take()
            .ok_or_else(|| SerializeError("map value without key".to_string()))?;
        self.values.insert(key, value.serialize(ValueSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Value> {
        Ok(Value::Object(self.values))
    }
}
impl SerializeStruct for MapSerializer {
    type Ok = Value;
    type Error = SerializeError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        self.values
            .insert(key.to_string(), value.serialize(ValueSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Value> {
        Ok(Value::Object(self.values))
    }
}

struct VariantMapSerializer {
    variant: String,
    map: MapSerializer,
}
impl SerializeStructVariant for VariantMapSerializer {
    type Ok = Value;
    type Error = SerializeError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        self.map.serialize_field(key, value)
    }
    fn end(self) -> Result<Value> {
        let mut outer = BTreeMap::new();
        outer.insert(self.variant, Value::Object(self.map.values));
        Ok(Value::Object(outer))
    }
}

struct ValueKeySerializer;
impl serde::Serializer for ValueKeySerializer {
    type Ok = Value;
    type Error = SerializeError;
    type SerializeSeq = ser::Impossible<Value, SerializeError>;
    type SerializeTuple = ser::Impossible<Value, SerializeError>;
    type SerializeTupleStruct = ser::Impossible<Value, SerializeError>;
    type SerializeTupleVariant = ser::Impossible<Value, SerializeError>;
    type SerializeMap = ser::Impossible<Value, SerializeError>;
    type SerializeStruct = ser::Impossible<Value, SerializeError>;
    type SerializeStructVariant = ser::Impossible<Value, SerializeError>;
    fn serialize_str(self, value: &str) -> Result<Value> {
        Ok(Value::String(value.to_string()))
    }
    fn serialize_bool(self, _: bool) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_i8(self, _: i8) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_i16(self, _: i16) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_i32(self, _: i32) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_i64(self, _: i64) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_u8(self, _: u8) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_u16(self, _: u16) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_u32(self, _: u32) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_u64(self, _: u64) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_f32(self, _: f32) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_f64(self, _: f64) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_char(self, _: char) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_bytes(self, _: &[u8]) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_none(self) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_some<T: ?Sized + Serialize>(self, _: &T) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_unit(self) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Value> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant> {
        Err(SerializeError("map keys must be strings".to_string()))
    }
}

/// Best-effort numeric visit for a [`Value::Number`] literal: integer first
/// (covers a magnitude-only-oversized integer literal), else lossy `f64`.
fn visit_number_literal<'de, V: de::Visitor<'de>>(
    text: &str,
    visitor: V,
) -> Result<V::Value, serde::de::value::Error> {
    if let Ok(value) = text.parse::<i64>() {
        return visitor.visit_i64(value);
    }
    if let Ok(value) = text.parse::<u64>() {
        return visitor.visit_u64(value);
    }
    match text.parse::<f64>() {
        Ok(value) => visitor.visit_f64(value),
        Err(_) => Err(de::Error::custom(format!(
            "invalid numeric literal `{text}`"
        ))),
    }
}

struct ValueDeserializer<'a>(&'a Value);

macro_rules! string_number {
    ($method:ident, $type:ty, $visit:ident) => {
        fn $method<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            match self.0 {
                Value::String(value) => value
                    .parse::<$type>()
                    .map_err(|_| de::Error::custom("invalid numeric string"))
                    .and_then(|value| visitor.$visit(value)),
                _ => self.deserialize_any(visitor),
            }
        }
    };
}

impl<'de, 'a> serde::Deserializer<'de> for ValueDeserializer<'a>
where
    'a: 'de,
{
    type Error = serde::de::value::Error;

    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.0 {
            Value::Null => visitor.visit_unit(),
            Value::Bool(value) => visitor.visit_bool(*value),
            Value::Integer(value) => visitor.visit_i64(*value),
            Value::Unsigned(value) => visitor.visit_u64(*value),
            Value::Float(value) => visitor.visit_f64(*value),
            // A `Value::Number` literal only exists because it does not fit
            // `i64`/`u64`/`f64` cleanly at the source-text level (e.g. an
            // integer beyond `u64::MAX`), but a typed Rust struct field can
            // only ever hold one of those three widths anyway — so a
            // best-effort numeric visit (integer first, else lossy `f64`) is
            // the most faithful a generic typed decode can be here. Callers
            // that need the exact digits read the untyped `Value` via
            // `Value::as_number_literal` instead of decoding through serde.
            Value::Number(text) => visit_number_literal(text, visitor),
            Value::String(value) => visitor.visit_string(value.clone()),
            Value::Array(values) => visitor.visit_seq(BorrowedSeqAccess {
                values: values.iter(),
            }),
            Value::Object(values) => visitor.visit_map(BorrowedMapAccess {
                values: values.iter(),
                pending: None,
            }),
        }
    }

    fn deserialize_option<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        if self.0.is_null() {
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    fn deserialize_enum<V: serde::de::Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.0 {
            Value::String(variant) => visitor.visit_enum(EnumAccess {
                variant: variant.clone(),
                value: None,
            }),
            Value::Object(values) if values.len() == 1 => {
                let mut entries = values.iter();
                if let Some((variant, value)) = entries.next() {
                    visitor.visit_enum(EnumAccess {
                        variant: variant.clone(),
                        value: Some(value),
                    })
                } else {
                    Err(de::Error::custom("missing enum variant"))
                }
            }
            _ => Err(de::Error::custom(
                "expected enum variant string or single-key object",
            )),
        }
    }

    fn deserialize_bool<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.0 {
            Value::String(value) => match value.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => visitor.visit_bool(true),
                "false" | "no" | "off" | "0" => visitor.visit_bool(false),
                _ => Err(de::Error::custom("invalid boolean string")),
            },
            _ => self.deserialize_any(visitor),
        }
    }

    string_number!(deserialize_i8, i8, visit_i8);
    string_number!(deserialize_i16, i16, visit_i16);
    string_number!(deserialize_i32, i32, visit_i32);
    string_number!(deserialize_i64, i64, visit_i64);
    string_number!(deserialize_i128, i128, visit_i128);
    string_number!(deserialize_u8, u8, visit_u8);
    string_number!(deserialize_u16, u16, visit_u16);
    string_number!(deserialize_u32, u32, visit_u32);
    string_number!(deserialize_u64, u64, visit_u64);
    string_number!(deserialize_u128, u128, visit_u128);
    string_number!(deserialize_f32, f32, visit_f32);
    string_number!(deserialize_f64, f64, visit_f64);

    serde::forward_to_deserialize_any! {
        char str string bytes byte_buf
        unit unit_struct newtype_struct seq tuple tuple_struct map struct identifier ignored_any
    }
}

struct BorrowedSeqAccess<'a> {
    values: std::slice::Iter<'a, Value>,
}

impl<'de, 'a> serde::de::SeqAccess<'de> for BorrowedSeqAccess<'a>
where
    'a: 'de,
{
    type Error = serde::de::value::Error;
    fn next_element_seed<T: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        self.values
            .next()
            .map(|value| seed.deserialize(ValueDeserializer(value)))
            .transpose()
    }
}

struct BorrowedMapAccess<'a> {
    values: std::collections::btree_map::Iter<'a, String, Value>,
    pending: Option<&'a Value>,
}

impl<'de, 'a> serde::de::MapAccess<'de> for BorrowedMapAccess<'a>
where
    'a: 'de,
{
    type Error = serde::de::value::Error;
    fn next_key_seed<K: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        match self.values.next() {
            Some((key, value)) => {
                self.pending = Some(value);
                seed.deserialize(serde::de::IntoDeserializer::into_deserializer(key.clone()))
                    .map(Some)
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<V: serde::de::DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let value = self
            .pending
            .take()
            .ok_or_else(|| de::Error::custom("map value without key"))?;
        seed.deserialize(ValueDeserializer(value))
    }
}

struct EnumAccess<'a> {
    variant: String,
    value: Option<&'a Value>,
}

impl<'de, 'a> serde::de::EnumAccess<'de> for EnumAccess<'a>
where
    'a: 'de,
{
    type Error = serde::de::value::Error;
    type Variant = VariantAccess<'a>;
    fn variant_seed<V: serde::de::DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Self::Error> {
        let variant =
            seed.deserialize(serde::de::IntoDeserializer::into_deserializer(self.variant))?;
        Ok((variant, VariantAccess { value: self.value }))
    }
}

struct VariantAccess<'a> {
    value: Option<&'a Value>,
}

#[cfg(test)]
mod tests {
    use super::{from_value, to_value};
    use crate::document::Value;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Config {
        name: String,
        enabled: Option<bool>,
        #[serde(flatten)]
        extra: Extra,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Extra {
        count: u64,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    enum Mode {
        Fast,
        Custom { level: u32 },
    }

    #[test]
    fn round_trips_struct_optional_flatten_and_enum() {
        let value = Value::Object(std::collections::BTreeMap::from([
            ("name".to_string(), Value::String("demo".to_string())),
            ("enabled".to_string(), Value::Null),
            ("count".to_string(), Value::Unsigned(9)),
        ]));
        let config: Config = from_value(&value, "root").unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(config.name, "demo");
        assert_eq!(config.enabled, None);
        assert_eq!(config.extra.count, 9);
        let mode = Mode::Custom { level: 3 };
        assert_eq!(
            from_value(&to_value(&mode).unwrap_or(Value::Null), "mode").unwrap_or(Mode::Fast),
            mode
        );
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Strict {
        #[allow(dead_code)]
        name: String,
    }

    #[test]
    fn reports_typed_path_and_unknown_field() {
        let value = Value::Object(std::collections::BTreeMap::from([
            ("name".to_string(), Value::String("demo".to_string())),
            ("extra".to_string(), Value::Bool(true)),
        ]));
        let error = from_value::<Strict>(&value, "settings").expect_err("unknown field must fail");
        assert!(error.to_string().contains("settings"));
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn directs_string_leaves_into_typed_scalars() {
        let value = Value::Object(std::collections::BTreeMap::from([
            ("enabled".to_string(), Value::String("yes".to_string())),
            ("count".to_string(), Value::String("42".to_string())),
        ]));
        #[derive(Debug, Deserialize, PartialEq)]
        struct StringBacked {
            enabled: bool,
            count: u64,
        }
        assert_eq!(
            from_value::<StringBacked>(&value, "env").unwrap_or_else(|error| panic!("{error}")),
            StringBacked {
                enabled: true,
                count: 42
            }
        );
    }
}
impl<'de, 'a> serde::de::VariantAccess<'de> for VariantAccess<'a>
where
    'a: 'de,
{
    type Error = serde::de::value::Error;
    fn unit_variant(self) -> Result<(), Self::Error> {
        if self.value.is_none() {
            Ok(())
        } else {
            Err(de::Error::custom("expected unit variant"))
        }
    }
    fn newtype_variant_seed<T: serde::de::DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, Self::Error> {
        seed.deserialize(ValueDeserializer(
            self.value
                .ok_or_else(|| de::Error::custom("missing newtype value"))?,
        ))
    }
    fn tuple_variant<V: serde::de::Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        ValueDeserializer(
            self.value
                .ok_or_else(|| de::Error::custom("missing tuple value"))?,
        )
        .deserialize_seq(visitor)
    }
    fn struct_variant<V: serde::de::Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        ValueDeserializer(
            self.value
                .ok_or_else(|| de::Error::custom("missing struct value"))?,
        )
        .deserialize_map(visitor)
    }
}
