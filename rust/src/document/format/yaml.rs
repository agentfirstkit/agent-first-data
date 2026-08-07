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

/// Replace one value in a YAML source document while retaining all unrelated
/// source bytes. Escaped keys and keyed-list routes are still rejected until
/// they have a lossless CST path adapter.
///
/// Scalars go through the CST's own `set_value`, which matches the site's
/// quoting. Collections cannot: noyalib's `set_value` refuses to grow a scalar
/// site into a sequence or mapping under any circumstances, so a collection is
/// emitted as a block fragment and spliced over the value's span — see
/// `replace_collection_in_place`, which also documents what that costs.
pub fn set_preserving(content: &str, path: &str, value: &Value) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    let yaml_path = cst_path(&segments, "set")?;
    guard_cst_segments(content, &segments, "set", true)?;
    let mut document = parse_document(content).map_err(|error| DocumentError::ParseError {
        format: "YAML".to_string(),
        detail: error.to_string(),
    })?;
    // Existing leaf: replace in place (preserves the scalar's style). Missing
    // leaf under an existing mapping: splice a sibling entry via `insert_entry`.
    let exists = load(content).ok().is_some_and(|loaded| {
        crate::document::get_path_ref(&loaded, path, Addressing::INDEX_ONLY).is_ok()
    });
    if exists && matches!(value, Value::Array(_) | Value::Object(_)) {
        replace_collection_in_place(&mut document, content, &yaml_path, value)?;
    } else if exists {
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
        let result = match value {
            // An empty collection through the typed path lands on its own line
            // (`tags:` then `  []`); the fragment spelling keeps it inline,
            // which is how anyone writes it by hand.
            Value::Array(items) if items.is_empty() => {
                document.insert_entry(&parent_path, last, "[]")
            }
            Value::Object(entries) if entries.is_empty() => {
                document.insert_entry(&parent_path, last, "{}")
            }
            // A new key is the one place noyalib will emit a collection itself,
            // indenting it under the key and rolling back if the re-parse
            // disagrees. Preferred over a hand-built fragment for exactly that.
            Value::Array(_) | Value::Object(_) => {
                document.insert_entry_value(&parent_path, last, &to_noyalib_value(value)?)
            }
            _ => document.insert_entry(&parent_path, last, yaml_fragment(value, "set")?.trim_end()),
        };
        if let Err(error) = result {
            // `insert_entry` places a new entry relative to the last existing
            // one, so it needs that entry to have a value node to measure. A
            // mapping ending in a bare key (`attachments:`, which reads as null)
            // has none, and the insert fails with nowhere to anchor. At the root
            // there is still an unambiguous answer — the end of the document,
            // which is where a person adding a key would put it.
            if parent_path.is_empty() {
                append_root_entry(&mut document, content, last, value)?;
            } else {
                return Err(DocumentError::UnsupportedOperation {
                    format: "YAML".to_string(),
                    operation: "set".to_string(),
                    detail: error.to_string(),
                });
            }
        }
    }
    document
        .validate()
        .map_err(|error| DocumentError::ParseError {
            format: "YAML".to_string(),
            detail: error.to_string(),
        })?;
    Ok(document.to_string())
}

/// Append a new top-level entry at the end of the document.
///
/// The fallback for a root mapping the CST cannot anchor an insertion to. The
/// entry is written at column zero after the last byte, with the document's own
/// newline style, and the result is validated like any other edit — so a source
/// this does not suit fails rather than silently producing something else.
fn append_root_entry(
    document: &mut noyalib::cst::Document,
    content: &str,
    key: &str,
    value: &Value,
) -> DocumentResult<()> {
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let rendered = yaml_fragment(value, "set")?;
    let rendered = rendered.trim_end_matches(['\r', '\n']);
    let separator = if content.is_empty() || content.ends_with('\n') {
        String::new()
    } else {
        newline.to_string()
    };
    let entry = if matches!(value, Value::Array(_) | Value::Object(_)) {
        // A collection cannot share the key's line; indent it one unit under.
        let pad = " ".repeat(document.indent_unit());
        let body = rendered
            .lines()
            .map(|line| format!("{newline}{pad}{line}"))
            .collect::<String>();
        format!("{separator}{key}:{body}{newline}")
    } else {
        format!("{separator}{key}: {rendered}{newline}")
    };
    let end = content.len();
    document
        .replace_span(end, end, &entry)
        .map_err(|error| DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "set".to_string(),
            detail: error.to_string(),
        })
}

