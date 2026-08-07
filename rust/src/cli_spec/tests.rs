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

/// The source declaration is a registry fact, so its own rules are enforced
/// where every other argument rule is: at build time, before a host can ship a
/// flag that promises a source it could never read.
#[test]
fn a_source_set_must_belong_to_a_string_argument_with_no_default() {
    let spec = |argument: ArgSpec| {
        let id = argument.argument_id.clone();
        CliSpec::new("demo", "1.0.0")
            .lifecycle_output(protocol())
            .command(
                CommandSpec::root().arg(argument).combination(
                    Combination::new("only")
                        .action("only")
                        .required([id])
                        .output(protocol()),
                ),
            )
            .build()
    };

    // A flag carries no value, and an enum's values are already known.
    let error = spec(ArgSpec::flag("--verbose").sources(SourceSet::config()))
        .expect_err("a flag cannot take a source");
    assert_eq!(error.rule, "sources_value_type");
    let error = spec(ArgSpec::option_enum("--mode", ["a", "b"]).sources(SourceSet::config()))
        .expect_err("an enum cannot take a source");
    assert_eq!(error.rule, "sources_value_type");

    let error = spec(ArgSpec::option("--token", "SOURCE").sources(SourceSet::new([])))
        .expect_err("an empty set says nothing");
    assert_eq!(error.rule, "sources_empty");

    // A default that is a source would be read from the host's environment
    // without anyone asking, which is not what a default means here.
    let error = spec(
        ArgSpec::option("--token", "SOURCE")
            .sources(SourceSet::config())
            .default("env:TOKEN"),
    )
    .expect_err("a sourced argument cannot default");
    assert_eq!(error.rule, "sources_default");

    assert!(spec(ArgSpec::option("--token", "SOURCE").sources(SourceSet::config())).is_ok());
}

