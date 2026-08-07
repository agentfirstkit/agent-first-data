use super::build::validate_spec;
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Serializable version-one closed-world CLI registry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CliSpec {
    pub schema: String,
    pub name: String,
    pub version: String,
    /// Human-facing product name, distinct from the binary identity in `name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Opaque build identifier (a git SHA, for example). The core only carries
    /// it; what it means is the host's business.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Arguments every command accepts, declared once.
    ///
    /// Spliced into each command at build time and added to every combination's
    /// optional set, so nothing downstream — resolution, help, `--docs` — needs
    /// to know they were shared. Serialized as their own list so a consumer in
    /// another language reads the same declaration rather than N copies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_arguments: Vec<ArgSpec>,
    pub lifecycle_output: OutputSpec,
    /// Exit codes this CLI returns beyond the 0/1/2 AFDATA defines, rendered
    /// into the reference's exit-code table. Without this the published
    /// reference documents only AFDATA's three, so a tool that also returns,
    /// say, a partial-success code ships a document that contradicts its own
    /// binary — on exactly the code a caller needs to branch on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exit_codes: Vec<ExitCodeSpec>,
    pub commands: Vec<CommandSpec>,
}

/// One exit code a CLI defines for itself, beyond AFDATA's 0/1/2.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExitCodeSpec {
    pub code: u8,
    /// What the code means to a caller, as one table cell.
    pub meaning: String,
}

impl CliSpec {
    /// Start a `cli-spec-v1` registry.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            schema: "cli-spec-v1".to_string(),
            name: name.into(),
            version: version.into(),
            display_name: None,
            build: None,
            about: None,
            shared_arguments: Vec::new(),
            lifecycle_output: OutputSpec::protocol_finite(
                ["json", "yaml", "plain"],
                ["split", "stdout", "stderr"],
                "json",
                "split",
            ),
            exit_codes: Vec::new(),
            commands: Vec::new(),
        }
    }

    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.about = nonempty(about.into());
        self
    }

    pub fn display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = nonempty(display_name.into());
        self
    }

    /// Record an opaque build identifier. Named `build_id` because `build()`
    /// already compiles the registry.
    pub fn build_id(mut self, build: impl Into<String>) -> Self {
        self.build = nonempty(build.into());
        self
    }

    pub fn lifecycle_output(mut self, output: OutputSpec) -> Self {
        self.lifecycle_output = output;
        self
    }

    /// Declare an exit code this CLI returns beyond AFDATA's 0/1/2, so the
    /// rendered reference documents what the binary actually does.
    pub fn exit_code(mut self, code: u8, meaning: impl Into<String>) -> Self {
        self.exit_codes.push(ExitCodeSpec {
            code,
            meaning: meaning.into(),
        });
        self
    }

    pub fn command(mut self, command: CommandSpec) -> Self {
        self.commands.push(command);
        self
    }

    /// Declare an argument every command accepts, once.
    ///
    /// AFDATA already parses `--output`, `--stdout-file` and friends at every
    /// command; this is the same capability for an argument the application
    /// owns, so a `--config` needed by six commands is written once instead of
    /// six times and cannot drift between them.
    ///
    /// It does **not** change where the argument may appear. The command path
    /// is still matched against the leading tokens of argv, so `tool --config x
    /// sub` remains an error and `tool sub --config x` is the accepted form.
    /// This is only about declaration, not position — a caller migrating from a
    /// parser with interleaved global flags has to move them after the path.
    ///
    /// A command with no combinations is skipped: it exists to carry help for
    /// its children, accepts nothing itself, and would otherwise fail the
    /// build for declaring an argument no combination covers.
    #[must_use]
    pub fn shared_arg(mut self, argument: ArgSpec) -> Self {
        self.shared_arguments.push(argument);
        self
    }

    /// Validate and compile the registry.
    pub fn build(mut self) -> Result<BuiltCliSpec, CliSpecError> {
        self.splice_shared_arguments()?;
        validate_spec(&self)?;
        Ok(BuiltCliSpec { spec: self })
    }

    /// Copy the shared arguments into every command, and into every
    /// combination's optional set.
    ///
    /// Done before validation so a shared argument is held to exactly the same
    /// rules as a declared one — reserved names, canonical spelling, and the
    /// id/flag agreement all report against it normally.
    fn splice_shared_arguments(&mut self) -> Result<(), CliSpecError> {
        if self.shared_arguments.is_empty() {
            return Ok(());
        }
        for shared in &self.shared_arguments {
            for command in &self.commands {
                if command
                    .arguments
                    .iter()
                    .any(|argument| argument.argument_id == shared.argument_id)
                {
                    return Err(CliSpecError::new(
                        "shared_argument_redeclared",
                        format!(
                            "argument `{}` is shared by every command, so `{}` must not declare \
                             it again",
                            shared.argument_id,
                            if command.command_path.is_empty() {
                                "the root command".to_string()
                            } else {
                                command.command_path.join(" ")
                            }
                        ),
                    ));
                }
            }
        }
        let shared = self.shared_arguments.clone();
        for command in &mut self.commands {
            // A combination-less command is a help-only path node (`tool
            // analysis --help` listing its children). It accepts no arguments
            // of its own, so splicing one in would leave an argument no
            // combination covers and fail validation.
            if command.combinations.is_empty() {
                continue;
            }
            for argument in &shared {
                command.arguments.push(argument.clone());
                for combination in &mut command.combinations {
                    // Shared arguments are never required and never fixed: a
                    // combination that needs one must declare its own.
                    if !combination.optional.contains(&argument.argument_id) {
                        combination.optional.push(argument.argument_id.clone());
                    }
                }
            }
        }
        Ok(())
    }
}