/// Splice a block-style collection over the span of an existing value.
///
/// noyalib's typed `set_value` refuses a sequence or mapping outright, so the
/// only route is a fragment written over the value's byte span. Three details
/// make that safe rather than approximate:
///
/// * The start widens back over the whitespace between the `:` and the old
///   value, because a block collection has to begin on its own line — spliced
///   at the raw span start it would either sit at column 0 or leave the source's
///   own newline behind as a blank line.
/// * The end widens over a trailing `#` comment. The span the CST reports stops
///   before it, so leaving it would re-attach a comment written about the *old*
///   last element to whatever the new one happens to be. Silently moving a
///   comment onto a different value is worse than dropping it with the value it
///   described.
/// * A path reached through an alias is refused. `span_at` resolves through
///   aliases to the anchor, so editing `other: *b` would rewrite `base: &b`
///   instead — a different key than the caller named.
///
/// The cost, which has no workaround in this version: comments and blank lines
/// *inside* the replaced collection are dropped, because no API models them as
/// movable. Everything outside the span — surrounding keys, their comments,
/// key order, quote styles, the trailing newline — is preserved byte for byte.
fn replace_collection_in_place(
    document: &mut noyalib::cst::Document,
    content: &str,
    yaml_path: &str,
    value: &Value,
) -> DocumentResult<()> {
    let unsupported = |detail: String| DocumentError::UnsupportedOperation {
        format: "YAML".to_string(),
        operation: "set".to_string(),
        detail,
    };
    let (value_start, value_end) = document
        .span_at(yaml_path)
        .ok_or_else(|| unsupported(format!("could not locate the value at `{yaml_path}`")))?;
    let (key_start, key_end) = document
        .key_span(yaml_path)
        .ok_or_else(|| unsupported(format!("could not locate the key at `{yaml_path}`")))?;
    if value_start < key_end {
        return Err(unsupported(format!(
            "`{yaml_path}` resolves through a YAML alias; editing it would rewrite the anchor \
             instead. Materialize the alias first."
        )));
    }

    let bytes = content.as_bytes();
    let key_column = line_column_of(content, key_start);
    let indent = key_column + document.indent_unit();
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };

    let mut start = value_start;
    while start > 0 && matches!(bytes[start - 1], b' ' | b'\t' | b'\n' | b'\r') {
        start -= 1;
    }
    let end = end_of_trailing_inline_comment(content, value_end);

    let fragment = block_fragment(value, document.indent_unit(), indent, newline)?;
    document
        .replace_span(start, end, &fragment)
        .map_err(|error| unsupported(error.to_string()))
}

/// Render `value` as the bytes that go after a mapping key.
///
/// An empty collection stays inline (` []`), because a bare `key:` parses as
/// null rather than as an empty sequence — the space is load-bearing. Anything
/// else becomes a block, preceded by a newline and indented under its key.
fn block_fragment(
    value: &Value,
    indent_unit: usize,
    indent: usize,
    newline: &str,
) -> DocumentResult<String> {
    match value {
        Value::Array(items) if items.is_empty() => return Ok(" []".to_string()),
        Value::Object(entries) if entries.is_empty() => return Ok(" {}".to_string()),
        _ => {}
    }
    let config = noyalib::SerializerConfig::new()
        .indent(indent_unit)
        .flow_style(noyalib::FlowStyle::Block);
    // Emitted by noyalib rather than by hand: element quoting has to survive
    // values like `true`, `123`, `a: b` and `#c`, which are only scalars
    // because they are quoted.
    let rendered = noyalib::to_string_value_with_config(&to_noyalib_value(value)?, &config)
        .map_err(|error| DocumentError::UnsupportedOperation {
            format: "YAML".to_string(),
            operation: "set".to_string(),
            detail: error.to_string(),
        })?;
    let pad = " ".repeat(indent);
    let mut fragment = String::new();
    for line in rendered.trim_end_matches(['\r', '\n']).lines() {
        fragment.push_str(newline);
        if line.is_empty() {
            continue;
        }
        fragment.push_str(&pad);
        fragment.push_str(line);
    }
    Ok(fragment)
}

/// Column of `offset` within its line, counting bytes from the line start.
fn line_column_of(content: &str, offset: usize) -> usize {
    let line_start = content[..offset].rfind('\n').map_or(0, |index| index + 1);
    offset - line_start
}

