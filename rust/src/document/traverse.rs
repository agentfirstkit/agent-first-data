//! Core dot-path traversal for get/set operations.

use crate::document::{
    DocumentError, DocumentResult, Value,
    keyed::{Addressing, KeyedList},
    path::{join_path, parse_path},
};

/// Resolve one non-numeric segment against `array`, returning the index of the
/// single element it names.
///
/// Shared by the read and write walks so both answer a given address the same
/// way — and so the refusals stay in one place: an array nothing claims is
/// [`DocumentError::UnregisteredArray`], no match is
/// [`DocumentError::SlugNotFound`], and several matches is
/// [`DocumentError::AmbiguousMatch`] rather than the first one.
fn resolve_named_element(
    array: &[Value],
    segment: &str,
    prefix: &[String],
    addressing: Addressing<'_>,
) -> DocumentResult<usize> {
    // A caller's explicit declaration outranks a format-wide rule: it names one
    // array on purpose, so it is the more specific statement about this one.
    if let Some(registration) = addressing
        .keyed_lists
        .iter()
        .find(|list| keyed_prefix_matches(list, prefix))
    {
        let hits = array
            .iter()
            .enumerate()
            .filter(|(_, element)| {
                element.get(registration.slug_field).and_then(Value::as_str) == Some(segment)
            })
            .map(|(index, _)| index)
            .collect();
        return resolve_unique_hit(hits, segment, &join_path(prefix));
    }

    let Some(rule) = addressing.array_rule else {
        return Err(DocumentError::UnregisteredArray {
            path: join_path(prefix),
        });
    };

    let hits: Vec<usize> = array
        .iter()
        .enumerate()
        .filter(|(_, element)| rule.matches(element, segment))
        .map(|(index, _)| index)
        .collect();
    resolve_unique_hit(hits, segment, &join_path(prefix))
}

fn resolve_unique_hit(hits: Vec<usize>, segment: &str, prefix: &str) -> DocumentResult<usize> {
    match hits.as_slice() {
        [] => Err(DocumentError::SlugNotFound {
            prefix: prefix.to_string(),
            slug: segment.to_string(),
        }),
        [only] => Ok(*only),
        several => Err(DocumentError::AmbiguousMatch {
            prefix: prefix.to_string(),
            segment: segment.to_string(),
            indices: several.to_vec(),
        }),
    }
}

/// Parse an array index without letting decimal overflow fall through to named
/// lookup.
///
/// `str::parse::<usize>()` alone cannot distinguish "not a number" from "a
/// number too large for this platform". The former may be a slug; the latter
/// is still lexically an index and must fail as one rather than unexpectedly
/// matching document content.
fn array_index(segment: &str) -> DocumentResult<Option<usize>> {
    if segment.is_empty() || !segment.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(None);
    }
    segment
        .parse::<usize>()
        .map(Some)
        .map_err(|_| DocumentError::PathSyntax {
            detail: "array index exceeds the platform index range".to_string(),
        })
}

/// Get a value at the given dot-path.
///
/// Handles:
/// - Object field access (any level of nesting)
/// - Named array access (keyed-list slug, or a format's own array rule)
/// - Greedy key matching for keys containing '.'
pub fn get_path_ref<'a>(
    root: &'a Value,
    path: &str,
    addressing: Addressing<'_>,
) -> DocumentResult<&'a Value> {
    if path.is_empty() {
        return Err(DocumentError::EmptyPath);
    }

    let segments = parse_path(path)?;
    let mut current = root;
    let mut accumulated_prefix: Vec<String> = Vec::new();
    let mut seg_idx = 0;

    while seg_idx < segments.len() {
        let current_seg = segments[seg_idx].as_str();

        match current {
            Value::Object(obj) => {
                // Try exact match first
                if let Some(next) = obj.get(current_seg) {
                    accumulated_prefix.push(current_seg.to_string());
                    current = next;
                    seg_idx += 1;
                } else {
                    return Err(DocumentError::UnknownSegment {
                        path: path.to_string(),
                        segment: current_seg.to_string(),
                    });
                }
            }
            Value::Array(arr) => {
                // Numeric index takes priority over keyed-list slug.
                if let Some(arr_idx) = array_index(current_seg)? {
                    let elem = arr
                        .get(arr_idx)
                        .ok_or_else(|| DocumentError::IndexOutOfBounds {
                            path: join_path(&accumulated_prefix),
                            index: arr_idx,
                            len: arr.len(),
                        })?;
                    accumulated_prefix.push(current_seg.to_string());
                    current = elem;
                    seg_idx += 1;
                } else {
                    let index =
                        resolve_named_element(arr, current_seg, &accumulated_prefix, addressing)?;
                    current = &arr[index];
                    accumulated_prefix.push(current_seg.to_string());
                    seg_idx += 1;
                }
            }
            _ => {
                return Err(DocumentError::NotTraversable {
                    path: path.to_string(),
                    got: current.kind_name().to_string(),
                });
            }
        }
    }

    Ok(current)
}

