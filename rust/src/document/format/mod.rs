//! Format detection and backend selection.

#[allow(unused_imports)]
use crate::document::{DocumentError, DocumentResult, Value};
use std::path::Path;

#[cfg(feature = "dotenv")]
pub mod dotenv;
#[cfg(feature = "ini")]
pub mod ini;
// JSON is a core (non-optional) dependency of agent-first-data, so this
// backend always compiles — unlike toml/yaml/dotenv/ini below.
pub mod json;
#[cfg(feature = "toml")]
pub mod toml;
#[cfg(feature = "yaml")]
pub mod yaml;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Toml,
    Yaml,
    Dotenv,
    Ini,
}

impl Format {
    /// Detect format from file extension.
    pub fn detect(path: &Path) -> Option<Self> {
        let file_name = path.file_name().and_then(|name| name.to_str())?;
        let file_name_lower = file_name.to_lowercase();
        if file_name_lower == ".env"
            || file_name_lower.starts_with(".env.")
            || path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("env"))
        {
            return Some(Format::Dotenv);
        }

        path.extension().and_then(|ext| ext.to_str()).and_then(|s| {
            match s.to_lowercase().as_str() {
                "json" => Some(Format::Json),
                "toml" => Some(Format::Toml),
                "yaml" | "yml" => Some(Format::Yaml),
                "ini" => Some(Format::Ini),
                _ => None,
            }
        })
    }

    /// Load a config file in the detected format.
    pub fn load(&self, content: &str) -> DocumentResult<Value> {
        match self {
            Format::Json => json::load(content),

            #[cfg(feature = "toml")]
            Format::Toml => toml::load(content),
            #[cfg(not(feature = "toml"))]
            Format::Toml => Err(DocumentError::UnsupportedOperation {
                format: "TOML".to_string(),
                operation: "load".to_string(),
                detail: "requires Cargo feature `toml`".to_string(),
            }),

            #[cfg(feature = "yaml")]
            Format::Yaml => yaml::load(content),
            #[cfg(not(feature = "yaml"))]
            Format::Yaml => Err(DocumentError::UnsupportedOperation {
                format: "YAML".to_string(),
                operation: "load".to_string(),
                detail: "requires Cargo feature `yaml`".to_string(),
            }),

            #[cfg(feature = "dotenv")]
            Format::Dotenv => dotenv::load(content),
            #[cfg(not(feature = "dotenv"))]
            Format::Dotenv => Err(DocumentError::UnsupportedOperation {
                format: "dotenv".to_string(),
                operation: "load".to_string(),
                detail: "requires Cargo feature `dotenv`".to_string(),
            }),

            #[cfg(feature = "ini")]
            Format::Ini => ini::load(content),
            #[cfg(not(feature = "ini"))]
            Format::Ini => Err(DocumentError::UnsupportedOperation {
                format: "INI".to_string(),
                operation: "load".to_string(),
                detail: "requires Cargo feature `ini`".to_string(),
            }),
        }
    }

    /// Save a config in the target format.
    pub fn save(&self, value: &Value) -> DocumentResult<String> {
        match self {
            Format::Json => json::save(value),

            #[cfg(feature = "toml")]
            Format::Toml => toml::save(value),
            #[cfg(not(feature = "toml"))]
            Format::Toml => Err(DocumentError::UnsupportedOperation {
                format: "TOML".to_string(),
                operation: "save".to_string(),
                detail: "requires Cargo feature `toml`".to_string(),
            }),

            #[cfg(feature = "yaml")]
            Format::Yaml => yaml::save(value),
            #[cfg(not(feature = "yaml"))]
            Format::Yaml => Err(DocumentError::UnsupportedOperation {
                format: "YAML".to_string(),
                operation: "save".to_string(),
                detail: "requires Cargo feature `yaml`".to_string(),
            }),

            #[cfg(feature = "dotenv")]
            Format::Dotenv => dotenv::save(value),
            #[cfg(not(feature = "dotenv"))]
            Format::Dotenv => Err(DocumentError::UnsupportedOperation {
                format: "dotenv".to_string(),
                operation: "save".to_string(),
                detail: "requires Cargo feature `dotenv`".to_string(),
            }),

            #[cfg(feature = "ini")]
            Format::Ini => ini::save(value),
            #[cfg(not(feature = "ini"))]
            Format::Ini => Err(DocumentError::UnsupportedOperation {
                format: "INI".to_string(),
                operation: "save".to_string(),
                detail: "requires Cargo feature `ini`".to_string(),
            }),
        }
    }

    /// Reject mutation before a backend-specific value is changed or written.
    pub fn ensure_writable(&self, _operation: &str) -> DocumentResult<()> {
        Ok(())
    }
}

#[cfg(feature = "dotenv")]
pub use dotenv::load as load_dotenv;
pub use json::{load as load_json, save as save_json};
#[cfg(feature = "toml")]
pub use toml::{load as load_toml, save as save_toml};
#[cfg(feature = "yaml")]
pub use yaml::{load as load_yaml, save as save_yaml};
