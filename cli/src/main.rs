#![allow(clippy::print_stdout, clippy::print_stderr)]

use agent_first_data::{
    OutputFormat, build_cli_error, cli_output, cli_parse_output, json_error, json_result,
    output_json, output_plain, output_yaml, validate_protocol_event, validate_protocol_stream,
};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Parser)]
#[command(
    name = "afdata",
    version,
    about = "Validate, lint, and format Agent-First Data JSON.",
    after_help = concat!("AFDATA: ", env!("CARGO_PKG_VERSION")),
    disable_help_subcommand = true
)]
struct Cli {
    /// Output format: json, yaml, or plain
    #[arg(long, global = true, default_value = "json")]
    output: String,

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
    /// Format JSON or JSONL with AFDATA redaction
    Format {
        /// Input file; stdin is used when omitted
        input: Option<PathBuf>,
    },
    /// Manage the bundled Agent Skill
    #[cfg(feature = "skill-admin")]
    Skill {
        /// Action: status, install, or uninstall
        action: String,
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
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                return write_text_exit(&err.render().to_string(), 0);
            }
            let event = build_cli_error(&err.to_string(), Some("try: afdata --help"));
            return emit_event(&event, OutputFormat::Json, 2);
        }
    };

    let format = match cli_parse_output(&cli.output) {
        Ok(format) => format,
        Err(message) => {
            let event = build_cli_error(&message, Some("valid values: json, yaml, plain"));
            return emit_event(&event, OutputFormat::Json, 2);
        }
    };

    match cli.command {
        Command::Lint { input } => run_lint(input.as_deref(), format),
        Command::Validate {
            input,
            strict,
            event,
        } => run_validate(input.as_deref(), format, strict, event),
        Command::Format { input } => run_format(input.as_deref(), format),
        #[cfg(feature = "skill-admin")]
        Command::Skill {
            action,
            agent,
            scope,
            skills_dir,
            force,
        } => run_skill(&action, &agent, &scope, skills_dir, force, format),
    }
}

fn run_lint(input: Option<&Path>, format: OutputFormat) -> ExitCode {
    let text = match read_input(input) {
        Ok(text) => text,
        Err(message) => {
            #[allow(clippy::expect_used)]
            let event = json_error("read_failed", &message)
                .build()
                .expect("json_error: builder failed unexpectedly");
            return emit_event(&event, format, 1);
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
            #[allow(clippy::expect_used)]
            let event = json_error("read_failed", &message)
                .build()
                .expect("json_error: builder failed unexpectedly");
            return emit_event(&event, format, 1);
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
            if let Err(message) = validate_protocol_stream(&events, strict) {
                findings.push(Finding::error(
                    "protocol_stream_invalid",
                    String::new(),
                    message,
                ));
            }
        }
        ParsedInput::Single(value) => validate_single_input(value, strict, &mut findings),
        ParsedInput::Lines(values) => {
            if let Err(message) = validate_protocol_stream(&values, strict) {
                findings.push(Finding::error(
                    "protocol_stream_invalid",
                    String::new(),
                    message,
                ));
            }
        }
    }
    emit_findings("validation_failed", "validation failed", findings, format)
}

fn validate_single_input(value: Value, strict: bool, findings: &mut Vec<Finding>) {
    let kind = value.get("kind").and_then(Value::as_str);
    if matches!(kind, Some("log" | "progress")) {
        if let Err(message) = validate_protocol_stream(&[value], strict) {
            findings.push(Finding::error(
                "protocol_stream_invalid",
                String::new(),
                message,
            ));
        }
        return;
    }
    validate_one_event(&value, strict, "", findings);
}

fn validate_one_event(value: &Value, strict: bool, pointer: &str, findings: &mut Vec<Finding>) {
    if let Err(message) = validate_protocol_event(value, strict) {
        findings.push(Finding::error(
            "protocol_event_invalid",
            pointer.to_string(),
            message,
        ));
    }
}

