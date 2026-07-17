//! KeyedList operations for slug-based array access.

use crate::document::{DocumentError, DocumentResult, Value};

/// Declares that an array at `prefix` is keyed by `slug_field`.
///
/// Example: `KeyedList { prefix: "identities", slug_field: "identity" }`
/// enables path `identities.me.email` to find the element where
/// `element["identity"] == "me"`, then read/write `element["email"]`.
#[derive(Debug, Clone, Copy)]
pub struct KeyedList<'a> {
    pub prefix: &'a str,
    pub slug_field: &'a str,
}

/// Add a new element to a keyed list.
///
/// The new element is built in three layers:
/// 1. `seed` fields (if provided) — default template values
/// 2. `{ slug_field: slug }` — always set, overrides any slug value in seed
/// 3. explicit `fields` — override both seed and slug (except the slug field)
pub fn add_keyed(
    root: &mut Value,
    prefix: &str,
    slug: &str,
    keyed_lists: &[KeyedList<'_>],
    seed: Option<&Value>,
    fields: &[(String, Value)],
) -> DocumentResult<()> {
    // Resolve the prefix through the single path grammar so top-level and nested
    // (dotted or escaped) prefixes are all matched by their normalized segments.
    let segments = crate::document::parse_path(prefix)?;
    let registered = keyed_lists.iter().any(|list| {
        crate::document::parse_path(list.prefix).ok().as_deref() == Some(segments.as_slice())
    });
    if !registered {
        return Err(DocumentError::UnregisteredArray {
            path: prefix.to_string(),
        });
    }

    // '.' is the path separator — a slug containing it would be unreachable via get/set_path.
    if slug.contains('.') {
        return Err(DocumentError::ParseError {
            format: "slug".to_string(),
            detail: format!("slug `{slug}` must not contain '.' (path separator)"),
        });
    }

    add_keyed_segments(root, &segments, 0, slug, seed, fields, keyed_lists)
}

/// Remove an element from a keyed list by slug.
pub fn remove_keyed(
    root: &mut Value,
    prefix: &str,
    slug: &str,
    keyed_lists: &[KeyedList<'_>],
) -> DocumentResult<()> {
    let segments = crate::document::parse_path(prefix)?;
    let registered = keyed_lists.iter().any(|list| {
        crate::document::parse_path(list.prefix).ok().as_deref() == Some(segments.as_slice())
    });
    if !registered {
        return Err(DocumentError::UnregisteredArray {
            path: prefix.to_string(),
        });
    }

    remove_keyed_segments(root, &segments, 0, slug, keyed_lists)
}

fn add_keyed_segments(
    current: &mut Value,
    segments: &[String],
    index: usize,
    slug: &str,
    seed: Option<&Value>,
    fields: &[(String, Value)],
    keyed_lists: &[KeyedList<'_>],
) -> DocumentResult<()> {
    if index + 1 < segments.len() {
        let Value::Object(object) = current else {
            return Err(DocumentError::NotTraversable {
                path: segments[..=index].join("."),
                got: "not an object".to_string(),
            });
        };
        let next = object
            .entry(segments[index].clone())
            .or_insert_with(|| Value::Object(Default::default()));
        return add_keyed_segments(next, segments, index + 1, slug, seed, fields, keyed_lists);
    }
    let Value::Object(object) = current else {
        return Err(DocumentError::NotTraversable {
            path: segments.join("."),
            got: "not an object".to_string(),
        });
    };
    let array = object
        .entry(segments[index].clone())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(array) = array else {
        return Err(DocumentError::NotTraversable {
            path: segments.join("."),
            got: "not an array".to_string(),
        });
    };
    let registration = keyed_lists
        .iter()
        .find(|list| crate::document::parse_path(list.prefix).ok().as_deref() == Some(segments))
        .ok_or_else(|| DocumentError::UnregisteredArray {
            path: segments.join("."),
        })?;
    if array.iter().any(|entry| {
        entry
            .as_object()
            .and_then(|object| object.get(registration.slug_field))
            .and_then(Value::as_str)
            == Some(slug)
    }) {
        return Err(DocumentError::SlugAlreadyExists {
            prefix: segments.join("."),
            slug: slug.to_string(),
        });
    }
    let mut element = Value::Object(Default::default());
    let object = element
        .as_object_mut()
        .ok_or_else(|| DocumentError::NotTraversable {
            path: segments.join("."),
            got: "failed to create object".to_string(),
        })?;
    if let Some(seed) = seed.and_then(Value::as_object) {
        for (key, value) in seed {
            if key != registration.slug_field {
                object.insert(key.clone(), value.clone());
            }
        }
    }
    object.insert(
        registration.slug_field.to_string(),
        Value::String(slug.to_string()),
    );
    for (key, value) in fields {
        if key == registration.slug_field {
            return Err(DocumentError::ParseError {
                format: "keyed list".to_string(),
                detail: format!("field `{key}` cannot override slug field"),
            });
        }
        object.insert(key.clone(), value.clone());
    }
    array.push(element);
    Ok(())
}

fn remove_keyed_segments(
    current: &mut Value,
    segments: &[String],
    index: usize,
    slug: &str,
    keyed_lists: &[KeyedList<'_>],
) -> DocumentResult<()> {
    if index + 1 < segments.len() {
        let Value::Object(object) = current else {
            return Err(DocumentError::NotTraversable {
                path: segments[..=index].join("."),
                got: "not an object".to_string(),
            });
        };
        let next = object
            .get_mut(&segments[index])
            .ok_or_else(|| DocumentError::PathNotFound {
                path: segments.join("."),
            })?;
        return remove_keyed_segments(next, segments, index + 1, slug, keyed_lists);
    }
    let Value::Object(object) = current else {
        return Err(DocumentError::NotTraversable {
            path: segments.join("."),
            got: "not an object".to_string(),
        });
    };
    let array = object
        .get_mut(&segments[index])
        .and_then(Value::as_array_mut)
        .ok_or_else(|| DocumentError::PathNotFound {
            path: segments.join("."),
        })?;
    let registration = keyed_lists
        .iter()
        .find(|list| crate::document::parse_path(list.prefix).ok().as_deref() == Some(segments))
        .ok_or_else(|| DocumentError::UnregisteredArray {
            path: segments.join("."),
        })?;
    let before = array.len();
    array.retain(|entry| {
        entry
            .as_object()
            .and_then(|object| object.get(registration.slug_field))
            .and_then(Value::as_str)
            != Some(slug)
    });
    if before == array.len() {
        return Err(DocumentError::SlugNotFound {
            prefix: segments.join("."),
            slug: slug.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    #[test]
    fn test_add_keyed() {
        let mut root = Value::Object(Default::default());
        let keyed = [KeyedList {
            prefix: "identities",
            slug_field: "identity",
        }];

        root.as_object_mut()
            .unwrap()
            .insert("identities".to_string(), Value::Array(vec![]));

        add_keyed(
            &mut root,
            "identities",
            "me",
            &keyed,
            None,
            &[
                (
                    "email".to_string(),
                    Value::String("me@example.com".to_string()),
                ),
                ("name".to_string(), Value::String("Me".to_string())),
            ],
        )
        .unwrap();

        let arr = root.get("identities").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 1);

        let elem = &arr[0];
        assert_eq!(elem.get("identity").unwrap().as_str().unwrap(), "me");
        assert_eq!(
            elem.get("email").unwrap().as_str().unwrap(),
            "me@example.com"
        );
    }

    #[test]
    fn test_add_keyed_with_seed() {
        let mut root = Value::Object(Default::default());
        let keyed = [KeyedList {
            prefix: "identities",
            slug_field: "identity",
        }];
        root.as_object_mut()
            .unwrap()
            .insert("identities".to_string(), Value::Array(vec![]));

        let mut seed_obj = std::collections::BTreeMap::new();
        seed_obj.insert("enabled".to_string(), Value::Bool(true));
        seed_obj.insert("role".to_string(), Value::String("user".to_string()));
        seed_obj.insert(
            "email".to_string(),
            Value::String("default@example.com".to_string()),
        );
        let seed = Value::Object(seed_obj);

        add_keyed(
            &mut root,
            "identities",
            "alice",
            &keyed,
            Some(&seed),
            &[(
                "email".to_string(),
                Value::String("alice@example.com".to_string()),
            )], // overrides seed
        )
        .unwrap();

        let elem = &root.get("identities").unwrap().as_array().unwrap()[0];
        assert_eq!(elem.get("identity").unwrap().as_str().unwrap(), "alice");
        assert_eq!(elem.get("role").unwrap().as_str().unwrap(), "user"); // from seed
        assert!(elem.get("enabled").unwrap().as_bool().unwrap()); // from seed
        assert_eq!(
            elem.get("email").unwrap().as_str().unwrap(),
            "alice@example.com"
        ); // fields override seed
    }

    #[test]
    fn test_remove_keyed() {
        let mut root = Value::Object(Default::default());
        let keyed = [KeyedList {
            prefix: "identities",
            slug_field: "identity",
        }];

        let mut elem1 = Value::Object(Default::default());
        elem1
            .as_object_mut()
            .unwrap()
            .insert("identity".to_string(), Value::String("me".to_string()));

        let mut elem2 = Value::Object(Default::default());
        elem2
            .as_object_mut()
            .unwrap()
            .insert("identity".to_string(), Value::String("other".to_string()));

        root.as_object_mut()
            .unwrap()
            .insert("identities".to_string(), Value::Array(vec![elem1, elem2]));

        remove_keyed(&mut root, "identities", "me", &keyed).unwrap();

        let arr = root.get("identities").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("identity").unwrap().as_str().unwrap(), "other");
    }

    #[test]
    fn test_add_and_remove_keyed_nested_dotted_prefix() {
        // A plain dotted (unescaped) nested prefix must route through the same
        // normalized-segment matcher as top-level and escaped prefixes.
        let mut root = Value::Object(Default::default());
        let keyed = [KeyedList {
            prefix: "cfg.users",
            slug_field: "uid",
        }];

        add_keyed(
            &mut root,
            "cfg.users",
            "bob",
            &keyed,
            None,
            &[("role".to_string(), Value::String("dev".to_string()))],
        )
        .unwrap();

        let arr = root
            .get("cfg")
            .unwrap()
            .get("users")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("uid").unwrap().as_str().unwrap(), "bob");
        assert_eq!(arr[0].get("role").unwrap().as_str().unwrap(), "dev");

        remove_keyed(&mut root, "cfg.users", "bob", &keyed).unwrap();
        let arr = root
            .get("cfg")
            .unwrap()
            .get("users")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(arr.is_empty());
    }
}
