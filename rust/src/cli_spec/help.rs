use super::*;
use serde::Serialize;
use std::collections::BTreeMap;

/// Agent-first help-v2 payload.
///
/// `defaults` carries [`CliValue`], which admits finite `f64`, so this model is
/// `PartialEq` but not `Eq`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CliHelpV2 {
    pub schema: String,
    pub command_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    /// Every legal shape of this command, each complete.
    ///
    /// Help is answered in one round trip rather than two. A second level would
    /// only pay off by omitting something, and what it omitted — the optional
    /// arguments — would be undiscoverable to a caller that stopped at the
    /// first level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<CliShape>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subcommands: Vec<String>,
    /// Argument meanings. Arguments belong to the command, not to one shape,
    /// so these are stated once rather than per shape.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub notes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, CliValue>,
}

/// One legal shape of a call.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CliShape {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    pub usage: String,
}

/// Resolved help response plus its lifecycle output plan.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedHelp {
    pub(super) model: CliHelpV2,
    pub(super) output: OutputPlan,
}

impl ResolvedHelp {
    pub fn model(&self) -> &CliHelpV2 {
        &self.model
    }

    pub fn output_plan(&self) -> &OutputPlan {
        &self.output
    }

    pub fn plain(&self) -> String {
        let mut output = String::new();
        if let Some(about) = &self.model.about {
            output.push_str(about);
            output.push_str("\n\n");
        }
        for shape in &self.model.shapes {
            if self.model.shapes.len() > 1 {
                let about = shape.about.as_deref().unwrap_or_default();
                output.push_str(&format!("{}  {about}\n", shape.id));
            }
            output.push_str(&shape.usage);
            output.push('\n');
        }
        // The same facts the structured form carries. Without these the plain
        // rendering was strictly weaker than the JSON it claims to be a human
        // catalog of — no argument meanings, no defaults — so a reader who
        // asked for the readable form got told less.
        if !self.model.notes.is_empty() {
            output.push('\n');
            let width = self
                .model
                .notes
                .keys()
                .map(String::len)
                .max()
                .unwrap_or_default();
            for (name, note) in &self.model.notes {
                output.push_str(&format!("  {name:<width$}  {note}\n"));
            }
        }
        if !self.model.defaults.is_empty() {
            output.push_str("\ndefaults:");
            for (name, value) in &self.model.defaults {
                output.push_str(&format!(" {name}={}", plain_value(value)));
            }
            output.push('\n');
        }
        for (index, subcommand) in self.model.subcommands.iter().enumerate() {
            output.push_str(if index == 0 {
                "\nmore:    "
            } else {
                "         "
            });
            output.push_str(subcommand);
            output.push('\n');
        }
        output
    }
}

/// Resolved version response plus its lifecycle output plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedVersion {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) display_name: Option<String>,
    pub(super) build: Option<String>,
    pub(super) output: OutputPlan,
}

impl ResolvedVersion {
    pub fn output_plan(&self) -> &OutputPlan {
        &self.output
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    pub fn build(&self) -> Option<&str> {
        self.build.as_deref()
    }
}

/// The registry asked to describe itself in full.
///
/// Rendering is not the core's business — it carries no Markdown — so this only
/// says the request was legal and where the bytes go. A host renders the
/// registry it already holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDocs {
    pub(super) output: OutputPlan,
}

impl ResolvedDocs {
    pub fn output_plan(&self) -> &OutputPlan {
        &self.output
    }
}

