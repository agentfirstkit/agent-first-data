use serde::Serialize;
use serde_json::Value;
use std::fmt;
use std::ops::Deref;

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

impl Deref for Event {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.0
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
/// - A reserved field is overwritten (code, message, hint, retryable for error; message for progress/log; code is deleted from log vocabulary)
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
    fn as_str(&self) -> &'static str {
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

    /// Build the event.
    pub fn build(self) -> Result<Event, BuildError> {
        let trace = self
            .trace
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let mut obj = serde_json::Map::new();
        obj.insert("kind".to_string(), Value::String("result".to_string()));
        obj.insert("result".to_string(), self.payload);
        obj.insert("trace".to_string(), trace);
        Ok(Event(Value::Object(obj)))
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
/// Panics if `code` or `message` is empty (required by protocol contract).
#[allow(clippy::panic)]
pub fn json_error(code: &str, message: &str) -> ErrorBuilder {
    if code.is_empty() {
        panic!("json_error: code must not be empty");
    }
    if message.is_empty() {
        panic!("json_error: message must not be empty");
    }
    ErrorBuilder {
        code: code.to_string(),
        message: message.to_string(),
        retryable: false,
        hint: None,
        fields: serde_json::Map::new(),
        trace: None,
        build_error: None,
    }
}

// ═══════════════════════════════════════════
// Progress Builder
// ═══════════════════════════════════════════

/// Builder for progress events.
pub struct ProgressBuilder {
    message: String,
    fields: serde_json::Map<String, Value>,
    trace: Option<Value>,
    build_error: Option<BuildError>,
}

impl ProgressBuilder {
    /// Add an extension field.
    pub fn field(mut self, name: &str, value: Value) -> Self {
        if self.build_error.is_none() {
            if name == "message" {
                self.build_error = Some(BuildError::ReservedField(
                    "cannot write reserved field \"message\" to progress payload".to_string(),
                ));
            } else {
                self.fields.insert(name.to_string(), value);
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
                        if k == "message" {
                            self.build_error = Some(BuildError::ReservedField(
                                "cannot write reserved field \"message\" to progress payload"
                                    .to_string(),
                            ));
                            return self;
                        }
                        self.fields.insert(k, v);
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
                        if k == "message" {
                            self.build_error = Some(BuildError::ReservedField(
                                "cannot write reserved field \"message\" to progress payload"
                                    .to_string(),
                            ));
                            return self;
                        }
                        self.fields.insert(k, v);
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

        let mut progress_obj = self.fields;
        progress_obj.insert("message".to_string(), Value::String(self.message));

        let trace = self
            .trace
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let mut obj = serde_json::Map::new();
        obj.insert("kind".to_string(), Value::String("progress".to_string()));
        obj.insert("progress".to_string(), Value::Object(progress_obj));
        obj.insert("trace".to_string(), trace);

        Ok(Event(Value::Object(obj)))
    }
}

/// Fluent builder: start building a progress event.
///
/// Panics if `message` is empty (required by protocol contract).
#[allow(clippy::panic)]
pub fn json_progress(message: &str) -> ProgressBuilder {
    if message.is_empty() {
        panic!("json_progress: message must not be empty");
    }
    ProgressBuilder {
        message: message.to_string(),
        fields: serde_json::Map::new(),
        trace: None,
        build_error: None,
    }
}

// ═══════════════════════════════════════════
// Log Builder
// ═══════════════════════════════════════════

/// Builder for log events.
pub struct LogBuilder {
    level: LogLevel,
    message: String,
    fields: serde_json::Map<String, Value>,
    trace: Option<Value>,
    build_error: Option<BuildError>,
}

impl LogBuilder {
    /// Add an extension field.
    pub fn field(mut self, name: &str, value: Value) -> Self {
        if self.build_error.is_none() {
            match name {
                "message" | "level" | "code" => {
                    self.build_error = Some(BuildError::ReservedField(format!(
                        "cannot write reserved field {name:?} to log payload"
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
                            "message" | "level" | "code" => {
                                self.build_error = Some(BuildError::ReservedField(format!(
                                    "cannot write reserved field {k:?} to log payload"
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
                            "message" | "level" | "code" => {
                                self.build_error = Some(BuildError::ReservedField(format!(
                                    "cannot write reserved field {k:?} to log payload"
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

        let mut log_obj = self.fields;
        log_obj.insert(
            "level".to_string(),
            Value::String(self.level.as_str().to_string()),
        );
        log_obj.insert("message".to_string(), Value::String(self.message));

        let trace = self
            .trace
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
        let mut obj = serde_json::Map::new();
        obj.insert("kind".to_string(), Value::String("log".to_string()));
        obj.insert("log".to_string(), Value::Object(log_obj));
        obj.insert("trace".to_string(), trace);

        Ok(Event(Value::Object(obj)))
    }
}

/// Fluent builder: start building a log event.
///
/// Panics if `message` is empty (required by protocol contract).
#[allow(clippy::panic)]
pub fn json_log(level: LogLevel, message: &str) -> LogBuilder {
    if message.is_empty() {
        panic!("json_log: message must not be empty");
    }
    LogBuilder {
        level,
        message: message.to_string(),
        fields: serde_json::Map::new(),
        trace: None,
        build_error: None,
    }
}

// ═══════════════════════════════════════════
// CLI Helper
// ═══════════════════════════════════════════

/// Build a CLI error event with optional hint.
///
/// Equivalent to: `json_error("cli_error", message).hint_if_some(hint).build()`
///
/// Always returns an event with:
/// - code: "cli_error"
/// - retryable: false
/// - trace: {}
///
/// Panics if `message` is empty (required by protocol contract).
#[allow(clippy::panic, clippy::expect_used)]
pub fn build_cli_error(message: &str, hint: Option<&str>) -> Event {
    if message.is_empty() {
        panic!("build_cli_error: message must not be empty");
    }
    json_error("cli_error", message)
        .hint_if_some(hint)
        .build()
        .expect("build_cli_error: builder returned error unexpectedly")
}

// ═══════════════════════════════════════════
// Validation
// ═══════════════════════════════════════════

/// Validate one protocol v1 event envelope.
///
/// `strict` additionally enforces the recommended strict profile: `trace` is
/// required, and kind-specific payload shapes are checked (see
/// [`validate_protocol_event_strict_payload`]).
pub fn validate_protocol_event(event: &Value, strict: bool) -> Result<(), String> {
    validate_protocol_event_base(event)?;
    if strict {
        validate_protocol_event_strict_payload(event)?;
    }
    Ok(())
}

fn validate_protocol_event_base(event: &Value) -> Result<(), String> {
    let Some(obj) = event.as_object() else {
        return Err("event must be a JSON object".to_string());
    };
    let Some(kind) = obj.get("kind").and_then(Value::as_str) else {
        return Err("event.kind must be one of result, error, progress, log".to_string());
    };
    if !matches!(kind, "result" | "error" | "progress" | "log") {
        return Err(format!("unsupported event kind {kind:?}"));
    }
    if !obj.contains_key(kind) {
        return Err(format!("event payload field {kind:?} is required"));
    }
    for key in obj.keys() {
        if key != "kind" && key != kind && key != "trace" {
            return Err(format!("unexpected top-level field {key:?}"));
        }
    }
    if let Some(trace) = obj.get("trace")
        && !trace.is_object()
    {
        return Err("event.trace must be a JSON object when present".to_string());
    }
    if kind == "error" {
        validate_error_payload(obj.get("error"))?;
    }
    Ok(())
}

fn validate_error_payload(error: Option<&Value>) -> Result<(), String> {
    let Some(error) = error.and_then(Value::as_object) else {
        return Err("event.error must be a JSON object".to_string());
    };
    match error.get("code").and_then(Value::as_str) {
        Some(code) if !code.is_empty() => {}
        _ => return Err("event.error.code must be a non-empty string".to_string()),
    }
    match error.get("message").and_then(Value::as_str) {
        Some(message) if !message.is_empty() => {}
        _ => return Err("event.error.message must be a non-empty string".to_string()),
    }
    if error.get("hint").is_some_and(|hint| !hint.is_string()) {
        return Err("event.error.hint must be a string when present".to_string());
    }
    Ok(())
}

/// Validate a finite structured CLI event stream:
/// `(log | progress)* -> exactly one (result | error) -> end`.
///
/// `strict` is forwarded to [`validate_protocol_event`] for every event.
pub fn validate_protocol_stream(events: &[Value], strict: bool) -> Result<(), String> {
    let mut terminal_kind: Option<&str> = None;
    for (idx, event) in events.iter().enumerate() {
        validate_protocol_event(event, strict).map_err(|err| format!("event {idx}: {err}"))?;
        let Some(kind) = event.get("kind").and_then(Value::as_str) else {
            return Err(format!("event {idx}: missing kind"));
        };
        match kind {
            "log" | "progress" => {
                if terminal_kind.is_some() {
                    return Err(format!("event {idx}: non-terminal event after terminal"));
                }
            }
            "result" | "error" => {
                if terminal_kind.is_some() {
                    return Err(format!("event {idx}: duplicate terminal event"));
                }
                terminal_kind = Some(kind);
            }
            _ => return Err(format!("event {idx}: unsupported event kind {kind:?}")),
        }
    }
    if terminal_kind.is_none() {
        return Err("event stream must contain exactly one terminal result or error".to_string());
    }
    Ok(())
}

/// Validate one protocol v1 event's payload against the recommended strict profile.
///
/// Assumes the base envelope shape (see [`validate_protocol_event_base`]) already passed.
fn validate_protocol_event_strict_payload(event: &Value) -> Result<(), String> {
    if !event.get("trace").is_some_and(Value::is_object) {
        return Err("event.trace is required by the strict profile".to_string());
    }
    match event.get("kind").and_then(Value::as_str) {
        Some("error") => validate_strict_error_payload(event.get("error")),
        Some("log") => validate_strict_log_payload(event.get("log")),
        Some("progress") => validate_strict_progress_payload(event.get("progress")),
        _ => Ok(()),
    }
}

fn validate_strict_error_payload(error: Option<&Value>) -> Result<(), String> {
    let Some(error) = error.and_then(Value::as_object) else {
        return Err("event.error must be a JSON object in the strict profile".to_string());
    };
    require_non_empty_string(error, "code", "event.error")?;
    require_non_empty_string(error, "message", "event.error")?;
    if error.get("retryable").and_then(Value::as_bool).is_none() {
        return Err("event.error.retryable must be a boolean in the strict profile".to_string());
    }
    if error.contains_key("hint") && !error.get("hint").is_some_and(Value::is_string) {
        return Err("event.error.hint must be a string when present".to_string());
    }
    Ok(())
}

fn validate_strict_log_payload(log: Option<&Value>) -> Result<(), String> {
    let Some(log) = log.and_then(Value::as_object) else {
        return Err("event.log must be a JSON object in the strict profile".to_string());
    };

    // 0.16 change: log.code is deleted from protocol vocabulary, must not be present
    if log.contains_key("code") {
        return Err(
            "event.log.code must not be present in the strict profile (deleted in 0.16)"
                .to_string(),
        );
    }

    require_non_empty_string(log, "message", "event.log")?;
    let valid_level = log
        .get("level")
        .and_then(Value::as_str)
        .is_some_and(|level| matches!(level, "debug" | "info" | "warn" | "error"));
    if !valid_level {
        return Err(
            "event.log.level must be one of debug, info, warn, error in the strict profile"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_strict_progress_payload(progress: Option<&Value>) -> Result<(), String> {
    let Some(progress) = progress.and_then(Value::as_object) else {
        return Err("event.progress must be a JSON object in the strict profile".to_string());
    };
    require_non_empty_string(progress, "message", "event.progress")
}

fn require_non_empty_string(
    payload: &serde_json::Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<(), String> {
    if payload
        .get(field)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
    {
        return Ok(());
    }
    Err(format!(
        "{path}.{field} must be a non-empty string in the strict profile"
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
    pub message: String,
    /// Payload keys beyond `message`.
    pub fields: serde_json::Map<String, Value>,
    /// The raw `trace` object, when present.
    pub trace: Option<Value>,
}

/// Decoded `kind:"log"` event.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedLog {
    pub level: LogLevel,
    pub message: String,
    /// Payload keys beyond `level`, `message`.
    pub fields: serde_json::Map<String, Value>,
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
    validate_protocol_event(&value, true).map_err(EventDecodeError::InvalidEvent)?;

    // Strict validation above guarantees the envelope is an object with a
    // recognized `kind` and a matching, object-shaped payload for
    // error/progress/log; the `ok_or_else` fallbacks below are defensive only.
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
        Some("progress") => {
            let mut fields = obj
                .get("progress")
                .and_then(Value::as_object)
                .ok_or_else(malformed)?
                .clone();
            let message = fields
                .remove("message")
                .and_then(|v| v.as_str().map(str::to_string))
                .ok_or_else(malformed)?;
            Ok(DecodedEvent::Progress(DecodedProgress {
                message,
                fields,
                trace,
            }))
        }
        Some("log") => {
            let mut fields = obj
                .get("log")
                .and_then(Value::as_object)
                .ok_or_else(malformed)?
                .clone();
            let message = fields
                .remove("message")
                .and_then(|v| v.as_str().map(str::to_string))
                .ok_or_else(malformed)?;
            let level = match fields.remove("level").as_ref().and_then(Value::as_str) {
                Some("debug") => LogLevel::Debug,
                Some("info") => LogLevel::Info,
                Some("warn") => LogLevel::Warn,
                Some("error") => LogLevel::Error,
                _ => return Err(malformed()),
            };
            Ok(DecodedEvent::Log(DecodedLog {
                level,
                message,
                fields,
                trace,
            }))
        }
        _ => Err(malformed()),
    }
}
