use super::*;

fn protocol() -> OutputSpec {
    OutputSpec::protocol_finite(
        ["json", "yaml", "plain"],
        ["split", "stdout", "stderr"],
        "json",
        "split",
    )
    .file_sinks(["stdout", "stderr"])
}

fn sample_spec() -> Result<BuiltCliSpec, CliSpecError> {
    CliSpec::new("demo", "1.0.0")
        .lifecycle_output(protocol())
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["query"])
                .arg(ArgSpec::option_enum("--mode", ["cli", "pipe"]).default("cli"))
                .arg(ArgSpec::option("--dsn", "DSN").sensitive())
                .arg(ArgSpec::option("--sql", "SQL"))
                .arg(ArgSpec::flag("--dry-run"))
                .combination(
                    Combination::new("query")
                        .action("query")
                        .about("Run one statement")
                        .fixed("mode", "cli")
                        .required(["dsn", "sql"])
                        .optional(["dry_run"])
                        .output(protocol()),
                )
                .combination(
                    Combination::new("pipe")
                        .action("pipe")
                        .about("Serve framed requests")
                        .fixed("mode", "pipe")
                        .required(["dsn"])
                        .output(OutputSpec::protocol_stream(
                            ["json"],
                            ["stdout", "stderr"],
                            "json",
                            "stdout",
                        )),
                ),
        )
        .build()
}

#[test]
fn resolves_one_registered_combination() {
    let spec = sample_spec().unwrap();
    let outcome = spec
        .resolve_from([
            "demo",
            "query",
            "--dsn",
            "secret",
            "--sql",
            "select 1",
            "--dry-run",
        ])
        .unwrap();
    let CliOutcome::Run(invocation) = outcome else {
        panic!("expected run");
    };
    assert_eq!(invocation.combination_id(), "query");
    assert_eq!(invocation.action_id(), "query");
    assert_eq!(invocation.required("dsn").as_str(), Some("secret"));
    assert_eq!(invocation.required("mode").as_str(), Some("cli"));
}

#[test]
fn rejects_known_but_unregistered_argument_combination_without_values() {
    let spec = sample_spec().unwrap();
    let error = spec
        .resolve_from([
            "demo",
            "query",
            "--mode",
            "pipe",
            "--dsn",
            "do-not-leak",
            "--dry-run",
        ])
        .unwrap_err();
    assert_eq!(error.rule, CliErrorRule::UnregisteredCombination);
    assert!(!error.message.contains("do-not-leak"));
}

#[test]
fn overlap_accounts_for_default_satisfying_fixed() {
    let result = CliSpec::new("demo", "1")
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["run"])
                .arg(ArgSpec::option_enum("--mode", ["cli", "pipe"]).default("cli"))
                .combination(Combination::new("fixed").action("run").fixed("mode", "cli"))
                .combination(Combination::new("absent").action("run")),
        )
        .build();
    assert_eq!(result.unwrap_err().rule, "overlapping_combinations");
}

#[test]
fn non_default_fixed_does_not_overlap_absent() {
    let result = CliSpec::new("demo", "1")
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["run"])
                .arg(ArgSpec::option_enum("--mode", ["cli", "pipe"]).default("cli"))
                .combination(
                    Combination::new("fixed")
                        .action("run")
                        .about("Explicit pipe mode")
                        .fixed("mode", "pipe"),
                )
                .combination(
                    Combination::new("absent")
                        .action("run")
                        .about("Mode left at its default"),
                ),
        )
        .build();
    assert!(result.is_ok());
}

