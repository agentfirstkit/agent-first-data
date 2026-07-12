use crate::cli::CliProtocolMode;
use crate::formatting::{output_yaml_with_options, serialize_json_output};
use crate::protocol::build_cli_error;
use crate::redaction::{
    OutputOptions, OutputStyle, RedactionContext, RedactionPolicy, Redactor, is_secret_flag_name,
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
    /// Scope used for `--help` / `-h` when neither `--recursive` nor a
    /// configured `recursive_flag` is present.
    pub default_scope: HelpScope,
    /// Format used for help when no explicit output flag is present.
    pub default_format: HelpFormat,
    /// Optional extra alias for the built-in `--recursive` scope modifier.
    ///
    /// `--recursive` is always recognized; set this only to accept an
    /// additional custom flag name (for example `--full`). Like `--recursive`,
    /// the alias is a *modifier* that selects recursive scope when `--help` is
    /// present; on its own it does not trigger help.
    pub recursive_flag: Option<&'static str>,
    /// Optional output flag to read help format from, for example `--output`.
    pub output_flag: Option<&'static str>,
    /// Whether an explicit output flag can override `default_format`.
    pub allow_output_format: bool,
    /// Envelope mode for structured help output and early errors.
    pub protocol_mode: CliProtocolMode,
}

#[cfg(feature = "cli-help")]
impl HelpConfig {
    /// Construct a custom help handler configuration.
    pub const fn new(default_scope: HelpScope, default_format: HelpFormat) -> Self {
        Self {
            default_scope,
            default_format,
            recursive_flag: None,
            output_flag: None,
            allow_output_format: false,
            protocol_mode: CliProtocolMode::Legacy,
        }
    }

    /// Recommended preset for human-facing CLIs.
    ///
    /// `--help` renders one-level plain help by default. Scope and format are
    /// orthogonal: `--recursive` expands the selected command subtree, while
    /// `--output json|yaml|markdown` picks the format. So `--help --recursive`
    /// is recursive plain text and `--help --recursive --output markdown` is a
    /// recursive Markdown export.
    pub const fn human_cli_default() -> Self {
        Self {
            default_scope: HelpScope::OneLevel,
            default_format: HelpFormat::Plain,
            recursive_flag: None,
            output_flag: Some("--output"),
            allow_output_format: true,
            protocol_mode: CliProtocolMode::Legacy,
        }
    }

    /// Recommended preset for agent-first CLIs that want full surface help by default.
    pub const fn agent_cli_default() -> Self {
        Self {
            default_scope: HelpScope::Recursive,
            default_format: HelpFormat::Plain,
            recursive_flag: None,
            output_flag: Some("--output"),
            allow_output_format: true,
            protocol_mode: CliProtocolMode::Legacy,
        }
    }

    /// Return a copy with a different default scope.
    pub const fn with_default_scope(mut self, scope: HelpScope) -> Self {
        self.default_scope = scope;
        self
    }

    /// Return a copy with a different default format.
    pub const fn with_default_format(mut self, format: HelpFormat) -> Self {
        self.default_format = format;
        self
    }

    /// Return a copy with a different recursive-help flag.
    pub const fn with_recursive_flag(mut self, flag: Option<&'static str>) -> Self {
        self.recursive_flag = flag;
        self
    }

    /// Return a copy with a different output flag.
    pub const fn with_output_flag(mut self, flag: Option<&'static str>) -> Self {
        self.output_flag = flag;
        self
    }

    /// Return a copy that enables or disables help format overrides.
    pub const fn with_output_format_override(mut self, enabled: bool) -> Self {
        self.allow_output_format = enabled;
        self
    }

    /// Return a copy that wraps JSON/YAML help in a protocol-v1 result event.
    pub const fn with_protocol_v1(mut self) -> Self {
        self.protocol_mode = CliProtocolMode::ProtocolV1;
        self
    }
}

