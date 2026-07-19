//! TOML format backend (format-preserving via toml_edit).

use crate::document::{DocumentError, DocumentResult, Value};

/// Edit an existing scalar item in a TOML document without reserializing the
/// surrounding document. Comments, ordering, whitespace, and datetime syntax
/// outside the target remain owned by `toml_edit::DocumentMut`.
pub fn set_preserving(content: &str, path: &str, value: &Value) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    if segments.iter().any(|segment| segment.contains(['.', '\\'])) {
        return Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "set".to_string(),
            detail: "escaped TOML keys are not supported by the current document path adapter"
                .to_string(),
        });
    }
    let mut document =
        content
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| DocumentError::ParseError {
                format: "TOML".to_string(),
                detail: error.to_string(),
            })?;
    let item = toml_item(value)?;
    let (last, parents) = segments.split_last().ok_or(DocumentError::EmptyPath)?;
    let mut current = document.as_item_mut();
    for parent in parents {
        // Auto-create a missing intermediate table so a sparse config can grow
        // (`set imap.host` when `[imap]` is absent), matching `set_path`.
        // toml_edit returns `Some(Item::None)` for absent keys, so treat that as
        // a genuinely missing parent rather than a navigable node.
        {
            let table =
                current
                    .as_table_like_mut()
                    .ok_or_else(|| DocumentError::UnsupportedOperation {
                        format: "TOML".to_string(),
                        operation: "set".to_string(),
                        detail: "cannot address a key inside a non-table TOML value".to_string(),
                    })?;
            if table.get(parent).filter(|item| !item.is_none()).is_none() {
                let mut created = toml_edit::Table::new();
                created.set_implicit(true);
                table.insert(parent, toml_edit::Item::Table(created));
            }
        }
        current = current
            .get_mut(parent)
            .filter(|item| !item.is_none())
            .ok_or_else(|| DocumentError::PathNotFound {
                path: path.to_string(),
            })?;
    }
    let table = current
        .as_table_like_mut()
        .ok_or_else(|| DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "set".to_string(),
            detail: "cannot address a key inside a non-table TOML value".to_string(),
        })?;
    match table.get_mut(last).filter(|item| !item.is_none()) {
        Some(target) => {
            if !target.is_value() {
                return Err(DocumentError::UnsupportedOperation {
                    format: "TOML".to_string(),
                    operation: "set".to_string(),
                    detail: "only existing scalar TOML values are supported by the document editor"
                        .to_string(),
                });
            }
            let decor = target.as_value().map(|value| value.decor().clone());
            *target = item;
            if let (Some(decor), Some(value)) = (decor, target.as_value_mut()) {
                *value.decor_mut() = decor;
            }
        }
        // New leaf: append into the (existing) parent table. Intermediate parent
        // tables are not auto-created — a missing parent fails above.
        None => {
            table.insert(last, item);
        }
    }
    Ok(document.to_string())
}

/// Remove an existing TOML item through `toml_edit`, retaining document decor.
pub fn unset_preserving(content: &str, path: &str) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    if segments.iter().any(|segment| segment.contains(['.', '\\'])) {
        return Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "unset".to_string(),
            detail: "escaped TOML keys are not supported by the current document path adapter"
                .to_string(),
        });
    }
    let (last, parents) = segments.split_last().ok_or(DocumentError::EmptyPath)?;
    let mut document =
        content
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| DocumentError::ParseError {
                format: "TOML".to_string(),
                detail: error.to_string(),
            })?;
    let mut current = document.as_item_mut();
    for parent in parents {
        current = current
            .get_mut(parent)
            .ok_or_else(|| DocumentError::PathNotFound {
                path: path.to_string(),
            })?;
    }
    let table = current
        .as_table_mut()
        .ok_or_else(|| DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "unset".to_string(),
            detail: "only table entries can be removed by the current TOML editor".to_string(),
        })?;
    if table.remove(last).is_none() {
        return Err(DocumentError::PathNotFound {
            path: path.to_string(),
        });
    }
    Ok(document.to_string())
}