#[test]
fn help_is_generated_from_combinations() {
    let spec = sample_spec().unwrap();
    let outcome = spec.resolve_from(["demo", "query", "--help"]).unwrap();
    let CliOutcome::Help(help) = outcome else {
        panic!("expected help");
    };
    // One round trip answers completely: every shape, each with the optional
    // arguments a second level would have hidden.
    let shapes = &help.model().shapes;
    assert_eq!(
        shapes
            .iter()
            .map(|shape| shape.id.as_str())
            .collect::<Vec<_>>(),
        ["query", "pipe"]
    );
    // `--mode` is fixed to `cli` here, but its own default already satisfies
    // that, so the parser accepts the call without it — help has to say so
    // rather than describe a stricter call than the one it will run.
    assert!(
        shapes[0]
            .usage
            .starts_with("demo query [--mode cli] --dsn <DSN> --sql <SQL> [--dry-run]"),
        "{}",
        shapes[0].usage
    );
    assert_eq!(
        help.model().defaults.get("--mode"),
        Some(&CliValue::String("cli".to_string()))
    );
    assert!(shapes.iter().all(|shape| shape.about.is_some()));
}

#[test]
fn fixed_argument_the_default_cannot_satisfy_stays_required() {
    let spec = CliSpec::new("demo", "1")
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["run"])
                .arg(ArgSpec::option_enum("--mode", ["cli", "pipe"]).default("cli"))
                .combination(
                    Combination::new("piped")
                        .action("run")
                        .fixed("mode", "pipe"),
                ),
        )
        .build()
        .unwrap();
    let CliOutcome::Help(help) = spec.resolve_from(["demo", "run", "--help"]).unwrap() else {
        panic!("expected help");
    };
    // The default is `cli`, so reaching this shape means writing `--mode pipe`.
    assert!(
        help.model().shapes[0].usage.contains("--mode pipe"),
        "{}",
        help.model().shapes[0].usage
    );
    assert!(!help.model().shapes[0].usage.contains("[--mode"));
    assert!(!help.model().defaults.contains_key("--mode"));
}

#[test]
fn help_names_enum_values_and_keys_notes_by_spelling() {
    let spec = CliSpec::new("demo", "1")
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["run"])
                .arg(ArgSpec::positional("file", 0, "FILE").about("Input file"))
                .arg(
                    ArgSpec::option_enum("--mode", ["cli", "pipe"])
                        .value_name("MODE")
                        .about("How to read input"),
                )
                .combination(
                    Combination::new("run")
                        .action("run")
                        .required(["file"])
                        .optional(["mode"]),
                ),
        )
        .build()
        .unwrap();
    let CliOutcome::Help(help) = spec.resolve_from(["demo", "run", "--help"]).unwrap() else {
        panic!("expected help");
    };
    // A closed value set that only an error message would reveal is registered
    // but undiscoverable, which is the defect this registry exists to remove.
    assert!(
        help.model().shapes[0].usage.contains("[--mode <cli|pipe>]"),
        "{}",
        help.model().shapes[0].usage
    );
    // Keys match the spelling in `usage`, so a caller looks up what it read.
    let notes = &help.model().notes;
    assert_eq!(
        notes.get("--mode").map(String::as_str),
        Some("How to read input")
    );
    assert_eq!(notes.get("FILE").map(String::as_str), Some("Input file"));
}

#[test]
fn output_contract_is_checked_after_shape_selection() {
    let spec = sample_spec().unwrap();
    let error = spec
        .resolve_from([
            "demo", "query", "--mode", "pipe", "--dsn", "secret", "--output", "yaml",
        ])
        .unwrap_err();
    assert_eq!(error.rule, CliErrorRule::InvalidArgumentValue);
    // The rejected argument is named in the message, not in a second field.
    assert!(error.message.contains("--output"), "{}", error.message);
}

#[test]
fn raw_output_rejects_format_as_unregistered() {
    let spec = CliSpec::new("demo", "1")
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["raw"]).combination(
                Combination::new("raw")
                    .action("raw")
                    .output(OutputSpec::raw()),
            ),
        )
        .build()
        .unwrap();
    let error = spec
        .resolve_from(["demo", "raw", "--output", "json"])
        .unwrap_err();
    assert_eq!(error.rule, CliErrorRule::UnregisteredCombination);
}

