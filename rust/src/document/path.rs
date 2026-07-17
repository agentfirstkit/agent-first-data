//! The single path grammar used by every traversal operation.
//!
//! A dot separates segments. `\.` embeds a dot in a key and `\\` embeds a
//! backslash. Every other escape is rejected so a path is reversible.

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
                '.' => {
                    if segment.is_empty() {
                        return Err(DocumentError::ParseError {
                            format: "path".to_string(),
                            detail: "empty path segment".to_string(),
                        });
                    }
                    segments.push(std::mem::take(&mut segment));
                }
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
    if segment.is_empty() {
        return Err(DocumentError::ParseError {
            format: "path".to_string(),
            detail: "empty path segment".to_string(),
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
