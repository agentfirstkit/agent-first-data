use crate::formatting::{
    output_json, output_json_with_options, output_plain, output_plain_with_options, output_yaml,
    output_yaml_with_options,
};
use crate::protocol::{
    Event, LogLevel, json_error, json_log, json_progress, json_result, validate_protocol_event,
};
use crate::redaction::OutputOptions;
use serde_json::Value;

// ═══════════════════════════════════════════
// Public API: CLI Helpers
// ═══════════════════════════════════════════

/// Output format for CLI and pipe/MCP modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Yaml,
    Plain,
}

/// Parsed and normalized log filters (trimmed, lowercased, deduplicated).
///
/// Filters enable selective tracing output via `--log` flags. An empty set means
/// no filtering (no logs emitted). The set "all" or "*" means all logs emitted.
/// Otherwise, a log message is emitted iff its event name (lowercased) starts
/// with any filter string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogFilters(Vec<String>);

impl LogFilters {
    /// Create a new LogFilters from filter strings. Entries are trimmed,
    /// lowercased, and de-duplicated; empty entries are dropped.
    pub fn new<I, S>(filters: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out: Vec<String> = Vec::new();
        for entry in filters {
            let s = entry.as_ref().trim().to_ascii_lowercase();
            if !s.is_empty() && !out.contains(&s) {
                out.push(s);
            }
        }
        Self(out)
    }

    /// Check if an event should be logged based on these filters.
    ///
    /// Returns `false` if empty (no logs). Returns `true` if contains "all" or "*".
    /// Otherwise returns `true` iff the lowercased event name starts with any filter.
    pub fn enabled(&self, event: &str) -> bool {
        if self.0.is_empty() {
            return false;
        }
        let event_lower = event.to_ascii_lowercase();
        if self.0.contains(&"all".to_string()) || self.0.contains(&"*".to_string()) {
            return true;
        }
        self.0.iter().any(|filter| event_lower.starts_with(filter))
    }

    /// Check if this filter set is empty (no filters configured).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Access the underlying filter strings as a slice.
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

/// Protocol envelope behavior for early CLI exits such as `--version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliProtocolMode {
    /// Preserve the historical structured payload shape.
    Legacy,
    /// Emit a strict AFDATA protocol-v1 event with an empty trace object.
    ProtocolV1,
}

/// Configuration for pre-parser `--version` handling.
///
/// This helper scans raw argv before the application's argument parser so
/// `--version --output json` can return an AFDATA event instead of letting
/// clap or another parser print conventional plain text and exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionConfig {
    /// Format used for `--version` when no explicit output flag is present.
    ///
    /// `Some(format)` renders an AFDATA `kind:"result"` version event in that
    /// format. `None` preserves conventional CLI output: `<name> <version>`.
    pub default_output: Option<OutputFormat>,
    /// Optional long output flag to read, for example `--output`.
    pub output_flag: Option<&'static str>,
    /// Optional short output flag to read, for example `-o`.
    pub output_short: Option<char>,
    /// Whether an explicit output flag can override `default_output`.
    pub allow_output_format: bool,
    /// Envelope mode for structured version output and early errors.
    pub protocol_mode: CliProtocolMode,
}

impl VersionConfig {
    /// Construct a custom version handler configuration.
    pub const fn new(default_output: Option<OutputFormat>) -> Self {
        Self {
            default_output,
            output_flag: None,
            output_short: None,
            allow_output_format: false,
            protocol_mode: CliProtocolMode::Legacy,
        }
    }

    /// Structured bare-version preset.
    ///
    /// A bare `--version` is a JSON AFDATA event. Most CLIs should prefer
    /// [`Self::conventional_default`] so human `--version` stays familiar while
    /// explicit `--output json|yaml|plain` remains structured.
    pub const fn agent_cli_default() -> Self {
        Self {
            default_output: Some(OutputFormat::Json),
            output_flag: Some("--output"),
            output_short: None,
            allow_output_format: true,
            protocol_mode: CliProtocolMode::Legacy,
        }
    }

