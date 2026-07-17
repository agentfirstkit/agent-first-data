#![allow(clippy::print_stdout, clippy::print_stderr)]

use agent_first_data::document::{
    Document, DocumentFile, Format as DocumentFormat, Value as DocumentValue, coerce_scalar,
    coerce_values_typed, get_path, parse_path,
};
use agent_first_data::{
    ErrorBuilder, Event, OutputFormat, OutputOptions, PlainStyle, Redactor, build_cli_error,
    cli_parse_output, is_valid_bcp47, is_valid_rfc3339, is_valid_rfc3339_date,
    is_valid_rfc3339_time, json_error, json_result, normalize_utc_offset, render,
    validate_protocol_event, validate_protocol_stream,
};
#[cfg(feature = "cli-help")]
use clap::CommandFactory;
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Parser)]
#[command(
    name = "afdata",
    version,
    about = "Validate, lint, and render Agent-First Data JSON.",
    after_help = concat!("AFDATA: ", env!("CARGO_PKG_VERSION")),
    disable_help_subcommand = true
)]
struct Cli {
    /// Output format: json, yaml, or plain (help also accepts markdown)
    #[arg(long, global = true, default_value = "json")]
    output: String,

    /// Redirect stdout to this file
    #[cfg(feature = "stream-redirect")]
    #[arg(long, value_name = "PATH", global = true)]
    stdout_file: Option<std::path::PathBuf>,

    /// Redirect stderr to this file
    #[cfg(feature = "stream-redirect")]
    #[arg(long, value_name = "PATH", global = true)]
    stderr_file: Option<std::path::PathBuf>,

    /// Document format override for `show`/`get`/`value`/`set`/`add`/`remove`/`unset`:
    /// json, toml, yaml, yml, dotenv, env, or ini.
    ///
    /// Overrides file-extension detection for a FILE/--input-file argument, and
    /// overrides the JSON default when a read command falls back to stdin.
    #[arg(long = "input-format", value_name = "FORMAT", global = true)]
    input_format: Option<String>,

    /// Extra field name to redact (beyond the `_secret` suffix convention) in
    /// `show`/`get`/`value` output. Repeatable.
    #[arg(long = "secret-name", value_name = "FIELD", global = true)]
    secret_names: Vec<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Lint ordinary JSON, JSONL, or JSON Schema for deterministic AFDATA issues
    Lint {
        /// Input file; stdin is used when omitted
        input: Option<PathBuf>,
    },
    /// Validate one protocol event or a finite protocol event stream
    Validate {
        /// Input file; stdin is used when omitted
        input: Option<PathBuf>,
        /// Enforce the recommended strict protocol profile
        #[arg(long)]
        strict: bool,
        /// Validate each input value as an independent event, without stream lifecycle rules
        #[arg(long)]
        event: bool,
    },
    /// Render JSON or JSONL through AFDATA output formatting and redaction
    Render {
        /// Input file; stdin is used when omitted
        input: Option<PathBuf>,
    },
    /// Validate an Agent Skill or manage the bundled Agent Skill
    #[cfg(feature = "skill")]
    Skill {
        /// Action: validate, status, install, or uninstall
        action: String,
        /// SKILL.md file or skill directory for the validate action; stdin when omitted
        input: Option<PathBuf>,
        /// Agent target: all, codex, claude-code, opencode, or hermes
        #[arg(long, default_value = "all")]
        agent: String,
        /// Skill scope: personal or workspace
        #[arg(long, default_value = "personal")]
        scope: String,
        /// Explicit skills directory; requires a single concrete --agent
        #[arg(long)]
        skills_dir: Option<String>,
        /// Overwrite or remove a skill this tool did not manage
        #[arg(long)]
        force: bool,
    },
    /// Show a document as a full AFDATA record
    ///
    /// Reads FILE (or stdin when omitted) and emits
    /// `{"code":"document","format":...,"value":...}`. `_secret`-suffixed
    /// fields (and any `--secret-name`) are redacted to `"***"` anywhere in
    /// the document.
    Show {
        /// Document file path; stdin is used when omitted
        file: Option<PathBuf>,
    },
    /// Get the value at a dot-path as an AFDATA record
    ///
    /// Emits `{"code":"document_value","format":...,"key":...,"value":...}`.
    /// If `KEY`'s leaf field name is a secret (the `_secret` suffix
    /// convention or `--secret-name`), the value is redacted to `"***"` even
    /// though it was explicitly targeted — use `value --reveal-secret` to
    /// read a secret's real value.
    Get {
        /// Dot-separated key path (`\.` escapes a literal dot, `\\` a backslash)
        key: String,
        /// Document file path; stdin is used when omitted
        file: Option<PathBuf>,
    },
    /// Get the value at a dot-path as raw scalar bytes on stdout, with no AFDATA envelope
    ///
    /// Only scalars (string/bool/integer/float/null) are supported; arrays
    /// and objects are rejected, as are non-finite floats. A secret-named
    /// leaf is rejected unless `--reveal-secret` is passed.
    #[command(name = "value")]
    ValueGet {
        /// Dot-separated key path
        key: String,
        /// Document file path; stdin is used when omitted
        file: Option<PathBuf>,
        /// Print a secret-named scalar instead of erroring
        #[arg(long = "reveal-secret")]
        reveal_secret: bool,
    },
    /// Set a scalar value at a dot-path, preserving the document's source formatting
    Set {
        /// Dot-separated key path
        key: String,
        /// Value(s) to set (multiple arguments become an array)
        #[arg(conflicts_with_all = ["value_secret", "value_secret_stdin", "value_secret_prompt", "value_secret_fd"])]
        values: Vec<String>,
        /// Secret scalar value (visible to process observers such as `ps`)
        #[arg(long = "value-secret", conflicts_with_all = ["value_secret_stdin", "value_secret_prompt", "value_secret_fd"])]
        value_secret: Option<String>,
        /// Read one secret scalar from stdin to EOF
        #[arg(long = "value-secret-stdin", conflicts_with_all = ["value_secret", "value_secret_prompt", "value_secret_fd"])]
        value_secret_stdin: bool,
        /// Read one secret scalar from the controlling terminal
        #[arg(long = "value-secret-prompt", conflicts_with_all = ["value_secret", "value_secret_stdin", "value_secret_fd"])]
        value_secret_prompt: bool,
        /// Read one secret scalar from an inherited Unix file descriptor
        #[arg(long = "value-secret-fd", value_name = "FD", conflicts_with_all = ["value_secret", "value_secret_stdin", "value_secret_prompt"])]
        value_secret_fd: Option<String>,
        /// Document file to mutate in place (required; mutation never reads stdin)
        #[arg(long = "input-file", value_name = "PATH", required = true)]
        input_file: PathBuf,
    },
    /// Add an element to a keyed list (an array of objects addressed by a slug field)
    Add {
        /// Dot-path to the keyed list
        key: String,
        /// Slug/ID for the new element
        slug: String,
        /// Field name that identifies each element (the slug field)
        #[arg(long = "slug-field")]
        slug_field: String,
        /// Additional `FIELD=VALUE` pairs to set on the new element
        #[arg(value_name = "FIELD=VALUE")]
        fields: Vec<String>,
        /// Document file to mutate in place (required; mutation never reads stdin)
        #[arg(long = "input-file", value_name = "PATH", required = true)]
        input_file: PathBuf,
    },
    /// Remove an element from a keyed list by slug
    Remove {
        /// Dot-path to the keyed list
        key: String,
        /// Slug/ID of the element to remove
        slug: String,
        /// Field name that identifies each element (the slug field)
        #[arg(long = "slug-field")]
        slug_field: String,
        /// Document file to mutate in place (required; mutation never reads stdin)
        #[arg(long = "input-file", value_name = "PATH", required = true)]
        input_file: PathBuf,
    },
    /// Remove one entry from a document entirely
    Unset {
        /// Dot-path to the entry to remove
        key: String,
        /// Document file to mutate in place (required; mutation never reads stdin)
        #[arg(long = "input-file", value_name = "PATH", required = true)]
        input_file: PathBuf,
    },
}

