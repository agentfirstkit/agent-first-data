use serde::Serialize;
use serde_json::Value;
use std::fmt;

// ═══════════════════════════════════════════
// Event Type and Build Errors (0.16 API)
// ═══════════════════════════════════════════

/// A typed, strict-valid AFDATA protocol v1 event.
///
/// Wraps the complete JSON envelope and provides access to the underlying value.
/// Events produced by builders are guaranteed to pass `validate_protocol_event(_, true)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Event(Value);

impl Event {
    /// Access the underlying JSON value.
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    /// Convert into the underlying JSON value.
    pub fn into_value(self) -> Value {
        self.0
    }
}

impl From<Event> for Value {
    fn from(event: Event) -> Self {
        event.0
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Error type for builder failures.
///
/// Errors occur when:
/// - A reserved error field is overwritten (code, message, hint, retryable)
/// - An object field (.fields or .extend) is not a JSON object
/// - A required field (code/message for convenience functions) is empty
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    ReservedField(String),
    NonObjectField(String),
    EmptyRequiredField(String),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservedField(msg) => write!(f, "reserved field: {msg}"),
            Self::NonObjectField(msg) => write!(f, "non-object field: {msg}"),
            Self::EmptyRequiredField(msg) => write!(f, "empty required field: {msg}"),
        }
    }
}

impl std::error::Error for BuildError {}

/// Log level enumeration (serialized as lowercase).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

// ═══════════════════════════════════════════
// Result Builder
// ═══════════════════════════════════════════

/// Builder for result events.
pub struct ResultBuilder {
    payload: Value,
    trace: Option<Value>,
}

impl ResultBuilder {
    /// Set the trace object.
    pub fn trace(mut self, trace: Value) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Build the event. This builder cannot fail.
    pub fn build(self) -> Event {
        let trace = self
            .trace
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let mut obj = serde_json::Map::new();
        obj.insert("kind".to_string(), Value::String("result".to_string()));
        obj.insert("result".to_string(), self.payload);
        obj.insert("trace".to_string(), trace);
        Event(Value::Object(obj))
    }
}

/// Fluent builder: start building a result event.
pub fn json_result(payload: Value) -> ResultBuilder {
    ResultBuilder {
        payload,
        trace: None,
    }
}

// ═══════════════════════════════════════════
// Error Builder
// ═══════════════════════════════════════════

/// Builder for error events.
pub struct ErrorBuilder {
    code: String,
    message: String,
    retryable: bool,
    hint: Option<String>,
    fields: serde_json::Map<String, Value>,
    trace: Option<Value>,
    build_error: Option<BuildError>,
}

impl ErrorBuilder {
    /// Mark the error as retryable.
    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    /// Set retryable based on a boolean condition.
    pub fn retryable_if(mut self, should_retry: bool) -> Self {
        self.retryable = should_retry;
        self
    }

