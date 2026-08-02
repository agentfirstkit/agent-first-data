use crate::formatting::OutputFormat;
use crate::protocol::{
    BuildError, Event, LogLevel, ProtocolViolation, json_error, json_log, json_progress,
    json_result, validate_protocol_event,
};
use crate::redaction::OutputOptions;
use serde_json::Value;

// ═══════════════════════════════════════════
// Public API: CLI Helpers
// ═══════════════════════════════════════════

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
/// Accepts pre-split entries produced by a caller's comma-list parser.
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

impl From<BuildError> for CliEmitterError {
    fn from(err: BuildError) -> Self {
        Self::Build(err)
    }
}

/// Where a [`CliEmitter`] sends its events, selected by `--output-to`.
///
/// The stream an event lands on follows the program's *consumption mode*, not
/// the event's shape (see the spec's CLI Event Framing):
///
/// - [`OutputTo::Split`] (the default) is finite one-shot mode: `result` goes
///   to `stdout`, while `error`/`progress`/`log` go to `stderr`. `stdout`
///   therefore carries only successful payloads, so a shell capture or pipe
///   never mistakes a failure for data.
/// - [`OutputTo::Stdout`] / [`OutputTo::Stderr`] are event-stream mode: every
///   event, including `error`, is collapsed onto that one stream so a consumer
///   reading it in order (`kind`-branching) sees preserved ordering.
///
/// A command is an event stream when it produces more than one caller-needed
/// output over time — a chunked payload, or an address the caller must act on
/// before the command can report its outcome. Such a command defaults to
/// [`OutputTo::Stdout`] and rejects an explicit `split`, because splitting it
/// would strand the caller's data on the diagnostic stream. If a
/// `kind:"progress"` event carries a payload the caller must read, the command
/// is an event stream that has not declared itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputTo {
    /// Finite one-shot: `result` → stdout, `error`/`progress`/`log` → stderr.
    Split,
    /// Event stream: every event onto stdout.
    Stdout,
    /// Event stream: every event onto stderr.
    Stderr,
}

impl OutputTo {
    /// Parse an `--output-to` value: `split` (default), `stdout`, or `stderr`.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "split" => Ok(Self::Split),
            "stdout" => Ok(Self::Stdout),
            "stderr" => Ok(Self::Stderr),
            other => Err(format!(
                "unsupported --output-to `{other}`; expected split, stdout, or stderr"
            )),
        }
    }
}

/// Stateful emitter for structured CLI executions.
///
/// The output format, redaction policy, and stream routing are fixed when the
/// emitter is created. Emitting after a terminal event, emitting a repeated
/// terminal event, and writer failures all return explicit errors.
///
/// Routing follows the consumption mode ([`OutputTo`]):
///
/// - [`CliEmitter::finite`] / [`CliEmitter::finite_with`] — finite one-shot:
///   `result` → the primary writer (stdout), `error`/`progress`/`log` → the
///   diagnostic writer (stderr). This is the recommended default for a
///   one-shot CLI, so shell capture and pipelines never treat a failure as data.
/// - [`CliEmitter::stream`] — event stream: every event, including `error`,
///   goes to the single writer, preserving interleaved ordering.
/// - [`CliEmitter::from_output_to`] builds either shape from a parsed
///   [`OutputTo`] selector.
pub struct CliEmitter<W: std::io::Write> {
    writer: W,
    diagnostic: Option<Box<dyn std::io::Write>>,
    format: OutputFormat,
    output_options: OutputOptions,
    strict_protocol: bool,
    terminal_emitted: bool,
}

impl<W: std::io::Write> CliEmitter<W> {
    /// Create an event-stream emitter: every event goes to `writer`.
    ///
    /// Alias for [`CliEmitter::stream`]. Use [`CliEmitter::finite`] for a
    /// one-shot command that should split `result`/`error` across stdout/stderr.
    pub fn new(writer: W, format: OutputFormat) -> Self {
        Self::stream(writer, format)
    }

    /// Create an event-stream emitter with custom output options.
    pub fn with_options(writer: W, format: OutputFormat, output_options: OutputOptions) -> Self {
        Self {
            writer,
            diagnostic: None,
            format,
            output_options,
            strict_protocol: false,
            terminal_emitted: false,
        }
    }

    /// Create an event-stream emitter: every event, including `error`, goes to
    /// the single `writer`, preserving interleaved ordering. Pick this when the
    /// consumer reads one ordered stream and branches on `kind`.
    pub fn stream(writer: W, format: OutputFormat) -> Self {
        Self::with_options(writer, format, OutputOptions::default())
    }

    /// Create a finite one-shot emitter with explicit sinks: `result` goes to
    /// `result_writer`, while `error`/`progress`/`log` go to `diagnostic`.
    pub fn finite_with(
        result_writer: W,
        diagnostic: impl std::io::Write + 'static,
        format: OutputFormat,
    ) -> Self {
        Self::finite_with_options(result_writer, diagnostic, format, OutputOptions::default())
    }

