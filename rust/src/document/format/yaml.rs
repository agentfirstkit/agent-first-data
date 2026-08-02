//! YAML format backend. Reads via the noyalib `Value` view; mutates via the
//! lossless `cst::Document` editor so comments, ordering, styles, and untouched
//! source bytes are preserved.

use crate::document::{Addressing, DocumentError, DocumentResult, Value};
use noyalib::{
    DuplicateKeyPolicy, Mapping as YamlMapping, ParserConfig, Value as YamlValue,
    cst::{GreenChild, GreenNode, SyntaxKind, parse_document},
    from_str_with_config, to_string,
};

pub fn load(content: &str) -> DocumentResult<Value> {
    let (protected, literals) = protect_exact_numbers(content)?;
    let parser_config = ParserConfig::new()
        .duplicate_key_policy(DuplicateKeyPolicy::Error)
        .lossless_u64_integers(true);
    from_str_with_config::<YamlValue>(&protected, &parser_config)
        .map(|value| value_to_our_value(value, &literals))
        .map_err(|e| DocumentError::ParseError {
            format: "YAML".to_string(),
            detail: e.to_string(),
        })
}

/// Replace one scalar in a YAML source document while retaining all unrelated
/// source bytes. Escaped keys, keyed-list routes, and collection replacement
/// are deliberately rejected until they have a lossless CST path adapter.
pub fn set_preserving(content: &str, path: &str, value: &Value) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    let yaml_path = cst_path(&segments, "set")?;
    guard_cst_segments(content, &segments, "set", true)?;
    if !matches!(
        value,
        Value::Null
            | Value::Bool(_)
            | Value::Integer(_)
            | Value::Unsigned(_)
            | Value::Float(_)
            | Value::Number(_)
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
    let exists = load(content).ok().is_some_and(|loaded| {
        crate::document::get_path_ref(&loaded, path, Addressing::INDEX_ONLY).is_ok()
    });
    if exists {
        let result = if let Value::Number(text) = value {
            document.set(&yaml_path, text)
        } else {
            document.set_value(&yaml_path, &to_noyalib_value(value)?)
        };
        result.map_err(|error| DocumentError::UnsupportedOperation {
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
        let fragment = yaml_fragment(value, "set")?;
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
    guard_cst_segments(content, &segments, "unset", false)?;
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
    let segments = array_path_segments(path)?;
    let yaml_path = cst_path(&segments, "add")?;
    let mut document = parse_document(content).map_err(|error| DocumentError::ParseError {
        format: "YAML".to_string(),
        detail: error.to_string(),
    })?;
    let loaded = load(content)?;
    let existing = array_at_path(&loaded, path, "add")?;
    let previous_len = existing.len();
    let mut fragment = yaml_fragment(item, "add")?;
    if previous_len > 0 {
        let item_path = format!("{yaml_path}[{}]", previous_len - 1);
        let (item_start, _) =
            document
                .span_at(&item_path)
                .ok_or_else(|| DocumentError::UnsupportedOperation {
                    format: "YAML".to_string(),
                    operation: "add".to_string(),
                    detail: "could not resolve the existing sequence-item indentation".to_string(),
                })?;
        let dash_column = preceding_sequence_dash_column(content, item_start).ok_or_else(|| {
            DocumentError::UnsupportedOperation {
                format: "YAML".to_string(),
                operation: "add".to_string(),
                detail: "only block sequences can be extended without rebuilding the document"
                    .to_string(),
            }
        })?;
        fragment = indent_continuation_lines(fragment.trim_end(), dash_column + 2);
    }
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
    let output = document.to_string();
    let edited = load(&output)?;
    let edited_items = array_at_path(&edited, path, "add")?;
    if edited_items.len() != previous_len + 1 || edited_items.last() != Some(item) {
        return Err(DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "add".to_string(),
            detail: "the source edit did not reproduce the requested keyed item".to_string(),
        });
    }
    Ok(output)
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
    let yaml_path = cst_path(&array_path_segments(path)?, "remove")?;
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

/// An empty keyed-list prefix names a root sequence. Ordinary set/unset paths
/// still pass through `parse_path` directly and therefore remain non-empty.
fn array_path_segments(path: &str) -> DocumentResult<Vec<String>> {
    if path.is_empty() {
        Ok(Vec::new())
    } else {
        crate::document::parse_path(path)
    }
}

fn array_at_path<'a>(
    value: &'a Value,
    path: &str,
    operation: &str,
) -> DocumentResult<&'a Vec<Value>> {
    let target = if path.is_empty() {
        value
    } else {
        crate::document::get_path_ref(value, path, Addressing::INDEX_ONLY)?
    };
    target
        .as_array()
        .ok_or_else(|| DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: operation.to_string(),
            detail: "target is not an array".to_string(),
        })
}