    /// Set the hint.
    pub fn hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.to_string());
        self
    }

    /// Set the hint if present.
    pub fn hint_if_some(mut self, hint: Option<&str>) -> Self {
        if let Some(h) = hint {
            self.hint = Some(h.to_string());
        }
        self
    }

    /// Add an extension field.
    pub fn field(mut self, name: &str, value: Value) -> Self {
        if self.build_error.is_none() {
            match name {
                "code" | "message" | "hint" | "retryable" => {
                    self.build_error = Some(BuildError::ReservedField(format!(
                        "cannot write reserved field {name:?} to error payload"
                    )));
                }
                _ => {
                    self.fields.insert(name.to_string(), value);
                }
            }
        }
        self
    }

    /// Add multiple extension fields from a JSON object.
    pub fn fields(mut self, fields: Value) -> Self {
        if self.build_error.is_none() {
            match fields {
                Value::Object(map) => {
                    for (k, v) in map {
                        match k.as_str() {
                            "code" | "message" | "hint" | "retryable" => {
                                self.build_error = Some(BuildError::ReservedField(format!(
                                    "cannot write reserved field {k:?} to error payload"
                                )));
                                return self;
                            }
                            _ => {
                                self.fields.insert(k, v);
                            }
                        }
                    }
                }
                _ => {
                    self.build_error = Some(BuildError::NonObjectField(
                        "fields() argument must be a JSON object".to_string(),
                    ));
                }
            }
        }
        self
    }

    /// Extend with a serializable value (must serialize to JSON object).
    pub fn extend<T: Serialize>(mut self, value: T) -> Self {
        if self.build_error.is_none() {
            match serde_json::to_value(&value) {
                Ok(Value::Object(map)) => {
                    for (k, v) in map {
                        match k.as_str() {
                            "code" | "message" | "hint" | "retryable" => {
                                self.build_error = Some(BuildError::ReservedField(format!(
                                    "cannot write reserved field {k:?} to error payload"
                                )));
                                return self;
                            }
                            _ => {
                                self.fields.insert(k, v);
                            }
                        }
                    }
                }
                Ok(_) => {
                    self.build_error = Some(BuildError::NonObjectField(
                        "extend() argument must serialize to a JSON object".to_string(),
                    ));
                }
                Err(_) => {
                    self.build_error = Some(BuildError::NonObjectField(
                        "extend() argument serialization failed".to_string(),
                    ));
                }
            }
        }
        self
    }

    /// Set the trace object.
    pub fn trace(mut self, trace: Value) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Build the event.
    pub fn build(self) -> Result<Event, BuildError> {
        if let Some(err) = self.build_error {
            return Err(err);
        }

        let mut error_obj = self.fields;
        error_obj.insert("code".to_string(), Value::String(self.code));
        error_obj.insert("message".to_string(), Value::String(self.message));
        error_obj.insert("retryable".to_string(), Value::Bool(self.retryable));
        if let Some(h) = self.hint {
            error_obj.insert("hint".to_string(), Value::String(h));
        }

        let trace = self
            .trace
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let mut obj = serde_json::Map::new();
        obj.insert("kind".to_string(), Value::String("error".to_string()));
        obj.insert("error".to_string(), Value::Object(error_obj));
        obj.insert("trace".to_string(), trace);

        Ok(Event(Value::Object(obj)))
    }
}

/// Fluent builder: start building an error event.
///
/// An empty `code` or `message` does not panic here; it seeds a deferred
/// [`BuildError::EmptyRequiredField`] that is surfaced when [`ErrorBuilder::build`]
/// is called.
pub fn json_error(code: &str, message: &str) -> ErrorBuilder {
    let build_error = if code.is_empty() {
        Some(BuildError::EmptyRequiredField(
            "error code must not be empty".to_string(),
        ))
    } else if message.is_empty() {
        Some(BuildError::EmptyRequiredField(
            "error message must not be empty".to_string(),
        ))
    } else {
        None
    };
    ErrorBuilder {
        code: code.to_string(),
        message: message.to_string(),
        retryable: false,
        hint: None,
        fields: serde_json::Map::new(),
        trace: None,
        build_error,
    }
}

// ═══════════════════════════════════════════
// Progress Builder
// ═══════════════════════════════════════════

/// Builder for progress events.
pub struct ProgressBuilder {
    payload: Value,
    trace: Option<Value>,
}

impl ProgressBuilder {
    /// Set the trace object.
    pub fn trace(mut self, trace: Value) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Build the event. This builder cannot fail.
    pub fn build(self) -> Event {
        let trace = self
            .trace
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let mut obj = serde_json::Map::new();
        obj.insert("kind".to_string(), Value::String("progress".to_string()));
        obj.insert("progress".to_string(), self.payload);
        obj.insert("trace".to_string(), trace);

        Event(Value::Object(obj))
    }
}

/// Fluent builder: start building a progress event.
///
pub fn json_progress(payload: Value) -> ProgressBuilder {
    ProgressBuilder {
        payload,
        trace: None,
    }
}

// ═══════════════════════════════════════════
// Log Builder
// ═══════════════════════════════════════════

/// Builder for log events.
pub struct LogBuilder {
    payload: Value,
    trace: Option<Value>,
}

impl LogBuilder {
    /// Set the trace object.
    pub fn trace(mut self, trace: Value) -> Self {
        self.trace = Some(trace);
        self
    }

    /// Build the event. This builder cannot fail.
    pub fn build(self) -> Event {
        let trace = self
            .trace
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let mut obj = serde_json::Map::new();
        obj.insert("kind".to_string(), Value::String("log".to_string()));
        obj.insert("log".to_string(), self.payload);
        obj.insert("trace".to_string(), trace);

        Event(Value::Object(obj))
    }
}

/// Fluent builder: start building a log event.
///
pub fn json_log(payload: Value) -> LogBuilder {
    LogBuilder {
        payload,
        trace: None,
    }
}

// ═══════════════════════════════════════════
// CLI Helper
// ═══════════════════════════════════════════

