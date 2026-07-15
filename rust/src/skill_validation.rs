//! Agent Skills `SKILL.md` front-matter validation.
//!
//! This module implements the metadata constraints from the Agent Skills
//! specification plus explicitly modeled, common client extensions. It parses
//! only the YAML front matter; Markdown body content is intentionally left
//! unrestricted.

use noyalib::{ParserConfig, from_str_with_config};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

#[cfg(feature = "skill-admin")]
pub use crate::skill_admin::*;

/// Validated Agent Skill metadata.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkillMetadata {
    /// Skill name and directory name.
    pub name: String,
    /// What the skill does and when an agent should use it.
    pub description: String,
    /// License name or reference to a bundled license file.
    pub license: Option<String>,
    /// Optional environment requirements.
    pub compatibility: Option<String>,
    /// Extension metadata defined by clients or skill authors.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Experimental, space-separated pre-approved tool declarations.
    #[serde(rename = "allowed-tools")]
    pub allowed_tools: Option<String>,
    /// Claude-compatible extension that prevents automatic model invocation.
    #[serde(rename = "disable-model-invocation")]
    pub disable_model_invocation: Option<bool>,
    /// Claude-compatible extension that controls direct user invocation.
    #[serde(rename = "user-invocable")]
    pub user_invocable: Option<bool>,
}

/// A deterministic `SKILL.md` validation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillValidationError {
    message: String,
}

impl SkillValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the operator-facing failure detail.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for SkillValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SkillValidationError {}

/// Validate one `SKILL.md` document and return its typed metadata.
///
/// The YAML parser uses its strict resource and duplicate-key policy. The
/// metadata fields and limits follow the Agent Skills specification; unknown
/// fields outside the documented client extensions are rejected.
pub fn validate_skill(text: &str) -> Result<SkillMetadata, SkillValidationError> {
    let frontmatter = extract_frontmatter(text)?;
    let metadata = from_str_with_config::<SkillMetadata>(frontmatter, &ParserConfig::strict())
        .map_err(|error| {
            SkillValidationError::new(format!("invalid YAML front matter: {error}"))
        })?;
    validate_metadata(&metadata)?;
    Ok(metadata)
}

/// Validate one `SKILL.md` document and require its name to match its directory.
pub fn validate_skill_named(
    text: &str,
    expected_name: &str,
) -> Result<SkillMetadata, SkillValidationError> {
    let metadata = validate_skill(text)?;
    if metadata.name != expected_name {
        return Err(SkillValidationError::new(format!(
            "skill name {:?} does not match directory name {expected_name:?}",
            metadata.name
        )));
    }
    Ok(metadata)
}

fn extract_frontmatter(text: &str) -> Result<&str, SkillValidationError> {
    let opening_end = text
        .find('\n')
        .ok_or_else(|| SkillValidationError::new("missing YAML front matter"))?;
    if text[..opening_end].trim_end_matches('\r') != "---" {
        return Err(SkillValidationError::new(
            "missing opening --- YAML front matter delimiter",
        ));
    }

    let frontmatter_start = opening_end + 1;
    let mut line_start = frontmatter_start;
    while line_start <= text.len() {
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |offset| line_start + offset);
        if text[line_start..line_end].trim_end_matches('\r') == "---" {
            return Ok(&text[frontmatter_start..line_start]);
        }
        if line_end == text.len() {
            break;
        }
        line_start = line_end + 1;
    }
    Err(SkillValidationError::new(
        "missing closing --- YAML front matter delimiter",
    ))
}

fn validate_metadata(metadata: &SkillMetadata) -> Result<(), SkillValidationError> {
    validate_name(&metadata.name)?;
    validate_length("description", &metadata.description, 1, 1024)?;
    if let Some(compatibility) = metadata.compatibility.as_deref() {
        validate_length("compatibility", compatibility, 1, 500)?;
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), SkillValidationError> {
    let length = name.chars().count();
    if !(1..=64).contains(&length) {
        return Err(SkillValidationError::new(
            "name must contain between 1 and 64 characters",
        ));
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(SkillValidationError::new(
            "name may contain only lowercase ASCII letters, digits, and hyphens",
        ));
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Err(SkillValidationError::new(
            "name must not start or end with a hyphen or contain consecutive hyphens",
        ));
    }
    Ok(())
}

fn validate_length(
    field: &str,
    value: &str,
    minimum: usize,
    maximum: usize,
) -> Result<(), SkillValidationError> {
    let length = value.chars().count();
    if !(minimum..=maximum).contains(&length) {
        return Err(SkillValidationError::new(format!(
            "{field} must contain between {minimum} and {maximum} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = concat!(
        "---\n",
        "name: code-review\n",
        "description: Reviews code. Use when checking a patch.\n",
        "license: MIT\n",
        "compatibility: Requires git\n",
        "metadata:\n",
        "  author: example-org\n",
        "allowed-tools: Bash(git:*) Read\n",
        "disable-model-invocation: false\n",
        "user-invocable: true\n",
        "---\n",
        "# Code review\n",
    );

    #[test]
    fn validates_spec_fields() {
        let result = validate_skill_named(VALID, "code-review");
        assert_eq!(
            result.map(|metadata| metadata.name),
            Ok("code-review".to_string())
        );
    }

    #[test]
    fn rejects_duplicate_or_unknown_fields() {
        let duplicate = "---\nname: first\nname: second\ndescription: valid\n---\n";
        assert!(validate_skill(duplicate).is_err());

        let unknown = "---\nname: first\ndescription: valid\nversion: 1\n---\n";
        assert!(validate_skill(unknown).is_err());
    }

    #[test]
    fn rejects_invalid_names_and_directory_mismatches() {
        let invalid = "---\nname: Bad--Name\ndescription: valid\n---\n";
        assert!(validate_skill(invalid).is_err());
        assert!(validate_skill_named(VALID, "other-name").is_err());
    }

    #[test]
    fn supports_crlf_frontmatter() {
        let input = "---\r\nname: code-review\r\ndescription: valid\r\n---\r\n# Body\r\n";
        assert!(validate_skill(input).is_ok());
    }
}