#[derive(Clone, Debug)]
struct Finding {
    rule_id: &'static str,
    severity: &'static str,
    pointer: String,
    message: String,
}

impl Finding {
    fn error(rule_id: &'static str, pointer: String, message: String) -> Self {
        Self {
            rule_id,
            severity: "error",
            pointer,
            message,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "rule_id": self.rule_id,
            "severity": self.severity,
            "pointer": self.pointer,
            "message": self.message,
        })
    }
}

enum ParsedInput {
    Single(Value),
    Lines(Vec<Value>),
}

struct ParseError {
    code: &'static str,
    message: String,
    hint: Option<String>,
    line: Option<usize>,
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().collect();

    // Redirect stdout/stderr before any output, per --stdout-file/--stderr-file.
    #[cfg(feature = "stream-redirect")]
    let _stream_redirect =
        match agent_first_data::stream_redirect::install_from_raw_args(raw.clone()) {
            Ok(installed) => installed,
            Err(err) => {
                let event = build_cli_error(&err.to_string(), None);
                return emit_event(event, OutputFormat::Json, 2);
            }
        };

    // Handle --version through AFDATA so `--version --output json` works too.
    match agent_first_data::cli_handle_version_or_continue(
        &raw,
        "afdata",
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(Some(version)) => return write_text_exit(&version, 0),
        Ok(None) => {}
        Err(err) => return emit_event(err, OutputFormat::Json, 2),
    }

    // Handle --help before clap so `--help --output markdown` works.
    #[cfg(feature = "cli-help")]
    match agent_first_data::cli_handle_help_or_continue(
        &raw,
        &Cli::command(),
        &agent_first_data::HelpConfig::human_cli_default(),
    ) {
        Ok(Some(help)) => return write_text_exit(&help, 0),
        Ok(None) => {}
        Err(err) => return emit_event(err, OutputFormat::Json, 2),
    }

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                return write_text_exit(&err.render().to_string(), 0);
            }
            let event = build_cli_error(&err.to_string(), Some("try: afdata --help"));
            return emit_event(event, OutputFormat::Json, 2);
        }
    };

    // Redirection is installed from raw args above; these fields exist for
    // clap's --help listing only.
    #[cfg(feature = "stream-redirect")]
    let _ = (&cli.stdout_file, &cli.stderr_file);

    let format = match cli_parse_output(&cli.output) {
        Ok(format) => format,
        Err(message) => {
            let event = build_cli_error(&message, Some("valid values: json, yaml, plain"));
            return emit_event(event, OutputFormat::Json, 2);
        }
    };

    let document_ctx = DocumentContext {
        input_format: cli.input_format.as_deref(),
        secret_names: &cli.secret_names,
        format,
    };

    match cli.command {
        Command::Lint { input } => run_lint(input.as_deref(), format),
        Command::Validate {
            input,
            strict,
            event,
        } => run_validate(input.as_deref(), format, strict, event),
        Command::Render { input } => run_render(input.as_deref(), format),
        #[cfg(feature = "skill")]
        Command::Skill {
            action,
            input,
            agent,
            scope,
            skills_dir,
            force,
        } => run_skill(
            &action,
            input.as_deref(),
            &agent,
            &scope,
            skills_dir,
            force,
            format,
        ),
        Command::Show { file } => run_show(file.as_deref(), &document_ctx),
        Command::Get { key, file } => run_get(&key, file.as_deref(), &document_ctx),
        Command::ValueGet {
            key,
            file,
            reveal_secret,
        } => run_value_get(&key, file.as_deref(), reveal_secret, &document_ctx),
        Command::Set {
            key,
            values,
            value_secret,
            value_secret_stdin,
            value_secret_prompt,
            value_secret_fd,
            input_file,
        } => run_set(
            &key,
            SetValueArgs {
                values,
                value_secret,
                value_secret_stdin,
                value_secret_prompt,
                value_secret_fd,
            },
            &input_file,
            &document_ctx,
        ),
        Command::Add {
            key,
            slug,
            slug_field,
            fields,
            input_file,
        } => run_add(
            &key,
            &slug,
            &slug_field,
            &fields,
            &input_file,
            &document_ctx,
        ),
        Command::Remove {
            key,
            slug,
            slug_field,
            input_file,
        } => run_remove(&key, &slug, &slug_field, &input_file, &document_ctx),
        Command::Unset { key, input_file } => run_unset(&key, &input_file, &document_ctx),
    }
}

fn run_lint(input: Option<&Path>, format: OutputFormat) -> ExitCode {
    let text = match read_input(input) {
        Ok(text) => text,
        Err(message) => {
            let event = build_error_event(json_error("read_failed", &message));
            return emit_event(event, format, 1);
        }
    };
    let parsed = match parse_json_or_jsonl(&text) {
        Ok(parsed) => parsed,
        Err(err) => return emit_parse_error(err, format),
    };
    let mut findings = Vec::new();
    match parsed {
        ParsedInput::Single(value) => lint_value(&value, "", &mut findings),
        ParsedInput::Lines(values) => {
            for (idx, value) in values.iter().enumerate() {
                lint_value(value, &format!("/{}", idx + 1), &mut findings);
            }
        }
    }
    emit_findings("lint_failed", "lint failed", findings, format)
}

fn run_validate(
    input: Option<&Path>,
    format: OutputFormat,
    strict: bool,
    event_mode: bool,
) -> ExitCode {
    let text = match read_input(input) {
        Ok(text) => text,
        Err(message) => {
            let event = build_error_event(json_error("read_failed", &message));
            return emit_event(event, format, 1);
        }
    };
    let parsed = match parse_json_or_jsonl(&text) {
        Ok(parsed) => parsed,
        Err(err) => return emit_parse_error(err, format),
    };
    let mut findings = Vec::new();
    if event_mode {
        match parsed {
            ParsedInput::Single(Value::Array(events)) | ParsedInput::Lines(events) => {
                for (idx, event) in events.iter().enumerate() {
                    validate_one_event(event, strict, &format!("/{idx}"), &mut findings);
                }
            }
            ParsedInput::Single(value) => validate_one_event(&value, strict, "", &mut findings),
        }
        return emit_findings("validation_failed", "validation failed", findings, format);
    }
    match parsed {
        ParsedInput::Single(Value::Array(events)) => {
            if let Err(vs) = validate_protocol_stream(&events, strict) {
                for v in vs {
                    findings.push(Finding::error(v.rule, v.pointer, v.message));
                }
            }
        }
        ParsedInput::Single(value) => validate_single_input(value, strict, &mut findings),
        ParsedInput::Lines(values) => {
            if let Err(vs) = validate_protocol_stream(&values, strict) {
                for v in vs {
                    findings.push(Finding::error(v.rule, v.pointer, v.message));
                }
            }
        }
    }
    emit_findings("validation_failed", "validation failed", findings, format)
}

