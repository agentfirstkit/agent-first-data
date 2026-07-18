//! Agent-First Data (AFDATA) output formatting and protocol templates.
//!
//! Public APIs, grouped by concern (see each item's own docs for details;
//! the full symbol list is the crate root's own rustdoc index, not repeated
//! here — it drifts out of sync with a hand-maintained count otherwise):
//! - Protocol v1 builders: [`json_result`], [`json_error`], [`json_progress`], [`json_log`]
//!   (each returns a builder; call `.build()`)
//! - Protocol reader: [`decode_protocol_event`] parses and strict-validates one protocol
//!   line into a typed [`DecodedEvent`]
//! - Redaction: [`redacted_value`] / [`Redactor::value`] (JSON values), [`redact_url_secrets`] /
//!   [`Redactor::url`] (URL strings) — `Redactor` carries custom `secret_names`/`policy`
//! - Output rendering: [`render`] — the single `value × format × options → String` entry point
//!   for JSON, YAML, and plain (logfmt) output
//! - Parse utilities: [`normalize_utc_offset`], [`is_valid_rfc3339_date`],
//!   [`is_valid_rfc3339_time`], [`is_valid_rfc3339`], [`is_valid_bcp47`]
//! - CLI helpers: [`cli_parse_output`], [`cli_parse_log_filters`] (returns [`LogFilters`]),
//!   [`build_cli_error`], [`build_cli_version`], [`cli_render_version`],
//!   [`cli_handle_version_or_continue`]
//! - (feature `cli-help`): configurable clap help rendering via [`cli_render_help_with_options`]
//!   and [`cli_handle_help_or_continue`]
//! - (feature `cli-help-markdown`): [`cli_render_help_markdown`] — recursive Markdown help
//! - (feature `skill`): [`skill::validate_skill`] / [`skill::validate_skill_named`] — strict
//!   Agent Skills `SKILL.md` front-matter validation
//! - (feature `skill-admin`): [`skill::run_skill_admin`] — install/uninstall/status a spore's
//!   embedded Agent Skill across Codex, Claude Code, opencode, and Hermes; returns a typed
//!   [`skill::SkillReport`]
//! - (feature `tracing`): [`afdata_tracing::try_init`] initializes an AFDATA stdout logging
//!   layer with configurable format and redaction; also [`afdata_tracing::LogFormat`]
//!
//! The shared cross-language contract (which of these exist, under what name, in each of
//! Rust/Python/TypeScript/Go) is tracked in `spec/api-surface.json` and cross-checked by
//! `scripts/validate_api_surface.py`.

#[cfg(feature = "tracing")]
pub mod afdata_tracing;

#[cfg(feature = "stream-redirect")]
pub mod stream_redirect;

#[cfg(feature = "skill-admin")]
#[path = "skill.rs"]
mod skill_admin;

#[cfg(feature = "skill")]
#[path = "skill_validation.rs"]
pub mod skill;

/// Format-independent document values (dot-path access, typed coercion, and
/// pluggable JSON/TOML/YAML/dotenv/INI backends), ported from `agent-first-config`.
pub mod document;

mod cli;
mod formatting;
#[cfg(feature = "cli-help")]
mod help;
mod protocol;
mod redaction;
mod validation;

pub use cli::{
    CliEmitter, CliEmitterError, LogFilters, OutputFormat, OutputTo, build_cli_version,
    cli_handle_version_or_continue, cli_parse_log_filters, cli_parse_output, cli_render_version,
};
pub use formatting::render;
#[cfg(feature = "cli-help")]
pub use help::{
    HelpConfig, HelpFormat, HelpOptions, HelpScope, cli_handle_help_or_continue, cli_render_help,
    cli_render_help_markdown, cli_render_help_with_options,
};
pub use protocol::{
    BuildError, DecodedError, DecodedEvent, DecodedLog, DecodedProgress, DecodedResult,
    ErrorBuilder, Event, EventDecodeError, LogBuilder, LogLevel, ProgressBuilder,
    ProtocolViolation, ResultBuilder, build_cli_error, decode_protocol_event, json_error, json_log,
    json_progress, json_result, validate_protocol_event, validate_protocol_stream,
};
pub use redaction::{
    OutputOptions, PlainStyle, RedactionPolicy, Redactor, redact_url_secrets, redacted_value,
};
pub use validation::{
    is_valid_bcp47, is_valid_rfc3339, is_valid_rfc3339_date, is_valid_rfc3339_time,
    normalize_utc_offset,
};

#[cfg(test)]
pub(crate) use formatting::{extract_currency_code, format_bytes_human, format_with_commas};

#[cfg(test)]
mod tests;
