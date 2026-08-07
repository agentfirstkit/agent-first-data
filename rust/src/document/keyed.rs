//! KeyedList operations for slug-based array access.

use crate::document::{DocumentError, DocumentResult, Value, join_path};

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

/// How a **non-numeric** path segment resolves against an array.
///
/// A non-empty ASCII-decimal segment is always an index and never consults
/// this (overflow is an error, not a slug fallback). Everything else has two
/// possible sources, tried in this order:
///
/// - [`keyed_lists`](Self::keyed_lists) — the caller's own declarations, e.g.
///   "the array at `identities` is keyed by each element's `identity` field".
///   A JSON or TOML document does not say this about itself, so only the
///   caller can, and it names one exact array by path.
/// - [`array_rule`](Self::array_rule) — the *format's* rule, for a format whose
///   value is a tree afdata synthesized and therefore knows the shape of. Only
///   Markdown has one today; it applies to every array in the document rather
///   than to one named path.
///
/// With neither, a non-numeric segment against an array is
/// [`DocumentError::UnregisteredArray`]
/// — afdata will not scan an array it was told nothing about.
#[derive(Debug, Clone, Copy, Default)]
pub struct Addressing<'a> {
    pub keyed_lists: &'a [KeyedList<'a>],
    pub array_rule: Option<ArrayRule<'a>>,
}

impl<'a> Addressing<'a> {
    /// Indices only: no keyed lists, no format rule.
    pub const INDEX_ONLY: Addressing<'static> = Addressing {
        keyed_lists: &[],
        array_rule: None,
    };

    /// Caller-declared keyed lists, with no format rule.
    #[must_use]
    pub const fn keyed(keyed_lists: &'a [KeyedList<'a>]) -> Self {
        Addressing {
            keyed_lists,
            array_rule: None,
        }
    }

    /// The same addressing plus a format's built-in array rule.
    #[must_use]
    pub const fn with_array_rule(self, array_rule: Option<ArrayRule<'a>>) -> Self {
        Addressing { array_rule, ..self }
    }
}

/// A format's built-in rule for resolving a non-numeric segment against an
/// array: compare the segment to [`field`](Self::field) on each element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayRule<'a> {
    /// Element field the segment is compared against. An element lacking it,
    /// or holding a non-string there, simply does not match.
    pub field: &'a str,
    pub match_kind: MatchKind,
}

/// How an [`ArrayRule`] compares a segment to an element's field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// The field equals the segment.
    Exact,
    /// The field contains the segment after Unicode lowercase conversion — so
    /// `h2.look`
    /// finds `## A Quick Look`. Prose headings are long and get reworded;
    /// a memorable word out of one is a far steadier address than either its
    /// position or its full text.
    ///
    /// Exactly one element must match. Several is
    /// [`DocumentError::AmbiguousMatch`],
    /// never the first one.
    Contains,
}

impl ArrayRule<'_> {
    /// Whether `element` matches `segment` under this rule.
    #[must_use]
    pub fn matches(&self, element: &Value, segment: &str) -> bool {
        let Some(field) = element.get(self.field).and_then(Value::as_str) else {
            return false;
        };
        match self.match_kind {
            MatchKind::Exact => field == segment,
            // An empty segment is not an address. `contains("")` is true for
            // every element, so without this a path built by interpolation —
            // which is how both consumers build theirs — resolves to a
            // confident wrong answer on any single-element array, and only
            // becomes an error once a second element exists. `Exact` already
            // rejects it by construction (nothing equals ""), so this keeps
            // the two modes agreeing about what is addressable.
            MatchKind::Contains if segment.is_empty() => false,
            MatchKind::Contains => field.to_lowercase().contains(&segment.to_lowercase()),
        }
    }
}

