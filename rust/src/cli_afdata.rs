//! AFDATA adapter for the closed-world CLI core.
//!
//! [`crate::cli_spec`] decides whether an invocation is legal and what it
//! resolved to. It deliberately knows nothing about how AFDATA expresses that
//! decision. This module is the only place where the two meet: it turns a
//! resolved outcome into a protocol event and owns AFDATA's `_secret` naming
//! convention.

use crate::cli_spec::SourceScheme;
use crate::cli_spec::{
    ArgValueType, BuiltCliSpec, CliError, CliSpec, CliSpecError, ResolvedHelp, ResolvedVersion,
};
use crate::protocol::{Event, json_error, json_result};

/// Build a registry under AFDATA's naming conventions.
///
/// `ArgSpec::sensitive` is not an independent switch. AFDATA already decides
/// what a secret is by the `_secret` suffix, and the same convention drives
/// config keys, log fields, and redaction. Deriving the bit from the argument
/// id keeps one source of truth: an argument cannot be sensitive to the parser
/// while staying invisible to redaction downstream. Marking a differently named
/// argument sensitive is the reverse mistake, so it fails the build instead of
/// silently diverging.
pub fn build_afdata_cli(mut spec: CliSpec) -> Result<BuiltCliSpec, CliSpecError> {
    for command in &mut spec.commands {
        for argument in &mut command.arguments {
            let suffixed = argument.argument_id.ends_with("_secret");
            let is_flag = matches!(argument.value_type, ArgValueType::Flag);
            if argument.sensitive && !suffixed {
                return Err(CliSpecError {
                    rule: "sensitive_without_secret_suffix",
                    message: format!(
                        "argument `{}` is marked sensitive; rename it to `{}_secret` so AFDATA \
                         redaction covers it too",
                        argument.argument_id, argument.argument_id
                    ),
                });
            }
            if argument.sensitive && is_flag {
                return Err(CliSpecError {
                    rule: "sensitive_flag",
                    message: format!(
                        "flag `{}` is marked sensitive, but a flag carries no value to redact",
                        argument.argument_id
                    ),
                });
            }
            // A flag has no value, so `_secret` in its name says what the flag
            // is *about* — `--reveal-secret` asks to reveal one, it does not
            // carry one. Marking it would put the sensitive bit on something
            // that structurally cannot leak, which reads as a real credential
            // to anything downstream that trusts the bit.
            argument.sensitive = suffixed && !is_flag;

            // A prompt suppresses terminal echo and blocks until a person
            // types — the first only means something for a secret, and the
            // second is a hang in the agent-run case this CLI is for. So the
            // source is available exactly where it earns its cost.
            if let Some(sources) = &argument.sources
                && sources.accepts(SourceScheme::Prompt)
                && !argument.sensitive
            {
                return Err(CliSpecError {
                    rule: "prompt_source_without_secret",
                    message: format!(
                        "argument `{}` accepts the `prompt` source; rename it to `{}_secret`, or \
                         drop the source — prompting blocks on a terminal for a value that is not \
                         a credential",
                        argument.argument_id, argument.argument_id
                    ),
                });
            }
        }
    }
    spec.build()
}

/// Wrap a resolved help response in a `cli-help-v2` result event.
pub fn cli_help_event(help: &ResolvedHelp) -> Event {
    json_result(serde_json::json!({
        "code": "help",
        "help": help.model(),
    }))
    .build()
}

/// Wrap a resolved version response in a protocol result event.
///
/// This delegates to [`crate::build_cli_version`] rather than assembling its
/// own payload. There is one version shape — `{code, name, version}` plus
/// `display_name`/`build` when the registry carries them — and it is the same
/// one the Go, Python, and TypeScript SDKs emit.
pub fn cli_version_event(version: &ResolvedVersion) -> Event {
    crate::cli::build_cli_version(
        version.name(),
        version.display_name(),
        version.version(),
        version.build(),
    )
}

