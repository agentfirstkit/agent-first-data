//! YAML format backend. Reads via the noyalib `Value` view; mutates via the
//! lossless `cst::Document` editor so comments, ordering, styles, and untouched
//! source bytes are preserved.

use crate::document::{DocumentError, DocumentResult, Value};
use noyalib::{
    DuplicateKeyPolicy, Mapping as YamlMapping, ParserConfig, Value as YamlValue,
    cst::parse_document, from_str_with_config, to_string,
};

pub fn load(content: &str) -> DocumentResult<Value> {
    let parser_config = ParserConfig::new()
        .duplicate_key_policy(DuplicateKeyPolicy::Error)
        .lossless_u64_integers(true);
    from_str_with_config::<YamlValue>(content, &parser_config)
        .map(value_to_our_value)
        .map_err(|e| DocumentError::ParseError {
            format: "YAML".to_string(),
            detail: e.to_string(),
        })
}

/// Replace one scalar in a YAML source document while retaining all unrelated
/// source bytes. Escaped keys, keyed-list routes, and collection replacement
/// are deliberately rejected until they have a lossless CST path adapter.
pub fn set_scalar_preserving(content: &str, path: &str, value: &Value) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    let yaml_path = cst_path(&segments, "set")?;
    if !matches!(
        value,
        Value::Null
            | Value::Bool(_)
            | Value::Integer(_)
            | Value::Unsigned(_)
            | Value::Float(_)
            | Value::String(_)
    ) {
        return Err(DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "set".to_string(),
            detail: "collection mutation requires a dedicated CST fragment editor".to_string(),
        });
    }
    let mut document = parse_document(content).map_err(|error| DocumentError::ParseError {
        format: "YAML".to_string(),
        detail: error.to_string(),
    })?;
    // Existing leaf: replace in place (preserves the scalar's style). Missing
    // leaf under an existing mapping: splice a sibling entry via `insert_entry`.
    let exists = load(content)
        .ok()
        .is_some_and(|loaded| crate::document::get_path_ref(&loaded, path, &[]).is_ok());
    if exists {
        document
            .set_value(&yaml_path, &to_noyalib_value(value)?)
            .map_err(|error| DocumentError::UnsupportedOperation {
                format: "YAML".to_string(),
                operation: "set".to_string(),
                detail: error.to_string(),
            })?;
    } else {
        let (last, parents) = segments.split_last().ok_or(DocumentError::EmptyPath)?;
        if last.parse::<usize>().is_ok() {
            return Err(DocumentError::UnsupportedOperation {
                format: "YAML".to_string(),
                operation: "set".to_string(),
                detail: "cannot create a new sequence index; the element must already exist"
                    .to_string(),
            });
        }
        let parent_path = if parents.is_empty() {
            String::new()
        } else {
            cst_path(parents, "set")?
        };
        let fragment = to_string(&to_noyalib_value(value)?).map_err(|error| {
            DocumentError::UnsupportedOperation {
                format: "YAML".to_string(),
                operation: "set".to_string(),
                detail: error.to_string(),
            }
        })?;
        document
            .insert_entry(&parent_path, last, fragment.trim_end())
            .map_err(|error| DocumentError::UnsupportedOperation {
                format: "YAML".to_string(),
                operation: "set".to_string(),
                detail: error.to_string(),
            })?;
    }
    document
        .validate()
        .map_err(|error| DocumentError::ParseError {
            format: "YAML".to_string(),
            detail: error.to_string(),
        })?;
    Ok(document.to_string())
}

/// Remove an existing YAML entry through the lossless CST editor.
pub fn unset_preserving(content: &str, path: &str) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    let yaml_path = cst_path(&segments, "unset")?;
    let mut document = parse_document(content).map_err(|error| DocumentError::ParseError {
        format: "YAML".to_string(),
        detail: error.to_string(),
    })?;
    document
        .remove(&yaml_path)
        .map_err(|error| DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "unset".to_string(),
            detail: error.to_string(),
        })?;
    document
        .validate()
        .map_err(|error| DocumentError::ParseError {
            format: "YAML".to_string(),
            detail: error.to_string(),
        })?;
    Ok(document.to_string())
}

/// Append an item to an existing block YAML sequence using the CST's
/// indentation-aware editor.
pub fn append_array_item_preserving(
    content: &str,
    path: &str,
    item: &Value,
) -> DocumentResult<String> {
    let mut document = parse_document(content).map_err(|error| DocumentError::ParseError {
        format: "YAML".to_string(),
        detail: error.to_string(),
    })?;
    let fragment = to_string(&to_noyalib_value(item)?).map_err(|error| {
        DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "add".to_string(),
            detail: error.to_string(),
        }
    })?;
    let yaml_path = cst_path(&crate::document::parse_path(path)?, "add")?;
    document
        .push_back(&yaml_path, fragment.trim_end())
        .map_err(|error| DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "add".to_string(),
            detail: error.to_string(),
        })?;
    document
        .validate()
        .map_err(|error| DocumentError::ParseError {
            format: "YAML".to_string(),
            detail: error.to_string(),
        })?;
    Ok(document.to_string())
}