/// One exact command path in a CLI registry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub command_path: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    pub arguments: Vec<ArgSpec>,
    pub combinations: Vec<Combination>,
}

impl CommandSpec {
    pub fn root() -> Self {
        Self::new(std::iter::empty::<String>())
    }

    pub fn new<I, S>(command_path: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            command_path: command_path.into_iter().map(Into::into).collect(),
            about: None,
            arguments: Vec::new(),
            combinations: Vec::new(),
        }
    }

    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.about = nonempty(about.into());
        self
    }

    pub fn arg(mut self, argument: ArgSpec) -> Self {
        self.arguments.push(argument);
        self
    }

    pub fn combination(mut self, combination: Combination) -> Self {
        self.combinations.push(combination);
        self
    }
}

/// An argument's exact command-local spelling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArgSyntax {
    Long { name: String },
    Positional { index: usize },
}

/// Closed portable value type used by CLI specs and resolved invocations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgValueType {
    Flag,
    String,
    I64,
    FiniteF64,
    Enum,
    Json,
    /// A canonical RFC 4122 UUID, kept as its string form.
    ///
    /// Declarative on purpose: the registry serializes to `cli-spec-v1`, so a
    /// value type has to be something another language can implement from the
    /// spec alone. A host-supplied parser could not survive that trip; this
    /// can. The value stays a `String` so the core takes no UUID dependency —
    /// what it buys is that a malformed one is a *usage* error, rejected before
    /// the command runs, instead of a domain failure the handler has to invent.
    Uuid,
}

/// A typed value produced by a built CLI registry.
///
/// `Json` deliberately holds the argument's raw source text rather than a
/// parsed `serde_json::Value`. AFDATA turns on `serde_json/arbitrary_precision`
/// and that feature unifies across a whole binary, so a parsed value's number
/// semantics would depend on which other crates happen to be linked in. The
/// text is validated as one JSON value at parse time; deciding what its numbers
/// mean is the caller's choice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CliValue {
    Bool(bool),
    String(String),
    I64(i64),
    FiniteF64(f64),
    Json(String),
    List(Vec<CliValue>),
}