/// Static prose for the generated CLI reference.
///
/// Kept as Markdown files rather than escaped Rust string literals: these are
/// paragraphs, and as `\`-continued literals they cannot be read as prose in a
/// diff, need every backtick and quote escaped, and turn a wording change into
/// an exercise in line continuations. The surrounding structure — headings,
/// tables, per-command sections — stays in code, because that part is generated
/// from the registry and is not prose.
///
/// This reference is emitted by every spore's `--docs`, not just afdata's, so
/// the text is shared library data and belongs beside the generator.
const SHAPES_PROSE: &str = include_str!("cli_reference/shapes.md");
const CLI_ERRORS_PROSE: &str = include_str!("cli_reference/cli-errors.md");

/// Render an offline Markdown reference for a whole registry.
///
/// help-v2 is deliberately lossy: it answers "how do I call this" one command
/// at a time, in as few tokens as possible. A reference manual wants the
/// opposite, so it is a separate capability rather than a format on the
/// discovery path — the agent's `--help` never grows a documentation mode, and
/// the manual can never disagree with the parser, because both are this
/// registry. `--docs` is injected into every registry, so a tool exposes this
/// without registering a command, and without spending a line of its
/// subcommand listing on something no agent calls.
///
/// What is deliberately *not* repeated: a lone combination's description and
/// id — the command heading already carries the description, and the id only
/// matters for telling siblings apart — plus the shared output defaults and
/// each command's arguments per combination. help repeats those because each help
/// response is read alone; a document is read in order.
pub fn render_cli_reference(cli: &BuiltCliSpec) -> String {
    let spec = cli.spec();
    let name = spec.name.as_str();
    let mut commands: Vec<&crate::cli_spec::CommandSpec> = spec
        .commands
        .iter()
        .filter(|command| !command.combinations.is_empty())
        .collect();
    commands.sort_by(|left, right| left.command_path.cmp(&right.command_path));

    let path_of = |command: &crate::cli_spec::CommandSpec| {
        if command.command_path.is_empty() {
            name.to_string()
        } else {
            format!("{name} {}", command.command_path.join(" "))
        }
    };

    let mut out = String::new();
    out.push_str(&format!("# {name} CLI reference\n\n"));
    out.push_str(&format!(
        "<!-- Generated by `{name} --docs`. Do not edit by hand. -->\n\n"
    ));
    if let Some(about) = &spec.about {
        out.push_str(&format!("{about}\n\n"));
    }
    out.push_str(&format!(
        "`{name}` is compiled from a closed `cli-spec-v1` registry: one source for argv parsing, \
         typed invocation values, which parameter combinations are legal, output contracts, and \
         help. An invocation runs only when it matches exactly one registered combination.\n\n"
    ));

    // Everything AFDATA registers on the caller's behalf, in one place. Split
    // across sections it reads as unrelated trivia, and `--version`/`--docs`
    // fall through the gap entirely — they belong to no command, so no command
    // section would ever mention them.
    let baseline = baseline_output(&commands);
    out.push_str("## Global arguments\n\n");
    // Not "no command declares them": `--version` and `--docs` are answered by
    // the root alone, so a command may declare its own argument under that
    // spelling — where one does, the entry below is still only the root's.
    out.push_str(
        "AFDATA registers these itself, so the syntax in [Commands](#commands) \
         leaves them out.\n\n",
    );
    out.push_str("| Argument | Where | What it does |\n|---|---|---|\n");
    out.push_str(
        "| `--help` | every command | Every legal shape of that command, complete, plus its \
         subcommands. JSON by default; `--output plain` for a terminal. |\n",
    );
    out.push_str(&format!(
        "| `--version` | {name} only | Name, version, and build identity as one protocol result. \
         |\n"
    ));
    out.push_str(&format!(
        "| `--docs` | {name} only | This document, rendered from the registry. |\n"
    ));
    if let Some(crate::cli_spec::OutputSpec::Protocol {
        formats,
        destinations,
        default_format,
        default_destination,
        ..
    }) = &baseline
    {
        out.push_str(&format!(
            "| `--output <FORMAT>` | per output contract | Render as {} (default \
             `{default_format}`). |\n",
            formats.join(", ")
        ));
        out.push_str(&format!(
            "| `--output-to <DESTINATION>` | per output contract | Route results and diagnostics \
             to {} (default `{default_destination}`). |\n",
            destinations.join(", ")
        ));
    }
    out.push_str(
        "| `--stdout-file <PATH>`, `--stderr-file <PATH>` | per output contract | Append that \
         stream to a file instead. |\n\n",
    );
    let baseline_line = baseline.as_ref().map(describe_output);
    // The table above already lists the arguments and their values; the only
    // thing left to say is what a successful call actually writes.
    if baseline.is_some() {
        out.push_str(
            "Success output is protocol events, on those terms, unless a command's own \
             **Output** line says otherwise.\n\n",
        );
    }
    out.push_str(SHAPES_PROSE);
    out.push('\n');

    out.push_str("## Commands\n\n");
    for command in &commands {
        let path = path_of(command);
        let anchor = path.replace(' ', "-");
        let about = command.about.as_deref().unwrap_or("");
        out.push_str(&format!("- [`{path}`](#{anchor}) — {about}\n"));
    }
    out.push('\n');

    for command in &commands {
        let path = path_of(command);
        out.push_str(&format!("### `{path}`\n\n"));
        if let Some(about) = &command.about {
            out.push_str(&format!("{about}\n\n"));
        }

        let Some(model) = cli.help(&command.command_path) else {
            continue;
        };
        for shape in &model.shapes {
            if model.shapes.len() > 1 {
                let differs = shape.about.as_deref().unwrap_or_default();
                out.push_str(&format!("#### `{}` — {differs}\n\n", shape.id));
            }
            out.push_str(&format!(
                "```\n{}\n```\n\n",
                trim_output_arguments(&shape.usage)
            ));
        }

        let combinations: Vec<&crate::cli_spec::Combination> =
            command.combinations.iter().collect();
        let contracts = output_contracts(&combinations);
        let is_baseline =
            matches!((contracts.as_slice(), &baseline_line), ([only], Some(line)) if only == line);
        if !is_baseline {
            out.push_str(&render_output(&contracts));
        }

        let documented: Vec<(&crate::cli_spec::ArgSpec, String)> = command
            .arguments
            .iter()
            .filter_map(|argument| Some((argument, argument.rendered_about()?)))
            .collect();
        if !documented.is_empty() {
            if model.shapes.len() > 1 {
                // One table per command, so it necessarily spans shapes that
                // cannot be used together; the syntax above is what says which
                // argument belongs where.
                out.push_str("Arguments across every shape above:\n\n");
            }
            out.push_str("| Argument | Meaning |\n|---|---|\n");
            for (argument, about) in documented {
                // The same spelling the usage line above uses, so the table can
                // be read against it without translating ids back to flags.
                out.push_str(&format!(
                    "| `{}` | {about} |\n",
                    crate::cli_spec::argument_key(argument)
                ));
            }
            out.push('\n');
        }
    }

    out.push_str("## Exit codes\n\n");
    out.push_str(
        "| Code | Meaning |\n|---|---|\n\
         | 0 | The command ran and succeeded. |\n\
         | 1 | The command ran and failed. The event carries a domain `error.code`. |\n\
         | 2 | The invocation was rejected before anything ran. `error.code` is one of the \
         `cli_*` codes below. |\n",
    );
    // Sorted rather than left in declaration order: the table is read by code,
    // and a registry author's ordering is not the reader's.
    let mut declared_exit_codes: Vec<&crate::cli_spec::ExitCodeSpec> =
        spec.exit_codes.iter().collect();
    declared_exit_codes.sort_by_key(|exit| exit.code);
    for exit in declared_exit_codes {
        out.push_str(&format!("| {} | {} |\n", exit.code, exit.meaning));
    }
    out.push_str(
        "\nThe split is the useful one for a caller: exit 2 means the call was never made, so \
         retrying it unchanged cannot help, while exit 1 means it was.\n\n",
    );

    out.push_str("## CLI errors\n\n");
    out.push_str(CLI_ERRORS_PROSE);
    out
}

