use super::error::{duplicate_error, invalid_value, missing_value};
use super::help::combination_usage;
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Validated registry used for resolution and help generation.
#[derive(Clone, Debug)]
pub struct BuiltCliSpec {
    pub(super) spec: CliSpec,
}

impl BuiltCliSpec {
    pub fn spec(&self) -> &CliSpec {
        &self.spec
    }

    pub fn resolve_from<I, S>(&self, args: I) -> Result<CliOutcome, CliError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let raw: Vec<OsString> = args.into_iter().map(Into::into).collect();
        self.resolve_os(&raw)
    }

    /// Generate type-correct argv for every combination and every fixed
    /// `one_of` member. These fixtures exercise the same normalized shapes
    /// used by help rendering without pretending help placeholders are argv.
    pub fn synthetic_invocations(&self) -> Vec<SyntheticInvocation> {
        let mut fixtures = Vec::new();
        for command in &self.spec.commands {
            for combination in &command.combinations {
                let mut variants = vec![vec![self.spec.name.clone()]];
                for part in &command.command_path {
                    for argv in &mut variants {
                        argv.push(part.clone());
                    }
                }
                for argument in &command.arguments {
                    let values: Vec<String> =
                        if let Some(fixed) = combination.fixed.get(&argument.argument_id) {
                            fixed.values().to_vec()
                        } else if combination
                            .required
                            .iter()
                            .any(|id| id == &argument.argument_id)
                        {
                            vec![synthetic_value(argument)]
                        } else {
                            continue;
                        };
                    let mut expanded = Vec::new();
                    for argv in variants {
                        for value in &values {
                            let mut candidate = argv.clone();
                            append_synthetic_argument(&mut candidate, argument, value);
                            expanded.push(candidate);
                        }
                    }
                    variants = expanded;
                }
                fixtures.extend(variants.into_iter().map(|argv| SyntheticInvocation {
                    command_path: command.command_path.clone(),
                    combination_id: combination.combination_id.clone(),
                    argv,
                }));
            }
        }
        fixtures
    }

    /// The help model for one command.
    ///
    /// The same model `--help` returns, reachable in-process. Build-time
    /// tooling — an offline reference renderer, for example — needs the whole
    /// registry, and this is how it consumes it without a full-spec dump on
    /// the agent's discovery path.
    pub fn help(&self, command_path: &[String]) -> Option<CliHelpV2> {
        let command = self
            .spec
            .commands
            .iter()
            .find(|candidate| candidate.command_path == command_path)?;
        Some(self.help_model(command))
    }

    /// Check exact action coverage and return an executable binding.
    pub fn bind_actions<R, I, S>(&self, handlers: I) -> Result<BoundCliSpec<R>, CliSpecError>
    where
        I: IntoIterator<Item = (S, fn(&ResolvedInvocation) -> R)>,
        S: Into<String>,
    {
        let expected: BTreeSet<&str> = self
            .spec
            .commands
            .iter()
            .flat_map(|command| command.combinations.iter())
            .map(|combination| combination.action_id.as_str())
            .collect();
        let mut actual = BTreeMap::new();
        for (action_id, handler) in handlers {
            let action_id = action_id.into();
            if actual.insert(action_id.clone(), handler).is_some() {
                return Err(CliSpecError::new(
                    "duplicate_action_handler",
                    format!("action `{action_id}` has more than one handler"),
                ));
            }
        }
        let actual_ids: BTreeSet<&str> = actual.keys().map(String::as_str).collect();
        if expected != actual_ids {
            let missing: Vec<&str> = expected.difference(&actual_ids).copied().collect();
            let extra: Vec<&str> = actual_ids.difference(&expected).copied().collect();
            return Err(CliSpecError::new(
                "action_handler_coverage",
                format!("handler coverage mismatch; missing={missing:?}, extra={extra:?}"),
            ));
        }
        Ok(BoundCliSpec {
            cli: self.clone(),
            handlers: actual,
        })
    }

    fn resolve_os(&self, raw: &[OsString]) -> Result<CliOutcome, CliError> {
        let mut utf8 = Vec::with_capacity(raw.len());
        for token in raw {
            let Some(token) = token.to_str() else {
                return Err(CliError::new(
                    CliErrorRule::InvalidUtf8,
                    self.spec.name.clone(),
                    "argv contains a token that is not valid UTF-8",
                ));
            };
            utf8.push(token.to_string());
        }
        let argv = if utf8.is_empty() { &[][..] } else { &utf8[1..] };
        let (command, consumed) = self.select_command(argv)?;
        let command_path = self.display_command_path(command);
        let parsed = tokenize(
            command,
            &argv[consumed..],
            &command_path,
            &self.spec.commands,
        )?;

        if parsed.control_count() > 0 {
            if parsed.control_count() != 1 || !parsed.application_values.is_empty() {
                return Err(CliError::unregistered(command_path));
            }
            // `--docs` renders the whole registry, which is raw bytes, not
            // protocol events. It therefore gets its own contract instead of
            // `lifecycle_output`, and must be settled before the shared
            // protocol plan below would wrongly accept `--output`.
            if parsed.docs {
                // Root-only, like `--version` below: past the command path the
                // spelling belongs to the application, and where it declared
                // one, `tokenize` bound the token to that argument and never
                // set this flag at all.
                if !command.command_path.is_empty() {
                    return Err(CliError::unregistered(command_path));
                }
                let contract = OutputSpec::raw()
                    .file_sinks(self.spec.lifecycle_output.file_sinks_ref().to_vec());
                let output = resolve_output(&contract, &parsed.output, &command_path)?;
                return Ok(CliOutcome::Docs(ResolvedDocs { output }));
            }
            let output =
                resolve_output(&self.spec.lifecycle_output, &parsed.output, &command_path)?;
            if parsed.help {
                return Ok(CliOutcome::Help(ResolvedHelp {
                    model: self.help_model(command),
                    output,
                }));
            }
            if parsed.version {
                if !command.command_path.is_empty() {
                    return Err(CliError::unregistered(command_path));
                }
                return Ok(CliOutcome::Version(ResolvedVersion {
                    name: self.spec.name.clone(),
                    version: self.spec.version.clone(),
                    display_name: self.spec.display_name.clone(),
                    build: self.spec.build.clone(),
                    output,
                }));
            }
        }

        let matching: Vec<&Combination> = command
            .combinations
            .iter()
            .filter(|combination| combination_matches(command, combination, &parsed))
            .collect();
        let Some(combination) = matching.first().copied() else {
            return Err(CliError::unregistered(command_path));
        };
        if matching.len() != 1 {
            return Err(CliError::new(
                CliErrorRule::UnregisteredCombination,
                command_path,
                "arguments match more than one registered CLI combination",
            ));
        }
        let output = resolve_output(&combination.output, &parsed.output, &command_path)?;
        let values = project_values(command, combination, &parsed);
        Ok(CliOutcome::Run(ResolvedInvocation {
            command_path: command.command_path.clone(),
            action_id: combination.action_id.clone(),
            combination_id: combination.combination_id.clone(),
            values,
            explicit_argument_ids: parsed.explicit_application_ids,
            output,
            strict_reads: false,
        }))
    }

    fn select_command<'a>(&'a self, argv: &[String]) -> Result<(&'a CommandSpec, usize), CliError> {
        let mut commands: Vec<&CommandSpec> = self.spec.commands.iter().collect();
        commands.sort_by_key(|command| std::cmp::Reverse(command.command_path.len()));
        if let Some(command) = commands.iter().copied().find(|command| {
            command.command_path.len() <= argv.len()
                && command
                    .command_path
                    .iter()
                    .zip(argv)
                    .all(|(expected, actual)| expected == actual)
        }) {
            let remaining = &argv[command.command_path.len()..];
            let has_children = self.spec.commands.iter().any(|candidate| {
                candidate.command_path.len() > command.command_path.len()
                    && candidate.command_path.starts_with(&command.command_path)
            });
            if remaining
                .first()
                .is_some_and(|token| !token.starts_with('-'))
                && has_children
            {
                return Err(CliError::new(
                    CliErrorRule::UnknownCommand,
                    self.display_command_path(command),
                    "unknown command",
                ));
            }
            return Ok((command, command.command_path.len()));
        }
        Err(CliError::new(
            CliErrorRule::UnknownCommand,
            self.spec.name.clone(),
            "unknown command",
        ))
    }

    fn display_command_path(&self, command: &CommandSpec) -> String {
        std::iter::once(self.spec.name.as_str())
            .chain(command.command_path.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn child_help_commands(&self, command: &CommandSpec) -> Vec<String> {
        let mut children: Vec<Vec<String>> = self
            .spec
            .commands
            .iter()
            .filter(|candidate| {
                candidate.command_path.len() == command.command_path.len() + 1
                    && candidate.command_path.starts_with(&command.command_path)
            })
            .map(|candidate| candidate.command_path.clone())
            .collect();
        children.sort();
        children
            .into_iter()
            .map(|path| format!("{} {} --help", self.spec.name, path.join(" ")))
            .collect()
    }

    fn help_model(&self, command: &CommandSpec) -> CliHelpV2 {
        let command_path = self.display_command_path(command);
        let mut shapes = Vec::new();
        let mut notes = BTreeMap::new();
        let mut defaults = BTreeMap::new();
        for combination in &command.combinations {
            let (usage, shape_notes, shape_defaults) =
                combination_usage(command, combination, &command_path, true);
            shapes.push(CliShape {
                id: combination.combination_id.clone(),
                // With one shape there is nothing to tell apart, so the
                // command's own description already covers it.
                about: if command.combinations.len() == 1 {
                    None
                } else {
                    combination.about.clone()
                },
                usage,
            });
            // Arguments belong to the command, so a note or default reached
            // through any shape is the same fact; collecting them once keeps
            // the response from repeating itself per shape.
            notes.extend(shape_notes);
            defaults.extend(shape_defaults);
        }
        CliHelpV2 {
            schema: "cli-help-v2".to_string(),
            command_path,
            // The root command has no description of its own — the registry's
            // does double duty, and without this fallback an agent's first
            // discovery call learns every subcommand but never what the tool is.
            about: command.about.clone().or_else(|| {
                command
                    .command_path
                    .is_empty()
                    .then(|| self.spec.about.clone())
                    .flatten()
            }),
            shapes,
            subcommands: self.child_help_commands(command),
            notes,
            defaults,
        }
    }
}

/// Type-correct argv generated from a registered shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntheticInvocation {
    pub command_path: Vec<String>,
    pub combination_id: String,
    pub argv: Vec<String>,
}

