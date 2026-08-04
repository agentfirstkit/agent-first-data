//! Canonical closed-world AFDATA CLI example.
//!
//! Run:
//! `cargo run --example agent_cli -- --help`
//! `cargo run --example agent_cli -- echo hello --dry-run`
//! `cargo run --example agent_cli -- ping --host example.com`

use agent_first_data::{
    ArgSpec, BoundOutcome, CliEmitter, CliSpec, Combination, CommandSpec, Event, OutputFormat,
    OutputPlan, OutputSpec, OutputTo, ResolvedInvocation, build_cli_error, cli_error_event,
    cli_help_event, cli_version_event, json_error, json_result, render_cli_reference,
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
    let Some(message) = invocation.required("message").as_str() else {
        return build_cli_error("resolved echo invocation is missing `message`", None);
    };
    json_result(json!({
        "code": "echo",
        "message": message,
        "dry_run": invocation.optional("dry_run").and_then(|value| value.as_bool()).unwrap_or(false),
    }))
    .build()
}

fn ping(invocation: &ResolvedInvocation) -> Event {
    let Some(host) = invocation.required("host").as_str() else {
        return build_cli_error("resolved ping invocation is missing `host`", None);
    };
    json_result(json!({
        "code": "ping",
        "host": host,
    }))
    .build()
}

fn release(invocation: &ResolvedInvocation) -> Event {
    let Some(version) = invocation.required("version").as_str() else {
        return build_cli_error("resolved release invocation is missing `version`", None);
    };
    json_result(json!({
        "code": "release",
        "version": version,
    }))
    .build()
}

fn plan_format(plan: &OutputPlan) -> OutputFormat {
    plan.output_format().unwrap_or(OutputFormat::Json)
}

fn plan_destination(plan: &OutputPlan) -> OutputTo {
    plan.output_to().unwrap_or(OutputTo::Stdout)
}

fn emit_event(event: Event, plan: &OutputPlan, success_code: u8) -> ExitCode {
    let mut emitter = CliEmitter::from_output_to(plan_destination(plan), plan_format(plan))
        .with_strict_protocol();
    ExitCode::from(emitter.finish(event, success_code))
}

fn write_text(text: &str, plan: &OutputPlan) -> ExitCode {
    match agent_first_data::write_raw(text, plan_destination(plan)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(4),
    }
}

fn write_startup_error(code: &str, message: &str) -> ExitCode {
    let event = match json_error(code, message).build() {
        Ok(event) => event,
        Err(_) => return ExitCode::FAILURE,
    };
    let mut emitter =
        CliEmitter::from_output_to(OutputTo::Stderr, OutputFormat::Json).with_strict_protocol();
    ExitCode::from(emitter.finish(event, 1))
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
            let mut emitter = CliEmitter::from_output_to(OutputTo::Stderr, OutputFormat::Json)
                .with_strict_protocol();
            return ExitCode::from(emitter.finish(cli_error_event(&error), error.exit_code()));
        }
    };
    match outcome {
        BoundOutcome::Run(invocation) => {
            // `run` consumes the invocation, so take the plan first — it is
            // readable before the handler runs precisely so a caller can set up
            // its output before anything is emitted.
            let plan = invocation.output_plan().clone();
            let event = invocation.run();
            let exit_code = if event.as_value()["kind"].as_str() == Some("error") {
                1
            } else {
                0
            };
            emit_event(event, &plan, exit_code)
        }
        BoundOutcome::Docs(docs) => write_text(&render_cli_reference(&cli), docs.output_plan()),
        BoundOutcome::Help(help)
            if help.output_plan().output_format() == Some(OutputFormat::Plain) =>
        {
            write_text(&help.plain(), help.output_plan())
        }
        BoundOutcome::Help(help) => emit_event(cli_help_event(&help), help.output_plan(), 0),
        BoundOutcome::Version(version) => {
            emit_event(cli_version_event(&version), version.output_plan(), 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_first_data::CliOutcome;

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
