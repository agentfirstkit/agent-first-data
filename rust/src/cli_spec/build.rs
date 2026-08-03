use super::resolve::validate_value_type;
use super::*;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_spec(spec: &CliSpec) -> Result<(), CliSpecError> {
    if spec.schema != "cli-spec-v1" {
        return Err(CliSpecError::new(
            "invalid_schema",
            "schema must be `cli-spec-v1`",
        ));
    }
    if spec.name.trim().is_empty() || spec.version.trim().is_empty() {
        return Err(CliSpecError::new(
            "empty_identity",
            "CLI name and version must be non-empty",
        ));
    }
    validate_output(&spec.lifecycle_output)?;
    let mut declared_exit_codes = BTreeSet::new();
    for exit in &spec.exit_codes {
        // 0/1/2 are AFDATA's own and already in the rendered table. Letting a
        // CLI redeclare one would either duplicate the row or quietly restate a
        // meaning the protocol fixes.
        if exit.code <= 2 {
            return Err(CliSpecError::new(
                "reserved_exit_code",
                format!(
                    "exit code {} is defined by AFDATA and cannot be redeclared",
                    exit.code
                ),
            ));
        }
        if !declared_exit_codes.insert(exit.code) {
            return Err(CliSpecError::new(
                "duplicate_exit_code",
                format!("exit code {} is declared more than once", exit.code),
            ));
        }
        if exit.meaning.trim().is_empty() {
            return Err(CliSpecError::new(
                "empty_exit_code_meaning",
                format!("exit code {} must say what it means", exit.code),
            ));
        }
    }
    // Checked here rather than inside the per-command loop: there it could only
    // run while iterating a non-empty list, so it never fired.
    if spec.commands.is_empty() {
        return Err(CliSpecError::new(
            "missing_commands",
            "CLI registry must contain commands",
        ));
    }
    let mut command_paths = BTreeSet::new();
    let mut combination_ids = BTreeSet::new();
    let mut has_root = false;
    for command in &spec.commands {
        if command.command_path.is_empty() {
            has_root = true;
        }
        if !command_paths.insert(command.command_path.clone()) {
            return Err(CliSpecError::new(
                "duplicate_command_path",
                format!("duplicate command path {:?}", command.command_path),
            ));
        }
        if command
            .command_path
            .iter()
            .any(|part| part.is_empty() || part.starts_with('-'))
        {
            return Err(CliSpecError::new(
                "invalid_command_path",
                format!("invalid command path {:?}", command.command_path),
            ));
        }
        validate_command(command, &mut combination_ids)?;
    }
    if !has_root {
        return Err(CliSpecError::new(
            "missing_root_command",
            "the root command_path=[] must be registered",
        ));
    }
    for command in &spec.commands {
        if command
            .arguments
            .iter()
            .any(|argument| matches!(argument.syntax, ArgSyntax::Positional { .. }))
            && spec.commands.iter().any(|candidate| {
                candidate.command_path.len() > command.command_path.len()
                    && candidate.command_path.starts_with(&command.command_path)
            })
        {
            return Err(CliSpecError::new(
                "positional_with_subcommands",
                format!(
                    "command {:?} has both positionals and child commands",
                    command.command_path
                ),
            ));
        }
    }
    Ok(())
}