/// A built registry with exactly one handler per action.
pub struct BoundCliSpec<R> {
    cli: BuiltCliSpec,
    handlers: BTreeMap<String, fn(&ResolvedInvocation) -> R>,
}

impl<R> BoundCliSpec<R> {
    /// Resolve argv against this registry, binding the run branch to its
    /// handler as it goes.
    ///
    /// The handler is attached here, where `bind_actions` has already proved
    /// one exists for every action, which is what makes
    /// [`BoundInvocation::run`] infallible. Resolving through the registry that
    /// owns the handlers also removes the possibility of dispatching an
    /// invocation from a *different* registry: there is no longer a step that
    /// takes one.
    pub fn resolve_from<I, S>(&self, args: I) -> Result<BoundOutcome<R>, CliError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Ok(match self.cli.resolve_from(args)? {
            CliOutcome::Run(invocation) => BoundOutcome::Run(self.bind(invocation)),
            CliOutcome::Help(help) => BoundOutcome::Help(help),
            CliOutcome::Version(version) => BoundOutcome::Version(version),
            CliOutcome::Docs(docs) => BoundOutcome::Docs(docs),
        })
    }

    /// Call every declared combination's handler with strict argument reads,
    /// returning each combination id with what its handler produced.
    ///
    /// **This runs your handlers.** Every one of them, on synthetic argv, in
    /// whatever order the registry declares them. Use it only where a handler
    /// is a pure projection of argv into a command value. Where the handler
    /// *is* the command, this will do whatever the command does — bind a port
    /// and never return, open a terminal, write files, or uninstall something.
    /// The name of this method is not a promise that it is safe to call; the
    /// shape of your handlers is.
    ///
    /// Where it does fit, it is the other half of
    /// [`ResolvedInvocation::required`] being infallible. A handler that reads
    /// an argument id its combination does not declare gets a failed read in
    /// production — silent, and exactly the class of typo no compiler catches.
    /// Here it panics naming the combination and the id, driven by the same
    /// synthetic invocations [`BuiltCliSpec::synthetic_invocations`] generates
    /// for help and docs. Production code carries no branch for a case it
    /// cannot reach, and the case is still caught before it ships.
    ///
    /// If your handlers are not callable here, that is worth reading as a
    /// design signal rather than a limitation of this method: separating "parse
    /// argv into a typed command" from "carry the command out" makes the check
    /// available and is the better shape independently.
    ///
    /// The results are returned rather than discarded because a handler that
    /// returns a `Result` has a second thing worth checking — that every
    /// combination actually *builds* — and a caller that had to write its own
    /// loop for that would be back to hand-rolling the half this replaces.
    /// Ignore the return when the handler's output says nothing useful.
    // This method exists to fail loudly from a test; diverging is the whole
    // point, and the doc above says so. The allow records that rather than
    // reshaping a deliberate abort into a value nobody can act on.
    #[allow(clippy::panic)]
    pub fn call_every_combination(&self) -> Vec<(String, R)> {
        let mut results = Vec::new();
        for fixture in self.cli.synthetic_invocations() {
            // A fixture that does not resolve to a run is a registry defect, and
            // skipping it would let this method report success while covering
            // nothing — the one failure a check like this must not have.
            let outcome = self.cli.resolve_from(fixture.argv.clone());
            let Ok(CliOutcome::Run(mut invocation)) = outcome else {
                panic!(
                    "combination `{}` generated argv {:?}, which does not resolve to a run",
                    fixture.combination_id, fixture.argv
                );
            };
            invocation.strict_reads = true;
            let combination = invocation.combination_id.clone();
            results.push((combination, self.bind(invocation).run()));
        }
        assert!(
            !results.is_empty(),
            "no combination was called; a registry with no runnable combination cannot be verified"
        );
        results
    }

    fn bind(&self, invocation: ResolvedInvocation) -> BoundInvocation<R> {
        // `bind_actions` rejected any registry whose action ids and handler ids
        // differ, and `action_id` can only come from a combination in that same
        // registry. The allow records that proof once, here, instead of making
        // every caller carry a branch for it.
        #[allow(clippy::expect_used)]
        let handler = *self
            .handlers
            .get(invocation.action_id())
            .expect("bind_actions guarantees one handler per action id");
        BoundInvocation {
            invocation,
            handler,
        }
    }
}