fn validate_single_input(value: Value, strict: bool, findings: &mut Vec<Finding>) {
    let kind = value.get("kind").and_then(Value::as_str);
    if matches!(kind, Some("log" | "progress")) {
        if let Err(vs) = validate_protocol_stream(&[value], strict) {
            for v in vs {
                findings.push(Finding::error(v.rule, v.pointer, v.message));
            }
        }
        return;
    }
    validate_one_event(&value, strict, "", findings);
}

fn validate_one_event(value: &Value, strict: bool, pointer: &str, findings: &mut Vec<Finding>) {
    if let Err(v) = validate_protocol_event(value, strict) {
        findings.push(Finding::error(
            v.rule,
            format!("{pointer}{}", v.pointer),
            v.message,
        ));
    }
}

fn run_render(input: Option<&Path>, format: OutputFormat) -> ExitCode {
    let text = match read_input(input) {
        Ok(text) => text,
        Err(message) => {
            let event = build_error_event(json_error("read_failed", &message));
            return emit_event(event, format, 1);
        }
    };
    let parsed = match parse_json_or_jsonl(&text) {
        Ok(parsed) => parsed,
        Err(err) => return emit_parse_error(err, OutputFormat::Json),
    };
    match parsed {
        ParsedInput::Single(value) => write_text_exit(&format_value(&value, format, false), 0),
        ParsedInput::Lines(values) => {
            let mut out = String::new();
            for (idx, value) in values.iter().enumerate() {
                if idx > 0 && is_yaml_format(format) {
                    out.push_str("---\n");
                }
                out.push_str(&format_value(value, format, idx > 0));
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            write_text_exit(&out, 0)
        }
    }
}

#[cfg(feature = "skill")]
fn run_skill(
    action: &str,
    input: Option<&Path>,
    agent: &str,
    scope: &str,
    skills_dir: Option<String>,
    force: bool,
    format: OutputFormat,
) -> ExitCode {
    if action == "validate" {
        return run_skill_validate(input, format);
    }

    #[cfg(not(feature = "skill-admin"))]
    {
        let _ = (agent, scope, skills_dir, force);
        let event = build_cli_error(
            &format!("invalid skill action '{action}'"),
            Some("valid action: validate; status/install/uninstall require feature skill-admin"),
        );
        return emit_event(event, format, 2);
    }

    #[cfg(feature = "skill-admin")]
    {
        if input.is_some() {
            let event = build_cli_error(
                "a SKILL.md path is only accepted by 'skill validate'",
                Some("remove the path argument for status, install, or uninstall"),
            );
            return emit_event(event, format, 2);
        }
        run_skill_admin_action(action, agent, scope, skills_dir, force, format)
    }
}

#[cfg(feature = "skill")]
fn run_skill_validate(input: Option<&Path>, format: OutputFormat) -> ExitCode {
    let (text, expected_name, display_path) = match read_skill_input(input) {
        Ok(value) => value,
        Err(message) => {
            let event = build_error_event(json_error("read_failed", &message));
            return emit_event(event, format, 1);
        }
    };
    let validation = match expected_name.as_deref() {
        Some(name) => agent_first_data::skill::validate_skill_named(&text, name),
        None => agent_first_data::skill::validate_skill(&text),
    };
    let metadata = match validation {
        Ok(metadata) => metadata,
        Err(error) => {
            let event = build_error_event(
                json_error("skill_invalid", error.message())
                    .hint("make SKILL.md front matter conform to the Agent Skills specification"),
            );
            return emit_event(event, format, 1);
        }
    };
    let event = json_result(json!({
        "code": "skill_valid",
        "path": display_path,
        "name": metadata.name,
        "description": metadata.description,
        "license": metadata.license,
        "compatibility": metadata.compatibility,
        "metadata": metadata.metadata,
        "allowed_tools": metadata.allowed_tools,
        "disable_model_invocation": metadata.disable_model_invocation,
        "user_invocable": metadata.user_invocable,
    }))
    .build();
    emit_event(event, format, 0)
}

#[cfg(feature = "skill")]
fn read_skill_input(input: Option<&Path>) -> Result<(String, Option<String>, String), String> {
    let Some(input) = input else {
        return read_input(None).map(|text| (text, None, "<stdin>".to_string()));
    };
    let input_metadata = std::fs::symlink_metadata(input)
        .map_err(|error| format!("failed to inspect {}: {error}", input.display()))?;
    if input_metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing to validate symlinked skill input at {}",
            input.display()
        ));
    }

    let (skill_path, expected_name) = if input_metadata.is_dir() {
        let name = path_file_name(input)?;
        (input.join("SKILL.md"), Some(name))
    } else if input_metadata.is_file() {
        let expected_name = if input.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
        {
            input.parent().map(path_file_name).transpose()?
        } else {
            None
        };
        (input.to_path_buf(), expected_name)
    } else {
        return Err(format!(
            "skill input is not a regular file or directory: {}",
            input.display()
        ));
    };

    let skill_metadata = std::fs::symlink_metadata(&skill_path)
        .map_err(|error| format!("failed to inspect {}: {error}", skill_path.display()))?;
    if skill_metadata.file_type().is_symlink() || !skill_metadata.is_file() {
        return Err(format!(
            "skill document is not a regular file: {}",
            skill_path.display()
        ));
    }
    let text = std::fs::read_to_string(&skill_path)
        .map_err(|error| format!("failed to read {}: {error}", skill_path.display()))?;
    Ok((text, expected_name, skill_path.display().to_string()))
}

#[cfg(feature = "skill")]
fn path_file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("path has no UTF-8 directory name: {}", path.display()))
}

#[cfg(feature = "skill-admin")]
fn run_skill_admin_action(
    action: &str,
    agent: &str,
    scope: &str,
    skills_dir: Option<String>,
    force: bool,
    format: OutputFormat,
) -> ExitCode {
    use agent_first_data::skill::{SkillOptions, SkillSpec, run_skill_admin};

    let action = match parse_skill_action(action) {
        Ok(action) => action,
        Err(message) => {
            let event =
                build_cli_error(&message, Some("valid actions: status, install, uninstall"));
            return emit_event(event, format, 2);
        }
    };
    let agent = match parse_skill_agent(agent) {
        Ok(agent) => agent,
        Err(message) => {
            let event = build_cli_error(
                &message,
                Some("valid agents: all, codex, claude-code, opencode, hermes"),
            );
            return emit_event(event, format, 2);
        }
    };
    let scope = match parse_skill_scope(scope) {
        Ok(scope) => scope,
        Err(message) => {
            let event = build_cli_error(&message, Some("valid scopes: personal, workspace"));
            return emit_event(event, format, 2);
        }
    };

    const SKILL_SOURCE: &str = include_str!("../../skills/agent-first-data/SKILL.md");
    let spec = SkillSpec {
        name: "agent-first-data",
        source: SKILL_SOURCE,
        title: "Agent-First Data",
        marker_slug: "afdata",
    };
    let options = SkillOptions {
        agent,
        scope,
        skills_dir,
        force,
    };
    match run_skill_admin(&spec, action, &options) {
        Ok(report) => match serde_json::to_value(report) {
            Ok(value) => {
                let event = json_result(value).build();
                emit_event(event, format, 0)
            }
            Err(err) => {
                let event = build_error_event(json_error(
                    "serialization_failed",
                    &format!("failed to serialize skill report: {err}"),
                ));
                emit_event(event, format, 1)
            }
        },
        Err(err) => {
            let mut builder = json_error("cli_error", &err.message);
            if let Some(hint) = err.hint.as_deref() {
                builder = builder.hint(hint);
            }
            if let Some(report) = err.partial_report.as_ref()
                && let Ok(partial_report) = serde_json::to_value(report)
            {
                builder = builder.field("partial_report", partial_report);
            }
            let event = build_error_event(builder);
            emit_event(event, format, 2)
        }
    }
}

