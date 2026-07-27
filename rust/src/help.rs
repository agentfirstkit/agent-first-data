use crate::formatting::{render_yaml, serialize_json_output};
use crate::protocol::build_cli_error;
use crate::redaction::{
    OutputOptions, PlainStyle, RedactionContext, RedactionPolicy, Redactor, is_secret_flag_name,
};
use serde_json::Value;

// ═══════════════════════════════════════════
// Public API: CLI Help Rendering (optional)
// ═══════════════════════════════════════════

/// How much of a command tree a help request should render.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpScope {
    /// Render only the selected command's own clap-style help.
    ///
    /// Clap's normal help still lists direct subcommands in the "Commands"
    /// section, but descendant command detail is not expanded.
    OneLevel,
    /// Render the selected command and all visible descendant subcommands.
    Recursive,
}

/// Output format for help rendering.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpFormat {
    Plain,
    Markdown,
    Json,
    Yaml,
}

#[cfg(feature = "cli-help")]
impl HelpFormat {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "plain" => Some(Self::Plain),
            "markdown" => Some(Self::Markdown),
            "json" => Some(Self::Json),
            "yaml" => Some(Self::Yaml),
            _ => None,
        }
    }
}

/// Options for rendering CLI help.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HelpOptions {
    pub scope: HelpScope,
    pub format: HelpFormat,
}

#[cfg(feature = "cli-help")]
impl HelpOptions {
    /// Human-friendly current-level plain help.
    pub const fn one_level_plain() -> Self {
        Self {
            scope: HelpScope::OneLevel,
            format: HelpFormat::Plain,
        }
    }

    /// Agent/doc-friendly recursive plain help.
    pub const fn recursive_plain() -> Self {
        Self {
            scope: HelpScope::Recursive,
            format: HelpFormat::Plain,
        }
    }
}

/// Configuration for pre-clap help handling.
///
/// The handler scans raw argv before `Cli::try_parse()` so applications can
/// support requests such as `--help --output markdown` without clap exiting
/// early with `DisplayHelp`.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpConfig {
    /// Scope used for `--help` when `--recursive` is absent.
    pub default_scope: HelpScope,
    /// Fallback format when the command has no valid default for `--output`.
    ///
    /// An explicit `--output` always wins. When it is omitted, the handler
    /// first inherits the command's own `--output` default so help follows the
    /// same output contract as every other successful result.
    pub default_format: HelpFormat,
}

#[cfg(feature = "cli-help")]
impl HelpConfig {
    /// The blessed preset for CLIs.
    ///
    /// `--help` renders one-level help and inherits the command's own
    /// `--output` default. Scope and format are orthogonal: `--recursive`
    /// expands the selected command subtree, while `--output
    /// plain|json|yaml|markdown` picks the format. Plain is only the fallback
    /// for commands that do not declare a supported `--output` default.
    pub const fn output_aware() -> Self {
        Self::output_aware_with_fallback(HelpFormat::Plain)
    }

    /// Output-aware help with an explicit fallback format.
    ///
    /// Use this for a fixed-format CLI that has no `--output` argument. For
    /// example, a CLI whose normal output is always protocol JSON should pass
    /// [`HelpFormat::Json`]. An explicit help `--output`, or a supported
    /// default declared by the selected command or an ancestor, still wins.
    pub const fn output_aware_with_fallback(default_format: HelpFormat) -> Self {
        Self {
            default_scope: HelpScope::OneLevel,
            default_format,
        }
    }

    /// Compatibility alias for [`Self::output_aware`].
    ///
    /// The historical name predates output-default inheritance and is less
    /// precise: a JSON-default command receives JSON help, not human text.
    pub const fn human_cli_default() -> Self {
        Self::output_aware()
    }
}

/// Render help for a clap command tree with explicit scope and format.
///
/// Walks to the subcommand identified by `subcommand_path` (empty = root),
/// then renders either the selected command only (`OneLevel`) or the selected
/// command and all descendants (`Recursive`).
///
/// `Plain` and `Markdown` are text. `Json` and `Yaml` are protocol-v1 terminal
/// results (`result.code:"help"`, model under `result.help`) — the same shape
/// [`cli_handle_help_or_continue`] emits, so structured help validates against
/// `cli-help-v1.schema.json` no matter which entry point produced it.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
pub fn cli_render_help_with_options(
    cmd: &clap::Command,
    subcommand_path: &[&str],
    options: &HelpOptions,
) -> String {
    let selected = scoped_help_command(cmd, subcommand_path);
    let target = &selected.conventional_command;
    let mut rendered = match options.format {
        HelpFormat::Plain => match options.scope {
            HelpScope::OneLevel => render_help_one_level_plain(target),
            HelpScope::Recursive => {
                let mut buf = String::new();
                render_help_recursive_plain(target, &[], &mut buf);
                buf
            }
        },
        HelpFormat::Markdown => render_help_markdown(
            target,
            std::slice::from_ref(&selected.command_path),
            options.scope,
        ),
        HelpFormat::Json => {
            serialize_json_output(help_result_event(&selected, options.scope).as_value())
        }
        HelpFormat::Yaml => render_yaml(
            help_result_event(&selected, options.scope).as_value(),
            &OutputOptions {
                redaction: Redactor::new().policy(RedactionPolicy::Off),
                style: PlainStyle::Raw,
            },
        ),
    };
    // Every format ends with exactly one trailing newline so `print!`-ing the
    // result is clean across plain/markdown/json/yaml (JSON and raw YAML would
    // otherwise have none).
    while rendered.ends_with('\n') {
        rendered.pop();
    }
    rendered.push('\n');
    rendered
}

