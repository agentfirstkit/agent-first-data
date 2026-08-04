//! Stable, public-facing domain error declarations.
//!
//! Runtime diagnostics are deliberately absent from these types. A parser,
//! database, or network error may be logged separately, but it cannot drift
//! into a protocol message merely because an application formats `source`.

use crate::{BuildError, ErrorBuilder, Event, json_error};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Stable fields that are safe to return to a caller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorSpec {
    code: String,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
    #[serde(default)]
    retryable: bool,
}

impl ErrorSpec {
    /// Declare a stable error code and public message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
            retryable: false,
        }
    }

    /// Add a stable caller-facing recovery hint.
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        let hint = hint.into();
        self.hint = (!hint.is_empty()).then_some(hint);
        self
    }

    /// Declare whether retrying may succeed without changing the request.
    pub const fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn hint_value(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    pub const fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// Start an error builder using only catalogued public fields.
    ///
    /// Applications may add explicitly selected structured fields to the
    /// returned builder. No runtime diagnostic is accepted by this API.
    pub fn builder(&self) -> ErrorBuilder {
        json_error(&self.code, &self.message)
            .hint_if_some(self.hint.as_deref())
            .retryable_if(self.retryable)
    }

    /// Build the strict protocol error event.
    pub fn event(&self) -> Result<Event, BuildError> {
        self.builder().build()
    }
}

/// Failure to construct or query an [`ErrorCatalog`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorCatalogError {
    DuplicateCode(String),
    UnknownCode(String),
    InvalidSpec { code: String, source: BuildError },
}

impl fmt::Display for ErrorCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateCode(code) => write!(formatter, "duplicate error code {code:?}"),
            Self::UnknownCode(code) => write!(formatter, "unknown error code {code:?}"),
            Self::InvalidSpec { code, source } => {
                write!(formatter, "invalid error spec {code:?}: {source}")
            }
        }
    }
}

impl std::error::Error for ErrorCatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidSpec { source, .. } => Some(source),
            Self::DuplicateCode(_) | Self::UnknownCode(_) => None,
        }
    }
}

/// Validated lookup table of stable domain errors.
#[derive(Clone, Debug, Default)]
pub struct ErrorCatalog {
    specs: BTreeMap<String, ErrorSpec>,
}

impl ErrorCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate and collect a finite set of declarations.
    pub fn from_specs<I>(specs: I) -> Result<Self, ErrorCatalogError>
    where
        I: IntoIterator<Item = ErrorSpec>,
    {
        let mut catalog = Self::new();
        for spec in specs {
            catalog.insert(spec)?;
        }
        Ok(catalog)
    }

    /// Validate and insert one declaration.
    pub fn insert(&mut self, spec: ErrorSpec) -> Result<(), ErrorCatalogError> {
        let code = spec.code.clone();
        if self.specs.contains_key(&code) {
            return Err(ErrorCatalogError::DuplicateCode(code));
        }
        spec.event()
            .map_err(|source| ErrorCatalogError::InvalidSpec {
                code: code.clone(),
                source,
            })?;
        self.specs.insert(code, spec);
        Ok(())
    }

    pub fn get(&self, code: &str) -> Option<&ErrorSpec> {
        self.specs.get(code)
    }

    /// Start a builder for a catalogued code.
    pub fn builder(&self, code: &str) -> Result<ErrorBuilder, ErrorCatalogError> {
        self.get(code)
            .map(ErrorSpec::builder)
            .ok_or_else(|| ErrorCatalogError::UnknownCode(code.to_string()))
    }

    /// Build a strict error event for a catalogued code.
    pub fn event(&self, code: &str) -> Result<Event, ErrorCatalogError> {
        self.builder(code)?
            .build()
            .map_err(|source| ErrorCatalogError::InvalidSpec {
                code: code.to_string(),
                source,
            })
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{ErrorCatalog, ErrorCatalogError, ErrorSpec};
    use crate::validate_protocol_event;

    #[test]
    fn catalog_builds_only_declared_public_fields() {
        let catalog = ErrorCatalog::from_specs([ErrorSpec::new(
            "config_load_failed",
            "Failed to load configuration",
        )
        .hint("inspect the configuration and retry")
        .retryable(false)])
        .unwrap_or_else(|error| panic!("{error}"));

        let event = catalog
            .event("config_load_failed")
            .unwrap_or_else(|error| panic!("{error}"));
        validate_protocol_event(event.as_value(), true).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            event.as_value()["error"]["message"],
            "Failed to load configuration"
        );
        assert!(!event.to_string().contains("database diagnostic"));
    }

    #[test]
    fn catalog_rejects_invalid_duplicate_and_unknown_codes() {
        let invalid = ErrorCatalog::from_specs([ErrorSpec::new("", "message")]);
        assert!(matches!(
            invalid,
            Err(ErrorCatalogError::InvalidSpec { .. })
        ));

        let duplicate = ErrorCatalog::from_specs([
            ErrorSpec::new("same", "first"),
            ErrorSpec::new("same", "second"),
        ]);
        assert!(matches!(
            duplicate,
            Err(ErrorCatalogError::DuplicateCode(code)) if code == "same"
        ));

        let empty = ErrorCatalog::new();
        assert!(matches!(
            empty.event("missing"),
            Err(ErrorCatalogError::UnknownCode(code)) if code == "missing"
        ));
    }
}