/// Build a CLI error event with optional hint.
///
/// Always returns a strict-valid `kind:"error"` event with code `"cli_error"`,
/// `retryable: false`, and an empty `trace`. An empty `message` is replaced with
/// a generic placeholder so the returned event stays strict-valid without panicking.
pub fn build_cli_error(message: &str, hint: Option<&str>) -> Event {
    let message = if message.is_empty() {
        "unspecified error"
    } else {
        message
    };
    match json_error("cli_error", message).hint_if_some(hint).build() {
        Ok(event) => event,
        // Unreachable in practice (non-empty code+message, no reserved fields);
        // construct a minimal valid envelope directly rather than panic.
        Err(_) => {
            let mut error_obj = serde_json::Map::new();
            error_obj.insert("code".to_string(), Value::String("cli_error".to_string()));
            error_obj.insert(
                "message".to_string(),
                Value::String("unspecified error".to_string()),
            );
            error_obj.insert("retryable".to_string(), Value::Bool(false));
            let mut obj = serde_json::Map::new();
            obj.insert("kind".to_string(), Value::String("error".to_string()));
            obj.insert("error".to_string(), Value::Object(error_obj));
            obj.insert("trace".to_string(), Value::Object(serde_json::Map::new()));
            Event(Value::Object(obj))
        }
    }
}

// ═══════════════════════════════════════════
// Validation
// ═══════════════════════════════════════════

/// A single protocol-validation violation: a stable machine-readable `rule`
/// slug, a JSON pointer to the offending location (`""` = the whole event),
/// and a human-readable `message`.
///
/// The `rule` slugs are a stable contract (the CLI maps them 1:1 onto its
/// `validate` finding `rule_id`s). The full set:
/// `event_not_object`, `kind_invalid`, `kind_unsupported`, `payload_missing`,
/// `unexpected_field`, `trace_not_object`, `trace_required`, `error_not_object`,
/// `error_code_invalid`, `error_message_invalid`, `error_hint_invalid`,
/// `error_retryable_invalid`, `stream_non_terminal_after_terminal`,
/// `stream_duplicate_terminal`, `stream_missing_terminal`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolViolation {
    pub rule: &'static str,
    pub pointer: String,
    pub message: String,
}

impl std::fmt::Display for ProtocolViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ProtocolViolation {}

fn violation(
    rule: &'static str,
    pointer: impl Into<String>,
    message: impl Into<String>,
) -> ProtocolViolation {
    ProtocolViolation {
        rule,
        pointer: pointer.into(),
        message: message.into(),
    }
}

/// Validate one protocol v1 event envelope.
///
/// `strict` additionally enforces the recommended strict profile: `trace` is
/// required, and kind-specific payload shapes are checked (see
/// [`validate_protocol_event_strict_payload`]).
///
/// Returns the first [`ProtocolViolation`] found, or `Ok(())`.
pub fn validate_protocol_event(event: &Value, strict: bool) -> Result<(), ProtocolViolation> {
    validate_protocol_event_base(event)?;
    if strict {
        validate_protocol_event_strict_payload(event)?;
    }
    Ok(())
}

fn validate_protocol_event_base(event: &Value) -> Result<(), ProtocolViolation> {
    let Some(obj) = event.as_object() else {
        return Err(violation(
            "event_not_object",
            "",
            "event must be a JSON object",
        ));
    };
    let Some(kind) = obj.get("kind").and_then(Value::as_str) else {
        return Err(violation(
            "kind_invalid",
            "/kind",
            "event.kind must be one of result, error, progress, log",
        ));
    };
    if !matches!(kind, "result" | "error" | "progress" | "log") {
        return Err(violation(
            "kind_unsupported",
            "/kind",
            format!("unsupported event kind {kind:?}"),
        ));
    }
    if !obj.contains_key(kind) {
        return Err(violation(
            "payload_missing",
            format!("/{kind}"),
            format!("event payload field {kind:?} is required"),
        ));
    }
    for key in obj.keys() {
        if key != "kind" && key != kind && key != "trace" {
            return Err(violation(
                "unexpected_field",
                format!("/{key}"),
                format!("unexpected top-level field {key:?}"),
            ));
        }
    }
    if let Some(trace) = obj.get("trace")
        && !trace.is_object()
    {
        return Err(violation(
            "trace_not_object",
            "/trace",
            "event.trace must be a JSON object when present",
        ));
    }
    if kind == "error" {
        validate_error_payload(obj.get("error"))?;
    }
    Ok(())
}