/// Render recursive plain-text help for a clap command tree.
///
/// Walks to the subcommand identified by `subcommand_path` (empty = root),
/// then recursively expands all descendant subcommands into a single output.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
pub fn cli_render_help(cmd: &clap::Command, subcommand_path: &[&str]) -> String {
    cli_render_help_with_options(cmd, subcommand_path, &HelpOptions::recursive_plain())
}

/// Render recursive Markdown help for a clap command tree.
///
/// Same tree walk as [`cli_render_help`], but outputs Markdown suitable for
/// documentation generation (`myapp --help --recursive --output markdown > docs/cli.md`).
///
/// Requires the `cli-help-markdown` feature.
#[cfg(feature = "cli-help-markdown")]
pub fn cli_render_help_markdown(cmd: &clap::Command, subcommand_path: &[&str]) -> String {
    cli_render_help_with_options(
        cmd,
        subcommand_path,
        &HelpOptions {
            scope: HelpScope::Recursive,
            format: HelpFormat::Markdown,
        },
    )
}

/// Render help from raw argv if a help flag is present; otherwise return `None`.
///
/// `raw_args` should be the full argv vector, including argv[0], as produced by
/// `std::env::args()`. The helper intentionally runs before clap parsing so
/// `--help --recursive` and `--help --output markdown` can select scope and
/// format instead of being consumed by clap's built-in help handling. Scope
/// (`--recursive`) and format (`--output`) are orthogonal.
///
/// A bare `--recursive` without `--help` is treated as a non-help request
/// (`Ok(None)`), leaving the flag for the application's own parser.
///
/// Returns a standard [`build_cli_error`] value when the help request is
/// malformed, for example `--help --output xml`.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
pub fn cli_handle_help_or_continue(
    raw_args: &[String],
    cmd: &clap::Command,
    config: &HelpConfig,
) -> Result<Option<String>, Value> {
    cli_handle_help_or_continue_inner(raw_args, cmd, config)
}

/// Handle a top-level version or help request through one early entry point.
///
/// This is the preferred CLI entry point when both helpers are enabled. The
/// caller supplies `name`, `display_name`, `version`, and `build` for the
/// version branch. Help is derived from `cmd` and intentionally does not repeat
/// version values. Version takes precedence when both flags are present,
/// matching the traditional two-call sequence.
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
#[allow(clippy::too_many_arguments)]
pub fn cli_handle_version_or_help_or_continue(
    raw_args: &[String],
    cmd: &clap::Command,
    config: &HelpConfig,
    name: &str,
    display_name: Option<&str>,
    version: &str,
    build: Option<&str>,
) -> Result<Option<String>, Value> {
    match crate::cli::cli_handle_version_or_continue(
        raw_args,
        cmd,
        name,
        display_name,
        version,
        build,
    ) {
        Ok(Some(rendered)) => return Ok(Some(rendered)),
        Ok(None) => {}
        Err(event) => return Err(event.into()),
    }
    // This entry point answers `--version` itself, so the command exposes the
    // flag whether or not the caller declared it on clap — an app has no reason
    // to, since the pre-parser intercepts the flag before clap ever sees it.
    // Materializing it on the clone is what makes help advertise `-V, --version`
    // exactly once; the model still carries only the flag, never its value. The
    // clone is used for rendering only, so the action is never dispatched.
    let cmd = version_aware_help_command(cmd);
    cli_handle_help_or_continue(raw_args, &cmd, config)
}

/// Clone `cmd` with a `--version` flag it can advertise in help.
///
/// Skipped when the command already exposes one (declared as an argument, or
/// auto-generated by clap from `Command::version`), and `-V` is claimed only
/// when no other argument owns that short.
#[cfg(feature = "cli-help")]
fn version_aware_help_command(cmd: &clap::Command) -> clap::Command {
    let already_exposed = cmd.get_arguments().any(|arg| {
        arg.get_long() == Some("version") || arg.get_short().is_some_and(|short| short == 'V')
    }) || (!cmd.is_disable_version_flag_set()
        && (cmd.get_version().is_some() || cmd.get_long_version().is_some()));
    if already_exposed {
        return cmd.clone();
    }
    let version_flag = clap::Arg::new("version")
        .long("version")
        .help("Print version")
        .action(clap::ArgAction::SetTrue);
    cmd.clone().disable_version_flag(true).arg(version_flag)
}