    /// Recommended preset: keep conventional bare version text while still
    /// honoring explicit `--output json|yaml|plain`.
    pub const fn conventional_default() -> Self {
        Self {
            default_output: None,
            output_flag: Some("--output"),
            output_short: None,
            allow_output_format: true,
            protocol_mode: CliProtocolMode::Legacy,
        }
    }

    /// Return a copy with a different default output.
    pub const fn with_default_output(mut self, default_output: Option<OutputFormat>) -> Self {
        self.default_output = default_output;
        self
    }

    /// Return a copy with a different long output flag.
    pub const fn with_output_flag(mut self, flag: Option<&'static str>) -> Self {
        self.output_flag = flag;
        self
    }

    /// Return a copy with a different short output flag.
    pub const fn with_output_short(mut self, flag: Option<char>) -> Self {
        self.output_short = flag;
        self
    }

    /// Return a copy that enables or disables explicit output overrides.
    pub const fn with_output_format_override(mut self, enabled: bool) -> Self {
        self.allow_output_format = enabled;
        self
    }

    /// Return a copy that emits protocol-v1 structured early exits.
    pub const fn with_protocol_v1(mut self) -> Self {
        self.protocol_mode = CliProtocolMode::ProtocolV1;
        self
    }
}

/// Parse `--output` flag value into [`OutputFormat`].
///
/// Returns `Err` with a message suitable for passing to [`build_cli_error`] on unknown values.
///
/// ```
/// use agent_first_data::{cli_parse_output, OutputFormat};
/// assert!(matches!(cli_parse_output("json"), Ok(OutputFormat::Json)));
/// assert!(cli_parse_output("xml").is_err());
/// ```
pub fn cli_parse_output(s: &str) -> Result<OutputFormat, String> {
    match s {
        "json" => Ok(OutputFormat::Json),
        "yaml" => Ok(OutputFormat::Yaml),
        "plain" => Ok(OutputFormat::Plain),
        _ => Err(format!(
            "invalid --output format '{s}': expected json, yaml, or plain"
        )),
    }
}

/// Normalize `--log` flag entries: trim, lowercase, deduplicate, remove empty.
///
/// Accepts pre-split entries as produced by clap's `value_delimiter = ','`.
///
/// ```
/// use agent_first_data::{cli_parse_log_filters, LogFilters};
/// let f = cli_parse_log_filters(&["Query", " error ", "query"]);
/// assert_eq!(f, LogFilters::new(["query", "error"]));
/// ```
pub fn cli_parse_log_filters<S: AsRef<str>>(entries: &[S]) -> LogFilters {
    LogFilters::new(entries.iter().map(AsRef::as_ref))
}

/// Dispatch output formatting by [`OutputFormat`].
///
/// Equivalent to calling [`output_json`], [`output_yaml`], or [`output_plain`] directly.
///
/// ```
/// use agent_first_data::{cli_output, json_result, OutputFormat};
/// let v = json_result(serde_json::json!({"ok": true})).build().expect("valid afdata event");
/// let s = cli_output(v.as_value(), OutputFormat::Plain);
/// assert!(s.contains("kind=result"));
/// ```
pub fn cli_output(value: &Value, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => output_json(value),
        OutputFormat::Yaml => output_yaml(value),
        OutputFormat::Plain => output_plain(value),
    }
}

/// Dispatch output formatting by [`OutputFormat`] with configurable output options.
///
/// JSON output ignores [`OutputStyle`] and always preserves original keys and values after
/// redaction. YAML and plain output use the requested style.
pub fn cli_output_with_options(
    value: &Value,
    format: OutputFormat,
    output_options: &OutputOptions,
) -> String {
    match format {
        OutputFormat::Json => output_json_with_options(value, output_options),
        OutputFormat::Yaml => output_yaml_with_options(value, output_options),
        OutputFormat::Plain => output_plain_with_options(value, output_options),
    }
}

/// Error returned by [`CliEmitter`].
#[derive(Debug)]
pub enum CliEmitterError {
    Validation(String),
    Lifecycle(String),
    Write(std::io::Error),
}

