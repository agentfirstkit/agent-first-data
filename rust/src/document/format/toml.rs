//! TOML format backend (format-preserving via toml_edit).

use crate::document::{DocumentError, DocumentResult, Value};

/// Edit an existing TOML item without reserializing the surrounding document.
///
/// Scalars, arrays, inline tables, and ordinary tables are supported. Existing
/// collection nodes are updated in place so their outer decor, element/key
/// decor, ordering, multiline layout, comments, and trailing-comma style stay
/// attached to the target wherever the replacement still has a corresponding
/// element or key. Arrays of tables remain deliberately unsupported because
/// replacing one safely requires an explicit identity policy.
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
    let (last, parents) = segments.split_last().ok_or(DocumentError::EmptyPath)?;
    let mut current = document.as_item_mut();
    for parent in parents {
        if current.is_array_of_tables() {
            return Err(collection_refusal(
                "editing an array of tables requires an explicit element identity",
            ));
        }
        if current
            .as_value()
            .and_then(toml_edit::Value::as_array)
            .is_some()
        {
            let index = parent
                .parse::<usize>()
                .map_err(|_| DocumentError::UnregisteredArray {
                    path: path.to_string(),
                })?;
            current = current
                .get_mut(index)
                .ok_or_else(|| DocumentError::PathNotFound {
                    path: path.to_string(),
                })?;
            continue;
        }
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
    if current
        .as_value()
        .and_then(toml_edit::Value::as_array)
        .is_some()
    {
        let index = last
            .parse::<usize>()
            .map_err(|_| DocumentError::UnregisteredArray {
                path: path.to_string(),
            })?;
        let target = current
            .get_mut(index)
            .ok_or_else(|| DocumentError::PathNotFound {
                path: path.to_string(),
            })?;
        replace_item_preserving(target, value)?;
    } else {
        let table =
            current
                .as_table_like_mut()
                .ok_or_else(|| DocumentError::UnsupportedOperation {
                    format: "TOML".to_string(),
                    operation: "set".to_string(),
                    detail: "cannot address a key inside a non-table TOML value".to_string(),
                })?;
        match table.get_mut(last).filter(|item| !item.is_none()) {
            Some(target) => replace_item_preserving(target, value)?,
            // New leaf: append into the existing parent table.
            None => {
                table.insert(last, toml_item(value)?);
            }
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
    // `as_table_like_mut`, not `as_table_mut`: an inline table is a `Value`, not
    // an `Item::Table`, and reading only the latter made `unset` refuse a key
    // `set` had just written — `foo = { path = "..." }` could be edited but not
    // emptied. The two verbs address the same nodes or neither is trustworthy.
    let table = current
        .as_table_like_mut()
        .ok_or_else(|| DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "unset".to_string(),
            detail: "cannot address a key inside a non-table TOML value".to_string(),
        })?;
    let Some(removed) = table.remove(last) else {
        return Err(DocumentError::PathNotFound {
            path: path.to_string(),
        });
    };
    // An inline table's closing space lives in the trailing decor of its final
    // value, so removing the last entry takes ` }` down to `}` — a change to
    // source the caller did not address. Hand that suffix to the new final
    // entry, unless it already carries one of its own.
    let removed_suffix = removed
        .as_value()
        .and_then(|value| value.decor().suffix())
        .cloned();
    if let (Some(inline), Some(suffix)) = (current.as_inline_table_mut(), removed_suffix)
        && let Some((_, final_value)) = inline.iter_mut().last()
    {
        let carries_its_own = final_value
            .decor()
            .suffix()
            .and_then(|raw| raw.as_str())
            .is_some_and(|text| !text.is_empty());
        if !carries_its_own {
            final_value.decor_mut().set_suffix(suffix);
        }
    }
    Ok(document.to_string())
}

fn toml_item(value: &Value) -> DocumentResult<toml_edit::Item> {
    toml_value(value, None).map(toml_edit::Item::Value)
}

fn toml_value(
    value: &Value,
    existing: Option<&toml_edit::Value>,
) -> DocumentResult<toml_edit::Value> {
    let mut converted = match value {
        Value::Null => Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "set".to_string(),
            detail: "TOML has no null value".to_string(),
        }),
        Value::Bool(value) => Ok(toml_edit::Value::from(*value)),
        Value::Integer(value) => Ok(toml_edit::Value::from(*value)),
        Value::Unsigned(value) => i64::try_from(*value)
            .map(toml_edit::Value::from)
            .map_err(|_| DocumentError::UnsupportedOperation {
                format: "TOML".to_string(),
                operation: "set".to_string(),
                detail: "unsigned integer exceeds TOML i64 range".to_string(),
            }),
        Value::Float(value) if value.is_finite() => Ok(toml_edit::Value::from(*value)),
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
            .map(toml_edit::Value::from)
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
        Value::String(value) => {
            if let Some(existing) = existing.filter(|item| {
                item.as_datetime()
                    .is_some_and(|datetime| datetime.to_string() == *value)
            }) {
                Ok(existing.clone())
            } else {
                Ok(toml_edit::Value::from(value.clone()))
            }
        }
        Value::Array(values) => array_value(values, existing.and_then(toml_edit::Value::as_array)),
        Value::Object(values) => {
            inline_table_value(values, existing.and_then(toml_edit::Value::as_inline_table))
        }
    }?;
    if let Some(existing) = existing {
        *converted.decor_mut() = existing.decor().clone();
    }
    Ok(converted)
}

