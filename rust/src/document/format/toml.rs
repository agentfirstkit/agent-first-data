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
    keep_bom(content, |body| set_body(body, path, value))
}

// Path segments arrive unescaped, and `toml_edit` quotes any key that needs
// it, so `hosts.example\.com` from `paths` addresses `"example.com"` here
// exactly as it does in every other format.
fn set_body(content: &str, path: &str, value: &Value) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
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
                hand_closing_space_to_new_entry(current);
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
        if last == "-" {
            let parent_key = crate::document::join_path(parents);
            let mut values = match crate::document::get_path(
                &load(content)?,
                &parent_key,
                crate::document::Addressing::INDEX_ONLY,
            )? {
                Value::Array(values) => values,
                _ => return Err(collection_refusal("append target is not an array")),
            };
            let unchanged = values.len();
            values.push(value.clone());
            let existing = current
                .as_array()
                .ok_or_else(|| collection_refusal("append target is not an array"))?;
            *current = toml_edit::Item::Value(array_value_with_prefix(
                &values,
                Some(existing),
                unchanged,
            )?);
            return Ok(preserve_line_endings(content, document.to_string()));
        }
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
    } else if current.is_array_of_tables() {
        // The same refusal the parent walk and `unset` give, rather than a
        // "non-table value" error about a node that is made of tables.
        return Err(collection_refusal(
            "editing an array of tables requires an explicit element identity",
        ));
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
                hand_closing_space_to_new_entry(current);
            }
        }
    }
    Ok(preserve_line_endings(content, document.to_string()))
}

/// After an entry was appended to `item`, move the space before `}` onto it.
///
/// In `{ y = 1 }` the space before `}` is the last value's suffix; left there
/// it ends up before the new comma (`{ y = 1 , z = 2 }`). Hand it to the new
/// last entry — a new leaf and a new table created on the way to one alike.
/// Anything other than an inline table has no such space and is left alone.
fn hand_closing_space_to_new_entry(item: &mut toml_edit::Item) {
    let Some(inline) = item.as_inline_table_mut() else {
        return;
    };
    let count = inline.len();
    if count < 2 {
        return;
    }
    let moved = inline.iter_mut().nth(count - 2).and_then(|(_, previous)| {
        let suffix = previous.decor().suffix()?.as_str()?.to_string();
        (!suffix.is_empty() && suffix.trim().is_empty()).then(|| {
            previous.decor_mut().set_suffix("");
            suffix
        })
    });
    if let Some((_, added)) = inline.iter_mut().last() {
        added.decor_mut().set_suffix(moved.unwrap_or_default());
    }
}

/// Remove an existing TOML item through `toml_edit`, retaining document decor.
pub fn unset_preserving(content: &str, path: &str) -> DocumentResult<String> {
    keep_bom(content, |body| unset_body(body, path))
}

fn unset_body(content: &str, path: &str) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    let (last, parents) = segments.split_last().ok_or(DocumentError::EmptyPath)?;
    let mut document =
        content
            .parse::<toml_edit::DocumentMut>()
            .map_err(|error| DocumentError::ParseError {
                format: "TOML".to_string(),
                detail: error.to_string(),
            })?;
    // The same walk `set` makes: an array is entered by index, so the two
    // verbs reach the same nodes.
    let mut current = document.as_item_mut();
    for parent in parents {
        if current.is_array_of_tables() {
            return Err(DocumentError::UnsupportedOperation {
                format: "TOML".to_string(),
                operation: "unset".to_string(),
                detail: "editing an array of tables requires an explicit element identity"
                    .to_string(),
            });
        }
        let next = if current.as_array().is_some() {
            parent
                .parse::<usize>()
                .ok()
                .and_then(|index| current.get_mut(index))
        } else {
            current.get_mut(parent)
        };
        current =
            next.filter(|item| !item.is_none())
                .ok_or_else(|| DocumentError::PathNotFound {
                    path: path.to_string(),
                })?;
    }
    if current.is_array_of_tables() {
        return Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "unset".to_string(),
            detail: "editing an array of tables requires an explicit element identity".to_string(),
        });
    }
    if let Some(array) = current.as_array_mut() {
        let index = last
            .parse::<usize>()
            .ok()
            .filter(|index| *index < array.len())
            .ok_or_else(|| DocumentError::PathNotFound {
                path: path.to_string(),
            })?;
        let removed = array.remove(index);
        remove_element_comment(array, index, &removed);
        return Ok(preserve_line_endings(content, document.to_string()));
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
    Ok(preserve_line_endings(content, document.to_string()))
}