/// Get a cloned value at the given dot-path.
pub fn get_path(root: &Value, path: &str, addressing: Addressing<'_>) -> DocumentResult<Value> {
    Ok(get_path_ref(root, path, addressing)?.clone())
}

/// Rewrite `path` into the equivalent index-only address for `root`.
///
/// Content addressing is something only the parsed value can answer, and the
/// source-preserving writers never see it: each backend re-walks the original
/// text by path, and nothing in `identities.me` tells a TOML editor which
/// element `me` is. So a write resolves the address once, here, where the whole
/// document is in hand, and hands the backends the canonical `identities.0`.
/// The alternative — teaching six backends to match on content — would give
/// each its own chance to disagree with the read walk about what an address
/// means.
///
/// Resolution stops at the first segment the document does not have and copies
/// the rest verbatim, because `set` creates missing object parents and a node
/// that does not exist yet cannot be addressed by its content anyway. Whatever
/// is wrong with the tail is then the writer's error to report, unchanged.
///
/// Two prefixes are tracked because they answer different questions: the
/// *semantic* one (the caller's own segments) is what a [`KeyedList`]
/// registration is matched against, so a registration keeps meaning what it
/// meant on the read path, while the *canonical* one (resolved indices) is what
/// gets returned.
pub fn resolve_path(
    root: &Value,
    path: &str,
    addressing: Addressing<'_>,
) -> DocumentResult<String> {
    if path.is_empty() {
        return Ok(String::new());
    }

    let segments = parse_path(path)?;
    let mut current = root;
    let mut semantic: Vec<String> = Vec::new();
    let mut canonical: Vec<String> = Vec::with_capacity(segments.len());

    for (idx, segment) in segments.iter().enumerate() {
        let copy_rest = |canonical: &mut Vec<String>| {
            canonical.extend(segments[idx..].iter().cloned());
        };
        match current {
            Value::Object(object) => match object.get(segment.as_str()) {
                Some(next) => {
                    canonical.push(segment.clone());
                    semantic.push(segment.clone());
                    current = next;
                }
                None => {
                    copy_rest(&mut canonical);
                    break;
                }
            },
            Value::Array(array) => {
                let index = match array_index(segment)? {
                    Some(index) => index,
                    None => resolve_named_element(array, segment, &semantic, addressing)?,
                };
                let Some(next) = array.get(index) else {
                    copy_rest(&mut canonical);
                    break;
                };
                canonical.push(index.to_string());
                semantic.push(segment.clone());
                current = next;
            }
            _ => {
                copy_rest(&mut canonical);
                break;
            }
        }
    }

    Ok(join_path(&canonical))
}

/// Set a value at the given dot-path. `value` is inserted as-is at the leaf —
/// no coercion happens here; callers that accept CLI strings (e.g. the
/// `afdata` binary) construct the typed `Value` first, via
/// [`crate::document::coerce::value_from_type`] (an explicit `--value-type`)
/// or a bare `Value::String` (zero coercion), before calling this.
pub fn set_path(
    root: &mut Value,
    path: &str,
    value: &Value,
    addressing: Addressing<'_>,
) -> DocumentResult<()> {
    if path.is_empty() {
        return Err(DocumentError::EmptyPath);
    }

    let segments = parse_path(path)?;
    set_path_recursive(root, &segments, 0, &mut Vec::new(), addressing, value)
}