fn validate_error_payload(error: Option<&Value>) -> Result<(), ProtocolViolation> {
    let Some(error) = error.and_then(Value::as_object) else {
        return Err(violation(
            "error_not_object",
            "/error",
            "event.error must be a JSON object",
        ));
    };
    match error.get("code").and_then(Value::as_str) {
        Some(code) if !code.is_empty() => {}
        _ => {
            return Err(violation(
                "error_code_invalid",
                "/error/code",
                "event.error.code must be a non-empty string",
            ));
        }
    }
    match error.get("message").and_then(Value::as_str) {
        Some(message) if !message.is_empty() => {}
        _ => {
            return Err(violation(
                "error_message_invalid",
                "/error/message",
                "event.error.message must be a non-empty string",
            ));
        }
    }
    if error.get("hint").is_some_and(|hint| !hint.is_string()) {
        return Err(violation(
            "error_hint_invalid",
            "/error/hint",
            "event.error.hint must be a string when present",
        ));
    }
    Ok(())
}

/// Validate a finite structured CLI event stream:
/// `(log | progress)* -> exactly one (result | error) -> end`.
///
/// `strict` is forwarded to [`validate_protocol_event`] for every event.
///
/// Unlike [`validate_protocol_event`], this collects every violation across
/// the whole stream instead of failing fast on the first one; per-event
/// violation pointers are prefixed with the event's index (`/{idx}{pointer}`).
pub fn validate_protocol_stream(
    events: &[Value],
    strict: bool,
) -> Result<(), Vec<ProtocolViolation>> {
    let mut violations = Vec::new();
    let mut terminal_seen = false;
    for (idx, event) in events.iter().enumerate() {
        if let Err(v) = validate_protocol_event(event, strict) {
            violations.push(ProtocolViolation {
                rule: v.rule,
                pointer: format!("/{idx}{}", v.pointer),
                message: v.message,
            });
        }
        match event.get("kind").and_then(Value::as_str) {
            Some("log") | Some("progress") => {
                if terminal_seen {
                    violations.push(violation(
                        "stream_non_terminal_after_terminal",
                        format!("/{idx}"),
                        "non-terminal event after terminal",
                    ));
                }
            }
            Some("result") | Some("error") => {
                if terminal_seen {
                    violations.push(violation(
                        "stream_duplicate_terminal",
                        format!("/{idx}"),
                        "duplicate terminal event",
                    ));
                } else {
                    terminal_seen = true;
                }
            }
            _ => {} // an invalid/absent kind is already recorded by validate_protocol_event above
        }
    }
    if !terminal_seen {
        violations.push(violation(
            "stream_missing_terminal",
            String::new(),
            "event stream must contain exactly one terminal result or error",
        ));
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Validate one protocol v1 event's payload against the recommended strict profile.
///
/// Assumes the base envelope shape (see [`validate_protocol_event_base`]) already passed.
fn validate_protocol_event_strict_payload(event: &Value) -> Result<(), ProtocolViolation> {
    if !event.get("trace").is_some_and(Value::is_object) {
        return Err(violation(
            "trace_required",
            "/trace",
            "event.trace is required by the strict profile",
        ));
    }
    match event.get("kind").and_then(Value::as_str) {
        Some("error") => validate_strict_error_payload(event.get("error")),
        _ => Ok(()),
    }
}

fn validate_strict_error_payload(error: Option<&Value>) -> Result<(), ProtocolViolation> {
    let Some(error) = error.and_then(Value::as_object) else {
        return Err(violation(
            "error_not_object",
            "/error",
            "event.error must be a JSON object in the strict profile",
        ));
    };
    require_non_empty_string(
        error,
        "code",
        "error_code_invalid",
        "/error/code",
        "event.error",
    )?;
    require_non_empty_string(
        error,
        "message",
        "error_message_invalid",
        "/error/message",
        "event.error",
    )?;
    if error.get("retryable").and_then(Value::as_bool).is_none() {
        return Err(violation(
            "error_retryable_invalid",
            "/error/retryable",
            "event.error.retryable must be a boolean in the strict profile",
        ));
    }
    if error.contains_key("hint") && !error.get("hint").is_some_and(Value::is_string) {
        return Err(violation(
            "error_hint_invalid",
            "/error/hint",
            "event.error.hint must be a string when present",
        ));
    }
    Ok(())
}

fn require_non_empty_string(
    payload: &serde_json::Map<String, Value>,
    field: &str,
    rule: &'static str,
    pointer: &'static str,
    path: &str,
) -> Result<(), ProtocolViolation> {
    if payload
        .get(field)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(());
    }
    Err(violation(
        rule,
        pointer,
        format!("{path}.{field} must be a non-empty string in the strict profile"),
    ))
}

// ═══════════════════════════════════════════
// Reader API: decode_protocol_event
// ═══════════════════════════════════════════

/// A decoded, strict-valid AFDATA protocol v1 event, typed by kind.
#[derive(Clone, Debug, PartialEq)]
pub enum DecodedEvent {
    Result(DecodedResult),
    Error(DecodedError),
    Progress(DecodedProgress),
    Log(DecodedLog),
}

/// Decoded `kind:"result"` event.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedResult {
    /// The raw `result` payload value.
    pub result: Value,
    /// The raw `trace` object, when present.
    pub trace: Option<Value>,
}