/// End of the `#` comment trailing the value that ends at `value_end`, or
/// `value_end` when the rest of the line holds anything else.
fn end_of_trailing_inline_comment(content: &str, value_end: usize) -> usize {
    let bytes = content.as_bytes();
    let mut index = value_end;
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t') {
        index += 1;
    }
    if index >= bytes.len() || bytes[index] != b'#' {
        return value_end;
    }
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    if index > 0 && bytes[index - 1] == b'\r' {
        index -= 1;
    }
    index
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
    if previous_len == 0 {
        // The CST's `push_back` needs an existing item to take indentation
        // from, so it refuses on `tags: []`. Writing the whole one-element
        // sequence gets there instead, and costs nothing: an empty collection
        // has no interior comments or layout to preserve.
        return set_preserving(content, path, &Value::Array(vec![item.clone()]));
    }
    let mut fragment = yaml_fragment(item, "add")?;
    {
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
    let segments = array_path_segments(path)?;
    let loaded = load(content)?;
    if array_at_path(&loaded, path, "remove")?.len() == 1 && index == 0 {
        // The CST refuses to delete a sequence's only entry — removing the last
        // `- item` line would leave a bare `key:`, which reads back as null
        // rather than as an empty sequence. Writing the empty sequence says
        // that outright, and mirrors the append path, which likewise falls back
        // to a whole-value write when there is no item to anchor to.
        return set_preserving(content, path, &Value::Array(Vec::new()));
    }
    let mut document = parse_document(content).map_err(|error| DocumentError::ParseError {
        format: "YAML".to_string(),
        detail: error.to_string(),
    })?;
    let yaml_path = cst_path(&segments, "remove")?;
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

    /// The shape that motivated the fragment editor: a frontmatter block a
    /// human has edited, where one collection field changes.
    fn contact_source() -> &'static str {
        "# a leading comment\ndisplay_name: Alice Example\nkind: contact\nemails:\n  - alice@example.com  # work\ntags: []\nrole: ''\n"
    }

    #[test]
    fn replacing_a_collection_keeps_every_other_byte() {
        let edited = set_preserving(
            contact_source(),
            "tags",
            &Value::Array(vec![Value::String("vip".into())]),
        )
        .expect("an empty sequence can grow");
        assert_eq!(
            edited,
            "# a leading comment\ndisplay_name: Alice Example\nkind: contact\nemails:\n  - alice@example.com  # work\ntags:\n  - vip\nrole: ''\n"
        );

        // Back to empty, and the inline spelling returns rather than a bare key
        // (which would parse as null) or a two-line `tags:\n  []`.
        let cleared = set_preserving(&edited, "tags", &Value::Array(Vec::new()))
            .expect("a sequence can shrink to empty");
        assert_eq!(cleared, contact_source());
    }

    #[test]
    fn replacing_a_collection_quotes_elements_that_need_it() {
        let edited = set_preserving(
            contact_source(),
            "tags",
            &Value::Array(vec![
                Value::String("true".into()),
                Value::String("123".into()),
                Value::String("a: b".into()),
                Value::String("#c".into()),
                Value::String(String::new()),
            ]),
        )
        .expect("tricky scalars are the emitter's problem, not ours");
        // Round-trips as the strings that went in, not as bool/int/comment.
        let loaded = load(&edited).expect("the result parses");
        assert_eq!(
            crate::document::get_path(&loaded, "tags", crate::document::Addressing::INDEX_ONLY)
                .expect("tags"),
            Value::Array(vec![
                Value::String("true".into()),
                Value::String("123".into()),
                Value::String("a: b".into()),
                Value::String("#c".into()),
                Value::String(String::new()),
            ])
        );
    }

    #[test]
    fn replacing_a_collection_takes_the_comment_that_described_it() {
        // The CST's span stops before a trailing comment, so leaving it in
        // place would re-point `# work` at whatever the new last element is.
        let edited = set_preserving(
            "emails:\n  - alice@example.com  # work\nrole: ''\n",
            "emails",
            &Value::Array(vec![Value::String("bob@example.com".into())]),
        )
        .expect("replace");
        assert_eq!(edited, "emails:\n  - bob@example.com\nrole: ''\n");
        assert!(!edited.contains("# work"), "{edited}");
    }

    #[test]
    fn a_collection_reached_through_an_alias_is_refused() {
        // `span_at` resolves through the alias to the anchor, so an unguarded
        // splice would rewrite `base` when asked for `other`.
        let source = "base: &b\n  - x\nother: *b\n";
        let error = set_preserving(
            source,
            "other",
            &Value::Array(vec![Value::String("y".into())]),
        )
        .expect_err("an alias target must not be edited through");
        assert!(error.to_string().contains("alias"), "{error}");
    }

    #[test]
    fn a_new_key_can_be_created_with_a_collection_value() {
        let edited = set_preserving(
            "kind: contact\n",
            "tags",
            &Value::Array(vec![Value::String("vip".into())]),
        )
        .expect("a missing key is inserted");
        assert_eq!(edited, "kind: contact\ntags:\n  - vip\n");

        let empty = set_preserving("kind: contact\n", "tags", &Value::Array(Vec::new()))
            .expect("a missing key can be inserted empty");
        assert_eq!(empty, "kind: contact\ntags: []\n");
    }

    #[test]
    fn replacing_a_collection_keeps_the_source_newline_style() {
        let edited = set_preserving(
            "kind: contact\r\ntags: []\r\nrole: ''\r\n",
            "tags",
            &Value::Array(vec![Value::String("vip".into())]),
        )
        .expect("CRLF source");
        assert_eq!(edited, "kind: contact\r\ntags:\r\n  - vip\r\nrole: ''\r\n");
        // No lone LF survives once every CRLF is accounted for.
        assert!(
            !edited.replace("\r\n", "").contains('\n'),
            "mixed endings: {edited:?}"
        );
    }

    #[test]
    fn a_new_key_lands_after_a_bare_null_last_entry() {
        // A mapping whose last entry has no value node (`attachments:` reads as
        // null) gives the CST nothing to anchor an insertion to.
        let source = "kind: draft\nattachments:\n";
        let edited = set_preserving(source, "sync_intent", &Value::String("send".into()))
            .expect("a new key can follow a bare-null entry");
        assert_eq!(edited, "kind: draft\nattachments:\nsync_intent: send\n");
    }

    #[test]
    fn save_rejects_non_finite_float() {
        let error = save(&Value::Float(f64::NAN)).expect_err("NaN must fail");
        assert!(error.to_string().contains("non-finite"));
    }
}