/// Remove one item from a YAML sequence by numeric index.
pub fn remove_array_item_preserving(
    content: &str,
    path: &str,
    index: usize,
) -> DocumentResult<String> {
    let mut document = parse_document(content).map_err(|error| DocumentError::ParseError {
        format: "YAML".to_string(),
        detail: error.to_string(),
    })?;
    let yaml_path = cst_path(&crate::document::parse_path(path)?, "remove")?;
    document
        .remove(&format!("{yaml_path}[{index}]"))
        .map_err(|error| DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "remove".to_string(),
            detail: error.to_string(),
        })?;
    document
        .validate()
        .map_err(|error| DocumentError::ParseError {
            format: "YAML".to_string(),
            detail: error.to_string(),
        })?;
    Ok(document.to_string())
}

fn cst_path(segments: &[String], operation: &str) -> DocumentResult<String> {
    if segments.is_empty() {
        return Err(DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: operation.to_string(),
            detail: "root mutation is not supported by the CST path adapter".to_string(),
        });
    }
    let mut path = String::new();
    for segment in segments {
        if segment.contains(['.', '\\']) {
            return Err(DocumentError::UnsupportedOperation {
                format: "YAML".to_string(),
                operation: operation.to_string(),
                detail: "escaped YAML keys require a quoted-key CST span and are not supported"
                    .to_string(),
            });
        }
        if let Ok(index) = segment.parse::<usize>() {
            path.push_str(&format!("[{index}]"));
        } else {
            if !path.is_empty() {
                path.push('.');
            }
            path.push_str(segment);
        }
    }
    Ok(path)
}

fn to_noyalib_value(value: &Value) -> DocumentResult<YamlValue> {
    match value {
        Value::Null => Ok(YamlValue::Null),
        Value::Bool(value) => Ok(YamlValue::Bool(*value)),
        Value::Integer(value) => Ok(YamlValue::from(*value)),
        Value::Unsigned(value) => Ok(YamlValue::from(*value)),
        Value::Float(value) if value.is_finite() => Ok(YamlValue::from(*value)),
        Value::Float(_) => Err(DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "set".to_string(),
            detail: "non-finite YAML float is not representable".to_string(),
        }),
        Value::String(value) => Ok(YamlValue::String(value.clone())),
        Value::Array(values) => Ok(YamlValue::Sequence(
            values
                .iter()
                .map(to_noyalib_value)
                .collect::<DocumentResult<Vec<_>>>()?,
        )),
        Value::Object(values) => {
            let mut mapping = YamlMapping::new();
            for (key, value) in values {
                mapping.insert(key.clone(), to_noyalib_value(value)?);
            }
            Ok(YamlValue::Mapping(mapping))
        }
    }
}

pub fn save(value: &Value) -> DocumentResult<String> {
    let yaml_val = our_value_to_yaml_value(value)?;
    to_string(&yaml_val).map_err(|e| DocumentError::ParseError {
        format: "YAML".to_string(),
        detail: e.to_string(),
    })
}

fn value_to_our_value(v: YamlValue) -> Value {
    match v {
        YamlValue::Null => Value::Null,
        YamlValue::Bool(b) => Value::Bool(b),
        YamlValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i)
            } else if let Some(u) = n.as_u64() {
                Value::Unsigned(u)
            } else {
                Value::Float(n.as_f64())
            }
        }
        YamlValue::String(s) => Value::String(s),
        YamlValue::Sequence(seq) => Value::Array(seq.into_iter().map(value_to_our_value).collect()),
        YamlValue::Mapping(map) => {
            let mut obj = std::collections::BTreeMap::new();
            for (key, value) in map {
                obj.insert(key, value_to_our_value(value));
            }
            Value::Object(obj)
        }
        YamlValue::Tagged(t) => {
            // Tagged values: recurse on inner value
            let (_, value) = t.into_parts();
            value_to_our_value(value)
        }
    }
}

fn our_value_to_yaml_value(v: &Value) -> DocumentResult<YamlValue> {
    match v {
        Value::Null => Ok(YamlValue::Null),
        Value::Bool(b) => Ok(YamlValue::Bool(*b)),
        Value::Integer(i) => Ok(YamlValue::Number((*i).into())),
        Value::Unsigned(i) => Ok(YamlValue::Number((*i).into())),
        Value::Float(f) => {
            // Keep the existing config representation: floats serialize as scalars.
            Ok(YamlValue::Number((*f).into()))
        }
        Value::String(s) => Ok(YamlValue::String(s.clone())),
        Value::Array(a) => {
            let seq = a
                .iter()
                .map(our_value_to_yaml_value)
                .collect::<DocumentResult<Vec<_>>>()?;
            Ok(YamlValue::Sequence(seq))
        }
        Value::Object(o) => {
            let mut mapping = YamlMapping::new();
            for (k, v) in o {
                mapping.insert(k.clone(), our_value_to_yaml_value(v)?);
            }
            Ok(YamlValue::Mapping(mapping))
        }
    }
}
