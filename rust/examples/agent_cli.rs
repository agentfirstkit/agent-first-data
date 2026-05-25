#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr
)]

// Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.
//
// Demonstrates: recursive --help (all subcommands expanded), --help-markdown,
// _secret flags, nested subcommands, cli_parse_output, cli_parse_log_filters,
// opt-in startup diagnostics, cli_output, build_cli_error, and error hints.
//
// Run:  cargo run --example agent_cli --features cli-help,cli-help-markdown -- --help
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- service --help
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- service start --help
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- --help-markdown
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- ping --timeout-ms 5000
//       cargo run --example agent_cli --features cli-help,cli-help-markdown -- --log startup ping --host example.com
// Test: cargo test --examples --features cli-help,cli-help-markdown

use agent_first_data::{
    build_cli_error, build_json, build_json_error, build_json_ok, cli_output,
    cli_parse_log_filters, cli_parse_output,
};
use clap::{CommandFactory, Parser, Subcommand};

const AGENT_CLI_HOST_ENV: &str = "AGENT_CLI_HOST";
const STARTUP_ENV_KEYS: &[&str] = &[AGENT_CLI_HOST_ENV];

#[derive(Parser)]
#[command(name = "agent-cli", version, about = "Minimal agent-first CLI example")]
struct Cli {
    /// Output format: json (default), yaml, plain
    #[arg(long, default_value = "json")]
    output: String,

    /// Log categories (comma-separated): startup, request, ...
    #[arg(long, value_delimiter = ',')]
    log: Vec<String>,

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