#[cfg(feature = "skill-admin")]
fn parse_skill_action(value: &str) -> Result<agent_first_data::skill::SkillAction, String> {
    match value {
        "status" => Ok(agent_first_data::skill::SkillAction::Status),
        "install" => Ok(agent_first_data::skill::SkillAction::Install),
        "uninstall" => Ok(agent_first_data::skill::SkillAction::Uninstall),
        other => Err(format!("invalid skill action '{other}'")),
    }
}

#[cfg(feature = "skill-admin")]
fn parse_skill_agent(value: &str) -> Result<agent_first_data::skill::SkillAgentSelection, String> {
    match value {
        "all" => Ok(agent_first_data::skill::SkillAgentSelection::All),
        "codex" => Ok(agent_first_data::skill::SkillAgentSelection::Codex),
        "claude-code" => Ok(agent_first_data::skill::SkillAgentSelection::ClaudeCode),
        "opencode" => Ok(agent_first_data::skill::SkillAgentSelection::Opencode),
        "hermes" => Ok(agent_first_data::skill::SkillAgentSelection::Hermes),
        other => Err(format!("invalid --agent '{other}'")),
    }
}

#[cfg(feature = "skill-admin")]
fn parse_skill_scope(value: &str) -> Result<agent_first_data::skill::SkillScope, String> {
    match value {
        "personal" => Ok(agent_first_data::skill::SkillScope::Personal),
        "workspace" => Ok(agent_first_data::skill::SkillScope::Workspace),
        other => Err(format!("invalid --scope '{other}'")),
    }
}

fn format_value(value: &Value, format: OutputFormat, suppress_yaml_boundary: bool) -> String {
    let mut out = render(value, format, &OutputOptions::default());
    if suppress_yaml_boundary
        && is_yaml_format(format)
        && let Some(stripped) = out.strip_prefix("---\n")
    {
        out = stripped.to_string();
    }
    out
}

/// Whether `format` is [`OutputFormat::Yaml`].
fn is_yaml_format(format: OutputFormat) -> bool {
    format == OutputFormat::Yaml
}

fn read_input(input: Option<&Path>) -> Result<String, String> {
    let mut text = String::new();
    match input {
        Some(path) => {
            text = std::fs::read_to_string(path)
                .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        }
        None => {
            // Never block on an interactive terminal: with no input file and no
            // piped stdin, fail fast instead of hanging until Ctrl-D/Ctrl-C.
            if std::io::stdin().is_terminal() {
                return Err("no input: pass a file path or pipe stdin".to_string());
            }
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(|err| format!("failed to read stdin: {err}"))?;
        }
    }
    Ok(text)
}

fn parse_json_or_jsonl(text: &str) -> Result<ParsedInput, ParseError> {
    if text.trim().is_empty() {
        return Err(ParseError {
            code: "json_parse_failed",
            message: "input is empty".to_string(),
            hint: Some("provide a JSON value or JSONL stream".to_string()),
            line: None,
        });
    }
    match serde_json::from_str::<Value>(text) {
        Ok(value) => Ok(ParsedInput::Single(value)),
        Err(whole_error) => {
            let mut values = Vec::new();
            for (idx, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(line) {
                    Ok(value) => values.push(value),
                    Err(line_error) => {
                        return Err(ParseError {
                            code: "jsonl_parse_failed",
                            message: format!("line {} is not valid JSON: {line_error}", idx + 1),
                            hint: Some(format!("complete JSON parse failed first: {whole_error}")),
                            line: Some(idx + 1),
                        });
                    }
                }
            }
            if values.is_empty() {
                Err(ParseError {
                    code: "json_parse_failed",
                    message: whole_error.to_string(),
                    hint: Some("provide a JSON value or JSONL stream".to_string()),
                    line: None,
                })
            } else {
                Ok(ParsedInput::Lines(values))
            }
        }
    }
}

/// Build an error event without an `expect`. The error builders here always use
/// non-empty literal codes/messages and non-reserved fields, so `build()` cannot
/// actually fail; on the impossible error we fall back to `build_cli_error` so the
/// function stays total and panic-free.
fn build_error_event(builder: ErrorBuilder) -> Event {
    match builder.build() {
        Ok(event) => event,
        Err(err) => build_cli_error(&err.to_string(), None),
    }
}

fn emit_parse_error(err: ParseError, format: OutputFormat) -> ExitCode {
    let mut fields = serde_json::Map::new();
    if let Some(line) = err.line {
        fields.insert("line".to_string(), json!(line));
    }
    let mut builder = json_error(err.code, &err.message);
    if let Some(hint) = err.hint.as_deref() {
        builder = builder.hint(hint);
    }
    builder = builder.fields(Value::Object(fields));
    let event = build_error_event(builder);
    emit_event(event, format, 1)
}

fn emit_findings(
    error_code: &'static str,
    error_message: &'static str,
    findings: Vec<Finding>,
    format: OutputFormat,
) -> ExitCode {
    let findings_json = Value::Array(findings.iter().map(Finding::to_json).collect());
    if findings.is_empty() {
        let event = json_result(json!({"ok": true, "findings": findings_json})).build();
        emit_event(event, format, 0)
    } else {
        let event = build_error_event(
            json_error(error_code, error_message).fields(json!({"findings": findings_json})),
        );
        emit_event(event, format, 1)
    }
}

fn emit_event(event: impl Into<Value>, format: OutputFormat, code: u8) -> ExitCode {
    emit_event_with_options(event, format, &OutputOptions::default(), code)
}

/// As [`emit_event`], but rendering through `output_options` (redaction and
/// style) instead of the crate defaults. Used by the document commands so
/// `--secret-name` reaches the final render, the same way [`emit_event`]'s
/// call is equivalent to passing [`OutputOptions::default`] here.
fn emit_event_with_options(
    event: impl Into<Value>,
    format: OutputFormat,
    output_options: &OutputOptions,
    code: u8,
) -> ExitCode {
    let mut event: Value = event.into();
    if event.get("trace").is_none()
        && let Some(object) = event.as_object_mut()
    {
        object.insert("trace".to_string(), json!({}));
    }
    if let Err(violation) = validate_protocol_event(&event, true) {
        let fallback = build_error_event(
            json_error(
                "internal_protocol_error",
                "afdata attempted to emit an invalid protocol event",
            )
            .field("validation_message", json!(violation.to_string())),
        );
        let mut text = render(
            fallback.as_value(),
            OutputFormat::Json,
            &OutputOptions::default(),
        );
        if !text.ends_with('\n') {
            text.push('\n');
        }
        return write_text_exit(&text, 1);
    }
    let mut text = render(&event, format, output_options);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    write_text_exit(&text, code)
}