impl CliValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I64(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::FiniteF64(value) => Some(*value),
            _ => None,
        }
    }

    /// The raw, still-unparsed JSON source text of a `json` argument.
    pub fn as_json_str(&self) -> Option<&str> {
        match self {
            Self::Json(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[CliValue]> {
        match self {
            Self::List(values) => Some(values),
            _ => None,
        }
    }
}

/// One command-local application argument.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArgSpec {
    pub argument_id: String,
    pub syntax: ArgSyntax,
    pub value_type: ArgValueType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<String>,
    /// Inclusive bounds for an `I64` argument.
    ///
    /// Lets a registry say `1..=1000` without a host-supplied parser, so a
    /// count that must fit an `i32`, a `usize`, or a `NonZero` is rejected at
    /// exit 2 with the other usage errors rather than checked again inside the
    /// handler — where the only honest report left is a domain failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[i64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<CliValue>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub repeatable: bool,
    /// This argument's value must never be echoed back.
    ///
    /// The core only consumes the bit: it suppresses help defaults, keeps the
    /// value out of rendered templates, and rejects a serializable default.
    /// Which arguments deserve it is a host convention — `crate::cli_afdata`
    /// derives it from AFDATA's `_secret` suffix.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sensitive: bool,
    /// The sources this argument accepts beside a literal value.
    ///
    /// Declared rather than assumed: a source turns an argument into a reader
    /// of files and environment variables, which is right for a credential and
    /// wrong for most everything else. The core validates that a value names
    /// only a scheme in this set, and renders the syntax into help so no host
    /// repeats it in an `about` string. Reading happens in the host, when it
    /// chooses — see [`crate::value_source`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<SourceSet>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
}

impl ArgSpec {
    pub fn flag(long: impl Into<String>) -> Self {
        Self::long(long, ArgValueType::Flag, None::<String>)
    }

    pub fn option(long: impl Into<String>, value_name: impl Into<String>) -> Self {
        Self::long(long, ArgValueType::String, Some(value_name.into()))
    }

    pub fn option_i64(long: impl Into<String>, value_name: impl Into<String>) -> Self {
        Self::long(long, ArgValueType::I64, Some(value_name.into()))
    }

    pub fn option_f64(long: impl Into<String>, value_name: impl Into<String>) -> Self {
        Self::long(long, ArgValueType::FiniteF64, Some(value_name.into()))
    }

    pub fn option_json(long: impl Into<String>, value_name: impl Into<String>) -> Self {
        Self::long(long, ArgValueType::Json, Some(value_name.into()))
    }

    pub fn option_enum<I, S>(long: impl Into<String>, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut spec = Self::long(long, ArgValueType::Enum, Some("VALUE".to_string()));
        spec.enum_values = values.into_iter().map(Into::into).collect();
        spec
    }

    /// Read this argument as a canonical RFC 4122 UUID.
    ///
    /// The resolved value is still a `CliValue::String`; what changes is that a
    /// malformed one is rejected as a usage error before the command runs.
    #[must_use]
    pub fn uuid(mut self) -> Self {
        self.value_type = ArgValueType::Uuid;
        self
    }

    /// Constrain an `I64` argument to an inclusive range.
    ///
    /// Use it for a count that must fit a narrower integer than `i64` — the
    /// check then reports as a usage error, beside the other argument
    /// failures, rather than as a domain failure from inside the handler.
    #[must_use]
    pub fn range(mut self, minimum: i64, maximum: i64) -> Self {
        self.value_type = ArgValueType::I64;
        self.range = Some([minimum, maximum]);
        self
    }

    pub fn positional(
        argument_id: impl Into<String>,
        index: usize,
        value_name: impl Into<String>,
    ) -> Self {
        Self {
            argument_id: argument_id.into(),
            syntax: ArgSyntax::Positional { index },
            value_type: ArgValueType::String,
            value_name: nonempty(value_name.into()),
            enum_values: Vec::new(),
            range: None,
            default: None,
            repeatable: false,
            sensitive: false,
            sources: None,
            about: None,
        }
    }

    pub fn positional_json(
        argument_id: impl Into<String>,
        index: usize,
        value_name: impl Into<String>,
    ) -> Self {
        Self {
            value_type: ArgValueType::Json,
            ..Self::positional(argument_id, index, value_name)
        }
    }

    pub fn positional_enum<I, S>(
        argument_id: impl Into<String>,
        index: usize,
        value_name: impl Into<String>,
        values: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut spec = Self {
            value_type: ArgValueType::Enum,
            ..Self::positional(argument_id, index, value_name)
        };
        spec.enum_values = values.into_iter().map(Into::into).collect();
        spec
    }

    fn long(long: impl Into<String>, value_type: ArgValueType, value_name: Option<String>) -> Self {
        let long = long.into();
        let argument_id = long
            .strip_prefix("--")
            .unwrap_or(long.as_str())
            .replace('-', "_");
        Self {
            argument_id,
            syntax: ArgSyntax::Long { name: long },
            value_type,
            value_name: value_name.and_then(nonempty),
            enum_values: Vec::new(),
            range: None,
            default: None,
            repeatable: false,
            sensitive: false,
            sources: None,
            about: None,
        }
    }

    pub fn value_name(mut self, value_name: impl Into<String>) -> Self {
        self.value_name = nonempty(value_name.into());
        self
    }

    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.default = Some(CliValue::String(value.into()));
        self
    }

    pub fn default_i64(mut self, value: i64) -> Self {
        self.default = Some(CliValue::I64(value));
        self
    }

    pub fn default_f64(mut self, value: f64) -> Self {
        self.default = Some(CliValue::FiniteF64(value));
        self
    }

    pub fn repeatable(mut self) -> Self {
        self.repeatable = true;
        self
    }

    /// Mark this argument's value as one that must never be echoed back.
    pub fn sensitive(mut self) -> Self {
        self.sensitive = true;
        self
    }

    /// Accept the value indirectly, from any source in `sources`.
    ///
    /// The help text follows from the set, so `about` should say what the value
    /// *is* and leave the syntax to this. Reading is the host's, at the moment
    /// it chooses: [`crate::cli_spec::SourceSet::parse`] on the resolved
    /// string, then `read` or `read_secret`.
    #[must_use]
    pub fn sources(mut self, sources: SourceSet) -> Self {
        self.sources = Some(sources);
        self
    }

    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.about = nonempty(about.into());
        self
    }

    /// What a reader is told about this argument: what the value means, plus
    /// how it may be sourced.
    ///
    /// Hosts declare those separately — `about` says what the value *is*, the
    /// source set says where it may come from — and every rendering path joins
    /// them here. That is the whole reason the set is declared rather than
    /// written into prose: one afhttp flag's syntax was repeated across nine
    /// rows of its generated reference, and one afpsql flag's across forty-eight,
    /// each a place to forget when a source is added.
    #[must_use]
    pub fn rendered_about(&self) -> Option<String> {
        match (&self.about, &self.sources) {
            (Some(about), Some(sources)) => Some(format!("{about} ({})", sources.syntax_summary())),
            (Some(about), None) => Some(about.clone()),
            (None, Some(sources)) => Some(sources.syntax_summary()),
            (None, None) => None,
        }
    }
}