#[test]
fn source_sets_reject_ambiguous_host_grammars_and_deserialized_duplicates() {
    let spec = |sources: SourceSet| {
        CliSpec::new("demo", "1.0.0")
            .lifecycle_output(protocol())
            .command(
                CommandSpec::root()
                    .arg(ArgSpec::option("--token", "SOURCE").sources(sources))
                    .combination(
                        Combination::new("only")
                            .action("only")
                            .required(["token"])
                            .output(protocol()),
                    ),
            )
            .build()
    };

    let duplicate_built_in: SourceSet =
        serde_json::from_value(serde_json::json!({"schemes": ["env", "env"]})).unwrap();
    assert_eq!(
        spec(duplicate_built_in).unwrap_err().rule,
        "sources_duplicate_scheme"
    );

    let duplicate_host = SourceSet::new([])
        .host_scheme("container", "container:NAME")
        .host_scheme("container", "container:ID");
    assert_eq!(
        spec(duplicate_host).unwrap_err().rule,
        "host_source_duplicate"
    );

    for (sources, rule) in [
        (
            SourceSet::new([]).host_scheme("env", "env:NAME"),
            "host_source_name_reserved",
        ),
        (
            SourceSet::new([]).host_scheme("Container", "Container:NAME"),
            "host_source_name_invalid",
        ),
        (
            SourceSet::new([]).host_scheme("container", "vault:NAME"),
            "host_source_syntax_invalid",
        ),
        (
            SourceSet::new([]).host_scheme("container", "container:"),
            "host_source_syntax_invalid",
        ),
    ] {
        assert_eq!(spec(sources).unwrap_err().rule, rule);
    }

    // A host is allowed to be the only source. The serialized schema must
    // therefore allow an empty built-in list when host_schemes is non-empty.
    assert!(spec(SourceSet::new([]).host_scheme("container", "container:NAME")).is_ok());
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
    // A misspelled id is a handler defect, not a runtime condition. It fails
    // its type check instead of reading as a value, and `call_every_combination`
    // is what names it.
    assert!(invocation.required("misspelled").as_str().is_none());
    assert_eq!(
        invocation.output_plan().output_format(),
        Some(OutputFormat::Json)
    );
    assert_eq!(invocation.output_plan().output_to(), Some(OutputTo::Split));
    assert_eq!(invocation.output_plan().format(), Some("json"));
    assert_eq!(invocation.output_plan().destination(), Some("split"));
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
fn action_binding_rejects_an_invocation_from_another_registry() {
    fn query(_: &ResolvedInvocation) -> &'static str {
        "query"
    }
    fn pipe(_: &ResolvedInvocation) -> &'static str {
        "pipe"
    }

    let first = sample_spec().unwrap();
    let app = first
        .bind_actions([
            ("query", query as fn(&ResolvedInvocation) -> &'static str),
            ("pipe", pipe as fn(&ResolvedInvocation) -> &'static str),
        ])
        .unwrap();
    let BoundOutcome::Run(own) = app
        .resolve_from(["demo", "query", "--dsn", "secret", "--sql", "select 1"])
        .unwrap()
    else {
        panic!("expected run");
    };
    assert_eq!(own.run(), "query");
}

/// Dispatch is infallible because the handler is bound during resolution, by
/// the registry that owns it. There is no step that accepts an invocation from
/// somewhere else, so the mismatch the old runtime check guarded against is not
/// expressible — which is why no caller carries a branch for it.
#[test]
fn resolution_binds_the_handler_so_running_cannot_miss() {
    fn query(_: &ResolvedInvocation) -> &'static str {
        "query"
    }
    fn pipe(_: &ResolvedInvocation) -> &'static str {
        "pipe"
    }
    let app = sample_spec()
        .unwrap()
        .bind_actions([
            ("query", query as fn(&ResolvedInvocation) -> &'static str),
            ("pipe", pipe as fn(&ResolvedInvocation) -> &'static str),
        ])
        .unwrap();

    // Driven from the registry's own fixtures rather than hand-written argv, so
    // the test cannot drift from the spec it is checking.
    let fixtures = sample_spec().unwrap().synthetic_invocations();
    assert!(!fixtures.is_empty());
    for fixture in fixtures {
        let BoundOutcome::Run(invocation) = app.resolve_from(fixture.argv.clone()).unwrap() else {
            continue;
        };
        // The plan is readable before the handler runs, so a caller can still
        // install redirection first.
        assert!(invocation.output_plan().output_format().is_some());
        let action = invocation.invocation().action_id().to_string();
        assert_eq!(invocation.run(), action);
    }
}

/// The other half of `required` being infallible: a handler that reads an id
/// its combination does not declare is caught here, from a test, rather than
/// silently receiving a failed read in production.
#[test]
#[should_panic(expected = "does not declare argument id `nonexistent`")]
fn call_every_combination_names_a_handler_reading_an_undeclared_id() {
    fn reads_a_bad_id(invocation: &ResolvedInvocation) -> &'static str {
        let _ = invocation.required("nonexistent");
        "ok"
    }
    fn fine(_: &ResolvedInvocation) -> &'static str {
        "ok"
    }
    let app = sample_spec()
        .unwrap()
        .bind_actions([
            (
                "query",
                reads_a_bad_id as fn(&ResolvedInvocation) -> &'static str,
            ),
            ("pipe", fine as fn(&ResolvedInvocation) -> &'static str),
        ])
        .unwrap();

    app.call_every_combination();
}