fn set_path_recursive(
    current: &mut Value,
    segments: &[String],
    idx: usize,
    accumulated_prefix: &mut Vec<String>,
    addressing: Addressing<'_>,
    value: &Value,
) -> DocumentResult<()> {
    if idx >= segments.len() {
        return Err(DocumentError::EmptyPath);
    }

    let current_seg = segments[idx].as_str();
    let is_last = idx == segments.len() - 1;

    match current {
        Value::Object(obj) => {
            // Path parsing makes dotted keys explicit via `\\.`.
            let key_to_use = current_seg.to_string();

            let segments_to_consume = 1;

            if is_last {
                // At leaf: insert the typed value directly.
                obj.insert(key_to_use, value.clone());
                Ok(())
            } else {
                // Not at leaf: ensure key exists and recurse
                let next_idx = idx + segments_to_consume;
                if next_idx >= segments.len() {
                    return Err(DocumentError::EmptyPath);
                }

                accumulated_prefix.push(key_to_use.clone());

                // Use entry API to avoid double borrow
                use std::collections::btree_map::Entry;
                match obj.entry(key_to_use) {
                    Entry::Occupied(mut ent) => set_path_recursive(
                        ent.get_mut(),
                        segments,
                        next_idx,
                        accumulated_prefix,
                        addressing,
                        value,
                    ),
                    Entry::Vacant(ent) => {
                        let mut new_obj = Value::Object(Default::default());
                        let result = set_path_recursive(
                            &mut new_obj,
                            segments,
                            next_idx,
                            accumulated_prefix,
                            addressing,
                            value,
                        );
                        if result.is_ok() {
                            ent.insert(new_obj);
                        }
                        result
                    }
                }
            }
        }
        Value::Array(arr) => {
            // Numeric index takes priority over keyed-list slug.
            if let Some(arr_idx) = array_index(current_seg)? {
                if arr_idx >= arr.len() {
                    return Err(DocumentError::IndexOutOfBounds {
                        path: join_path(accumulated_prefix),
                        index: arr_idx,
                        len: arr.len(),
                    });
                }
                if is_last {
                    arr[arr_idx] = value.clone();
                    Ok(())
                } else {
                    accumulated_prefix.push(current_seg.to_string());
                    set_path_recursive(
                        &mut arr[arr_idx],
                        segments,
                        idx + 1,
                        accumulated_prefix,
                        addressing,
                        value,
                    )
                }
            } else {
                let elem_idx =
                    resolve_named_element(arr, current_seg, accumulated_prefix, addressing)?;
                if is_last {
                    Err(DocumentError::UnsupportedOperation {
                        format: "keyed list".to_string(),
                        operation: "set".to_string(),
                        detail:
                            "a keyed-list slug resolves to an element; set a child field instead"
                                .to_string(),
                    })
                } else {
                    accumulated_prefix.push(current_seg.to_string());
                    set_path_recursive(
                        &mut arr[elem_idx],
                        segments,
                        idx + 1,
                        accumulated_prefix,
                        addressing,
                        value,
                    )
                }
            }
        }
        _ => Err(DocumentError::NotTraversable {
            path: join_path(accumulated_prefix),
            got: current.kind_name().to_string(),
        }),
    }
}

pub(crate) fn keyed_prefix_matches(
    registration: &KeyedList<'_>,
    semantic_prefix: &[String],
) -> bool {
    if registration.prefix.is_empty() {
        return semantic_prefix.is_empty();
    }
    crate::document::parse_path(registration.prefix)
        .ok()
        .is_some_and(|segments| segments == semantic_prefix)
}

/// Remove the key at the given dot-path from its parent object.
///
/// This is the free-fn "remove a key" verb (paired with keyed-element removal
/// via [`crate::document::remove_keyed`]).
pub fn unset_path(root: &mut Value, path: &str) -> DocumentResult<()> {
    if path.is_empty() {
        return Err(DocumentError::EmptyPath);
    }
    let segments = parse_path(path)?;
    unset_path_recursive(root, &segments, 0, &mut Vec::new())
}