    /// Create a finite one-shot emitter with explicit sinks and output options.
    pub fn finite_with_options(
        result_writer: W,
        diagnostic: impl std::io::Write + 'static,
        format: OutputFormat,
        output_options: OutputOptions,
    ) -> Self {
        Self {
            writer: result_writer,
            diagnostic: Some(Box::new(diagnostic)),
            format,
            output_options,
            strict_protocol: false,
            terminal_emitted: false,
        }
    }

    /// Require the AFDATA recommended strict profile for every emitted event.
    pub fn with_strict_protocol(mut self) -> Self {
        self.strict_protocol = true;
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
        self.emit(json_error(code, message).build()?)
    }

    /// Convenience: build and emit a progress event.
    pub fn emit_progress(&mut self, message: &str) -> Result<(), CliEmitterError> {
        self.emit(json_progress(serde_json::json!({ "message": message })).build())
    }

    /// Convenience: build and emit a log event.
    pub fn emit_log(&mut self, level: LogLevel, message: &str) -> Result<(), CliEmitterError> {
        self.emit(
            json_log(serde_json::json!({
            "level": level.as_str(),
            "message": message,
            }))
            .build(),
        )
    }

    /// Emit `event` as the terminal event and resolve the outcome to a process
    /// exit code, so a one-shot CLI need not hand-roll the emit-then-exit dance.
    ///
    /// A successful write returns `success_code`; a broken pipe (the reader hung
    /// up) returns `0`; any other write or validation failure returns `4`. A
    /// library never calls `process::exit` itself — return this code from `main`
    /// (`std::process::ExitCode::from(code)`).
    pub fn finish(&mut self, event: Event, success_code: u8) -> u8 {
        match self.emit(event) {
            Ok(()) => success_code,
            Err(err) if err.io_error_kind() == Some(std::io::ErrorKind::BrokenPipe) => 0,
            Err(_) => 4,
        }
    }

    /// Convenience over [`CliEmitter::finish`]: emit a `result` payload and
    /// return `0` on success.
    ///
    /// For an error, build it with [`json_error`] (`.hint(…)`, `.retryable(…)`,
    /// `.field(…)` as needed) and pass the event to [`CliEmitter::finish`] with
    /// the desired exit code — the builder is the error type, so no separate
    /// error-emitting convenience is needed.
    pub fn finish_result(&mut self, payload: Value) -> u8 {
        self.finish(json_result(payload).build(), 0)
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
        // Finite mode (a diagnostic sink is present) splits by kind: `result`
        // stays on the primary writer (stdout), while `error`/`progress`/`log`
        // are diagnostics routed to the diagnostic writer (stderr). Event-stream
        // mode (no diagnostic sink) keeps every event on the single writer.
        match &mut self.diagnostic {
            Some(diagnostic) if kind != "result" => {
                write_event_line(diagnostic.as_mut(), &rendered)
            }
            _ => write_event_line(&mut self.writer, &rendered),
        }?;
        if matches!(kind, "result" | "error") {
            self.terminal_emitted = true;
        }
        Ok(())
    }
}