/// Path segments a keyed-list prefix names, where the empty prefix means the
/// document root is itself the keyed array.
///
/// `parse_path` rejects an empty path, which is right for an address but wrong
/// for this prefix: `traverse::keyed_prefix_matches` has always accepted
/// `KeyedList { prefix: "" }`, so a root array could be read by slug while
/// `add`/`remove` answered `EmptyPath` for the very same registration.
fn keyed_prefix_segments(prefix: &str) -> DocumentResult<Vec<String>> {
    if prefix.is_empty() {
        return Ok(Vec::new());
    }
    crate::document::parse_path(prefix)
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
    let segments = keyed_prefix_segments(prefix)?;
    let registered = keyed_lists
        .iter()
        .any(|list| crate::document::keyed_prefix_matches(list, &segments));
    if !registered {
        return Err(DocumentError::UnregisteredArray {
            path: prefix.to_string(),
        });
    }

    add_keyed_segments(root, &segments, 0, slug, seed, fields, keyed_lists)
}

/// Remove the unique element named by `slug` and return its former index.
///
/// The index lets a source-preserving backend remove the exact same element
/// from its syntax tree without independently repeating the identity lookup.
pub fn remove_keyed(
    root: &mut Value,
    prefix: &str,
    slug: &str,
    keyed_lists: &[KeyedList<'_>],
) -> DocumentResult<usize> {
    let segments = keyed_prefix_segments(prefix)?;
    let registered = keyed_lists
        .iter()
        .any(|list| crate::document::keyed_prefix_matches(list, &segments));
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
                path: join_path(&segments[..=index]),
                got: current.kind_name().to_string(),
            });
        };
        let next = object
            .entry(segments[index].clone())
            .or_insert_with(|| Value::Object(Default::default()));
        return add_keyed_segments(next, segments, index + 1, slug, seed, fields, keyed_lists);
    }
    let array = if segments.is_empty() {
        current
    } else {
        let Value::Object(object) = current else {
            return Err(DocumentError::NotTraversable {
                path: join_path(segments),
                got: current.kind_name().to_string(),
            });
        };
        object
            .entry(segments[index].clone())
            .or_insert_with(|| Value::Array(Vec::new()))
    };
    let Value::Array(array) = array else {
        return Err(DocumentError::NotTraversable {
            path: join_path(segments),
            got: array.kind_name().to_string(),
        });
    };
    let registration = keyed_lists
        .iter()
        .find(|list| crate::document::keyed_prefix_matches(list, segments))
        .ok_or_else(|| DocumentError::UnregisteredArray {
            path: join_path(segments),
        })?;
    if array.iter().any(|entry| {
        entry
            .as_object()
            .and_then(|object| object.get(registration.slug_field))
            .and_then(Value::as_str)
            == Some(slug)
    }) {
        return Err(DocumentError::SlugAlreadyExists {
            prefix: join_path(segments),
            slug: slug.to_string(),
        });
    }
    let mut element = Value::Object(Default::default());
    let object = element
        .as_object_mut()
        .ok_or_else(|| DocumentError::NotTraversable {
            path: join_path(segments),
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
            return Err(DocumentError::InvalidArgument {
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
) -> DocumentResult<usize> {
    if index + 1 < segments.len() {
        let Value::Object(object) = current else {
            return Err(DocumentError::NotTraversable {
                path: join_path(&segments[..=index]),
                got: current.kind_name().to_string(),
            });
        };
        let next = object
            .get_mut(&segments[index])
            .ok_or_else(|| DocumentError::PathNotFound {
                path: join_path(segments),
            })?;
        return remove_keyed_segments(next, segments, index + 1, slug, keyed_lists);
    }
    let target = if segments.is_empty() {
        current
    } else {
        let Value::Object(object) = current else {
            return Err(DocumentError::NotTraversable {
                path: join_path(segments),
                got: current.kind_name().to_string(),
            });
        };
        object
            .get_mut(&segments[index])
            .ok_or_else(|| DocumentError::PathNotFound {
                path: join_path(segments),
            })?
    };
    let Value::Array(array) = target else {
        return Err(DocumentError::NotTraversable {
            path: join_path(segments),
            got: target.kind_name().to_string(),
        });
    };
    let registration = keyed_lists
        .iter()
        .find(|list| crate::document::keyed_prefix_matches(list, segments))
        .ok_or_else(|| DocumentError::UnregisteredArray {
            path: join_path(segments),
        })?;
    let matches: Vec<usize> = array
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry
                .as_object()
                .and_then(|object| object.get(registration.slug_field))
                .and_then(Value::as_str)
                == Some(slug)
        })
        .map(|(index, _)| index)
        .collect();
    let index = match matches.as_slice() {
        [] => {
            return Err(DocumentError::SlugNotFound {
                prefix: join_path(segments),
                slug: slug.to_string(),
            });
        }
        [index] => *index,
        _ => {
            return Err(DocumentError::AmbiguousMatch {
                prefix: join_path(segments),
                segment: slug.to_string(),
                indices: matches,
            });
        }
    };
    array.remove(index);
    Ok(index)
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

        let removed_index = remove_keyed(&mut root, "identities", "me", &keyed).unwrap();

        assert_eq!(removed_index, 0);
        let arr = root.get("identities").unwrap().as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].get("identity").unwrap().as_str().unwrap(), "other");
    }

    #[test]
    fn test_remove_keyed_refuses_duplicate_slug_without_mutating() {
        let item = |email: &str| {
            let mut value = Value::Object(Default::default());
            let object = value.as_object_mut().unwrap();
            object.insert("identity".to_string(), Value::String("me".to_string()));
            object.insert("email".to_string(), Value::String(email.to_string()));
            value
        };
        let mut original = Value::Object(Default::default());
        original.as_object_mut().unwrap().insert(
            "identities".to_string(),
            Value::Array(vec![item("first"), item("second")]),
        );
        let mut root = original.clone();
        let keyed = [KeyedList {
            prefix: "identities",
            slug_field: "identity",
        }];

        let error = remove_keyed(&mut root, "identities", "me", &keyed).unwrap_err();

        assert!(matches!(
            error,
            DocumentError::AmbiguousMatch {
                prefix,
                segment,
                indices
            } if prefix == "identities"
                && segment == "me"
                && indices == vec![0, 1]
        ));
        assert_eq!(root, original);
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

#[cfg(test)]
mod root_array_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::document::{Addressing, get_path};
    use std::collections::BTreeMap;

    fn element(id: &str) -> Value {
        Value::Object(BTreeMap::from([(
            "id".to_string(),
            Value::String(id.to_string()),
        )]))
    }

    #[test]
    fn a_root_array_reads_and_edits_through_the_same_registration() {
        // `KeyedList { prefix: "" }` has always resolved on the read walk;
        // `add`/`remove` rejected it with `EmptyPath` because they ran the
        // prefix through `parse_path`, which refuses an empty address. One
        // registration must mean one thing to both.
        let lists = [KeyedList {
            prefix: "",
            slug_field: "id",
        }];
        let mut root = Value::Array(vec![element("a"), element("b")]);

        assert_eq!(
            get_path(&root, "a.id", Addressing::keyed(&lists)).unwrap(),
            Value::String("a".to_string())
        );
        assert_eq!(remove_keyed(&mut root, "", "a", &lists).unwrap(), 0);
        assert_eq!(root.as_array().map(Vec::len), Some(1));

        add_keyed(&mut root, "", "c", &lists, None, &[]).unwrap();
        assert_eq!(
            get_path(&root, "c.id", Addressing::keyed(&lists)).unwrap(),
            Value::String("c".to_string())
        );

        // An unregistered root array is still refused, not scanned.
        let mut bare = Value::Array(vec![element("a")]);
        assert_eq!(
            remove_keyed(&mut bare, "", "a", &[]).unwrap_err().code(),
            "document_path_not_found"
        );
    }
}