/// What an argv resolved to, with the run branch already bound to its handler.
pub enum BoundOutcome<R> {
    Run(BoundInvocation<R>),
    Help(ResolvedHelp),
    Version(ResolvedVersion),
    Docs(ResolvedDocs),
}

impl<R> std::fmt::Debug for BoundOutcome<R> {
    /// Written by hand so it does not require `R: Debug`: `R` is whatever the
    /// application's handlers return, and demanding `Debug` of it would make
    /// this enum unusable in a test that wants to assert on the outcome.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Run(invocation) => formatter.debug_tuple("Run").field(invocation).finish(),
            Self::Help(help) => formatter.debug_tuple("Help").field(help).finish(),
            Self::Version(version) => formatter.debug_tuple("Version").field(version).finish(),
            Self::Docs(docs) => formatter.debug_tuple("Docs").field(docs).finish(),
        }
    }
}

/// A resolved invocation together with the handler that will run it.
///
/// Running cannot fail: the handler was looked up at resolution, by the
/// registry that owns it.
pub struct BoundInvocation<R> {
    invocation: ResolvedInvocation,
    handler: fn(&ResolvedInvocation) -> R,
}

impl<R> BoundInvocation<R> {
    /// The output contract, readable before the handler runs so a caller can
    /// install redirection first.
    pub fn output_plan(&self) -> &OutputPlan {
        self.invocation.output_plan()
    }

