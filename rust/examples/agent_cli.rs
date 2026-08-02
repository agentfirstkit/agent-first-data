//! Canonical closed-world AFDATA CLI example.
//!
//! Run:
//! `cargo run --example agent_cli -- --help`
//! `cargo run --example agent_cli -- echo hello --dry-run`
//! `cargo run --example agent_cli -- ping --host example.com`

use agent_first_data::{
    ArgSpec, CliOutcome, CliSpec, Combination, CommandSpec, Event, OutputFormat, OutputPlan,
    OutputSpec, OutputTo, ResolvedInvocation, cli_error_event, cli_help_event, cli_version_event,
    json_error, json_result, render,
};
use serde_json::json;
use std::process::ExitCode;

fn output() -> OutputSpec {
    OutputSpec::protocol_finite(
        ["json", "yaml", "plain"],
        ["split", "stdout", "stderr"],
        "json",
        "split",
    )
}

fn cli_spec() -> Result<agent_first_data::BuiltCliSpec, agent_first_data::CliSpecError> {
    CliSpec::new("agent-cli", env!("CARGO_PKG_VERSION"))
        .about("Canonical closed-world AFDATA CLI example")
        .lifecycle_output(output())
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["echo"])
                .about("Echo one message")
                .arg(ArgSpec::positional("message", 0, "MESSAGE").about("Message to echo"))
                .arg(ArgSpec::flag("--dry-run").about("Preview without executing"))
                .combination(
                    Combination::new("echo")
                        .action("echo")
                        .about("Echo one message")
                        .required(["message"])
                        .optional(["dry_run"])
                        .output(output()),
                ),
        )
        .command(
            CommandSpec::new(["ping"])
                .about("Describe a ping request")
                .arg(ArgSpec::option("--host", "HOST").about("Target host"))
                .combination(
                    Combination::new("ping")
                        .action("ping")
                        .about("Describe a ping request")
                        .required(["host"])
                        .output(output()),
                ),
        )
        // Here so it stays proven: AFDATA answers `--version` at the root, but
        // the name past the command path belongs to the application — this one
        // is the release's version, not a request for agent-cli's.
        .command(
            CommandSpec::new(["release"])
                .about("Describe a release request")
                .arg(ArgSpec::option("--version", "VERSION").about("Version to release"))
                .combination(
                    Combination::new("release")
                        .action("release")
                        .about("Describe a release request")
                        .required(["version"])
                        .output(output()),
                ),
        )
        .build()
}

fn echo(invocation: &ResolvedInvocation) -> Event {
    json_result(json!({
        "code": "echo",
        "message": invocation.required("message").as_str(),
        "dry_run": invocation.optional("dry_run").and_then(|value| value.as_bool()).unwrap_or(false),
    }))
    .build()
}

fn ping(invocation: &ResolvedInvocation) -> Event {
    json_result(json!({
        "code": "ping",
        "host": invocation.required("host").as_str(),
    }))
    .build()
}

fn release(invocation: &ResolvedInvocation) -> Event {
    json_result(json!({
        "code": "release",
        "version": invocation.required("version").as_str(),
    }))
    .build()
}

fn output_format(plan: &OutputPlan) -> OutputFormat {
    match plan.format() {
        Some("yaml") => OutputFormat::Yaml,
        Some("plain") => OutputFormat::Plain,
        _ => OutputFormat::Json,
    }
}

// Routing, and the rule that a broken pipe is not a failure, belong to AFDATA:
// `--docs` and plain help are injected into every registry, so a CLI that
// hand-rolled this write would be reimplementing a decision it does not own.
fn write_text(text: &str, stderr: bool, code: u8) -> ExitCode {
    let selector = if stderr {
        OutputTo::Stderr
    } else {
        OutputTo::Stdout
    };
    match agent_first_data::write_raw(text, selector) {
        Ok(()) => ExitCode::from(code),
        Err(_) => ExitCode::FAILURE,
    }
}

