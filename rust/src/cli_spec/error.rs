use serde::Serialize;

/// Stable closed set of invocation-error rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliErrorRule {
    InvalidUtf8,
    UnknownCommand,
    UnknownArgument,
    MissingArgumentValue,
    InvalidArgumentValue,
    DuplicateArgument,
    UnexpectedPositional,
    UnregisteredCombination,
}

impl CliErrorRule {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "invalid_utf8",
            Self::UnknownCommand => "unknown_command",
            Self::UnknownArgument => "unknown_argument",
            Self::MissingArgumentValue => "missing_argument_value",
            Self::InvalidArgumentValue => "invalid_argument_value",
            Self::DuplicateArgument => "duplicate_argument",
            Self::UnexpectedPositional => "unexpected_positional",
            Self::UnregisteredCombination => "unregistered_combination",
        }
    }

    /// The `error.code` this failure is reported under.
    ///
    /// The classification belongs in the code, beside `document_type_mismatch`
    /// and the rest — not in a second field next to a generic `cli_error`,
    /// which asked an agent to learn a second place to look and cost a closed
    /// enum kept in step across four files and four mirrored schemas.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "cli_invalid_utf8",
            Self::UnknownCommand => "cli_unknown_command",
            Self::UnknownArgument => "cli_unknown_argument",
            Self::MissingArgumentValue => "cli_missing_argument_value",
            Self::InvalidArgumentValue => "cli_invalid_argument_value",
            Self::DuplicateArgument => "cli_duplicate_argument",
            Self::UnexpectedPositional => "cli_unexpected_positional",
            Self::UnregisteredCombination => "cli_unregistered_combination",
        }
    }
}

/// Structured CLI-resolution error. It never contains raw argument values.
///
/// `rule` is the classification and the only thing worth branching on; the two
/// strings are for a human. The command path and the implicated argument names
/// are not fields: both are already inside `hint` and `message` respectively,
/// and a second, stringly-typed copy of either is a place for them to drift.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError {
    pub rule: CliErrorRule,
    pub message: String,
    pub hint: String,
}

impl CliError {
    pub(super) fn new(
        rule: CliErrorRule,
        command_path: String,
        message: impl Into<String>,
    ) -> Self {
        // The remedy follows the rule: only a mixture of known arguments is
        // fixed by choosing among the registered combinations. A bad or
        // missing value, a repeat, or an unknown name each has its own next
        // step, and pointing them all at "combinations" sends a caller who
        // typed one wrong number off to compare argument sets.
        let hint = match rule {
            CliErrorRule::UnregisteredCombination => {
                format!("run `{command_path} --help` and choose one registered combination")
            }
            CliErrorRule::UnknownCommand => {
                format!("run `{command_path} --help` to list the registered commands")
            }
            CliErrorRule::UnknownArgument | CliErrorRule::UnexpectedPositional => {
                format!("run `{command_path} --help` to list the arguments each shape accepts")
            }
            CliErrorRule::InvalidArgumentValue => format!(
                "pass a value of the type and range `{command_path} --help` lists for that argument"
            ),
            CliErrorRule::MissingArgumentValue => format!(
                "pass a value right after the argument; `{command_path} --help` shows its value name"
            ),
            CliErrorRule::DuplicateArgument => format!(
                "pass the argument once; `{command_path} --help` shows which arguments repeat"
            ),
            CliErrorRule::InvalidUtf8 => {
                format!("pass every argument as valid UTF-8; `{command_path} --help` lists them")
            }
        };
        Self {
            rule,
            message: message.into(),
            hint,
        }
    }

    pub(super) fn unregistered(command_path: String) -> Self {
        Self::new(
            CliErrorRule::UnregisteredCombination,
            command_path,
            "arguments do not match a registered CLI combination",
        )
    }

    /// Fixed process exit status for every closed-world CLI error.
    ///
    /// Derived rather than stored: every rule in [`CliErrorRule`] exits 2, and
    /// a field would only be somewhere for that to drift.
    pub const fn exit_code(&self) -> u8 {
        2
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

pub(super) fn invalid_value(command_path: &str, name: String, message: &str) -> CliError {
    CliError::new(
        CliErrorRule::InvalidArgumentValue,
        command_path.to_string(),
        format!("invalid value for `{name}`: {message}"),
    )
}

pub(super) fn missing_value(command_path: &str, name: &str) -> CliError {
    CliError::new(
        CliErrorRule::MissingArgumentValue,
        command_path.to_string(),
        format!("argument `{name}` requires a value"),
    )
}

pub(super) fn duplicate_error(command_path: &str, name: &str) -> CliError {
    CliError::new(
        CliErrorRule::DuplicateArgument,
        command_path.to_string(),
        format!("argument `{name}` may not be repeated"),
    )
}
