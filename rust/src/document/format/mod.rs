//! Format detection and backend selection.

#[allow(unused_imports)]
use crate::document::{DocumentError, DocumentResult, Value};
use std::path::Path;

#[cfg(feature = "dotenv")]
pub mod dotenv;
// The frontmatter splitter has no format dependency, so it always compiles; the
// inner TOML/YAML backends it delegates to are gated at the call sites below.
pub mod frontmatter;
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
    /// A `+++`-fenced TOML frontmatter block; the Markdown body is frozen. Never
    /// auto-detected — selected only via `--input-format toml-frontmatter`.
    TomlFrontmatter,
    /// A `---`-fenced YAML frontmatter block; the Markdown body is frozen. Never
    /// auto-detected — selected only via `--input-format yaml-frontmatter`.
    YamlFrontmatter,
}

impl Format {
    /// Stable human-readable label used in document results and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Toml => "TOML",
            Self::Yaml => "YAML",
            Self::Dotenv => "dotenv",
            Self::Ini => "INI",
            Self::TomlFrontmatter => "TOML frontmatter",
            Self::YamlFrontmatter => "YAML frontmatter",
        }
    }

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

            #[cfg(feature = "toml")]
            Format::TomlFrontmatter => {
                toml::load(frontmatter::split(content, frontmatter::Delimiter::Plus)?.frontmatter)
            }
            #[cfg(not(feature = "toml"))]
            Format::TomlFrontmatter => Err(DocumentError::UnsupportedOperation {
                format: "TOML frontmatter".to_string(),
                operation: "load".to_string(),
                detail: "requires Cargo feature `toml`".to_string(),
            }),

            #[cfg(feature = "yaml")]
            Format::YamlFrontmatter => {
                yaml::load(frontmatter::split(content, frontmatter::Delimiter::Dash)?.frontmatter)
            }
            #[cfg(not(feature = "yaml"))]
            Format::YamlFrontmatter => Err(DocumentError::UnsupportedOperation {
                format: "YAML frontmatter".to_string(),
                operation: "load".to_string(),
                detail: "requires Cargo feature `yaml`".to_string(),
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

            // Frontmatter has no whole-document re-render: the Markdown body is
            // frozen source, not part of the parsed value, so a fresh render
            // cannot reconstruct the file. Edits go through the source-preserving
            // set/unset seam (see `DocumentFile`), never here.
            Format::TomlFrontmatter | Format::YamlFrontmatter => {
                Err(DocumentError::UnsupportedOperation {
                    format: "frontmatter".to_string(),
                    operation: "save".to_string(),
                    detail:
                        "frontmatter mode has no whole-document re-render; the Markdown body is \
                             not part of the parsed value — use source-preserving set/unset"
                            .to_string(),
                })
            }
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

#[cfg(test)]
mod tests {
    use super::Format;

    #[test]
    fn format_names_are_stable() {
        let cases = [
            (Format::Json, "JSON"),
            (Format::Toml, "TOML"),
            (Format::Yaml, "YAML"),
            (Format::Dotenv, "dotenv"),
            (Format::Ini, "INI"),
            (Format::TomlFrontmatter, "TOML frontmatter"),
            (Format::YamlFrontmatter, "YAML frontmatter"),
        ];

        for (format, expected) in cases {
            assert_eq!(format.name(), expected);
        }
    }
}
