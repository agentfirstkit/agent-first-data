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
//!   [`Redactor::url`] (URL strings), [`redact_urls_in_text`] / [`Redactor::urls_in_text`]
//!   (explicit prose URL spans), [`redact_argv`] / [`Redactor::argv`] (command lines) —
//!   `Redactor` carries custom `secret_names`/`url_names`/`policy`
//! - Output rendering: [`render`] — the single `value × format × options → String` entry point
//!   for JSON, YAML, and plain (logfmt) output
//! - Parse utilities: [`normalize_utc_offset`], [`is_valid_rfc3339_date`],
//!   [`is_valid_rfc3339_time`], [`is_valid_rfc3339`], [`is_valid_bcp47`]
//! - Closed-world CLI compiler: [`CliSpec`], [`CommandSpec`], [`ArgSpec`],
//!   [`Combination`], and [`OutputSpec`] generate parsing, typed
//!   [`ResolvedInvocation`] values, output plans, and help-v2 from one registry.
//! - Established CLI utilities: [`cli_parse_output`], [`cli_parse_log_filters`]
//!   (returns [`LogFilters`]), [`CliEmitter`], and [`write_raw`].
//! - Domain errors and validation: [`ErrorSpec`] / [`ErrorCatalog`] declare
//!   stable public errors; [`lint_value`] and the `assert_*` helpers validate
//!   real serialized values in tests.
//! - Documents: [`document::Document`] provides source-preserving in-memory
//!   edits and typed [`document::Document::decode`]; [`document::DocumentFile`]
//!   adds capped reads, safe first creation, atomic commits, and
//!   [`document::DocumentFile::edit_and_validate`].
//! - (feature `skill`): [`skill::validate_skill`] / [`skill::validate_skill_named`] — strict
//!   Agent Skills `SKILL.md` front-matter validation
//! - (feature `skill-admin`): [`skill::run_skill_admin`] — install/uninstall/status a spore's
//!   embedded Agent Skill across Codex, Claude Code, opencode, and Hermes; returns a typed
//!   [`skill::SkillReport`]
//! - (feature `tracing`): [`afdata_tracing::AfdataLayer`] is a composable AFDATA
//!   logging layer with injectable writers and a nested-value
//!   [`afdata_tracing::StructuredLogHandle`]; [`afdata_tracing::try_init`] is
//!   the global-subscriber convenience entry point.
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

/// Reading a value that named where it is — an environment variable, an
/// address inside a config file, a stream, a terminal prompt — and the policy
/// that separates a printable value from a credential. The grammar itself is
/// [`cli_spec::SourceSet`]; what may be done with the result is carried by the
/// return type, [`value_source::SecretString`].
#[cfg(feature = "cli")]
pub mod value_source;

mod error_catalog;

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
mod lint;
mod output;
mod protocol;
mod redaction;
mod validation;

#[cfg(feature = "cli")]
pub use cli::{
    CliEmitter, CliEmitterError, LogFilters, build_cli_version, cli_parse_log_filters,
    cli_parse_output, cli_render_version, write_raw,
};
#[cfg(feature = "cli")]
pub use cli_afdata::{
    build_afdata_cli, cli_error_event, cli_help_event, cli_invocation_invalid_event,
    cli_version_event, render_cli_reference,
};
#[cfg(feature = "cli")]
pub use cli_spec::{
    ArgSpec, ArgSyntax, ArgValueType, BoundCliSpec, BoundInvocation, BoundOutcome, BuiltCliSpec,
    CliError, CliErrorRule, CliHelpV2, CliOutcome, CliShape, CliSpec, CliSpecError, CliValue,
    Combination, CommandSpec, ExitCodeSpec, FixedValue, HostScheme, OutputLifecycle, OutputPlan,
    OutputSpec, ResolvedDocs, ResolvedHelp, ResolvedInvocation, ResolvedVersion, SourceError,
    SourceScheme, SourceSet, SyntheticInvocation, ValueSource,
};
pub use error_catalog::{ErrorCatalog, ErrorCatalogError, ErrorSpec};
pub use formatting::render;
pub use lint::{
    LintFinding, LintOptions, LintSeverity, RedactionCanaryError, assert_no_lint_findings,
    assert_no_lint_findings_with_options, assert_redaction_canary_absent, assert_strict_event,
    lint_value,
};
pub use output::OutputFormat;
#[cfg(feature = "cli")]
pub use output::OutputTo;
pub use protocol::{
    BuildError, DecodedError, DecodedEvent, DecodedLog, DecodedProgress, DecodedResult,
    ErrorBuilder, Event, EventDecodeError, LogBuilder, LogLevel, ProgressBuilder,
    ProtocolViolation, ResultBuilder, build_cli_error, decode_protocol_event, json_error, json_log,
    json_progress, json_result, validate_protocol_event, validate_protocol_stream,
};
pub use redaction::{
    OutputOptions, PlainStyle, RedactionPolicy, Redactor, redact_argv, redact_url_secrets,
    redact_urls_in_text, redacted_value,
};
pub use validation::{
    is_valid_bcp47, is_valid_rfc3339, is_valid_rfc3339_date, is_valid_rfc3339_time,
    normalize_utc_offset,
};

#[cfg(test)]
pub(crate) use formatting::{extract_currency_code, format_bytes_human, format_with_commas};

#[cfg(test)]
mod tests;
