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

    /// The help model for one command, or for one of its combinations.
    ///
    /// The same model `--help` and `--help-combination` return, reachable
    /// in-process. Build-time tooling — an offline reference renderer, for
    /// example — needs the whole registry, and this is how it consumes it
    /// without a full-spec dump on the agent's discovery path.
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
                    Vec::new(),
                    "argv contains a token that is not valid UTF-8",
                ));
            };
            utf8.push(token.to_string());
        }
        let argv = if utf8.is_empty() { &[][..] } else { &utf8[1..] };
        let (command, consumed) = self.select_command(argv)?;
        let command_path = self.display_command_path(command);
        let parsed = tokenize(command, &argv[consumed..], &command_path)?;

        if parsed.control_count() > 0 {
            if parsed.control_count() != 1 || !parsed.application_values.is_empty() {
                return Err(CliError::unregistered(
                    command_path,
                    parsed.explicit_names(),
                ));
            }
            // `--docs` renders the whole registry, which is raw bytes, not
            // protocol events. It therefore gets its own contract instead of
            // `lifecycle_output`, and must be settled before the shared
            // protocol plan below would wrongly accept `--output`.
            if parsed.docs {
                if !command.command_path.is_empty() {
                    return Err(CliError::unregistered(
                        command_path,
                        vec!["--docs".to_string()],
                    ));
                }
                let contract = OutputSpec::raw()
                    .file_sinks(self.spec.lifecycle_output.file_sinks_ref().to_vec());
                let output = resolve_output(
                    &contract,
                    &parsed.output,
                    &command_path,
                    parsed.explicit_names(),
                )?;
                return Ok(CliOutcome::Docs(ResolvedDocs { output }));
            }
            let output = resolve_output(
                &self.spec.lifecycle_output,
                &parsed.output,
                &command_path,
                parsed.explicit_names(),
            )?;
            if parsed.help {
                return Ok(CliOutcome::Help(ResolvedHelp {
                    model: self.help_model(command),
                    output,
                }));
            }
            if parsed.version {
                if !command.command_path.is_empty() {
                    return Err(CliError::unregistered(
                        command_path,
                        vec!["--version".to_string()],
                    ));
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
            return Err(CliError::unregistered(
                command_path,
                parsed.explicit_names(),
            ));
        };
        if matching.len() != 1 {
            return Err(CliError::new(
                CliErrorRule::UnregisteredCombination,
                command_path,
                parsed.explicit_names(),
                "arguments match more than one registered CLI combination",
            ));
        }
        let output = resolve_output(
            &combination.output,
            &parsed.output,
            &command_path,
            parsed.explicit_names(),
        )?;
        let values = project_values(command, combination, &parsed);
        Ok(CliOutcome::Run(ResolvedInvocation {
            command_path: command.command_path.clone(),
            action_id: combination.action_id.clone(),
            combination_id: combination.combination_id.clone(),
            values,
            explicit_argument_ids: parsed.explicit_application_ids,
            output,
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
                    Vec::new(),
                    format!("unknown command `{}`", remaining[0]),
                ));
            }
            return Ok((command, command.command_path.len()));
        }
        Err(CliError::new(
            CliErrorRule::UnknownCommand,
            self.spec.name.clone(),
            Vec::new(),
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
    pub fn resolve_from<I, S>(&self, args: I) -> Result<CliOutcome, CliError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.cli.resolve_from(args)
    }

    /// Run the handler bound to this invocation's action.
    ///
    /// `bind_actions` proved at startup that every action id has exactly one
    /// handler, so this lookup cannot miss. The allow records that proof
    /// instead of inventing a fallback value of a caller-chosen type `R`.
    #[allow(clippy::expect_used)]
    pub fn execute(&self, invocation: &ResolvedInvocation) -> R {
        let handler = self
            .handlers
            .get(invocation.action_id())
            .expect("bind_actions guarantees one handler per action id");
        handler(invocation)
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
}

/// Stands in for an argument the resolver guarantees is present.
///
/// Asking for an id the selected shape cannot produce is a programming error,
/// not a user error, so it must not become a `cli_error` — and it must not
/// panic either. Reads of this value simply fail their type check.
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

    pub fn required(&self, argument_id: &str) -> &CliValue {
        self.values.get(argument_id).unwrap_or(&MISSING)
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
        format: String,
        destination: String,
        stdout_file: Option<PathBuf>,
        stderr_file: Option<PathBuf>,
    },
}

impl OutputPlan {
    pub fn format(&self) -> Option<&str> {
        match self {
            Self::Raw { .. } => None,
            Self::Protocol { format, .. } => Some(format),
        }
    }

    pub fn destination(&self) -> Option<&str> {
        match self {
            Self::Raw { .. } => None,
            Self::Protocol { destination, .. } => Some(destination),
        }
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
    explicit_application_names: BTreeMap<String, String>,
    output: ParsedOutput,
    help: bool,
    version: bool,
    docs: bool,
}

impl ParsedArgs {
    fn control_count(&self) -> usize {
        usize::from(self.help) + usize::from(self.version) + usize::from(self.docs)
    }

    fn explicit_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.explicit_application_names.values().cloned().collect();
        names.extend(self.output.explicit_names());
        if self.help {
            names.push("--help".to_string());
        }
        if self.version {
            names.push("--version".to_string());
        }
        if self.docs {
            names.push("--docs".to_string());
        }
        names.sort();
        names.dedup();
        names
    }
}

#[derive(Default)]
struct ParsedOutput {
    format: Option<String>,
    destination: Option<String>,
    stdout_file: Option<PathBuf>,
    stderr_file: Option<PathBuf>,
}

impl ParsedOutput {
    fn explicit_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        if self.format.is_some() {
            names.push("--output".to_string());
        }
        if self.destination.is_some() {
            names.push("--output-to".to_string());
        }
        if self.stdout_file.is_some() {
            names.push("--stdout-file".to_string());
        }
        if self.stderr_file.is_some() {
            names.push("--stderr-file".to_string());
        }
        names
    }
}
fn tokenize(
    command: &CommandSpec,
    tokens: &[String],
    command_path: &str,
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
                        take_value(tokens, &mut index, inline_value, name, command_path)?,
                        name,
                        command_path,
                    )?);
                }
                "--output-to" => {
                    parsed.output.destination = Some(set_output_value(
                        parsed.output.destination.as_ref(),
                        take_value(tokens, &mut index, inline_value, name, command_path)?,
                        name,
                        command_path,
                    )?);
                }
                "--stdout-file" => {
                    parsed.output.stdout_file = Some(PathBuf::from(set_output_value(
                        parsed.output.stdout_file.as_ref(),
                        take_value(tokens, &mut index, inline_value, name, command_path)?,
                        name,
                        command_path,
                    )?));
                }
                "--stderr-file" => {
                    parsed.output.stderr_file = Some(PathBuf::from(set_output_value(
                        parsed.output.stderr_file.as_ref(),
                        take_value(tokens, &mut index, inline_value, name, command_path)?,
                        name,
                        command_path,
                    )?));
                }
                _ => {
                    return Err(CliError::new(
                        CliErrorRule::UnknownArgument,
                        command_path.to_string(),
                        vec![name.to_string()],
                        format!("unknown argument `{name}`"),
                    ));
                }
            }
            index += 1;
            continue;
        }
        if !options_done && token.starts_with('-') && token != "-" {
            return Err(CliError::new(
                CliErrorRule::UnknownArgument,
                command_path.to_string(),
                vec![token.to_string()],
                format!("unknown argument `{token}`"),
            ));
        }
        let Some(argument) = positionals.get(positional_index).copied() else {
            return Err(CliError::new(
                CliErrorRule::UnexpectedPositional,
                command_path.to_string(),
                Vec::new(),
                "unexpected positional argument",
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
    if value.starts_with('-') && value != "-" {
        return Err(missing_value(command_path, name));
    }
    *index += 1;
    Ok(value)
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
    parsed
        .explicit_application_names
        .insert(argument.argument_id.clone(), display);
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
        ArgValueType::I64 => raw
            .parse::<i64>()
            .map(CliValue::I64)
            .map_err(|_| "expected an i64 integer".to_string()),
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
        | (ArgValueType::I64, CliValue::I64(_))
        | (ArgValueType::Json, CliValue::Json(_)) => Ok(()),
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
    argument_names: Vec<String>,
) -> Result<OutputPlan, CliError> {
    match spec {
        OutputSpec::Raw { file_sinks } => {
            if parsed.format.is_some() || parsed.destination.is_some() {
                return Err(CliError::unregistered(
                    command_path.to_string(),
                    argument_names,
                ));
            }
            ensure_sinks(file_sinks, parsed, command_path, argument_names)?;
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
            ensure_sinks(file_sinks, parsed, command_path, argument_names.clone())?;
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
            Ok(OutputPlan::Protocol {
                lifecycle: *lifecycle,
                format: format.clone(),
                destination: destination.clone(),
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
    argument_names: Vec<String>,
) -> Result<(), CliError> {
    if (parsed.stdout_file.is_some() && !allowed.iter().any(|sink| sink == "stdout"))
        || (parsed.stderr_file.is_some() && !allowed.iter().any(|sink| sink == "stderr"))
    {
        return Err(CliError::unregistered(
            command_path.to_string(),
            argument_names,
        ));
    }
    Ok(())
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
        ArgValueType::I64 => "1".to_string(),
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