fn write_event(event: &Event, plan: &OutputPlan, is_error: bool) -> ExitCode {
    let mut text = render(
        event.as_value(),
        output_format(plan),
        &agent_first_data::OutputOptions::default(),
    );
    if !text.ends_with('\n') {
        text.push('\n');
    }
    let stderr = is_error || plan.destination() == Some("stderr");
    write_text(&text, stderr, if is_error { 2 } else { 0 })
}

fn write_startup_error(code: &str, message: &str) -> ExitCode {
    let event = match json_error(code, message).build() {
        Ok(event) => event,
        Err(_) => return ExitCode::FAILURE,
    };
    let mut text = render(
        event.as_value(),
        OutputFormat::Json,
        &agent_first_data::OutputOptions::default(),
    );
    text.push('\n');
    write_text(&text, true, 1)
}

fn main() -> ExitCode {
    let cli = match cli_spec() {
        Ok(cli) => cli,
        Err(error) => return write_startup_error("cli_spec_invalid", &error.to_string()),
    };
    let app = match cli.bind_actions([
        ("echo", echo as fn(&ResolvedInvocation) -> Event),
        ("ping", ping as fn(&ResolvedInvocation) -> Event),
        ("release", release as fn(&ResolvedInvocation) -> Event),
    ]) {
        Ok(app) => app,
        Err(error) => return write_startup_error("cli_actions_invalid", &error.to_string()),
    };
    let outcome = match app.resolve_from(std::env::args_os()) {
        Ok(outcome) => outcome,
        Err(error) => {
            let plan = OutputPlan::Protocol {
                lifecycle: agent_first_data::OutputLifecycle::Finite,
                format: "json".to_string(),
                destination: "stderr".to_string(),
                stdout_file: None,
                stderr_file: None,
            };
            return write_event(&cli_error_event(&error), &plan, true);
        }
    };
    match outcome {
        CliOutcome::Run(invocation) => {
            let event = app.execute(&invocation);
            write_event(&event, invocation.output_plan(), false)
        }
        // Injected into every registry: an offline reference without registering
        // a command, and without a line of the agent's discovery surface.
        CliOutcome::Docs(docs) => write_text(
            &agent_first_data::render_cli_reference(&cli),
            docs.output_plan().destination() == Some("stderr"),
            0,
        ),
        CliOutcome::Help(help) if help.output_plan().format() == Some("plain") => write_text(
            &help.plain(),
            help.output_plan().destination() == Some("stderr"),
            0,
        ),
        CliOutcome::Help(help) => write_event(&cli_help_event(&help), help.output_plan(), false),
        CliOutcome::Version(version) => {
            write_event(&cli_version_event(&version), version.output_plan(), false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_comes_from_registered_combinations() {
        let cli = cli_spec().unwrap();
        let CliOutcome::Help(help) = cli.resolve_from(["agent-cli", "echo", "--help"]).unwrap()
        else {
            panic!("expected help");
        };
        let [shape] = help.model().shapes.as_slice() else {
            panic!("expected one shape");
        };
        assert_eq!(shape.id, "echo");
        // One round trip, so the optional argument is in the answer rather
        // than behind a second call the caller might never make.
        assert!(shape.usage.contains("[--dry-run]"), "{}", shape.usage);
    }

    #[test]
    fn a_subcommand_owns_the_version_name_the_root_answers() {
        let cli = cli_spec().unwrap();
        let CliOutcome::Run(invocation) = cli
            .resolve_from(["agent-cli", "release", "--version", "1.2.0"])
            .unwrap()
        else {
            panic!("expected a run outcome");
        };
        assert_eq!(invocation.required("version").as_str(), Some("1.2.0"));

        let CliOutcome::Version(version) = cli.resolve_from(["agent-cli", "--version"]).unwrap()
        else {
            panic!("expected version");
        };
        assert_eq!(version.name(), "agent-cli");
    }

    #[test]
    fn known_but_unregistered_mix_is_rejected() {
        let cli = cli_spec().unwrap();
        let error = cli
            .resolve_from(["agent-cli", "ping", "--host", "example.com", "--dry-run"])
            .unwrap_err();
        assert_eq!(error.rule, agent_first_data::CliErrorRule::UnknownArgument);
    }
}
