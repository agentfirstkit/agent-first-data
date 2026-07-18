//! Error types with context and helpful hints.

use std::fmt;
use std::io;

pub type DocumentResult<T> = Result<T, DocumentError>;

#[derive(Debug, Clone)]
pub enum DocumentError {
    EmptyPath,
    EmptyValues,
    UnknownSegment {
        path: String,
        segment: String,
    },
    UnregisteredArray {
        path: String,
    },
    SlugNotFound {
        prefix: String,
        slug: String,
    },
    SlugAlreadyExists {
        prefix: String,
        slug: String,
    },
    NotTraversable {
        path: String,
        got: String,
    },
    TypeMismatch {
        path: String,
        expected: String,
        got: String,
        hint: Option<String>,
    },
    PathNotFound {
        path: String,
    },
    IndexOutOfBounds {
        path: String,
        index: usize,
        len: usize,
    },
    ParseError {
        format: String,
        detail: String,
    },
    IoError {
        detail: String,
    },
    UnsupportedOperation {
        format: String,
        operation: String,
        detail: String,
    },
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentError::EmptyPath => {
                write!(f, "empty path provided")
            }
            DocumentError::EmptyValues => {
                write!(f, "at least one value required")
            }
            DocumentError::UnknownSegment { path, segment } => {
                write!(f, "path `{}` segment `{}` not found", path, segment)
            }
            DocumentError::UnregisteredArray { path } => {
                write!(f, "array at `{}` not registered in KeyedList", path)
            }
            DocumentError::SlugNotFound { prefix, slug } => {
                write!(f, "no element with slug `{}` found in `{}`", slug, prefix)
            }
            DocumentError::SlugAlreadyExists { prefix, slug } => {
                write!(f, "slug `{}` already exists in `{}`", slug, prefix)
            }
            DocumentError::NotTraversable { path, got } => {
                write!(f, "path `{}` is {}, cannot traverse further", path, got)
            }
            DocumentError::TypeMismatch {
                path,
                expected,
                got,
                hint,
            } => {
                write!(f, "field `{}` expects {}, got `{}`", path, expected, got)?;
                if let Some(h) = hint {
                    write!(f, "\n  hint: {}", h)?;
                }
                Ok(())
            }
            DocumentError::PathNotFound { path } => {
                write!(f, "path `{}` not found in document", path)
            }
            DocumentError::IndexOutOfBounds { path, index, len } => {
                write!(
                    f,
                    "index {} out of bounds at `{}` (len {})",
                    index, path, len
                )
            }
            DocumentError::ParseError { format, detail } => {
                write!(f, "failed to parse {}: {}", format, detail)
            }
            DocumentError::IoError { detail } => {
                write!(f, "io error: {}", detail)
            }
            DocumentError::UnsupportedOperation {
                format,
                operation,
                detail,
            } => write!(f, "{} does not support {}: {}", format, operation, detail),
        }
    }
}

impl std::error::Error for DocumentError {}

impl DocumentError {
    /// Stable, program-decidable error code for this failure category.
    ///
    /// Multiple variants can share a code when callers should handle them in
    /// the same way. In particular, all read-address failures report
    /// `document_path_not_found`.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ParseError { .. } => "document_parse_failed",
            Self::PathNotFound { .. }
            | Self::UnknownSegment { .. }
            | Self::IndexOutOfBounds { .. }
            | Self::UnregisteredArray { .. } => "document_path_not_found",
            Self::NotTraversable { .. } | Self::TypeMismatch { .. } => "document_type_mismatch",
            Self::SlugNotFound { .. } => "document_slug_not_found",
            Self::SlugAlreadyExists { .. } => "document_slug_exists",
            Self::IoError { .. } => "document_io_failed",
            Self::UnsupportedOperation { .. } => "document_unsupported_operation",
            Self::EmptyPath | Self::EmptyValues => "document_invalid_argument",
        }
    }

    /// Best-effort, content-free source location for a parse failure.
    ///
    /// Returns e.g. `"line 5 column 12"` (or `"line 5"`) for a
    /// [`DocumentError::ParseError`], and `None` for every other variant or
    /// when the underlying parser reported no position. The returned string is
    /// derived from the parser's position only and never contains document
    /// content, so it is safe to surface even when the parsed file may hold
    /// secrets.
    #[must_use]
    pub fn location(&self) -> Option<String> {
        let Self::ParseError { detail, .. } = self else {
            return None;
        };
        let rest = &detail[detail.find("line ")? + 5..];
        let line: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if line.is_empty() {
            return None;
        }
        let column = rest
            .find("column ")
            .map(|start| &rest[start + 7..])
            .map(|tail| {
                tail.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
            })
            .filter(|value| !value.is_empty());
        Some(match column {
            Some(column) => format!("line {line} column {column}"),
            None => format!("line {line}"),
        })
    }

    /// A display message with any potentially content-bearing detail removed —
    /// safe to surface when the document may hold secrets.
    ///
    /// A [`DocumentError::ParseError`] renders as `failed to parse {format}`
    /// (with the [`location`](Self::location) appended when known) and drops
    /// the raw parser detail, which can echo a snippet of the source. Every
    /// other variant renders the same as its [`Display`], since those carry
    /// only structural context (paths, type and format names), not content.
    #[must_use]
    pub fn redacted_message(&self) -> String {
        match self {
            Self::ParseError { format, .. } => match self.location() {
                Some(location) => format!("failed to parse {format} at {location}"),
                None => format!("failed to parse {format}"),
            },
            other => other.to_string(),
        }
    }

    /// Wrap a serde deserialization failure as a `TypeMismatch` so callers that
    /// do a read-modify-write cycle (set_path → serde round-trip) surface a
    /// consistent error style rather than a raw serde message.
    pub fn from_serde(path: impl Into<String>, err: impl std::fmt::Display) -> Self {
        let msg = err.to_string();
        // serde messages look like "invalid type: string \"x\", expected u16 at …"
        // Strip the trailing " at line N column M" to keep the hint concise.
        let hint = msg
            .split(" at line ")
            .next()
            .unwrap_or(&msg)
            .trim()
            .to_string();
        DocumentError::TypeMismatch {
            path: path.into(),
            expected: String::new(),
            got: hint,
            hint: None,
        }
    }
}