/// Render help for a clap command tree with explicit scope and format.
///
/// Walks to the subcommand identified by `subcommand_path` (empty = root),
/// then renders either the selected command only (`OneLevel`) or the selected
/// command and all descendants (`Recursive`).
///
/// Requires the `cli-help` feature.
#[cfg(feature = "cli-help")]
pub fn cli_render_help_with_options(
    cmd: &clap::Command,
    subcommand_path: &[&str],
    options: &HelpOptions,
) -> String {
    let target = walk_to_subcommand(cmd, subcommand_path);
    let mut rendered = match options.format {
        HelpFormat::Plain => {
            let mut help = match options.scope {
                HelpScope::OneLevel => render_help_one_level_plain(target),
                HelpScope::Recursive => {
                    let mut buf = String::new();
                    render_help_recursive_plain(target, &[], &mut buf);
                    buf
                }
            };
            append_afdata_version_line(&mut help);
            help
        }
        HelpFormat::Markdown => {
            let mut help = render_help_markdown(cmd, subcommand_path, options.scope);
            append_afdata_version_line(&mut help);
            help
        }
        HelpFormat::Json => {
            serialize_json_output(&build_help_schema(cmd, subcommand_path, options.scope))
        }
        HelpFormat::Yaml => output_yaml_with_options(
            &build_help_schema(cmd, subcommand_path, options.scope),
            &OutputOptions {
                redaction: Redactor::new().policy(RedactionPolicy::RedactionNone),
                style: OutputStyle::Raw,
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

#[cfg(feature = "cli-help")]
fn append_afdata_version_line(help: &mut String) {
    const LINE: &str = concat!("AFDATA: ", env!("CARGO_PKG_VERSION"));
    if help.lines().any(|line| line.trim() == LINE) {
        return;
    }
    if !help.is_empty() && !help.ends_with('\n') {
        help.push('\n');
    }
    help.push_str(LINE);
    help.push('\n');
}

#[cfg(feature = "cli-help")]
fn afdata_versions_value() -> Value {
    serde_json::json!({ "afdata": env!("CARGO_PKG_VERSION") })
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
    let parsed = parse_help_request(raw_args, cmd, config);
    if !parsed.help_requested {
        return Ok(None);
    }
    if let Some(error) = parsed.output_error {
        let event = build_cli_error(
            &error,
            Some("valid help output formats: plain, markdown, json, yaml"),
        );
        let mut event_value: Value = event.into();
        if config.protocol_mode == CliProtocolMode::ProtocolV1
            && let Some(obj) = event_value.as_object_mut()
        {
            obj.insert("trace".to_string(), serde_json::json!({}));
        }
        return Err(event_value);
    }

    let (scope, format) = resolve_help_options(&parsed, config);
    let path: Vec<&str> = parsed.subcommand_path.iter().map(String::as_str).collect();
    let options = HelpOptions { scope, format };
    if config.protocol_mode == CliProtocolMode::ProtocolV1
        && matches!(format, HelpFormat::Json | HelpFormat::Yaml)
    {
        #[allow(clippy::expect_used)]
        let event = crate::protocol::json_result(serde_json::json!({
            "code": "help",
            "help": build_help_schema(cmd, &path, scope),
        }))
        .trace(serde_json::json!({}))
        .build()
        .expect("help builder failed");
        let rendered = match format {
            HelpFormat::Json => serialize_json_output(&event),
            HelpFormat::Yaml => output_yaml_with_options(
                &event,
                &OutputOptions {
                    redaction: Redactor::new().policy(RedactionPolicy::RedactionNone),
                    style: OutputStyle::Raw,
                },
            ),
            HelpFormat::Plain | HelpFormat::Markdown => unreachable!(),
        };
        return Ok(Some(format!("{rendered}\n")));
    }
    Ok(Some(cli_render_help_with_options(cmd, &path, &options)))
}

#[cfg(feature = "cli-help")]
fn resolve_help_options(
    parsed: &ParsedHelpRequest,
    config: &HelpConfig,
) -> (HelpScope, HelpFormat) {
    // Scope and format are orthogonal: `--recursive` (or the configured
    // recursive flag, or a recursive default_scope) decides one-level vs
    // recursive, while `--output` independently decides the format.
    let scope = if parsed.recursive_requested {
        HelpScope::Recursive
    } else {
        config.default_scope
    };
    let format = if config.allow_output_format {
        parsed.output_format.unwrap_or(config.default_format)
    } else {
        config.default_format
    };
    (scope, format)
}

#[cfg(feature = "cli-help")]
fn walk_to_subcommand<'a>(cmd: &'a clap::Command, path: &[&str]) -> &'a clap::Command {
    let mut current = cmd;
    for name in path {
        current = current.find_subcommand(name).unwrap_or(current);
    }
    current
}

#[cfg(feature = "cli-help")]
fn walk_to_subcommand_with_names<'a>(
    cmd: &'a clap::Command,
    path: &[&str],
) -> (&'a clap::Command, Vec<String>) {
    let mut current = cmd;
    let mut names = vec![cmd.get_name().to_string()];
    for name in path {
        if let Some(next) = current.find_subcommand(name) {
            current = next;
            names.push(next.get_name().to_string());
        } else {
            break;
        }
    }
    (current, names)
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
    let cmd = redact_secret_help_defaults(cmd.clone());
    let description = if visible_subcommands(&cmd).next().is_some() {
        HELP_FLAG_WITH_SUBCOMMANDS
    } else {
        HELP_FLAG_LEAF
    };
    // clap auto-generates `-h, --help` lazily during build, so `mut_arg` cannot
    // reach it yet. Replace it with an explicit flag carrying the enriched
    // description. This command is only rendered, never parsed (afdata handles
    // `--help` before clap), so the action is immaterial.
    cmd.disable_help_flag(true).arg(
        clap::Arg::new("help")
            .short('h')
            .long("help")
            .help(description)
            .long_help(description)
            .action(clap::ArgAction::Help),
    )
}

/// Description for the `-h, --help` flag on commands that have subcommands.
#[cfg(feature = "cli-help")]
const HELP_FLAG_WITH_SUBCOMMANDS: &str = "Print help. Add --recursive to expand every nested subcommand; \
     add --output json|yaml|markdown to render this help in another format.";

/// Description for the `-h, --help` flag on leaf commands (no subcommands).
#[cfg(feature = "cli-help")]
const HELP_FLAG_LEAF: &str =
    "Print help. Add --output json|yaml|markdown to render this help in another format.";

#[cfg(feature = "cli-help")]
fn render_help_recursive_plain(cmd: &clap::Command, parent_path: &[&str], buf: &mut String) {
    use std::fmt::Write;

    // Build the full command path (e.g. "myapp service start")
    let mut cmd_path = parent_path.to_vec();
    cmd_path.push(cmd.get_name());
    let path_str = cmd_path.join(" ");

    // Separator between commands (skip for the first one)
    if !buf.is_empty() {
        let _ = writeln!(buf);
        let _ = writeln!(buf, "{}", "═".repeat(60));
    }

    // Header: "myapp service start — description"
    if let Some(about) = cmd.get_about() {
        let _ = writeln!(buf, "{path_str} — {about}");
    } else {
        let _ = writeln!(buf, "{path_str}");
    }
    let _ = writeln!(buf);

    // Render clap's built-in help for this command (usage, args, options).
    // Only the target command (top of the recursion) advertises the help
    // modifiers; repeating them on every descendant block would be pure noise.
    let is_target = parent_path.is_empty();
    let styled = if is_target {
        enriched_help_command(cmd).render_long_help()
    } else {
        redact_secret_help_defaults(cmd.clone()).render_long_help()
    };
    let help_text = styled.to_string();
    let _ = write!(buf, "{help_text}");

    // Recurse into visible subcommands
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" || sub.is_hide_set() {
            continue; // skip clap's auto-generated "help" subcommand
        }
        render_help_recursive_plain(sub, &cmd_path, buf);
    }
}

