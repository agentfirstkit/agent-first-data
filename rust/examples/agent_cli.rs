#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    unused_imports
)]

// Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.
//
// Demonstrates: human `--help` (one-level) plus orthogonal `--recursive`
// scope and `--output markdown|json|yaml` format for full surface export,
// _secret flags, nested subcommands, cli_parse_output, cli_parse_log_filters,
// opt-in startup diagnostics, render, build_cli_error, error hints, and
// (with the `skill-admin` feature) a `skill` subcommand that installs/uninstalls/
// reports status of an embedded Agent Skill across Codex, Claude Code, opencode, and Hermes.
//
// Run:  cargo run --example agent_cli --features cli-help,cli-help-markdown -- --help
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- service --help
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- service start --help
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- --help --recursive
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- --help --recursive --output markdown
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- ping --timeout-ms 5000
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- --log startup ping --host example.com
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- --log all ping   # or --verbose
//       cargo run --example agent_cli --features cli-help,cli-help-markdown,stream-redirect -- --stdout-file /tmp/agent-cli.out --stderr-file /tmp/agent-cli.err ping
//       cargo run --example agent_cli --features cli-help,cli-help-markdown,skill-admin -- skill status --agent opencode --skills-dir /tmp/ex
//       cargo run --example agent_cli --features cli-help,cli-help-markdown,skill-admin -- skill install --agent opencode --skills-dir /tmp/ex
// Test: cargo test --examples --features cli-help,cli-help-markdown
//       cargo test --examples --features cli-help,cli-help-markdown,skill-admin

use agent_first_data::{
    CliEmitter, CliEmitterError, Event, OutputFormat, OutputOptions, build_cli_error,
    cli_parse_log_filters, cli_parse_output, json_error, json_result, render,
};
#[cfg(feature = "cli-help")]
use clap::CommandFactory;
use clap::{Parser, Subcommand};
use std::io::Write;

// Human-facing help/version TEXT (not protocol envelopes) prints to stdout.
macro_rules! stdout {
    ($($arg:tt)*) => {{
        write_stdout_or_exit(&format!($($arg)*));
    }};
}

fn write_stdout_or_exit(text: &str) {
    let mut stdout = std::io::stdout().lock();
    if let Err(err) = stdout.write_all(text.as_bytes()) {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        std::process::exit(1);
    }
}

// Emit one standalone error envelope through a finite emitter and exit, using
// the library's `finish()` to map the outcome to a process exit code (broken
// pipe → 0, other write failure → 4). Used for pre-command/usage errors that
// occur before the main emitter is built.
fn emit_error_exit(format: OutputFormat, event: Event, code: u8) -> ! {
    std::process::exit(CliEmitter::finite(format).finish(event, code).into())
}

// As `emit_error_exit`, for a help error already shaped as a JSON value.
// `finish()` takes a typed Event, so the value path maps the emit outcome here.
fn emit_value_error_exit(format: OutputFormat, event: serde_json::Value, code: u8) -> ! {
    let mut emitter = CliEmitter::finite(format);
    let exit = match emitter.emit_validated_value(event) {
        Ok(()) => code,
        Err(err) if err.io_error_kind() == Some(std::io::ErrorKind::BrokenPipe) => 0,
        Err(_) => 4,
    };
    std::process::exit(exit.into())
}

// Turn a library `finish*` exit code into a process exit.
fn exit_with(code: u8) -> ! {
    std::process::exit(code.into())
}

const AGENT_CLI_HOST_ENV: &str = "AGENT_CLI_HOST";
const STARTUP_ENV_KEYS: &[&str] = &[AGENT_CLI_HOST_ENV];

// A fictional spore's embedded Agent Skill, used by the `skill` subcommand to
// demonstrate `agent_first_data::skill::run_skill_admin`.
#[cfg(feature = "skill-admin")]
const WIDGET_SKILL: &str = "---\nname: agent-first-widget\ndescription: Example skill bundled by the agent-cli demo.\n---\n\n# Agent-First Widget\n\nExample behavior rules go here.\n";

#[cfg(feature = "skill-admin")]
const WIDGET_SPEC: agent_first_data::skill::SkillSpec = agent_first_data::skill::SkillSpec {
    name: "agent-first-widget",
    source: WIDGET_SKILL,
    title: "Agent-First Widget",
    marker_slug: "afwidget",
};

#[derive(Parser)]
#[command(
    name = env!("DISPLAY_NAME"),
    bin_name = "agent-cli",
    version = env!("CARGO_PKG_VERSION"),
    about = env!("CARGO_PKG_DESCRIPTION"),
    long_about = r#"### Interface Policy

- Agent-facing CLI surfaces should keep stdout structured.
- Human help remains conventional unless `--output markdown|json|yaml` is requested.
- Binary names are command-specific, so this example sets `name = "agent-cli"` explicitly.

### Example Usage

```text
agent-cli ping --host example.com
agent-cli --help --recursive --output markdown
```
"#,
    disable_help_subcommand = true
)]
struct Cli {
    /// Output format: json (default), yaml, plain; help also accepts markdown
    #[arg(long, default_value = "json")]
    output: String,

    /// Equivalent to --output json; conflicts with other explicit output formats
    #[arg(long)]
    json: bool,

    /// Log categories (comma-separated). Use `--log all` (or --verbose) to
    /// enable every category and discover them from the tagged output.
    #[arg(long, value_delimiter = ',')]
    log: Vec<String>,

    /// Enable all log categories (shorthand for `--log all`)
    #[arg(long)]
    verbose: bool,

    /// Redirect stdout to this file
    #[cfg(feature = "stream-redirect")]
    #[arg(long, value_name = "PATH", global = true)]
    stdout_file: Option<std::path::PathBuf>,

    /// Redirect stderr to this file
    #[cfg(feature = "stream-redirect")]
    #[arg(long, value_name = "PATH", global = true)]
    stderr_file: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Service operations
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Ping a remote target
    Ping {
        /// Target host to ping (falls back to AGENT_CLI_HOST)
        #[arg(long)]
        host: Option<String>,
        /// Ping timeout
        #[arg(long, default_value = "5000")]
        timeout_ms: u64,
    },
    /// Return a tool-defined cancellation error
    Cancel,
    /// Manage this tool's embedded Agent Skill (Codex, Claude Code, opencode, Hermes)
    #[cfg(feature = "skill-admin")]
    Skill {
        #[command(subcommand)]
        action: SkillCmd,
    },
}