fn toml_item(value: &Value) -> DocumentResult<toml_edit::Item> {
    match value {
        Value::Null => Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "set".to_string(),
            detail: "TOML has no null value".to_string(),
        }),
        Value::Bool(value) => Ok(toml_edit::value(*value)),
        Value::Integer(value) => Ok(toml_edit::value(*value)),
        Value::Unsigned(value) => i64::try_from(*value).map(toml_edit::value).map_err(|_| {
            DocumentError::UnsupportedOperation {
                format: "TOML".to_string(),
                operation: "set".to_string(),
                detail: "unsigned integer exceeds TOML i64 range".to_string(),
            }
        }),
        Value::Float(value) if value.is_finite() => Ok(toml_edit::value(*value)),
        Value::Float(_) => Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "set".to_string(),
            detail: "non-finite TOML float is not representable".to_string(),
        }),
        // A `Value::Number` literal is float-shaped (has a `.`/`e`) or
        // integer-shaped (does not); an integer-shaped one only exists
        // because it overflows `u64`, which also overflows TOML's 64-bit
        // integer grammar, so it can never be written. A float-shaped one
        // parses to `f64` cleanly (it already passed JSON-number syntax
        // validation) and writes like any other TOML float — TOML floats
        // are canonically `f64` at the format level, so this is not a
        // fidelity regression versus `Value::Float` above.
        Value::Number(text) if value.is_float() => text
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(toml_edit::value)
            .ok_or_else(|| DocumentError::UnsupportedOperation {
                format: "TOML".to_string(),
                operation: "set".to_string(),
                detail: format!("float literal `{text}` is not representable in TOML"),
            }),
        Value::Number(text) => Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "set".to_string(),
            detail: format!("integer literal `{text}` exceeds TOML's 64-bit integer range"),
        }),
        Value::String(value) => Ok(toml_edit::value(value.clone())),
        Value::Array(_) | Value::Object(_) => Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "set".to_string(),
            detail: "collection mutation requires a dedicated TOML editor".to_string(),
        }),
    }
}

pub fn load(content: &str) -> DocumentResult<Value> {
    toml::from_str::<toml::Value>(content)
        .map(value_to_our_value)
        .map_err(|e| DocumentError::ParseError {
            format: "TOML".to_string(),
            detail: e.to_string(),
        })
}

pub fn save(value: &Value) -> DocumentResult<String> {
    let toml_val = our_value_to_toml_value(value)?;
    toml::to_string_pretty(&toml_val).map_err(|e| DocumentError::ParseError {
        format: "TOML".to_string(),
        detail: e.to_string(),
    })
}

fn value_to_our_value(v: toml::Value) -> Value {
    match v {
        toml::Value::Boolean(b) => Value::Bool(b),
        toml::Value::Integer(i) => Value::Integer(i),
        toml::Value::Float(f) => Value::Float(f),
        toml::Value::String(s) => Value::String(s),
        toml::Value::Array(a) => Value::Array(a.into_iter().map(value_to_our_value).collect()),
        toml::Value::Table(t) => {
            let map = t
                .into_iter()
                .map(|(k, v)| (k, value_to_our_value(v)))
                .collect();
            Value::Object(map)
        }
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
    }
}

fn our_value_to_toml_value(v: &Value) -> DocumentResult<toml::Value> {
    match v {
        Value::Null => Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "save".to_string(),
            detail: "TOML has no null value".to_string(),
        }),
        Value::Bool(b) => Ok(toml::Value::Boolean(*b)),
        Value::Integer(i) => Ok(toml::Value::Integer(*i)),
        Value::Unsigned(i) => i64::try_from(*i).map(toml::Value::Integer).map_err(|_| {
            DocumentError::UnsupportedOperation {
                format: "TOML".to_string(),
                operation: "save".to_string(),
                detail: "unsigned integer exceeds TOML i64 range".to_string(),
            }
        }),
        Value::Float(f) => Ok(toml::Value::Float(*f)),
        Value::Number(text) if v.is_float() => text
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(toml::Value::Float)
            .ok_or_else(|| DocumentError::UnsupportedOperation {
                format: "TOML".to_string(),
                operation: "save".to_string(),
                detail: format!("float literal `{text}` is not representable in TOML"),
            }),
        Value::Number(text) => Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "save".to_string(),
            detail: format!("integer literal `{text}` exceeds TOML's 64-bit integer range"),
        }),
        Value::String(s) => Ok(toml::Value::String(s.clone())),
        Value::Array(a) => {
            let arr = a
                .iter()
                .map(our_value_to_toml_value)
                .collect::<DocumentResult<Vec<_>>>()?;
            Ok(toml::Value::Array(arr))
        }
        Value::Object(o) => {
            let mut table = toml::map::Map::new();
            for (k, v) in o {
                table.insert(k.clone(), our_value_to_toml_value(v)?);
            }
            Ok(toml::Value::Table(table))
        }
    }
}
