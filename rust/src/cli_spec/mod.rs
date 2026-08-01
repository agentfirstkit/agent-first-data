//! Closed-world CLI specifications and invocation resolution.
//!
//! A [`CliSpec`] is the single registry from which parsing, combination
//! matching, typed invocations, and agent-facing help are derived. Application
//! arguments that are not part of the selected [`Combination`] are rejected
//! before application code runs.
//!
//! # Boundary contract
//!
//! Everything under `cli_spec/` is pure computation: it decides whether an
//! invocation is legal and what it resolved to. It must not know how AFDATA
//! expresses that decision. Concretely, no file in this directory may:
//!
//! - reference `crate::` (protocol events, emitters, rendering, redaction);
//! - name `serde_json::Value` in a public type — a caller's number semantics
//!   depend on which `serde_json` features the final binary unifies, so raw
//!   JSON argument values stay unparsed source text here;
//! - perform process I/O, install file sinks, or write to stdout/stderr;
//! - hardcode an AFDATA naming convention such as the `_secret` suffix.
//!
//! `crate::cli_afdata` is the adapter that turns [`ResolvedHelp`],
//! [`ResolvedVersion`] and [`CliError`] into AFDATA events and owns the
//! `_secret` policy. `scripts/validate_cli_core_boundary.py` enforces the
//! rules above.

mod build;
mod error;
mod help;
mod resolve;
mod spec;
#[cfg(test)]
mod tests;

pub use error::{CliError, CliErrorRule};
pub(crate) use help::argument_key;
pub use help::{CliHelpV2, CliShape, ResolvedDocs, ResolvedHelp, ResolvedVersion};
pub use resolve::{
    BoundCliSpec, BuiltCliSpec, CliOutcome, OutputPlan, ResolvedInvocation, SyntheticInvocation,
};
pub use spec::{
    ArgSpec, ArgSyntax, ArgValueType, CliSpec, CliSpecError, CliValue, Combination, CommandSpec,
    FixedValue, OutputLifecycle, OutputSpec,
};

pub(crate) const RESERVED_ARGUMENTS: &[&str] = &[
    "--help",
    "--version",
    "--docs",
    "--output",
    "--output-to",
    "--stdout-file",
    "--stderr-file",
];

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_snake_case(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn is_canonical_long(value: &str) -> bool {
    let Some(name) = value.strip_prefix("--") else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}