fn validate_command(
    command: &CommandSpec,
    combination_ids: &mut BTreeSet<String>,
) -> Result<(), CliSpecError> {
    let mut ids = BTreeSet::new();
    let mut longs = BTreeSet::new();
    let mut positional_indices = BTreeSet::new();
    for argument in &command.arguments {
        if argument.argument_id.is_empty() || !is_snake_case(&argument.argument_id) {
            return Err(CliSpecError::new(
                "invalid_argument_id",
                format!("invalid argument id `{}`", argument.argument_id),
            ));
        }
        if !ids.insert(argument.argument_id.clone()) {
            return Err(CliSpecError::new(
                "duplicate_argument_id",
                format!("duplicate argument id `{}`", argument.argument_id),
            ));
        }
        match &argument.syntax {
            ArgSyntax::Long { name } => {
                if is_reserved_long(&command.command_path, name) {
                    return Err(CliSpecError::new(
                        "reserved_long_argument",
                        format!(
                            "long `{name}` is reserved by AFDATA at {}",
                            if RESERVED_ARGUMENTS.contains(&name.as_str()) {
                                "every command"
                            } else {
                                "the root command; a subcommand may declare it"
                            }
                        ),
                    ));
                }
                if !is_canonical_long(name)
                    || name.trim_start_matches("--").replace('-', "_") != argument.argument_id
                {
                    return Err(CliSpecError::new(
                        "invalid_long_argument",
                        format!(
                            "long `{name}` must canonically map to `{}`",
                            argument.argument_id
                        ),
                    ));
                }
                if !longs.insert(name.clone()) {
                    return Err(CliSpecError::new(
                        "duplicate_long_argument",
                        format!("duplicate long argument `{name}`"),
                    ));
                }
            }
            ArgSyntax::Positional { index } => {
                if !positional_indices.insert(*index) {
                    return Err(CliSpecError::new(
                        "duplicate_positional_index",
                        format!("duplicate positional index {index}"),
                    ));
                }
            }
        }
        validate_argument(argument)?;
    }
    if positional_indices
        .iter()
        .copied()
        .ne(0..positional_indices.len())
    {
        return Err(CliSpecError::new(
            "non_contiguous_positionals",
            "positional indexes must start at zero and be contiguous",
        ));
    }
    let mut covered = BTreeSet::new();
    for combination in &command.combinations {
        if combination.action_id.trim().is_empty() {
            return Err(CliSpecError::new(
                "empty_action_id",
                format!(
                    "combination `{}` has an empty action id",
                    combination.combination_id
                ),
            ));
        }
        if combination.combination_id.trim().is_empty()
            || !combination_ids.insert(combination.combination_id.clone())
        {
            return Err(CliSpecError::new(
                "duplicate_combination_id",
                format!(
                    "combination id `{}` is empty or duplicated",
                    combination.combination_id
                ),
            ));
        }
        validate_combination(command, combination, &mut covered)?;
    }
    if covered != ids {
        let uncovered: Vec<&String> = ids.difference(&covered).collect();
        return Err(CliSpecError::new(
            "uncovered_argument",
            format!(
                "command {:?} has arguments absent from every combination: {uncovered:?}",
                command.command_path
            ),
        ));
    }
    for (index, left) in command.combinations.iter().enumerate() {
        for right in &command.combinations[index + 1..] {
            if combinations_overlap(command, left, right) {
                return Err(CliSpecError::new(
                    "overlapping_combinations",
                    format!(
                        "command {:?} combinations `{}` and `{}` overlap",
                        command.command_path, left.combination_id, right.combination_id
                    ),
                ));
            }
        }
    }
    // A command's own `about` describes its single combination perfectly well,
    // and repeating it verbatim is noise. But once a command owns more than one
    // shape, that inherited text cannot say how they differ — which is exactly
    // when an agent needs to be told. Requiring it only here keeps the writing
    // burden equal to the real ambiguity, and makes it non-optional where it
    // matters instead of optional everywhere and skipped.
    if command.combinations.len() > 1 {
        for combination in &command.combinations {
            if combination
                .about
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
            {
                return Err(CliSpecError::new(
                    "undescribed_combination",
                    format!(
                        "command {:?} has {} combinations, so `{}` must declare an `about` saying \
                         how it differs",
                        command.command_path,
                        command.combinations.len(),
                        combination.combination_id
                    ),
                ));
            }
        }
    }

    Ok(())
}