/// A finite fixed enum constraint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FixedValue {
    Value(String),
    OneOf { one_of: Vec<String> },
}

impl FixedValue {
    pub(super) fn values(&self) -> &[String] {
        match self {
            Self::Value(value) => std::slice::from_ref(value),
            Self::OneOf { one_of } => one_of,
        }
    }
}

/// One named legal application invocation shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Combination {
    pub combination_id: String,
    pub action_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fixed: BTreeMap<String, FixedValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional: Vec<String>,
    pub output: OutputSpec,
}

impl Combination {
    pub fn new(combination_id: impl Into<String>) -> Self {
        Self {
            combination_id: combination_id.into(),
            action_id: String::new(),
            about: None,
            fixed: BTreeMap::new(),
            required: Vec::new(),
            optional: Vec::new(),
            output: OutputSpec::protocol_finite(
                ["json", "yaml", "plain"],
                ["split", "stdout", "stderr"],
                "json",
                "split",
            ),
        }
    }

    pub fn action(mut self, action_id: impl Into<String>) -> Self {
        self.action_id = action_id.into();
        self
    }

    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.about = nonempty(about.into());
        self
    }

    pub fn fixed(mut self, argument_id: impl Into<String>, value: impl Into<String>) -> Self {
        self.fixed
            .insert(argument_id.into(), FixedValue::Value(value.into()));
        self
    }

    pub fn fixed_one_of<I, S>(mut self, argument_id: impl Into<String>, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.fixed.insert(
            argument_id.into(),
            FixedValue::OneOf {
                one_of: values.into_iter().map(Into::into).collect(),
            },
        );
        self
    }

    pub fn required<I, S>(mut self, argument_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.required
            .extend(argument_ids.into_iter().map(Into::into));
        self
    }

    pub fn optional<I, S>(mut self, argument_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.optional
            .extend(argument_ids.into_iter().map(Into::into));
        self
    }

    pub fn output(mut self, output: OutputSpec) -> Self {
        self.output = output;
        self
    }
}