    /// The resolved invocation, for a caller that needs more than the plan.
    pub fn invocation(&self) -> &ResolvedInvocation {
        &self.invocation
    }

    /// Run the bound handler.
    pub fn run(self) -> R {
        (self.handler)(&self.invocation)
    }

    /// Run the bound handler and give the invocation back.
    ///
    /// For a caller that still needs the resolved values afterwards — a command
    /// path, or globals read once the handler has produced its command. Without
    /// this the only way to keep them past [`run`](Self::run) is to clone the
    /// value maps before calling it.
    pub fn run_with_invocation(self) -> (R, ResolvedInvocation) {
        let result = (self.handler)(&self.invocation);
        (result, self.invocation)
    }
}

impl<R> std::fmt::Debug for BoundInvocation<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BoundInvocation")
            .field("invocation", &self.invocation)
            .finish_non_exhaustive()
    }
}

/// What an argv resolved to.
#[derive(Clone, Debug, PartialEq)]
pub enum CliOutcome {
    Run(ResolvedInvocation),
    Help(ResolvedHelp),
    Version(ResolvedVersion),
    Docs(ResolvedDocs),
}

/// One legal invocation, projected onto the shape that matched it.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedInvocation {
    pub(super) command_path: Vec<String>,
    pub(super) action_id: String,
    pub(super) combination_id: String,
    pub(super) values: BTreeMap<String, CliValue>,
    pub(super) explicit_argument_ids: BTreeSet<String>,
    pub(super) output: OutputPlan,
    /// Set only by [`BoundCliSpec::call_every_combination`]. Reading an argument id
    /// the combination does not declare is a defect in the handler, and this is
    /// the mode that says so instead of handing back a default.
    pub(super) strict_reads: bool,
}

/// Stands in for an argument the selected combination does not declare.
///
/// Reads of it simply fail their type check, so a handler bug degrades to a
/// failed read rather than a plausible value.
const MISSING: CliValue = CliValue::Bool(false);

impl ResolvedInvocation {
    pub fn command_path(&self) -> &[String] {
        &self.command_path
    }

    pub fn action_id(&self) -> &str {
        &self.action_id
    }

    pub fn combination_id(&self) -> &str {
        &self.combination_id
    }

    pub fn output_plan(&self) -> &OutputPlan {
        &self.output
    }

    /// Whether the caller wrote this argument, as opposed to inheriting it
    /// from the shape's fixed value or the argument's default.
    pub fn was_explicit(&self, argument_id: &str) -> bool {
        self.explicit_argument_ids.contains(argument_id)
    }

    pub fn optional(&self, argument_id: &str) -> Option<&CliValue> {
        self.values.get(argument_id)
    }

    /// Read a value the selected combination declares as required or fixed.
    ///
    /// Infallible by construction: resolution has already proved the selected
    /// combination supplies every id it declares. Asking for an id it does not
    /// declare is a defect in the handler, not a runtime condition, so this
    /// does not make every caller branch on a case a correct program cannot
    /// reach — [`BoundCliSpec::call_every_combination`] is where that defect
    /// surfaces, from a test, naming the combination and the id.
    pub fn required(&self, argument_id: &str) -> &CliValue {
        match self.values.get(argument_id) {
            Some(value) => value,
            None => {
                assert!(
                    !self.strict_reads,
                    "combination `{}` does not declare argument id `{argument_id}`",
                    self.combination_id
                );
                // Fails every typed accessor rather than reading as a valid
                // flag, so a miss that escapes verification still cannot be
                // mistaken for a real value.
                &MISSING
            }
        }
    }

    pub fn repeated(&self, argument_id: &str) -> &[CliValue] {
        self.values
            .get(argument_id)
            .and_then(CliValue::as_list)
            .unwrap_or(&[])
    }
}