fn write_text_exit(text: &str, code: u8) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    if let Err(err) = stdout.write_all(text.as_bytes()) {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            return ExitCode::from(0);
        }
        return ExitCode::from(1);
    }
    ExitCode::from(code)
}

// ═══════════════════════════════════════════
// Document read/edit commands
// ═══════════════════════════════════════════

/// Shared per-invocation context threaded through the document read/edit
/// commands: the `--input-format` override, the `--secret-name` redaction
/// list, and the negotiated `--output` rendering format.
struct DocumentContext<'a> {
    input_format: Option<&'a str>,
    secret_names: &'a [String],
    format: OutputFormat,
}

/// `show`: read a document and emit it whole as an AFDATA record.
fn run_show(file: Option<&Path>, ctx: &DocumentContext<'_>) -> ExitCode {
    let input_format = match resolve_input_format(ctx.input_format) {
        Ok(format) => format,
        Err(message) => return emit_document_error(&message, ctx),
    };
    let (value, doc_format) = match read_document_input(file, input_format) {
        Ok(pair) => pair,
        Err(message) => return emit_document_error(&message, ctx),
    };
    let json_value: Value = value.into();
    let event = json_result(json!({
        "code": "document",
        "format": document_format_name(doc_format),
        "value": json_value,
    }))
    .build();
    emit_document_event(event, ctx, 0)
}

/// `get`: read a document and emit the value at dot-path `key`.
fn run_get(key: &str, file: Option<&Path>, ctx: &DocumentContext<'_>) -> ExitCode {
    let input_format = match resolve_input_format(ctx.input_format) {
        Ok(format) => format,
        Err(message) => return emit_document_error(&message, ctx),
    };
    let (value, doc_format) = match read_document_input(file, input_format) {
        Ok(pair) => pair,
        Err(message) => return emit_document_error(&message, ctx),
    };
    let target = match get_path(&value, key, &[]) {
        Ok(target) => target,
        Err(err) => return emit_document_error(&err.to_string(), ctx),
    };
    let is_secret = match document_leaf_is_secret(key, ctx.secret_names) {
        Ok(is_secret) => is_secret,
        Err(message) => return emit_document_error(&message, ctx),
    };
    // A generic whole-document redact walk (applied below via `--secret-name`
    // options) only rewrites object fields it finds by name; the value here
    // sits under the generic `"value"` wrapper key instead of its own field
    // name, so a directly-targeted secret leaf needs this explicit check.
    let json_value: Value = if is_secret {
        json!("***")
    } else {
        target.into()
    };
    let event = json_result(json!({
        "code": "document_value",
        "format": document_format_name(doc_format),
        "key": key,
        "value": json_value,
    }))
    .build();
    emit_document_event(event, ctx, 0)
}

/// `value`: like `get`, but writes only the scalar's raw bytes to stdout —
/// no AFDATA envelope, no forced trailing newline. Arrays, objects, and
/// non-finite floats are rejected. A secret-named leaf is rejected unless
/// `--reveal-secret` is passed (never bypassed by default).
fn run_value_get(
    key: &str,
    file: Option<&Path>,
    reveal_secret: bool,
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    let input_format = match resolve_input_format(ctx.input_format) {
        Ok(format) => format,
        Err(message) => return emit_document_error(&message, ctx),
    };
    let (value, _doc_format) = match read_document_input(file, input_format) {
        Ok(pair) => pair,
        Err(message) => return emit_document_error(&message, ctx),
    };
    if !reveal_secret {
        match document_leaf_is_secret(key, ctx.secret_names) {
            Ok(true) => {
                return emit_document_error(
                    &format!("path `{key}` names a secret; pass --reveal-secret"),
                    ctx,
                );
            }
            Ok(false) => {}
            Err(message) => return emit_document_error(&message, ctx),
        }
    }
    let target = match get_path(&value, key, &[]) {
        Ok(target) => target,
        Err(err) => return emit_document_error(&err.to_string(), ctx),
    };
    match document_scalar_bytes(&target, key) {
        Ok(bytes) => write_raw_exit(&bytes),
        Err(message) => emit_document_error(&message, ctx),
    }
}

/// Value source for `set`: either the positional VALUES (multiple arguments
/// become an array) or exactly one `--value-secret*` source (mutually
/// exclusive with VALUES and with each other; enforced by clap's
/// `conflicts_with_all` on the `Command::Set` fields).
struct SetValueArgs {
    values: Vec<String>,
    value_secret: Option<String>,
    value_secret_stdin: bool,
    value_secret_prompt: bool,
    value_secret_fd: Option<String>,
}

/// `set`: write a scalar at dot-path `key` into `--input-file`, coercing
/// toward the type already stored there, preserving the rest of the
/// document's source formatting (comments, key order, unrelated values).
fn run_set(
    key: &str,
    args: SetValueArgs,
    input_file: &Path,
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    if let Err(message) = reject_input_file_dash(input_file) {
        return emit_document_error(&message, ctx);
    }
    let input_format = match resolve_input_format(ctx.input_format) {
        Ok(format) => format,
        Err(message) => return emit_document_error(&message, ctx),
    };
    let mut doc = match DocumentFile::open(input_file, input_format) {
        Ok(doc) => doc,
        Err(err) => return emit_document_error(&err.to_string(), ctx),
    };
    // Guard the target before consuming a `--value-secret-stdin`/`-prompt`/`-fd`
    // source: an unsafe target (symlink, or on unix a hardlink) must be
    // rejected before the secret is read, not after.
    if let Err(err) = doc.ensure_mutable("set") {
        return emit_document_error(&err.to_string(), ctx);
    }
    let values = match secret_or_values(args) {
        Ok(values) => values,
        Err(message) => return emit_document_error(&message, ctx),
    };
    let existing = get_path(doc.value(), key, &[]).ok();
    let new_value = match coerce_values_typed(&values, existing.as_ref()) {
        Ok(value) => value,
        Err(err) => return emit_document_error(&err.to_string(), ctx),
    };
    if let Err(err) = doc.set(key, new_value) {
        return emit_document_error(&err.to_string(), ctx);
    }
    let event = json_result(json!({
        "code": "document_set",
        "format": document_format_name(doc.format()),
        "key": key,
        "write_count": values.len(),
    }))
    .build();
    emit_document_event(event, ctx, 0)
}

/// `add`: append an element to the keyed list at dot-path `key` in
/// `--input-file`, preserving the rest of the document's source formatting.
fn run_add(
    key: &str,
    slug: &str,
    slug_field: &str,
    fields: &[String],
    input_file: &Path,
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    if let Err(message) = reject_input_file_dash(input_file) {
        return emit_document_error(&message, ctx);
    }
    let input_format = match resolve_input_format(ctx.input_format) {
        Ok(format) => format,
        Err(message) => return emit_document_error(&message, ctx),
    };
    let mut field_pairs: Vec<(String, DocumentValue)> = Vec::with_capacity(fields.len());
    for field in fields {
        let Some((name, value)) = field.split_once('=') else {
            return emit_document_error(&format!("field `{field}` must use FIELD=VALUE"), ctx);
        };
        if name.is_empty() {
            return emit_document_error("field name must not be empty", ctx);
        }
        field_pairs.push((name.to_string(), coerce_scalar(value)));
    }
    let mut doc = match DocumentFile::open(input_file, input_format) {
        Ok(doc) => doc,
        Err(err) => return emit_document_error(&err.to_string(), ctx),
    };
    if let Err(err) = doc.add(key, slug, slug_field, &field_pairs) {
        return emit_document_error(&err.to_string(), ctx);
    }
    let event = json_result(json!({
        "code": "document_added",
        "format": document_format_name(doc.format()),
        "key": key,
        "slug": slug,
    }))
    .build();
    emit_document_event(event, ctx, 0)
}