#[cfg(feature = "cli-help")]
fn cli_handle_help_or_continue_inner(
    raw_args: &[String],
    cmd: &clap::Command,
    config: &HelpConfig,
) -> Result<Option<String>, Value> {
    let parsed = parse_help_request(raw_args, cmd);
    if !parsed.help_requested {
        return Ok(None);
    }
    if let Some(error) = parsed.output_error {
        let event = build_cli_error(
            &error,
            Some("valid help output formats: plain, markdown, json, yaml"),
        );
        return Err(event.into());
    }

    let (scope, format) = resolve_help_options(&parsed, cmd, config);
    let path: Vec<&str> = parsed.subcommand_path.iter().map(String::as_str).collect();
    // The one blessed structured shape: `--help --output json|yaml` wraps the
    // help schema in a protocol-v1 `kind:"result"` event (`code:"help"`), the
    // same envelope discipline as the version handler. Plain/Markdown stay text.
    if matches!(format, HelpFormat::Json | HelpFormat::Yaml) {
        let event = help_result_event(&scoped_help_command(cmd, &path), scope);
        let rendered = match format {
            HelpFormat::Json => serialize_json_output(event.as_value()),
            HelpFormat::Yaml => render_yaml(
                event.as_value(),
                &OutputOptions {
                    redaction: Redactor::new().policy(RedactionPolicy::Off),
                    style: PlainStyle::Raw,
                },
            ),
            HelpFormat::Plain | HelpFormat::Markdown => unreachable!(),
        };
        return Ok(Some(format!("{rendered}\n")));
    }
    let options = HelpOptions { scope, format };
    Ok(Some(cli_render_help_with_options(cmd, &path, &options)))
}

/// Wrap a help model in the protocol-v1 terminal result every structured help
/// path shares.
#[cfg(feature = "cli-help")]
fn help_result_event(selected: &ScopedHelpCommand, scope: HelpScope) -> crate::protocol::Event {
    crate::protocol::json_result(serde_json::json!({
        "code": "help",
        "help": build_selected_help_schema(selected, scope),
    }))
    .trace(serde_json::json!({}))
    .build()
}

#[cfg(feature = "cli-help")]
fn resolve_help_options(
    parsed: &ParsedHelpRequest,
    cmd: &clap::Command,
    config: &HelpConfig,
) -> (HelpScope, HelpFormat) {
    // Scope and format are orthogonal: `--recursive` selects one-level vs
    // recursive, while `--output` independently decides the format.
    let scope = if parsed.recursive_requested {
        HelpScope::Recursive
    } else {
        config.default_scope
    };
    let format = parsed
        .output_format
        .or_else(|| command_output_default(cmd, &parsed.subcommand_path))
        .unwrap_or(config.default_format);
    (scope, format)
}

#[cfg(feature = "cli-help")]
fn command_output_default(cmd: &clap::Command, path: &[String]) -> Option<HelpFormat> {
    let mut current = cmd;
    let mut format = command_output_default_here(current);
    for name in path {
        let Some(next) = current.find_subcommand(name) else {
            break;
        };
        current = next;
        if let Some(selected) = command_output_default_here(current) {
            format = Some(selected);
        }
    }
    format
}

#[cfg(feature = "cli-help")]
fn command_output_default_here(cmd: &clap::Command) -> Option<HelpFormat> {
    cmd.get_arguments()
        .find(|arg| arg.get_long() == Some("output"))
        .and_then(|arg| arg.get_default_values().first())
        .and_then(|value| value.to_str())
        .and_then(HelpFormat::parse)
}

#[cfg(feature = "cli-help")]
struct ScopedHelpCommand {
    local_command: clap::Command,
    conventional_command: clap::Command,
    command_path: String,
    inherited_arguments_from: Vec<String>,
}

