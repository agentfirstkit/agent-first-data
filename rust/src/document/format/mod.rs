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

/// A format this build can actually read.
///
/// The optional backends gate their variants, not just their bodies: in a build
/// without `toml` there is no `Format::Toml` at all, so code naming a format it
/// did not enable fails to compile instead of returning a runtime refusal from
/// deep inside `load`. Detection still answers for those files — see
/// [`Format::unavailable`] — because a file's format is a fact about the file,
/// not about this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    #[cfg(feature = "toml")]
    Toml,
    #[cfg(feature = "yaml")]
    Yaml,
    #[cfg(feature = "dotenv")]
    Dotenv,
    #[cfg(feature = "ini")]
    Ini,
    /// A `+++`-fenced TOML frontmatter block; the Markdown body is frozen. Never
    /// auto-detected — selected only via `--input-format toml-frontmatter`.
    #[cfg(feature = "toml")]
    TomlFrontmatter,
    /// A `---`-fenced YAML frontmatter block; the Markdown body is frozen. Never
    /// auto-detected — selected only via `--input-format yaml-frontmatter`.
    #[cfg(feature = "yaml")]
    YamlFrontmatter,
    /// A CommonMark document read as a tree of heading sections (`preamble`,
    /// `h1`, `h1.0.h2`, …). Read-only, and never auto-detected — the same `.md`
    /// file is legitimately readable as frontmatter, and choosing between two
    /// valid readings is not something an extension can decide.
    #[cfg(feature = "markdown")]
    Markdown,
}

impl Format {
    /// Stable human-readable label used in document results and diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            #[cfg(feature = "toml")]
            Self::Toml => "TOML",
            #[cfg(feature = "yaml")]
            Self::Yaml => "YAML",
            #[cfg(feature = "dotenv")]
            Self::Dotenv => "dotenv",
            #[cfg(feature = "ini")]
            Self::Ini => "INI",
            #[cfg(feature = "toml")]
            Self::TomlFrontmatter => "TOML frontmatter",
            #[cfg(feature = "yaml")]
            Self::YamlFrontmatter => "YAML frontmatter",
            #[cfg(feature = "markdown")]
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
        match self {
            #[cfg(feature = "markdown")]
            Self::Markdown => true,
            _ => false,
        }
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
            #[cfg(feature = "markdown")]
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

    /// Frontmatter has no whole-document re-render, and both frontmatter
    /// variants say so identically — one constructor so the two gated arms
    /// cannot drift into two different reasons for the same fact.
    #[cfg(any(feature = "toml", feature = "yaml"))]
    fn frontmatter_save_error() -> DocumentError {
        DocumentError::UnsupportedOperation {
            format: "frontmatter".to_string(),
            operation: "save".to_string(),
            detail: "frontmatter mode has no whole-document re-render; the Markdown body is not \
                     part of the parsed value — use source-preserving set/unset"
                .to_string(),
        }
    }

