//! The single path grammar used by every traversal operation.
//!
//! A dot separates segments. `\.` embeds a dot in a key and `\\` embeds a
//! backslash. Every other escape is rejected so a path is reversible.
//!
//! A segment may be empty, because `""` is a legal key in JSON and YAML — npm
//! writes one into every `package-lock.json` as the root package. Rejecting it
//! did not make it unreachable, only unspeakable: [`join_path`] still rendered
//! `["packages", ""]` as `packages.`, so `paths` emitted addresses that `value`
//! and `set` then refused, and the reversibility this grammar exists to
//! guarantee did not hold.
//!
//! The one sequence with no spelling is `[""]` on its own: it renders as the
//! empty string, which names no path at all. A `""` key at the document root is
//! therefore unaddressable, and that is the only hole left.

use crate::document::{DocumentError, DocumentResult};

pub fn parse_path(path: &str) -> DocumentResult<Vec<String>> {
    if path.is_empty() {
        return Err(DocumentError::EmptyPath);
    }
    let mut segments = Vec::new();
    let mut segment = String::new();
    let mut escaped = false;
    for character in path.chars() {
        if escaped {
            match character {
                '.' | '\\' => segment.push(character),
                other => {
                    return Err(DocumentError::ParseError {
                        format: "path".to_string(),
                        detail: format!("invalid escape `\\{other}`"),
                    });
                }
            }
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                '.' => segments.push(std::mem::take(&mut segment)),
                other => segment.push(other),
            }
        }
    }
    if escaped {
        return Err(DocumentError::ParseError {
            format: "path".to_string(),
            detail: "trailing path escape".to_string(),
        });
    }
    segments.push(segment);
    Ok(segments)
}

pub fn join_path(segments: &[String]) -> String {
    segments
        .iter()
        .map(|segment| segment.replace('\\', "\\\\").replace('.', "\\."))
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property this grammar exists to have: every key sequence has one
    /// spelling, and that spelling reads back as the same sequence.
    #[test]
    fn every_key_sequence_round_trips_through_its_spelling() {
        let cases: &[&[&str]] = &[
            &["packages", "", "version"],
            &["packages", ""],
            &["", "a"],
            &["a", "", "", "b"],
            &["plain"],
            &["a.dotted.key", "b"],
            &["back\\slash"],
            &["node_modules/@esbuild/darwin-arm64"],
        ];
        for segments in cases {
            let owned: Vec<String> = segments.iter().map(|s| (*s).to_string()).collect();
            let spelling = join_path(&owned);
            let parsed = parse_path(&spelling)
                .unwrap_or_else(|error| panic!("{owned:?} spelled `{spelling}`: {error}"));
            assert_eq!(parsed, owned, "spelled `{spelling}`");
        }
    }

    #[test]
    fn an_empty_segment_is_a_key_not_an_error() {
        // npm writes `""` as the root package of every package-lock.json.
        // Rejecting it did not make it unreachable, only unspeakable.
        assert_eq!(
            parse_path("packages..version").unwrap(),
            vec!["packages".to_string(), String::new(), "version".to_string()]
        );
        assert_eq!(
            parse_path("packages.").unwrap(),
            vec!["packages".to_string(), String::new()]
        );
        assert_eq!(
            parse_path(".leading").unwrap(),
            vec![String::new(), "leading".to_string()]
        );
    }

    #[test]
    fn the_empty_string_still_names_no_path() {
        // The one sequence with no spelling. `[""]` renders as "", which is
        // how a caller says "I gave you no path" — so a `""` key at the
        // document root stays unaddressable, and that is the only hole.
        assert!(matches!(parse_path(""), Err(DocumentError::EmptyPath)));
        assert_eq!(join_path(&[String::new()]), "");
    }

    #[test]
    fn malformed_escapes_are_still_refused() {
        assert!(parse_path(r"trailing\").is_err());
        assert!(parse_path(r"bad\qescape").is_err());
    }
}