#[cfg(feature = "cli-help")]
fn scoped_help_command(cmd: &clap::Command, path: &[&str]) -> ScopedHelpCommand {
    let mut current = cmd;
    let mut command_names = vec![cmd.get_bin_name().unwrap_or(cmd.get_name()).to_string()];
    let mut inherited_groups = Vec::new();

    for name in path {
        let Some(next) = current.find_subcommand(name) else {
            break;
        };
        let globals: Vec<clap::Arg> = current
            .get_arguments()
            .filter(|arg| arg.is_global_set() && !arg.is_hide_set())
            .cloned()
            .collect();
        if !globals.is_empty() {
            inherited_groups.push((command_names.join(" "), globals));
        }
        current = next;
        command_names.push(next.get_name().to_string());
    }

    let command_path = command_names.join(" ");
    let local_command = current.clone().bin_name(command_path.clone());
    let mut conventional_command = local_command.clone();
    let mut inherited_arguments_from = Vec::new();
    for (source, arguments) in inherited_groups {
        let mut inherited_from_source = false;
        for argument in arguments {
            let already_declared = conventional_command
                .get_arguments()
                .any(|candidate| candidate.get_id() == argument.get_id());
            if !already_declared {
                conventional_command = conventional_command.arg(argument);
                inherited_from_source = true;
            }
        }
        if inherited_from_source {
            inherited_arguments_from.push(source);
        }
    }

    ScopedHelpCommand {
        local_command,
        conventional_command,
        command_path,
        inherited_arguments_from,
    }
}

#[cfg(feature = "cli-help")]
fn render_help_one_level_plain(cmd: &clap::Command) -> String {
    enriched_help_command(cmd).render_long_help().to_string()
}

#[cfg(feature = "cli-help")]
fn redact_secret_help_defaults(mut cmd: clap::Command) -> clap::Command {
    let context = RedactionContext::default();
    let ids: Vec<String> = cmd
        .get_arguments()
        .filter(|arg| !arg.get_default_values().is_empty())
        .filter(|arg| help_arg_is_secret(arg, &context))
        .map(|arg| arg.get_id().to_string())
        .collect();
    for id in ids {
        cmd = cmd.mut_arg(id, |arg| arg.default_value("***"));
    }
    cmd
}

#[cfg(feature = "cli-help")]
fn help_arg_is_secret(arg: &clap::Arg, context: &RedactionContext) -> bool {
    is_secret_flag_name(arg.get_id().as_ref(), context)
        || arg
            .get_long()
            .is_some_and(|long| is_secret_flag_name(long, context))
}

/// Clone `cmd` and fold the afdata-handled help modifiers into clap's own
/// `-h, --help` description.
///
/// Help is rendered by clap, which has no knowledge of the `--recursive` scope
/// modifier or the `--output` help formats (afdata consumes both before clap
/// parses). Rather than appending a separate section, we patch the description
/// of the existing help flag so the help surface is documented in place — in
/// every format, since plain/markdown render this flag and the JSON/YAML schema
/// reads it. Commands with subcommands advertise `--recursive`; leaf commands
/// only advertise the `--output` formats (they have nothing to expand).
#[cfg(feature = "cli-help")]
fn enriched_help_command(cmd: &clap::Command) -> clap::Command {
    // The canonical discovery surface is `--help`; never let clap synthesize
    // its legacy `help` pseudo-command while rendering an isolated subtree.
    let cmd = redact_secret_help_defaults(cmd.clone()).disable_help_subcommand(true);
    let description = if visible_subcommands(&cmd).next().is_some() {
        HELP_FLAG_WITH_SUBCOMMANDS
    } else {
        HELP_FLAG_LEAF
    };
    let has_explicit_version = cmd.get_arguments().any(|arg| {
        arg.get_long() == Some("version") || arg.get_short().is_some_and(|short| short == 'V')
    });
    let exposes_auto_version = !cmd.is_disable_version_flag_set()
        && (cmd.get_version().is_some() || cmd.get_long_version().is_some());
    // `--help` has no short. `-h` would be a byte-identical alias, and the whole
    // point of this release is that help answers in the command's output format:
    // a short flag that returns JSON is a trap for the human reaching for it, and
    // an agent gains nothing from two spellings. `-h` therefore stays the
    // application's to spend (`--host`, the psql/mysql convention).
    let help_flag = clap::Arg::new("help")
        .long("help")
        .help(description)
        .long_help(description)
        .action(clap::ArgAction::Help);
    // clap auto-generates its help flag lazily during build, so `mut_arg` cannot
    // reach it yet. The built-in version flag is lazy for the same reason, so
    // materialize both before the compact schema reads `get_arguments()`.
    let mut enriched = cmd.disable_help_flag(true).arg(help_flag);
    if exposes_auto_version && !has_explicit_version {
        enriched = enriched.disable_version_flag(true).arg(
            clap::Arg::new("version")
                .long("version")
                .help("Print version")
                .action(clap::ArgAction::Version),
        );
    }
    enriched
}

/// Description for the `--help` flag on commands that have subcommands.
#[cfg(feature = "cli-help")]
const HELP_FLAG_WITH_SUBCOMMANDS: &str = "Print help. Add --recursive to expand every nested subcommand; \
     add --output plain|json|yaml|markdown to choose the format.";

/// Description for the `--help` flag on leaf commands (no subcommands).
#[cfg(feature = "cli-help")]
const HELP_FLAG_LEAF: &str =
    "Print help. Add --output plain|json|yaml|markdown to choose the format.";