/// `remove`: delete the element identified by `slug`/`slug_field` from the
/// keyed list at dot-path `key` in `--input-file`, preserving the rest of
/// the document's source formatting.
fn run_remove(
    key: &str,
    slug: &str,
    slug_field: &str,
    input_file: &Path,
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    if let Err(message) = reject_input_file_dash(input_file) {
        return emit_document_error(&message, ctx);
    }
    let input_format = match resolve_input_format(ctx.input_format) {
        Ok(format) => format,
        Err(message) => return emit_document_error(&message, ctx),
    };
    let mut doc = match DocumentFile::open(input_file, input_format) {
        Ok(doc) => doc,
        Err(err) => return emit_document_error(&err.to_string(), ctx),
    };
    if let Err(err) = doc.remove(key, slug, slug_field) {
        return emit_document_error(&err.to_string(), ctx);
    }
    let event = json_result(json!({
        "code": "document_removed",
        "format": document_format_name(doc.format()),
        "key": key,
        "slug": slug,
    }))
    .build();
    emit_document_event(event, ctx, 0)
}

/// `unset`: remove the entry at dot-path `key` entirely from `--input-file`,
/// preserving the rest of the document's source formatting.
fn run_unset(key: &str, input_file: &Path, ctx: &DocumentContext<'_>) -> ExitCode {
    if let Err(message) = reject_input_file_dash(input_file) {
        return emit_document_error(&message, ctx);
    }
    let input_format = match resolve_input_format(ctx.input_format) {
        Ok(format) => format,
        Err(message) => return emit_document_error(&message, ctx),
    };
    let mut doc = match DocumentFile::open(input_file, input_format) {
        Ok(doc) => doc,
        Err(err) => return emit_document_error(&err.to_string(), ctx),
    };
    if let Err(err) = doc.unset(key) {
        return emit_document_error(&err.to_string(), ctx);
    }
    let event = json_result(json!({
        "code": "document_unset",
        "format": document_format_name(doc.format()),
        "key": key,
    }))
    .build();
    emit_document_event(event, ctx, 0)
}

/// Emit a document command's successful result event, redacting through
/// `ctx.secret_names` in addition to the crate's default `_secret`-suffix
/// convention.
fn emit_document_event(event: impl Into<Value>, ctx: &DocumentContext<'_>, code: u8) -> ExitCode {
    let output_options = OutputOptions {
        redaction: Redactor::new().secret_names(ctx.secret_names.iter().cloned()),
        style: PlainStyle::default(),
    };
    emit_event_with_options(event, ctx.format, &output_options, code)
}

/// Emit a document command failure as a `document_error` AFDATA record
/// (exit code 1). Argument-shape problems detected after clap parsing (bad
/// `--input-format`, malformed `FIELD=VALUE`, a rejected `--input-file -`,
/// ...) share this path with runtime document errors, mirroring the
/// uniform `config_error`-style reporting `agent-first-config`'s CLI used.
fn emit_document_error(message: &str, ctx: &DocumentContext<'_>) -> ExitCode {
    let event = build_error_event(json_error("document_error", message));
    emit_document_event(event, ctx, 1)
}

/// Parse an explicit `--input-format` value into a [`DocumentFormat`].
fn parse_document_format(name: &str) -> Result<DocumentFormat, String> {
    match name.to_ascii_lowercase().as_str() {
        "json" => Ok(DocumentFormat::Json),
        "toml" => Ok(DocumentFormat::Toml),
        "yaml" | "yml" => Ok(DocumentFormat::Yaml),
        "dotenv" | "env" => Ok(DocumentFormat::Dotenv),
        "ini" => Ok(DocumentFormat::Ini),
        other => Err(format!(
            "unsupported --input-format `{other}`; expected json, toml, yaml, yml, dotenv, env, or ini"
        )),
    }
}

/// Resolve an optional `--input-format` string into an optional
/// [`DocumentFormat`], surfacing a parse error as `Err`.
fn resolve_input_format(input_format: Option<&str>) -> Result<Option<DocumentFormat>, String> {
    input_format.map(parse_document_format).transpose()
}

/// Human-readable format name for document command output (the `format`
/// field): `JSON`/`TOML`/`YAML`/`dotenv`/`INI`.
fn document_format_name(format: DocumentFormat) -> &'static str {
    match format {
        DocumentFormat::Json => "JSON",
        DocumentFormat::Toml => "TOML",
        DocumentFormat::Yaml => "YAML",
        DocumentFormat::Dotenv => "dotenv",
        DocumentFormat::Ini => "INI",
    }
}

/// Resolve `(value, format)` for a document read command from either FILE or
/// stdin. FILE is opened through [`DocumentFile::open`] (format detected
/// from its extension unless `input_format` overrides it). With no FILE,
/// stdin is read only when it is not a TTY — an interactive terminal errors
/// immediately rather than blocking — defaulting to JSON unless
/// `input_format` overrides it.
fn read_document_input(
    file: Option<&Path>,
    input_format: Option<DocumentFormat>,
) -> Result<(DocumentValue, DocumentFormat), String> {
    match file {
        Some(path) => {
            let doc = DocumentFile::open(path, input_format).map_err(|err| err.to_string())?;
            Ok((doc.value().clone(), doc.format()))
        }
        None => {
            if std::io::stdin().is_terminal() {
                return Err("no input: pass a FILE or pipe stdin".to_string());
            }
            let format = input_format.unwrap_or(DocumentFormat::Json);
            let doc = Document::from_reader(std::io::stdin().lock(), format)
                .map_err(|err| err.to_string())?;
            Ok((doc.value().clone(), doc.format()))
        }
    }
}

/// Extract `key`'s scalar as raw bytes for `value`: a string's bytes are
/// copied verbatim; other scalars render their display form; `Null` renders
/// `"null"`; a non-finite float, array, or object is rejected.
fn document_scalar_bytes(value: &DocumentValue, key: &str) -> Result<Vec<u8>, String> {
    let text = match value {
        DocumentValue::String(value) => return Ok(value.as_bytes().to_vec()),
        DocumentValue::Bool(value) => value.to_string(),
        DocumentValue::Integer(value) => value.to_string(),
        DocumentValue::Unsigned(value) => value.to_string(),
        DocumentValue::Float(value) => {
            if !value.is_finite() {
                return Err(format!("non-finite scalar at `{key}`"));
            }
            value.to_string()
        }
        DocumentValue::Null => "null".to_string(),
        DocumentValue::Array(_) | DocumentValue::Object(_) => {
            return Err(format!("path `{key}` is not a scalar"));
        }
    };
    Ok(text.into_bytes())
}