/// One line per command saying what its success output is, because the usage
/// line can only show that by omission.
fn describe_output(output: &crate::cli_spec::OutputSpec) -> String {
    use crate::cli_spec::OutputSpec;
    match output {
        OutputSpec::Raw { file_sinks } => format!(
            "raw bytes on success; rejects `--output` and `--output-to`{}. Failures are still \
             strict JSON on stderr",
            render_file_sinks(file_sinks)
        ),
        OutputSpec::Protocol {
            formats,
            destinations,
            default_format,
            default_destination,
            file_sinks,
            ..
        } => format!(
            "protocol events; `--output` {} (default `{default_format}`), `--output-to` {} \
             (default `{default_destination}`){}",
            formats.join("/"),
            destinations.join("/"),
            render_file_sinks(file_sinks),
        ),
    }
}

/// Each distinct output contract a command exposes, in a stable order.
fn output_contracts(combinations: &[&crate::cli_spec::Combination]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for combination in combinations {
        let line = describe_output(&combination.output);
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    lines
}

/// The output contract most commands share, if there is one worth hoisting.
///
/// Deterministic: ties break on the rendered description, never on registration
/// order, so the same registry always renders the same document.
fn baseline_output(
    commands: &[&crate::cli_spec::CommandSpec],
) -> Option<crate::cli_spec::OutputSpec> {
    let mut counts: std::collections::BTreeMap<String, (usize, crate::cli_spec::OutputSpec)> =
        std::collections::BTreeMap::new();
    for command in commands {
        let mut contracts: Vec<&crate::cli_spec::OutputSpec> = Vec::new();
        for combination in &command.combinations {
            if !contracts.contains(&&combination.output) {
                contracts.push(&combination.output);
            }
        }
        if let [only] = contracts.as_slice() {
            let entry = counts
                .entry(describe_output(only))
                .or_insert((0, (*only).clone()));
            entry.0 += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.0.cmp(&right.1.0).then_with(|| right.0.cmp(&left.0)))
        .filter(|(_, (count, _))| *count > 1)
        .map(|(_, (_, spec))| spec)
}

fn render_output(contracts: &[String]) -> String {
    match contracts {
        [] => String::new(),
        [only] => format!("Output: {only}.\n\n"),
        many => {
            let mut out = String::from("Output differs by combination:\n\n");
            for line in many {
                out.push_str(&format!("- {line}\n"));
            }
            out.push('\n');
            out
        }
    }
}

/// Drop the trailing AFDATA output arguments from a usage line.
///
/// The compiler always renders them last and in a fixed order, and their names
/// are reserved, so no application argument can be mistaken for one. A document
/// states the output contract once per command; only `--help`, whose response
/// is read on its own, needs them inline.
fn trim_output_arguments(usage: &str) -> &str {
    let cut = ["[--output ", "[--stdout-file ", "[--stderr-file "]
        .iter()
        .filter_map(|marker| usage.find(marker))
        .min();
    match cut {
        Some(index) => usage[..index].trim_end(),
        None => usage,
    }
}

fn render_file_sinks(file_sinks: &[String]) -> String {
    let mut names: Vec<&str> = Vec::new();
    if file_sinks.iter().any(|sink| sink == "stdout") {
        names.push("`--stdout-file`");
    }
    if file_sinks.iter().any(|sink| sink == "stderr") {
        names.push("`--stderr-file`");
    }
    if names.is_empty() {
        String::new()
    } else {
        format!("; redirect with {}", names.join(" or "))
    }
}

/// Wrap a CLI-resolution failure in an event whose `code` names the failure.
///
/// The classification lives in `code`, the way `document_path_not_found` and
/// its siblings already do — not in a second field beside a generic
/// `cli_error`. One error taxonomy, one place to read it, and the skill's
/// standing instruction ("branch on `error.code`") covers CLI errors too.
///
/// The event never carries raw argument values; `message` names the offending
/// argument, and `hint` says what to run next.
pub fn cli_error_event(error: &CliError) -> Event {
    let builder = json_error(error.rule.code(), &error.message).hint(&error.hint);
    match builder.build() {
        Ok(event) => event,
        Err(_) => json_error("cli_error", "failed to build CLI error")
            .build()
            .unwrap_or_else(|_| {
                // All literals above are valid; this branch is unreachable
                // but keeps production code panic-free.
                json_result(serde_json::json!({"code":"internal_cli_error"})).build()
            }),
    }
}

/// The standard event for a program that misread its own resolved invocation.
///
/// Dispatch itself cannot fail —
/// [`crate::cli_spec::BoundCliSpec::resolve_from`] binds the handler while
/// resolving, so there is no undispatchable invocation to report. What remains
/// is a handler reading an argument the selected combination does not supply:
/// an id it does not declare, or one whose type does not match the accessor.
/// [`crate::cli_spec::BoundCliSpec::call_every_combination`] catches the first
/// from a test; this is for a program that also wants to say something at
/// runtime.
///
/// It is a defect in the program, never in what the user typed, and the code
/// and exit status have to say so. Reporting it as a usage error tells the
/// caller to fix their command line and retry, and retrying cannot help — a
/// mistake that has already been made in the wild, as
/// `cli_invalid_argument_value` at exit 2. Emit this and exit 1; the
/// application still owns the exit.
pub fn cli_invocation_invalid_event(detail: &str) -> Event {
    let builder = json_error("cli_invocation_invalid", detail)
        .hint("this is a defect in the program, not in the command; report it");
    match builder.build() {
        Ok(event) => event,
        Err(_) => json_error("cli_invocation_invalid", "invocation cannot be dispatched")
            .build()
            .unwrap_or_else(|_| {
                // The literals above are valid, so this is unreachable; it
                // keeps the path panic-free the way `cli_error_event` does.
                json_result(serde_json::json!({"code":"internal_cli_error"})).build()
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_spec::SourceSet;
    use crate::cli_spec::{ArgSpec, CliOutcome, Combination, CommandSpec, OutputSpec};

    fn output() -> OutputSpec {
        OutputSpec::protocol_finite(["json"], ["split"], "json", "split")
    }

    fn spec_with(argument: ArgSpec) -> CliSpec {
        let id = argument.argument_id.clone();
        CliSpec::new("demo", "1").command(
            CommandSpec::root().arg(argument).combination(
                Combination::new("only")
                    .action("only")
                    .required([id])
                    .output(output()),
            ),
        )
    }

    #[test]
    fn a_cli_declares_exit_codes_beyond_afdatas_own() {
        let spec = CliSpec::new("demo", "1.0.0")
            .lifecycle_output(output())
            .exit_code(4, "The output could not be written.")
            .exit_code(3, "The command ran and partly succeeded.")
            .command(CommandSpec::root())
            .build()
            .unwrap();
        let reference = render_cli_reference(&spec);
        let table = reference
            .split("## Exit codes")
            .nth(1)
            .expect("the reference documents exit codes");
        // Sorted by code, not by declaration order: the table is read by code.
        let partial = table.find("| 3 | The command ran and partly succeeded. |");
        let write_failed = table.find("| 4 | The output could not be written. |");
        assert!(partial.is_some() && write_failed.is_some(), "{table}");
        assert!(partial < write_failed, "{table}");
    }

    /// The syntax lives in the declaration, so help and `--docs` render it and
    /// no `about` string repeats it. This is the duplication the declaration
    /// exists to remove.
    #[test]
    fn a_declared_source_set_renders_itself_into_help_and_docs() {
        let built = build_afdata_cli(spec_with(
            ArgSpec::option("--token-secret", "SOURCE")
                .about("Token this host requires")
                .sources(SourceSet::config().host_scheme("container", "container:NAME")),
        ))
        .expect("registry builds");

        let reference = render_cli_reference(&built);
        assert!(
            reference.contains(
                "Token this host requires (the value, or where to read it: env:NAME, \
                 file[+FORMAT]:PATH#DOT_PATH, container:NAME, literal:VALUE)"
            ),
            "{reference}"
        );

        let CliOutcome::Help(help) = built
            .resolve_from(vec!["demo", "--help"])
            .expect("help resolves")
        else {
            panic!("--help must resolve to help");
        };
        let model = serde_json::to_value(help.model()).expect("help serializes");
        let note = model["notes"]["--token-secret"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(note.contains("file[+FORMAT]:PATH#DOT_PATH"), "{model}");
    }

    /// A value naming a scheme the argument does not accept is a usage error,
    /// beside the other argv rejections — and nothing was opened to find out.
    #[test]
    fn an_unaccepted_scheme_is_an_argv_rejection() {
        let built = build_afdata_cli(spec_with(
            ArgSpec::option("--token-secret", "SOURCE").sources(SourceSet::config()),
        ))
        .expect("registry builds");
        let error = built
            .resolve_from(vec!["demo", "--token-secret", "prompt"])
            .expect_err("prompt is not in config()");
        assert_eq!(
            error.rule,
            crate::cli_spec::CliErrorRule::InvalidArgumentValue
        );
        assert!(error.message.contains("env:NAME"), "{}", error.message);

        // …and one it does accept resolves to the raw string, unread.
        let outcome = built
            .resolve_from(vec!["demo", "--token-secret", "env:NAME"])
            .expect("env is accepted");
        let CliOutcome::Run(invocation) = outcome else {
            panic!("must resolve to a run");
        };
        assert_eq!(
            invocation.required("token_secret").as_str(),
            Some("env:NAME")
        );
    }

    /// Prompting suppresses echo and blocks on a terminal. Both only make sense
    /// for a credential, so the source is refused anywhere else.
    #[test]
    fn the_prompt_source_is_refused_on_a_non_secret_argument() {
        let error = build_afdata_cli(spec_with(
            ArgSpec::option("--label", "LABEL").sources(SourceSet::stream()),
        ))
        .expect_err("prompt on a non-secret argument");
        assert_eq!(error.rule, "prompt_source_without_secret");
        // The same set is fine once the name says it carries a credential.
        assert!(
            build_afdata_cli(spec_with(
                ArgSpec::option("--token-secret", "SOURCE").sources(SourceSet::stream())
            ))
            .is_ok()
        );
    }

    #[test]
    fn secret_suffix_drives_the_sensitive_bit() {
        let built = build_afdata_cli(spec_with(ArgSpec::option("--dsn-secret", "DSN"))).unwrap();
        let argument = &built.spec().commands[0].arguments[0];
        assert!(argument.sensitive);
    }

    #[test]
    fn sensitive_without_the_suffix_fails_the_build() {
        let error = build_afdata_cli(spec_with(ArgSpec::option("--token", "TOKEN").sensitive()))
            .unwrap_err();
        assert_eq!(error.rule, "sensitive_without_secret_suffix");
    }

    #[test]
    fn a_plain_argument_stays_insensitive() {
        let built = build_afdata_cli(spec_with(ArgSpec::option("--host", "HOST"))).unwrap();
        assert!(!built.spec().commands[0].arguments[0].sensitive);
    }

    // Locks the version payload shape. `--version` is a discovery entry point
    // agents parse, and it lost `display_name`/`build` once before by going
    // through a second, hand-rolled payload instead of `build_cli_version`.
    #[test]
    fn version_events_carry_the_full_documented_payload() {
        let built = CliSpec::new("demo", "1.2.3")
            .display_name("Demo Tool")
            .build_id("abc1234")
            .command(CommandSpec::root())
            .build()
            .unwrap();
        let CliOutcome::Version(version) = built.resolve_from(["demo", "--version"]).unwrap()
        else {
            panic!("expected a version outcome");
        };
        assert_eq!(
            cli_version_event(&version).as_value(),
            &serde_json::json!({
                "kind": "result",
                "result": {
                    "code": "version",
                    "name": "demo",
                    "display_name": "Demo Tool",
                    "version": "1.2.3",
                    "build": "abc1234",
                },
                "trace": {},
            })
        );
    }

    #[test]
    fn version_events_omit_absent_metadata() {
        let built = CliSpec::new("demo", "1.2.3")
            .command(CommandSpec::root())
            .build()
            .unwrap();
        let CliOutcome::Version(version) = built.resolve_from(["demo", "--version"]).unwrap()
        else {
            panic!("expected a version outcome");
        };
        let payload = serde_json::to_string(cli_version_event(&version).as_value()).unwrap();
        assert!(!payload.contains("display_name"), "{payload}");
        assert!(!payload.contains("build"), "{payload}");
    }

    #[test]
    fn cli_error_events_never_carry_a_secret_value() {
        let built = build_afdata_cli(spec_with(ArgSpec::option("--dsn-secret", "DSN"))).unwrap();
        let error = built
            .resolve_from([
                "demo",
                "--dsn-secret",
                "postgres://user:password@example.test/db",
                "--unknown",
            ])
            .unwrap_err();
        let serialized = serde_json::to_string(cli_error_event(&error).as_value()).unwrap();
        assert!(!serialized.contains("password"));
        // The classification is the code, not a field beside it.
        assert!(serialized.contains("\"code\":\"cli_unknown_argument\""));
        // The command to run next reaches the caller through `hint`, which is
        // the channel every error event already has.
        assert!(serialized.contains("run `demo --help`"));
    }

    /// The point of shipping this helper is that every consumer reports the
    /// same thing. Pin the code, so a consumer branching on it keeps working,
    /// and pin that it is not a `cli_*` usage code — a caller must not be told
    /// to fix their command line for a defect in the program.
    #[test]
    fn invocation_invalid_event_is_a_program_defect_not_a_usage_error() {
        let event = cli_invocation_invalid_event("no handler for this registry's invocation");
        let serialized = serde_json::to_string(event.as_value()).unwrap();

        assert!(serialized.contains("\"code\":\"cli_invocation_invalid\""));
        assert!(serialized.contains("no handler for this registry's invocation"));
        // The hint has to say who should act. A usage hint here would send the
        // caller to debug their own arguments for a dispatch-table bug.
        assert!(serialized.contains("defect in the program"));
        assert!(
            crate::validate_protocol_event(event.as_value(), true).is_ok(),
            "the helper must emit a strict event: {serialized}"
        );
    }

    #[test]
    fn a_secret_named_flag_is_not_marked_sensitive() {
        // `--reveal-secret` asks to reveal a secret; it does not carry one, and
        // a flag has no value that could leak. The suffix must not put the
        // sensitive bit on it.
        let built = build_afdata_cli(spec_with(ArgSpec::flag("--reveal-secret"))).unwrap();
        let argument = built
            .spec()
            .commands
            .iter()
            .flat_map(|command| &command.arguments)
            .find(|argument| argument.argument_id == "reveal_secret")
            .expect("the flag is registered");
        assert!(!argument.sensitive, "a flag has no value to redact");
    }

    #[test]
    fn marking_a_flag_sensitive_is_a_contradiction() {
        let error = build_afdata_cli(spec_with(ArgSpec::flag("--reveal-secret").sensitive()))
            .expect_err("a sensitive flag must not build");
        assert_eq!(error.rule, "sensitive_flag");
    }

    #[test]
    fn a_value_carrying_secret_argument_is_still_marked() {
        let built = build_afdata_cli(spec_with(ArgSpec::option("--dsn-secret", "DSN"))).unwrap();
        let argument = built
            .spec()
            .commands
            .iter()
            .flat_map(|command| &command.arguments)
            .find(|argument| argument.argument_id == "dsn_secret")
            .expect("the option is registered");
        assert!(argument.sensitive, "an option with a value still counts");
    }
}