#[cfg(feature = "cli-help")]
fn render_help_recursive_plain(cmd: &clap::Command, parent_path: &[&str], buf: &mut String) {
    use std::fmt::Write;

    let mut cmd_path = parent_path.to_vec();
    cmd_path.push(if parent_path.is_empty() {
        cmd.get_bin_name().unwrap_or(cmd.get_name())
    } else {
        cmd.get_name()
    });
    let path = cmd_path.join(" ");
    let usage = compact_usage(cmd, &path);
    if let Some(about) = cmd.get_about() {
        let _ = writeln!(buf, "{usage} — {about}");
    } else {
        let _ = writeln!(buf, "{usage}");
    }

    let is_target = parent_path.is_empty();
    let owned = if is_target {
        Some(enriched_help_command(cmd))
    } else {
        None
    };
    let source = owned.as_ref().unwrap_or(cmd);
    for arg in source.get_arguments().filter(|arg| !arg.is_hide_set()) {
        let _ = writeln!(buf, "  {}", compact_plain_argument(arg));
    }

    for sub in visible_subcommands(cmd) {
        if !buf.ends_with('\n') {
            let _ = writeln!(buf);
        }
        render_help_recursive_plain(sub, &cmd_path, buf);
    }
}

#[cfg(feature = "cli-help")]
fn compact_usage(cmd: &clap::Command, path: &str) -> String {
    let rendered = cmd.clone().render_usage().to_string();
    let usage = rendered.strip_prefix("Usage: ").unwrap_or(&rendered);
    let invocation = cmd.get_bin_name().unwrap_or(cmd.get_name());
    usage.strip_prefix(invocation).map_or_else(
        || format!("{path} {usage}"),
        |suffix| format!("{path}{suffix}"),
    )
}

#[cfg(feature = "cli-help")]
fn compact_usage_suffix(cmd: &clap::Command) -> String {
    let rendered = cmd.clone().render_usage().to_string();
    let usage = rendered.strip_prefix("Usage: ").unwrap_or(&rendered);
    usage
        .strip_prefix(cmd.get_bin_name().unwrap_or(cmd.get_name()))
        .unwrap_or(usage)
        .trim()
        .to_string()
}

#[cfg(feature = "cli-help")]
fn compact_plain_argument(arg: &clap::Arg) -> String {
    use std::fmt::Write;

    let mut rendered = String::new();
    if let Some(short) = arg.get_short() {
        let _ = write!(rendered, "-{short}");
        if arg.get_long().is_some() {
            rendered.push_str(", ");
        }
    }
    if let Some(long) = arg.get_long() {
        let _ = write!(rendered, "--{long}");
    } else if arg.get_short().is_none() {
        rendered.push_str(
            &arg.get_value_names()
                .and_then(|names| names.first())
                .map_or_else(|| arg.get_id().to_string(), ToString::to_string),
        );
    }
    if argument_takes_values(arg)
        && let Some(names) = arg.get_value_names()
        && (arg.get_long().is_some() || arg.get_short().is_some())
    {
        for name in names {
            let _ = write!(rendered, " <{name}>");
        }
    }
    if matches!(
        arg.get_action(),
        clap::ArgAction::Append | clap::ArgAction::Count
    ) {
        rendered.push_str("...");
    }
    let defaults = redacted_default_values(arg);
    if !defaults.is_empty() {
        let _ = write!(rendered, " [default={}]", defaults.join(","));
    }
    if let Some(help) = arg.get_help() {
        let _ = write!(rendered, ": {help}");
    }
    rendered
}

#[cfg(feature = "cli-help")]
fn render_help_markdown(cmd: &clap::Command, names: &[String], scope: HelpScope) -> String {
    let mut buf = String::new();
    render_markdown_command(cmd, names, &mut buf, 1, true);
    if matches!(scope, HelpScope::Recursive) {
        render_markdown_descendants(cmd, names, &mut buf, 2);
    }
    buf
}

#[cfg(feature = "cli-help")]
fn render_markdown_descendants(
    cmd: &clap::Command,
    parent_names: &[String],
    buf: &mut String,
    level: usize,
) {
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" || sub.is_hide_set() {
            continue;
        }
        let mut names = parent_names.to_vec();
        names.push(sub.get_name().to_string());
        render_markdown_command(sub, &names, buf, level, false);
        render_markdown_descendants(sub, &names, buf, level.saturating_add(1));
    }
}

