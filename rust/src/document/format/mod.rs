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
#[cfg(feature = "markdown")]
pub mod markdown;
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
    /// A CommonMark document read as a tree of heading sections (`preamble`,
    /// `h1`, `h1.0.h2`, …). Read-only, and never auto-detected — the same `.md`
    /// file is legitimately readable as frontmatter, and choosing between two
    /// valid readings is not something an extension can decide.
    Markdown,
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
            Self::Markdown => "Markdown",
        }
    }

    /// Whether this format can be written at all.
    ///
    /// Markdown is the one reader-only format: its parsed value is a flattened
    /// view of prose, so re-rendering a document from it would discard
    /// everything the flattening dropped. Every mutating verb refuses up front
    /// rather than producing that file.
    #[must_use]
    pub const fn is_read_only(self) -> bool {
        matches!(self, Self::Markdown)
    }

    /// This format's own rule for resolving a **non-numeric** path segment
    /// against an array, or `None` when only a caller-declared keyed list can.
    ///
    /// A format earns a rule by owning the shape of its own value. Markdown's
    /// value is a tree afdata synthesized — every node carries `text` because
    /// this crate put it there — so `h2.look` can be answered from the format
    /// alone. JSON, TOML, YAML, dotenv, and INI hand back whatever the file
    /// said; nothing in `deps.foo` tells afdata which field of a `deps` element
    /// `foo` is supposed to match, and inventing one is the shape-guessing this
    /// crate exists to refuse. Those keep needing an explicit
    /// [`KeyedList`](crate::document::KeyedList).
    #[must_use]
    pub const fn array_rule(self) -> Option<crate::document::ArrayRule<'static>> {
        match self {
            Self::Markdown => Some(crate::document::ArrayRule {
                field: "text",
                match_kind: crate::document::MatchKind::Contains,
            }),
            _ => None,
        }
    }

    /// The refusal every mutating operation answers with for a read-only
    /// format. One constructor so `save` and the document verbs cannot drift
    /// into giving two different reasons for the same fact.
    pub(crate) fn read_only_error(self, operation: &str) -> DocumentError {
        DocumentError::UnsupportedOperation {
            format: self.name().to_string(),
            operation: operation.to_string(),
            detail: format!(
                "{} is a read-only format: afdata reads its structure and never writes it",
                self.name()
            ),
        }
    }

    /// Exact CLI token accepted by `--input-format` and emitted in result
    /// payloads. Unlike [`Self::name`], this is stable machine data rather than
    /// a display label.
    #[must_use]
    pub const fn cli_name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Yaml => "yaml",
            Self::Dotenv => "dotenv",
            Self::Ini => "ini",
            Self::TomlFrontmatter => "toml-frontmatter",
            Self::YamlFrontmatter => "yaml-frontmatter",
            Self::Markdown => "markdown",
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

            #[cfg(feature = "markdown")]
            Format::Markdown => markdown::load(content),
            #[cfg(not(feature = "markdown"))]
            Format::Markdown => Err(DocumentError::UnsupportedOperation {
                format: "Markdown".to_string(),
                operation: "load".to_string(),
                detail: "requires Cargo feature `markdown`".to_string(),
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

            // A read-only format has no writer at all — not even a
            // non-preserving one. See `Format::is_read_only`.
            Format::Markdown => Err(self.read_only_error("save")),
        }
    }
}

#[cfg(feature = "dotenv")]
pub use dotenv::load as load_dotenv;
pub use json::{load as load_json, save as save_json};
#[cfg(feature = "markdown")]
pub use markdown::load as load_markdown;
#[cfg(feature = "toml")]
pub use toml::{load as load_toml, save as save_toml};
#[cfg(feature = "yaml")]
pub use yaml::{load as load_yaml, save as save_yaml};

#[cfg(test)]
mod tests {
    use super::Format;
    use std::path::Path;

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
            (Format::Markdown, "Markdown"),
        ];

        for (format, expected) in cases {
            assert_eq!(format.name(), expected);
        }

        let cli_names = [
            (Format::Json, "json"),
            (Format::Toml, "toml"),
            (Format::Yaml, "yaml"),
            (Format::Dotenv, "dotenv"),
            (Format::Ini, "ini"),
            (Format::TomlFrontmatter, "toml-frontmatter"),
            (Format::YamlFrontmatter, "yaml-frontmatter"),
            (Format::Markdown, "markdown"),
        ];
        for (format, expected) in cli_names {
            assert_eq!(format.cli_name(), expected);
        }
    }

    #[test]
    fn markdown_is_the_only_read_only_format() {
        for format in [
            Format::Json,
            Format::Toml,
            Format::Yaml,
            Format::Dotenv,
            Format::Ini,
            Format::TomlFrontmatter,
            Format::YamlFrontmatter,
        ] {
            let name = format.name();
            assert!(!format.is_read_only(), "{name} must stay writable");
        }
        assert!(Format::Markdown.is_read_only());
    }

    #[test]
    fn markdown_is_never_detected_from_an_extension() {
        // `.md` is genuinely ambiguous — the same file is readable as
        // frontmatter — so it resolves to no format and the caller must say
        // which reading it wants.
        assert_eq!(Format::detect(Path::new("README.md")), None);
        assert_eq!(Format::detect(Path::new("README.markdown")), None);
    }
}