fn validate_argument(argument: &ArgSpec) -> Result<(), CliSpecError> {
    if argument.value_type == ArgValueType::Flag {
        if argument.default.is_some() {
            return Err(CliSpecError::new(
                "flag_default",
                format!("flag `{}` cannot declare a default", argument.argument_id),
            ));
        }
        if argument.repeatable {
            return Err(CliSpecError::new(
                "repeatable_flag",
                format!("flag `{}` cannot be repeatable", argument.argument_id),
            ));
        }
    }
    if argument.sensitive && argument.default.is_some() {
        return Err(CliSpecError::new(
            "sensitive_default",
            format!(
                "sensitive argument `{}` cannot declare a serializable default",
                argument.argument_id
            ),
        ));
    }
    if argument.value_type == ArgValueType::Json && argument.default.is_some() {
        return Err(CliSpecError::new(
            "json_default",
            format!(
                "json argument `{}` cannot declare a default; its value stays raw source text",
                argument.argument_id
            ),
        ));
    }
    if argument.value_type == ArgValueType::Enum {
        if argument.enum_values.is_empty() {
            return Err(CliSpecError::new(
                "empty_enum",
                format!("enum `{}` has no values", argument.argument_id),
            ));
        }
        let unique: BTreeSet<&String> = argument.enum_values.iter().collect();
        if unique.len() != argument.enum_values.len() {
            return Err(CliSpecError::new(
                "duplicate_enum_value",
                format!("enum `{}` has duplicate values", argument.argument_id),
            ));
        }
    } else if !argument.enum_values.is_empty() {
        return Err(CliSpecError::new(
            "enum_values_on_non_enum",
            format!(
                "non-enum `{}` cannot declare enum values",
                argument.argument_id
            ),
        ));
    }
    if let Some(default) = &argument.default {
        validate_value_type(argument, default).map_err(|message| {
            CliSpecError::new(
                "invalid_default",
                format!(
                    "default for `{}` is invalid: {message}",
                    argument.argument_id
                ),
            )
        })?;
    }
    Ok(())
}

fn validate_combination(
    command: &CommandSpec,
    combination: &Combination,
    covered: &mut BTreeSet<String>,
) -> Result<(), CliSpecError> {
    validate_output(&combination.output)?;
    let args: BTreeMap<&str, &ArgSpec> = command
        .arguments
        .iter()
        .map(|argument| (argument.argument_id.as_str(), argument))
        .collect();
    let fixed: BTreeSet<&str> = combination.fixed.keys().map(String::as_str).collect();
    let required: BTreeSet<&str> = combination.required.iter().map(String::as_str).collect();
    let optional: BTreeSet<&str> = combination.optional.iter().map(String::as_str).collect();
    if fixed.len() != combination.fixed.len()
        || required.len() != combination.required.len()
        || optional.len() != combination.optional.len()
        || !fixed.is_disjoint(&required)
        || !fixed.is_disjoint(&optional)
        || !required.is_disjoint(&optional)
    {
        return Err(CliSpecError::new(
            "combination_partition",
            format!(
                "combination `{}` fixed/required/optional must be unique and disjoint",
                combination.combination_id
            ),
        ));
    }
    for id in fixed.iter().chain(&required).chain(&optional) {
        let Some(argument) = args.get(id).copied() else {
            return Err(CliSpecError::new(
                "unknown_combination_argument",
                format!(
                    "combination `{}` references unknown argument `{id}`",
                    combination.combination_id
                ),
            ));
        };
        covered.insert((*id).to_string());
        if required.contains(id) && argument.default.is_some() {
            return Err(CliSpecError::new(
                "required_argument_default",
                format!("required argument `{id}` cannot have a default"),
            ));
        }
    }
    for (id, fixed_value) in &combination.fixed {
        let argument = args[id.as_str()];
        if argument.value_type != ArgValueType::Enum || argument.repeatable {
            return Err(CliSpecError::new(
                "invalid_fixed_argument",
                format!("fixed `{id}` must reference a non-repeatable enum"),
            ));
        }
        if fixed_value.values().is_empty()
            || fixed_value
                .values()
                .iter()
                .any(|value| !argument.enum_values.contains(value))
        {
            return Err(CliSpecError::new(
                "invalid_fixed_value",
                format!("fixed values for `{id}` must be a non-empty enum subset"),
            ));
        }
    }
    validate_combination_positionals(command, combination)
}