#[cfg(feature = "cli-help")]
fn render_markdown_command(
    cmd: &clap::Command,
    names: &[String],
    buf: &mut String,
    level: usize,
    enrich: bool,
) {
    use std::fmt::Write;

    if !buf.is_empty() {
        let _ = writeln!(buf);
    }
    let heading_level = "#".repeat(level.max(1));
    // The joined path is the invocable command, not the (possibly branded)
    // `Command::name` — a reader must be able to copy the heading and the
    // fenced `Usage:` line straight into a shell.
    let path = names.join(" ");
    if let Some(about) = cmd.get_about() {
        let _ = writeln!(buf, "{heading_level} {path} - {about}");
    } else {
        let _ = writeln!(buf, "{heading_level} {path}");
    }
    if let Some(long_about) = markdown_long_about(cmd) {
        let _ = writeln!(buf);
        write_trimmed_help(buf, &long_about);
    }
    let _ = writeln!(buf);
    let _ = writeln!(buf, "```text");
    let help = markdown_help_block_command(cmd, enrich)
        .bin_name(path.clone())
        .render_long_help();
    write_trimmed_help(buf, &help.to_string());
    if !buf.ends_with('\n') {
        let _ = writeln!(buf);
    }
    let _ = writeln!(buf, "```");
}

#[cfg(feature = "cli-help")]
fn markdown_long_about(cmd: &clap::Command) -> Option<String> {
    let long_about = cmd.get_long_about()?.to_string();
    let rendered = match cmd.get_about() {
        Some(about) => {
            let about_str = about.to_string();
            if long_about.trim() == format!("{} - {}", cmd.get_name(), about_str) {
                return None;
            }
            strip_leading_about_paragraph(&long_about, &about_str)
        }
        None => long_about.as_str(),
    };
    let rendered = rendered.trim_matches(['\r', '\n']);
    if rendered.is_empty() {
        None
    } else {
        Some(rendered.to_string())
    }
}

#[cfg(feature = "cli-help")]
fn strip_leading_about_paragraph<'a>(long_about: &'a str, about: &str) -> &'a str {
    let long_about = long_about.trim_start_matches(['\r', '\n']);
    let Some(rest) = long_about.strip_prefix(about) else {
        return long_about;
    };
    if rest.is_empty() {
        return "";
    }
    rest.strip_prefix("\r\n\r\n")
        .or_else(|| rest.strip_prefix("\n\n"))
        .unwrap_or(long_about)
}

#[cfg(feature = "cli-help")]
fn markdown_help_block_command(cmd: &clap::Command, enrich: bool) -> clap::Command {
    let cmd = if enrich {
        enriched_help_command(cmd)
    } else {
        redact_secret_help_defaults(cmd.clone()).disable_help_subcommand(true)
    };
    cmd.about(None::<&str>).long_about(None::<&str>)
}

#[cfg(feature = "cli-help")]
fn write_trimmed_help(buf: &mut String, help: &str) {
    use std::fmt::Write;

    for line in help.lines() {
        let _ = writeln!(buf, "{}", line.trim_end());
    }
}

#[cfg(feature = "cli-help")]
struct ParsedHelpRequest {
    help_requested: bool,
    recursive_requested: bool,
    output_format: Option<HelpFormat>,
    output_error: Option<String>,
    subcommand_path: Vec<String>,
}

#[cfg(feature = "cli-help")]
fn parse_help_request(raw_args: &[String], cmd: &clap::Command) -> ParsedHelpRequest {
    let args = match raw_args.first() {
        Some(first) if first.starts_with('-') || cmd.find_subcommand(first).is_some() => raw_args,
        _ => raw_args.get(1..).unwrap_or(&[]),
    };
    let mut help_requested = false;
    let mut recursive_requested = false;
    let mut output_format = None;
    let mut output_error = None;
    let mut subcommand_path = Vec::new();
    let mut current = cmd;

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            break;
        }

        let (flag_name, inline_value) = split_flag(arg);
        // Long form only. `-h` and `--help` would be byte-identical aliases, so
        // the short buys nothing for an agent and misleads a human (it answers in
        // the command's output format, which for an agent-first CLI is JSON, not
        // text). Leaving the short unclaimed also keeps it available to apps that
        // spend `-h` on `--host`, and removes a whole class of collision.
        if arg == "--help" {
            help_requested = true;
            i += 1;
            continue;
        }
        // `--recursive` is a help *modifier*, not a help trigger: it only
        // selects recursive scope when `--help` is also present. A bare
        // `--recursive` leaves help_requested false so the full argv falls
        // through to the application's own parser untouched.
        if arg == "--recursive" {
            recursive_requested = true;
            i += 1;
            continue;
        }
        if arg == "--json" {
            set_help_output_format(
                &mut output_format,
                HelpFormat::Json,
                "--json",
                &mut output_error,
            );
            i += 1;
            continue;
        }
        if flag_name == Some("output") {
            let value = inline_value.or_else(|| {
                args.get(i + 1)
                    .map(String::as_str)
                    .filter(|next| !next.starts_with('-'))
            });
            if let Some(value) = value {
                match HelpFormat::parse(value) {
                    Some(format) => set_help_output_format(
                        &mut output_format,
                        format,
                        &format!("--output {value}"),
                        &mut output_error,
                    ),
                    None => {
                        output_error = Some(format!(
                            "invalid --output format '{value}': expected plain, json, yaml, or markdown"
                        ));
                    }
                }
            } else {
                output_error = Some(
                    "missing value for --output: expected plain, json, yaml, or markdown"
                        .to_string(),
                );
            }
            i += if inline_value.is_some() || value.is_none() {
                1
            } else {
                2
            };
            continue;
        }
        if arg.starts_with('-') {
            i += if inline_value.is_none() && flag_takes_value(current, arg) {
                2
            } else {
                1
            };
            continue;
        }
        if let Some(sub) = current.find_subcommand(arg)
            && sub.get_name() != "help"
            && !sub.is_hide_set()
        {
            subcommand_path.push(sub.get_name().to_string());
            current = sub;
        }
        i += 1;
    }

    ParsedHelpRequest {
        help_requested,
        recursive_requested,
        output_format,
        output_error,
        subcommand_path,
    }
}