/// Where a resolved call's output goes, and in what form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputPlan {
    Raw {
        stdout_file: Option<PathBuf>,
        stderr_file: Option<PathBuf>,
    },
    Protocol {
        lifecycle: OutputLifecycle,
        format: OutputFormat,
        destination: OutputTo,
        stdout_file: Option<PathBuf>,
        stderr_file: Option<PathBuf>,
    },
}

impl OutputPlan {
    /// Structured output format, or `None` for a raw-byte command.
    pub const fn output_format(&self) -> Option<OutputFormat> {
        match self {
            Self::Raw { .. } => None,
            Self::Protocol { format, .. } => Some(*format),
        }
    }

    /// Structured output routing, or `None` for a raw-byte command.
    pub const fn output_to(&self) -> Option<OutputTo> {
        match self {
            Self::Raw { .. } => None,
            Self::Protocol { destination, .. } => Some(*destination),
        }
    }

    /// Canonical format spelling retained for string-oriented callers.
    pub fn format(&self) -> Option<&str> {
        self.output_format().map(OutputFormat::as_str)
    }

    /// Canonical destination spelling retained for string-oriented callers.
    pub fn destination(&self) -> Option<&str> {
        self.output_to().map(OutputTo::as_str)
    }

    pub fn stdout_file(&self) -> Option<&Path> {
        match self {
            Self::Raw { stdout_file, .. } | Self::Protocol { stdout_file, .. } => {
                stdout_file.as_deref()
            }
        }
    }

    pub fn stderr_file(&self) -> Option<&Path> {
        match self {
            Self::Raw { stderr_file, .. } | Self::Protocol { stderr_file, .. } => {
                stderr_file.as_deref()
            }
        }
    }
}

#[derive(Default)]
struct ParsedArgs {
    application_values: BTreeMap<String, Vec<CliValue>>,
    explicit_application_ids: BTreeSet<String>,
    output: ParsedOutput,
    help: bool,
    version: bool,
    docs: bool,
}

impl ParsedArgs {
    fn control_count(&self) -> usize {
        usize::from(self.help) + usize::from(self.version) + usize::from(self.docs)
    }
}

#[derive(Default)]
struct ParsedOutput {
    format: Option<String>,
    destination: Option<String>,
    stdout_file: Option<PathBuf>,
    stderr_file: Option<PathBuf>,
}

fn tokenize(
    command: &CommandSpec,
    tokens: &[String],
    command_path: &str,
    all_commands: &[CommandSpec],
) -> Result<ParsedArgs, CliError> {
    let longs: BTreeMap<&str, &ArgSpec> = command
        .arguments
        .iter()
        .filter_map(|argument| match &argument.syntax {
            ArgSyntax::Long { name } => Some((name.as_str(), argument)),
            ArgSyntax::Positional { .. } => None,
        })
        .collect();
    let mut positionals: Vec<&ArgSpec> = command
        .arguments
        .iter()
        .filter(|argument| matches!(argument.syntax, ArgSyntax::Positional { .. }))
        .collect();
    positionals.sort_by_key(|argument| match argument.syntax {
        ArgSyntax::Positional { index } => index,
        ArgSyntax::Long { .. } => usize::MAX,
    });

    let mut parsed = ParsedArgs::default();
    let mut index = 0;
    let mut positional_index = 0;
    let mut options_done = false;
    while index < tokens.len() {
        let token = &tokens[index];
        if !options_done && token == "--" {
            options_done = true;
            index += 1;
            continue;
        }
        if !options_done && token.starts_with("--") {
            let (name, inline_value) = token
                .split_once('=')
                .map_or((token.as_str(), None), |(name, value)| (name, Some(value)));
            if let Some(argument) = longs.get(name).copied() {
                let display = name.to_string();
                let raw_value = if argument.value_type == ArgValueType::Flag {
                    if inline_value.is_some() {
                        return Err(invalid_value(
                            command_path,
                            display,
                            "flags do not accept values",
                        ));
                    }
                    None
                } else {
                    Some(take_value(
                        tokens,
                        &mut index,
                        inline_value,
                        Some(argument),
                        name,
                        command_path,
                    )?)
                };
                let value = match raw_value {
                    Some(value) => parse_value(argument, value).map_err(|message| {
                        invalid_value(command_path, display.clone(), &message)
                    })?,
                    None => CliValue::Bool(true),
                };
                insert_application(&mut parsed, argument, value, display, command_path)?;
                index += 1;
                continue;
            }
            match name {
                "--help" => {
                    reject_inline_value(inline_value, name, command_path)?;
                    set_once(&mut parsed.help, name, command_path)?;
                }
                "--version" => {
                    reject_inline_value(inline_value, name, command_path)?;
                    set_once(&mut parsed.version, name, command_path)?;
                }
                "--docs" => {
                    reject_inline_value(inline_value, name, command_path)?;
                    set_once(&mut parsed.docs, name, command_path)?;
                }
                "--output" => {
                    parsed.output.format = Some(set_output_value(
                        parsed.output.format.as_ref(),
                        take_value(tokens, &mut index, inline_value, None, name, command_path)?,
                        name,
                        command_path,
                    )?);
                }
                "--output-to" => {
                    parsed.output.destination = Some(set_output_value(
                        parsed.output.destination.as_ref(),
                        take_value(tokens, &mut index, inline_value, None, name, command_path)?,
                        name,
                        command_path,
                    )?);
                }
                "--stdout-file" => {
                    parsed.output.stdout_file = Some(PathBuf::from(set_output_value(
                        parsed.output.stdout_file.as_ref(),
                        take_value(tokens, &mut index, inline_value, None, name, command_path)?,
                        name,
                        command_path,
                    )?));
                }
                "--stderr-file" => {
                    parsed.output.stderr_file = Some(PathBuf::from(set_output_value(
                        parsed.output.stderr_file.as_ref(),
                        take_value(tokens, &mut index, inline_value, None, name, command_path)?,
                        name,
                        command_path,
                    )?));
                }
                _ => {
                    return Err(CliError::new(
                        CliErrorRule::UnknownArgument,
                        command_path.to_string(),
                        format!("unknown argument `{name}`"),
                    ));
                }
            }
            index += 1;
            continue;
        }
        if !options_done && token.starts_with('-') && token != "-" {
            let positional_accepts_value = positionals
                .get(positional_index)
                .is_some_and(|argument| accepts_hyphen_prefixed_value(argument, token));
            if !positional_accepts_value {
                return Err(CliError::new(
                    CliErrorRule::UnknownArgument,
                    command_path.to_string(),
                    "unknown short argument",
                ));
            }
        }
        let Some(argument) = positionals.get(positional_index).copied() else {
            // A registered command name here is almost always a caller who put
            // the command after its arguments. Saying only "unexpected
            // positional argument" is true and useless: the fix is an ordering
            // one, and nothing else in the message would suggest it.
            let message = if is_registered_command_segment(token, all_commands) {
                "command name must come before its arguments"
            } else {
                "unexpected positional argument"
            };
            return Err(CliError::new(
                CliErrorRule::UnexpectedPositional,
                command_path.to_string(),
                message,
            ));
        };
        let value = parse_value(argument, token).map_err(|message| {
            invalid_value(command_path, argument.argument_id.clone(), &message)
        })?;
        insert_application(
            &mut parsed,
            argument,
            value,
            argument.argument_id.clone(),
            command_path,
        )?;
        if !argument.repeatable {
            positional_index += 1;
        }
        index += 1;
    }
    Ok(parsed)
}