fn preceding_sequence_dash_column(content: &str, value_start: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut position = value_start;
    let dash = loop {
        position = position.checked_sub(1)?;
        match bytes.get(position)? {
            b' ' | b'\t' => {}
            b'-' => break position,
            _ => return None,
        }
    };
    let line_start = content.as_bytes()[..dash]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |newline| newline + 1);
    Some(dash - line_start)
}

fn indent_continuation_lines(fragment: &str, indent: usize) -> String {
    if !fragment.contains('\n') {
        return fragment.to_string();
    }
    let padding = " ".repeat(indent);
    let mut output = String::with_capacity(fragment.len() + indent * 4);
    for (index, line) in fragment.split('\n').enumerate() {
        if index > 0 {
            output.push('\n');
            if !line.is_empty() {
                output.push_str(&padding);
            }
        }
        output.push_str(line);
    }
    output
}

fn cst_path(segments: &[String], operation: &str) -> DocumentResult<String> {
    let mut path = String::new();
    for segment in segments {
        if segment.contains(['.', '\\', '[', ']']) {
            return Err(DocumentError::UnsupportedOperation {
                format: "YAML".to_string(),
                operation: operation.to_string(),
                detail:
                    "escaped or bracketed YAML keys require a quoted-key CST span and are not supported"
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

fn guard_cst_segments(
    content: &str,
    segments: &[String],
    operation: &str,
    allow_missing: bool,
) -> DocumentResult<()> {
    let root = load(content)?;
    let mut current = &root;
    for (index, segment) in segments.iter().enumerate() {
        match current {
            Value::Object(object) => {
                if segment.parse::<usize>().is_ok() || segment.contains(['[', ']']) {
                    return Err(DocumentError::UnsupportedOperation {
                        format: "YAML".to_string(),
                        operation: operation.to_string(),
                        detail: format!(
                            "mapping key `{segment}` is ambiguous in the CST path grammar"
                        ),
                    });
                }
                match object.get(segment) {
                    Some(next) => current = next,
                    None if allow_missing => {
                        if segments[index..]
                            .iter()
                            .any(|part| part.parse::<usize>().is_ok() || part.contains(['[', ']']))
                        {
                            return Err(DocumentError::UnsupportedOperation {
                                format: "YAML".to_string(),
                                operation: operation.to_string(),
                                detail: "a missing mapping chain contains a CST-ambiguous key"
                                    .to_string(),
                            });
                        }
                        return Ok(());
                    }
                    None => {
                        return Err(DocumentError::PathNotFound {
                            path: crate::document::join_path(&segments[..=index]),
                        });
                    }
                }
            }
            Value::Array(values) => {
                let array_index =
                    segment
                        .parse::<usize>()
                        .map_err(|_| DocumentError::UnregisteredArray {
                            path: crate::document::join_path(&segments[..index]),
                        })?;
                current =
                    values
                        .get(array_index)
                        .ok_or_else(|| DocumentError::IndexOutOfBounds {
                            path: crate::document::join_path(&segments[..index]),
                            index: array_index,
                            len: values.len(),
                        })?;
            }
            value => {
                return Err(DocumentError::NotTraversable {
                    path: crate::document::join_path(&segments[..index]),
                    got: value.kind_name().to_string(),
                });
            }
        }
    }
    Ok(())
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
        // See the identical fallback in `format::toml::toml_item`: a
        // float-shaped `Value::Number` literal parses to `f64` cleanly (YAML
        // floats are canonically `f64` at the format level too); an
        // integer-shaped one only exists because it overflows `u64`, so it
        // can never be written through noyalib's numeric `Value`.
        Value::Number(text) if value.is_float() => text
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(YamlValue::from)
            .ok_or_else(|| DocumentError::UnsupportedOperation {
                format: "YAML".to_string(),
                operation: "set".to_string(),
                detail: format!("float literal `{text}` is not representable in YAML"),
            }),
        Value::Number(text) => Err(DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "set".to_string(),
            detail: format!("integer literal `{text}` exceeds YAML's 64-bit integer range"),
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
    let mut prefix = "__AFDATA_EXACT_NUMBER_".to_string();
    while value_contains_text(value, &prefix) {
        prefix.push('_');
    }
    let mut literals = Vec::new();
    let yaml_val = our_value_to_yaml_value(value, &prefix, &mut literals)?;
    let mut output = to_string(&yaml_val).map_err(|e| DocumentError::ParseError {
        format: "YAML".to_string(),
        detail: e.to_string(),
    })?;
    for (sentinel, literal) in literals {
        output = output.replace(&sentinel, &literal);
    }
    Ok(output)
}

fn value_to_our_value(v: YamlValue, literals: &std::collections::HashMap<String, String>) -> Value {
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
        YamlValue::String(s) => literals
            .get(&s)
            .cloned()
            .map(Value::Number)
            .unwrap_or(Value::String(s)),
        YamlValue::Sequence(seq) => Value::Array(
            seq.into_iter()
                .map(|value| value_to_our_value(value, literals))
                .collect(),
        ),
        YamlValue::Mapping(map) => {
            let mut obj = std::collections::BTreeMap::new();
            for (key, value) in map {
                let key = literals.get(&key).cloned().unwrap_or(key);
                obj.insert(key, value_to_our_value(value, literals));
            }
            Value::Object(obj)
        }
        YamlValue::Tagged(t) => {
            // Tagged values: recurse on inner value
            let (_, value) = t.into_parts();
            value_to_our_value(value, literals)
        }
    }
}

fn our_value_to_yaml_value(
    v: &Value,
    prefix: &str,
    literals: &mut Vec<(String, String)>,
) -> DocumentResult<YamlValue> {
    match v {
        Value::Null => Ok(YamlValue::Null),
        Value::Bool(b) => Ok(YamlValue::Bool(*b)),
        Value::Integer(i) => Ok(YamlValue::Number((*i).into())),
        Value::Unsigned(i) => Ok(YamlValue::Number((*i).into())),
        Value::Float(f) if f.is_finite() => Ok(YamlValue::Number((*f).into())),
        Value::Float(_) => Err(DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "save".to_string(),
            detail: "non-finite float is not representable in YAML".to_string(),
        }),
        Value::Number(text) => {
            if !is_json_number(text) {
                return Err(DocumentError::UnsupportedOperation {
                    format: "YAML".to_string(),
                    operation: "save".to_string(),
                    detail: format!("invalid number literal `{text}`"),
                });
            }
            let sentinel = format!("{prefix}{}__", literals.len());
            literals.push((sentinel.clone(), text.clone()));
            Ok(YamlValue::String(sentinel))
        }
        Value::String(s) => Ok(YamlValue::String(s.clone())),
        Value::Array(a) => {
            let seq = a
                .iter()
                .map(|value| our_value_to_yaml_value(value, prefix, literals))
                .collect::<DocumentResult<Vec<_>>>()?;
            Ok(YamlValue::Sequence(seq))
        }
        Value::Object(o) => {
            let mut mapping = YamlMapping::new();
            for (k, v) in o {
                mapping.insert(k.clone(), our_value_to_yaml_value(v, prefix, literals)?);
            }
            Ok(YamlValue::Mapping(mapping))
        }
    }
}

fn yaml_fragment(value: &Value, operation: &str) -> DocumentResult<String> {
    if let Value::Number(text) = value
        && is_json_number(text)
    {
        return Ok(text.clone());
    }
    if matches!(value, Value::Array(_) | Value::Object(_)) {
        return save(value);
    }
    to_string(&to_noyalib_value(value)?).map_err(|error| DocumentError::UnsupportedOperation {
        format: "YAML".to_string(),
        operation: operation.to_string(),
        detail: error.to_string(),
    })
}

fn protect_exact_numbers(
    content: &str,
) -> DocumentResult<(String, std::collections::HashMap<String, String>)> {
    let document = parse_document(content).map_err(|error| DocumentError::ParseError {
        format: "YAML".to_string(),
        detail: error.to_string(),
    })?;
    let mut spans = Vec::new();
    collect_exact_number_spans(document.syntax(), content, 0, &mut spans);
    let mut prefix = "__AFDATA_EXACT_NUMBER_".to_string();
    while content.contains(&prefix) {
        prefix.push('_');
    }
    let mut protected = content.to_string();
    let mut literals = std::collections::HashMap::new();
    for (index, (start, end)) in spans.into_iter().enumerate().rev() {
        let literal = content[start..end].to_string();
        let sentinel = format!("{prefix}{index}__");
        let quoted =
            serde_json::to_string(&sentinel).map_err(|error| DocumentError::ParseError {
                format: "YAML".to_string(),
                detail: error.to_string(),
            })?;
        protected.replace_range(start..end, &quoted);
        literals.insert(sentinel, literal);
    }
    Ok((protected, literals))
}

fn collect_exact_number_spans(
    node: &GreenNode,
    source: &str,
    base: usize,
    spans: &mut Vec<(usize, usize)>,
) {
    let mut offset = base;
    for child in node.children() {
        match child {
            GreenChild::Node(node) => collect_exact_number_spans(node, source, offset, spans),
            GreenChild::Token {
                kind: SyntaxKind::PlainScalar,
                len,
            } => {
                let token_end = offset + *len as usize;
                let text = source[offset..token_end].trim_end_matches([' ', '\t', '\r', '\n']);
                let end = offset + text.len();
                if should_preserve_number(text) {
                    spans.push((offset, end));
                }
            }
            GreenChild::Token { .. } => {}
        }
        offset += child.text_len();
    }
}

fn should_preserve_number(text: &str) -> bool {
    if !is_json_number(text) {
        return false;
    }
    text.contains(['.', 'e', 'E']) || (text.parse::<i64>().is_err() && text.parse::<u64>().is_err())
}

fn is_json_number(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text).is_ok_and(|value| value.is_number())
}

fn value_contains_text(value: &Value, needle: &str) -> bool {
    match value {
        Value::Number(text) | Value::String(text) => text.contains(needle),
        Value::Array(values) => values
            .iter()
            .any(|value| value_contains_text(value, needle)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key.contains(needle) || value_contains_text(value, needle)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{load, save, set_preserving};
    use crate::document::Value;

    #[test]
    fn preserves_large_integer_and_high_precision_float_literals() {
        let source = concat!(
            "huge: 123456789012345678901234567890\n",
            "precise: 0.1000000000000000055511151231257827\n",
        );
        let value = load(source).expect("load");
        assert_eq!(
            value.get("huge"),
            Some(&Value::Number("123456789012345678901234567890".to_string()))
        );
        assert_eq!(
            value.get("precise"),
            Some(&Value::Number(
                "0.1000000000000000055511151231257827".to_string()
            ))
        );

        let rendered = save(&value).expect("save");
        assert!(rendered.contains("123456789012345678901234567890"));
        assert!(rendered.contains("0.1000000000000000055511151231257827"));
        assert_eq!(load(&rendered).expect("reload"), value);
    }

    #[test]
    fn exact_number_set_keeps_the_literal() {
        let edited = set_preserving(
            "price: 1.0\n",
            "price",
            &Value::Number("12345678901234567890.123456789".to_string()),
        )
        .expect("set");
        assert_eq!(edited, "price: 12345678901234567890.123456789\n");
        assert_eq!(
            load(&edited).expect("reload").get("price"),
            Some(&Value::Number("12345678901234567890.123456789".to_string()))
        );
    }

    #[test]
    fn save_rejects_non_finite_float() {
        let error = save(&Value::Float(f64::NAN)).expect_err("NaN must fail");
        assert!(error.to_string().contains("non-finite"));
    }
}