#[cfg(feature = "cli-help")]
fn set_help_output_format(
    current: &mut Option<HelpFormat>,
    next: HelpFormat,
    source: &str,
    output_error: &mut Option<String>,
) {
    if let Some(existing) = current
        && *existing != next
    {
        *output_error = Some(format!(
            "conflicting output formats: {source} conflicts with previous output format"
        ));
        return;
    }
    *current = Some(next);
}

fn split_flag(arg: &str) -> (Option<&str>, Option<&str>) {
    if let Some(stripped) = arg.strip_prefix("--") {
        if let Some((name, value)) = stripped.split_once('=') {
            (Some(name), Some(value))
        } else {
            (Some(stripped), None)
        }
    } else if let Some(stripped) = arg.strip_prefix('-') {
        (Some(stripped), None)
    } else {
        (None, None)
    }
}

#[cfg(feature = "cli-help")]
fn flag_takes_value(cmd: &clap::Command, raw_flag: &str) -> bool {
    let Some(flag) = raw_flag.strip_prefix('-') else {
        return false;
    };
    let name = flag.trim_start_matches('-');
    cmd.get_arguments().any(|arg| {
        let long_matches = arg.get_long().is_some_and(|long| long == name);
        let short_matches =
            name.len() == 1 && arg.get_short().is_some_and(|short| name.starts_with(short));
        (long_matches || short_matches)
            && matches!(
                arg.get_action(),
                clap::ArgAction::Set | clap::ArgAction::Append
            )
    })
}