#[test]
fn projected_defaults_are_local_to_selected_combination() {
    let spec = CliSpec::new("demo", "1")
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["run"])
                .arg(ArgSpec::option_enum("--mode", ["one", "two"]))
                .arg(ArgSpec::option("--only-one", "VALUE").default("default"))
                .combination(
                    Combination::new("one")
                        .about("First shape")
                        .action("run")
                        .fixed("mode", "one")
                        .optional(["only_one"]),
                )
                .combination(
                    Combination::new("two")
                        .action("run")
                        .about("Second shape")
                        .fixed("mode", "two"),
                ),
        )
        .build()
        .unwrap();
    let CliOutcome::Run(invocation) = spec.resolve_from(["demo", "run", "--mode", "two"]).unwrap()
    else {
        panic!("expected run");
    };
    assert!(invocation.optional("only_one").is_none());
}

#[test]
fn every_synthetic_invocation_resolves_to_its_own_combination() {
    let spec = sample_spec().unwrap();
    let fixtures = spec.synthetic_invocations();
    assert_eq!(fixtures.len(), 2);
    for fixture in fixtures {
        let CliOutcome::Run(invocation) = spec.resolve_from(fixture.argv).unwrap() else {
            panic!("synthetic invocation did not resolve to Run");
        };
        assert_eq!(invocation.combination_id(), fixture.combination_id);
    }
}

#[test]
fn every_optional_subset_is_accepted() {
    let spec = sample_spec().unwrap();
    for suffix in [Vec::<&str>::new(), vec!["--dry-run"]] {
        let mut argv = vec!["demo", "query", "--dsn", "secret", "--sql", "select 1"];
        argv.extend(suffix);
        let CliOutcome::Run(invocation) = spec.resolve_from(argv).unwrap() else {
            panic!("optional subset did not resolve");
        };
        assert_eq!(invocation.combination_id(), "query");
    }
}

#[test]
fn action_binding_requires_exact_distinct_coverage() {
    fn handler(_: &ResolvedInvocation) {}

    let spec = sample_spec().unwrap();
    let missing = spec.bind_actions([("query", handler as fn(&ResolvedInvocation))]);
    assert!(matches!(
        missing,
        Err(CliSpecError {
            rule: "action_handler_coverage",
            ..
        })
    ));

    let exact = spec.bind_actions([
        ("query", handler as fn(&ResolvedInvocation)),
        ("pipe", handler as fn(&ResolvedInvocation)),
    ]);
    assert!(exact.is_ok());
}

#[test]
fn secret_values_never_enter_structured_cli_errors() {
    let spec = sample_spec().unwrap();
    let error = spec
        .resolve_from([
            "demo",
            "query",
            "--dsn",
            "postgres://user:password@example.test/db",
            "--sql",
            "select 1",
            "--unknown",
        ])
        .unwrap_err();
    let rendered = format!("{:?}|{}|{}", error.rule, error.message, error.hint);
    assert!(!rendered.contains("password"));
    assert_eq!(error.rule, CliErrorRule::UnknownArgument);
}

#[test]
fn malformed_specs_fail_before_resolution() {
    let uncovered = CliSpec::new("demo", "1")
        .command(CommandSpec::root().arg(ArgSpec::flag("--unused")))
        .build();
    assert_eq!(uncovered.unwrap_err().rule, "uncovered_argument");

    let fixed_non_enum = CliSpec::new("demo", "1")
        .command(
            CommandSpec::root()
                .arg(ArgSpec::option("--mode", "MODE"))
                .combination(Combination::new("bad").action("bad").fixed("mode", "cli")),
        )
        .build();
    assert_eq!(fixed_non_enum.unwrap_err().rule, "invalid_fixed_argument");

    let required_default = CliSpec::new("demo", "1")
        .command(
            CommandSpec::root()
                .arg(ArgSpec::option("--mode", "MODE").default("cli"))
                .combination(Combination::new("bad").action("bad").required(["mode"])),
        )
        .build();
    assert_eq!(
        required_default.unwrap_err().rule,
        "required_argument_default"
    );
}