impl From<io::Error> for DocumentError {
    fn from(err: io::Error) -> Self {
        DocumentError::IoError {
            detail: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DocumentError;

    #[test]
    fn document_error_codes_are_stable() {
        let cases = [
            (DocumentError::EmptyPath, "document_invalid_argument"),
            (DocumentError::EmptyValues, "document_invalid_argument"),
            (
                DocumentError::UnknownSegment {
                    path: "root.key".to_string(),
                    segment: "key".to_string(),
                },
                "document_path_not_found",
            ),
            (
                DocumentError::UnregisteredArray {
                    path: "items".to_string(),
                },
                "document_path_not_found",
            ),
            (
                DocumentError::SlugNotFound {
                    prefix: "items".to_string(),
                    slug: "missing".to_string(),
                },
                "document_slug_not_found",
            ),
            (
                DocumentError::SlugAlreadyExists {
                    prefix: "items".to_string(),
                    slug: "existing".to_string(),
                },
                "document_slug_exists",
            ),
            (
                DocumentError::NotTraversable {
                    path: "root".to_string(),
                    got: "string".to_string(),
                },
                "document_type_mismatch",
            ),
            (
                DocumentError::TypeMismatch {
                    path: "root.key".to_string(),
                    expected: "integer".to_string(),
                    got: "string".to_string(),
                    hint: None,
                },
                "document_type_mismatch",
            ),
            (
                DocumentError::PathNotFound {
                    path: "root.key".to_string(),
                },
                "document_path_not_found",
            ),
            (
                DocumentError::IndexOutOfBounds {
                    path: "items".to_string(),
                    index: 2,
                    len: 1,
                },
                "document_path_not_found",
            ),
            (
                DocumentError::ParseError {
                    format: "JSON".to_string(),
                    detail: "invalid input".to_string(),
                },
                "document_parse_failed",
            ),
            (
                DocumentError::IoError {
                    detail: "unreadable".to_string(),
                },
                "document_io_failed",
            ),
            (
                DocumentError::UnsupportedOperation {
                    format: "INI".to_string(),
                    operation: "set".to_string(),
                    detail: "unsupported".to_string(),
                },
                "document_unsupported_operation",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.code(), expected);
        }
    }

    #[test]
    fn location_extracts_position_without_content() {
        let err = DocumentError::ParseError {
            format: "YAML".to_string(),
            detail: "secret: [ TOPSECRET at line 5 column 12".to_string(),
        };
        assert_eq!(err.location().as_deref(), Some("line 5 column 12"));

        let no_column = DocumentError::ParseError {
            format: "JSON".to_string(),
            detail: "boom at line 3".to_string(),
        };
        assert_eq!(no_column.location().as_deref(), Some("line 3"));

        // No position, and non-parse variants, carry no location.
        assert!(
            DocumentError::ParseError {
                format: "INI".to_string(),
                detail: "sensitive value".to_string(),
            }
            .location()
            .is_none()
        );
        assert!(
            DocumentError::PathNotFound {
                path: "a.b".to_string(),
            }
            .location()
            .is_none()
        );
    }

    #[test]
    fn redacted_message_drops_parser_detail() {
        let err = DocumentError::ParseError {
            format: "YAML".to_string(),
            detail: "unexpected TOPSECRET at line 5 column 12".to_string(),
        };
        let redacted = err.redacted_message();
        assert_eq!(redacted, "failed to parse YAML at line 5 column 12");
        assert!(!redacted.contains("TOPSECRET"));

        // Structural variants pass through unchanged.
        let path_err = DocumentError::PathNotFound {
            path: "database.url".to_string(),
        };
        assert_eq!(path_err.redacted_message(), path_err.to_string());
    }
}