impl std::fmt::Display for CliEmitterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(err) | Self::Lifecycle(err) => f.write_str(err),
            Self::Write(err) => write!(f, "failed to write CLI event: {err}"),
        }
    }
}

impl CliEmitterError {
    /// Return the underlying writer error, when event emission failed during I/O.
    pub const fn io_error(&self) -> Option<&std::io::Error> {
        match self {
            Self::Write(err) => Some(err),
            Self::Validation(_) | Self::Lifecycle(_) => None,
        }
    }

    /// Return the underlying writer error kind, when available.
    pub fn io_error_kind(&self) -> Option<std::io::ErrorKind> {
        self.io_error().map(std::io::Error::kind)
    }
}

impl std::error::Error for CliEmitterError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.io_error()
            .map(|err| err as &(dyn std::error::Error + 'static))
    }
}

impl From<std::io::Error> for CliEmitterError {
    fn from(err: std::io::Error) -> Self {
        Self::Write(err)
    }
}

/// Stateful emitter for finite structured CLI executions.
///
/// The output format and redaction policy are fixed when the emitter is
/// created. Emitting after a terminal event, emitting a repeated terminal
/// event, and writer failures all return explicit errors.
///
/// 0.16 API: Accepts typed Event, provides semantic convenience methods,
/// and supports per-log default field provider.
pub struct CliEmitter<W: std::io::Write> {
    writer: W,
    format: OutputFormat,
    output_options: OutputOptions,
    strict_protocol: bool,
    terminal_emitted: bool,
    log_fields_provider: Option<Box<dyn Fn() -> Value>>,
}

impl<W: std::io::Write> CliEmitter<W> {
    /// Create a new CLI emitter with default output options.
    pub fn new(writer: W, format: OutputFormat) -> Self {
        Self::with_options(writer, format, OutputOptions::default())
    }

    /// Create a new CLI emitter with custom output options.
    pub fn with_options(writer: W, format: OutputFormat, output_options: OutputOptions) -> Self {
        Self {
            writer,
            format,
            output_options,
            strict_protocol: false,
            terminal_emitted: false,
            log_fields_provider: None,
        }
    }

    /// Require the AFDATA recommended strict profile for every emitted event.
    pub fn with_strict_protocol(mut self) -> Self {
        self.strict_protocol = true;
        self
    }

    /// Set a provider for default log fields.
    ///
    /// The provider is called for every log event (via emit_log or emit with kind:log).
    /// Its output is merged as extension fields; explicit call-site fields take precedence.
    /// The provider must not write reserved fields (message, level); violations return a typed error.
    pub fn with_log_fields<F>(mut self, provider: F) -> Self
    where
        F: Fn() -> Value + 'static,
    {
        self.log_fields_provider = Some(Box::new(provider));
        self
    }

    /// Emit a typed Event (unified entry for all event kinds).
    ///
    /// Accepts only SDK-constructed Event; for dynamic JSON, use emit_validated_value.
    pub fn emit(&mut self, event: Event) -> Result<(), CliEmitterError> {
        let value = event.into_value();
        self.write_event(value)
    }

    /// Emit and validate dynamic JSON, then apply redaction/formatting/write.
    ///
    /// Runs strict validation first, ensuring the dynamic JSON is safe.
    pub fn emit_validated_value(&mut self, value: Value) -> Result<(), CliEmitterError> {
        validate_protocol_event(&value, true).map_err(CliEmitterError::Validation)?;
        self.write_event(value)
    }

    /// Convenience: build and emit a result event.
    pub fn emit_result(&mut self, payload: Value) -> Result<(), CliEmitterError> {
        #[allow(clippy::expect_used)]
        self.emit(
            json_result(payload)
                .build()
                .expect("json_result: builder failed unexpectedly"),
        )
    }

    /// Convenience: build and emit an error event.
    pub fn emit_error(&mut self, code: &str, message: &str) -> Result<(), CliEmitterError> {
        #[allow(clippy::expect_used)]
        self.emit(
            json_error(code, message)
                .build()
                .expect("json_error: builder failed unexpectedly"),
        )
    }

