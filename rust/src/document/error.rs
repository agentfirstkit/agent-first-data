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
    /// A non-numeric segment addressed an array that nothing claims: no
    /// [`KeyedList`](crate::document::KeyedList) registration covers it and its
    /// format states no rule of its own.
    ///
    /// The message names the two ways out rather than the internal type that
    /// happens to be missing — a caller can supply a rule or use an index, and
    /// neither is discoverable from the name of a Rust struct.
    UnregisteredArray {
        path: String,
    },
    SlugNotFound {
        prefix: String,
        slug: String,
    },
    /// A non-numeric segment matched several elements of the array at `prefix`.
    ///
    /// Substring matching can produce this, as can an explicit keyed-list field
    /// when an externally authored document contains duplicate identities.
    /// Naming several things at once is not an address, and picking the first
    /// would silently answer a different question than the one asked, so it is
    /// refused with structural candidate indices.
    ///
    /// Candidate text is deliberately absent: an error must not become a route
    /// for document content to bypass normal output redaction.
    AmbiguousMatch {
        prefix: String,
        segment: String,
        indices: Vec<usize>,
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
    /// A parser rejected the source. `detail` is the parser's own text, which
    /// quotes the offending line — see [`Self::redacted_message`].
    ParseError {
        format: String,
        detail: String,
    },
    /// A dot-path is malformed: a bad escape, a trailing `\`, a bare `*`, an
    /// index past the platform's range.
    ///
    /// Distinct from [`Self::ParseError`] because nothing here came from the
    /// document — the caller's own address is what failed to parse, and
    /// `detail` is afdata's own words about it. Sharing a code with a rejected
    /// *file* sent readers to inspect the wrong thing.
    PathSyntax {
        detail: String,
    },
    /// afdata declines to read a source its parser would accept, because it
    /// cannot answer honestly about it.
    ///
    /// `detail` is authored here and names the way out; it holds no document
    /// text, so [`Self::redacted_message`] keeps it. Distinct from
    /// [`Self::ParseError`] because the file is not malformed — reporting it as
    /// a parse failure sends the reader hunting for a syntax error that is not
    /// there.
    SourceRefused {
        format: String,
        detail: String,
    },
    /// A caller argument contradicts itself or the document. `detail` is
    /// afdata's own words about the argument, never document content.
    InvalidArgument {
        detail: String,
    },
    /// A staged edit rendered source this format's own parser rejects, caught
    /// by the read-back in `save_atomic` before any bytes reached disk.
    ///
    /// `detail` is already redacted: it comes from
    /// [`Self::redacted_message`] of the rejection, not from its `Display`.
    WriteWouldCorrupt {
        format: String,
        detail: String,
    },
    /// No format could be inferred for `path`, so nothing was parsed at all.
    ///
    /// Distinct from [`Self::ParseError`] because it is about the file's name,
    /// never its contents: it carries no document text, and dropping its detail
    /// as a precaution would throw away the only actionable thing it says.
    FormatUnknown {
        path: String,
    },
    /// A create-only commit found an existing target. Kept separate from
    /// [`Self::IoError`] so callers can safely implement idempotent create
    /// workflows without parsing platform-specific I/O messages.
    AlreadyExists {
        path: String,
    },
    /// A capped read found more bytes than the caller allowed. Kept separate
    /// from [`Self::IoError`] for the same reason as [`Self::AlreadyExists`]: a
    /// caller enforcing a size budget has to tell "too big" from "missing" or
    /// "unreadable", and should not have to match on a message to do it.
    TooLarge {
        path: String,
        max_bytes: u64,
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
                write!(
                    f,
                    "array at `{}` has no rule for naming an element by its content; \
                     address an element by index, or name the field that identifies one",
                    path
                )
            }
            DocumentError::SlugNotFound { prefix, slug } => {
                write!(f, "no element with slug `{}` found in `{}`", slug, prefix)
            }
            DocumentError::SlugAlreadyExists { prefix, slug } => {
                write!(f, "slug `{}` already exists in `{}`", slug, prefix)
            }
            DocumentError::AmbiguousMatch {
                prefix,
                segment,
                indices,
            } => {
                let candidates = indices
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "segment `{}` matches {} elements of `{}` at indices {}",
                    segment,
                    indices.len(),
                    prefix,
                    candidates
                )
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
            DocumentError::PathSyntax { detail } => {
                write!(f, "invalid path: {}", detail)
            }
            DocumentError::SourceRefused { format, detail } => {
                write!(f, "refusing to read this {}: {}", format, detail)
            }
            DocumentError::InvalidArgument { detail } => {
                write!(f, "invalid argument: {}", detail)
            }
            DocumentError::WriteWouldCorrupt { format, detail } => {
                write!(
                    f,
                    "refusing to write: the edit produced {} this parser rejects ({}); the file is unchanged",
                    format, detail
                )
            }
            DocumentError::FormatUnknown { path } => {
                write!(
                    f,
                    "cannot detect format from file extension `{}`; pass an explicit format",
                    path
                )
            }
            DocumentError::AlreadyExists { path } => {
                write!(f, "document target `{path}` already exists")
            }
            DocumentError::TooLarge { path, max_bytes } => {
                write!(f, "`{path}` exceeds the {max_bytes}-byte read limit")
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
    /// Multiple variants share a code only when callers should handle them in
    /// the same way. An ordinary missing path is `document_path_not_found`;
    /// named array lookup distinguishes a missing slug from an ambiguous one.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ParseError { .. } => "document_parse_failed",
            Self::PathSyntax { .. } => "document_invalid_path",
            Self::SourceRefused { .. } => "document_source_refused",
            Self::FormatUnknown { .. } => "document_format_unknown",
            Self::AlreadyExists { .. } => "document_target_exists",
            Self::TooLarge { .. } => "document_too_large",
            Self::WriteWouldCorrupt { .. } => "document_write_would_corrupt",
            Self::PathNotFound { .. }
            | Self::UnknownSegment { .. }
            | Self::IndexOutOfBounds { .. }
            | Self::UnregisteredArray { .. } => "document_path_not_found",
            Self::NotTraversable { .. } | Self::TypeMismatch { .. } => "document_type_mismatch",
            Self::SlugNotFound { .. } => "document_slug_not_found",
            Self::AmbiguousMatch { .. } => "document_ambiguous_match",
            Self::SlugAlreadyExists { .. } => "document_slug_exists",
            Self::IoError { .. } => "document_io_failed",
            Self::UnsupportedOperation { .. } => "document_unsupported_operation",
            Self::EmptyPath | Self::EmptyValues | Self::InvalidArgument { .. } => {
                "document_invalid_argument"
            }
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
        // A parser diagnostic opens with its own position and echoes the
        // offending source on the lines below it:
        //
        //     TOML parse error at line 2, column 5
        //       |
        //     2 | note = "see at line 999 for details" bad
        //
        // So the position is on the first line and document content never is.
        // Searching the whole detail — from either end — can read the echo:
        // from the end it finds `at line 999`, which is the file's own text.
        let head = detail.split('\n').next().unwrap_or(detail);
        let rest = match head.find(" at line ") {
            Some(start) => &head[start + " at line ".len()..],
            None => head.strip_prefix("line ")?,
        };
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
    /// Two variants can quote material that originates in the document and are
    /// rewritten here:
    ///
    /// - [`DocumentError::ParseError`] renders as `failed to parse {format}`
    ///   (with the [`location`](Self::location) appended when known), dropping
    ///   the parser detail, which echoes a snippet of the source.
    ///
    /// [`DocumentError::PathSyntax`], [`DocumentError::SourceRefused`] and
    /// [`DocumentError::InvalidArgument`] keep their detail in full. It is the
    /// reason they exist as separate variants: their text is written here, about
    /// the caller's address, argument, or file *encoding* — never lifted from
    /// document content — and it is the only part that says what to do next.
    /// Dropping it as a precaution against a leak that cannot happen turned an
    /// actionable refusal into `failed to parse Markdown`.
    /// - [`DocumentError::TypeMismatch`] drops `got` and `hint`. When built by
    ///   [`Self::from_serde`] those carry serde's rendering of the offending
    ///   value, which is document content.
    ///
    /// Every other variant renders the same as its [`Display`], carrying only
    /// structural context: paths, requested slugs, indices, and type or format
    /// names. In particular, [`Self::AmbiguousMatch`] carries candidate indices
    /// rather than matched field values.
    /// [`DocumentError::NotTraversable`] belongs to that group because `got` is
    /// a [`Value::kind_name`](crate::document::Value::kind_name), not a value.
    #[must_use]
    pub fn redacted_message(&self) -> String {
        match self {
            Self::ParseError { format, .. } => match self.location() {
                Some(location) => format!("failed to parse {format} at {location}"),
                None => format!("failed to parse {format}"),
            },
            Self::TypeMismatch { path, expected, .. } => {
                if expected.is_empty() {
                    format!("field `{path}` has the wrong type")
                } else {
                    format!("field `{path}` expects {expected}")
                }
            }
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
                DocumentError::AmbiguousMatch {
                    prefix: "items".to_string(),
                    segment: "look".to_string(),
                    indices: vec![0, 2],
                },
                "document_ambiguous_match",
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
                DocumentError::AlreadyExists {
                    path: "config.toml".to_string(),
                },
                "document_target_exists",
            ),
            (
                DocumentError::TooLarge {
                    path: "config.toml".to_string(),
                    max_bytes: 1024,
                },
                "document_too_large",
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
        // The real layout a parser produces: its own position first, then the
        // offending source echoed below. The echo here carries `at line 999`
        // out of the document, and a search from the end reads exactly that —
        // reporting a wrong line and leaking a document-derived number through
        // `redacted_message`, which promises never to surface file content.
        let err = DocumentError::ParseError {
            format: "TOML".to_string(),
            detail: "TOML parse error at line 5, column 12\n  |\n\
                     5 | note = \"see at line 999 for TOPSECRET\" bad\n  |     ^"
                .to_string(),
        };
        assert_eq!(err.location().as_deref(), Some("line 5 column 12"));
        assert!(!err.redacted_message().contains("999"));
        assert!(!err.redacted_message().contains("TOPSECRET"));

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

    #[test]
    fn redacted_message_drops_the_offending_value() {
        // `from_serde` keeps serde's rendering, which quotes the value that
        // failed the type check — document content, and a secret as often as not.
        let err = DocumentError::from_serde(
            "credentials.token",
            "invalid type: string \"sk-live-TOPSECRET\", expected u16",
        );
        assert!(err.to_string().contains("sk-live-TOPSECRET"));
        let redacted = err.redacted_message();
        assert!(!redacted.contains("sk-live-TOPSECRET"), "{redacted}");
        assert!(redacted.contains("credentials.token"), "{redacted}");
    }

    #[test]
    fn ambiguous_match_reports_indices_without_document_content() {
        let err = DocumentError::AmbiguousMatch {
            prefix: "h1.0.h2".to_string(),
            segment: "look".to_string(),
            indices: vec![0, 2],
        };
        assert_eq!(
            err.redacted_message(),
            "segment `look` matches 2 elements of `h1.0.h2` at indices 0, 2"
        );
        assert!(!err.redacted_message().contains("Quick look"));
    }

    #[test]
    fn not_traversable_names_the_type_not_the_value() {
        // This one is safe by construction rather than by redaction: `got` is a
        // kind name, so even `Display` cannot echo the leaf.
        let err = DocumentError::NotTraversable {
            path: "token_secret.inner".to_string(),
            got: crate::document::Value::String("sk-live-TOPSECRET".to_string())
                .kind_name()
                .to_string(),
        };
        assert_eq!(
            err.to_string(),
            "path `token_secret.inner` is string, cannot traverse further"
        );
        assert_eq!(err.redacted_message(), err.to_string());
    }
}