pub(super) fn combination_usage(
    command: &CommandSpec,
    combination: &Combination,
    command_path: &str,
    detail: bool,
) -> (String, BTreeMap<String, String>, BTreeMap<String, CliValue>) {
    let mut parts = vec![command_path.to_string()];
    let mut notes = BTreeMap::new();
    let mut defaults = BTreeMap::new();
    for argument in &command.arguments {
        let id = argument.argument_id.as_str();
        let fixed = combination.fixed.get(id);
        let required = combination.required.iter().any(|candidate| candidate == id);
        let optional = combination.optional.iter().any(|candidate| candidate == id);
        if fixed.is_none() && !required && !optional {
            continue;
        }
        if optional && !detail {
            continue;
        }
        // A fixed argument the caller may leave out: `matches_combination`
        // accepts the argument's own default in place of an explicit value, so
        // rendering it as required would describe a stricter call than the one
        // the parser demands.
        let default_satisfies_fixed = fixed.is_some_and(|fixed| {
            argument
                .default
                .as_ref()
                .and_then(CliValue::as_str)
                .is_some_and(|value| fixed.values().iter().any(|fixed| fixed == value))
        });
        let omittable = optional || default_satisfies_fixed;
        let token = render_argument(argument, fixed, omittable);
        parts.push(token);
        if detail {
            let key = argument_key(argument);
            if let Some(about) = argument.rendered_about() {
                notes.insert(key.clone(), about);
            }
            // A default is only worth stating where leaving the argument out is
            // legal; on a required argument it describes nothing the caller can
            // reach.
            if omittable
                && !argument.sensitive
                && let Some(default) = &argument.default
            {
                defaults.insert(key, default.clone());
            }
        }
    }
    if detail {
        match &combination.output {
            OutputSpec::Raw { file_sinks } => {
                render_file_sinks(&mut parts, file_sinks);
            }
            OutputSpec::Protocol {
                formats,
                destinations,
                default_format,
                default_destination,
                file_sinks,
                ..
            } => {
                parts.push(format!("[--output <{}>]", formats.join("|")));
                parts.push(format!("[--output-to <{}>]", destinations.join("|")));
                render_file_sinks(&mut parts, file_sinks);
                defaults.insert(
                    "--output".to_string(),
                    CliValue::String(default_format.clone()),
                );
                defaults.insert(
                    "--output-to".to_string(),
                    CliValue::String(default_destination.clone()),
                );
            }
        }
    }
    (parts.join(" "), notes, defaults)
}

/// A default rendered for the plain catalog, without JSON's quoting.
fn plain_value(value: &CliValue) -> String {
    match value {
        CliValue::Bool(value) => value.to_string(),
        CliValue::String(value) | CliValue::Json(value) => value.clone(),
        CliValue::I64(value) => value.to_string(),
        CliValue::FiniteF64(value) => value.to_string(),
        CliValue::List(values) => values.iter().map(plain_value).collect::<Vec<_>>().join(","),
    }
}

/// The spelling the caller reads in `usage`, and therefore the key `notes` and
/// `defaults` use — an agent looks up a token it just read without having to
/// transform it first.
///
/// It also keeps AFDATA's own convention from misfiring on this crate's help:
/// an argument id like `reveal_secret` names a description, not a secret, and
/// the `_secret` suffix would redact this command's own help text away.
pub(crate) fn argument_key(argument: &ArgSpec) -> String {
    match &argument.syntax {
        ArgSyntax::Long { name } => name.clone(),
        ArgSyntax::Positional { .. } => argument
            .value_name
            .clone()
            .unwrap_or_else(|| argument.argument_id.clone()),
    }
}

fn render_argument(argument: &ArgSpec, fixed: Option<&FixedValue>, optional: bool) -> String {
    let value = if let Some(fixed) = fixed {
        match fixed {
            FixedValue::Value(value) => value.clone(),
            FixedValue::OneOf { one_of } => format!("<{}>", one_of.join("|")),
        }
    } else if argument.value_type == ArgValueType::Flag {
        String::new()
    } else if !argument.enum_values.is_empty() {
        // A closed value set is the argument's contract. Printing only the
        // placeholder would leave those values registered but undiscoverable —
        // the caller would have to provoke an error to be told what they are.
        format!("<{}>", argument.enum_values.join("|"))
    } else {
        format!(
            "<{}>",
            argument
                .value_name
                .as_deref()
                .unwrap_or(argument.argument_id.as_str())
        )
    };
    let mut token = match &argument.syntax {
        ArgSyntax::Long { name } => {
            if value.is_empty() {
                name.clone()
            } else {
                format!("{name} {value}")
            }
        }
        ArgSyntax::Positional { .. } => value,
    };
    if argument.repeatable {
        token.push_str("...");
    }
    if optional {
        token = format!("[{token}]");
    }
    token
}

fn render_file_sinks(parts: &mut Vec<String>, file_sinks: &[String]) {
    if file_sinks.iter().any(|sink| sink == "stdout") {
        parts.push("[--stdout-file <PATH>]".to_string());
    }
    if file_sinks.iter().any(|sink| sink == "stderr") {
        parts.push("[--stderr-file <PATH>]".to_string());
    }
}
