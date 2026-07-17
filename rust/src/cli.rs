use crate::protocol::{
    BuildError, Event, LogLevel, ProtocolViolation, build_cli_error, json_error, json_log,
    json_progress, json_result, validate_protocol_event,
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
    /// Structure-preserving YAML (same semantics as [`OutputFormat::Json`]).
    Yaml,
    Plain,
}

/// Parsed and normalized log filters (trimmed, lowercased, deduplicated).
///
/// Semantics (a stable contract):
/// - An **empty** set emits no logs (filtering is opt-in, not opt-out).
/// - The single wildcard word `"all"` emits every log. (`"*"` is not special —
///   there is one wildcard spelling, not two.)
/// - Otherwise a log is emitted iff its lowercased event name **starts with**
///   any filter string (prefix match).
///
/// Consequence to know: a mistyped filter simply matches nothing, so it
/// silently emits no output — that is the documented behavior, not a bug.
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
    /// Returns `false` if empty (no logs). Returns `true` if the set contains
    /// the wildcard word `"all"`. Otherwise returns `true` iff the lowercased
    /// event name starts with any filter (prefix match).
    pub fn enabled(&self, event: &str) -> bool {
        if self.0.is_empty() {
            return false;
        }
        let event_lower = event.to_ascii_lowercase();
        if self.0.contains(&"all".to_string()) {
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

/// Error returned by [`CliEmitter`].
#[derive(Debug)]
pub enum CliEmitterError {
    /// A protocol-validation failure.
    Validation(ProtocolViolation),
    /// An event builder rejected its inputs (empty code/message, reserved field).
    Build(BuildError),
    /// An emitter lifecycle rule was violated (terminal ordering).
    Lifecycle(String),
    /// Writing the event to the underlying writer failed.
    Write(std::io::Error),
}

impl std::fmt::Display for CliEmitterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(v) => write!(f, "{v}"),
            Self::Build(e) => write!(f, "{e}"),
            Self::Lifecycle(err) => f.write_str(err),
            Self::Write(err) => write!(f, "failed to write CLI event: {err}"),
        }
    }
}

impl CliEmitterError {
    /// Return the underlying writer error, when event emission failed during I/O.
    pub const fn io_error(&self) -> Option<&std::io::Error> {
        match self {
            Self::Write(err) => Some(err),
            Self::Validation(_) | Self::Build(_) | Self::Lifecycle(_) => None,
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
        self.emit(json_result(payload).build())
    }

    /// Convenience: build and emit an error event.
    pub fn emit_error(&mut self, code: &str, message: &str) -> Result<(), CliEmitterError> {
        match json_error(code, message).build() {
            Ok(event) => self.emit(event),
            Err(err) => Err(CliEmitterError::Build(err)),
        }
    }

    /// Convenience: build and emit a progress event.
    pub fn emit_progress(&mut self, message: &str) -> Result<(), CliEmitterError> {
        self.emit(json_progress(serde_json::json!({ "message": message })).build())
    }

    /// Convenience: build and emit a log event with default fields.
    ///
    /// Applies log_fields_provider if configured; explicit fields take precedence.
    pub fn emit_log(&mut self, level: LogLevel, message: &str) -> Result<(), CliEmitterError> {
        let mut event = json_log(serde_json::json!({
            "level": level.as_str(),
            "message": message,
        }))
        .build()
        .into_value();
        if let Some(provider) = &self.log_fields_provider {
            let provider_fields = provider();
            if let Some(log_obj) = event.get_mut("log").and_then(|v| v.as_object_mut())
                && let Value::Object(fields) = provider_fields
            {
                for (k, v) in fields {
                    log_obj.entry(k).or_insert(v);
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
        let kind = event.get("kind").and_then(Value::as_str).ok_or_else(|| {
            CliEmitterError::Validation(ProtocolViolation {
                rule: "kind_invalid",
                pointer: "/kind".to_string(),
                message: "event.kind is required".to_string(),
            })
        })?;
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
                return Err(CliEmitterError::Validation(ProtocolViolation {
                    rule: "kind_unsupported",
                    pointer: "/kind".to_string(),
                    message: format!("unsupported event kind {kind:?}"),
                }));
            }
        }
        let rendered = crate::formatting::render(&event, self.format, &self.output_options);
        self.writer.write_all(rendered.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        if matches!(kind, "result" | "error") {
            self.terminal_emitted = true;
        }
        Ok(())
    }
}

/// Build a standard CLI version event: a `kind:"result"` event whose payload
/// is `{ "code": "version", "version": <version> }`.
pub fn build_cli_version(version: &str) -> Event {
    json_result(serde_json::json!({ "code": "version", "version": version })).build()
}

/// Render a CLI version response.
///
/// Pass `Some(format)` for an AFDATA event in JSON/YAML/plain. Pass `None` to
/// preserve conventional `<name> <version>` output.
pub fn cli_render_version(name: &str, version: &str, format: Option<OutputFormat>) -> String {
    let mut rendered = match format {
        Some(format) => crate::formatting::render(
            build_cli_version(version).as_value(),
            format,
            &OutputOptions::default(),
        ),
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
/// Only a *top-level* version request is recognized: scanning stops at the first
/// positional argument (the subcommand), so `tool sub --version <value>` leaves
/// `--version` for the subcommand's parser rather than printing the tool version.
///
/// The one blessed behavior: a bare `--version` prints conventional
/// `<name> <version>` text; an explicit `--output json|yaml|plain` (or `--json`)
/// prints a protocol-v1 `kind:"result"` version event (payload
/// `{ "code": "version", "version": ... }`). Returns a standard
/// [`build_cli_error`] event when the request is malformed, for example
/// `--version --output xml`.
pub fn cli_handle_version_or_continue(
    raw_args: &[String],
    name: &str,
    version: &str,
) -> Result<Option<String>, Event> {
    let parsed = parse_version_request(raw_args);
    if !parsed.version_requested {
        return Ok(None);
    }
    if let Some(error) = parsed.output_error {
        let event = build_cli_error(
            &error,
            Some("valid version output formats: json, yaml, plain"),
        );
        return Err(event);
    }
    Ok(Some(cli_render_version(
        name,
        version,
        parsed.output_format,
    )))
}

struct ParsedVersionRequest {
    version_requested: bool,
    output_format: Option<OutputFormat>,
    output_error: Option<String>,
}

fn parse_version_request(raw_args: &[String]) -> ParsedVersionRequest {
    let args = raw_args.get(1..).unwrap_or(&[]);
    let mut version_requested = false;
    let mut output_format = None;
    let mut output_error = None;

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            break;
        }
        // The first positional argument marks the subcommand boundary. Past it,
        // `--version` (and `-V`) belong to the subcommand's own parser, matching
        // git/cargo/clap: the pre-parser only owns a top-level version request.
        if !arg.starts_with('-') {
            break;
        }

        let (flag_name, inline_value) = split_flag(arg);
        if matches!(arg, "--version" | "-V") {
            version_requested = true;
            i += 1;
            continue;
        }

        if arg == "--json" {
            set_version_output_format(
                &mut output_format,
                OutputFormat::Json,
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
                match cli_parse_output(value) {
                    Ok(format) => set_version_output_format(
                        &mut output_format,
                        format,
                        &format!("--output {value}"),
                        &mut output_error,
                    ),
                    Err(err) => output_error = Some(err),
                }
            } else {
                output_error =
                    Some("missing value for --output: expected json, yaml, or plain".to_string());
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