fn take_value<'a>(
    tokens: &'a [String],
    index: &mut usize,
    inline_value: Option<&'a str>,
    argument: Option<&ArgSpec>,
    name: &str,
    command_path: &str,
) -> Result<&'a str, CliError> {
    if let Some(value) = inline_value {
        if value.is_empty() {
            return Err(missing_value(command_path, name));
        }
        return Ok(value);
    }
    let Some(value) = tokens.get(*index + 1) else {
        return Err(missing_value(command_path, name));
    };
    if value.starts_with('-')
        && value != "-"
        && !argument.is_some_and(|argument| accepts_hyphen_prefixed_value(argument, value))
    {
        return Err(missing_value(command_path, name));
    }
    *index += 1;
    Ok(value)
}

/// Whether a `-`-prefixed token is this argument's value rather than a flag.
///
/// Deliberately a *syntax* question, not a validity one. Asking `parse_value`
/// would fold semantic constraints in, so `--limit -1` against a `1..=1000`
/// range would be read as "no value supplied" — reporting a missing argument
/// for a value the caller plainly typed. Range and enum membership are checked
/// after the token is claimed, where they can say what is actually wrong.
fn accepts_hyphen_prefixed_value(argument: &ArgSpec, value: &str) -> bool {
    match argument.value_type {
        ArgValueType::I64 => value.parse::<i64>().is_ok(),
        ArgValueType::FiniteF64 => value.parse::<f64>().is_ok_and(f64::is_finite),
        ArgValueType::Json => serde_json::from_str::<serde::de::IgnoredAny>(value).is_ok(),
        _ => false,
    }
}

fn reject_inline_value(
    inline_value: Option<&str>,
    name: &str,
    command_path: &str,
) -> Result<(), CliError> {
    if inline_value.is_some() {
        return Err(invalid_value(
            command_path,
            name.to_string(),
            "control flags do not accept values",
        ));
    }
    Ok(())
}

fn set_once(value: &mut bool, name: &str, command_path: &str) -> Result<(), CliError> {
    if *value {
        return Err(duplicate_error(command_path, name));
    }
    *value = true;
    Ok(())
}

fn set_output_value<T>(
    existing: Option<&T>,
    value: &str,
    name: &str,
    command_path: &str,
) -> Result<String, CliError> {
    if existing.is_some() {
        return Err(duplicate_error(command_path, name));
    }
    Ok(value.to_string())
}