/// `toml_edit` drops a leading byte-order mark, so a one-key edit would
/// rewrite the file's first bytes. Edit the body and put the mark back.
fn keep_bom(
    content: &str,
    edit: impl FnOnce(&str) -> DocumentResult<String>,
) -> DocumentResult<String> {
    match content.strip_prefix('\u{feff}') {
        Some(body) => edit(body).map(|edited| format!("\u{feff}{edited}")),
        None => edit(content),
    }
}

fn preserve_line_endings(content: &str, rendered: String) -> String {
    if content.contains("\r\n")
        && content.as_bytes().iter().enumerate().all(|(index, byte)| {
            *byte != b'\n' || (index > 0 && content.as_bytes()[index - 1] == b'\r')
        })
    {
        rendered.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        rendered
    }
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
                detail: "float literal is not representable in TOML".to_string(),
            }),
        Value::Number(_) => Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "set".to_string(),
            detail: "integer literal exceeds TOML's 64-bit integer range".to_string(),
        }),
        Value::String(value) => {
            // Reads report a datetime as its text, so text written back
            // over a datetime stays one when it still spells a datetime;
            // otherwise a read-edit-write round trip turns it into a string.
            let existing_datetime = existing.filter(|item| item.is_datetime());
            if let Some(existing) = existing_datetime.filter(|item| {
                item.as_datetime()
                    .is_some_and(|datetime| datetime.to_string() == *value)
            }) {
                Ok(existing.clone())
            } else if let Some(datetime) =
                existing_datetime.and_then(|_| value.parse::<toml_edit::Datetime>().ok())
            {
                Ok(toml_edit::Value::from(datetime))
            } else if value.contains('\n') {
                single_line_string(value)
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

/// A string holding a newline, written as a one-line basic string.
///
/// The default rendering is a `\"\"\"` multiline literal, which puts the value's
/// own newlines into the source as raw line breaks. [`preserve_line_endings`]
/// cannot tell those from the document's line endings, so on a CRLF file it
/// would rewrite the value's `\n` to `\r\n` — a successful `set` that stores
/// something other than what it was given. Escaped, the value carries no raw
/// line break for a line-ending pass to touch.
fn single_line_string(value: &str) -> DocumentResult<toml_edit::Value> {
    use std::fmt::Write as _;
    let mut literal = String::with_capacity(value.len() + 2);
    literal.push('"');
    for character in value.chars() {
        match character {
            '\\' => literal.push_str("\\\\"),
            '"' => literal.push_str("\\\""),
            '\n' => literal.push_str("\\n"),
            '\r' => literal.push_str("\\r"),
            '\t' => literal.push_str("\\t"),
            control if control.is_control() => {
                let _ = write!(literal, "\\u{:04X}", u32::from(control));
            }
            other => literal.push(other),
        }
    }
    literal.push('"');
    let mut parsed = literal.parse::<toml_edit::Value>().map_err(|error| {
        DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "set".to_string(),
            detail: format!("string is not representable in TOML: {error}"),
        }
    })?;
    // A parsed value carries explicit empty decor; clear it so the surrounding
    // table or array applies its own spacing as it does for any new value.
    parsed.decor_mut().clear();
    Ok(parsed)
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

/// Re-attach comments after `removed` left `array` at `index`.
///
/// An element's own comment is written after its comma, so it is stored in the
/// *next* element's prefix, or in the array's trailing decor (the removed
/// element's suffix, without a trailing comma) when it was the last one. Left
/// alone, removal deletes the comment of the element before it and hands the
/// removed element's comment to that neighbour. The first line of each decor
/// is the comment beside the previous value; later lines (own-line comments,
/// indentation) belong to what follows and stay put.
///
/// The first element is the same rule with `[` as the "previous value": the
/// first line of its prefix is the comment beside the bracket (`[ # header`),
/// which stays, while its later lines are the removed element's own-line
/// comments, which go with it.
fn remove_element_comment(array: &mut toml_edit::Array, index: usize, removed: &toml_edit::Value) {
    let decor_text = |raw: Option<&toml_edit::RawString>| {
        raw.and_then(toml_edit::RawString::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let removed_prefix = decor_text(removed.decor().prefix());
    if index == 0
        && let Some(first) = array.get_mut(0)
    {
        let prefix = decor_text(first.decor().prefix());
        let beside_bracket = removed_prefix.split('\n').next().unwrap_or_default();
        let layout = if prefix.contains('\n') {
            // The new first element keeps its own lines — the own-line
            // comments above it — and drops its first line, which was the
            // removed element's comment.
            format!("{beside_bracket}{}", after_first_line(&prefix))
        } else if removed_prefix.contains('\n') {
            // `[\n  1, 2\n]`: the new first element sat on the removed one's
            // line, so it takes the line break and indentation it had.
            let indentation = &removed_prefix[removed_prefix.rfind('\n').unwrap_or_default()..];
            format!("{beside_bracket}{indentation}")
        } else {
            // One line (`[ 1, 2 ]`): the space after `[` is the array's
            // layout, so it stays at the front.
            removed_prefix
        };
        first.decor_mut().set_prefix(layout);
        return;
    }
    // The comment beside the element (or, for the first one, the bracket)
    // before the removed one, which the removed element's prefix held.
    let carried = comment_line(&removed_prefix);
    if let Some(next) = array.get_mut(index) {
        let prefix = decor_text(next.decor().prefix());
        let rest = after_first_line(&prefix);
        match (carried, comment_line(&prefix)) {
            (Some(comment), _) => next.decor_mut().set_prefix(format!("{comment}{rest}")),
            (None, Some(_)) => next.decor_mut().set_prefix(rest.to_string()),
            (None, None) => {}
        }
        return;
    }
    let tail = format!(
        "{}{}",
        decor_text(removed.decor().suffix()),
        array.trailing().as_str().unwrap_or_default()
    );
    let rest = after_first_line(&tail);
    let trailing = match (carried, comment_line(&tail)) {
        // A comment must end its line before the closing bracket.
        (Some(comment), _) if rest.is_empty() => format!("{comment}\n"),
        (Some(comment), _) => format!("{comment}{rest}"),
        (None, Some(_)) => rest.to_string(),
        (None, None) => tail.clone(),
    };
    array.set_trailing(trailing);
}

/// A decor's first line when it carries a comment — the comment written
/// beside the preceding value — including the spaces before it.
fn comment_line(decor: &str) -> Option<&str> {
    let line = decor.split('\n').next().unwrap_or_default();
    line.contains('#').then_some(line)
}

/// Everything from a decor's first line break on.
fn after_first_line(decor: &str) -> &str {
    decor.find('\n').map_or("", |newline| &decor[newline..])
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
    array_value_with_prefix(values, existing, 0)
}

/// `unchanged` leading elements are kept verbatim for an append operation.
fn array_value_with_prefix(
    values: &[Value],
    existing: Option<&toml_edit::Array>,
    unchanged: usize,
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
    // Without a trailing comma, the comment and line break after the last
    // element are that element's suffix rather than the array's trailing
    // decor, so the new comma would land after them (`"y" # c\n,`). Moving
    // them to the trailing decor lets the branch below treat both spellings
    // alike.
    if values.len() > length_before_append
        && let Some(last) = length_before_append
            .checked_sub(1)
            .and_then(|index| array.get_mut(index))
    {
        let suffix = last
            .decor()
            .suffix()
            .and_then(toml_edit::RawString::as_str)
            .unwrap_or_default()
            .to_string();
        if suffix.contains(['#', '\n']) {
            last.decor_mut().set_suffix("");
            let trailing = array.trailing().as_str().unwrap_or_default().to_string();
            array.set_trailing(format!("{suffix}{trailing}"));
        }
    }
    for (index, value) in values.iter().enumerate() {
        if index < unchanged {
            continue;
        }
        let hint = existing_values.get(index);
        let mut converted = toml_value(value, hint)?;
        if index < array.len() {
            array.replace_formatted(index, converted);
        } else {
            let layout = existing_values
                .last()
                .and_then(|item| item.decor().prefix())
                .and_then(toml_edit::RawString::as_str)
                .map(layout_without_comments)
                .unwrap_or_default();
            // The first element of `[1]` has no prefix to copy, but a second
            // one still needs the space after its comma.
            let layout = if layout.is_empty() && index > 0 {
                " ".to_string()
            } else {
                layout
            };
            converted.decor_mut().set_prefix(layout);
            if index == length_before_append {
                // A last element's inline comment is held in the array's
                // trailing decor. Keep it beside that element when extending.
                let trailing = array.trailing().as_str().unwrap_or_default().to_string();
                if first_comment(&trailing).is_some() {
                    let layout = converted
                        .decor()
                        .prefix()
                        .and_then(toml_edit::RawString::as_str)
                        .unwrap_or_default();
                    let indent = layout.rsplit('\n').next().unwrap_or_default().to_string();
                    converted.decor_mut().set_prefix(format!(
                        "{}{indent}",
                        trailing.trim_end_matches([' ', '\t'])
                    ));
                    // Everything up to the last line break moved in front of
                    // the new element; only the bracket's own indentation stays.
                    let bracket_indent = trailing.rsplit('\n').next().unwrap_or_default();
                    array.set_trailing(format!("\n{bracket_indent}"));
                }
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
        return Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "set".to_string(),
            detail: "editing an array of tables requires an explicit element identity".to_string(),
        });
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
                detail: "float literal is not representable in TOML".to_string(),
            }),
        Value::Number(_) => Err(DocumentError::UnsupportedOperation {
            format: "TOML".to_string(),
            operation: "save".to_string(),
            detail: "integer literal exceeds TOML's 64-bit integer range".to_string(),
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