#[cfg(feature = "skill-admin")]
#[derive(Subcommand)]
enum SkillCmd {
    /// Show whether the skill is installed, valid, and up to date
    Status(SkillTargetArgs),
    /// Install or refresh the skill
    Install(SkillWriteArgs),
    /// Remove a managed skill
    Uninstall(SkillWriteArgs),
}

#[cfg(feature = "skill-admin")]
#[derive(clap::Args)]
struct SkillTargetArgs {
    /// Agent to manage: all, codex, claude-code, opencode, hermes
    #[arg(long, default_value = "all")]
    agent: String,
    /// Skill scope: personal, workspace
    #[arg(long, default_value = "personal")]
    scope: String,
    /// Skills directory (requires a single concrete --agent)
    #[arg(long)]
    skills_dir: Option<String>,
}

#[cfg(feature = "skill-admin")]
#[derive(clap::Args)]
struct SkillWriteArgs {
    #[command(flatten)]
    target: SkillTargetArgs,
    /// Overwrite or remove a skill this tool did not manage
    #[arg(long)]
    force: bool,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,
    /// Set a configuration value
    Set {
        /// Configuration key
        #[arg(long)]
        key: String,
        /// Configuration value
        #[arg(long)]
        value: String,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Start the service
    Start {
        /// Listen port
        #[arg(long, default_value = "8080")]
        port: u16,
        /// API authentication key (redacted in logs)
        #[arg(long)]
        api_key_secret: Option<String>,
    },
    /// Stop the service
    Stop,
    /// Show service status
    Status,
}

fn main() {
    let raw: Vec<String> = std::env::args().collect();

    #[cfg(feature = "stream-redirect")]
    let _stream_redirect =
        match agent_first_data::stream_redirect::install_from_raw_args(raw.clone()) {
            Ok(installed) => installed,
            Err(err) => emit_error_exit(
                OutputFormat::Json,
                build_cli_error(&err.to_string(), None),
                2,
            ),
        };

    match agent_first_data::cli_handle_version_or_continue(
        &raw,
        "agent-cli",
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(Some(version)) => {
            stdout!("{version}");
            std::process::exit(0);
        }
        Ok(None) => {}
        Err(err) => emit_error_exit(OutputFormat::Json, err, 2),
    }

    // Handle help before clap so `--help --output markdown` can work. Help TEXT
    // prints to stdout; a help *error* is an error envelope on stderr.
    #[cfg(feature = "cli-help")]
    {
        match agent_first_data::cli_handle_help_or_continue(
            &raw,
            &Cli::command(),
            &agent_first_data::HelpConfig::human_cli_default(),
        ) {
            Ok(Some(help)) => {
                stdout!("{help}");
                std::process::exit(0);
            }
            Ok(None) => {}
            Err(err) => emit_value_error_exit(OutputFormat::Json, err, 2),
        }
    }

    // try_parse — a clap error becomes a structured error envelope (on stderr),
    // never raw clap text.
    let cli = Cli::try_parse().unwrap_or_else(|e| {
        if matches!(
            e.kind(),
            clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp
        ) {
            e.exit();
        }
        emit_error_exit(
            OutputFormat::Json,
            build_cli_error(&e.to_string(), Some("try: agent-cli --help")),
            2,
        )
    });
    #[cfg(feature = "stream-redirect")]
    let _stream_redirect_args = (&cli.stdout_file, &cli.stderr_file);

    // Parse --output/--json and --log
    let output = resolve_output(&cli.output, cli.json).unwrap_or_else(|e| {
        emit_error_exit(
            OutputFormat::Json,
            build_cli_error(&e, Some("valid values: json, yaml, plain")),
            2,
        )
    });
    let format = cli_parse_output(&output).unwrap_or_else(|e| {
        emit_error_exit(
            OutputFormat::Json,
            build_cli_error(&e, Some("valid values: json, yaml, plain")),
            2,
        )
    });
    let log = if cli.verbose {
        // --verbose is shorthand for --log all.
        let mut entries: Vec<String> = cli.log.clone();
        entries.push("all".to_string());
        cli_parse_log_filters(&entries)
    } else {
        cli_parse_log_filters(&cli.log)
    };

    // One finite emitter for the command: `result` → stdout, `error`/`log` →
    // stderr, per the AFDATA output-stream contract.
    let mut emitter = CliEmitter::finite(format);

    // Each diagnostic line self-tags with its `category`, so `--log all` reveals
    // the full set from real output. Diagnostics land on stderr.
    if log.enabled("request") {
        let _ = emitter.emit(build_request_log(cli.command.as_ref()));
    }
    if log.enabled("startup") {
        let _ = emitter.emit(build_startup_log(
            cli.command.as_ref(),
            &output,
            &log,
            cli.verbose,
        ));
    }

    match cli.command {
        // Error → build via the error builder, hand the event to finish().
        None => exit_with(emitter.finish(
            build_cli_error("no subcommand provided", Some("try: agent-cli --help")),
            2,
        )),
        // Result → finish_result (broken-pipe-safe, returns 0 on success).
        Some(Command::Config { action }) => match action {
            ConfigAction::Show => {
                exit_with(emitter.finish_result(serde_json::json!({"action": "config_show"})))
            }
            ConfigAction::Set { key, value } => exit_with(emitter.finish_result(
                serde_json::json!({"action": "config_set", "key": key, "value": value}),
            )),
        },
        Some(Command::Service { action }) => match action {
            ServiceAction::Start {
                port,
                api_key_secret,
            } => exit_with(emitter.finish_result(
                serde_json::json!({"action": "service_start", "port": port, "api_key_secret": api_key_secret}),
            )),
            ServiceAction::Stop => {
                exit_with(emitter.finish_result(serde_json::json!({"action": "service_stop"})))
            }
            ServiceAction::Status => {
                exit_with(emitter.finish_result(serde_json::json!({"action": "service_status"})))
            }
        },
        Some(Command::Ping { host, timeout_ms }) => {
            let host = host.or_else(|| std::env::var(AGENT_CLI_HOST_ENV).ok());
            if host.is_none() {
                // Rich error (hint + extra field) → build the event, finish it.
                let err = json_error("ping_target_not_configured", "ping target not configured")
                    .hint("pass --host or set AGENT_CLI_HOST")
                    .field("duration_ms", serde_json::json!(0))
                    .build()
                    .expect("error builder failed");
                exit_with(emitter.finish(err, 1));
            }
            exit_with(emitter.finish_result(
                serde_json::json!({"action": "ping", "host": host, "timeout_ms": timeout_ms}),
            ))
        }
        Some(Command::Cancel) => {
            let err = json_error("cancelled", "operation cancelled")
                .hint("the operation was cancelled before completion")
                .field("duration_ms", serde_json::json!(0))
                .build()
                .expect("error builder failed");
            exit_with(emitter.finish(err, 1))
        }
        #[cfg(feature = "skill-admin")]
        Some(Command::Skill { action }) => {
            std::process::exit(run_skill(&mut emitter, action));
        }
    }
}

fn command_label(command: Option<&Command>) -> &'static str {
    match command {
        None => "none",
        Some(Command::Config { .. }) => "config",
        Some(Command::Service { .. }) => "service",
        Some(Command::Ping { .. }) => "ping",
        Some(Command::Cancel) => "cancel",
        #[cfg(feature = "skill-admin")]
        Some(Command::Skill { .. }) => "skill",
    }
}