/// Keep a decor's layout (newlines and indentation) and drop its comments.
///
/// An element's prefix decor holds whatever followed the previous element's
/// comma, which in a multiline array is that element's trailing comment. Copying
/// it verbatim onto an appended element would reproduce a comment beside a value
/// the author never wrote it for — inventing content in the one module whose
/// promise is that it preserves the author's bytes. The indentation is safe to
/// reuse; the prose is not.
fn layout_without_comments(decor: &str) -> String {
    let mut kept = String::with_capacity(decor.len());
    let mut rest = decor;
    while let Some(hash) = rest.find('#') {
        // Also drop the spaces that separated the comment from the value, or
        // removing it leaves a ragged trailing run behind.
        kept.push_str(rest[..hash].trim_end_matches([' ', '\t']));
        match rest[hash..].find('\n') {
            // Drop the comment body, keep the newline that ends it.
            Some(newline) => rest = &rest[hash + newline..],
            // A trailing comment with no newline ends the decor.
            None => return kept,
        }
    }
    kept.push_str(rest);
    kept
}

/// The first comment in a decor string, without its leading whitespace.
fn first_comment(decor: &str) -> Option<&str> {
    let start = decor.find('#')?;
    let end = decor[start..]
        .find('\n')
        .map_or(decor.len(), |newline| start + newline);
    Some(&decor[start..end])
}

/// Rewrite a decor's first comment, or remove it when `replacement` is `None`.
fn with_first_comment(decor: &str, replacement: Option<&str>) -> String {
    let Some(start) = decor.find('#') else {
        return decor.to_string();
    };
    let end = decor[start..]
        .find('\n')
        .map_or(decor.len(), |newline| start + newline);
    let mut out = String::with_capacity(decor.len());
    match replacement {
        Some(comment) => {
            out.push_str(&decor[..start]);
            out.push_str(comment);
        }
        None => out.push_str(decor[..start].trim_end_matches([' ', '\t'])),
    }
    out.push_str(&decor[end..]);
    out
}

