#![allow(clippy::print_stdout, clippy::print_stderr)]

use agent_first_data::document::{
    BareOverwrite, Document, DocumentError, DocumentFile, Format as DocumentFormat,
    Value as DocumentValue, ValueType, get_path, guard_bare_overwrite, join_path, parse_path,
    value_from_type,
};
use agent_first_data::{
    ArgSpec, CliOutcome, CliSpec, Combination, CommandSpec, ErrorBuilder, Event, OutputFormat,
    OutputOptions, OutputPlan, OutputSpec, OutputTo, PlainStyle, Redactor, ResolvedInvocation,
    build_afdata_cli, build_cli_error, cli_error_event, cli_help_event, cli_parse_output,
    cli_version_event, is_valid_bcp47, is_valid_rfc3339, is_valid_rfc3339_date,
    is_valid_rfc3339_time, json_error, json_log, json_result, normalize_utc_offset, render,
    render_cli_reference, validate_protocol_event, validate_protocol_stream,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

enum EmitCommand {
    /// Emit a diagnostic log event (stderr under the default split)
    Log {
        /// Log level: debug, info, warn, or error
        level: String,
        /// Human-readable message; do not place secrets in free text
        message: String,
    },
    /// Emit a terminal result event (stdout under the default split)
    Result {
        /// Human-readable result message
        message: String,
    },
    /// Emit a terminal error event and exit with status 1
    Error {
        /// Stable machine-readable error code
        code: String,
        /// Human-readable error message
        message: String,
        /// Suggested corrective action
        hint: Option<String>,
        /// Mark the failure as safe to retry
        retryable: bool,
    },
}

enum ShellCommand {
    /// Print the sourceable Bash authoring kit as raw Bash on stdout
    Bash,
}

#[cfg(feature = "skill")]
enum SkillCommand {
    /// Validate a SKILL.md file or skill directory against the Agent Skills spec
    Validate {
        /// SKILL.md file or skill directory, or `-` for SKILL.md text on stdin
        input: PathBuf,
    },
    /// Report whether the bundled Agent Skill is installed for each target agent
    #[cfg(feature = "skill-admin")]
    Status {
        /// Agent target: all, codex, claude-code, opencode, or hermes
        agent: String,
        /// Skill scope: personal or workspace
        scope: String,
        /// Explicit skills directory; requires a single concrete --agent
        skills_dir: Option<String>,
    },
    /// Install the bundled Agent Skill for each target agent
    #[cfg(feature = "skill-admin")]
    Install {
        /// Agent target: all, codex, claude-code, opencode, or hermes
        agent: String,
        /// Skill scope: personal or workspace
        scope: String,
        /// Explicit skills directory; requires a single concrete --agent
        skills_dir: Option<String>,
        /// Overwrite a skill this tool did not manage
        force: bool,
    },
    /// Uninstall the bundled Agent Skill for each target agent
    #[cfg(feature = "skill-admin")]
    Uninstall {
        /// Agent target: all, codex, claude-code, opencode, or hermes
        agent: String,
        /// Skill scope: personal or workspace
        scope: String,
        /// Explicit skills directory; requires a single concrete --agent
        skills_dir: Option<String>,
        /// Remove a skill this tool did not manage
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

    /// A finding the tool is not certain about. Convention-adoption checks are
    /// heuristic — they read intent from a field's name — so they must not fail
    /// a document the way a violated suffix rule does. They still have to be
    /// said: staying silent about them is what let `lint` pass a file made
    /// entirely of the ambiguities this convention exists to remove.
    fn warning(rule_id: &'static str, pointer: String, message: String) -> Self {
        Self {
            rule_id,
            severity: "warning",
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

fn protocol_output() -> OutputSpec {
    OutputSpec::protocol_finite(
        ["json", "yaml", "plain"],
        ["split", "stdout", "stderr"],
        "json",
        "split",
    )
    .file_sinks(stream_file_sinks())
}

fn raw_output() -> OutputSpec {
    OutputSpec::raw().file_sinks(stream_file_sinks())
}

fn stream_file_sinks() -> Vec<&'static str> {
    #[cfg(feature = "stream-redirect")]
    {
        vec!["stdout", "stderr"]
    }
    #[cfg(not(feature = "stream-redirect"))]
    {
        Vec::new()
    }
}

fn positional(id: &str, index: usize, value_name: &str, about: &str) -> ArgSpec {
    ArgSpec::positional(id, index, value_name).about(about)
}

fn input_format_arg() -> ArgSpec {
    ArgSpec::option_enum(
        "--input-format",
        [
            "json",
            "toml",
            "yaml",
            "yml",
            "dotenv",
            "env",
            "ini",
            "toml-frontmatter",
            "yaml-frontmatter",
        ],
    )
    .value_name("FORMAT")
    .about("Document format override")
}

fn secret_name_arg() -> ArgSpec {
    ArgSpec::option("--secret-name", "FIELD")
        .repeatable()
        .about("Extra exact field name to redact")
}

fn afdata_cli_spec() -> Result<agent_first_data::BuiltCliSpec, agent_first_data::CliSpecError> {
    let protocol = protocol_output();
    let raw = raw_output();
    let mut spec = CliSpec::new("afdata", env!("CARGO_PKG_VERSION"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .display_name(env!("DISPLAY_NAME"))
        // build.rs writes "unknown" when no git tree is reachable (a crates.io
        // tarball); an unknown build is no build.
        .build_id(match env!("GIT_SHA") {
            "unknown" => "",
            sha => sha,
        })
        .lifecycle_output(protocol.clone())
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["lint"])
                .about("Lint structured data for deterministic AFDATA issues")
                .arg(positional(
                    "input",
                    0,
                    "INPUT",
                    "Input file, or - for stdin",
                ))
                .arg(input_format_arg())
                .arg(
                    ArgSpec::option_enum("--min-severity", ["warning", "error"])
                        .value_name("SEVERITY")
                        .default("warning")
                        .about("Lowest severity to report; `error` drops the heuristic checks"),
                )
                .combination(
                    Combination::new("lint")
                        .action("lint")
                        .required(["input"])
                        .optional(["input_format", "min_severity"])
                        .output(protocol.clone()),
                ),
        )
        .command(
            CommandSpec::new(["validate"])
                .about("Validate protocol-v1 events or a finite event stream")
                .arg(positional(
                    "input",
                    0,
                    "INPUT",
                    "Input file, or - for stdin",
                ))
                .arg(ArgSpec::flag("--strict").about("Enforce the strict protocol profile"))
                .arg(
                    ArgSpec::flag("--per-event")
                        .about("Validate values independently without stream lifecycle rules"),
                )
                .combination(
                    Combination::new("validate")
                        .action("validate")
                        .required(["input"])
                        .optional(["strict", "per_event"])
                        .output(protocol.clone()),
                ),
        )
        .command(
            CommandSpec::new(["render"])
                .about("Render JSON or JSONL through AFDATA redaction and formatting")
                .arg(positional(
                    "input",
                    0,
                    "INPUT",
                    "Input file, or - for stdin",
                ))
                .arg(secret_name_arg())
                .combination(
                    Combination::new("render")
                        .action("render")
                        .required(["input"])
                        .optional(["secret_name"])
                        .output(protocol.clone()),
                ),
        )
        .command(CommandSpec::new(["emit"]).about("Emit one AFDATA event"))
        .command(
            CommandSpec::new(["emit", "log"])
                .about("Emit a diagnostic log event")
                .arg(
                    ArgSpec::positional_enum(
                        "level",
                        0,
                        "LEVEL",
                        ["debug", "info", "warn", "error"],
                    )
                    .about("debug, info, warn, or error"),
                )
                .arg(positional(
                    "message",
                    1,
                    "MESSAGE",
                    "Human-readable message",
                ))
                .combination(
                    Combination::new("emit-log")
                        .action("emit_log")
                        .required(["level", "message"])
                        .output(protocol.clone()),
                ),
        )
        .command(
            CommandSpec::new(["emit", "result"])
                .about("Emit a terminal result event")
                .arg(positional("message", 0, "MESSAGE", "Result message"))
                .combination(
                    Combination::new("emit-result")
                        .action("emit_result")
                        .required(["message"])
                        .output(protocol.clone()),
                ),
        )
        .command(
            CommandSpec::new(["emit", "error"])
                .about("Emit a terminal error event")
                .arg(positional("code", 0, "CODE", "Stable error code"))
                .arg(positional("message", 1, "MESSAGE", "Error message"))
                .arg(ArgSpec::option("--hint", "HINT").about("Suggested corrective action"))
                .arg(ArgSpec::flag("--retryable").about("Mark the failure safe to retry"))
                .combination(
                    Combination::new("emit-error")
                        .action("emit_error")
                        .required(["code", "message"])
                        .optional(["hint", "retryable"])
                        .output(protocol.clone()),
                ),
        )
        .command(CommandSpec::new(["shell"]).about("Export a shell authoring kit"))
        .command(
            CommandSpec::new(["shell", "bash"])
                .about("Print the sourceable Bash authoring kit")
                .combination(
                    Combination::new("shell-bash")
                        .action("shell_bash")
                        .output(raw.clone()),
                ),
        )
        .command(
            CommandSpec::new(["get"])
                .about("Read a document or one value as an AFDATA result")
                .arg(positional(
                    "file",
                    0,
                    "FILE",
                    "Document file, or - for stdin",
                ))
                .arg(positional("key", 1, "KEY", "Optional dot-path"))
                .arg(input_format_arg())
                .arg(secret_name_arg())
                .combination(
                    Combination::new("get")
                        .action("get")
                        .required(["file"])
                        .optional(["key", "input_format", "secret_name"])
                        .output(protocol.clone()),
                ),
        )
        .command(
            CommandSpec::new(["value"])
                .about("Read one scalar as raw stdout bytes")
                .arg(positional(
                    "file",
                    0,
                    "FILE",
                    "Document file, or - for stdin",
                ))
                .arg(positional("key", 1, "KEY", "Dot-path to one scalar"))
                .arg(ArgSpec::flag("--reveal-secret").about("Allow a secret-named leaf"))
                .arg(ArgSpec::option("--default", "VALUE").about("Fallback for missing or null"))
                .arg(input_format_arg())
                .arg(secret_name_arg())
                .combination(
                    Combination::new("value")
                        .action("value")
                        .required(["file", "key"])
                        .optional(["reveal_secret", "default", "input_format", "secret_name"])
                        .output(raw.clone()),
                ),
        )
        .command(
            CommandSpec::new(["values"])
                .about("Read many scalars as raw lines, from one parse of the document")
                .arg(positional(
                    "file",
                    0,
                    "FILE",
                    "Document file, or - for stdin",
                ))
                .arg(
                    ArgSpec::positional("key", 1, "KEY")
                        .repeatable()
                        .about("Dot-path to one scalar; repeat for each value wanted"),
                )
                .arg(ArgSpec::flag("--reveal-secret").about("Allow a secret-named leaf"))
                .arg(ArgSpec::option("--default", "VALUE").about("Fallback for missing or null"))
                .arg(input_format_arg())
                .arg(secret_name_arg())
                .combination(
                    Combination::new("values")
                        .action("values")
                        .required(["file", "key"])
                        .optional(["reveal_secret", "default", "input_format", "secret_name"])
                        .output(raw.clone()),
                ),
        )
        .command(enumerate_command(
            "paths",
            "paths",
            "List each child's full dot-path as raw lines",
            &raw,
        ))
        .command(enumerate_command(
            "keys",
            "keys",
            "List child names as raw lines, without their parent path",
            &raw,
        ))
        .command(
            CommandSpec::new(["set"])
                .about("Set a value at a dot-path, creating missing object parents")
                .arg(positional("file", 0, "FILE", "Document file to mutate"))
                .arg(positional("key", 1, "KEY", "Dot-path to set"))
                .arg(positional("value", 2, "VALUE", "Value to write"))
                .arg(
                    ArgSpec::option_enum(
                        "--value-type",
                        ["string", "number", "bool", "null", "json"],
                    )
                    .value_name("TYPE")
                    .default("string")
                    .about("Exact VALUE type"),
                )
                .arg(
                    ArgSpec::option("--secret-from", "SOURCE")
                        .about("Read a secret string from stdin, prompt, fd:N, or env:VAR"),
                )
                .arg(input_format_arg())
                .combination(
                    Combination::new("set-value")
                        .action("set")
                        .about("Set one typed scalar or JSON value")
                        .fixed_one_of("value_type", ["string", "number", "bool", "json"])
                        .required(["file", "key", "value"])
                        .optional(["input_format"])
                        .output(protocol.clone()),
                )
                .combination(
                    Combination::new("set-null")
                        .action("set")
                        .about("Set the key to null; takes no VALUE")
                        .fixed("value_type", "null")
                        .required(["file", "key"])
                        .optional(["input_format"])
                        .output(protocol.clone()),
                )
                .combination(
                    Combination::new("set-secret")
                        .action("set")
                        .about("Set the key from a secret source, never from argv")
                        .required(["file", "key", "secret_from"])
                        .optional(["input_format"])
                        .output(protocol.clone()),
                ),
        )
        .command(
            CommandSpec::new(["unset"])
                .about("Remove one document entry")
                .arg(positional("file", 0, "FILE", "Document file to mutate"))
                .arg(positional("key", 1, "KEY", "Dot-path to remove"))
                .arg(input_format_arg())
                .combination(
                    Combination::new("unset")
                        .action("unset")
                        .required(["file", "key"])
                        .optional(["input_format"])
                        .output(protocol.clone()),
                ),
        )
        .command(
            CommandSpec::new(["add"])
                .about("Add an element to a keyed list")
                .arg(positional("file", 0, "FILE", "Document file to mutate"))
                .arg(positional("key", 1, "KEY", "Dot-path to the keyed list"))
                .arg(positional("slug", 2, "SLUG", "New element slug"))
                .arg(
                    ArgSpec::positional("fields", 3, "FIELD=VALUE")
                        .repeatable()
                        .about("Additional string fields"),
                )
                .arg(
                    ArgSpec::option("--slug-field", "FIELD")
                        .about("Field that identifies each list element"),
                )
                .arg(input_format_arg())
                .combination(
                    Combination::new("add")
                        .action("add")
                        .required(["file", "key", "slug", "slug_field"])
                        .optional(["fields", "input_format"])
                        .output(protocol.clone()),
                ),
        )
        .command(
            CommandSpec::new(["remove"])
                .about("Remove a keyed-list element by slug")
                .arg(positional("file", 0, "FILE", "Document file to mutate"))
                .arg(positional("key", 1, "KEY", "Dot-path to the keyed list"))
                .arg(positional("slug", 2, "SLUG", "Element slug"))
                .arg(
                    ArgSpec::option("--slug-field", "FIELD")
                        .about("Field that identifies each list element"),
                )
                .arg(input_format_arg())
                .combination(
                    Combination::new("remove")
                        .action("remove")
                        .required(["file", "key", "slug", "slug_field"])
                        .optional(["input_format"])
                        .output(protocol.clone()),
                ),
        );

    #[cfg(feature = "skill")]
    {
        spec = spec
            .command(CommandSpec::new(["skill"]).about("Validate or manage the bundled skill"))
            .command(
                CommandSpec::new(["skill", "validate"])
                    .about("Validate an Agent Skill")
                    .arg(positional(
                        "input",
                        0,
                        "INPUT",
                        "SKILL.md file, directory, or - for stdin",
                    ))
                    .combination(
                        Combination::new("skill-validate")
                            .action("skill_validate")
                            .required(["input"])
                            .output(protocol.clone()),
                    ),
            );
    }

    #[cfg(feature = "skill-admin")]
    {
        spec = spec
            .command(skill_admin_command(
                "status",
                "skill_status",
                false,
                &protocol,
            ))
            .command(skill_admin_command(
                "install",
                "skill_install",
                true,
                &protocol,
            ))
            .command(skill_admin_command(
                "uninstall",
                "skill_uninstall",
                true,
                &protocol,
            ));
    }

    build_afdata_cli(spec)
}

fn enumerate_command(name: &str, action: &str, about: &str, output: &OutputSpec) -> CommandSpec {
    CommandSpec::new([name])
        .about(about)
        .arg(positional(
            "file",
            0,
            "FILE",
            "Document file, or - for stdin",
        ))
        .arg(positional("key", 1, "KEY", "Optional container dot-path"))
        .arg(input_format_arg())
        .arg(ArgSpec::flag("--missing-ok").about("Succeed with no output when KEY is absent"))
        .arg(ArgSpec::flag("--null").about("Use NUL separators"))
        .combination(
            Combination::new(name)
                .action(action)
                .required(["file"])
                .optional(["key", "input_format", "missing_ok", "null"])
                .output(output.clone()),
        )
}

#[cfg(feature = "skill-admin")]
fn skill_admin_command(
    command: &str,
    action: &str,
    force: bool,
    output: &OutputSpec,
) -> CommandSpec {
    let mut spec = CommandSpec::new(["skill", command])
        .about(match action {
            "skill_status" => "Report whether the bundled Agent Skill is installed and current",
            "skill_install" => "Install the bundled Agent Skill",
            _ => "Remove an afdata-managed Agent Skill",
        })
        .arg(
            ArgSpec::option_enum(
                "--agent",
                ["all", "codex", "claude-code", "opencode", "hermes"],
            )
            .value_name("AGENT")
            .default("all")
            .about("Target agent"),
        )
        .arg(
            ArgSpec::option_enum("--scope", ["personal", "workspace"])
                .value_name("SCOPE")
                .default("personal")
                .about("Skill scope"),
        )
        .arg(
            ArgSpec::option("--skills-dir", "PATH")
                .about("Explicit directory; only valid with one concrete agent"),
        );
    if force {
        spec = spec.arg(ArgSpec::flag("--force").about("Overwrite or remove an unmanaged skill"));
    }
    // The two shapes differ only in which agents they target, so each must say
    // so: the command's own description cannot distinguish them.
    let verb = match action {
        "skill_status" => "Report on",
        "skill_install" => "Install into",
        _ => "Remove from",
    };
    let mut common_optional = vec!["scope"];
    let mut concrete_optional = vec!["scope", "skills_dir"];
    if force {
        common_optional.push("force");
        concrete_optional.push("force");
    }
    spec.combination(
        Combination::new(format!("skill-{command}-all"))
            .action(action)
            .about(format!("{verb} every agent that supports the scope"))
            .fixed("agent", "all")
            .optional(common_optional)
            .output(output.clone()),
    )
    .combination(
        Combination::new(format!("skill-{command}-agent"))
            .action(action)
            .about(format!(
                "{verb} one named agent; only this shape accepts --skills-dir"
            ))
            .fixed_one_of("agent", ["codex", "claude-code", "opencode", "hermes"])
            .optional(concrete_optional)
            .output(output.clone()),
    )
}

fn main() -> ExitCode {
    closed_world_main()
}

type AfdataActionHandler = fn(&ResolvedInvocation) -> ExitCode;

fn closed_world_main() -> ExitCode {
    let cli = match afdata_cli_spec() {
        Ok(cli) => cli,
        Err(error) => {
            let event = build_error_event(
                json_error("cli_spec_invalid", &error.to_string())
                    .hint("fix the built-in afdata cli-spec-v1 registry"),
            );
            return emit_event(event, OutputFormat::Json, 1);
        }
    };
    let mut handlers: Vec<(&str, AfdataActionHandler)> = vec![
        ("lint", dispatch_invocation),
        ("values", dispatch_invocation),
        ("validate", dispatch_invocation),
        ("render", dispatch_invocation),
        ("emit_log", dispatch_invocation),
        ("emit_result", dispatch_invocation),
        ("emit_error", dispatch_invocation),
        ("shell_bash", dispatch_invocation),
        ("get", dispatch_invocation),
        ("value", dispatch_invocation),
        ("paths", dispatch_invocation),
        ("keys", dispatch_invocation),
        ("set", dispatch_invocation),
        ("unset", dispatch_invocation),
        ("add", dispatch_invocation),
        ("remove", dispatch_invocation),
    ];
    #[cfg(feature = "skill")]
    handlers.push(("skill_validate", dispatch_invocation));
    #[cfg(feature = "skill-admin")]
    {
        handlers.push(("skill_status", dispatch_invocation));
        handlers.push(("skill_install", dispatch_invocation));
        handlers.push(("skill_uninstall", dispatch_invocation));
    }
    let app = match cli.bind_actions(handlers) {
        Ok(app) => app,
        Err(error) => {
            let event = build_error_event(
                json_error("cli_actions_invalid", &error.to_string())
                    .hint("make action handler coverage exactly match cli-spec-v1"),
            );
            return emit_event(event, OutputFormat::Json, 1);
        }
    };
    let outcome = match app.resolve_from(std::env::args_os()) {
        Ok(outcome) => outcome,
        Err(error) => {
            let code = error.exit_code();
            return emit_event(cli_error_event(&error), OutputFormat::Json, code);
        }
    };
    match outcome {
        CliOutcome::Run(invocation) => {
            let _stream_redirect = match install_output_redirect(invocation.output_plan()) {
                Ok(guard) => guard,
                Err(message) => return emit_output_setup_error(&message),
            };
            if let Err(message) = activate_output_plan(invocation.output_plan()) {
                return emit_output_setup_error(&message);
            }
            app.execute(&invocation)
        }
        // Injected for every registry, so a spore gets an offline reference
        // without registering anything — and without spending a line of the
        // agent's discovery surface on a command no agent calls.
        CliOutcome::Docs(docs) => {
            let _stream_redirect = match install_output_redirect(docs.output_plan()) {
                Ok(guard) => guard,
                Err(message) => return emit_output_setup_error(&message),
            };
            write_text_exit(&render_cli_reference(&cli), 0)
        }
        CliOutcome::Help(help) => {
            let _stream_redirect = match install_output_redirect(help.output_plan()) {
                Ok(guard) => guard,
                Err(message) => return emit_output_setup_error(&message),
            };
            let format = match activate_output_plan(help.output_plan()) {
                Ok(format) => format,
                Err(message) => return emit_output_setup_error(&message),
            };
            if format == OutputFormat::Plain {
                write_text_exit_to(&help.plain(), 0, result_stream(output_to()))
            } else {
                emit_event(cli_help_event(&help), format, 0)
            }
        }
        CliOutcome::Version(version) => {
            let _stream_redirect = match install_output_redirect(version.output_plan()) {
                Ok(guard) => guard,
                Err(message) => return emit_output_setup_error(&message),
            };
            let format = match activate_output_plan(version.output_plan()) {
                Ok(format) => format,
                Err(message) => return emit_output_setup_error(&message),
            };
            emit_event(cli_version_event(&version), format, 0)
        }
    }
}

#[cfg(feature = "stream-redirect")]
type OutputRedirectGuard = agent_first_data::stream_redirect::InstalledStreamRedirect;

#[cfg(not(feature = "stream-redirect"))]
struct OutputRedirectGuard;

fn install_output_redirect(plan: &OutputPlan) -> Result<Option<OutputRedirectGuard>, String> {
    #[cfg(feature = "stream-redirect")]
    {
        let config = agent_first_data::stream_redirect::StreamRedirectConfig::new(
            plan.stdout_file().map(Path::to_path_buf),
            plan.stderr_file().map(Path::to_path_buf),
        )
        .map_err(|error| error.to_string())?;
        config
            .as_ref()
            .map(agent_first_data::stream_redirect::install)
            .transpose()
            .map_err(|error| error.to_string())
    }
    #[cfg(not(feature = "stream-redirect"))]
    {
        if plan.stdout_file().is_some() || plan.stderr_file().is_some() {
            return Err("file sinks are unavailable in this build".to_string());
        }
        Ok(None)
    }
}

fn activate_output_plan(plan: &OutputPlan) -> Result<OutputFormat, String> {
    match plan {
        OutputPlan::Raw { .. } => {
            OUTPUT_TO
                .set(OutputTo::Split)
                .map_err(|_| "output plan was activated more than once".to_string())?;
            Ok(OutputFormat::Json)
        }
        OutputPlan::Protocol {
            format,
            destination,
            ..
        } => {
            let format = cli_parse_output(format)?;
            let destination = OutputTo::parse(destination)?;
            OUTPUT_TO
                .set(destination)
                .map_err(|_| "output plan was activated more than once".to_string())?;
            Ok(format)
        }
    }
}

fn emit_output_setup_error(message: &str) -> ExitCode {
    let event = build_error_event(json_error("output_setup_failed", message));
    emit_event_to(
        event,
        OutputFormat::Json,
        &OutputOptions::default(),
        1,
        Stream::Stderr,
    )
}

fn invocation_string(invocation: &ResolvedInvocation, id: &str) -> String {
    invocation
        .required(id)
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn invocation_optional_string(invocation: &ResolvedInvocation, id: &str) -> Option<String> {
    invocation
        .optional(id)
        .and_then(agent_first_data::CliValue::as_str)
        .map(str::to_string)
}

fn invocation_strings(invocation: &ResolvedInvocation, id: &str) -> Vec<String> {
    invocation
        .repeated(id)
        .iter()
        .filter_map(agent_first_data::CliValue::as_str)
        .map(str::to_string)
        .collect()
}

fn invocation_flag(invocation: &ResolvedInvocation, id: &str) -> bool {
    invocation
        .optional(id)
        .and_then(agent_first_data::CliValue::as_bool)
        .unwrap_or(false)
}

fn dispatch_invocation(invocation: &ResolvedInvocation) -> ExitCode {
    let format = invocation
        .output_plan()
        .format()
        .and_then(|format| cli_parse_output(format).ok())
        .unwrap_or(OutputFormat::Json);
    match invocation.action_id() {
        "lint" => {
            let input = invocation_string(invocation, "input");
            let input_format = invocation_optional_string(invocation, "input_format");
            let min_severity = invocation_string(invocation, "min_severity");
            run_lint(
                Path::new(&input),
                input_format.as_deref(),
                &min_severity,
                format,
            )
        }
        "validate" => {
            let input = invocation_string(invocation, "input");
            run_validate(
                Path::new(&input),
                format,
                invocation_flag(invocation, "strict"),
                invocation_flag(invocation, "per_event"),
            )
        }
        "render" => {
            let input = invocation_string(invocation, "input");
            run_render(
                Path::new(&input),
                &invocation_strings(invocation, "secret_name"),
                format,
            )
        }
        "emit_log" => run_emit(
            EmitCommand::Log {
                level: invocation_string(invocation, "level"),
                message: invocation_string(invocation, "message"),
            },
            format,
        ),
        "emit_result" => run_emit(
            EmitCommand::Result {
                message: invocation_string(invocation, "message"),
            },
            format,
        ),
        "emit_error" => run_emit(
            EmitCommand::Error {
                code: invocation_string(invocation, "code"),
                message: invocation_string(invocation, "message"),
                hint: invocation_optional_string(invocation, "hint"),
                retryable: invocation_flag(invocation, "retryable"),
            },
            format,
        ),
        "shell_bash" => run_shell(ShellCommand::Bash),
        #[cfg(feature = "skill")]
        "skill_validate" => run_skill(
            SkillCommand::Validate {
                input: PathBuf::from(invocation_string(invocation, "input")),
            },
            format,
        ),
        #[cfg(feature = "skill-admin")]
        "skill_status" => run_skill(
            SkillCommand::Status {
                agent: invocation_string(invocation, "agent"),
                scope: invocation_string(invocation, "scope"),
                skills_dir: invocation_optional_string(invocation, "skills_dir"),
            },
            format,
        ),
        #[cfg(feature = "skill-admin")]
        "skill_install" => run_skill(
            SkillCommand::Install {
                agent: invocation_string(invocation, "agent"),
                scope: invocation_string(invocation, "scope"),
                skills_dir: invocation_optional_string(invocation, "skills_dir"),
                force: invocation_flag(invocation, "force"),
            },
            format,
        ),
        #[cfg(feature = "skill-admin")]
        "skill_uninstall" => run_skill(
            SkillCommand::Uninstall {
                agent: invocation_string(invocation, "agent"),
                scope: invocation_string(invocation, "scope"),
                skills_dir: invocation_optional_string(invocation, "skills_dir"),
                force: invocation_flag(invocation, "force"),
            },
            format,
        ),
        "get" => dispatch_get(invocation, format),
        "value" => dispatch_value(invocation, format),
        "values" => dispatch_values(invocation, format),
        "paths" | "keys" => dispatch_enumerate(invocation, format),
        "set" => dispatch_set(invocation, format),
        "unset" => dispatch_unset(invocation, format),
        "add" => dispatch_add(invocation, format),
        "remove" => dispatch_remove(invocation, format),
        _ => emit_event(
            build_error_event(json_error(
                "cli_action_unreachable",
                "resolved action has no implementation",
            )),
            OutputFormat::Json,
            1,
        ),
    }
}

fn dispatch_get(invocation: &ResolvedInvocation, format: OutputFormat) -> ExitCode {
    let file = invocation_string(invocation, "file");
    let key = invocation_optional_string(invocation, "key");
    let input_format = invocation_optional_string(invocation, "input_format");
    let secret_names = invocation_strings(invocation, "secret_name");
    run_get(
        Path::new(&file),
        key.as_deref(),
        &DocumentContext {
            input_format: input_format.as_deref(),
            secret_names: &secret_names,
            format,
        },
    )
}

fn dispatch_values(invocation: &ResolvedInvocation, format: OutputFormat) -> ExitCode {
    let file = invocation_string(invocation, "file");
    let keys = invocation_strings(invocation, "key");
    let default = invocation_optional_string(invocation, "default");
    let input_format = invocation_optional_string(invocation, "input_format");
    let secret_names = invocation_strings(invocation, "secret_name");
    let ctx = DocumentContext {
        input_format: input_format.as_deref(),
        secret_names: &secret_names,
        format,
    };
    finish_raw(
        compute_values(
            Path::new(&file),
            &keys,
            invocation_flag(invocation, "reveal_secret"),
            default.as_deref(),
            &ctx,
        ),
        &ctx,
    )
}

fn dispatch_value(invocation: &ResolvedInvocation, format: OutputFormat) -> ExitCode {
    let file = invocation_string(invocation, "file");
    let key = invocation_string(invocation, "key");
    let default = invocation_optional_string(invocation, "default");
    let input_format = invocation_optional_string(invocation, "input_format");
    let secret_names = invocation_strings(invocation, "secret_name");
    run_value_get(
        Path::new(&file),
        &key,
        invocation_flag(invocation, "reveal_secret"),
        default.as_deref(),
        &DocumentContext {
            input_format: input_format.as_deref(),
            secret_names: &secret_names,
            format,
        },
    )
}

fn dispatch_enumerate(invocation: &ResolvedInvocation, format: OutputFormat) -> ExitCode {
    let file = invocation_string(invocation, "file");
    let key = invocation_optional_string(invocation, "key");
    let input_format = invocation_optional_string(invocation, "input_format");
    run_enumerate(
        Path::new(&file),
        key.as_deref(),
        input_format.as_deref(),
        invocation_flag(invocation, "missing_ok"),
        invocation_flag(invocation, "null"),
        format,
        if invocation.action_id() == "paths" {
            EnumerateMode::Paths
        } else {
            EnumerateMode::Keys
        },
    )
}

fn dispatch_set(invocation: &ResolvedInvocation, format: OutputFormat) -> ExitCode {
    let file = invocation_string(invocation, "file");
    let key = invocation_string(invocation, "key");
    let value = invocation_optional_string(invocation, "value");
    let value_type = invocation
        .was_explicit("value_type")
        .then(|| invocation_optional_string(invocation, "value_type"))
        .flatten();
    let secret_from = invocation_optional_string(invocation, "secret_from");
    let input_format = invocation_optional_string(invocation, "input_format");
    run_set(
        Path::new(&file),
        &key,
        value,
        value_type.as_deref(),
        secret_from.as_deref(),
        &DocumentContext {
            input_format: input_format.as_deref(),
            secret_names: &[],
            format,
        },
    )
}

fn dispatch_unset(invocation: &ResolvedInvocation, format: OutputFormat) -> ExitCode {
    let file = invocation_string(invocation, "file");
    let key = invocation_string(invocation, "key");
    let input_format = invocation_optional_string(invocation, "input_format");
    run_unset(
        Path::new(&file),
        &key,
        &DocumentContext {
            input_format: input_format.as_deref(),
            secret_names: &[],
            format,
        },
    )
}

fn dispatch_add(invocation: &ResolvedInvocation, format: OutputFormat) -> ExitCode {
    let file = invocation_string(invocation, "file");
    let key = invocation_string(invocation, "key");
    let slug = invocation_string(invocation, "slug");
    let slug_field = invocation_string(invocation, "slug_field");
    let fields = invocation_strings(invocation, "fields");
    let input_format = invocation_optional_string(invocation, "input_format");
    run_add(
        Path::new(&file),
        &key,
        &slug,
        &slug_field,
        &fields,
        &DocumentContext {
            input_format: input_format.as_deref(),
            secret_names: &[],
            format,
        },
    )
}

fn dispatch_remove(invocation: &ResolvedInvocation, format: OutputFormat) -> ExitCode {
    let file = invocation_string(invocation, "file");
    let key = invocation_string(invocation, "key");
    let slug = invocation_string(invocation, "slug");
    let slug_field = invocation_string(invocation, "slug_field");
    let input_format = invocation_optional_string(invocation, "input_format");
    run_remove(
        Path::new(&file),
        &key,
        &slug,
        &slug_field,
        &DocumentContext {
            input_format: input_format.as_deref(),
            secret_names: &[],
            format,
        },
    )
}

// ═══════════════════════════════════════════
// Protocol tools: lint, validate, render, skill
// ═══════════════════════════════════════════

fn run_lint(
    input: &Path,
    input_format: Option<&str>,
    min_severity: &str,
    format: OutputFormat,
) -> ExitCode {
    let resolved = match resolve_input_format(input_format) {
        Ok(resolved) => resolved,
        Err(message) => return emit_usage_error(&message, format),
    };
    // R6: lint reuses the document input layer (extension inference +
    // `--input-format` override) so TOML/YAML/dotenv/INI documents get the
    // same AFDATA naming/suffix checks as JSON. Unlike other document
    // commands, an undetected extension (or `-` with no override) falls
    // back to JSON/JSONL rather than erroring — that dual-mode behavior
    // predates this command's document-format support and stays unchanged.
    let effective = resolved
        .or_else(|| {
            if input == Path::new("-") {
                None
            } else {
                DocumentFormat::detect(input)
            }
        })
        .unwrap_or(DocumentFormat::Json);

    let mut findings = Vec::new();
    if effective == DocumentFormat::Json {
        let text = match read_input_or_stdin(input) {
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
        match parsed {
            ParsedInput::Single(value) => lint_value(&value, "", &mut findings),
            ParsedInput::Lines(values) => {
                for (idx, value) in values.iter().enumerate() {
                    lint_value(value, &format!("/{}", idx + 1), &mut findings);
                }
            }
        }
    } else {
        let (value, _doc_format) = match read_document_input(input, Some(effective)) {
            Ok(pair) => pair,
            Err(err) => {
                let event = build_error_event(json_error(err.code(), &err.redacted_message()));
                return emit_event(event, format, 1);
            }
        };
        let json_value = match Value::try_from(value) {
            Ok(value) => value,
            Err(error) => {
                let event = build_error_event(json_error(error.code(), &error.redacted_message()));
                return emit_event(event, format, 1);
            }
        };
        lint_value(&json_value, "", &mut findings);
    }
    if min_severity == "error" {
        findings.retain(|finding| finding.severity == "error");
    }
    emit_findings("lint_failed", "lint failed", findings, format)
}

fn run_validate(input: &Path, format: OutputFormat, strict: bool, per_event: bool) -> ExitCode {
    let text = match read_input_or_stdin(input) {
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
    if per_event {
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

fn run_render(input: &Path, secret_names: &[String], format: OutputFormat) -> ExitCode {
    let text = match read_input_or_stdin(input) {
        Ok(text) => text,
        Err(message) => {
            let event = build_error_event(json_error("read_failed", &message));
            return emit_event(event, format, 1);
        }
    };
    let parsed = match parse_json_or_jsonl(&text) {
        Ok(parsed) => parsed,
        // R5: parse-error presentation honors the negotiated `--output`
        // (previously hardcoded to JSON, inconsistent with lint/validate).
        Err(err) => return emit_parse_error(err, format),
    };
    let output_options = OutputOptions {
        redaction: Redactor::new().secret_names(secret_names.iter().cloned()),
        style: PlainStyle::default(),
    };
    match parsed {
        ParsedInput::Single(value) => {
            write_text_exit(&format_value(&value, format, false, &output_options), 0)
        }
        ParsedInput::Lines(values) => {
            let mut out = String::new();
            for (idx, value) in values.iter().enumerate() {
                if idx > 0 && is_yaml_format(format) {
                    out.push_str("---\n");
                }
                out.push_str(&format_value(value, format, idx > 0, &output_options));
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            write_text_exit(&out, 0)
        }
    }
}

fn run_emit(action: EmitCommand, format: OutputFormat) -> ExitCode {
    match action {
        EmitCommand::Log { level, message } => {
            if message.is_empty() {
                return emit_cli_usage_error(
                    "log MESSAGE must not be empty",
                    "run `afdata emit log --help` and choose one registered combination",
                    format,
                );
            }
            let event = json_log(json!({
                "level": level,
                "message": message,
            }))
            .build();
            emit_event(event, format, 0)
        }
        EmitCommand::Result { message } => {
            if message.is_empty() {
                return emit_cli_usage_error(
                    "result MESSAGE must not be empty",
                    "run `afdata emit result --help` and choose one registered combination",
                    format,
                );
            }
            let event = json_result(json!({"message": message})).build();
            emit_event(event, format, 0)
        }
        EmitCommand::Error {
            code,
            message,
            hint,
            retryable,
        } => {
            if code.is_empty() || message.is_empty() {
                return emit_cli_usage_error(
                    "error CODE and MESSAGE must not be empty",
                    "run `afdata emit error --help` and choose one registered combination",
                    format,
                );
            }
            let event = build_error_event(
                json_error(&code, &message)
                    .hint_if_some(hint.as_deref())
                    .retryable_if(retryable),
            );
            emit_event(event, format, 1)
        }
    }
}

fn run_shell(action: ShellCommand) -> ExitCode {
    match action {
        ShellCommand::Bash => {
            const BASH_SOURCE: &str = include_str!("../../bash/afdata.sh");
            write_text_exit(BASH_SOURCE, 0)
        }
    }
}

#[cfg(feature = "skill")]
fn run_skill(action: SkillCommand, format: OutputFormat) -> ExitCode {
    match action {
        SkillCommand::Validate { input } => run_skill_validate(&input, format),
        #[cfg(feature = "skill-admin")]
        SkillCommand::Status {
            agent,
            scope,
            skills_dir,
        } => run_skill_admin_action(
            agent_first_data::skill::SkillAction::Status,
            &agent,
            &scope,
            skills_dir,
            false,
            format,
        ),
        #[cfg(feature = "skill-admin")]
        SkillCommand::Install {
            agent,
            scope,
            skills_dir,
            force,
        } => run_skill_admin_action(
            agent_first_data::skill::SkillAction::Install,
            &agent,
            &scope,
            skills_dir,
            force,
            format,
        ),
        #[cfg(feature = "skill-admin")]
        SkillCommand::Uninstall {
            agent,
            scope,
            skills_dir,
            force,
        } => run_skill_admin_action(
            agent_first_data::skill::SkillAction::Uninstall,
            &agent,
            &scope,
            skills_dir,
            force,
            format,
        ),
    }
}

#[cfg(feature = "skill")]
fn run_skill_validate(input: &Path, format: OutputFormat) -> ExitCode {
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
fn read_skill_input(input: &Path) -> Result<(String, Option<String>, String), String> {
    if input == Path::new("-") {
        return read_input_or_stdin(Path::new("-")).map(|text| (text, None, "<stdin>".to_string()));
    }
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
    action: agent_first_data::skill::SkillAction,
    agent: &str,
    scope: &str,
    skills_dir: Option<String>,
    force: bool,
    format: OutputFormat,
) -> ExitCode {
    use agent_first_data::skill::{SkillAsset, SkillOptions, SkillSpec, run_skill_admin};

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
    // SKILL.md points agents at these bundled references, so they must install
    // alongside it. Keep this list in step with skills/agent-first-data/references/
    // — the package smoke installs to a temp dir and asserts the whole tree lands.
    const SKILL_ASSETS: &[SkillAsset] = &[
        SkillAsset {
            path: "references/bash.md",
            contents: include_str!("../../skills/agent-first-data/references/bash.md"),
        },
        SkillAsset {
            path: "references/cli-protocol.md",
            contents: include_str!("../../skills/agent-first-data/references/cli-protocol.md"),
        },
        SkillAsset {
            path: "references/documents.md",
            contents: include_str!("../../skills/agent-first-data/references/documents.md"),
        },
        SkillAsset {
            path: "references/naming-output.md",
            contents: include_str!("../../skills/agent-first-data/references/naming-output.md"),
        },
        SkillAsset {
            path: "references/registry.json",
            contents: include_str!("../../skills/agent-first-data/references/registry.json"),
        },
        SkillAsset {
            path: "references/protocol-v1.schema.json",
            contents: include_str!(
                "../../skills/agent-first-data/references/protocol-v1.schema.json"
            ),
        },
        SkillAsset {
            path: "references/cli-help-v2.schema.json",
            contents: include_str!(
                "../../skills/agent-first-data/references/cli-help-v2.schema.json"
            ),
        },
        SkillAsset {
            path: "references/cli-spec-v1.schema.json",
            contents: include_str!(
                "../../skills/agent-first-data/references/cli-spec-v1.schema.json"
            ),
        },
    ];
    let spec = SkillSpec {
        name: "agent-first-data",
        source: SKILL_SOURCE,
        title: "Agent-First Data",
        marker_slug: "afdata",
        assets: SKILL_ASSETS,
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

fn format_value(
    value: &Value,
    format: OutputFormat,
    suppress_yaml_boundary: bool,
    output_options: &OutputOptions,
) -> String {
    let mut out = render(value, format, output_options);
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

/// Read `input` fully: `-` reads stdin to EOF, otherwise reads the file at
/// that path. D1: no implicit stdin fallback and no TTY detection — every
/// caller passes an explicit path or `-`, so there is no "maybe hangs"
/// shape left to guard against; reading `-` on an interactive terminal is
/// the user's explicit request, same as any other Unix tool.
fn read_input_or_stdin(input: &Path) -> Result<String, String> {
    if input == Path::new("-") {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|err| format!("failed to read stdin: {err}"))?;
        return Ok(text);
    }
    std::fs::read_to_string(input)
        .map_err(|err| format!("failed to read {}: {err}", input.display()))
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
    if findings.iter().any(|finding| finding.severity == "error") {
        let event = build_error_event(
            json_error(error_code, error_message).fields(json!({"findings": findings_json})),
        );
        return emit_event(event, format, 1);
    }
    // Warnings are reported but do not fail: they are heuristic. `ok` still
    // turns false, so a caller reading one field learns there is something to
    // read, and exit 0 keeps a heuristic from breaking a pipeline.
    let event = json_result(json!({"ok": findings.is_empty(), "findings": findings_json})).build();
    emit_event(event, format, 0)
}

/// A usage-class error (R2): the CLI invocation's own shape was wrong
/// (an invalid `--input-format`/`--value-type` name, a malformed
/// `FIELD=VALUE`, a mutation command given `-`, …) — distinct from a
/// runtime document error, and always exit 2. Routed by [`emit_event`] as a
/// `kind:"error"` envelope, so it lands on stderr under the default split (and
/// on the chosen stream under `--output-to stdout|stderr`).
fn emit_usage_error(message: &str, format: OutputFormat) -> ExitCode {
    let event = build_error_event(json_error("document_usage_error", message));
    emit_event(event, format, 2)
}

/// A value the registry accepted but this command cannot use.
///
/// `cli-spec-v1` has no "non-empty string" type, so emptiness is checked here —
/// one layer after the parser, but it is the same verdict the parser would have
/// reached, so it carries the parser's own code rather than a second spelling
/// for it. `build_cli_error`'s generic `cli_error` stays for CLIs that have no
/// registry to name a rule; this one has.
fn emit_cli_usage_error(message: &str, hint: &str, format: OutputFormat) -> ExitCode {
    let event = build_error_event(json_error("cli_invalid_argument_value", message).hint(hint));
    emit_event(event, format, 2)
}

/// The resolved `--output-to` selector. Read through [`output_to`], which
/// falls back to `Split` before the flag is parsed so any pre-dispatch
/// usage/parse error still routes its `error` envelope to stderr by default.
static OUTPUT_TO: std::sync::OnceLock<OutputTo> = std::sync::OnceLock::new();

fn output_to() -> OutputTo {
    OUTPUT_TO.get().copied().unwrap_or(OutputTo::Split)
}

/// Stream a `kind:"result"` payload lands on under `selector`.
fn result_stream(selector: OutputTo) -> Stream {
    match selector {
        OutputTo::Split | OutputTo::Stdout => Stream::Stdout,
        OutputTo::Stderr => Stream::Stderr,
    }
}

/// Stream a `kind:"error"` (or `progress`/`log` diagnostic) lands on under
/// `selector`.
fn error_stream(selector: OutputTo) -> Stream {
    match selector {
        OutputTo::Split | OutputTo::Stderr => Stream::Stderr,
        OutputTo::Stdout => Stream::Stdout,
    }
}

/// Route an already-built event by its `kind`: `result` follows
/// [`result_stream`], every other kind follows [`error_stream`].
fn stream_for(event: &Value, selector: OutputTo) -> Stream {
    if event.get("kind").and_then(Value::as_str) == Some("result") {
        result_stream(selector)
    } else {
        error_stream(selector)
    }
}

fn emit_event(event: impl Into<Value>, format: OutputFormat, code: u8) -> ExitCode {
    let event: Value = event.into();
    let stream = stream_for(&event, output_to());
    emit_event_to(event, format, &OutputOptions::default(), code, stream)
}

enum Stream {
    Stdout,
    Stderr,
}

/// As [`emit_event`], but rendering through `output_options` (redaction and
/// style) and writing to an explicit `stream`. Used by the document commands
/// so `--secret-name` reaches the final render and so callers that already
/// know the routing (via [`result_stream`]/[`error_stream`] under the resolved
/// `--output-to`) can name the stream directly.
fn emit_event_to(
    event: impl Into<Value>,
    format: OutputFormat,
    output_options: &OutputOptions,
    code: u8,
    stream: Stream,
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
        return write_text_exit_to(&text, 1, stream);
    }
    let mut text = render(&event, format, output_options);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    write_text_exit_to(&text, code, stream)
}

fn write_text_exit(text: &str, code: u8) -> ExitCode {
    write_text_exit_to(text, code, Stream::Stdout)
}

// The sanctioned emitter sink. Per the spec's Channel policy, a finite
// command splits by kind (`result` → stdout, `error`/diagnostics → stderr) and
// `--output-to stdout|stderr` collapses everything onto one stream; both routes
// resolve to a `Stream` here. This is the one place allowed to touch
// `std::io::stderr` directly (clippy.toml's `disallowed-methods` guards against
// stray stderr writes elsewhere that would bypass this routing).
#[allow(clippy::disallowed_methods)]
fn write_text_exit_to(text: &str, code: u8, stream: Stream) -> ExitCode {
    let result = match stream {
        Stream::Stdout => std::io::stdout().lock().write_all(text.as_bytes()),
        Stream::Stderr => std::io::stderr().lock().write_all(text.as_bytes()),
    };
    if let Err(err) = result {
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

/// A document command failure, classified per R2: `Usage` is a CLI-shape
/// mistake (bad `--input-format`/`--value-type` name, malformed
/// `FIELD=VALUE`, a mutation command given `-`, VALUE absent with no
/// `--value-type`/`--secret-from`, …) and always exit 2; `Document` wraps a
/// library [`DocumentError`], mapped to a stable code by
/// [`DocumentError::code`] and always exit 1; `Runtime` covers the handful
/// of CLI-originated *runtime* facts about the document/secret source that
/// are not `DocumentError` variants (a secret leaf without
/// `--reveal-secret`, a non-scalar `value` target, a scalar `paths`/`keys`
/// target, a failed `--secret-from` read) — also exit 1, with an explicit
/// stable code.
enum CliDocError {
    Usage(String),
    Document(DocumentError),
    Runtime { code: &'static str, message: String },
}

impl CliDocError {
    fn runtime(code: &'static str, message: impl Into<String>) -> Self {
        Self::Runtime {
            code,
            message: message.into(),
        }
    }
}

impl From<DocumentError> for CliDocError {
    fn from(err: DocumentError) -> Self {
        Self::Document(err)
    }
}

/// Whether `err` is in the `document_path_not_found` bucket — the "the
/// address doesn't exist" family that `value --default` and
/// `paths`/`keys --missing-ok` are allowed to swallow. Reuses
/// [`DocumentError::code`] directly so the exemption is always exactly the
/// bucket agents see in `error.code`, never a separately-maintained variant list.
fn is_path_not_found(err: &CliDocError) -> bool {
    matches!(err, CliDocError::Document(doc_err) if doc_err.code() == "document_path_not_found")
}

/// Render `err` into `(event, exit_code)`.
fn document_error_event(err: &CliDocError) -> (Event, u8) {
    match err {
        CliDocError::Usage(message) => (
            build_error_event(json_error("document_usage_error", message)),
            2,
        ),
        // `redacted_message`, never `Display`: an error event is the thing an
        // agent routinely writes to a log, and `Display` quotes the offending
        // document text — which is how a `_secret` leaf reached stderr verbatim.
        CliDocError::Document(doc_err) => (
            build_error_event(json_error(doc_err.code(), &doc_err.redacted_message())),
            1,
        ),
        CliDocError::Runtime { code, message } => (build_error_event(json_error(code, message)), 1),
    }
}

/// Finish a `get`/`set`/`unset`/`add`/`remove` invocation. Under the default
/// `--output-to split` the success `result` envelope goes to stdout and the
/// `error` envelope to stderr (so `x=$(afdata get f k)` never captures an error
/// as data and `afdata set f k v >/dev/null` never swallows the diagnostic);
/// `--output-to stdout|stderr` collapses both onto the one chosen stream.
fn finish_document(result: Result<Value, CliDocError>, ctx: &DocumentContext<'_>) -> ExitCode {
    let selector = output_to();
    match result {
        Ok(payload) => emit_document_event(
            json_result(payload).build(),
            ctx,
            0,
            result_stream(selector),
        ),
        Err(err) => {
            let (event, code) = document_error_event(&err);
            emit_document_event(event, ctx, code, error_stream(selector))
        }
    }
}

/// Finish a `value`/`paths`/`keys` invocation: success writes raw bytes
/// straight to stdout with no envelope; failure writes the envelope to stderr,
/// leaving stdout empty — so `x=$(afdata value f k)` never captures a JSON
/// error as data. These commands are always split (they reject a non-default
/// `--output-to`), so `error_stream` here always resolves to stderr.
fn finish_raw(result: Result<Vec<u8>, CliDocError>, ctx: &DocumentContext<'_>) -> ExitCode {
    match result {
        Ok(bytes) => write_raw_exit(&bytes),
        Err(err) => {
            let (event, code) = document_error_event(&err);
            emit_document_event(event, ctx, code, error_stream(output_to()))
        }
    }
}

/// Emit a document command's event to `stream`, redacting through
/// `ctx.secret_names` in addition to the crate's default `_secret`-suffix
/// convention.
fn emit_document_event(
    event: impl Into<Value>,
    ctx: &DocumentContext<'_>,
    code: u8,
    stream: Stream,
) -> ExitCode {
    let output_options = document_output_options(ctx);
    emit_event_to(event, ctx.format, &output_options, code, stream)
}

fn document_output_options(ctx: &DocumentContext<'_>) -> OutputOptions {
    OutputOptions {
        redaction: Redactor::new().secret_names(ctx.secret_names.iter().cloned()),
        style: PlainStyle::default(),
    }
}

/// Parse an explicit `--input-format` value into a [`DocumentFormat`].
fn parse_document_format(name: &str) -> Result<DocumentFormat, String> {
    match name.to_ascii_lowercase().as_str() {
        "json" => Ok(DocumentFormat::Json),
        "toml" => Ok(DocumentFormat::Toml),
        "yaml" | "yml" => Ok(DocumentFormat::Yaml),
        "dotenv" | "env" => Ok(DocumentFormat::Dotenv),
        "ini" => Ok(DocumentFormat::Ini),
        "toml-frontmatter" => Ok(DocumentFormat::TomlFrontmatter),
        "yaml-frontmatter" => Ok(DocumentFormat::YamlFrontmatter),
        other => Err(format!(
            "unsupported --input-format `{other}`; expected json, toml, yaml, yml, dotenv, env, ini, \
             toml-frontmatter, or yaml-frontmatter"
        )),
    }
}

/// Resolve an optional `--input-format` string into an optional
/// [`DocumentFormat`], surfacing a parse error as `Err`.
fn resolve_input_format(input_format: Option<&str>) -> Result<Option<DocumentFormat>, String> {
    input_format.map(parse_document_format).transpose()
}

/// Resolve `(value, format)` for a document read command from `FILE`
/// (`-` reads stdin). `-` defaults to JSON unless `input_format` overrides
/// it; a real path's format is detected from its extension unless
/// `input_format` overrides it.
fn read_document_input(
    file: &Path,
    input_format: Option<DocumentFormat>,
) -> Result<(DocumentValue, DocumentFormat), DocumentError> {
    if file == Path::new("-") {
        let format = input_format.unwrap_or(DocumentFormat::Json);
        let doc = Document::from_reader(std::io::stdin().lock(), format)?;
        return Ok((doc.value().clone(), doc.format()));
    }
    let doc = DocumentFile::open(file, input_format)?;
    Ok((doc.value().clone(), doc.format()))
}

/// Extract `key`'s scalar as raw bytes for `value`: a string's bytes are
/// copied verbatim; other scalars render their display form; a null,
/// non-finite float, array, or object is rejected.
///
/// §4 digit fidelity: [`DocumentValue::Number`] is emitted byte for byte
/// from its stored literal — never routed through `f64::to_string()`, which
/// is exactly the corruption path this variant exists to avoid. `value` is
/// otherwise type-lossy (every scalar becomes plain text) but this makes it
/// digit-faithful: the exact source digits, always.
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
        DocumentValue::Number(text) => return Ok(text.clone().into_bytes()),
        DocumentValue::Null => return Err(format!("path `{key}` is null")),
        DocumentValue::Array(_) | DocumentValue::Object(_) => {
            return Err(format!("path `{key}` is not a scalar"));
        }
    };
    Ok(text.into_bytes())
}

/// Whether `key` addresses anything afdata's secret-naming convention covers:
/// **any** segment on the path carrying an exact `_secret`/`_SECRET` suffix, or
/// matching `secret_names` (the `--secret-name` list) exactly.
///
/// The whole path, not just its leaf. A name marks the subtree beneath it, so
/// `credentials_secret.password` is exactly as secret as `credentials_secret`.
/// Judging the leaf alone let a caller step past the marked node and read the
/// subtree out in the clear.
fn document_path_is_secret(key: &str, secret_names: &[String]) -> Result<bool, DocumentError> {
    let segments = parse_path(key)?;
    if segments.is_empty() {
        return Err(DocumentError::EmptyPath);
    }
    let redactor = Redactor::new().secret_names(secret_names.iter().cloned());
    Ok(segments
        .iter()
        .any(|segment| redactor.is_secret_name(segment)))
}

/// Reject a literal `-` FILE for a mutation command (D1): unlike read
/// commands (where FILE `-` means stdin), mutation commands never read
/// stdin, so `-` has no meaning and is a usage error rather than a silent
/// attempt to open a file literally named `-`.
fn reject_mutation_dash(file: &Path) -> Result<(), CliDocError> {
    if file == Path::new("-") {
        return Err(CliDocError::Usage(
            "`-` is not a valid FILE for a mutation command; mutation commands never read stdin"
                .to_string(),
        ));
    }
    Ok(())
}

/// Write `bytes` directly to stdout with no AFDATA envelope and no forced
/// trailing newline (used by `value`'s raw scalar output and `paths`/`keys`'
/// line-oriented output).
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

/// `get`: read a document whole, or the value at dot-path `key` (D2 — `show`
/// no longer exists; omitting `key` is the whole-document form).
fn run_get(file: &Path, key: Option<&str>, ctx: &DocumentContext<'_>) -> ExitCode {
    finish_document(compute_get(file, key, ctx), ctx)
}

fn compute_get(
    file: &Path,
    key: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> Result<Value, CliDocError> {
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let (root, doc_format) = read_document_input(file, input_format)?;
    let mut payload = serde_json::Map::new();
    payload.insert("code".to_string(), json!("document"));
    payload.insert("format".to_string(), json!(doc_format.cli_name()));
    let json_value: Value = match key {
        None => Value::try_from(root)?,
        Some(key) => {
            let target = get_path(&root, key, &[])?;
            let is_secret = document_path_is_secret(key, ctx.secret_names)?;
            payload.insert("key".to_string(), json!(key));
            // A generic whole-document redact walk (applied via the
            // `--secret-name` output options below) only rewrites object
            // fields it finds by name; the value here sits under the
            // generic `"value"` wrapper key instead of its own field name,
            // so a directly-targeted secret leaf needs this explicit check.
            if is_secret {
                json!("***")
            } else {
                Value::try_from(target)?
            }
        }
    };
    payload.insert("value".to_string(), json_value);
    Ok(Value::Object(payload))
}

/// `value`: like `get` with a required KEY, but writes only the scalar's raw
/// bytes to stdout — no AFDATA envelope, no forced trailing newline. Arrays,
/// objects, and non-finite floats are rejected. A secret-named leaf is
/// rejected unless `--reveal-secret` is passed (never bypassed by default,
/// not even by `--default`). `--default VAL` prints `VAL` instead of erroring
/// when KEY's path does not exist or its value is `null`; an empty string is
/// a real value and does not trigger it (§2).
fn run_value_get(
    file: &Path,
    key: &str,
    reveal_secret: bool,
    default: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    finish_raw(compute_value(file, key, reveal_secret, default, ctx), ctx)
}

fn compute_value(
    file: &Path,
    key: &str,
    reveal_secret: bool,
    default: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> Result<Vec<u8>, CliDocError> {
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let (root, _doc_format) = read_document_input(file, input_format)?;
    scalar_bytes_at(&root, key, reveal_secret, default, ctx)
}

/// Every path in `keys`, one line each, from a single parse of the document.
///
/// `value` re-reads and re-parses the whole file per invocation, so answering
/// "which element has name X" from a shell costs one process and one full parse
/// per candidate — 110 packages in a Cargo.lock took 400ms, and the parse work
/// grows with the square of the document. Reading many paths at once is the
/// same work done once.
///
/// Line framing is the contract, so a value containing a newline is refused
/// rather than silently splitting across two lines: a caller pairing lines with
/// the paths it asked for would read every later value against the wrong path.
fn compute_values(
    file: &Path,
    keys: &[String],
    reveal_secret: bool,
    default: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> Result<Vec<u8>, CliDocError> {
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let (root, _doc_format) = read_document_input(file, input_format)?;
    let mut out = Vec::new();
    for key in keys {
        let bytes = scalar_bytes_at(&root, key, reveal_secret, default, ctx)?;
        if bytes.contains(&b'\n') {
            return Err(CliDocError::runtime(
                "document_multiline_value",
                format!(
                    "path `{key}` holds a value containing a newline, which one-per-line output cannot frame; read that path with `value`"
                ),
            ));
        }
        out.extend_from_slice(&bytes);
        out.push(b'\n');
    }
    Ok(out)
}

fn scalar_bytes_at(
    root: &DocumentValue,
    key: &str,
    reveal_secret: bool,
    default: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> Result<Vec<u8>, CliDocError> {
    if !reveal_secret && document_path_is_secret(key, ctx.secret_names)? {
        return Err(CliDocError::runtime(
            "document_secret_redacted",
            format!("path `{key}` names a secret; pass --reveal-secret"),
        ));
    }
    let target = match get_path(root, key, &[]) {
        Ok(target) => target,
        Err(err) => {
            let err = CliDocError::Document(err);
            if default.is_some() && is_path_not_found(&err) {
                return Ok(default.unwrap_or_default().as_bytes().to_vec());
            }
            return Err(err);
        }
    };
    if target.is_null()
        && let Some(default) = default
    {
        return Ok(default.as_bytes().to_vec());
    }
    if target.is_null() {
        return Err(CliDocError::runtime(
            "document_null_value",
            format!("path `{key}` is null; pass --default to choose replacement output"),
        ));
    }
    document_scalar_bytes(&target, key)
        .map_err(|message| CliDocError::runtime("document_not_scalar", message))
}

/// `paths`/`keys` share everything except how a child name becomes an output
/// line: `Paths` emits the full grammar-escaped dot-path from the root,
/// `Keys` emits the raw child key name / array index.
enum EnumerateMode {
    Paths,
    Keys,
}

/// §1: enumerate the immediate children of the container at `key` (the
/// document's top level when `key` is omitted). Rejects a scalar target (the
/// dual of `value` rejecting a container target). `--missing-ok` swallows
/// only a `document_path_not_found`-coded failure into empty output + exit
/// 0; every other error still fails, matching `value`'s stdout-stays-empty
/// contract (R1).
fn run_enumerate(
    file: &Path,
    key: Option<&str>,
    input_format: Option<&str>,
    missing_ok: bool,
    null_separated: bool,
    format: OutputFormat,
    mode: EnumerateMode,
) -> ExitCode {
    let ctx = DocumentContext {
        input_format,
        secret_names: &[],
        format,
    };
    match compute_enumerate(file, key, &ctx, mode) {
        Ok(lines) => {
            let separator: u8 = if null_separated { 0 } else { b'\n' };
            let mut bytes = Vec::new();
            for line in &lines {
                bytes.extend_from_slice(line.as_bytes());
                bytes.push(separator);
            }
            write_raw_exit(&bytes)
        }
        Err(err) if missing_ok && is_path_not_found(&err) => write_raw_exit(&[]),
        Err(err) => {
            let (event, code) = document_error_event(&err);
            emit_document_event(event, &ctx, code, error_stream(output_to()))
        }
    }
}

fn compute_enumerate(
    file: &Path,
    key: Option<&str>,
    ctx: &DocumentContext<'_>,
    mode: EnumerateMode,
) -> Result<Vec<String>, CliDocError> {
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let (root, _doc_format) = read_document_input(file, input_format)?;
    let base_segments: Vec<String> = match key {
        Some(key) => parse_path(key)?,
        None => Vec::new(),
    };
    let target = match key {
        Some(key) => get_path(&root, key, &[])?,
        None => root,
    };
    let names: Vec<String> = match &target {
        DocumentValue::Object(map) => map.keys().cloned().collect(),
        DocumentValue::Array(items) => (0..items.len()).map(|index| index.to_string()).collect(),
        _ => {
            return Err(CliDocError::runtime(
                "document_not_container",
                format!(
                    "path `{}` is a scalar; nothing to enumerate",
                    key.unwrap_or("<root>")
                ),
            ));
        }
    };
    let mut lines = Vec::with_capacity(names.len());
    for name in names {
        match mode {
            EnumerateMode::Keys => lines.push(name),
            EnumerateMode::Paths => {
                let mut segments = base_segments.clone();
                segments.push(name);
                let path = join_path(&segments);
                // Never hand back an address this tool cannot then read. A
                // `""` key at the document root joins to the empty string,
                // which names no path at all — emitting it would repeat the
                // defect the empty-segment grammar exists to remove, one
                // level up. Say so instead of printing a blank line the
                // caller will feed back and be refused.
                if path.is_empty() {
                    return Err(CliDocError::runtime(
                        "document_unaddressable_key",
                        "the document root has an empty-string key, which has no path spelling; \
                         read its children by their own paths, or use `keys`"
                            .to_string(),
                    ));
                }
                lines.push(path);
            }
        }
    }
    Ok(lines)
}

/// `set`: write a value at dot-path `key` into `file`, preserving the rest
/// of the document's source formatting. See the `Set` variant's doc comment
/// for the bare-VALUE/`--value-type`/heterogeneous-overwrite-guard contract
/// (§3) and `--secret-from` (D4).
fn run_set(
    file: &Path,
    key: &str,
    value: Option<String>,
    value_type: Option<&str>,
    secret_from: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    finish_document(
        compute_set(file, key, value, value_type, secret_from, ctx),
        ctx,
    )
}

fn compute_set(
    file: &Path,
    key: &str,
    value: Option<String>,
    value_type: Option<&str>,
    secret_from: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> Result<Value, CliDocError> {
    reject_mutation_dash(file)?;
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let mut doc = DocumentFile::open(file, input_format)?;
    // Guard the target before consuming a `--secret-from stdin`/`prompt`/`fd:N`
    // source: an unsafe target (symlink, or on unix a hardlink) must be
    // rejected before the secret is read, not after.
    doc.ensure_mutable("set")?;

    let new_value = if let Some(secret_from) = secret_from {
        let source = parse_secret_from(secret_from).map_err(CliDocError::Usage)?;
        DocumentValue::String(read_secret_from(source)?)
    } else if let Some(type_name) = value_type {
        let parsed_type = ValueType::parse(type_name).ok_or_else(|| {
            CliDocError::Usage(format!(
                "invalid --value-type `{type_name}`; expected string, number, bool, null, or json"
            ))
        })?;
        value_from_type(parsed_type, value.as_deref())?
    } else {
        let raw = value.ok_or_else(|| {
            CliDocError::Usage(
                "set requires VALUE, --value-type null, or --secret-from".to_string(),
            )
        })?;
        // Heterogeneous-overwrite guard: a bare VALUE is always a string, so writing one over
        // anything that is not already a string rewrites what is there — an
        // argument error, not a coercion decision. Only a brand-new key or an
        // existing string is unguarded. Containers are included: replacing an
        // array with a string used to exit 0 and discard every element.
        let existing = get_path(doc.value(), key, &[]).ok();
        if let Err(overwrite) = guard_bare_overwrite(existing.as_ref()) {
            let verb = match overwrite {
                BareOverwrite::Container(_) => "discard",
                BareOverwrite::Scalar(_) => "change",
            };
            let keep_instruction = if overwrite.found() == "null" {
                "pass --value-type null without VALUE to keep it".to_string()
            } else {
                format!("pass --value-type {} to keep it", overwrite.keeps())
            };
            return Err(CliDocError::Usage(format!(
                "bare VALUE would silently {verb} the {found} at `{key}`; \
                 {keep_instruction}, or pass --value-type string to replace it explicitly",
                found = overwrite.found(),
            )));
        }
        DocumentValue::String(raw)
    };

    doc.set(key, new_value)?;
    doc.save()?;
    Ok(json!({
        "code": "document_set",
        "format": doc.format().cli_name(),
        "key": key,
        "path": doc.path().display().to_string(),
    }))
}

/// `add`: append an element to the keyed list at dot-path `key` in `file`,
/// preserving the rest of the document's source formatting. Idempotency:
/// adding a `slug` that already exists is an error (`document_slug_exists`).
fn run_add(
    file: &Path,
    key: &str,
    slug: &str,
    slug_field: &str,
    fields: &[String],
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    finish_document(compute_add(file, key, slug, slug_field, fields, ctx), ctx)
}

fn compute_add(
    file: &Path,
    key: &str,
    slug: &str,
    slug_field: &str,
    fields: &[String],
    ctx: &DocumentContext<'_>,
) -> Result<Value, CliDocError> {
    reject_mutation_dash(file)?;
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let mut field_pairs: Vec<(String, DocumentValue)> = Vec::with_capacity(fields.len());
    for field in fields {
        let Some((name, value)) = field.split_once('=') else {
            return Err(CliDocError::Usage(format!(
                "field `{field}` must use FIELD=VALUE"
            )));
        };
        if name.is_empty() {
            return Err(CliDocError::Usage(
                "field name must not be empty".to_string(),
            ));
        }
        // Close the side door: add's FIELD=VALUE is always a string, the same
        // zero-coercion rule as set's bare VALUE — no separate type syntax.
        field_pairs.push((name.to_string(), DocumentValue::String(value.to_string())));
    }
    let mut doc = DocumentFile::open(file, input_format)?;
    doc.add(key, slug, slug_field, &field_pairs)?;
    doc.save()?;
    Ok(json!({
        "code": "document_added",
        "format": doc.format().cli_name(),
        "key": key,
        "slug": slug,
        "path": doc.path().display().to_string(),
    }))
}

/// `remove`: delete the element identified by `slug`/`slug_field` from the
/// keyed list at dot-path `key` in `file`, preserving the rest of the
/// document's source formatting. Idempotency: removing a `slug` that does
/// not exist is an error (`document_slug_not_found`).
fn run_remove(
    file: &Path,
    key: &str,
    slug: &str,
    slug_field: &str,
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    finish_document(compute_remove(file, key, slug, slug_field, ctx), ctx)
}

fn compute_remove(
    file: &Path,
    key: &str,
    slug: &str,
    slug_field: &str,
    ctx: &DocumentContext<'_>,
) -> Result<Value, CliDocError> {
    reject_mutation_dash(file)?;
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let mut doc = DocumentFile::open(file, input_format)?;
    doc.remove(key, slug, slug_field)?;
    doc.save()?;
    Ok(json!({
        "code": "document_removed",
        "format": doc.format().cli_name(),
        "key": key,
        "slug": slug,
        "path": doc.path().display().to_string(),
    }))
}

/// `unset`: remove the entry at dot-path `key` entirely from `file`,
/// preserving the rest of the document's source formatting. Idempotency:
/// unsetting an absent `key` is an error (`document_path_not_found`).
fn run_unset(file: &Path, key: &str, ctx: &DocumentContext<'_>) -> ExitCode {
    finish_document(compute_unset(file, key, ctx), ctx)
}

fn compute_unset(file: &Path, key: &str, ctx: &DocumentContext<'_>) -> Result<Value, CliDocError> {
    reject_mutation_dash(file)?;
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let mut doc = DocumentFile::open(file, input_format)?;
    // `Document::unset` is idempotent, but the `afdata unset` CLI keeps its
    // strict contract: unsetting an absent key is a caught error, so scripts
    // can tell an actual removal from a no-op.
    if !doc.unset(key)? {
        return Err(DocumentError::PathNotFound {
            path: key.to_string(),
        }
        .into());
    }
    doc.save()?;
    Ok(json!({
        "code": "document_unset",
        "format": doc.format().cli_name(),
        "key": key,
        "path": doc.path().display().to_string(),
    }))
}

// ═══════════════════════════════════════════
// D4: --secret-from stdin|prompt|fd:<N>|env:<VAR>
// ═══════════════════════════════════════════

enum SecretSource {
    Stdin,
    Prompt,
    Fd(i32),
    Env(String),
}

/// Parse a `--secret-from` flag value. Descriptor/name shape problems
/// (non-numeric `fd:`, `fd:` below 3, empty `env:` name, an unrecognized
/// source keyword) are usage errors (R2) caught here, before anything is
/// read; a failure actually *reading* the source is a separate runtime
/// concern (see [`read_secret_from`]).
fn parse_secret_from(spec: &str) -> Result<SecretSource, String> {
    match spec {
        "stdin" => Ok(SecretSource::Stdin),
        "prompt" => Ok(SecretSource::Prompt),
        other => {
            if let Some(fd) = other.strip_prefix("fd:") {
                let number = fd.parse::<i32>().map_err(|_| {
                    format!("invalid --secret-from `{spec}`; fd: requires a numeric descriptor")
                })?;
                if number < 3 {
                    return Err(format!(
                        "invalid --secret-from `{spec}`; fd: requires a descriptor >= 3"
                    ));
                }
                Ok(SecretSource::Fd(number))
            } else if let Some(var) = other.strip_prefix("env:") {
                if var.is_empty() {
                    return Err(format!(
                        "invalid --secret-from `{spec}`; env: requires a variable name"
                    ));
                }
                Ok(SecretSource::Env(var.to_string()))
            } else {
                Err(format!(
                    "invalid --secret-from `{spec}`; expected stdin, prompt, fd:<N>, or env:<VAR>"
                ))
            }
        }
    }
}

const MAX_VALUE_SECRET_BYTES: usize = 1024 * 1024;

fn read_secret_from(source: SecretSource) -> Result<String, CliDocError> {
    match source {
        SecretSource::Stdin => read_secret_reader(std::io::stdin().lock(), "stdin"),
        SecretSource::Prompt => {
            #[cfg(unix)]
            {
                read_secret_prompt()
            }
            #[cfg(not(unix))]
            {
                Err(CliDocError::runtime(
                    "document_secret_source_failed",
                    "prompt secret input is unsupported on this platform",
                ))
            }
        }
        SecretSource::Fd(number) => {
            #[cfg(unix)]
            {
                use std::os::unix::io::FromRawFd;
                // SAFETY: ownership is transferred exactly once and the descriptor is closed on drop.
                let file = unsafe { std::fs::File::from_raw_fd(number) };
                read_secret_reader(file, "file descriptor")
            }
            #[cfg(not(unix))]
            {
                let _ = number;
                Err(CliDocError::runtime(
                    "document_secret_source_failed",
                    "raw file descriptors are unsupported on this platform",
                ))
            }
        }
        SecretSource::Env(name) => std::env::var(&name).map_err(|_| {
            CliDocError::runtime(
                "document_secret_source_failed",
                format!("environment variable `{name}` is not set"),
            )
        }),
    }
}

fn read_secret_reader<R: std::io::Read>(reader: R, source: &str) -> Result<String, CliDocError> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_VALUE_SECRET_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliDocError::runtime(
                "document_secret_source_failed",
                format!("read secret from {source}: {error}"),
            )
        })?;
    if bytes.len() > MAX_VALUE_SECRET_BYTES {
        return Err(CliDocError::runtime(
            "document_secret_source_failed",
            format!("secret exceeds {MAX_VALUE_SECRET_BYTES} bytes"),
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        CliDocError::runtime(
            "document_secret_source_failed",
            "secret input must be valid UTF-8",
        )
    })
}

#[cfg(unix)]
fn read_secret_prompt() -> Result<String, CliDocError> {
    use std::io::BufRead;
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|error| {
            CliDocError::runtime(
                "document_secret_source_failed",
                format!("open controlling terminal: {error}"),
            )
        })?;
    let status = std::process::Command::new("stty")
        .args(["-echo"])
        .status()
        .map_err(|error| {
            CliDocError::runtime(
                "document_secret_source_failed",
                format!("disable terminal echo: {error}"),
            )
        })?;
    if !status.success() {
        return Err(CliDocError::runtime(
            "document_secret_source_failed",
            "disable terminal echo failed",
        ));
    }
    let _echo_guard = TerminalEchoGuard;
    write!(tty, "Secret: ").map_err(|error| {
        CliDocError::runtime(
            "document_secret_source_failed",
            format!("write prompt: {error}"),
        )
    })?;
    let mut value = String::new();
    let result = {
        let mut reader = std::io::BufReader::new(&mut tty);
        reader.read_line(&mut value)
    }
    .map_err(|error| {
        CliDocError::runtime(
            "document_secret_source_failed",
            format!("read secret from prompt: {error}"),
        )
    });
    let _ = writeln!(tty);
    result?;
    let value = value.trim_end_matches(['\n', '\r']);
    if value.len() > MAX_VALUE_SECRET_BYTES {
        return Err(CliDocError::runtime(
            "document_secret_source_failed",
            format!("secret exceeds {MAX_VALUE_SECRET_BYTES} bytes"),
        ));
    }
    Ok(value.to_string())
}

#[cfg(unix)]
struct TerminalEchoGuard;

#[cfg(unix)]
impl Drop for TerminalEchoGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("stty").arg("echo").status();
    }
}

// ═══════════════════════════════════════════
// Lint rules (deterministic AFDATA naming/suffix checks)
// ═══════════════════════════════════════════

fn lint_value(value: &Value, pointer: &str, findings: &mut Vec<Finding>) {
    lint_unsafe_integer(value, pointer, findings);
    match value {
        Value::Object(map) => {
            if let Some(Value::Object(properties)) = map.get("properties") {
                for (name, schema) in properties {
                    lint_schema_property(
                        name,
                        schema,
                        &join_pointer(pointer, "properties"),
                        findings,
                    );
                }
            }
            for (key, child) in map {
                // A JSON Schema `properties` object maps public field names to
                // schema descriptors. Those descriptors are not runtime field
                // values, so applying `lint_suffix_type` to the descriptor
                // object itself would reject every correctly typed suffix
                // property (for example `duration_ms: {"type":"integer"}`).
                // Each property is checked by `lint_schema_property` above and
                // its descriptor is recursively inspected there.
                if key == "properties" && child.is_object() {
                    continue;
                }
                lint_suffix_type(key, child, &join_pointer(pointer, key), findings);
                lint_missing_suffix(key, child, &join_pointer(pointer, key), findings);
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

fn lint_schema_property(
    name: &str,
    schema: &Value,
    properties_pointer: &str,
    findings: &mut Vec<Finding>,
) {
    let property_pointer = join_pointer(properties_pointer, name);
    let normalized_name = name.to_ascii_lowercase();
    // A JSON Schema is always an object or a boolean. A scalar or array here
    // means the enclosing `properties` object is runtime data that happens to
    // use `properties` as a field name, not a property map, so the value keeps
    // the ordinary runtime suffix check.
    if !matches!(schema, Value::Object(_) | Value::Bool(_)) {
        lint_suffix_type(name, schema, &property_pointer, findings);
        lint_value(schema, &property_pointer, findings);
        return;
    }
    if normalized_name.ends_with("_secret")
        && let Some(obj) = schema.as_object()
    {
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
    lint_schema_suffix_type(name, schema, &property_pointer, findings);
    lint_value(schema, &property_pointer, findings);
}

fn lint_schema_suffix_type(name: &str, schema: &Value, pointer: &str, findings: &mut Vec<Finding>) {
    let normalized = name.to_ascii_lowercase();
    let (expected, description): (&[&str], &str) = if normalized.ends_with("_bytes") {
        (&["integer"], "an integer byte count")
    } else if normalized.ends_with("_epoch_s") || normalized.ends_with("_epoch_ms") {
        (&["integer"], "an integer epoch timestamp")
    } else if normalized.ends_with("_epoch_ns") {
        (&["string"], "a decimal integer string")
    } else if normalized.ends_with("_sats") || normalized.ends_with("_msats") {
        (
            &["integer", "string"],
            "an integer or decimal integer string",
        )
    } else if normalized.ends_with("_percent") || is_duration_suffix(&normalized) {
        (&["integer", "number"], "a numeric value")
    } else if is_currency_minor_unit_suffix(&normalized) {
        (&["integer"], "an integer currency amount")
    } else if normalized.ends_with("_rfc3339")
        || normalized.ends_with("_url")
        || normalized.ends_with("_bcp47")
        || normalized.ends_with("_rfc3339_date")
        || normalized.ends_with("_rfc3339_time")
        || normalized.ends_with("_utc_offset")
    {
        (&["string"], "a string")
    } else {
        return;
    };

    if !schema_accepts_any_type(schema, expected) {
        findings.push(Finding::error(
            "suffix_type_mismatch",
            join_pointer(pointer, "type"),
            format!("schema property {name:?} must allow {description}"),
        ));
    }
}

fn schema_accepts_any_type(schema: &Value, expected: &[&str]) -> bool {
    let Some(object) = schema.as_object() else {
        // Boolean schemas and unresolved references do not declare a
        // contradictory primitive type at this location.
        return true;
    };

    if let Some(schema_type) = object.get("type") {
        return match schema_type {
            Value::String(value) => expected.contains(&value.as_str()),
            Value::Array(values) => values.iter().any(|value| {
                value
                    .as_str()
                    .is_some_and(|value| expected.contains(&value))
            }),
            _ => true,
        };
    }

    for keyword in ["anyOf", "oneOf"] {
        if let Some(Value::Array(branches)) = object.get(keyword) {
            return branches
                .iter()
                .any(|branch| schema_accepts_any_type(branch, expected));
        }
    }
    if let Some(Value::Array(branches)) = object.get("allOf") {
        return branches
            .iter()
            .all(|branch| schema_accepts_any_type(branch, expected));
    }

    // With no local primitive constraint, a `$ref`, `const`, or other schema
    // keyword may still admit the required type. Do not invent a mismatch.
    true
}

fn is_redacted_secret_literal(value: &Value) -> bool {
    matches!(value, Value::Null) || matches!(value, Value::String(s) if s == "***")
}

/// Every suffix `spec/registry.json` defines, by category.
///
/// Kept in step with the registry by `registry_suffixes_match_the_table`
/// rather than by hand — a suffix added there and missed here would silently
/// turn correctly-named fields into warnings.
const REGISTERED_SUFFIXES: &[(&str, &[&str])] = &[
    (
        "duration",
        &["_ns", "_us", "_ms", "_s", "_minutes", "_hours", "_days"],
    ),
    (
        "timestamp",
        &["_epoch_s", "_epoch_ms", "_epoch_ns", "_rfc3339"],
    ),
    (
        "strict_string",
        &["_rfc3339_date", "_rfc3339_time", "_bcp47", "_utc_offset"],
    ),
    ("size", &["_bytes"]),
    ("percentage", &["_percent"]),
    (
        "currency",
        &[
            "_msats",
            "_sats",
            "_usd_cents",
            "_eur_cents",
            "_jpy",
            "_{code}_cents",
            "_{code}_micro",
        ],
    ),
    ("sensitive", &["_secret", "_url"]),
];

/// Field-name stems that name a dimension the convention has a suffix for.
///
/// Matched against the whole key or its trailing `_`-separated tail, so
/// `request_timeout` counts and `timeout_ms` does not (it already carries one).
const UNSUFFIXED_STEMS: &[(&str, &str)] = &[
    ("timeout", "duration"),
    ("elapsed", "duration"),
    ("duration", "duration"),
    ("ttl", "duration"),
    ("interval", "duration"),
    ("latency", "duration"),
    ("delay", "duration"),
    ("uptime", "duration"),
    ("price", "currency"),
    ("amount", "currency"),
    ("cost", "currency"),
    ("fee", "currency"),
    ("balance", "currency"),
    ("subtotal", "currency"),
    ("revenue", "currency"),
    ("created", "timestamp"),
    ("updated", "timestamp"),
    ("modified", "timestamp"),
    ("expires", "timestamp"),
    ("issued", "timestamp"),
    ("timestamp", "timestamp"),
    ("apikey", "sensitive"),
    ("api_key", "sensitive"),
    ("token", "sensitive"),
    ("password", "sensitive"),
    ("passwd", "sensitive"),
    ("secret", "sensitive"),
    ("credential", "sensitive"),
    ("credentials", "sensitive"),
];

fn has_registered_suffix(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    REGISTERED_SUFFIXES
        .iter()
        .flat_map(|(_, suffixes)| suffixes.iter())
        .any(|suffix| match suffix.strip_prefix("_{code}") {
            // `_{code}_cents` / `_{code}_micro` are templated on a currency
            // code, so match the shape rather than the literal.
            Some(tail) => lower
                .strip_suffix(tail)
                .and_then(|rest| rest.rsplit_once('_'))
                .is_some_and(|(_, code)| {
                    code.len() == 3 && code.chars().all(|c| c.is_ascii_alphabetic())
                }),
            None => lower.ends_with(suffix),
        })
}

/// Flag a field whose name announces a dimension but carries no unit.
///
/// This is the question the convention exists to answer — is `timeout` seconds
/// or milliseconds, is `price` dollars or cents, is `created` an id or a date,
/// does `api_key` get redacted — and until now `lint` only checked fields that
/// had already answered it, so a document made entirely of these passed clean.
fn lint_missing_suffix(key: &str, value: &Value, pointer: &str, findings: &mut Vec<Finding>) {
    if value.is_null() || has_registered_suffix(key) {
        return;
    }
    let lower = key.to_ascii_lowercase();
    // `_at` is the common spelling for "this is a time", and it names no unit.
    let category = if lower.ends_with("_at") {
        Some("timestamp")
    } else {
        UNSUFFIXED_STEMS
            .iter()
            .find(|(stem, _)| lower == *stem || lower.ends_with(&format!("_{stem}")))
            .map(|(_, category)| *category)
    };
    let Some(category) = category else {
        return;
    };
    // Only complain where the value could actually carry the dimension: a
    // `timeout` object is a config block, not an unlabelled number.
    let plausible = match category {
        "duration" | "currency" | "size" | "percentage" => value.is_number(),
        "timestamp" => value.is_number() || value.is_string(),
        _ => value.is_string(),
    };
    if !plausible {
        return;
    }
    let suffixes = REGISTERED_SUFFIXES
        .iter()
        .find(|(name, _)| *name == category)
        .map(|(_, suffixes)| suffixes.join(", "))
        .unwrap_or_default();
    let message = if category == "sensitive" {
        format!(
            "`{key}` looks like a credential but is not marked, so it is printed and logged in \
             the clear. Rename it with one of: {suffixes}"
        )
    } else {
        format!(
            "`{key}` names a {category} but carries no unit, so a reader cannot tell what it \
             means without asking. Rename it with one of: {suffixes}"
        )
    };
    findings.push(Finding::warning(
        "missing_suffix",
        pointer.to_string(),
        message,
    ));
}

fn lint_suffix_type(key: &str, value: &Value, pointer: &str, findings: &mut Vec<Finding>) {
    // `null` means the field is absent/unset, not present-with-the-wrong-type:
    // the suffix type constraint below applies only to present, non-null
    // values. Absence may be expressed by omitting the key entirely or by an
    // explicit `null`; both are valid. This mirrors `is_redacted_secret_literal`,
    // which already treats `Value::Null` as a valid absent literal for
    // `_secret` schema properties.
    if value.is_null() {
        return;
    }
    let normalized = key.to_ascii_lowercase();
    let message = if normalized.ends_with("_bytes") {
        if is_non_negative_integer(value) {
            None
        } else {
            Some(format!("{key:?} must be a non-negative integer byte count"))
        }
    } else if normalized.ends_with("_epoch_s") || normalized.ends_with("_epoch_ms") {
        if is_integer(value) {
            None
        } else {
            Some(format!("{key:?} must be an integer epoch timestamp"))
        }
    } else if normalized.ends_with("_epoch_ns") {
        if is_decimal_integer_string(value) {
            None
        } else {
            Some(format!("{key:?} must be a decimal integer string"))
        }
    } else if normalized.ends_with("_sats") || normalized.ends_with("_msats") {
        if is_integer(value) || is_decimal_integer_string(value) {
            None
        } else {
            Some(format!(
                "{key:?} must be an integer or decimal integer string"
            ))
        }
    } else if normalized.ends_with("_percent") {
        if value.is_number() {
            None
        } else {
            Some(format!("{key:?} must be numeric"))
        }
    } else if is_duration_suffix(&normalized) {
        if value.is_number() {
            None
        } else {
            Some(format!("{key:?} must be a numeric duration"))
        }
    } else if is_currency_minor_unit_suffix(&normalized) {
        if is_integer(value) {
            None
        } else {
            Some(format!("{key:?} must be an integer currency amount"))
        }
    } else if normalized.ends_with("_rfc3339") {
        if value.as_str().is_some_and(is_valid_rfc3339) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 date-time with a mandatory offset (e.g. 2026-02-14T10:30:00Z)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if normalized.ends_with("_url") {
        if value.as_str().is_some_and(is_wellformed_url_field) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be a single URL (no internal whitespace or bare credentials)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if normalized.ends_with("_bcp47") {
        if value.as_str().is_some_and(is_valid_bcp47) {
            None
        } else if value.is_string() {
            Some(format!("{key:?} must be a well-formed BCP 47 language tag"))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if normalized.ends_with("_rfc3339_date") {
        if value.as_str().is_some_and(is_valid_rfc3339_date) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 full-date (YYYY-MM-DD)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if normalized.ends_with("_rfc3339_time") {
        if value.as_str().is_some_and(is_valid_rfc3339_time) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 partial-time (HH:MM:SS[.fraction], no Z or offset)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if normalized.ends_with("_utc_offset") {
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

#[cfg(test)]
mod cli_registry_tests {
    use super::*;

    /// The missing-suffix heuristic decides a field is unlabelled by *not*
    /// finding a registered suffix on it. A suffix added to the registry and
    /// missed here would start warning about correctly-named fields, so the two
    /// are compared rather than trusted.
    #[test]
    fn registry_suffixes_match_the_table() {
        const REGISTRY: &str =
            include_str!("../../skills/agent-first-data/references/registry.json");
        let registry: Value = serde_json::from_str(REGISTRY).unwrap();
        let mut from_registry: Vec<(String, String)> = registry["suffixes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                (
                    entry["category"].as_str().unwrap().to_string(),
                    entry["suffix"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let mut from_table: Vec<(String, String)> = REGISTERED_SUFFIXES
            .iter()
            .flat_map(|(category, suffixes)| {
                suffixes
                    .iter()
                    .map(move |suffix| ((*category).to_string(), (*suffix).to_string()))
            })
            .collect();
        from_registry.sort();
        from_table.sort();
        assert_eq!(from_table, from_registry);
    }

    #[test]
    fn missing_suffix_reads_intent_from_the_name() {
        let mut findings = Vec::new();
        lint_missing_suffix("timeout", &json!(5000), "/timeout", &mut findings);
        lint_missing_suffix(
            "request_timeout",
            &json!(1),
            "/request_timeout",
            &mut findings,
        );
        lint_missing_suffix("expires_at", &json!(1), "/expires_at", &mut findings);
        assert_eq!(findings.len(), 3);
        assert!(findings.iter().all(|finding| finding.severity == "warning"));

        // Already labelled, so there is nothing to ask about.
        let mut clean = Vec::new();
        lint_missing_suffix("timeout_ms", &json!(5000), "/timeout_ms", &mut clean);
        lint_missing_suffix("price_gbp_cents", &json!(1), "/price_gbp_cents", &mut clean);
        lint_missing_suffix(
            "api_key_secret",
            &json!("sk"),
            "/api_key_secret",
            &mut clean,
        );
        // A dimension name over a container is a config block, not a bare number.
        lint_missing_suffix("timeout", &json!({"connect": 1}), "/timeout", &mut clean);
        // An unrelated name says nothing about a dimension.
        lint_missing_suffix("name", &json!("demo"), "/name", &mut clean);
        assert!(clean.is_empty(), "{clean:?}");
    }

    #[test]
    fn every_registered_afdata_shape_round_trips() {
        let cli = afdata_cli_spec().unwrap();
        let fixtures = cli.synthetic_invocations();
        assert!(!fixtures.is_empty());
        for fixture in fixtures {
            let outcome = cli.resolve_from(fixture.argv).unwrap();
            let CliOutcome::Run(invocation) = outcome else {
                panic!("synthetic afdata fixture did not resolve to Run");
            };
            assert_eq!(invocation.combination_id(), fixture.combination_id);
        }
    }

    #[test]
    fn set_conflicts_are_closed_world_errors() {
        let cli = afdata_cli_spec().unwrap();
        let error = cli
            .resolve_from([
                "afdata",
                "set",
                "data.json",
                "key",
                "visible",
                "--secret-from",
                "env:VALUE_SECRET",
            ])
            .unwrap_err();
        assert_eq!(
            error.rule,
            agent_first_data::CliErrorRule::UnregisteredCombination
        );
        assert!(!error.message.contains("VALUE_SECRET"));
    }

    #[test]
    fn raw_commands_reject_protocol_output_arguments() {
        let cli = afdata_cli_spec().unwrap();
        for command in ["value", "paths", "keys"] {
            let mut argv = vec!["afdata", command, "data.json"];
            if command == "value" {
                argv.push("key");
            }
            argv.extend(["--output", "json"]);
            let error = cli.resolve_from(argv).unwrap_err();
            assert_eq!(
                error.rule,
                agent_first_data::CliErrorRule::UnregisteredCombination
            );
        }
    }
}

#[cfg(all(test, feature = "skill"))]
mod skill_tests {
    use super::*;

    #[test]
    fn parses_and_validates_skill_directory() {
        let cli = afdata_cli_spec().unwrap();
        let parsed = cli
            .resolve_from(["afdata", "skill", "validate", "skills/agent-first-data"])
            .unwrap();
        let CliOutcome::Run(invocation) = parsed else {
            panic!("expected a resolved skill validation");
        };
        assert_eq!(invocation.action_id(), "skill_validate");
        assert_eq!(
            invocation.required("input").as_str(),
            Some("skills/agent-first-data")
        );

        let loaded = read_skill_input(Path::new("skills/agent-first-data"));
        assert!(matches!(
            loaded,
            Ok((text, Some(expected_name), _))
                if expected_name == "agent-first-data"
                    && agent_first_data::skill::validate_skill_named(&text, &expected_name).is_ok()
        ));
    }

    #[cfg(feature = "skill-admin")]
    #[test]
    fn resolves_registered_skill_admin_combination() {
        let cli = afdata_cli_spec().unwrap();
        let parsed = cli
            .resolve_from([
                "afdata",
                "skill",
                "status",
                "--agent",
                "opencode",
                "--scope",
                "workspace",
            ])
            .unwrap();
        let CliOutcome::Run(invocation) = parsed else {
            panic!("expected a resolved skill status");
        };
        assert_eq!(invocation.combination_id(), "skill-status-agent");
        assert_eq!(invocation.required("agent").as_str(), Some("opencode"));
        assert_eq!(invocation.required("scope").as_str(), Some("workspace"));
    }

    #[cfg(feature = "skill-admin")]
    #[test]
    fn admin_subcommands_reject_a_validate_only_input_path() {
        let cli = afdata_cli_spec().unwrap();
        let parsed = cli.resolve_from(["afdata", "skill", "status", "some/path"]);
        assert!(parsed.is_err());
    }
}