fn resolve_output(output: &str, json: bool) -> Result<String, String> {
    if json && output != "json" {
        return Err(format!(
            "conflicting output formats: --json conflicts with --output {output}"
        ));
    }
    if json {
        Ok("json".to_string())
    } else {
        Ok(output.to_string())
    }
}

fn build_request_log(command: Option<&Command>) -> Event {
    agent_first_data::json_log(serde_json::json!({
        "level": "info",
        "message": "request",
        "category": "request",
        "command": command_label(command),
    }))
    .build()
}

fn build_startup_log(
    command: Option<&Command>,
    output: &str,
    log: &agent_first_data::LogFilters,
    verbose: bool,
) -> Event {
    agent_first_data::json_log(serde_json::json!({
        "level": "info",
        "message": "startup",
        "category": "startup",
        "event": "startup",
        "parsed": {
                "command": command_label(command),
                "output": output,
                "log": log.as_slice(),
                "verbose": verbose,
        },
        "effective_config": {
                "output": output,
                "log": log.as_slice(),
        },
        "env": startup_env_snapshot(),
    }))
    .build()
}

fn startup_env_snapshot() -> serde_json::Value {
    serde_json::Value::Array(
        STARTUP_ENV_KEYS
            .iter()
            .map(|key| {
                serde_json::json!({
                    "key": key,
                    "present": std::env::var_os(*key).is_some(),
                    "value": std::env::var(*key).ok(),
                })
            })
            .collect(),
    )
}

// Wire the parsed `skill` subcommand to the library and print the result. Returns
// the process exit code (0 ok, 1 action error, 2 bad flag value).
#[cfg(feature = "skill-admin")]
fn run_skill(emitter: &mut CliEmitter<std::io::Stdout>, action: SkillCmd) -> i32 {
    use agent_first_data::skill::{self, SkillAction};
    let (verb, target, force) = match action {
        SkillCmd::Status(target) => (SkillAction::Status, target, false),
        SkillCmd::Install(write) => (SkillAction::Install, write.target, write.force),
        SkillCmd::Uninstall(write) => (SkillAction::Uninstall, write.target, write.force),
    };
    let options = match build_skill_options(target, force) {
        Ok(options) => options,
        Err((message, hint)) => {
            let _ = emitter.emit(build_cli_error(&message, Some(&hint)));
            return 2;
        }
    };
    match skill::run_skill_admin(&WIDGET_SPEC, verb, &options) {
        Ok(report) => match serde_json::to_value(&report) {
            Ok(value) => {
                let _ = emitter.emit_result(value);
                0
            }
            Err(e) => {
                let _ = emitter.emit(build_cli_error(&e.to_string(), None));
                1
            }
        },
        Err(err) => {
            let _ = emitter.emit(build_cli_error(&err.message, err.hint.as_deref()));
            1
        }
    }
}