    /// Exact CLI token accepted by `--input-format` and emitted in result
    /// payloads. Unlike [`Self::name`], this is stable machine data rather than
    /// a display label.
    #[must_use]
    pub const fn cli_name(self) -> &'static str {
        match self {
            Self::Json => "json",
            #[cfg(feature = "toml")]
            Self::Toml => "toml",
            #[cfg(feature = "yaml")]
            Self::Yaml => "yaml",
            #[cfg(feature = "dotenv")]
            Self::Dotenv => "dotenv",
            #[cfg(feature = "ini")]
            Self::Ini => "ini",
            #[cfg(feature = "toml")]
            Self::TomlFrontmatter => "toml-frontmatter",
            #[cfg(feature = "yaml")]
            Self::YamlFrontmatter => "yaml-frontmatter",
            #[cfg(feature = "markdown")]
            Self::Markdown => "markdown",
        }
    }

    /// The format a caller named, when this build can read it.
    ///
    /// The inverse of [`Format::cli_name`], plus the spellings a person is
    /// likely to type for the same thing (`yml`, `env`). `None` covers both an
    /// unknown name and a known one this build lacks a parser for — the caller
    /// says which, since only it knows whether that distinction is worth a
    /// different message.
    #[must_use]
    pub fn from_cli_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "json" => Some(Self::Json),
            #[cfg(feature = "toml")]
            "toml" => Some(Self::Toml),
            #[cfg(feature = "yaml")]
            "yaml" | "yml" => Some(Self::Yaml),
            #[cfg(feature = "dotenv")]
            "dotenv" | "env" => Some(Self::Dotenv),
            #[cfg(feature = "ini")]
            "ini" => Some(Self::Ini),
            #[cfg(feature = "toml")]
            "toml-frontmatter" => Some(Self::TomlFrontmatter),
            #[cfg(feature = "yaml")]
            "yaml-frontmatter" => Some(Self::YamlFrontmatter),
            #[cfg(feature = "markdown")]
            "markdown" => Some(Self::Markdown),
            _ => None,
        }
    }

    /// Detect format from file extension, when this build can read it.
    pub fn detect(path: &Path) -> Option<Self> {
        match Self::extension_kind(path)? {
            #[cfg(feature = "dotenv")]
            "dotenv" => Some(Format::Dotenv),
            "json" => Some(Format::Json),
            #[cfg(feature = "toml")]
            "toml" => Some(Format::Toml),
            #[cfg(feature = "yaml")]
            "yaml" => Some(Format::Yaml),
            #[cfg(feature = "ini")]
            "ini" => Some(Format::Ini),
            _ => None,
        }
    }

    /// The Cargo feature a path's format needs, when this build lacks it.
    ///
    /// `detect` answers `None` both for a file this crate has never heard of
    /// and for one it knows perfectly well but was not built to read. Only the
    /// second is worth a different message, and only this can tell them apart,
    /// so the caller reports the missing feature instead of calling a `.toml`
    /// file's format unknown.
    #[must_use]
    pub fn unavailable(path: &Path) -> Option<&'static str> {
        if Self::detect(path).is_some() {
            return None;
        }
        match Self::extension_kind(path)? {
            "dotenv" => Some("dotenv"),
            "toml" => Some("toml"),
            "yaml" => Some("yaml"),
            "ini" => Some("ini"),
            // JSON is a core dependency; there is no feature to be missing.
            _ => None,
        }
    }

    /// The format family a path's name implies, independent of this build.
    ///
    /// Always compiled, for exactly the reason the enum is not: which format a
    /// file is written in does not change with the features this binary chose.
    fn extension_kind(path: &Path) -> Option<&'static str> {
        let file_name = path.file_name().and_then(|name| name.to_str())?;
        let file_name_lower = file_name.to_lowercase();
        if file_name_lower == ".env"
            || file_name_lower.starts_with(".env.")
            || path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("env"))
        {
            return Some("dotenv");
        }

        match path
            .extension()
            .and_then(|ext| ext.to_str())?
            .to_lowercase()
            .as_str()
        {
            "json" => Some("json"),
            "toml" => Some("toml"),
            "yaml" | "yml" => Some("yaml"),
            "ini" => Some("ini"),
            _ => None,
        }
    }

    /// Load a config file in the detected format.
    pub fn load(&self, content: &str) -> DocumentResult<Value> {
        match self {
            Format::Json => json::load(content),

            #[cfg(feature = "toml")]
            Format::Toml => toml::load(content),

            #[cfg(feature = "yaml")]
            Format::Yaml => yaml::load(content),

            #[cfg(feature = "dotenv")]
            Format::Dotenv => dotenv::load(content),

            #[cfg(feature = "ini")]
            Format::Ini => ini::load(content),

            #[cfg(feature = "toml")]
            Format::TomlFrontmatter => {
                toml::load(frontmatter::split(content, frontmatter::Delimiter::Plus)?.frontmatter)
            }

            #[cfg(feature = "yaml")]
            Format::YamlFrontmatter => {
                yaml::load(frontmatter::split(content, frontmatter::Delimiter::Dash)?.frontmatter)
            }

            #[cfg(feature = "markdown")]
            Format::Markdown => markdown::load(content),
        }
    }

    /// Save a config in the target format.
    pub fn save(&self, value: &Value) -> DocumentResult<String> {
        match self {
            Format::Json => json::save(value),

            #[cfg(feature = "toml")]
            Format::Toml => toml::save(value),

            #[cfg(feature = "yaml")]
            Format::Yaml => yaml::save(value),

            #[cfg(feature = "dotenv")]
            Format::Dotenv => dotenv::save(value),

            #[cfg(feature = "ini")]
            Format::Ini => ini::save(value),

            // Frontmatter has no whole-document re-render: the Markdown body is
            // frozen source, not part of the parsed value, so a fresh render
            // cannot reconstruct the file. Edits go through the source-preserving
            // set/unset seam (see `DocumentFile`), never here.
            #[cfg(feature = "toml")]
            Format::TomlFrontmatter => Err(Self::frontmatter_save_error()),
            #[cfg(feature = "yaml")]
            Format::YamlFrontmatter => Err(Self::frontmatter_save_error()),

            // A read-only format has no writer at all — not even a
            // non-preserving one. See `Format::is_read_only`.
            #[cfg(feature = "markdown")]
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

// Every test here enumerates the whole format table, so the module says which
// build it is describing rather than each test repeating the condition. A
// narrowed build has fewer variants by design; the gate exercises the full one.
#[cfg(all(
    test,
    feature = "toml",
    feature = "yaml",
    feature = "dotenv",
    feature = "ini",
    feature = "markdown"
))]
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