    // Extract subcommand path (args before any --flags) for scoped help
    let subcommand_path: Vec<&str> = raw[1..]
        .iter()
        .take_while(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    // --help → recursive plain-text help, all subcommands expanded
    if raw.iter().any(|a| a == "--help" || a == "-h") {
        let cmd = Cli::command();
        print!(
            "{}",
            agent_first_data::cli_render_help(&cmd, &subcommand_path)
        );
        std::process::exit(0);
    }

    // --help-markdown → Markdown help for documentation generation
    if raw.iter().any(|a| a == "--help-markdown") {
        let cmd = Cli::command();
        print!(
            "{}",
            agent_first_data::cli_render_help_markdown(&cmd, &subcommand_path)
        );
        std::process::exit(0);
    }

    // try_parse — clap errors become JSONL to stdout, not stderr text
    let cli = Cli::try_parse().unwrap_or_else(|e| {
        if matches!(e.kind(), clap::error::ErrorKind::DisplayVersion) {
            e.exit();
        }
        println!(
            "{}",
            agent_first_data::output_json(&build_cli_error(
                &e.to_string(),
                Some("try: agent-cli --help"),
            ))
        );
        std::process::exit(2);
    });

    // Parse --output and --log
    let format = cli_parse_output(&cli.output).unwrap_or_else(|e| {
        println!(
            "{}",
            agent_first_data::output_json(&build_cli_error(
                &e,
                Some("valid values: json, yaml, plain"),
            ))
        );
        std::process::exit(2);
    });
    let log = cli_parse_log_filters(&cli.log);

    if startup_log_requested(&log) {
        println!("{}", cli_output(&build_startup_log(), format));
    }

    match cli.command {
        None => {
            println!(
                "{}",
                agent_first_data::output_json(&build_cli_error(
                    "no subcommand provided",
                    Some("try: agent-cli --help"),
                ))
            );
            std::process::exit(2);
        }
        Some(Command::Config { action }) => match action {
            ConfigAction::Show => {
                let result = build_json_ok(serde_json::json!({"action": "config_show"}), None);
                println!("{}", cli_output(&result, format));
            }
            ConfigAction::Set { key, value } => {
                let result = build_json_ok(
                    serde_json::json!({"action": "config_set", "key": key, "value": value}),
                    None,
                );
                println!("{}", cli_output(&result, format));
            }
        },
        Some(Command::Service { action }) => match action {
            ServiceAction::Start {
                port,
                api_key_secret,
            } => {
                let result = build_json_ok(
                    serde_json::json!({"action": "service_start", "port": port, "api_key_secret": api_key_secret}),
                    None,
                );
                println!("{}", cli_output(&result, format));
            }
            ServiceAction::Stop => {
                let result = build_json_ok(serde_json::json!({"action": "service_stop"}), None);
                println!("{}", cli_output(&result, format));
            }
            ServiceAction::Status => {
                let result = build_json_ok(serde_json::json!({"action": "service_status"}), None);
                println!("{}", cli_output(&result, format));
            }
        },
        Some(Command::Ping { host, timeout_ms }) => {
            let host = host.or_else(|| std::env::var(AGENT_CLI_HOST_ENV).ok());
            if host.is_none() {
                let err = build_json_error(
                    "ping target not configured",
                    Some("pass --host or set AGENT_CLI_HOST"),
                    Some(serde_json::json!({"duration_ms": 0})),
                );
                println!("{}", cli_output(&err, format));
                std::process::exit(1);
            }
            let result = build_json_ok(
                serde_json::json!({"action": "ping", "host": host, "timeout_ms": timeout_ms}),
                None,
            );
            println!("{}", cli_output(&result, format));
        }
    }
}

fn startup_log_requested(log: &[String]) -> bool {
    log.iter()
        .any(|category| matches!(category.as_str(), "startup" | "all" | "*"))
}

fn build_startup_log() -> serde_json::Value {
    build_json(
        "log",
        serde_json::json!({
            "event": "startup",
            "env": startup_env_snapshot(),
        }),
        None,
    )
}

fn startup_env_snapshot() -> serde_json::Value {
    serde_json::Value::Array(
        STARTUP_ENV_KEYS
            .iter()
            .map(|key| {
                serde_json::json!({
                    "key": key,
                    "present": std::env::var_os(*key).is_some(),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_first_data::OutputFormat;

    // ── Plain-text help tests ────────────────────────────────────────────

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

    #[test]
    fn help_root_contains_secret_flags() {
        let cmd = Cli::command();
        let help = agent_first_data::cli_render_help(&cmd, &[]);
        assert!(
            help.contains("--api-key-secret"),
            "must include secret flag"
        );
    }

    #[test]
    fn help_root_contains_suffix_flags() {
        let cmd = Cli::command();
        let help = agent_first_data::cli_render_help(&cmd, &[]);
        assert!(
            help.contains("--timeout-ms"),
            "must include timeout_ms flag"
        );
    }

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

    // ── Markdown help tests ──────────────────────────────────────────────

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

    #[test]
    fn help_markdown_has_headings() {
        let cmd = Cli::command();
        let md = agent_first_data::cli_render_help_markdown(&cmd, &[]);
        assert!(md.contains('#'), "markdown must have headings");
    }

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

    // ── CLI helper tests (unchanged) ─────────────────────────────────────

    #[test]
    fn parse_output_all_variants() {
        assert!(matches!(cli_parse_output("json"), Ok(OutputFormat::Json)));
        assert!(matches!(cli_parse_output("yaml"), Ok(OutputFormat::Yaml)));
        assert!(matches!(cli_parse_output("plain"), Ok(OutputFormat::Plain)));
        assert!(cli_parse_output("xml").is_err());
    }

    #[test]
    fn parse_log_normalizes() {
        let f = cli_parse_log_filters(&["Startup", " REQUEST ", "startup"]);
        assert_eq!(f, vec!["startup", "request"]);
    }

    #[test]
    fn startup_log_filter_is_explicit() {
        assert!(!startup_log_requested(
            &cli_parse_log_filters::<String>(&[])
        ));
        assert!(!startup_log_requested(&cli_parse_log_filters(&[
            "query.result"
        ])));
        assert!(startup_log_requested(&cli_parse_log_filters(&["startup"])));
        assert!(startup_log_requested(&cli_parse_log_filters(&["all"])));
        assert!(startup_log_requested(&cli_parse_log_filters(&["*"])));
    }

    #[test]
    fn startup_log_contains_only_env_presence() {
        let v = build_startup_log();
        assert_eq!(v["code"], "log");
        assert_eq!(v["event"], "startup");
        assert!(v.get("args").is_none());
        assert!(v.get("mode").is_none());
        assert!(v.get("command").is_none());

        let env = v["env"].as_array().expect("env must be an array");
        let host = env
            .iter()
            .find(|entry| entry["key"] == AGENT_CLI_HOST_ENV)
            .expect("startup env must include exact AGENT_CLI_HOST key");
        assert!(host["present"].is_boolean());
        assert!(host.get("value").is_none());
    }

    #[test]
    fn build_cli_error_structure() {
        let v = build_cli_error("--output: invalid value 'xml'", None);
        assert_eq!(v["code"], "error");
        assert_eq!(v["error_code"], "invalid_request");
        assert_eq!(v["retryable"], false);
        assert_eq!(v["trace"]["duration_ms"], 0);
    }

    #[test]
    fn build_cli_error_with_hint() {
        let v = build_cli_error("unknown action: foo", Some("valid actions: echo, ping"));
        assert_eq!(v["code"], "error");
        assert_eq!(v["hint"], "valid actions: echo, ping");
    }

    #[test]
    fn build_json_error_with_hint() {
        let v = build_json_error("not configured", Some("set PING_HOST"), None);
        assert_eq!(v["code"], "error");
        assert_eq!(v["error"], "not configured");
        assert_eq!(v["hint"], "set PING_HOST");
    }

    #[test]
    fn build_json_error_without_hint_has_no_hint_key() {
        let v = build_json_error("something failed", None, None);
        assert!(v.get("hint").is_none());
    }

    #[test]
    fn cli_output_all_formats_compile_and_run() {
        let v = serde_json::json!({"code": "ok"});
        let json_out = cli_output(&v, OutputFormat::Json);
        let yaml_out = cli_output(&v, OutputFormat::Yaml);
        let plain_out = cli_output(&v, OutputFormat::Plain);

        assert!(json_out.contains("\"code\""));
        assert!(yaml_out.starts_with("---"));
        assert!(plain_out.contains("code=ok"));
    }

    #[test]
    fn error_round_trip_is_valid_jsonl() {
        let err = build_cli_error("unknown flag: --foo", None);
        let line = agent_first_data::output_json(&err);
        let parsed: serde_json::Value =
            serde_json::from_str(&line).unwrap_or(serde_json::Value::Null);
        assert_eq!(parsed["code"], "error");
        assert!(!line.contains('\n'));
    }
}