// Parse the `--agent`/`--scope` string flags into the library enums. Returns
// `(message, hint)` on an unknown value so the caller can emit a CLI error.
#[cfg(feature = "skill-admin")]
fn build_skill_options(
    target: SkillTargetArgs,
    force: bool,
) -> Result<agent_first_data::skill::SkillOptions, (String, String)> {
    use agent_first_data::skill::{SkillAgentSelection, SkillOptions, SkillScope};
    let agent = match target.agent.as_str() {
        "all" => SkillAgentSelection::All,
        "codex" => SkillAgentSelection::Codex,
        "claude-code" => SkillAgentSelection::ClaudeCode,
        "opencode" => SkillAgentSelection::Opencode,
        "hermes" => SkillAgentSelection::Hermes,
        other => {
            return Err((
                format!("invalid --agent '{other}'"),
                "valid values: all, codex, claude-code, opencode, hermes".to_string(),
            ));
        }
    };
    let scope = match target.scope.as_str() {
        "personal" => SkillScope::Personal,
        "workspace" => SkillScope::Workspace,
        other => {
            return Err((
                format!("invalid --scope '{other}'"),
                "valid values: personal, workspace".to_string(),
            ));
        }
    };
    Ok(SkillOptions {
        agent,
        scope,
        skills_dir: target.skills_dir,
        force,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_first_data::OutputFormat;

    // ── Plain-text help tests ────────────────────────────────────────────

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_root_contains_all_subcommands() {
        let cmd = Cli::command();
        let help = agent_first_data::cli_render_help(&cmd, &[]);
        assert!(help.contains("config"), "must include config");
        assert!(help.contains("service"), "must include service");
        assert!(help.contains("ping"), "must include ping");
        assert!(help.contains("--output"), "must include global --output");
        assert!(help.contains("--log"), "must include global --log");
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_root_contains_nested_commands() {
        let cmd = Cli::command();
        let help = agent_first_data::cli_render_help(&cmd, &[]);
        // Must expand into nested subcommands
        assert!(help.contains("config show"), "must include config show");
        assert!(help.contains("config set"), "must include config set");
        assert!(help.contains("service start"), "must include service start");
        assert!(help.contains("service stop"), "must include service stop");
        assert!(
            help.contains("service status"),
            "must include service status"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_root_contains_secret_flags() {
        let cmd = Cli::command();
        let help = agent_first_data::cli_render_help(&cmd, &[]);
        assert!(
            help.contains("--api-key-secret"),
            "must include secret flag"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_root_contains_suffix_flags() {
        let cmd = Cli::command();
        let help = agent_first_data::cli_render_help(&cmd, &[]);
        assert!(
            help.contains("--timeout-ms"),
            "must include timeout_ms flag"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_one_level_plain_omits_nested_details() {
        let cmd = Cli::command();
        let help = agent_first_data::cli_render_help_with_options(
            &cmd,
            &[],
            &agent_first_data::HelpOptions {
                scope: agent_first_data::HelpScope::OneLevel,
                format: agent_first_data::HelpFormat::Plain,
            },
        );
        assert!(help.contains("config"), "one-level help must list config");
        assert!(help.contains("service"), "one-level help must list service");
        assert!(help.contains("ping"), "one-level help must list ping");
        assert!(
            !help.contains("Print this message or the help"),
            "one-level help should not advertise clap's help pseudo-command"
        );
        assert!(
            help.contains("--output"),
            "one-level help must include globals"
        );
        assert!(
            help.contains(concat!("AFDATA: ", env!("CARGO_PKG_VERSION"))),
            "one-level help must include AFDATA version"
        );
        assert!(
            !help.contains("--help-all"),
            "one-level help must not advertise removed recursive help flag"
        );
        assert!(
            !help.contains("--stream"),
            "one-level help must not advertise stream mode"
        );
        assert!(
            !help.contains("--result-only"),
            "one-level help must not advertise result-only mode"
        );
        assert!(
            !help.contains("config show"),
            "one-level root help must not expand config show"
        );
        assert!(
            !help.contains("--api-key-secret"),
            "one-level root help must not include service start flags"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_subcommand_scoped() {
        let cmd = Cli::command();
        let help = agent_first_data::cli_render_help(&cmd, &["service"]);
        assert!(help.contains("start"), "service help must include start");
        assert!(help.contains("stop"), "service help must include stop");
        assert!(help.contains("status"), "service help must include status");
        assert!(
            !help.contains("config show"),
            "service help must NOT include config show"
        );
        assert!(
            !help.contains("--timeout-ms"),
            "service help must NOT include ping's --timeout-ms"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_nested_subcommand_scoped() {
        let cmd = Cli::command();
        let help = agent_first_data::cli_render_help(&cmd, &["service", "start"]);
        assert!(
            help.contains("--port"),
            "service start help must include --port"
        );
        assert!(
            help.contains("--api-key-secret"),
            "service start help must include --api-key-secret"
        );
        assert!(
            !help.contains("service stop"),
            "service start help must NOT include stop"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_is_plain_text() {
        let cmd = Cli::command();
        let help = agent_first_data::cli_render_help(&cmd, &[]);
        // Plain text must not contain markdown heading markers or bold markers
        assert!(
            !help.contains("\n# "),
            "plain text must not have markdown headings"
        );
        assert!(
            !help.contains("**"),
            "plain text must not have markdown bold"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_root_uses_human_default() {
        let cmd = Cli::command();
        let raw = vec!["agent-cli".to_string(), "--help".to_string()];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("help should render");
        assert!(help.contains("--output"), "root help must include globals");
        assert!(
            !help.contains("--api-key-secret"),
            "human default must not recursively expand leaf flags"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_root_advertises_recursive_modifier() {
        let cmd = Cli::command();
        let raw = vec!["agent-cli".to_string(), "--help".to_string()];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("help should render");
        assert!(
            help.contains("--recursive"),
            "one-level root help must advertise the --recursive modifier:\n{help}"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_leaf_command_omits_recursive_modifier() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "ping".to_string(),
            "--help".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("help should render");
        assert!(
            !help.contains("--recursive"),
            "a leaf command with no subcommands must not advertise --recursive:\n{help}"
        );
        assert!(
            help.contains("--output"),
            "even a leaf --help must document the --output formats:\n{help}"
        );
    }

    /// Invariant: any `--help` invocation, in any scope and any output format,
    /// must document the help formats in at least one place.
    #[cfg(feature = "cli-help")]
    #[test]
    fn help_always_documents_formats_in_every_output() {
        for output in ["plain", "json", "yaml", "markdown"] {
            for extra in [Vec::new(), vec!["--recursive".to_string()]] {
                let mut raw = vec!["agent-cli".to_string(), "--help".to_string()];
                raw.extend(extra.iter().cloned());
                raw.push("--output".to_string());
                raw.push(output.to_string());

                let help = agent_first_data::cli_handle_help_or_continue(
                    &raw,
                    &Cli::command(),
                    &agent_first_data::HelpConfig::human_cli_default(),
                )
                .expect("valid help request")
                .unwrap_or_else(|| panic!("help should render for --output {output} {extra:?}"));

                for token in ["--recursive", "--output", "json", "yaml", "markdown"] {
                    assert!(
                        help.contains(token),
                        "--help --output {output} {extra:?} must document '{token}':\n{help}"
                    );
                }
            }
        }
    }

    #[cfg(feature = "cli-help")]
    fn secret_default_help_command() -> clap::Command {
        clap::Command::new("secret-defaults").arg(
            clap::Arg::new("api_key_secret")
                .long("api-key-secret")
                .default_value("sk-help-default")
                .help("API key default shown only as a redacted value"),
        )
    }

    #[cfg(feature = "cli-help")]
    fn security_help_default_case() -> serde_json::Value {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/spec/fixtures/security.json");
        let data = std::fs::read_to_string(path).expect("read security fixture");
        let fixture: serde_json::Value =
            serde_json::from_str(&data).expect("parse security fixture");
        fixture["help_default_cases"][0].clone()
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_redacts_secret_default_values_in_every_output() {
        let help_case = security_help_default_case();
        let default = help_case["default"].as_str().expect("fixture default");
        let expected = help_case["expected"].as_str().expect("fixture expected");
        assert_eq!(default, "sk-help-default");
        assert_eq!(expected, "***");
        for output in ["plain", "json", "yaml", "markdown"] {
            let raw = vec![
                "secret-defaults".to_string(),
                "--help".to_string(),
                "--output".to_string(),
                output.to_string(),
            ];
            let help = agent_first_data::cli_handle_help_or_continue(
                &raw,
                &secret_default_help_command(),
                &agent_first_data::HelpConfig::human_cli_default(),
            )
            .expect("valid help request")
            .unwrap_or_else(|| panic!("help should render for --output {output}"));
            assert!(
                help.contains(expected),
                "--help --output {output} must show the redaction marker:\n{help}"
            );
            assert!(
                !help.contains(default),
                "--help --output {output} must not leak secret defaults:\n{help}"
            );
        }
    }

    /// Token economy: in a recursive dump the help modifiers must be documented
    /// once (on the target command), not repeated on every descendant block.
    #[cfg(feature = "cli-help")]
    #[test]
    fn recursive_help_documents_modifiers_once() {
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--recursive".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &Cli::command(),
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("recursive help should render");
        let occurrences = help.matches("render this help in another format").count();
        assert_eq!(
            occurrences, 1,
            "recursive plain help must advertise the modifiers exactly once \
             (found {occurrences}):\n{help}"
        );
        // Descendant command blocks fall back to clap's bare wording.
        assert!(
            help.contains("Print help\n"),
            "descendant commands should keep the plain 'Print help' wording:\n{help}"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_output_plain_is_one_level() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--output".to_string(),
            "plain".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("plain help should render");
        assert!(help.contains("--output"), "plain help must include globals");
        assert!(
            !help.contains("--api-key-secret"),
            "plain help must stay one-level"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_output_json_without_recursive_is_one_level() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--output".to_string(),
            "json".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("json help should render");
        let parsed: serde_json::Value = serde_json::from_str(&help).expect("json help must parse");
        assert_eq!(parsed["result"]["code"], "help");
        assert_eq!(parsed["result"]["help"]["scope"], "one_level");
        assert!(
            !parsed.to_string().contains("api_key_secret"),
            "one-level json must not expand nested leaf flags"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_recursive_output_json_is_recursive() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--recursive".to_string(),
            "--output".to_string(),
            "json".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("json help should render");
        let parsed: serde_json::Value = serde_json::from_str(&help).expect("json help must parse");
        assert_eq!(parsed["result"]["code"], "help");
        assert_eq!(parsed["result"]["help"]["scope"], "recursive");
        assert!(
            parsed.to_string().contains("api_key_secret"),
            "recursive json export must include nested flags"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_recursive_plain_expands_tree() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--recursive".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("recursive plain help should render");
        assert!(
            help.contains("--api-key-secret"),
            "recursive plain help must expand nested leaf flags"
        );
        assert!(
            !help.contains("\n# "),
            "recursive plain help must stay plain text"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_bare_recursive_falls_through() {
        let cmd = Cli::command();
        let raw = vec!["agent-cli".to_string(), "--recursive".to_string()];
        assert!(
            agent_first_data::cli_handle_help_or_continue(
                &raw,
                &cmd,
                &agent_first_data::HelpConfig::human_cli_default(),
            )
            .expect("valid non-help request")
            .is_none(),
            "a bare --recursive without --help must fall through to clap"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_scopes_subcommand_help() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "service".to_string(),
            "--help".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("service help should render");
        assert!(help.contains("start"), "service help must list start");
        assert!(
            !help.contains("--api-key-secret"),
            "one-level service help must not expand service start flags"
        );
        assert!(
            !help.contains("--timeout-ms"),
            "service help must not include ping flags"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_does_not_intercept_help_pseudo_command() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "help".to_string(),
            "service".to_string(),
        ];
        assert!(
            agent_first_data::cli_handle_help_or_continue(
                &raw,
                &cmd,
                &agent_first_data::HelpConfig::human_cli_default(),
            )
            .expect("valid non-helper request")
            .is_none(),
            "`help <subcommand>` is not a recommended afdata help path"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_does_not_intercept_help_all_flag() {
        let cmd = Cli::command();
        let raw = vec!["agent-cli".to_string(), "--help-all".to_string()];
        assert!(
            agent_first_data::cli_handle_help_or_continue(
                &raw,
                &cmd,
                &agent_first_data::HelpConfig::human_cli_default(),
            )
            .expect("valid non-helper request")
            .is_none(),
            "`--help-all` is not part of the canonical afdata help path"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_markdown_without_recursive_is_one_level() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--output".to_string(),
            "markdown".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("markdown help should render");
        assert!(
            help.contains(&format!(
                "# {} - {}",
                env!("DISPLAY_NAME"),
                env!("CARGO_PKG_DESCRIPTION")
            )),
            "markdown heading must use display name and Cargo package description"
        );
        assert!(help.contains("```text"), "markdown must wrap clap help");
        assert_eq!(
            help.matches("Agent-facing CLI surfaces should keep stdout structured.")
                .count(),
            1,
            "long_about should render once, outside the fenced clap help block"
        );
        assert!(
            !help.contains("--api-key-secret"),
            "one-level markdown must not expand nested leaf flags"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_recursive_markdown_output() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--recursive".to_string(),
            "--output".to_string(),
            "markdown".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("markdown help should render");
        assert!(
            help.contains(&format!("# {}", env!("DISPLAY_NAME"))),
            "markdown must have heading"
        );
        assert!(help.contains("```text"), "markdown must wrap clap help");
        assert_eq!(
            help.matches("Agent-facing CLI surfaces should keep stdout structured.")
                .count(),
            1,
            "recursive markdown should not repeat root long_about in fenced help"
        );
        assert!(
            help.contains("--api-key-secret"),
            "recursive markdown export must include nested flags"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_supports_inline_output_format() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--output=json".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("json help should render");
        let parsed: serde_json::Value =
            serde_json::from_str(&help).expect("inline json help must parse");
        assert_eq!(parsed["result"]["code"], "help");
        assert_eq!(parsed["result"]["help"]["scope"], "one_level");
        assert_eq!(parsed["result"]["help"]["name"], env!("DISPLAY_NAME"));
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_invalid_output_format_is_error() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--output".to_string(),
            "xml".to_string(),
        ];
        let err = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect_err("invalid help output must return error");
        assert_eq!(err["kind"], "error");
        assert_eq!(err["error"]["code"], "cli_error");
        assert!(
            err["error"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("xml")),
            "error should mention invalid value: {err}"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_protocol_v1_json_wraps_help_schema() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--output".to_string(),
            "json".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("json help should render");
        let event: serde_json::Value = serde_json::from_str(help.trim()).expect("json event");
        assert_eq!(event["kind"], "result");
        assert_eq!(event["result"]["code"], "help");
        assert!(event["result"]["help"].is_object());
        assert_eq!(event["trace"], serde_json::json!({}));
        agent_first_data::validate_protocol_event(&event, true).expect("strict event");
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_protocol_v1_yaml_wraps_help_schema() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--output".to_string(),
            "yaml".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("yaml help should render");
        assert!(help.contains("kind: \"result\""), "{help}");
        assert!(help.contains("code: \"help\""), "{help}");
        assert!(help.contains("trace: {}"), "{help}");
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_protocol_v1_invalid_format_has_trace() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--output".to_string(),
            "xml".to_string(),
        ];
        let event = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect_err("invalid help output must fail");
        assert_eq!(event["kind"], "error");
        assert_eq!(event["error"]["code"], "cli_error");
        assert_eq!(event["trace"], serde_json::json!({}));
        agent_first_data::validate_protocol_event(&event, true).expect("strict event");
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_missing_output_format_is_error() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--help".to_string(),
            "--output".to_string(),
        ];
        let err = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect_err("missing help output value must return error");
        assert_eq!(err["kind"], "error");
        assert_eq!(err["error"]["code"], "cli_error");
        assert!(
            err["error"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("missing value")),
            "error should mention missing value: {err}"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_missing_output_before_help_is_error() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--output".to_string(),
            "--help".to_string(),
        ];
        let err = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect_err("missing help output value must return error");
        assert_eq!(err["kind"], "error");
        assert_eq!(err["error"]["code"], "cli_error");
        assert!(
            err["error"]["message"]
                .as_str()
                .is_some_and(|s| s.contains("missing value")),
            "error should mention missing value: {err}"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_skips_flag_values_when_finding_subcommands() {
        let cmd = Cli::command();
        let raw = vec![
            "agent-cli".to_string(),
            "--log".to_string(),
            "service".to_string(),
            "--help".to_string(),
            "--recursive".to_string(),
            "--output".to_string(),
            "markdown".to_string(),
        ];
        let help = agent_first_data::cli_handle_help_or_continue(
            &raw,
            &cmd,
            &agent_first_data::HelpConfig::human_cli_default(),
        )
        .expect("valid help request")
        .expect("markdown help should render");
        assert!(
            help.contains("config show"),
            "flag value named like a subcommand must not scope help"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_handler_without_help_returns_none() {
        let cmd = Cli::command();
        let raw = vec!["agent-cli".to_string(), "ping".to_string()];
        assert!(
            agent_first_data::cli_handle_help_or_continue(
                &raw,
                &cmd,
                &agent_first_data::HelpConfig::human_cli_default(),
            )
            .expect("valid non-help request")
            .is_none(),
            "non-help invocations must continue to clap"
        );
    }

    // ── Markdown help tests ──────────────────────────────────────────────

    #[cfg(feature = "cli-help-markdown")]
    #[test]
    fn help_markdown_root_contains_all() {
        let cmd = Cli::command();
        let md = agent_first_data::cli_render_help_markdown(&cmd, &[]);
        assert!(md.contains("config"), "markdown must include config");
        assert!(md.contains("service"), "markdown must include service");
        assert!(md.contains("ping"), "markdown must include ping");
        assert!(
            md.contains("--api-key-secret"),
            "markdown must include secret flag"
        );
        assert!(
            md.contains("--timeout-ms"),
            "markdown must include timeout flag"
        );
    }

    #[cfg(feature = "cli-help-markdown")]
    #[test]
    fn help_markdown_has_headings() {
        let cmd = Cli::command();
        let md = agent_first_data::cli_render_help_markdown(&cmd, &[]);
        assert!(md.contains('#'), "markdown must have headings");
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_markdown_one_level_omits_descendant_details() {
        let cmd = Cli::command();
        let md = agent_first_data::cli_render_help_with_options(
            &cmd,
            &[],
            &agent_first_data::HelpOptions {
                scope: agent_first_data::HelpScope::OneLevel,
                format: agent_first_data::HelpFormat::Markdown,
            },
        );
        assert!(
            md.contains(&format!("# {}", env!("DISPLAY_NAME"))),
            "markdown must include root"
        );
        assert!(
            !md.contains("--api-key-secret"),
            "one-level markdown must not include nested flags"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_markdown_strips_about_from_leading_long_about() {
        let cmd = clap::Command::new("sample")
            .about("Shared summary")
            .long_about("Shared summary\n\nDetailed policy.");
        let md = agent_first_data::cli_render_help_with_options(
            &cmd,
            &[],
            &agent_first_data::HelpOptions {
                scope: agent_first_data::HelpScope::OneLevel,
                format: agent_first_data::HelpFormat::Markdown,
            },
        );
        assert!(md.contains("# sample - Shared summary"));
        assert_eq!(
            md.matches("Shared summary").count(),
            1,
            "about should live in the heading, not repeat at the start of long_about"
        );
        assert!(md.contains("Detailed policy."));
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_markdown_suppresses_name_dash_about_body() {
        let cmd = clap::Command::new("Agent CLI")
            .about("Brief summary")
            .long_about("Agent CLI - Brief summary");
        let md = agent_first_data::cli_render_help_with_options(
            &cmd,
            &[],
            &agent_first_data::HelpOptions {
                scope: agent_first_data::HelpScope::OneLevel,
                format: agent_first_data::HelpFormat::Markdown,
            },
        );
        assert!(
            md.contains("# Agent CLI - Brief summary"),
            "heading must include name and about"
        );
        assert_eq!(
            md.matches("Brief summary").count(),
            1,
            "when long_about equals 'name - about', body is suppressed to avoid duplication"
        );
    }

    #[cfg(feature = "cli-help-markdown")]
    #[test]
    fn help_markdown_no_footer() {
        let cmd = Cli::command();
        let md = agent_first_data::cli_render_help_markdown(&cmd, &[]);
        assert!(
            !md.contains("<hr/>"),
            "markdown must not have clap-markdown footer"
        );
        assert!(
            !md.contains("<small>"),
            "markdown must not have clap-markdown footer"
        );
    }

    #[cfg(feature = "cli-help-markdown")]
    #[test]
    fn help_markdown_subcommand_scoped() {
        let cmd = Cli::command();
        let md = agent_first_data::cli_render_help_markdown(&cmd, &["service"]);
        assert!(
            md.contains("--api-key-secret"),
            "service markdown must include secret flag"
        );
        assert!(
            !md.contains("--timeout-ms"),
            "service markdown must NOT include ping's --timeout-ms"
        );
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_json_schema_is_parseable() {
        let cmd = Cli::command();
        let json = agent_first_data::cli_render_help_with_options(
            &cmd,
            &[],
            &agent_first_data::HelpOptions {
                scope: agent_first_data::HelpScope::OneLevel,
                format: agent_first_data::HelpFormat::Json,
            },
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("help json must parse");
        assert_eq!(parsed["code"], "help");
        assert_eq!(parsed["scope"], "one_level");
        assert_eq!(parsed["versions"]["afdata"], env!("CARGO_PKG_VERSION"));
        assert_eq!(parsed["name"], env!("DISPLAY_NAME"));
        assert_eq!(parsed["subcommands"][0]["arguments"], serde_json::json!([]));
    }

    #[cfg(feature = "cli-help")]
    #[test]
    fn help_yaml_schema_is_raw_yaml() {
        let cmd = Cli::command();
        let yaml = agent_first_data::cli_render_help_with_options(
            &cmd,
            &[],
            &agent_first_data::HelpOptions {
                scope: agent_first_data::HelpScope::Recursive,
                format: agent_first_data::HelpFormat::Yaml,
            },
        );
        assert!(yaml.starts_with("---"), "yaml help must start with marker");
        assert!(
            yaml.contains("code: \"help\""),
            "yaml help must include code"
        );
        assert!(
            yaml.contains("scope: \"recursive\""),
            "yaml help must include recursive scope"
        );
        assert!(
            yaml.contains("versions:") && yaml.contains("afdata:"),
            "yaml help must include the AFDATA version"
        );
        assert!(
            yaml.contains("api_key_secret"),
            "raw help schema must preserve secret-like argument ids"
        );
    }

    // ── CLI helper tests (unchanged) ─────────────────────────────────────

    #[test]
    fn parse_output_all_variants() {
        assert!(matches!(cli_parse_output("json"), Ok(OutputFormat::Json)));
        assert!(matches!(cli_parse_output("yaml"), Ok(OutputFormat::Yaml)));
        assert!(matches!(cli_parse_output("plain"), Ok(OutputFormat::Plain)));
        assert!(cli_parse_output("xml").is_err());
    }

    #[test]
    fn json_alias_resolves_to_json_output() {
        assert_eq!(resolve_output("json", true).expect("json alias"), "json");
    }

    #[test]
    fn json_alias_conflicts_with_other_output_formats() {
        let err = resolve_output("yaml", true).expect_err("json/yaml conflict");
        assert!(err.contains("conflicting output formats"));
    }

    #[test]
    fn parse_log_normalizes() {
        let f = cli_parse_log_filters(&["Startup", " REQUEST ", "startup"]);
        assert_eq!(f.as_slice(), &["startup", "request"]);
    }

    #[test]
    fn log_filter_is_explicit_with_wildcards() {
        assert!(!cli_parse_log_filters::<String>(&[]).enabled("startup"));
        assert!(!cli_parse_log_filters(&["query.result"]).enabled("startup"));
        assert!(cli_parse_log_filters(&["startup"]).enabled("startup"));
        // `all` is the single wildcard word; it enables every category.
        assert!(cli_parse_log_filters(&["all"]).enabled("startup"));
        assert!(cli_parse_log_filters(&["all"]).enabled("request"));
        // `*` is not special — it is a literal prefix, so it enables nothing
        // unless an event name actually starts with it.
        assert!(!cli_parse_log_filters(&["*"]).enabled("request"));
        // an explicit, unrelated category does not enable a different one.
        assert!(!cli_parse_log_filters(&["startup"]).enabled("request"));
    }

    #[test]
    fn request_log_is_category_tagged() {
        let v = build_request_log(None).into_value();
        assert_eq!(v["kind"], "log");
        assert_eq!(v["log"]["category"], "request");
        assert_eq!(v["log"]["command"], "none");
    }

    #[test]
    fn startup_log_contains_parsed_config_and_scoped_env() {
        let log = cli_parse_log_filters(&["startup"]);
        let command = Command::Ping {
            host: Some("example.com".to_string()),
            timeout_ms: 5000,
        };
        let v = build_startup_log(Some(&command), "yaml", &log, false).into_value();
        assert_eq!(v["kind"], "log");
        assert_eq!(v["log"]["category"], "startup");
        assert_eq!(v["log"]["event"], "startup");
        assert_eq!(
            v["log"]["parsed"],
            serde_json::json!({
                "command": "ping",
                "output": "yaml",
                "log": ["startup"],
                "verbose": false,
            })
        );
        assert_eq!(
            v["log"]["effective_config"],
            serde_json::json!({
                "output": "yaml",
                "log": ["startup"],
            })
        );

        let env = v["log"]["env"].as_array().expect("env must be an array");
        let host = env
            .iter()
            .find(|entry| entry["key"] == AGENT_CLI_HOST_ENV)
            .expect("startup env must include exact AGENT_CLI_HOST key");
        assert!(host["present"].is_boolean());
        if std::env::var_os(AGENT_CLI_HOST_ENV).is_some() {
            assert!(host["value"].is_string());
        } else {
            assert!(host["value"].is_null());
        }
    }

    #[test]
    fn build_cli_error_structure() {
        let v = build_cli_error("--output: invalid value 'xml'", None).into_value();
        assert_eq!(v["kind"], "error");
        assert_eq!(v["error"]["code"], "cli_error");
        assert_eq!(v["error"]["message"], "--output: invalid value 'xml'");
        assert!(v.get("error_code").is_none());
        assert!(v.get("retryable").is_none());
        assert!(v["trace"].is_object());
    }

    #[test]
    fn build_cli_error_with_hint() {
        let v =
            build_cli_error("unknown action: foo", Some("valid actions: echo, ping")).into_value();
        assert_eq!(v["kind"], "error");
        assert_eq!(v["error"]["hint"], "valid actions: echo, ping");
    }

    #[test]
    fn json_error_with_hint() {
        let v = json_error("not_configured", "not configured")
            .hint("set PING_HOST")
            .build()
            .expect("error builder failed")
            .into_value();
        assert_eq!(v["kind"], "error");
        assert_eq!(v["error"]["code"], "not_configured");
        assert_eq!(v["error"]["message"], "not configured");
        assert_eq!(v["error"]["hint"], "set PING_HOST");
    }

    #[test]
    fn json_error_without_hint_has_no_hint_key() {
        let v = json_error("failed", "something failed")
            .build()
            .expect("error builder failed")
            .into_value();
        assert!(v["error"].get("hint").is_none());
    }

    #[test]
    fn render_all_formats_compile_and_run() {
        let v = json_result(serde_json::json!({"ok": true}))
            .build()
            .into_value();
        let json_out = render(&v, OutputFormat::Json, &OutputOptions::default());
        let yaml_out = render(&v, OutputFormat::Yaml, &OutputOptions::default());
        let plain_out = render(&v, OutputFormat::Plain, &OutputOptions::default());

        assert!(json_out.contains("\"kind\""));
        assert!(yaml_out.starts_with("---"));
        assert!(plain_out.contains("kind=result"));
    }

    #[test]
    fn error_round_trip_is_valid_jsonl() {
        let err = build_cli_error("unknown flag: --foo", None).into_value();
        let line = render(
            &err,
            agent_first_data::OutputFormat::Json,
            &OutputOptions::default(),
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&line).unwrap_or(serde_json::Value::Null);
        assert_eq!(parsed["kind"], "error");
        assert_eq!(parsed["error"]["code"], "cli_error");
        assert!(!line.contains('\n'));
    }

    // ── Skill subcommand tests (feature `skill-admin`) ───────────────────

    #[cfg(feature = "skill-admin")]
    mod skill {
        use super::*;
        use agent_first_data::skill::{
            SkillAction, SkillAgentSelection, SkillOptions, SkillScope, run_skill_admin,
        };
        use std::path::PathBuf;
        use std::time::{SystemTime, UNIX_EPOCH};

        fn temp_skills_dir(tag: &str) -> PathBuf {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            std::env::temp_dir().join(format!("agent_cli_skill_{tag}_{}", suffix))
        }

        #[cfg(feature = "cli-help")]
        #[test]
        fn help_includes_skill_subcommand() {
            let cmd = Cli::command();
            let help = agent_first_data::cli_render_help(&cmd, &[]);
            assert!(help.contains("skill"), "root help must include skill");
            assert!(
                help.contains("skill install"),
                "help must expand skill install"
            );
            assert!(
                help.contains("--skills-dir"),
                "help must include --skills-dir"
            );
        }

        #[test]
        fn build_options_parses_agents_and_scopes() {
            let target = SkillTargetArgs {
                agent: "opencode".to_string(),
                scope: "workspace".to_string(),
                skills_dir: Some("/tmp/x".to_string()),
            };
            let options = build_skill_options(target, true).expect("valid options");
            assert_eq!(options.agent, SkillAgentSelection::Opencode);
            assert_eq!(options.scope, SkillScope::Workspace);
            assert!(options.force);

            let workspace = SkillTargetArgs {
                agent: "codex".to_string(),
                scope: "workspace".to_string(),
                skills_dir: None,
            };
            let options = build_skill_options(workspace, false).expect("valid options");
            assert_eq!(options.agent, SkillAgentSelection::Codex);
            assert_eq!(options.scope, SkillScope::Workspace);
        }

        #[test]
        fn build_options_rejects_unknown_agent_and_scope() {
            let bad_agent = SkillTargetArgs {
                agent: "vim".to_string(),
                scope: "personal".to_string(),
                skills_dir: None,
            };
            assert!(build_skill_options(bad_agent, false).is_err());

            let bad_scope = SkillTargetArgs {
                agent: "codex".to_string(),
                scope: "global".to_string(),
                skills_dir: None,
            };
            assert!(build_skill_options(bad_scope, false).is_err());
        }

        #[test]
        fn install_status_uninstall_widget_skill() {
            let dir = temp_skills_dir("opencode");
            let options = SkillOptions {
                agent: SkillAgentSelection::Opencode,
                scope: SkillScope::Personal,
                skills_dir: Some(dir.to_string_lossy().to_string()),
                force: false,
            };

            let installed = run_skill_admin(&WIDGET_SPEC, SkillAction::Install, &options);
            assert!(installed.is_ok());
            let skill_path = dir.join("agent-first-widget").join("SKILL.md");
            assert!(skill_path.is_file());

            let report =
                run_skill_admin(&WIDGET_SPEC, SkillAction::Status, &options).expect("status ok");
            let status = serde_json::to_value(&report).expect("serialize report");
            assert_eq!(status["installed_all"], true);
            assert_eq!(status["valid_all"], true);
            assert_eq!(status["current_all"], true);
            assert_eq!(status["targets"][0]["agent"], "opencode");

            let removed = run_skill_admin(&WIDGET_SPEC, SkillAction::Uninstall, &options);
            assert!(removed.is_ok());
            assert!(!skill_path.exists());
            let _ = std::fs::remove_dir_all(dir);
        }

        #[test]
        fn run_skill_returns_zero_on_success() {
            let dir = temp_skills_dir("run");
            let args = SkillTargetArgs {
                agent: "codex".to_string(),
                scope: "personal".to_string(),
                skills_dir: Some(dir.to_string_lossy().to_string()),
            };
            let mut emitter = CliEmitter::finite(OutputFormat::Json);
            let code = run_skill(
                &mut emitter,
                SkillCmd::Install(SkillWriteArgs {
                    target: args,
                    force: false,
                }),
            );
            assert_eq!(code, 0);
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