/// Decoded `kind:"error"` event.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub hint: Option<String>,
    /// Payload keys beyond `code`, `message`, `retryable`, `hint`.
    pub fields: serde_json::Map<String, Value>,
    /// The raw `trace` object, when present.
    pub trace: Option<Value>,
}

/// Decoded `kind:"progress"` event.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedProgress {
    /// The raw `progress` payload value.
    pub progress: Value,
    /// The raw `trace` object, when present.
    pub trace: Option<Value>,
}

/// Decoded `kind:"log"` event.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedLog {
    /// The raw `log` payload value.
    pub log: Value,
    /// The raw `trace` object, when present.
    pub trace: Option<Value>,
}

/// Error returned by [`decode_protocol_event`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventDecodeError {
    /// `text` is not valid JSON.
    InvalidJson(String),
    /// The parsed JSON value failed strict protocol validation.
    InvalidEvent(String),
}

impl fmt::Display for EventDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(err) => write!(f, "invalid JSON: {err}"),
            Self::InvalidEvent(err) => write!(f, "invalid protocol event: {err}"),
        }
    }
}

impl std::error::Error for EventDecodeError {}

/// Parse one protocol v1 line, strict-validate it, and return a typed decoded event.
///
/// `text` is a single JSON text value (one protocol line), not a JSONL stream.
pub fn decode_protocol_event(text: &str) -> Result<DecodedEvent, EventDecodeError> {
    let value: Value =
        serde_json::from_str(text).map_err(|err| EventDecodeError::InvalidJson(err.to_string()))?;
    validate_protocol_event(&value, true)
        .map_err(|v| EventDecodeError::InvalidEvent(v.to_string()))?;

    // Strict validation above guarantees the envelope is an object with a
    // recognized `kind`; the `ok_or_else` fallbacks below are defensive only.
    let malformed = || {
        EventDecodeError::InvalidEvent(
            "event passed strict validation but has an unexpected shape".to_string(),
        )
    };
    let obj = value.as_object().ok_or_else(malformed)?;
    let trace = obj.get("trace").cloned();
    match obj.get("kind").and_then(Value::as_str) {
        Some("result") => Ok(DecodedEvent::Result(DecodedResult {
            result: obj.get("result").cloned().unwrap_or(Value::Null),
            trace,
        })),
        Some("error") => {
            let mut fields = obj
                .get("error")
                .and_then(Value::as_object)
                .ok_or_else(malformed)?
                .clone();
            let code = fields
                .remove("code")
                .and_then(|v| v.as_str().map(str::to_string))
                .ok_or_else(malformed)?;
            let message = fields
                .remove("message")
                .and_then(|v| v.as_str().map(str::to_string))
                .ok_or_else(malformed)?;
            let retryable = fields
                .remove("retryable")
                .and_then(|v| v.as_bool())
                .ok_or_else(malformed)?;
            let hint = fields
                .remove("hint")
                .and_then(|v| v.as_str().map(str::to_string));
            Ok(DecodedEvent::Error(DecodedError {
                code,
                message,
                retryable,
                hint,
                fields,
                trace,
            }))
        }
        Some("progress") => Ok(DecodedEvent::Progress(DecodedProgress {
            progress: obj.get("progress").cloned().unwrap_or(Value::Null),
            trace,
        })),
        Some("log") => Ok(DecodedEvent::Log(DecodedLog {
            log: obj.get("log").cloned().unwrap_or(Value::Null),
            trace,
        })),
        _ => Err(malformed()),
    }
}