fn insert_application(
    parsed: &mut ParsedArgs,
    argument: &ArgSpec,
    value: CliValue,
    display: String,
    command_path: &str,
) -> Result<(), CliError> {
    let values = parsed
        .application_values
        .entry(argument.argument_id.clone())
        .or_default();
    if !argument.repeatable && !values.is_empty() {
        return Err(duplicate_error(command_path, &display));
    }
    values.push(value);
    parsed
        .explicit_application_ids
        .insert(argument.argument_id.clone());
    Ok(())
}

fn parse_value(argument: &ArgSpec, raw: &str) -> Result<CliValue, String> {
    match argument.value_type {
        ArgValueType::Flag => Ok(CliValue::Bool(true)),
        ArgValueType::String | ArgValueType::Enum => {
            if argument.value_type == ArgValueType::Enum
                && !argument.enum_values.iter().any(|value| value == raw)
            {
                return Err(format!(
                    "expected one of {}",
                    argument.enum_values.join(", ")
                ));
            }
            Ok(CliValue::String(raw.to_string()))
        }
        ArgValueType::I64 => {
            let value = raw
                .parse::<i64>()
                .map_err(|_| "expected an i64 integer".to_string())?;
            if let Some([minimum, maximum]) = argument.range
                && !(minimum..=maximum).contains(&value)
            {
                return Err(format!("expected an integer in {minimum}..={maximum}"));
            }
            Ok(CliValue::I64(value))
        }
        ArgValueType::Uuid => {
            if is_canonical_uuid(raw) {
                Ok(CliValue::String(raw.to_string()))
            } else {
                Err("expected a UUID (8-4-4-4-12 hexadecimal digits)".to_string())
            }
        }
        ArgValueType::FiniteF64 => raw
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(CliValue::FiniteF64)
            .ok_or_else(|| "expected a finite f64 number".to_string()),
        // Validate that `raw` is exactly one JSON value, then keep the source
        // text. `IgnoredAny` checks the grammar without building a
        // `serde_json::Value`, so the core never commits to a number
        // representation on the caller's behalf.
        ArgValueType::Json => serde_json::from_str::<serde::de::IgnoredAny>(raw)
            .map(|_| CliValue::Json(raw.to_string()))
            .map_err(|_| "expected one valid JSON value".to_string()),
    }
}

pub(super) fn validate_value_type(argument: &ArgSpec, value: &CliValue) -> Result<(), String> {
    match (&argument.value_type, value) {
        (ArgValueType::Flag, CliValue::Bool(_))
        | (ArgValueType::String, CliValue::String(_))
        | (ArgValueType::Json, CliValue::Json(_)) => Ok(()),
        // A default or fixed value is held to the same range as one typed on
        // the command line. Checking only the type would let a registry ship a
        // default its own argument rejects — a contradiction nothing downstream
        // could report, because the value never passes through the parser.
        (ArgValueType::I64, CliValue::I64(number)) => match argument.range {
            Some([minimum, maximum]) if !(minimum..=maximum).contains(number) => Err(format!(
                "value {number} is outside the argument's {minimum}..={maximum} range"
            )),
            _ => Ok(()),
        },
        (ArgValueType::Uuid, CliValue::String(text)) if is_canonical_uuid(text) => Ok(()),
        (ArgValueType::Uuid, CliValue::String(_)) => {
            Err("value is not a UUID (8-4-4-4-12 hexadecimal digits)".to_string())
        }
        (ArgValueType::FiniteF64, CliValue::FiniteF64(value)) if value.is_finite() => Ok(()),
        (ArgValueType::Enum, CliValue::String(value)) if argument.enum_values.contains(value) => {
            Ok(())
        }
        _ => Err("value type does not match the argument".to_string()),
    }
}

fn combination_matches(
    command: &CommandSpec,
    combination: &Combination,
    parsed: &ParsedArgs,
) -> bool {
    let allowed: BTreeSet<&str> = combination
        .fixed
        .keys()
        .map(String::as_str)
        .chain(combination.required.iter().map(String::as_str))
        .chain(combination.optional.iter().map(String::as_str))
        .collect();
    if parsed
        .explicit_application_ids
        .iter()
        .any(|id| !allowed.contains(id.as_str()))
        || combination
            .required
            .iter()
            .any(|id| !parsed.explicit_application_ids.contains(id))
    {
        return false;
    }
    combination.fixed.iter().all(|(id, fixed)| {
        let argument = command
            .arguments
            .iter()
            .find(|argument| argument.argument_id == *id);
        let effective = parsed
            .application_values
            .get(id)
            .and_then(|values| values.first())
            .or_else(|| argument.and_then(|argument| argument.default.as_ref()));
        effective
            .and_then(CliValue::as_str)
            .is_some_and(|value| fixed.values().iter().any(|fixed| fixed == value))
    })
}