#[cfg(feature = "cli-help")]
fn build_selected_help_schema(selected: &ScopedHelpCommand, scope: HelpScope) -> Value {
    let mut schema = command_schema(
        &selected.local_command,
        matches!(scope, HelpScope::Recursive),
        true,
    );
    if let Value::Object(map) = &mut schema {
        map.insert(
            "scope".to_string(),
            Value::String(help_scope_tag(scope).to_string()),
        );
        map.insert(
            "command_path".to_string(),
            Value::String(selected.command_path.clone()),
        );
        if !selected.inherited_arguments_from.is_empty() {
            map.insert(
                "inherited_arguments_from".to_string(),
                Value::Array(
                    selected
                        .inherited_arguments_from
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
    }
    schema
}

#[cfg(feature = "cli-help")]
fn help_scope_tag(scope: HelpScope) -> &'static str {
    match scope {
        HelpScope::OneLevel => "one_level",
        HelpScope::Recursive => "recursive",
    }
}

#[cfg(feature = "cli-help")]
fn command_schema(cmd: &clap::Command, recursive: bool, enrich: bool) -> Value {
    let subcommands: Vec<Value> = visible_subcommands(cmd)
        .map(|sub| {
            if recursive {
                // Descendants never re-advertise the help modifiers (enrich=false).
                command_schema(sub, true, false)
            } else {
                command_summary_schema(sub)
            }
        })
        .collect();

    let mut schema = serde_json::Map::new();
    schema.insert(
        "name".to_string(),
        Value::String(cmd.get_name().to_string()),
    );
    insert_non_empty(
        &mut schema,
        "about",
        cmd.get_about().map(ToString::to_string).unwrap_or_default(),
    );
    let usage = compact_usage_suffix(cmd);
    if !usage.is_empty() {
        schema.insert("usage".to_string(), Value::String(usage));
    }
    let arguments = command_arguments_schema(cmd, enrich);
    if !arguments.is_empty() {
        schema.insert("arguments".to_string(), Value::Array(arguments));
    }
    if !subcommands.is_empty() {
        schema.insert("subcommands".to_string(), Value::Array(subcommands));
    }
    Value::Object(schema)
}

#[cfg(feature = "cli-help")]
fn command_summary_schema(cmd: &clap::Command) -> Value {
    let mut schema = serde_json::Map::new();
    schema.insert(
        "name".to_string(),
        Value::String(cmd.get_name().to_string()),
    );
    insert_non_empty(
        &mut schema,
        "about",
        cmd.get_about().map(ToString::to_string).unwrap_or_default(),
    );
    Value::Object(schema)
}

#[cfg(feature = "cli-help")]
fn visible_subcommands(cmd: &clap::Command) -> impl Iterator<Item = &clap::Command> {
    cmd.get_subcommands()
        .filter(|sub| sub.get_name() != "help" && !sub.is_hide_set())
}

#[cfg(feature = "cli-help")]
fn command_arguments_schema(cmd: &clap::Command, enrich: bool) -> Vec<Value> {
    // For the target command, render through the enriched clone so the schema
    // documents the `-h, --help` modifiers (`--recursive`, `--output`) just like
    // the plain and markdown formats do (clap adds `--help` lazily during build,
    // so the raw command would omit it). Descendants stay un-enriched to avoid
    // repeating the same modifier doc on every command in a recursive dump.
    let owned = enrich.then(|| enriched_help_command(cmd));
    let source = owned.as_ref().unwrap_or(cmd);
    source
        .get_arguments()
        .filter(|arg| !arg.is_hide_set())
        .map(argument_schema)
        .collect()
}

/// Insert `key` only when `value` is a non-empty string.
///
/// `cli-help-v1.schema.json` sets `minLength: 1` on every optional string, and
/// the spec requires omitting empty values. Clap happily returns `Some("")` for
/// an explicit `about("")` / `help("")` / `value_name("")`, so every optional
/// string goes through this gate rather than being inserted on `Some(..)` alone.
#[cfg(feature = "cli-help")]
fn insert_non_empty(schema: &mut serde_json::Map<String, Value>, key: &str, value: String) {
    if !value.is_empty() {
        schema.insert(key.to_string(), Value::String(value));
    }
}

#[cfg(feature = "cli-help")]
fn argument_schema(arg: &clap::Arg) -> Value {
    let mut schema = serde_json::Map::new();
    if let Some(long) = arg.get_long() {
        schema.insert("name".to_string(), Value::String(format!("--{long}")));
    } else if let Some(short) = arg.get_short() {
        schema.insert("name".to_string(), Value::String(format!("-{short}")));
    } else {
        schema.insert(
            "name".to_string(),
            Value::String(
                arg.get_value_names()
                    .and_then(|names| names.first())
                    .map_or_else(|| arg.get_id().to_string(), ToString::to_string),
            ),
        );
    }
    if arg.get_long().is_some()
        && let Some(short) = arg.get_short()
    {
        schema.insert("short".to_string(), Value::String(format!("-{short}")));
    }
    insert_non_empty(
        &mut schema,
        "help",
        arg.get_help().map(ToString::to_string).unwrap_or_default(),
    );
    if arg.is_required_set() {
        schema.insert("required".to_string(), Value::Bool(true));
    }
    if arg.is_global_set() {
        schema.insert("global".to_string(), Value::Bool(true));
    }
    if matches!(
        arg.get_action(),
        clap::ArgAction::Append | clap::ArgAction::Count
    ) {
        schema.insert("repeatable".to_string(), Value::Bool(true));
    }
    if (arg.get_long().is_some() || arg.get_short().is_some())
        && argument_takes_values(arg)
        && let Some(names) = arg.get_value_names()
    {
        if names.len() == 1 {
            insert_non_empty(&mut schema, "value", names[0].to_string());
        } else {
            let values: Vec<Value> = names
                .iter()
                .map(ToString::to_string)
                .filter(|name| !name.is_empty())
                .map(Value::String)
                .collect();
            if !values.is_empty() {
                schema.insert("values".to_string(), Value::Array(values));
            }
        }
    }
    let defaults = redacted_default_values(arg);
    if !defaults.is_empty() {
        if defaults.len() == 1 {
            schema.insert("default".to_string(), Value::String(defaults[0].clone()));
        } else {
            schema.insert(
                "defaults".to_string(),
                Value::Array(defaults.into_iter().map(Value::String).collect()),
            );
        }
    }
    Value::Object(schema)
}

#[cfg(feature = "cli-help")]
fn argument_takes_values(arg: &clap::Arg) -> bool {
    matches!(
        arg.get_action(),
        clap::ArgAction::Set | clap::ArgAction::Append
    )
}

#[cfg(feature = "cli-help")]
fn redacted_default_values(arg: &clap::Arg) -> Vec<String> {
    // An author who called `hide_default_value(true)` hid it from clap's own
    // help; the structured model must not disclose what plain help withholds.
    if arg.is_hide_default_value_set() {
        return Vec::new();
    }
    arg.get_default_values()
        .iter()
        .map(|value| {
            if help_arg_is_secret(arg, &RedactionContext::default()) {
                "***".to_string()
            } else {
                value.to_string_lossy().to_string()
            }
        })
        .collect()
}