#[cfg(feature = "cli-help")]
fn render_help_markdown(cmd: &clap::Command, subcommand_path: &[&str], scope: HelpScope) -> String {
    let (target, names) = walk_to_subcommand_with_names(cmd, subcommand_path);
    let mut buf = String::new();
    render_markdown_command(target, &names, &mut buf, 1, true);
    if matches!(scope, HelpScope::Recursive) {
        render_markdown_descendants(target, &names, &mut buf, 2);
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
    let help = markdown_help_block_command(cmd, enrich).render_long_help();
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
        redact_secret_help_defaults(cmd.clone())
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
fn parse_help_request(
    raw_args: &[String],
    cmd: &clap::Command,
    config: &HelpConfig,
) -> ParsedHelpRequest {
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
    let output_flag = config.output_flag.map(normalize_long_flag);
    let recursive_flag = config.recursive_flag.map(normalize_long_flag);

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            break;
        }

        let (flag_name, inline_value) = split_flag(arg);
        if matches!(arg, "--help" | "-h") {
            help_requested = true;
            i += 1;
            continue;
        }
        // `--recursive` is a help *modifier*, not a help trigger: it only
        // selects recursive scope when `--help` is also present. A bare
        // `--recursive` leaves help_requested false so the full argv falls
        // through to the application's own parser untouched.
        if arg == "--recursive"
            || flag_name
                .zip(recursive_flag)
                .is_some_and(|(seen, expected)| seen == expected)
        {
            recursive_requested = true;
            i += 1;
            continue;
        }
        if config.allow_output_format && arg == "--json" {
            set_help_output_format(
                &mut output_format,
                HelpFormat::Json,
                "--json",
                &mut output_error,
            );
            i += 1;
            continue;
        }
        if config.allow_output_format
            && flag_name
                .zip(output_flag)
                .is_some_and(|(seen, expected)| seen == expected)
        {
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
                        &format!("--{} {value}", output_flag.unwrap_or("output")),
                        &mut output_error,
                    ),
                    None => {
                        output_error = Some(format!(
                            "invalid --{} format '{}': expected plain, json, yaml, or markdown",
                            output_flag.unwrap_or("output"),
                            value
                        ));
                    }
                }
            } else {
                output_error = Some(format!(
                    "missing value for --{}: expected plain, json, yaml, or markdown",
                    output_flag.unwrap_or("output")
                ));
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

fn normalize_long_flag(flag: &str) -> &str {
    flag.trim_start_matches('-')
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
fn build_help_schema(cmd: &clap::Command, subcommand_path: &[&str], scope: HelpScope) -> Value {
    let (target, names) = walk_to_subcommand_with_names(cmd, subcommand_path);
    let mut schema = command_schema(target, &names, matches!(scope, HelpScope::Recursive), true);
    if let Value::Object(map) = &mut schema {
        map.insert("code".to_string(), Value::String("help".to_string()));
        map.insert(
            "scope".to_string(),
            Value::String(help_scope_tag(scope).to_string()),
        );
        map.insert("versions".to_string(), afdata_versions_value());
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
fn command_schema(cmd: &clap::Command, names: &[String], recursive: bool, enrich: bool) -> Value {
    let subcommands: Vec<Value> = visible_subcommands(cmd)
        .map(|sub| {
            let mut child_names = names.to_vec();
            child_names.push(sub.get_name().to_string());
            if recursive {
                // Descendants never re-advertise the help modifiers (enrich=false).
                command_schema(sub, &child_names, true, false)
            } else {
                command_summary_schema(sub, &child_names)
            }
        })
        .collect();

    serde_json::json!({
        "name": cmd.get_name(),
        "command_path": names.join(" "),
        "path": names,
        "about": styled_to_value(cmd.get_about()),
        "long_about": styled_to_value(cmd.get_long_about()),
        "usage": cmd.clone().render_usage().to_string(),
        "arguments": command_arguments_schema(cmd, enrich),
        "subcommands": subcommands,
    })
}

#[cfg(feature = "cli-help")]
fn command_summary_schema(cmd: &clap::Command, names: &[String]) -> Value {
    serde_json::json!({
        "name": cmd.get_name(),
        "command_path": names.join(" "),
        "path": names,
        "about": styled_to_value(cmd.get_about()),
        "long_about": styled_to_value(cmd.get_long_about()),
        "usage": Value::Null,
        "arguments": [],
        "subcommands": [],
    })
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

#[cfg(feature = "cli-help")]
fn argument_schema(arg: &clap::Arg) -> Value {
    let value_names: Vec<String> = arg
        .get_value_names()
        .map(|names| names.iter().map(ToString::to_string).collect())
        .unwrap_or_default();
    let default_values: Vec<String> = arg
        .get_default_values()
        .iter()
        .map(|value| {
            if help_arg_is_secret(arg, &RedactionContext::default()) {
                "***".to_string()
            } else {
                value.to_string_lossy().to_string()
            }
        })
        .collect();
    serde_json::json!({
        "id": arg.get_id().to_string(),
        "kind": if arg.get_long().is_some() || arg.get_short().is_some() { "option" } else { "argument" },
        "long": arg.get_long(),
        "short": arg.get_short().map(|c| c.to_string()),
        "help": styled_to_value(arg.get_help()),
        "long_help": styled_to_value(arg.get_long_help()),
        "required": arg.is_required_set(),
        "action": format!("{:?}", arg.get_action()),
        "value_names": value_names,
        "default_values": default_values,
    })
}

#[cfg(feature = "cli-help")]
fn styled_to_value(value: Option<&clap::builder::StyledStr>) -> Value {
    value.map_or(Value::Null, |s| Value::String(s.to_string()))
}