/// Write one rendered event line (payload plus trailing newline) and flush.
fn write_event_line(writer: &mut dyn std::io::Write, rendered: &str) -> std::io::Result<()> {
    writer.write_all(rendered.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

// The emitter's own diagnostic sink is the spec's sanctioned exception to the
// "no ad-hoc stderr" rule (Channel policy): a finite one-shot emitter routes
// `error`/`progress`/`log` to `std::io::stderr` on purpose, so these wired
// constructors are allowed to name it directly.
#[allow(clippy::disallowed_methods)]
impl CliEmitter<std::io::Stdout> {
    /// Create a finite one-shot emitter wired to the process streams: `result`
    /// → `stdout`, `error`/`progress`/`log` → `stderr`. The recommended default
    /// for a one-shot CLI.
    pub fn finite(format: OutputFormat) -> Self {
        Self::finite_with(std::io::stdout(), std::io::stderr(), format)
    }

    /// Create a finite one-shot emitter wired to the process streams, with
    /// custom output options.
    pub fn finite_options(format: OutputFormat, output_options: OutputOptions) -> Self {
        Self::finite_with_options(std::io::stdout(), std::io::stderr(), format, output_options)
    }
}

// Same sanctioned exception as above: `from_output_to` wires the process
// streams (`std::io::stderr` included) as the emitter's own sinks.
#[allow(clippy::disallowed_methods)]
impl CliEmitter<Box<dyn std::io::Write>> {
    /// Build an emitter from a parsed [`OutputTo`] selector, wired to the
    /// process streams: `Split` is finite mode (`result` → stdout, everything
    /// else → stderr); `Stdout`/`Stderr` are event-stream mode onto that stream.
    pub fn from_output_to(selector: OutputTo, format: OutputFormat) -> Self {
        Self::from_output_to_with(selector, format, OutputOptions::default())
    }

    /// As [`CliEmitter::from_output_to`], with custom output options.
    pub fn from_output_to_with(
        selector: OutputTo,
        format: OutputFormat,
        output_options: OutputOptions,
    ) -> Self {
        match selector {
            OutputTo::Split => Self::finite_with_options(
                Box::new(std::io::stdout()),
                std::io::stderr(),
                format,
                output_options,
            ),
            OutputTo::Stdout => {
                Self::with_options(Box::new(std::io::stdout()), format, output_options)
            }
            OutputTo::Stderr => {
                Self::with_options(Box::new(std::io::stderr()), format, output_options)
            }
        }
    }
}

/// Write a raw document to the process stream `selector` names.
///
/// Not everything a CLI emits is an event. `--docs` renders a Markdown
/// reference, `--output plain` help renders a human catalog, and `shell bash`
/// renders a sourceable script: the consumer is `>` or `source`, so wrapping
/// them in a protocol envelope would mean `tool --docs > cli.md` wrote JSON.
/// Every CLI compiled from a registry inherits those outcomes, so something
/// must write bytes to a process stream — and this is that one place. Without
/// it each CLI hand-rolls the same dispatch, and they drift: this crate's own
/// binary treats a broken pipe as success while other tools returned a failure
/// exit code for it.
///
/// A broken pipe is success. `tool --docs | head` closes the reader early, and
/// reading the first page of a document is not a failure to report.
///
/// `Split` writes to stdout: a document is the result, not a diagnostic.
// The spec's Channel policy sanctions this sink, exactly as it does the
// emitter's constructors above; `clippy.toml`'s `disallowed-methods` keeps
// stray writes elsewhere from bypassing this routing.
#[allow(clippy::disallowed_methods)]
pub fn write_raw(text: &str, selector: OutputTo) -> std::io::Result<()> {
    use std::io::Write;
    let written = match selector {
        OutputTo::Stderr => std::io::stderr().lock().write_all(text.as_bytes()),
        OutputTo::Split | OutputTo::Stdout => std::io::stdout().lock().write_all(text.as_bytes()),
    };
    forgive_broken_pipe(written)
}

/// A closed reader is not a failure to report.
///
/// Split out so the policy is reachable from a test: piping a document into
/// `head` is a normal way to read one, and the exit code must not say the tool
/// failed. Every hand-rolled copy of this write got to decide that separately,
/// and they did not agree.
fn forgive_broken_pipe(result: std::io::Result<()>) -> std::io::Result<()> {
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

/// Build a standard CLI version event: a `kind:"result"` event whose payload is
/// `{ "code": "version", "name": <name>, "version": <version> }`, plus
/// `"display_name"`/`"build"` when given. `name` is the short/bin identity
/// (e.g. `"afdata"`); `display_name` is an optional human-facing product name
/// (e.g. `"Agent-First Data"`); `build` is an opaque caller-supplied identifier (a git
/// commit SHA, for example) — its meaning is entirely up to the caller. Both
/// are `None` when unavailable, and simply absent from the payload.
pub fn build_cli_version(
    name: &str,
    display_name: Option<&str>,
    version: &str,
    build: Option<&str>,
) -> Event {
    let mut payload = serde_json::json!({
        "code": "version",
        "name": name,
        "version": version,
    });
    if let Some(display_name) = display_name {
        payload["display_name"] = Value::String(display_name.to_string());
    }
    if let Some(build) = build {
        payload["build"] = Value::String(build.to_string());
    }
    json_result(payload).build()
}

/// Render a CLI version response as a protocol-v1 event in `format`.
pub fn cli_render_version(
    name: &str,
    display_name: Option<&str>,
    version: &str,
    build: Option<&str>,
    format: OutputFormat,
) -> String {
    let mut rendered = crate::formatting::render(
        build_cli_version(name, display_name, version, build).as_value(),
        format,
        &OutputOptions::default(),
    );
    while rendered.ends_with('\n') {
        rendered.pop();
    }
    rendered.push('\n');
    rendered
}

#[cfg(test)]
mod raw_write_tests {
    use super::forgive_broken_pipe;
    use std::io::{Error, ErrorKind};

    #[test]
    fn a_closed_reader_is_success_and_every_other_failure_is_not() {
        assert!(forgive_broken_pipe(Ok(())).is_ok());
        assert!(forgive_broken_pipe(Err(Error::from(ErrorKind::BrokenPipe))).is_ok());
        let denied = forgive_broken_pipe(Err(Error::from(ErrorKind::PermissionDenied)));
        assert_eq!(
            denied.map_err(|error| error.kind()),
            Err(ErrorKind::PermissionDenied)
        );
    }
}