fn validate_combination_positionals(
    command: &CommandSpec,
    combination: &Combination,
) -> Result<(), CliSpecError> {
    let mut positionals: Vec<&ArgSpec> = command
        .arguments
        .iter()
        .filter(|argument| matches!(argument.syntax, ArgSyntax::Positional { .. }))
        .collect();
    positionals.sort_by_key(|argument| match argument.syntax {
        ArgSyntax::Positional { index } => index,
        ArgSyntax::Long { .. } => usize::MAX,
    });
    let mut saw_omitted = false;
    let mut saw_optional = false;
    for (index, argument) in positionals.iter().enumerate() {
        let id = argument.argument_id.as_str();
        let included = combination.fixed.contains_key(id)
            || combination.required.iter().any(|candidate| candidate == id)
            || combination.optional.iter().any(|candidate| candidate == id);
        if !included {
            saw_omitted = true;
            continue;
        }
        if saw_omitted {
            return Err(CliSpecError::new(
                "positional_not_prefix",
                format!(
                    "combination `{}` positionals must form a prefix",
                    combination.combination_id
                ),
            ));
        }
        let optional = combination.optional.iter().any(|candidate| candidate == id);
        if saw_optional && !optional {
            return Err(CliSpecError::new(
                "required_after_optional_positional",
                format!(
                    "combination `{}` has a required positional after an optional one",
                    combination.combination_id
                ),
            ));
        }
        saw_optional |= optional;
        if argument.repeatable && index + 1 != positionals.len() {
            return Err(CliSpecError::new(
                "repeatable_positional_not_last",
                format!(
                    "repeatable positional `{}` must be the final positional",
                    argument.argument_id
                ),
            ));
        }
    }
    Ok(())
}

fn validate_output(output: &OutputSpec) -> Result<(), CliSpecError> {
    let valid_sinks = ["stdout", "stderr"];
    if output
        .file_sinks_ref()
        .iter()
        .any(|sink| !valid_sinks.contains(&sink.as_str()))
    {
        return Err(CliSpecError::new(
            "invalid_file_sink",
            "file sinks must be `stdout` and/or `stderr`",
        ));
    }
    if let OutputSpec::Protocol {
        formats,
        destinations,
        default_format,
        default_destination,
        ..
    } = output
    {
        let valid_formats = ["json", "yaml", "plain"];
        let valid_destinations = ["split", "stdout", "stderr"];
        if formats.is_empty()
            || destinations.is_empty()
            || formats
                .iter()
                .any(|format| !valid_formats.contains(&format.as_str()))
            || destinations
                .iter()
                .any(|destination| !valid_destinations.contains(&destination.as_str()))
            || !formats.contains(default_format)
            || !destinations.contains(default_destination)
        {
            return Err(CliSpecError::new(
                "invalid_output_contract",
                "protocol output allowed/default sets are inconsistent",
            ));
        }
    }
    Ok(())
}

fn combinations_overlap(command: &CommandSpec, left: &Combination, right: &Combination) -> bool {
    command.arguments.iter().all(|argument| {
        let left_state = combination_state(argument, left);
        let right_state = combination_state(argument, right);
        (left_state.absent && right_state.absent)
            || (left_state.explicit
                && right_state.explicit
                && value_sets_intersect(left_state.values.as_ref(), right_state.values.as_ref()))
    })
}

struct CombinationState {
    absent: bool,
    explicit: bool,
    values: Option<BTreeSet<String>>,
}

fn combination_state(argument: &ArgSpec, combination: &Combination) -> CombinationState {
    if let Some(fixed) = combination.fixed.get(&argument.argument_id) {
        let values: BTreeSet<String> = fixed.values().iter().cloned().collect();
        let absent = argument
            .default
            .as_ref()
            .and_then(CliValue::as_str)
            .is_some_and(|default| values.contains(default));
        return CombinationState {
            absent,
            explicit: true,
            values: Some(values),
        };
    }
    if combination
        .required
        .iter()
        .any(|id| id == &argument.argument_id)
    {
        return CombinationState {
            absent: false,
            explicit: true,
            values: None,
        };
    }
    if combination
        .optional
        .iter()
        .any(|id| id == &argument.argument_id)
    {
        return CombinationState {
            absent: true,
            explicit: true,
            values: None,
        };
    }
    CombinationState {
        absent: true,
        explicit: false,
        values: None,
    }
}

fn value_sets_intersect(left: Option<&BTreeSet<String>>, right: Option<&BTreeSet<String>>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => !left.is_disjoint(right),
        _ => true,
    }
}