fn array_value(
    values: &[Value],
    existing: Option<&toml_edit::Array>,
) -> DocumentResult<toml_edit::Value> {
    let existing_values = existing
        .map(|array| array.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut array = existing.cloned().unwrap_or_default();
    if array.len() > values.len() {
        // The comment written after element N-1's comma is stored in element
        // N's *prefix*, and the last element's trailing comment lives in the
        // array's trailing decor. Truncating therefore deletes the comment that
        // documented the last surviving element while keeping the one that
        // documented a value being removed — leaving a comment attached to the
        // wrong element. Carry the surviving element's own comment across.
        let surviving_comment = existing_values
            .get(values.len())
            .and_then(|item| item.decor().prefix())
            .and_then(toml_edit::RawString::as_str)
            .and_then(first_comment)
            .map(str::to_string);
        while array.len() > values.len() {
            let last = array.len() - 1;
            array.remove(last);
        }
        let trailing = array.trailing().as_str().unwrap_or_default().to_string();
        if first_comment(&trailing).is_some() {
            array.set_trailing(with_first_comment(&trailing, surviving_comment.as_deref()));
        }
    }
    let length_before_append = array.len();
    for (index, value) in values.iter().enumerate() {
        let hint = existing_values.get(index);
        let mut converted = toml_value(value, hint)?;
        if index < array.len() {
            array.replace_formatted(index, converted);
        } else {
            if let Some(prefix) = existing_values
                .last()
                .and_then(|item| item.decor().prefix())
                .and_then(toml_edit::RawString::as_str)
            {
                converted
                    .decor_mut()
                    .set_prefix(layout_without_comments(prefix));
            }
            array.push_formatted(converted);
        }
    }
    // Appending to a single-line array: the previously-last element's suffix is
    // the space that sat before `]`. Left where it is, it ends up before the new
    // comma (`[ "one" , "two"]`), so move it to the array's trailing decor where
    // it goes on separating the last value from the bracket. Done after the
    // element loop, which restores each replaced element's original decor.
    if values.len() > length_before_append
        && let Some(last_index) = length_before_append.checked_sub(1)
    {
        let suffix = array
            .get(last_index)
            .and_then(|item| item.decor().suffix())
            .and_then(toml_edit::RawString::as_str)
            .filter(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c == ' ' || c == '\t'))
            .map(str::to_string);
        let trailing_is_empty = array.trailing().as_str().is_none_or(str::is_empty);
        if let Some(suffix) = suffix
            && trailing_is_empty
        {
            if let Some(item) = array.get_mut(last_index) {
                item.decor_mut().set_suffix("");
            }
            array.set_trailing(suffix);
        }
    }
    if values.is_empty() {
        array.set_trailing_comma(false);
    }
    if existing.is_none() {
        array.fmt();
    }
    Ok(toml_edit::Value::Array(array))
}

fn inline_table_value(
    values: &std::collections::BTreeMap<String, Value>,
    existing: Option<&toml_edit::InlineTable>,
) -> DocumentResult<toml_edit::Value> {
    let mut table = existing.cloned().unwrap_or_default();
    table.retain(|key, _| values.contains_key(key));
    for (key, value) in values {
        if let Some(current) = table.get_mut(key) {
            *current = toml_value(value, Some(current))?;
        } else {
            table.insert(key, toml_value(value, None)?);
        }
    }
    if values.is_empty() {
        table.set_trailing_comma(false);
    }
    if existing.is_none() {
        table.fmt();
    }
    Ok(toml_edit::Value::InlineTable(table))
}

fn replace_item_preserving(target: &mut toml_edit::Item, value: &Value) -> DocumentResult<()> {
    if target.is_array_of_tables() {
        return Err(collection_refusal(
            "editing an array of tables requires an explicit element identity",
        ));
    }
    if let (Some(table), Value::Object(values)) = (target.as_table_mut(), value) {
        return sync_table(table, values);
    }
    let converted = toml_value(value, target.as_value())?;
    *target = toml_edit::Item::Value(converted);
    Ok(())
}

fn sync_table(
    table: &mut toml_edit::Table,
    values: &std::collections::BTreeMap<String, Value>,
) -> DocumentResult<()> {
    table.retain(|key, _| values.contains_key(key));
    for (key, value) in values {
        if let Some(current) = table.get_mut(key) {
            replace_item_preserving(current, value)?;
        } else {
            table.insert(key, toml_item(value)?);
        }
    }
    Ok(())
}

fn collection_refusal(detail: &str) -> DocumentError {
    DocumentError::UnsupportedOperation {
        format: "TOML".to_string(),
        operation: "set".to_string(),
        detail: detail.to_string(),
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