    /// Convenience: build and emit a progress event.
    pub fn emit_progress(&mut self, message: &str) -> Result<(), CliEmitterError> {
        #[allow(clippy::expect_used)]
        self.emit(
            json_progress(message)
                .build()
                .expect("json_progress: builder failed unexpectedly"),
        )
    }

    /// Convenience: build and emit a log event with default fields.
    ///
    /// Applies log_fields_provider if configured; explicit fields take precedence.
    pub fn emit_log(&mut self, level: LogLevel, message: &str) -> Result<(), CliEmitterError> {
        #[allow(clippy::expect_used)]
        let mut event = json_log(level, message)
            .build()
            .expect("json_log: builder failed unexpectedly")
            .into_value();
        if let Some(provider) = &self.log_fields_provider {
            let provider_fields = provider();
            if let Some(log_obj) = event.get_mut("log").and_then(|v| v.as_object_mut())
                && let Value::Object(fields) = provider_fields
            {
                for (k, v) in fields {
                    if !matches!(k.as_str(), "message" | "level" | "code") {
                        log_obj.entry(k).or_insert(v);
                    }
                }
            }
        }
        self.write_event(event)
    }

    /// Access the underlying writer.
    pub fn into_inner(self) -> W {
        self.writer
    }

    fn write_event(&mut self, event: Value) -> Result<(), CliEmitterError> {
        validate_protocol_event(&event, self.strict_protocol)
            .map_err(CliEmitterError::Validation)?;
        let kind = event
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| CliEmitterError::Validation("event.kind is required".to_string()))?;
        match kind {
            "log" | "progress" => {
                if self.terminal_emitted {
                    return Err(CliEmitterError::Lifecycle(
                        "cannot emit non-terminal event after terminal event".to_string(),
                    ));
                }
            }
            "result" | "error" => {
                if self.terminal_emitted {
                    return Err(CliEmitterError::Lifecycle(
                        "cannot emit duplicate terminal event".to_string(),
                    ));
                }
            }
            _ => {
                return Err(CliEmitterError::Validation(format!(
                    "unsupported event kind {kind:?}"
                )));
            }
        }
        let rendered = cli_output_with_options(&event, self.format, &self.output_options);
        self.writer.write_all(rendered.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        if matches!(kind, "result" | "error") {
            self.terminal_emitted = true;
        }
        Ok(())
    }
}

/// Build a standard CLI version event.
#[allow(clippy::expect_used)]
pub fn build_cli_version(version: &str) -> Event {
    json_result(serde_json::json!({ "version": version }))
        .build()
        .expect("build_cli_version: builder failed unexpectedly")
}

fn build_cli_version_with_mode(version: &str, mode: CliProtocolMode) -> Event {
    match mode {
        CliProtocolMode::Legacy => build_cli_version(version),
        CliProtocolMode::ProtocolV1 => {
            let payload = serde_json::json!({ "code": "version", "version": version });
            #[allow(clippy::expect_used)]
            json_result(payload)
                .trace(serde_json::json!({}))
                .build()
                .expect("build_cli_version_with_mode: builder failed unexpectedly")
        }
    }
}

/// Render a CLI version response.
///
/// Pass `Some(format)` for an AFDATA event in JSON/YAML/plain. Pass `None` to
/// preserve conventional `<name> <version>` output.
pub fn cli_render_version(name: &str, version: &str, format: Option<OutputFormat>) -> String {
    let mut rendered = match format {
        Some(format) => cli_output(build_cli_version(version).as_value(), format),
        None => format!("{name} {version}"),
    };
    while rendered.ends_with('\n') {
        rendered.pop();
    }
    rendered.push('\n');
    rendered
}