#[test]
fn json_arguments_keep_their_source_text() {
    let spec = CliSpec::new("demo", "1")
        .command(
            CommandSpec::root()
                .arg(ArgSpec::option_json("--param", "JSON"))
                .combination(
                    Combination::new("only")
                        .action("only")
                        .required(["param"])
                        .output(OutputSpec::protocol_finite(
                            ["json"],
                            ["split"],
                            "json",
                            "split",
                        )),
                ),
        )
        .build()
        .unwrap();

    // An object argument must survive: the old parse produced a `serde_json::Value`,
    // so switching to raw text could silently start rejecting anything but strings.
    let CliOutcome::Run(invocation) = spec
        .resolve_from(["demo", "--param", r#"{"n":10000000000000000000000.5}"#])
        .unwrap()
    else {
        panic!("expected a run outcome");
    };
    assert_eq!(
        invocation.required("param").as_json_str(),
        Some(r#"{"n":10000000000000000000000.5}"#)
    );

    for raw in ["[1,2]", "\"hi\"", "3", "null"] {
        assert!(spec.resolve_from(["demo", "--param", raw]).is_ok(), "{raw}");
    }
    for raw in ["{", "1 2", "", "nope"] {
        let error = spec.resolve_from(["demo", "--param", raw]).unwrap_err();
        assert_eq!(error.rule, CliErrorRule::InvalidArgumentValue, "{raw}");
    }
}

#[test]
fn a_lone_combination_needs_no_description_of_its_own() {
    // Its command already says what it does, and repeating that verbatim is
    // noise; help inherits the command's `about` instead.
    let spec = CliSpec::new("demo", "1")
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["run"])
                .about("Run the only thing this command does")
                .arg(ArgSpec::flag("--now"))
                .combination(
                    Combination::new("only")
                        .action("run")
                        .optional(["now"])
                        .output(protocol()),
                ),
        )
        .build()
        .unwrap();
    let CliOutcome::Help(help) = spec.resolve_from(["demo", "run", "--help"]).unwrap() else {
        panic!("expected help");
    };
    assert_eq!(
        help.model().about.as_deref(),
        Some("Run the only thing this command does")
    );
    let [shape] = help.model().shapes.as_slice() else {
        panic!("expected one shape");
    };
    assert_eq!(shape.about, None, "a lone shape repeats nothing");
}

#[test]
fn sibling_combinations_must_say_how_they_differ() {
    let error = CliSpec::new("demo", "1")
        .command(CommandSpec::root())
        .command(
            CommandSpec::new(["run"])
                .about("Run something")
                .arg(ArgSpec::option_enum("--mode", ["one", "two"]))
                .combination(
                    Combination::new("first")
                        .action("run")
                        .about("The first way")
                        .fixed("mode", "one"),
                )
                .combination(
                    Combination::new("second")
                        .action("run")
                        .fixed("mode", "two"),
                ),
        )
        .build()
        .unwrap_err();
    assert_eq!(error.rule, "undescribed_combination");
}

#[test]
fn docs_is_injected_and_costs_the_agent_nothing() {
    let spec = sample_spec().unwrap();
    // Injected without being registered, and absent from what an agent reads.
    let CliOutcome::Help(help) = spec.resolve_from(["demo", "--help"]).unwrap() else {
        panic!("expected help");
    };
    assert!(!format!("{:?}", help.model()).contains("--docs"));

    let CliOutcome::Docs(docs) = spec.resolve_from(["demo", "--docs"]).unwrap() else {
        panic!("expected docs");
    };
    // A reference is raw bytes, so it must not inherit the protocol contract.
    assert!(matches!(docs.output_plan(), OutputPlan::Raw { .. }));

    // Root-only, and never mixed with another lifecycle entry or with argv.
    for argv in [
        vec!["demo", "query", "--docs"],
        vec!["demo", "--docs", "--version"],
        vec!["demo", "--docs", "--output", "json"],
    ] {
        assert_eq!(
            spec.resolve_from(argv.clone()).unwrap_err().rule,
            CliErrorRule::UnregisteredCombination,
            "{argv:?}"
        );
    }
}

/// A registry whose only application argument is `long`, declared on `path`.
fn declaring(path: &[&str], long: &str) -> Result<BuiltCliSpec, CliSpecError> {
    let argument = ArgSpec::option(long, "VALUE");
    let command = CommandSpec::new(path.to_vec())
        .arg(argument.clone())
        .combination(
            Combination::new("only")
                .action("only")
                .required([argument.argument_id])
                .output(protocol()),
        );
    let spec = CliSpec::new("demo", "1.0.0").lifecycle_output(protocol());
    if path.is_empty() {
        spec.command(command).build()
    } else {
        spec.command(CommandSpec::root()).command(command).build()
    }
}

#[test]
fn a_subcommand_may_declare_a_name_the_root_reserves() {
    // `tool release --version 1.2.0` names the release's version, not a request
    // for the tool's own. Reserving the spelling everywhere would take a name
    // the application legitimately owns and force it to invent `--app-version`.
    let spec = declaring(&["release"], "--version").unwrap();
    let CliOutcome::Run(invocation) = spec
        .resolve_from(["demo", "release", "--version", "1.2.0"])
        .unwrap()
    else {
        panic!("expected a run outcome");
    };
    assert_eq!(invocation.required("version").as_str(), Some("1.2.0"));

    // And the lifecycle answer is untouched where AFDATA does inject it.
    let CliOutcome::Version(version) = spec.resolve_from(["demo", "--version"]).unwrap() else {
        panic!("expected version");
    };
    assert_eq!(version.version(), "1.0.0");
}

#[test]
fn reservation_reaches_exactly_as_far_as_injection() {
    // Reserved everywhere, because AFDATA parses them at every command: taking
    // one would shadow a name the caller can always write.
    for long in [
        "--help",
        "--output",
        "--output-to",
        "--stdout-file",
        "--stderr-file",
    ] {
        for path in [&[][..], &["release"][..]] {
            assert_eq!(
                declaring(path, long).unwrap_err().rule,
                "reserved_long_argument",
                "{long} on {path:?}"
            );
        }
    }
    // Reserved at the root alone, because that is the only command that answers
    // them — a subcommand's is its own argument, and `tool release --version`
    // without a value is that argument missing one, not a version request.
    for long in ["--version", "--docs"] {
        assert_eq!(
            declaring(&[], long).unwrap_err().rule,
            "reserved_long_argument",
            "{long}"
        );
        let spec = declaring(&["release"], long).unwrap();
        assert_eq!(
            spec.resolve_from(["demo", "release", long])
                .unwrap_err()
                .rule,
            CliErrorRule::MissingArgumentValue,
            "{long}"
        );
    }
    // A subcommand that declares neither still gets the old answer: AFDATA
    // reaches no further than the root, so there is nothing to run.
    let spec = sample_spec().unwrap();
    for long in ["--version", "--docs"] {
        assert_eq!(
            spec.resolve_from(["demo", "query", long]).unwrap_err().rule,
            CliErrorRule::UnregisteredCombination,
            "{long}"
        );
    }
}

#[test]
fn an_empty_registry_is_rejected() {
    // This gate used to sit inside the per-command loop, where the list it
    // tested for emptiness was necessarily non-empty, so it never fired.
    let error = CliSpec::new("demo", "1").build().unwrap_err();
    assert_eq!(error.rule, "missing_commands");
}

#[test]
fn a_cli_cannot_redeclare_or_repeat_an_exit_code() {
    let build = |codes: &[(u8, &str)]| {
        let mut spec = CliSpec::new("demo", "1.0.0").lifecycle_output(protocol());
        for (code, meaning) in codes {
            spec = spec.exit_code(*code, *meaning);
        }
        spec.command(CommandSpec::root()).build()
    };
    // 0/1/2 are the protocol's; a tool restating one could contradict it.
    for reserved in [0u8, 1, 2] {
        assert_eq!(
            build(&[(reserved, "mine now")]).unwrap_err().rule,
            "reserved_exit_code",
            "{reserved}"
        );
    }
    assert_eq!(
        build(&[(3, "partial"), (3, "also partial")])
            .unwrap_err()
            .rule,
        "duplicate_exit_code"
    );
    assert_eq!(
        build(&[(3, "  ")]).unwrap_err().rule,
        "empty_exit_code_meaning"
    );
}