/// Protocol output lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputLifecycle {
    Finite,
    Stream,
}

/// Closed output contract for one combination.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputSpec {
    Raw {
        #[serde(default)]
        file_sinks: Vec<String>,
    },
    Protocol {
        lifecycle: OutputLifecycle,
        formats: Vec<String>,
        destinations: Vec<String>,
        default_format: String,
        default_destination: String,
        #[serde(default)]
        file_sinks: Vec<String>,
    },
}

impl OutputSpec {
    pub fn raw() -> Self {
        Self::Raw {
            file_sinks: Vec::new(),
        }
    }

    pub fn protocol_finite<FI, FS, DI, DS>(
        formats: FI,
        destinations: DI,
        default_format: impl Into<String>,
        default_destination: impl Into<String>,
    ) -> Self
    where
        FI: IntoIterator<Item = FS>,
        FS: Into<String>,
        DI: IntoIterator<Item = DS>,
        DS: Into<String>,
    {
        Self::Protocol {
            lifecycle: OutputLifecycle::Finite,
            formats: formats.into_iter().map(Into::into).collect(),
            destinations: destinations.into_iter().map(Into::into).collect(),
            default_format: default_format.into(),
            default_destination: default_destination.into(),
            file_sinks: Vec::new(),
        }
    }

    pub fn protocol_stream<FI, FS, DI, DS>(
        formats: FI,
        destinations: DI,
        default_format: impl Into<String>,
        default_destination: impl Into<String>,
    ) -> Self
    where
        FI: IntoIterator<Item = FS>,
        FS: Into<String>,
        DI: IntoIterator<Item = DS>,
        DS: Into<String>,
    {
        Self::Protocol {
            lifecycle: OutputLifecycle::Stream,
            formats: formats.into_iter().map(Into::into).collect(),
            destinations: destinations.into_iter().map(Into::into).collect(),
            default_format: default_format.into(),
            default_destination: default_destination.into(),
            file_sinks: Vec::new(),
        }
    }

    pub fn file_sinks<I, S>(mut self, sinks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let values = sinks.into_iter().map(Into::into).collect();
        match &mut self {
            Self::Raw { file_sinks } | Self::Protocol { file_sinks, .. } => {
                *file_sinks = values;
            }
        }
        self
    }

    pub(super) fn file_sinks_ref(&self) -> &[String] {
        match self {
            Self::Raw { file_sinks } | Self::Protocol { file_sinks, .. } => file_sinks,
        }
    }
}

/// Stable build-time registry error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliSpecError {
    pub rule: &'static str,
    pub message: String,
}

impl CliSpecError {
    pub(super) fn new(rule: &'static str, message: impl Into<String>) -> Self {
        Self {
            rule,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CliSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.rule, self.message)
    }
}

impl std::error::Error for CliSpecError {}