/// Render version output from raw argv if `--version` or `-V` is present.
///
/// `raw_args` should be the full argv vector, including argv[0], as produced by
/// `std::env::args()`. The helper intentionally runs before clap or another
/// parser so explicit `--output json|yaml|plain` is honored instead of being
/// bypassed by built-in version handling.
///
/// Returns a standard [`build_cli_error`] event when the version request is
/// malformed, for example `--version --output xml`.
pub fn cli_handle_version_or_continue(
    raw_args: &[String],
    name: &str,
    version: &str,
    config: &VersionConfig,
) -> Result<Option<String>, Event> {
    let parsed = parse_version_request(raw_args, config);
    if !parsed.version_requested {
        return Ok(None);
    }
    if let Some(error) = parsed.output_error {
        #[allow(clippy::expect_used)]
        let event = json_error("cli_error", &error)
            .hint_if_some(Some("valid version output formats: json, yaml, plain"))
            .build()
            .expect("cli_handle_version_or_continue: builder failed");
        return Err(event);
    }
    let format = if config.allow_output_format {
        parsed.output_format.or(config.default_output)
    } else {
        config.default_output
    };
    if config.protocol_mode == CliProtocolMode::Legacy {
        return Ok(Some(cli_render_version(name, version, format)));
    }
    let Some(format) = format else {
        return Ok(Some(cli_render_version(name, version, None)));
    };
    let mut rendered = cli_output(
        build_cli_version_with_mode(version, config.protocol_mode).as_value(),
        format,
    );
    while rendered.ends_with('\n') {
        rendered.pop();
    }
    rendered.push('\n');
    Ok(Some(rendered))
}

struct ParsedVersionRequest {
    version_requested: bool,
    output_format: Option<OutputFormat>,
    output_error: Option<String>,
}

fn parse_version_request(raw_args: &[String], config: &VersionConfig) -> ParsedVersionRequest {
    let args = raw_args.get(1..).unwrap_or(&[]);
    let mut version_requested = false;
    let mut output_format = None;
    let mut output_error = None;
    let output_flag = config.output_flag.map(normalize_long_flag);

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            break;
        }

        let (flag_name, inline_value) = split_flag(arg);
        if matches!(arg, "--version" | "-V") {
            version_requested = true;
            i += 1;
            continue;
        }

        if config.allow_output_format && arg == "--json" {
            set_version_output_format(
                &mut output_format,
                OutputFormat::Json,
                "--json",
                &mut output_error,
            );
            i += 1;
            continue;
        }

        if config.allow_output_format
            && version_output_flag_matches(flag_name, output_flag, config.output_short)
        {
            let value = inline_value.or_else(|| {
                args.get(i + 1)
                    .map(String::as_str)
                    .filter(|next| !next.starts_with('-'))
            });
            if let Some(value) = value {
                match cli_parse_output(value) {
                    Ok(format) => set_version_output_format(
                        &mut output_format,
                        format,
                        &format!("--{} {value}", output_flag.unwrap_or("output")),
                        &mut output_error,
                    ),
                    Err(err) => output_error = Some(err),
                }
            } else {
                output_error = Some(format!(
                    "missing value for --{}: expected json, yaml, or plain",
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
        i += 1;
    }

    ParsedVersionRequest {
        version_requested,
        output_format,
        output_error,
    }
}

fn set_version_output_format(
    current: &mut Option<OutputFormat>,
    next: OutputFormat,
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

fn version_output_flag_matches(
    flag_name: Option<&str>,
    output_flag: Option<&str>,
    output_short: Option<char>,
) -> bool {
    let Some(seen) = flag_name else {
        return false;
    };
    output_flag.is_some_and(|expected| seen == expected)
        || output_short.is_some_and(|short| {
            let mut chars = seen.chars();
            chars.next().is_some_and(|seen_short| seen_short == short) && chars.next().is_none()
        })
}

fn normalize_long_flag(flag: &str) -> &str {
    flag.strip_prefix("--").unwrap_or(flag)
}

fn split_flag(arg: &str) -> (Option<&str>, Option<&str>) {
    if !arg.starts_with('-') || arg == "-" {
        return (None, None);
    }
    let (flag, value) = arg.split_once('=').unwrap_or((arg, ""));
    let name = flag.trim_start_matches('-');
    if name.is_empty() {
        (None, None)
    } else if arg.contains('=') {
        (Some(name), Some(value))
    } else {
        (Some(name), None)
    }
}
