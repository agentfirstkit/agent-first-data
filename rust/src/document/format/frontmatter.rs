//! Frontmatter mode: address the leading TOML (`+++`) or YAML (`---`) metadata
//! block of a Markdown-ish file as a config document, leaving the body bytes
//! untouched.
//!
//! This backend does not parse Markdown. It splits the source into three
//! byte-exact spans — the opening delimiter line, the frontmatter block, and
//! the closing delimiter line plus everything after it (the body) — and hands
//! only the middle span to the inner TOML/YAML backend. Every edit re-splices
//! `pre + edited_frontmatter + post`, so the body survives verbatim; the frozen
//! body is exactly why frontmatter mode is source-preserving-only (there is no
//! whole-document re-render — the body is not part of the parsed value).
//!
//! Detection is never automatic: the caller selects the delimiter explicitly
//! (`--input-format toml-frontmatter|yaml-frontmatter`). A file that does not
//! open with the requested delimiter, or whose block is never closed, is a hard
//! error — sniffing a fence is exactly the shape-guessing AFDATA avoids.

use crate::document::{DocumentError, DocumentResult};

/// The fence delimiter that brackets a frontmatter block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    /// `+++` — Zola's TOML frontmatter.
    Plus,
    /// `---` — Jekyll/Hugo/Obsidian YAML frontmatter.
    Dash,
}

impl Delimiter {
    /// The literal fence line for this delimiter.
    fn marker(self) -> &'static str {
        match self {
            Delimiter::Plus => "+++",
            Delimiter::Dash => "---",
        }
    }

    /// Human-readable format label used in error envelopes.
    fn label(self) -> &'static str {
        match self {
            Delimiter::Plus => "TOML frontmatter",
            Delimiter::Dash => "YAML frontmatter",
        }
    }
}

/// Three byte-exact spans of a frontmatter document: the opening delimiter line
/// (`pre`), the frontmatter block handed to the inner backend (`frontmatter`),
/// and the closing delimiter line plus the body (`post`). Concatenating `pre`,
/// `frontmatter`, and `post` reproduces the original source byte for byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts<'a> {
    pub pre: &'a str,
    pub frontmatter: &'a str,
    pub post: &'a str,
}

/// Split `source` into its frontmatter spans for the given delimiter.
///
/// The opening delimiter must be the file's very first line (no leading blank
/// lines, no BOM). The closing delimiter is the next line equal to the marker
/// at column zero. A missing opening or closing fence is a [`DocumentError`],
/// never a silent "there is no frontmatter, treat the whole file as body".
pub fn split(source: &str, delim: Delimiter) -> DocumentResult<Parts<'_>> {
    let marker = delim.marker();

    let first_line_end = source.find('\n');
    let first_line = match first_line_end {
        Some(nl) => &source[..nl],
        None => source,
    };
    if first_line.trim_end() != marker {
        return Err(DocumentError::ParseError {
            format: delim.label().to_string(),
            detail: format!("file does not begin with a `{marker}` frontmatter delimiter line"),
        });
    }
    let Some(nl) = first_line_end else {
        // A bare `+++`/`---` with no trailing newline: opened, never closed.
        return Err(unterminated(delim));
    };
    let block_start = nl + 1;

    // The closing delimiter is the next line whose trimmed content is the
    // marker. Leading whitespace disqualifies it (an indented `+++` is content,
    // not a fence); trailing whitespace/CR is tolerated.
    let mut idx = block_start;
    loop {
        let line_end = source[idx..]
            .find('\n')
            .map(|offset| idx + offset)
            .unwrap_or(source.len());
        let line = &source[idx..line_end];
        if line.trim_end() == marker {
            return Ok(Parts {
                pre: &source[..block_start],
                frontmatter: &source[block_start..idx],
                post: &source[idx..],
            });
        }
        if line_end == source.len() {
            return Err(unterminated(delim));
        }
        idx = line_end + 1;
    }
}

fn unterminated(delim: Delimiter) -> DocumentError {
    DocumentError::ParseError {
        format: delim.label().to_string(),
        detail: format!(
            "unterminated frontmatter: no closing `{}` delimiter",
            delim.marker()
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]
    use super::*;

    fn assert_roundtrip(source: &str, parts: &Parts<'_>) {
        assert_eq!(
            format!("{}{}{}", parts.pre, parts.frontmatter, parts.post),
            source,
            "spans must concatenate back to the source byte for byte"
        );
    }

    #[test]
    fn splits_toml_frontmatter_and_freezes_body() {
        let source = "+++\ntitle = \"x\"\n+++\n# Body\n\nprose\n";
        let parts = split(source, Delimiter::Plus).unwrap();
        assert_eq!(parts.pre, "+++\n");
        assert_eq!(parts.frontmatter, "title = \"x\"\n");
        assert_eq!(parts.post, "+++\n# Body\n\nprose\n");
        assert_roundtrip(source, &parts);
    }

    #[test]
    fn splits_yaml_frontmatter() {
        let source = "---\ntitle: x\n---\nbody\n";
        let parts = split(source, Delimiter::Dash).unwrap();
        assert_eq!(parts.pre, "---\n");
        assert_eq!(parts.frontmatter, "title: x\n");
        assert_eq!(parts.post, "---\nbody\n");
        assert_roundtrip(source, &parts);
    }

    #[test]
    fn empty_block_is_valid() {
        let source = "+++\n+++\nbody\n";
        let parts = split(source, Delimiter::Plus).unwrap();
        assert_eq!(parts.frontmatter, "");
        assert_eq!(parts.post, "+++\nbody\n");
        assert_roundtrip(source, &parts);
    }

    #[test]
    fn frontmatter_only_no_body() {
        let source = "+++\ntitle = \"x\"\n+++\n";
        let parts = split(source, Delimiter::Plus).unwrap();
        assert_eq!(parts.frontmatter, "title = \"x\"\n");
        assert_eq!(parts.post, "+++\n");
        assert_roundtrip(source, &parts);
    }

    #[test]
    fn preserves_crlf_line_endings() {
        let source = "+++\r\ntitle = \"x\"\r\n+++\r\nbody\r\n";
        let parts = split(source, Delimiter::Plus).unwrap();
        assert_eq!(parts.pre, "+++\r\n");
        assert_eq!(parts.frontmatter, "title = \"x\"\r\n");
        assert_eq!(parts.post, "+++\r\nbody\r\n");
        assert_roundtrip(source, &parts);
    }

    #[test]
    fn missing_opening_delimiter_errors() {
        let err = split("# Just a markdown heading\n", Delimiter::Plus).unwrap_err();
        assert!(matches!(err, DocumentError::ParseError { .. }));
    }

    #[test]
    fn wrong_delimiter_errors() {
        // A `+++` file addressed as YAML frontmatter must not silently succeed.
        let err = split("+++\ntitle = \"x\"\n+++\n", Delimiter::Dash).unwrap_err();
        assert!(matches!(err, DocumentError::ParseError { .. }));
    }

    #[test]
    fn unterminated_block_errors() {
        let err = split("+++\ntitle = \"x\"\nno closing fence\n", Delimiter::Plus).unwrap_err();
        assert!(matches!(err, DocumentError::ParseError { .. }));
    }

    #[test]
    fn indented_fence_is_not_a_close() {
        // A `+++` that is not at column zero is body content, so the block is
        // unterminated rather than closed there.
        let err = split("+++\ntitle = \"x\"\n  +++\n", Delimiter::Plus).unwrap_err();
        assert!(matches!(err, DocumentError::ParseError { .. }));
    }
}
