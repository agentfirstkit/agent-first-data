// Minimal agent-first CLI — canonical pattern for tools built on agent-first-data.
//
// Demonstrates the correct use of: try_parse, cli_parse_output, cli_parse_log_filters,
// cli_output, build_cli_error, --dry-run, and error hints.
//
// Run:  cargo run --example agent_cli -- echo --output json
//       cargo run --example agent_cli -- echo --dry-run --output yaml
//       cargo run --example agent_cli -- ping --output json
//       API_KEY_SECRET=sk-example cargo run --example agent_cli -- echo --output yaml --log startup,request
// Test: cargo test --examples

#![allow(clippy::print_stdout)]

use agent_first_data::{
    build_cli_error, build_json_error, cli_output, cli_parse_log_filters, cli_parse_output,
};
use clap::Parser;

#[derive(Parser)]
#[command(name = "agent-cli", version, about = "Minimal agent-first CLI example")]
struct Cli {
    /// Action to perform (echo, ping)
    action: String,

    /// Output format: json (default), yaml, plain
    #[arg(long, default_value = "json")]
    output: String,

    /// Preview the operation without executing
    #[arg(long)]
    dry_run: bool,

    /// Log categories (comma-separated): startup, request, ...
    #[arg(long, value_delimiter = ',')]
    log: Vec<String>,
}

fn main() {
    // Step 1: try_parse — clap errors become JSONL to stdout, not stderr text
    let cli = Cli::try_parse().unwrap_or_else(|e| {
        if matches!(
            e.kind(),
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
        ) {
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

    // Step 2: parse --output with shared helper
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

    // Step 3: parse --log with shared helper (trim + lowercase + dedup)
    let log = cli_parse_log_filters(&cli.log);

    // Step 4: validate action — demonstrate build_cli_error with hint
    let valid_actions = ["echo", "ping"];
    if !valid_actions.contains(&cli.action.as_str()) {
        let msg = format!("unknown action: {}", cli.action);
        let hint = format!("valid actions: {}", valid_actions.join(", "));
        println!(
            "{}",
            agent_first_data::output_json(&build_cli_error(&msg, Some(&hint)))
        );
        std::process::exit(2);
    }

    // Step 5: optionally emit startup diagnostic event
    if startup_log_enabled(&log) {
        let startup = agent_first_data::build_json(
            "log",
            serde_json::json!({
                "event": "startup",
                "args": {
                    "action": cli.action,
                    "output": cli.output,
                    "dry_run": cli.dry_run,
                    "log": log,
                },
                "env": {
                    "API_KEY_SECRET": std::env::var("API_KEY_SECRET").ok(),
                    "RUST_LOG": std::env::var("RUST_LOG").ok(),
                }
            }),
            None,
        );
        println!("{}", cli_output(&startup, format));
    }

    // Step 6: --dry-run → preview without executing
    if cli.dry_run {
        let preview = agent_first_data::build_json(
            "dry_run",
            serde_json::json!({
                "action": cli.action,
                "log": log,
            }),
            Some(serde_json::json!({"duration_ms": 0})),
        );
        println!("{}", cli_output(&preview, format));
        return;
    }

    // Step 7: do work — demonstrate build_json_error with hint on failure
    if cli.action == "ping" {
        let err = build_json_error(
            "ping target not configured",
            Some("set PING_HOST or pass --host"),
            Some(serde_json::json!({"duration_ms": 0})),
        );
        println!("{}", cli_output(&err, format));
        std::process::exit(1);
    }

    let result = agent_first_data::build_json_ok(
        serde_json::json!({
            "action": cli.action,
            "log": log,
        }),
        None,
    );
    println!("{}", cli_output(&result, format));
}

fn startup_log_enabled(filters: &[String]) -> bool {
    filters
        .iter()
        .any(|f| matches!(f.as_str(), "startup" | "all" | "*"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_first_data::OutputFormat;

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
    fn startup_log_enabled_matches_expected_categories() {
        assert!(startup_log_enabled(&["startup".to_string()]));
        assert!(startup_log_enabled(&["all".to_string()]));
        assert!(startup_log_enabled(&["*".to_string()]));
        assert!(!startup_log_enabled(&["request".to_string()]));
        assert!(!startup_log_enabled(&[]));
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

    // Verify the full pattern compiles: try_parse error → build_cli_error → output_json
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