fn unset_path_recursive(
    current: &mut Value,
    segments: &[String],
    idx: usize,
    accumulated_prefix: &mut Vec<String>,
) -> DocumentResult<()> {
    if idx >= segments.len() {
        return Err(DocumentError::EmptyPath);
    }
    let current_seg = segments[idx].as_str();
    let is_last = idx == segments.len() - 1;

    match current {
        Value::Object(obj) => {
            let key_to_use = current_seg.to_string();
            let segments_to_consume = 1;

            if is_last {
                if obj.remove(&key_to_use).is_none() {
                    return Err(DocumentError::PathNotFound { path: key_to_use });
                }
                Ok(())
            } else {
                let next_idx = idx + segments_to_consume;
                accumulated_prefix.push(key_to_use.clone());
                if let Some(next) = obj.get_mut(&key_to_use) {
                    unset_path_recursive(next, segments, next_idx, accumulated_prefix)
                } else {
                    Err(DocumentError::PathNotFound {
                        path: join_path(accumulated_prefix),
                    })
                }
            }
        }
        Value::Array(arr) => {
            if let Some(arr_idx) = array_index(current_seg)? {
                if arr_idx >= arr.len() {
                    return Err(DocumentError::IndexOutOfBounds {
                        path: join_path(accumulated_prefix),
                        index: arr_idx,
                        len: arr.len(),
                    });
                }
                if is_last {
                    arr.remove(arr_idx);
                    Ok(())
                } else {
                    accumulated_prefix.push(current_seg.to_string());
                    unset_path_recursive(&mut arr[arr_idx], segments, idx + 1, accumulated_prefix)
                }
            } else {
                Err(DocumentError::UnregisteredArray {
                    path: join_path(accumulated_prefix),
                })
            }
        }
        _ => Err(DocumentError::NotTraversable {
            path: join_path(accumulated_prefix),
            got: current.kind_name().to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::panic,
        clippy::expect_used,
        clippy::bool_assert_comparison
    )]
    use super::*;

    fn make_test_object() -> Value {
        let mut root = Value::Object(Default::default());
        let mut imap = Value::Object(Default::default());
        imap.as_object_mut().unwrap().insert(
            "host".to_string(),
            Value::String("mail.example.com".to_string()),
        );
        imap.as_object_mut()
            .unwrap()
            .insert("port".to_string(), Value::Integer(993));

        root.as_object_mut()
            .unwrap()
            .insert("imap".to_string(), imap);

        root
    }

    #[test]
    fn test_get_path_simple() {
        let root = make_test_object();
        let result = get_path(&root, "imap.host", Addressing::INDEX_ONLY).unwrap();
        assert_eq!(result.as_str().unwrap(), "mail.example.com");
    }

    #[test]
    fn test_get_path_integer() {
        let root = make_test_object();
        let result = get_path(&root, "imap.port", Addressing::INDEX_ONLY).unwrap();
        assert_eq!(result.as_integer().unwrap(), 993);
    }

    #[test]
    fn test_set_path_new_key() {
        let mut root = make_test_object();
        set_path(
            &mut root,
            "imap.tls",
            &Value::Bool(true),
            Addressing::INDEX_ONLY,
        )
        .unwrap();

        let result = get_path(&root, "imap.tls", Addressing::INDEX_ONLY).unwrap();
        assert_eq!(result.as_bool().unwrap(), true);
    }

    #[test]
    fn test_set_path_overwrite() {
        let mut root = make_test_object();
        set_path(
            &mut root,
            "imap.port",
            &Value::Integer(587),
            Addressing::INDEX_ONLY,
        )
        .unwrap();

        let result = get_path(&root, "imap.port", Addressing::INDEX_ONLY).unwrap();
        assert_eq!(result.as_integer().unwrap(), 587);
    }

    #[test]
    fn test_set_path_array_value() {
        let mut root = Value::Object(Default::default());
        let value = Value::Array(vec![
            Value::String("dev".to_string()),
            Value::String("staging".to_string()),
        ]);
        set_path(&mut root, "tags", &value, Addressing::INDEX_ONLY).unwrap();

        let result = get_path(&root, "tags", Addressing::INDEX_ONLY).unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    fn make_steps_object() -> Value {
        // { "steps": [{"name": "a", "port": 1}, {"name": "b", "port": 2}] }
        let mut root = Value::Object(Default::default());
        let mut s0 = Value::Object(Default::default());
        s0.as_object_mut()
            .unwrap()
            .insert("name".to_string(), Value::String("a".to_string()));
        s0.as_object_mut()
            .unwrap()
            .insert("port".to_string(), Value::Integer(1));
        let mut s1 = Value::Object(Default::default());
        s1.as_object_mut()
            .unwrap()
            .insert("name".to_string(), Value::String("b".to_string()));
        s1.as_object_mut()
            .unwrap()
            .insert("port".to_string(), Value::Integer(2));
        root.as_object_mut()
            .unwrap()
            .insert("steps".to_string(), Value::Array(vec![s0, s1]));
        root
    }

    #[test]
    fn test_get_path_numeric_index() {
        let root = make_steps_object();
        let name = get_path(&root, "steps.0.name", Addressing::INDEX_ONLY).unwrap();
        assert_eq!(name.as_str().unwrap(), "a");
        let port = get_path(&root, "steps.1.port", Addressing::INDEX_ONLY).unwrap();
        assert_eq!(port.as_integer().unwrap(), 2);
    }

    #[test]
    fn test_set_path_numeric_index() {
        let mut root = make_steps_object();
        set_path(
            &mut root,
            "steps.0.port",
            &Value::Integer(99),
            Addressing::INDEX_ONLY,
        )
        .unwrap();
        let result = get_path(&root, "steps.0.port", Addressing::INDEX_ONLY).unwrap();
        assert_eq!(result.as_integer().unwrap(), 99);
        // other element unchanged
        let other = get_path(&root, "steps.1.port", Addressing::INDEX_ONLY).unwrap();
        assert_eq!(other.as_integer().unwrap(), 2);
    }

    #[test]
    fn test_get_path_index_out_of_bounds() {
        let root = make_steps_object();
        let err = get_path(&root, "steps.5.name", Addressing::INDEX_ONLY).unwrap_err();
        assert!(matches!(
            err,
            DocumentError::IndexOutOfBounds {
                index: 5,
                len: 2,
                ..
            }
        ));
    }

    #[test]
    fn test_remove_path_numeric_index() {
        let mut root = make_steps_object();
        unset_path(&mut root, "steps.0").unwrap();
        let arr = get_path(&root, "steps", Addressing::INDEX_ONLY).unwrap();
        let arr = arr.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("name").unwrap().as_str().unwrap(), "b");
    }

    #[test]
    fn named_root_array_keeps_a_semantic_prefix_for_nested_keyed_lists() {
        let mut child = Value::Object(Default::default());
        child
            .as_object_mut()
            .unwrap()
            .insert("id".to_string(), Value::String("beta".to_string()));
        child
            .as_object_mut()
            .unwrap()
            .insert("value".to_string(), Value::Integer(1));

        let mut parent = Value::Object(Default::default());
        parent
            .as_object_mut()
            .unwrap()
            .insert("id".to_string(), Value::String("alpha".to_string()));
        parent
            .as_object_mut()
            .unwrap()
            .insert("children".to_string(), Value::Array(vec![child]));

        let mut root = Value::Array(vec![parent]);
        let keyed = [
            KeyedList {
                prefix: "",
                slug_field: "id",
            },
            KeyedList {
                prefix: "alpha.children",
                slug_field: "id",
            },
        ];
        let addressing = Addressing::keyed(&keyed);

        assert_eq!(
            get_path(&root, "alpha.children.beta.value", addressing).unwrap(),
            Value::Integer(1)
        );
        set_path(
            &mut root,
            "alpha.children.beta.value",
            &Value::Integer(2),
            addressing,
        )
        .unwrap();
        assert_eq!(
            get_path(&root, "alpha.children.beta.value", addressing).unwrap(),
            Value::Integer(2)
        );
    }

    #[test]
    fn oversized_decimal_array_segment_never_falls_through_to_a_slug() {
        let oversized = format!("{}0", usize::MAX);
        let mut item = Value::Object(Default::default());
        item.as_object_mut()
            .unwrap()
            .insert("id".to_string(), Value::String(oversized.clone()));
        let root = Value::Array(vec![item]);
        let keyed = [KeyedList {
            prefix: "",
            slug_field: "id",
        }];

        let error = get_path(&root, &oversized, Addressing::keyed(&keyed)).unwrap_err();
        assert!(matches!(error, DocumentError::PathSyntax { .. }));
    }

    #[test]
    fn dotted_key_and_nested_path_keep_distinct_keyed_registrations() {
        let item = |field: &str, slug: &str| {
            let mut value = Value::Object(Default::default());
            value
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), Value::String(slug.to_string()));
            value
        };

        let mut nested = Value::Object(Default::default());
        nested.as_object_mut().unwrap().insert(
            "b".to_string(),
            Value::Array(vec![item("nested_id", "nested")]),
        );

        let mut root = Value::Object(Default::default());
        root.as_object_mut().unwrap().insert(
            "a.b".to_string(),
            Value::Array(vec![item("dotted_id", "dotted")]),
        );
        root.as_object_mut()
            .unwrap()
            .insert("a".to_string(), nested);

        let keyed = [
            KeyedList {
                prefix: r"a\.b",
                slug_field: "dotted_id",
            },
            KeyedList {
                prefix: "a.b",
                slug_field: "nested_id",
            },
        ];
        let addressing = Addressing::keyed(&keyed);

        assert_eq!(
            get_path(&root, r"a\.b.dotted.dotted_id", addressing).unwrap(),
            Value::String("dotted".to_string())
        );
        assert_eq!(
            get_path(&root, "a.b.nested.nested_id", addressing).unwrap(),
            Value::String("nested".to_string())
        );
    }
}