/// Whether `key`'s leaf (final) path segment would be redacted by afdata's
/// secret-naming convention: an exact `_secret`/`_SECRET` suffix, or an exact
/// match against `secret_names` (the `--secret-name` list).
fn document_leaf_is_secret(key: &str, secret_names: &[String]) -> Result<bool, String> {
    let segments = parse_path(key).map_err(|err| err.to_string())?;
    let Some(leaf) = segments.last() else {
        return Err(format!("path `{key}` has no segments"));
    };
    let redactor = Redactor::new().secret_names(secret_names.iter().cloned());
    Ok(redactor.is_secret_name(leaf))
}

/// Reject a literal `-` for `--input-file`. Unlike read commands (where a
/// missing FILE already means "read stdin"), mutation commands never read
/// stdin, so a dash has no meaning here and would otherwise silently try
/// (and fail) to open a file literally named `-`.
fn reject_input_file_dash(input_file: &Path) -> Result<(), String> {
    if input_file == Path::new("-") {
        return Err(
            "--input-file `-` is not a valid path; mutation commands do not read stdin".to_string(),
        );
    }
    Ok(())
}

/// Write `bytes` directly to stdout with no AFDATA envelope and no forced
/// trailing newline (used by `value`'s raw scalar output).
fn write_raw_exit(bytes: &[u8]) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    if let Err(err) = stdout.write_all(bytes) {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            return ExitCode::from(0);
        }
        return ExitCode::from(1);
    }
    if stdout.flush().is_err() {
        return ExitCode::from(1);
    }
    ExitCode::from(0)
}

const MAX_VALUE_SECRET_BYTES: usize = 1024 * 1024;

/// Resolve `set`'s value source: an explicit `--value-secret*` source wins
/// over positional VALUES (clap already enforces they are mutually
/// exclusive); otherwise the positional VALUES are returned as-is.
fn secret_or_values(args: SetValueArgs) -> Result<Vec<String>, String> {
    let SetValueArgs {
        values,
        value_secret,
        value_secret_stdin,
        value_secret_prompt,
        value_secret_fd,
    } = args;
    if let Some(value) = value_secret {
        return Ok(vec![value]);
    }
    if value_secret_stdin {
        if std::io::stdin().is_terminal() {
            return Err("stdin is a TTY; use --value-secret-prompt".to_string());
        }
        return read_secret_reader(std::io::stdin().lock(), "stdin");
    }
    if value_secret_prompt {
        #[cfg(unix)]
        {
            return read_secret_prompt();
        }
        #[cfg(not(unix))]
        {
            return Err("prompt secret input is unsupported on this platform".to_string());
        }
    }
    if let Some(fd) = value_secret_fd {
        #[cfg(unix)]
        {
            let number = fd
                .parse::<i32>()
                .map_err(|_| "--value-secret-fd requires a numeric descriptor".to_string())?;
            if number < 3 {
                return Err("--value-secret-fd requires a descriptor >= 3".to_string());
            }
            use std::os::unix::io::FromRawFd;
            // SAFETY: ownership is transferred exactly once and the descriptor is closed on drop.
            let file = unsafe { std::fs::File::from_raw_fd(number) };
            return read_secret_reader(file, "file descriptor");
        }
        #[cfg(not(unix))]
        {
            let _ = fd;
            return Err("raw file descriptors are unsupported on this platform".to_string());
        }
    }
    Ok(values)
}

fn read_secret_reader<R: std::io::Read>(reader: R, source: &str) -> Result<Vec<String>, String> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_VALUE_SECRET_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read secret from {source}: {error}"))?;
    if bytes.len() > MAX_VALUE_SECRET_BYTES {
        return Err(format!("secret exceeds {MAX_VALUE_SECRET_BYTES} bytes"));
    }
    let value =
        String::from_utf8(bytes).map_err(|_| "secret input must be valid UTF-8".to_string())?;
    Ok(vec![value])
}

#[cfg(unix)]
fn read_secret_prompt() -> Result<Vec<String>, String> {
    use std::io::BufRead;
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|error| format!("open controlling terminal: {error}"))?;
    let status = std::process::Command::new("stty")
        .args(["-echo"])
        .status()
        .map_err(|error| format!("disable terminal echo: {error}"))?;
    if !status.success() {
        return Err("disable terminal echo failed".to_string());
    }
    let _echo_guard = TerminalEchoGuard;
    write!(tty, "Secret: ").map_err(|error| format!("write prompt: {error}"))?;
    let mut value = String::new();
    let result = {
        let mut reader = std::io::BufReader::new(&mut tty);
        reader.read_line(&mut value)
    }
    .map_err(|error| format!("read secret from prompt: {error}"));
    let _ = writeln!(tty);
    result?;
    let value = value.trim_end_matches(['\n', '\r']);
    if value.len() > MAX_VALUE_SECRET_BYTES {
        return Err(format!("secret exceeds {MAX_VALUE_SECRET_BYTES} bytes"));
    }
    Ok(vec![value.to_string()])
}

#[cfg(unix)]
struct TerminalEchoGuard;

#[cfg(unix)]
impl Drop for TerminalEchoGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("stty").arg("echo").status();
    }
}

fn lint_value(value: &Value, pointer: &str, findings: &mut Vec<Finding>) {
    lint_unsafe_integer(value, pointer, findings);
    match value {
        Value::Object(map) => {
            if let Some(Value::Object(properties)) = map.get("properties") {
                for (name, schema) in properties {
                    lint_secret_schema_property(
                        name,
                        schema,
                        &join_pointer(pointer, "properties"),
                        findings,
                    );
                }
            }
            for (key, child) in map {
                lint_suffix_type(key, child, &join_pointer(pointer, key), findings);
                lint_value(child, &join_pointer(pointer, key), findings);
            }
        }
        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                lint_value(item, &join_pointer(pointer, &idx.to_string()), findings);
            }
        }
        _ => {}
    }
}

fn lint_secret_schema_property(
    name: &str,
    schema: &Value,
    properties_pointer: &str,
    findings: &mut Vec<Finding>,
) {
    if !name.ends_with("_secret") {
        return;
    }
    let Some(obj) = schema.as_object() else {
        return;
    };
    let property_pointer = join_pointer(properties_pointer, name);
    for field in ["default", "example"] {
        if let Some(value) = obj.get(field)
            && !is_redacted_secret_literal(value)
        {
            findings.push(Finding::error(
                "secret_schema_value_exposed",
                join_pointer(&property_pointer, field),
                format!("schema property {name:?} exposes secret {field}"),
            ));
        }
    }
    if let Some(Value::Array(examples)) = obj.get("examples") {
        for (idx, value) in examples.iter().enumerate() {
            if !is_redacted_secret_literal(value) {
                findings.push(Finding::error(
                    "secret_schema_value_exposed",
                    join_pointer(
                        &join_pointer(&property_pointer, "examples"),
                        &idx.to_string(),
                    ),
                    format!("schema property {name:?} exposes secret example"),
                ));
            }
        }
    }
}

fn is_redacted_secret_literal(value: &Value) -> bool {
    matches!(value, Value::Null) || matches!(value, Value::String(s) if s == "***")
}

