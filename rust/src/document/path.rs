//! The single path grammar used by every traversal operation.
//!
//! A dot separates segments. `\.` embeds a dot in a key, `\\` embeds a
//! backslash, and `\*` embeds a star. Every other escape is rejected so a path
//! is reversible.
//!
//! A bare `*` segment is reserved: it means "each child here" to the commands
//! that accept a pattern, and [`parse_path`] refuses it rather than reading it
//! as a key. `*` is a legal key in JSON, so [`join_path`] escapes it — a
//! document carrying one is still addressable, as `\*`.
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
    // `\*` and `*` both parse to the same string, so the reservation below has
    // to know which one was written.
    let mut wrote_bare_star = false;
    let mut literal = false;
    for character in path.chars() {
        if escaped {
            match character {
                '.' | '\\' | '*' => {
                    segment.push(character);
                    literal = true;
                }
                other => {
                    return Err(DocumentError::PathSyntax {
                        detail: format!("invalid escape `\\{other}`"),
                    });
                }
            }
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                '.' => {
                    if segment == "*" && !literal {
                        wrote_bare_star = true;
                    }
                    segments.push(std::mem::take(&mut segment));
                    literal = false;
                }
                other => segment.push(other),
            }
        }
    }
    if escaped {
        return Err(DocumentError::PathSyntax {
            detail: "trailing path escape".to_string(),
        });
    }
    if segment == "*" && !literal {
        wrote_bare_star = true;
    }
    segments.push(segment);
    // A bare `*` means "each child" wherever a path is written, so refuse it
    // here rather than let it read as a key in one command and expand in
    // another. The literal spelling is `\*`, which reaches this point as the
    // same string and must still be accepted.
    if wrote_bare_star {
        return Err(DocumentError::PathSyntax {
            detail: "a bare `*` segment is a pattern; write `\\*` for a literal star key, or use \
                     a command that expands patterns"
                .to_string(),
        });
    }
    Ok(segments)
}

pub fn join_path(segments: &[String]) -> String {
    segments
        .iter()
        .map(|segment| {
            segment
                .replace('\\', "\\\\")
                .replace('.', "\\.")
                .replace('*', "\\*")
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// One segment of a path that may address many nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternSegment {
    /// A literal key or array index.
    Key(String),
    /// A bare `*`: every child of the container at this point.
    Wildcard,
}

/// Parse a path in which a bare `*` segment means "each child here".
///
/// Reading one field across a collection was three steps — enumerate the
/// children, append the field to each address, then read them — and the middle
/// step was string surgery on this grammar's own output. A pattern says it
/// directly: `package.*.name`.
///
/// A literal star is still addressable as `\*`, so no document becomes
/// unreachable by reserving the bare form.
pub fn parse_path_pattern(path: &str) -> DocumentResult<Vec<PatternSegment>> {
    if path.is_empty() {
        return Err(DocumentError::EmptyPath);
    }
    let mut segments = Vec::new();
    let mut segment = String::new();
    let mut escaped = false;
    let mut literal = false;
    for character in path.chars() {
        if escaped {
            match character {
                '.' | '\\' | '*' => {
                    segment.push(character);
                    literal = true;
                }
                other => {
                    return Err(DocumentError::PathSyntax {
                        detail: format!("invalid escape `\\{other}`"),
                    });
                }
            }
            escaped = false;
        } else {
            match character {
                '\\' => escaped = true,
                '.' => {
                    segments.push(finish_pattern_segment(
                        std::mem::take(&mut segment),
                        literal,
                    ));
                    literal = false;
                }
                other => segment.push(other),
            }
        }
    }
    if escaped {
        return Err(DocumentError::PathSyntax {
            detail: "trailing path escape".to_string(),
        });
    }
    segments.push(finish_pattern_segment(segment, literal));
    Ok(segments)
}

fn finish_pattern_segment(segment: String, literal: bool) -> PatternSegment {
    if segment == "*" && !literal {
        PatternSegment::Wildcard
    } else {
        PatternSegment::Key(segment)
    }
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
    fn a_star_key_survives_the_wildcard_reservation() {
        // `*` is a legal JSON key. Reserving the bare form for patterns must
        // not make a document carrying one unaddressable — the same mistake
        // the empty-segment rejection made.
        assert_eq!(join_path(&["*".to_string()]), r"\*");
        assert_eq!(parse_path(r"\*").unwrap(), vec!["*".to_string()]);
        assert_eq!(
            parse_path(r"a.\*.b").unwrap(),
            vec!["a".to_string(), "*".to_string(), "b".to_string()]
        );
        // Round-trip, which is the property that matters.
        let segments = vec!["a".to_string(), "*".to_string()];
        assert_eq!(parse_path(&join_path(&segments)).unwrap(), segments);
    }

    #[test]
    fn a_bare_star_is_reserved_for_patterns() {
        // One spelling, one meaning: it must not read as a key here and expand
        // somewhere else.
        assert!(parse_path("*").is_err());
        assert!(parse_path("package.*.name").is_err());
        // The pattern parser is where it means something.
        assert_eq!(
            parse_path_pattern("package.*.name").unwrap(),
            vec![
                PatternSegment::Key("package".to_string()),
                PatternSegment::Wildcard,
                PatternSegment::Key("name".to_string()),
            ]
        );
        // And escaped, it is a key even there.
        assert_eq!(
            parse_path_pattern(r"a.\*").unwrap(),
            vec![
                PatternSegment::Key("a".to_string()),
                PatternSegment::Key("*".to_string()),
            ]
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