/// It stays quiet when every handler reads only what it declared, and hands
/// back what each one produced so a fallible handler can be checked too —
/// otherwise a caller would still hand-roll the loop this replaces.
#[test]
fn call_every_combination_passes_and_returns_each_handler_result() {
    fn query(invocation: &ResolvedInvocation) -> &'static str {
        let _ = invocation.required("dsn");
        "query"
    }
    fn pipe(invocation: &ResolvedInvocation) -> &'static str {
        let _ = invocation.required("dsn");
        "pipe"
    }
    let app = sample_spec()
        .unwrap()
        .bind_actions([
            ("query", query as fn(&ResolvedInvocation) -> &'static str),
            ("pipe", pipe as fn(&ResolvedInvocation) -> &'static str),
        ])
        .unwrap();

    let results = app.call_every_combination();
    assert!(!results.is_empty());
    for (combination, produced) in &results {
        assert!(
            ["query", "pipe"].contains(produced),
            "combination `{combination}` produced {produced}"
        );
    }
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

    for argv in [
        vec!["demo", "postgres://user:command-secret@example.test/db"],
        vec!["demo", "query", "-short-secret"],
    ] {
        let error = spec.resolve_from(argv).unwrap_err();
        let rendered = format!("{:?}|{}|{}", error.rule, error.message, error.hint);
        assert!(!rendered.contains("secret"), "{rendered}");
    }
}

#[test]
fn separated_and_positional_negative_numbers_are_values() {
    let mut positional = ArgSpec::positional("limit", 0, "LIMIT");
    positional.value_type = ArgValueType::I64;
    let spec = CliSpec::new("demo", "1")
        .command(
            CommandSpec::root()
                .arg(ArgSpec::option_i64("--offset", "OFFSET"))
                .arg(positional)
                .combination(
                    Combination::new("numbers")
                        .action("numbers")
                        .required(["offset", "limit"]),
                ),
        )
        .build()
        .unwrap();

    let CliOutcome::Run(invocation) = spec.resolve_from(["demo", "--offset", "-2", "-3"]).unwrap()
    else {
        panic!("expected run");
    };
    assert_eq!(invocation.required("offset").as_i64(), Some(-2));
    assert_eq!(invocation.required("limit").as_i64(), Some(-3));
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

/// A shared argument is accepted at every command, wherever the user puts it on
/// the command line — which is the point: without it a root `--config` has to be
/// redeclared on every leaf, and users who typed `tool --config x sub` before
/// suddenly have to type `tool sub --config x`.
#[test]
fn a_shared_argument_is_accepted_at_every_command() {
    fn handler(invocation: &ResolvedInvocation) -> String {
        invocation
            .optional("config")
            .and_then(CliValue::as_str)
            .unwrap_or("default")
            .to_string()
    }
    let spec = CliSpec::new("demo", "1")
        .shared_arg(ArgSpec::option("--config", "PATH").default("default"))
        .command(
            CommandSpec::root().combination(Combination::new("root").action("root").output(
                OutputSpec::protocol_finite(["json"], ["split"], "json", "split"),
            )),
        )
        .command(
            CommandSpec::new(["sub"]).combination(Combination::new("sub").action("sub").output(
                OutputSpec::protocol_finite(["json"], ["split"], "json", "split"),
            )),
        )
        .build()
        .unwrap();
    let app = spec
        .bind_actions([
            ("root", handler as fn(&ResolvedInvocation) -> String),
            ("sub", handler),
        ])
        .unwrap();

    for argv in [
        vec!["demo", "--config", "here"],
        vec!["demo", "sub", "--config", "here"],
    ] {
        let BoundOutcome::Run(invocation) = app.resolve_from(argv.clone()).unwrap() else {
            panic!("expected a run for {argv:?}");
        };
        assert_eq!(invocation.run(), "here", "{argv:?}");
    }

    // Every combination still sees it as optional, so omitting it is legal.
    let BoundOutcome::Run(invocation) = app.resolve_from(["demo", "sub"]).unwrap() else {
        panic!("expected a run");
    };
    assert_eq!(invocation.run(), "default");
}

/// Sharing an argument and also declaring it on a command is a contradiction,
/// not a merge: one of the two spellings would silently win.
#[test]
fn redeclaring_a_shared_argument_fails_the_build() {
    let error = CliSpec::new("demo", "1")
        .shared_arg(ArgSpec::option("--config", "PATH"))
        .command(
            CommandSpec::root()
                .arg(ArgSpec::option("--config", "PATH"))
                .combination(
                    Combination::new("root")
                        .action("root")
                        .optional(["config"])
                        .output(OutputSpec::protocol_finite(
                            ["json"],
                            ["split"],
                            "json",
                            "split",
                        )),
                ),
        )
        .build()
        .unwrap_err();

    assert_eq!(error.rule, "shared_argument_redeclared");
}

/// A shared argument is held to the same reserved-name rule as a declared one.
#[test]
fn a_shared_argument_cannot_take_a_reserved_name() {
    let error = CliSpec::new("demo", "1")
        .shared_arg(ArgSpec::option("--output", "FORMAT"))
        .command(
            CommandSpec::root().combination(Combination::new("root").action("root").output(
                OutputSpec::protocol_finite(["json"], ["split"], "json", "split"),
            )),
        )
        .build()
        .unwrap_err();

    assert_eq!(error.rule, "reserved_long_argument");
}

/// A UUID argument is checked before the command runs, so a malformed one is a
/// usage error at exit 2 rather than something the handler has to re-report as
/// a domain failure.
#[test]
fn a_uuid_argument_is_validated_at_parse_time() {
    let spec = CliSpec::new("demo", "1")
        .command(
            CommandSpec::root()
                .arg(ArgSpec::option("--analysis-id", "UUID").uuid())
                .combination(
                    Combination::new("root")
                        .action("root")
                        .required(["analysis_id"])
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

    let ok = spec
        .resolve_from([
            "demo",
            "--analysis-id",
            "e8414c67-dc1c-4da3-a6f7-69541b0841c7",
        ])
        .unwrap();
    let CliOutcome::Run(invocation) = ok else {
        panic!("expected a run");
    };
    assert_eq!(
        invocation.required("analysis_id").as_str(),
        Some("e8414c67-dc1c-4da3-a6f7-69541b0841c7")
    );

    for bad in ["not-a-uuid", "e8414c67dc1c4da3a6f769541b0841c7", ""] {
        let error = spec
            .resolve_from(["demo", "--analysis-id", bad])
            .unwrap_err();
        assert_eq!(error.rule, CliErrorRule::InvalidArgumentValue, "{bad:?}");
        assert_eq!(error.exit_code(), 2);
    }
}

/// A range keeps a count that must fit a narrower integer out of the handler's
/// error channel: out of bounds is rejected with the other usage errors.
#[test]
fn an_integer_range_is_enforced_at_parse_time() {
    let spec = CliSpec::new("demo", "1")
        .command(
            CommandSpec::root()
                .arg(ArgSpec::option("--limit", "N").range(1, 1000))
                .combination(
                    Combination::new("root")
                        .action("root")
                        .required(["limit"])
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

    for good in ["1", "1000", "500"] {
        let CliOutcome::Run(invocation) = spec.resolve_from(["demo", "--limit", good]).unwrap()
        else {
            panic!("expected a run for {good}");
        };
        assert_eq!(
            invocation.required("limit").as_i64(),
            Some(good.parse().unwrap())
        );
    }

    for bad in ["0", "1001", "-1", "99999999999"] {
        let error = spec.resolve_from(["demo", "--limit", bad]).unwrap_err();
        assert_eq!(error.rule, CliErrorRule::InvalidArgumentValue, "{bad:?}");
        assert_eq!(error.exit_code(), 2);
    }
}

/// `shared_arg` removes the duplicate declaration; it does not change where the
/// argument may appear. The command path is still matched against the leading
/// tokens of argv, so the interleaved form stays an error.
///
/// Pinned explicitly because the first version of this feature shipped a doc
/// comment claiming the opposite, and the test beside it only covered the two
/// forms that already worked.
#[test]
fn a_shared_argument_does_not_move_the_command_path() {
    fn handler(_: &ResolvedInvocation) -> &'static str {
        "ok"
    }
    let spec = CliSpec::new("demo", "1")
        .shared_arg(ArgSpec::option("--config", "PATH").default("default"))
        .command(
            CommandSpec::root().combination(Combination::new("root").action("root").output(
                OutputSpec::protocol_finite(["json"], ["split"], "json", "split"),
            )),
        )
        .command(
            CommandSpec::new(["sub"]).combination(Combination::new("sub").action("sub").output(
                OutputSpec::protocol_finite(["json"], ["split"], "json", "split"),
            )),
        )
        .build()
        .unwrap();
    let app = spec
        .bind_actions([
            ("root", handler as fn(&ResolvedInvocation) -> &'static str),
            ("sub", handler),
        ])
        .unwrap();

    // Accepted: after the path.
    assert!(
        app.resolve_from(["demo", "sub", "--config", "here"])
            .is_ok_and(|outcome| matches!(outcome, BoundOutcome::Run(_)))
    );
    // Rejected: before it. `sub` is read as a positional, not a command.
    let error = app
        .resolve_from(["demo", "--config", "here", "sub"])
        .unwrap_err();
    assert_eq!(error.rule, CliErrorRule::UnexpectedPositional);
}

/// A help-only path node carries no combinations, so a shared argument must not
/// be spliced into it — the registry would fail to build for declaring an
/// argument no combination covers.
#[test]
fn a_shared_argument_skips_a_help_only_command() {
    let spec = CliSpec::new("demo", "1")
        .shared_arg(ArgSpec::option("--config", "PATH").default("default"))
        .command(
            CommandSpec::root().combination(Combination::new("root").action("root").output(
                OutputSpec::protocol_finite(["json"], ["split"], "json", "split"),
            )),
        )
        // Exists only so `demo group --help` lists its children.
        .command(CommandSpec::new(["group"]))
        .command(
            CommandSpec::new(["group", "leaf"]).combination(
                Combination::new("leaf")
                    .action("leaf")
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

    assert!(matches!(
        spec.resolve_from(["demo", "group", "leaf", "--config", "here"]),
        Ok(CliOutcome::Run(_))
    ));
}

/// A default is held to the same range as a value typed on the command line.
/// Otherwise a registry can ship a default its own argument rejects, and
/// nothing downstream can report it — the value never passes through the parser.
#[test]
fn a_default_outside_the_range_fails_the_build() {
    let error = CliSpec::new("demo", "1")
        .command(
            CommandSpec::root()
                .arg(ArgSpec::option("--limit", "N").range(1, 100).default_i64(0))
                .combination(
                    Combination::new("root")
                        .action("root")
                        .optional(["limit"])
                        .output(OutputSpec::protocol_finite(
                            ["json"],
                            ["split"],
                            "json",
                            "split",
                        )),
                ),
        )
        .build()
        .unwrap_err();

    assert!(
        error.message.contains("0..=100") || error.message.contains("1..=100"),
        "{}",
        error.message
    );
}

/// Putting the command after its arguments is the one mistake this rule
/// reliably produces, and "unexpected positional argument" does not name the
/// fix. When the offending token is a registered command name, say so.
#[test]
fn a_misordered_command_name_says_what_is_wrong() {
    let spec = CliSpec::new("tool", "1")
        .shared_arg(ArgSpec::option("--config", "PATH").default("cfg"))
        .command(
            CommandSpec::root().combination(Combination::new("root").action("root").output(
                OutputSpec::protocol_finite(["json"], ["split"], "json", "split"),
            )),
        )
        .command(
            CommandSpec::new(["sub"]).combination(Combination::new("sub").action("sub").output(
                OutputSpec::protocol_finite(["json"], ["split"], "json", "split"),
            )),
        )
        .build()
        .unwrap();

    let error = spec
        .resolve_from(["tool", "--config", "x", "sub"])
        .unwrap_err();
    assert_eq!(error.rule, CliErrorRule::UnexpectedPositional);
    assert_eq!(error.message, "command name must come before its arguments");

    // A token that is not a command keeps the general message — the sharper one
    // would be a guess.
    let error = spec
        .resolve_from(["tool", "--config", "x", "nonsense"])
        .unwrap_err();
    assert_eq!(error.message, "unexpected positional argument");
}
