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
                write!(f, "path `{}` not found in config", path)
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