fn run_format(input: Option<&Path>, format: OutputFormat) -> ExitCode {
    let text = match read_input(input) {
        Ok(text) => text,
        Err(message) => {
            #[allow(clippy::expect_used)]
            let event = json_error("read_failed", &message)
                .build()
                .expect("json_error: builder failed unexpectedly");
            return emit_event(&event, format, 1);
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
                if idx > 0 && format == OutputFormat::Yaml {
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

#[cfg(feature = "skill-admin")]
fn run_skill(
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
            return emit_event(&event, format, 2);
        }
    };
    let agent = match parse_skill_agent(agent) {
        Ok(agent) => agent,
        Err(message) => {
            let event = build_cli_error(
                &message,
                Some("valid agents: all, codex, claude-code, opencode, hermes"),
            );
            return emit_event(&event, format, 2);
        }
    };
    let scope = match parse_skill_scope(scope) {
        Ok(scope) => scope,
        Err(message) => {
            let event = build_cli_error(&message, Some("valid scopes: personal, workspace"));
            return emit_event(&event, format, 2);
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
                #[allow(clippy::expect_used)]
                let event = json_result(value)
                    .build()
                    .expect("json_result: builder failed unexpectedly");
                emit_event(&event, format, 0)
            }
            Err(err) => {
                #[allow(clippy::expect_used)]
                let event = json_error(
                    "serialization_failed",
                    &format!("failed to serialize skill report: {err}"),
                )
                .build()
                .expect("json_error: builder failed unexpectedly");
                emit_event(&event, format, 1)
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
            #[allow(clippy::expect_used)]
            let event = builder.build().expect("builder failed");
            emit_event(&event, format, 2)
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
    let mut out = match format {
        OutputFormat::Json => output_json(value),
        OutputFormat::Yaml => output_yaml(value),
        OutputFormat::Plain => output_plain(value),
    };
    if suppress_yaml_boundary
        && format == OutputFormat::Yaml
        && let Some(stripped) = out.strip_prefix("---\n")
    {
        out = stripped.to_string();
    }
    out
}

fn read_input(input: Option<&Path>) -> Result<String, String> {
    let mut text = String::new();
    match input {
        Some(path) => {
            text = std::fs::read_to_string(path)
                .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        }
        None => {
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
    #[allow(clippy::expect_used)]
    let event = builder.build().expect("builder failed");
    emit_event(&event, format, 1)
}

fn emit_findings(
    error_code: &'static str,
    error_message: &'static str,
    findings: Vec<Finding>,
    format: OutputFormat,
) -> ExitCode {
    let findings_json = Value::Array(findings.iter().map(Finding::to_json).collect());
    if findings.is_empty() {
        #[allow(clippy::expect_used)]
        let event = json_result(json!({"ok": true, "findings": findings_json}))
            .build()
            .expect("json_result: builder failed unexpectedly");
        emit_event(&event, format, 0)
    } else {
        #[allow(clippy::expect_used)]
        let event = json_error(error_code, error_message)
            .fields(json!({"findings": findings_json}))
            .build()
            .expect("builder failed");
        emit_event(&event, format, 1)
    }
}

fn emit_event(event: &Value, format: OutputFormat, code: u8) -> ExitCode {
    let mut event = event.clone();
    if event.get("trace").is_none()
        && let Some(object) = event.as_object_mut()
    {
        object.insert("trace".to_string(), json!({}));
    }
    if let Err(validation_message) = validate_protocol_event(&event, true) {
        #[allow(clippy::expect_used)]
        let fallback = json_error(
            "internal_protocol_error",
            "afdata attempted to emit an invalid protocol event",
        )
        .field("validation_message", json!(validation_message))
        .build()
        .expect("builder failed");
        let mut text = cli_output(&fallback, OutputFormat::Json);
        if !text.ends_with('\n') {
            text.push('\n');
        }
        return write_text_exit(&text, 1);
    }
    let mut text = cli_output(&event, format);
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
    } else if key.ends_with("_rfc3339") || key.ends_with("_url") {
        if value.is_string() {
            None
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

fn join_pointer(base: &str, token: &str) -> String {
    let escaped = token.replace('~', "~0").replace('/', "~1");
    if base.is_empty() {
        format!("/{escaped}")
    } else {
        format!("{base}/{escaped}")
    }
}