fn project_values(
    command: &CommandSpec,
    combination: &Combination,
    parsed: &ParsedArgs,
) -> BTreeMap<String, CliValue> {
    let allowed: BTreeSet<&str> = combination
        .fixed
        .keys()
        .map(String::as_str)
        .chain(combination.required.iter().map(String::as_str))
        .chain(combination.optional.iter().map(String::as_str))
        .collect();
    command
        .arguments
        .iter()
        .filter(|argument| allowed.contains(argument.argument_id.as_str()))
        .filter_map(|argument| {
            let value = parsed
                .application_values
                .get(&argument.argument_id)
                .map(|values| {
                    if argument.repeatable {
                        CliValue::List(values.clone())
                    } else {
                        values[0].clone()
                    }
                })
                .or_else(|| argument.default.clone())
                .or_else(|| {
                    (argument.value_type == ArgValueType::Flag).then_some(CliValue::Bool(false))
                });
            value.map(|value| (argument.argument_id.clone(), value))
        })
        .collect()
}

fn resolve_output(
    spec: &OutputSpec,
    parsed: &ParsedOutput,
    command_path: &str,
) -> Result<OutputPlan, CliError> {
    match spec {
        OutputSpec::Raw { file_sinks } => {
            if parsed.format.is_some() || parsed.destination.is_some() {
                return Err(CliError::unregistered(command_path.to_string()));
            }
            ensure_sinks(file_sinks, parsed, command_path)?;
            Ok(OutputPlan::Raw {
                stdout_file: parsed.stdout_file.clone(),
                stderr_file: parsed.stderr_file.clone(),
            })
        }
        OutputSpec::Protocol {
            lifecycle,
            formats,
            destinations,
            default_format,
            default_destination,
            file_sinks,
        } => {
            ensure_sinks(file_sinks, parsed, command_path)?;
            let format = parsed.format.as_ref().unwrap_or(default_format);
            if !formats.contains(format) {
                return Err(invalid_value(
                    command_path,
                    "--output".to_string(),
                    &format!("expected one of {}", formats.join(", ")),
                ));
            }
            let destination = parsed.destination.as_ref().unwrap_or(default_destination);
            if !destinations.contains(destination) {
                return Err(invalid_value(
                    command_path,
                    "--output-to".to_string(),
                    &format!("expected one of {}", destinations.join(", ")),
                ));
            }
            let output_format = format.parse::<OutputFormat>().map_err(|_| {
                invalid_value(
                    command_path,
                    "--output".to_string(),
                    "expected one of json, yaml, plain",
                )
            })?;
            let output_to = destination.parse::<OutputTo>().map_err(|_| {
                invalid_value(
                    command_path,
                    "--output-to".to_string(),
                    "expected one of split, stdout, stderr",
                )
            })?;
            Ok(OutputPlan::Protocol {
                lifecycle: *lifecycle,
                format: output_format,
                destination: output_to,
                stdout_file: parsed.stdout_file.clone(),
                stderr_file: parsed.stderr_file.clone(),
            })
        }
    }
}

fn ensure_sinks(
    allowed: &[String],
    parsed: &ParsedOutput,
    command_path: &str,
) -> Result<(), CliError> {
    if (parsed.stdout_file.is_some() && !allowed.iter().any(|sink| sink == "stdout"))
        || (parsed.stderr_file.is_some() && !allowed.iter().any(|sink| sink == "stderr"))
    {
        return Err(CliError::unregistered(command_path.to_string()));
    }
    Ok(())
}

/// Whether `token` names a segment of some registered command path.
///
/// Used only to sharpen a diagnosis, so a plain containment test is enough: the
/// point is to recognise that the caller typed a command where a value was
/// expected, not to work out which command they meant.
fn is_registered_command_segment(token: &str, all_commands: &[CommandSpec]) -> bool {
    all_commands
        .iter()
        .any(|command| command.command_path.iter().any(|segment| segment == token))
}

/// Whether `raw` is a canonical 8-4-4-4-12 hexadecimal UUID.
///
/// Hand-written rather than pulled from a crate: the check has to be something
/// another language can reimplement from `cli-spec-v1` alone, and it keeps the
/// core free of a dependency for one argument shape.
fn is_canonical_uuid(raw: &str) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut parts = raw.split('-');
    for width in GROUPS {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.len() != width || !part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }
    }
    parts.next().is_none()
}

fn synthetic_value(argument: &ArgSpec) -> String {
    match argument.value_type {
        ArgValueType::Flag => String::new(),
        ArgValueType::String => {
            if argument.sensitive {
                "synthetic-sensitive".to_string()
            } else {
                "value".to_string()
            }
        }
        ArgValueType::I64 => argument
            .range
            .map_or_else(|| "1".to_string(), |[minimum, _]| minimum.to_string()),
        ArgValueType::Uuid => "00000000-0000-0000-0000-000000000000".to_string(),
        ArgValueType::FiniteF64 => "1.5".to_string(),
        ArgValueType::Enum => argument.enum_values.first().cloned().unwrap_or_default(),
        ArgValueType::Json => "{}".to_string(),
    }
}

fn append_synthetic_argument(argv: &mut Vec<String>, argument: &ArgSpec, value: &str) {
    match &argument.syntax {
        ArgSyntax::Long { name } => {
            argv.push(name.clone());
            if argument.value_type != ArgValueType::Flag {
                argv.push(value.to_string());
            }
        }
        ArgSyntax::Positional { .. } => argv.push(value.to_string()),
    }
}
