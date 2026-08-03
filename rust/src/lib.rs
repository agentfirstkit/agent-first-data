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
//!   [`Redactor::url`] (URL strings), [`redact_argv`] / [`Redactor::argv`] (command lines) —
//!   `Redactor` carries custom `secret_names`/`policy`
//! - Output rendering: [`render`] — the single `value × format × options → String` entry point
//!   for JSON, YAML, and plain (logfmt) output
//! - Parse utilities: [`normalize_utc_offset`], [`is_valid_rfc3339_date`],
//!   [`is_valid_rfc3339_time`], [`is_valid_rfc3339`], [`is_valid_bcp47`]
//! - Closed-world CLI compiler: [`CliSpec`], [`CommandSpec`], [`ArgSpec`],
//!   [`Combination`], and [`OutputSpec`] generate parsing, typed
//!   [`ResolvedInvocation`] values, output plans, and help-v2 from one registry.
//! - Established CLI utilities: [`cli_parse_output`], [`cli_parse_log_filters`]
//!   (returns [`LogFilters`]), and [`CliEmitter`].
//! - (feature `skill`): [`skill::validate_skill`] / [`skill::validate_skill_named`] — strict
//!   Agent Skills `SKILL.md` front-matter validation
//! - (feature `skill-admin`): [`skill::run_skill_admin`] — install/uninstall/status a spore's
//!   embedded Agent Skill across Codex, Claude Code, opencode, and Hermes; returns a typed
//!   [`skill::SkillReport`]
//! - (feature `tracing`): [`afdata_tracing::try_init`] initializes an AFDATA stderr logging
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
/// pluggable JSON/TOML/YAML/dotenv/INI backends, plus a read-only Markdown
/// block reader).
pub mod document;

// The closed-world CLI compiler: spec types, build gates, argv resolution, and
// the help-v2 model. Nothing else in this crate may reference it — only the
// adapter below does — which `cargo build --no-default-features` proves.
#[cfg(feature = "cli")]
mod cli_spec;

// The AFDATA CLI surface: output format parsing, the emitter, and version
// payloads.
#[cfg(feature = "cli")]
mod cli;

// The one place the compiler and AFDATA meet.
#[cfg(feature = "cli")]
mod cli_afdata;

mod formatting;
mod protocol;
mod redaction;
mod validation;

#[cfg(feature = "cli")]
pub use cli::{
    CliEmitter, CliEmitterError, LogFilters, OutputTo, build_cli_version, cli_parse_log_filters,
    cli_parse_output, cli_render_version, write_raw,
};
#[cfg(feature = "cli")]
pub use cli_afdata::{
    build_afdata_cli, cli_error_event, cli_help_event, cli_version_event, render_cli_reference,
};
#[cfg(feature = "cli")]
pub use cli_spec::{
    ArgSpec, ArgSyntax, ArgValueType, BoundCliSpec, BuiltCliSpec, CliError, CliErrorRule,
    CliHelpV2, CliOutcome, CliShape, CliSpec, CliSpecError, CliValue, Combination, CommandSpec,
    ExitCodeSpec, FixedValue, OutputLifecycle, OutputPlan, OutputSpec, ResolvedDocs, ResolvedHelp,
    ResolvedInvocation, ResolvedVersion, SyntheticInvocation,
};
pub use formatting::{OutputFormat, render};
pub use protocol::{
    BuildError, DecodedError, DecodedEvent, DecodedLog, DecodedProgress, DecodedResult,
    ErrorBuilder, Event, EventDecodeError, LogBuilder, LogLevel, ProgressBuilder,
    ProtocolViolation, ResultBuilder, build_cli_error, decode_protocol_event, json_error, json_log,
    json_progress, json_result, validate_protocol_event, validate_protocol_stream,
};
pub use redaction::{
    OutputOptions, PlainStyle, RedactionPolicy, Redactor, redact_argv, redact_url_secrets,
    redacted_value,
};
pub use validation::{
    is_valid_bcp47, is_valid_rfc3339, is_valid_rfc3339_date, is_valid_rfc3339_time,
    normalize_utc_offset,
};

#[cfg(test)]
pub(crate) use formatting::{extract_currency_code, format_bytes_human, format_with_commas};

#[cfg(test)]
mod tests;