fn lint_suffix_type(key: &str, value: &Value, pointer: &str, findings: &mut Vec<Finding>) {
    let message = if key.ends_with("_bytes") {
        if is_non_negative_integer(value) {
            None
        } else {
            Some(format!("{key:?} must be a non-negative integer byte count"))
        }
    } else if key.ends_with("_epoch_s") || key.ends_with("_epoch_ms") {
        if is_integer(value) {
            None
        } else {
            Some(format!("{key:?} must be an integer epoch timestamp"))
        }
    } else if key.ends_with("_epoch_ns") {
        if is_decimal_integer_string(value) {
            None
        } else {
            Some(format!("{key:?} must be a decimal integer string"))
        }
    } else if key.ends_with("_sats") || key.ends_with("_msats") {
        if is_integer(value) || is_decimal_integer_string(value) {
            None
        } else {
            Some(format!(
                "{key:?} must be an integer or decimal integer string"
            ))
        }
    } else if key.ends_with("_percent") {
        if value.is_number() {
            None
        } else {
            Some(format!("{key:?} must be numeric"))
        }
    } else if is_duration_suffix(key) {
        if value.is_number() {
            None
        } else {
            Some(format!("{key:?} must be a numeric duration"))
        }
    } else if is_currency_minor_unit_suffix(key) {
        if is_integer(value) {
            None
        } else {
            Some(format!("{key:?} must be an integer currency amount"))
        }
    } else if key.ends_with("_rfc3339") {
        if value.as_str().is_some_and(is_valid_rfc3339) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 date-time with a mandatory offset (e.g. 2026-02-14T10:30:00Z)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if key.ends_with("_url") {
        if value.as_str().is_some_and(is_wellformed_url_field) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be a single URL (no internal whitespace or bare credentials)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if key.ends_with("_bcp47") {
        if value.as_str().is_some_and(is_valid_bcp47) {
            None
        } else if value.is_string() {
            Some(format!("{key:?} must be a well-formed BCP 47 language tag"))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if key.ends_with("_rfc3339_date") {
        if value.as_str().is_some_and(is_valid_rfc3339_date) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 full-date (YYYY-MM-DD)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if key.ends_with("_rfc3339_time") {
        if value.as_str().is_some_and(is_valid_rfc3339_time) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 partial-time (HH:MM:SS[.fraction], no Z or offset)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if key.ends_with("_utc_offset") {
        if value.as_str().and_then(normalize_utc_offset).is_some() {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be a fixed UTC offset (\"UTC\" or ±HH:MM)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else {
        None
    };
    if let Some(message) = message {
        findings.push(Finding::error(
            "suffix_type_mismatch",
            pointer.to_string(),
            message,
        ));
    }
}

fn lint_unsafe_integer(value: &Value, pointer: &str, findings: &mut Vec<Finding>) {
    let Value::Number(number) = value else {
        return;
    };
    if number.is_i64() {
        let Some(value) = number.as_i64() else {
            return;
        };
        if value.unsigned_abs() > MAX_SAFE_INTEGER {
            findings.push(unsafe_integer_finding(pointer));
        }
    } else if number.is_u64() {
        let Some(value) = number.as_u64() else {
            return;
        };
        if value > MAX_SAFE_INTEGER {
            findings.push(unsafe_integer_finding(pointer));
        }
    }
}

fn unsafe_integer_finding(pointer: &str) -> Finding {
    Finding::error(
        "unsafe_integer",
        pointer.to_string(),
        "integer exceeds JavaScript safe integer range ±(2^53-1)".to_string(),
    )
}

fn is_integer(value: &Value) -> bool {
    matches!(value, Value::Number(number) if number.is_i64() || number.is_u64())
}

fn is_non_negative_integer(value: &Value) -> bool {
    matches!(value, Value::Number(number) if number.as_u64().is_some())
}

fn is_decimal_integer_string(value: &Value) -> bool {
    let Value::String(text) = value else {
        return false;
    };
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit())
}

/// A numeric duration suffix (`timeout_s`, `retry_after_ms`, `ttl_minutes`, …).
/// The epoch suffixes (`_epoch_s`/`_epoch_ms`/`_epoch_ns`) are matched earlier in
/// the chain, so they never reach here.
fn is_duration_suffix(key: &str) -> bool {
    key.ends_with("_ns")
        || key.ends_with("_us")
        || key.ends_with("_ms")
        || key.ends_with("_s")
        || key.ends_with("_minutes")
        || key.ends_with("_hours")
        || key.ends_with("_days")
}

/// An integer minor-unit currency suffix (`price_usd_cents`, `fee_jpy`,
/// `budget_btc_micro`, …). `_sats`/`_msats` allow a decimal-string form and are
/// matched earlier in the chain.
fn is_currency_minor_unit_suffix(key: &str) -> bool {
    key.ends_with("_cents") || key.ends_with("_micro") || key.ends_with("_jpy")
}

/// True when a `_url` field value is a single URL: a scheme-prefixed absolute URL,
/// or a schemeless relative reference with no internal whitespace and no bare `@`
/// credential sigil. This mirrors the redaction gate in the library's
/// `redaction.rs`: a value this rejects is exactly one redaction would blanket-
/// redact (internal whitespace, or a schemeless `user:pass@host` connection
/// string) rather than surgically clean.
fn is_wellformed_url_field(s: &str) -> bool {
    if is_scheme_prefixed_url(s) || is_scheme_prefixed_url(s.trim()) {
        return true;
    }
    !s.chars().any(char::is_whitespace) && !s.contains('@')
}

/// True when `s` begins with a URL scheme (`ALPHA *(ALPHA / DIGIT / "+" / "-" /
/// ".") "://"`) and contains no ASCII whitespace — a single bare absolute URL.
fn is_scheme_prefixed_url(s: &str) -> bool {
    if s.bytes().any(|b| b.is_ascii_whitespace()) {
        return false;
    }
    let bytes = s.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.') {
            i += 1;
        } else {
            break;
        }
    }
    s[i..].starts_with("://")
}

fn join_pointer(base: &str, token: &str) -> String {
    let escaped = token.replace('~', "~0").replace('/', "~1");
    if base.is_empty() {
        format!("/{escaped}")
    } else {
        format!("{base}/{escaped}")
    }
}

#[cfg(all(test, feature = "skill"))]
mod skill_tests {
    use super::*;

    #[test]
    fn parses_and_validates_skill_directory() {
        let parsed =
            Cli::try_parse_from(["afdata", "skill", "validate", "skills/agent-first-data"]);
        assert!(matches!(
            parsed,
            Ok(Cli {
                command: Command::Skill {
                    action,
                    input: Some(input),
                    ..
                },
                ..
            }) if action == "validate" && input == Path::new("skills/agent-first-data")
        ));

        let loaded = read_skill_input(Some(Path::new("skills/agent-first-data")));
        assert!(matches!(
            loaded,
            Ok((text, Some(expected_name), _))
                if expected_name == "agent-first-data"
                    && agent_first_data::skill::validate_skill_named(&text, &expected_name).is_ok()
        ));
    }

    #[cfg(feature = "skill-admin")]
    #[test]
    fn keeps_existing_skill_admin_command_shape() {
        let parsed = Cli::try_parse_from([
            "afdata",
            "skill",
            "status",
            "--agent",
            "opencode",
            "--scope",
            "workspace",
        ]);
        assert!(matches!(
            parsed,
            Ok(Cli {
                command: Command::Skill {
                    action,
                    input: None,
                    agent,
                    scope,
                    ..
                },
                ..
            }) if action == "status" && agent == "opencode" && scope == "workspace"
        ));
    }
}
