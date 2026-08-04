//! AFDATA-compliant tracing layer.
//!
//! Outputs log events using agent-first-data's `render` function:
//! - JSON: single-line JSONL (secrets redacted, original keys)
//! - Plain: single-line logfmt (keys stripped, values formatted)
//! - YAML: multi-line, structure-preserving
//!   (original keys and values kept, secrets redacted)
//!
//! Span fields are flattened into every event line (e.g. `request_id`).
//! All other tracing features (macros, spans, EnvFilter) work unchanged.
//!
//! # Usage
//! ```ignore
//! use agent_first_data::{Redactor, afdata_tracing::{self, LogFormat}};
//! use tracing_subscriber::EnvFilter;
//!
//! afdata_tracing::try_init(EnvFilter::new("info"), LogFormat::Json, Redactor::new())?;
//! ```

use std::io::{self, Write};
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};
use tracing::field::{Field, Visit};
use tracing::span;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::TryInitError;

/// Output format for the AFDATA tracing layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Json,
    Plain,
    /// Structure-preserving YAML.
    Yaml,
}

impl LogFormat {
    /// Return the canonical config spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Plain => "plain",
            Self::Yaml => "yaml",
        }
    }
}

impl std::fmt::Display for LogFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for LogFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "json" => Ok(Self::Json),
            "plain" => Ok(Self::Plain),
            "yaml" => Ok(Self::Yaml),
            _ => Err("invalid log format: expected json, plain, or yaml".to_string()),
        }
    }
}

impl From<LogFormat> for crate::OutputFormat {
    fn from(value: LogFormat) -> Self {
        match value {
            LogFormat::Json => Self::Json,
            LogFormat::Plain => Self::Plain,
            LogFormat::Yaml => Self::Yaml,
        }
    }
}

trait LogSink: Send + Sync {
    fn write_line(&self, line: &str, metadata: Option<&tracing::Metadata<'_>>) -> io::Result<()>;
}

/// The one destination a layer and every handle it issued write through.
///
/// `with_writer` swaps what is inside the cell rather than handing out a new
/// one, so a `StructuredLogHandle` taken before configuration cannot keep
/// writing to the old stream — a split that nothing would report at compile or
/// run time.
struct SharedSink {
    inner: RwLock<Arc<dyn LogSink>>,
}

impl SharedSink {
    fn new(sink: Arc<dyn LogSink>) -> Self {
        Self {
            inner: RwLock::new(sink),
        }
    }

    fn replace(&self, sink: Arc<dyn LogSink>) {
        match self.inner.write() {
            Ok(mut current) => *current = sink,
            Err(poisoned) => *poisoned.into_inner() = sink,
        }
    }
}

impl LogSink for SharedSink {
    fn write_line(&self, line: &str, metadata: Option<&tracing::Metadata<'_>>) -> io::Result<()> {
        let sink = self
            .inner
            .read()
            .map_err(|_| io::Error::other("AFDATA log sink lock is poisoned"))?;
        sink.write_line(line, metadata)
    }
}

struct MakeWriterSink<W> {
    writer: Mutex<W>,
}

impl<W> LogSink for MakeWriterSink<W>
where
    W: Send + for<'writer> MakeWriter<'writer>,
    for<'writer> <W as MakeWriter<'writer>>::Writer: Write,
{
    fn write_line(&self, line: &str, metadata: Option<&tracing::Metadata<'_>>) -> io::Result<()> {
        let factory = self
            .writer
            .lock()
            .map_err(|_| io::Error::other("AFDATA log writer lock is poisoned"))?;
        let mut writer = match metadata {
            Some(metadata) => factory.make_writer_for(metadata),
            None => factory.make_writer(),
        };
        // One write, not two. The inner `Mutex` only serializes this layer's
        // own writes; a shared destination like `io::stderr` locks per call, so
        // splitting the line from its newline lets another writer on the same
        // stream — `CliEmitter`'s error events, under default split routing —
        // land between them. That reader then loses both events to one
        // malformed line, including the terminal one.
        let mut framed = String::with_capacity(line.len() + 1);
        framed.push_str(line);
        framed.push('\n');
        writer.write_all(framed.as_bytes())?;
        writer.flush()
    }
}

/// A tracing Layer that outputs AFDATA-compliant log lines.
pub struct AfdataLayer {
    format: LogFormat,
    redactor: crate::Redactor,
    sink: Arc<SharedSink>,
}

impl std::fmt::Debug for AfdataLayer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AfdataLayer")
            .field("format", &self.format)
            .field("redactor", &self.redactor)
            .finish_non_exhaustive()
    }
}

/// Direct nested-JSON logger sharing an [`AfdataLayer`]'s writer and ordering.
#[derive(Clone)]
pub struct StructuredLogHandle {
    format: LogFormat,
    redactor: crate::Redactor,
    sink: Arc<SharedSink>,
}

impl std::fmt::Debug for StructuredLogHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StructuredLogHandle")
            .field("format", &self.format)
            .field("redactor", &self.redactor)
            .finish_non_exhaustive()
    }
}

/// Try to initialize tracing with AFDATA output.
///
/// Returns `Err` if a global tracing subscriber is already initialized. This is
/// the convenience entry point for a global subscriber. Use
/// [`AfdataLayer::new`] directly when composing a subscriber, injecting a
/// writer, or retaining a [`StructuredLogHandle`].
///
/// # Arguments
/// * `filter` - tracing_subscriber::EnvFilter controlling which events are recorded
/// * `format` - LogFormat::Json, LogFormat::Plain, or LogFormat::Yaml
/// * `redactor` - Redactor with optional custom secret field names and policy
pub fn try_init(
    filter: tracing_subscriber::EnvFilter,
    format: LogFormat,
    redactor: crate::Redactor,
) -> Result<(), TryInitError> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    tracing_subscriber::registry()
        .with(filter)
        .with(AfdataLayer::new(format, redactor))
        .try_init()
}

impl AfdataLayer {
    /// Build a composable layer that writes to stderr.
    ///
    /// Diagnostic log events use stderr by default; [`AfdataLayer::with_writer`]
    /// replaces that sanctioned process sink.
    #[allow(clippy::disallowed_methods)]
    pub fn new(format: LogFormat, redactor: crate::Redactor) -> Self {
        Self {
            format,
            redactor,
            sink: make_shared_sink(io::stderr),
        }
    }

    /// Replace the writer factory used by this layer.
    ///
    /// The factory is serialized behind a shared lock, so ordinary tracing and
    /// [`StructuredLogHandle`] writes use the same sink in call order. Handles
    /// taken before this call follow the new writer too: they share one cell
    /// rather than a snapshot, so the two cannot end up addressing different
    /// streams depending on the order the layer was configured in.
    pub fn with_writer<W>(self, writer: W) -> Self
    where
        W: Send + for<'writer> MakeWriter<'writer> + 'static,
        for<'writer> <W as MakeWriter<'writer>>::Writer: Write,
    {
        self.sink.replace(make_sink(writer));
        self
    }

    /// Get a direct nested-JSON logger sharing this layer's sink.
    pub fn structured_log_handle(&self) -> StructuredLogHandle {
        StructuredLogHandle {
            format: self.format,
            redactor: self.redactor.clone(),
            sink: Arc::clone(&self.sink),
        }
    }

    fn output_options(&self) -> crate::OutputOptions {
        crate::OutputOptions {
            redaction: self.redactor.clone(),
            style: crate::PlainStyle::Readable,
        }
    }

    fn format_value(&self, value: &serde_json::Value) -> String {
        crate::render(value, self.format.into(), &self.output_options())
    }
}

fn make_sink<W>(writer: W) -> Arc<dyn LogSink>
where
    W: Send + for<'writer> MakeWriter<'writer> + 'static,
    for<'writer> <W as MakeWriter<'writer>>::Writer: Write,
{
    Arc::new(MakeWriterSink {
        writer: Mutex::new(writer),
    })
}

fn make_shared_sink<W>(writer: W) -> Arc<SharedSink>
where
    W: Send + for<'writer> MakeWriter<'writer> + 'static,
    for<'writer> <W as MakeWriter<'writer>>::Writer: Write,
{
    Arc::new(SharedSink::new(make_sink(writer)))
}

impl StructuredLogHandle {
    /// Emit a nested JSON log payload.
    ///
    /// Unlike tracing's scalar field visitor, objects and arrays stay
    /// structured. Redaction and formatting are identical to the layer.
    ///
    /// The line carries `level` and `timestamp_epoch_ms` like every line the
    /// layer writes, so one stream does not mix records that a level filter or
    /// a timestamp correlation can read with records it silently drops. Supply
    /// `level` in `payload` to override the default; the timestamp is always
    /// stamped here, as it is for a tracing event.
    ///
    /// Note that a writer built with a level filter (for example
    /// `io::stderr.with_max_level(..)`) cannot route these events: they have no
    /// tracing metadata to route on, so they always take the default writer.
    pub fn emit(&self, payload: serde_json::Value) -> io::Result<()> {
        let event = crate::json_log(stamp_log_metadata(payload)).build();
        let options = crate::OutputOptions {
            redaction: self.redactor.clone(),
            style: crate::PlainStyle::Readable,
        };
        let line = crate::render(event.as_value(), self.format.into(), &options);
        self.sink.write_line(&line, None)
    }
}

/// Stored in span extensions to carry structured fields.
struct SpanFields(Vec<(String, serde_json::Value)>);

impl<S> Layer<S> for AfdataLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let mut visitor = JsonVisitor::new();
        attrs.record(&mut visitor);

        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(visitor.fields));
        }
    }

    fn on_record(&self, id: &span::Id, values: &span::Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut visitor = JsonVisitor::new();
            values.record(&mut visitor);

            let mut extensions = span.extensions_mut();
            if let Some(existing) = extensions.get_mut::<SpanFields>() {
                existing.0.extend(visitor.fields);
            } else {
                extensions.insert(SpanFields(visitor.fields));
            }
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();

        // Collect fields from the event
        let mut visitor = JsonVisitor::new();
        event.record(&mut visitor);

        // Build output object with AFDATA field names.
        let mut map = serde_json::Map::with_capacity(4 + visitor.fields.len());

        let level = match *meta.level() {
            // AFDATA's log level set starts at debug. TRACE remains
            // distinguishable from info without inventing a fifth level.
            Level::TRACE | Level::DEBUG => crate::LogLevel::Debug,
            Level::INFO => crate::LogLevel::Info,
            Level::WARN => crate::LogLevel::Warn,
            Level::ERROR => crate::LogLevel::Error,
        };

        // "message" field from the tracing macro's format string
        let message = visitor
            .message
            .take()
            .unwrap_or_else(|| "(no message)".to_string());

        // Flatten span fields from root to leaf (child overrides parent on collision)
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                let extensions = span.extensions();
                if let Some(fields) = extensions.get::<SpanFields>() {
                    for (k, v) in &fields.0 {
                        if !is_reserved_log_field(k) {
                            map.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }

        // Append event-level structured fields. Logs no longer use top-level
        // protocol code; code may be a tool-defined field inside the log
        // payload.
        for (k, v) in visitor.fields {
            if !is_reserved_log_field(&k) {
                map.insert(k, v);
            }
        }

        // Adapter metadata wins over same-named span/event fields.
        map.insert(
            "timestamp_epoch_ms".into(),
            serde_json::Value::Number(chrono::Utc::now().timestamp_millis().into()),
        );
        map.insert(
            "level".to_string(),
            serde_json::Value::String(level.as_str().to_string()),
        );
        map.insert("message".to_string(), serde_json::Value::String(message));
        let builder = crate::json_log(serde_json::Value::Object(map));
        let value = builder.build();

        // Format using the library's own output functions.
        let line = self.format_value(value.as_value());

        let _ = self.sink.write_line(&line, Some(meta));
    }
}

fn is_reserved_log_field(field: &str) -> bool {
    matches!(field, "level" | "message" | "timestamp_epoch_ms")
}

/// Give a directly-emitted payload the same envelope fields the layer stamps on
/// every tracing event, so both kinds of record are readable the same way.
///
/// A non-object payload is left alone: it has nowhere to put the fields, and
/// wrapping it would change the shape the caller asked for.
fn stamp_log_metadata(payload: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(mut map) = payload else {
        return payload;
    };
    map.entry("level".to_string())
        .or_insert_with(|| serde_json::Value::String("info".to_string()));
    map.insert(
        "timestamp_epoch_ms".to_string(),
        serde_json::Value::Number(chrono::Utc::now().timestamp_millis().into()),
    );
    serde_json::Value::Object(map)
}

/// Visitor that collects tracing event fields into a JSON map.
struct JsonVisitor {
    message: Option<String>,
    fields: Vec<(String, serde_json::Value)>,
}

impl JsonVisitor {
    fn new() -> Self {
        Self {
            message: None,
            fields: Vec::new(),
        }
    }
}

impl Visit for JsonVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let val = format!("{:?}", value);
        if field.name() == "message" {
            self.message = Some(val);
        } else {
            // Push the raw value under its field name. Redaction happens at emit
            // time in `on_event` via `render`, which redacts by field name
            // (`_secret` suffix, `_url` scrubbing) —
            // exactly like every other AFDATA surface. The visitor never scans
            // rendered values for secret markers.
            self.fields
                .push((field.name().to_string(), serde_json::Value::String(val)));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push((
                field.name().to_string(),
                serde_json::Value::String(value.to_string()),
            ));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.push((
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        ));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.push((
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        ));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if let Some(n) = serde_json::Number::from_f64(value) {
            self.fields
                .push((field.name().to_string(), serde_json::Value::Number(n)));
        } else {
            self.fields.push((
                field.name().to_string(),
                serde_json::Value::String(value.to_string()),
            ));
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .push((field.name().to_string(), serde_json::Value::Bool(value)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // The tracing layer redacts log fields the same way every AFDATA surface
    // does: by FIELD NAME, applied by `output_*` at emit time — never by
    // scanning a rendered value for the substring "_secret". These tests pin
    // that contract (the visitor records raw values; emit redacts by name).

    #[test]
    fn code_field_is_accepted_by_log_builder() {
        let value = crate::json_log(json!({"code": "cache_miss"})).build();
        assert_eq!(value.as_value()["log"]["code"], "cache_miss");
    }

    #[test]
    fn secret_named_field_is_redacted_at_emit() {
        let line = crate::render(
            &json!({
                "code": "info",
                "api_key_secret": "sk-live-123",
            }),
            crate::OutputFormat::Json,
            &crate::OutputOptions::default(),
        );
        assert!(line.contains("\"api_key_secret\":\"***\""), "{line}");
        assert!(!line.contains("sk-live-123"), "{line}");
    }

    #[test]
    fn non_secret_field_whose_value_mentions_secret_is_not_redacted() {
        // A real secret value never contains the literal "_secret"; the old
        // substring scan only ever produced false positives like this one.
        let line = crate::render(
            &json!({
                "code": "info",
                "note": "see the api_key_secret field in docs",
            }),
            crate::OutputFormat::Json,
            &crate::OutputOptions::default(),
        );
        assert!(
            line.contains("see the api_key_secret field in docs"),
            "{line}"
        );
    }

    #[test]
    fn secret_typed_field_is_redacted_regardless_of_record_path() {
        // record_str / record_i64 etc. push raw values too; emit-time redaction
        // covers every record_* path, not just record_debug.
        let line = crate::render(
            &json!({
                "code": "warn",
                "db_password_secret": 1234,
            }),
            crate::OutputFormat::Json,
            &crate::OutputOptions::default(),
        );
        assert!(line.contains("\"db_password_secret\":\"***\""), "{line}");
    }

    #[test]
    fn legacy_secret_names_are_redacted_when_layer_has_options() {
        let value = crate::json_log(json!({
            "level": "info",
            "message": "authorization appears in message but is not name-redacted",
            "timestamp_epoch_ms": 1,
            "authorization": "Bearer legacy",
            "request_url": "https://example.test/path?authorization=legacy&ok=1",
        }))
        .build();
        let redactor = crate::Redactor::new().secret_names(vec!["authorization".to_string()]);

        let formats = [LogFormat::Json, LogFormat::Plain, LogFormat::Yaml];

        for format in formats {
            let layer = AfdataLayer::new(format, redactor.clone());
            let line = layer.format_value(value.as_value());
            assert!(line.contains("***"), "{line}");
            assert!(
                !line.contains("Bearer legacy"),
                "legacy field value should be redacted: {line}"
            );
            assert!(
                !line.contains("authorization=legacy"),
                "legacy URL query parameter should be redacted: {line}"
            );
            assert!(
                line.contains("authorization appears in message"),
                "message is free-form and should remain readable: {line}"
            );
        }
    }

    #[test]
    fn legacy_secret_names_are_visible_without_layer_options() {
        let value = crate::json_log(json!({
            "level": "info",
            "message": "ready",
            "timestamp_epoch_ms": 1,
            "authorization": "Bearer visible",
        }))
        .build();
        let layer = AfdataLayer::new(LogFormat::Json, crate::Redactor::new());

        let line = layer.format_value(value.as_value());
        assert!(
            line.contains("\"authorization\":\"Bearer visible\""),
            "{line}"
        );
    }

    #[test]
    fn log_format_round_trips_text_and_serde() {
        for format in [LogFormat::Json, LogFormat::Plain, LogFormat::Yaml] {
            assert_eq!(format.to_string().parse(), Ok(format));
            let encoded = serde_json::to_string(&format).unwrap_or_default();
            assert_eq!(
                serde_json::from_str::<LogFormat>(&encoded).ok(),
                Some(format)
            );
        }

        let canary = "canary-log-format-secret";
        let error = canary.parse::<LogFormat>().unwrap_err();
        assert!(!error.contains(canary));
        assert!(error.contains("json"));
    }

    #[derive(Clone)]
    struct MemoryMakeWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    struct MemoryWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for MemoryWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let mut bytes = self
                .bytes
                .lock()
                .map_err(|_| io::Error::other("test buffer lock poisoned"))?;
            bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> MakeWriter<'writer> for MemoryMakeWriter {
        type Writer = MemoryWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            MemoryWriter {
                bytes: Arc::clone(&self.bytes),
            }
        }
    }

    #[test]
    fn structured_handle_and_tracing_share_writer_and_keep_nested_json() {
        use tracing_subscriber::layer::SubscriberExt;

        let bytes = Arc::new(Mutex::new(Vec::new()));
        let layer = AfdataLayer::new(LogFormat::Json, crate::Redactor::new()).with_writer(
            MemoryMakeWriter {
                bytes: Arc::clone(&bytes),
            },
        );
        let structured = layer.structured_log_handle();
        structured
            .emit(json!({
                "level": "info",
                "message": "configuration loaded",
                "configuration": {
                    "region": "test",
                    "credential_secret": "do-not-log"
                }
            }))
            .unwrap_or_else(|error| panic!("{error}"));

        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(request_id = 7_u64, "ordinary event");
        });

        let output = {
            let bytes = bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            String::from_utf8(bytes.clone()).unwrap_or_else(|error| panic!("{error}"))
        };
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2, "{output}");
        let structured_value: serde_json::Value =
            serde_json::from_str(lines[0]).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            structured_value["log"]["configuration"]["region"],
            serde_json::json!("test")
        );
        assert_eq!(
            structured_value["log"]["configuration"]["credential_secret"],
            serde_json::json!("***")
        );
        assert!(lines[1].contains("ordinary event"), "{output}");
    }

    /// Taking the handle before `with_writer` must not leave it addressing the
    /// old stream. Nothing would report that split, so the ordering the docs
    /// promise is pinned here rather than assumed.
    #[test]
    fn handle_taken_before_with_writer_follows_the_new_writer() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let layer = AfdataLayer::new(LogFormat::Json, crate::Redactor::new());
        // Handle first, writer second — the inverse of the usual order.
        let structured = layer.structured_log_handle();
        let _layer = layer.with_writer(MemoryMakeWriter {
            bytes: Arc::clone(&bytes),
        });

        structured
            .emit(json!({"message": "after rewiring"}))
            .unwrap_or_else(|error| panic!("{error}"));

        let output = {
            let bytes = bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            String::from_utf8(bytes.clone()).unwrap_or_else(|error| panic!("{error}"))
        };
        assert!(
            output.contains("after rewiring"),
            "handle wrote somewhere else entirely: {output:?}"
        );
    }

    /// Every line on the stream carries the envelope a reader filters on, so a
    /// level filter cannot silently drop the directly-emitted ones.
    #[test]
    fn directly_emitted_events_carry_level_and_timestamp() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let layer = AfdataLayer::new(LogFormat::Json, crate::Redactor::new()).with_writer(
            MemoryMakeWriter {
                bytes: Arc::clone(&bytes),
            },
        );
        let structured = layer.structured_log_handle();
        structured
            .emit(json!({"message": "defaulted"}))
            .unwrap_or_else(|error| panic!("{error}"));
        structured
            .emit(json!({"level": "warn", "message": "explicit"}))
            .unwrap_or_else(|error| panic!("{error}"));

        let output = {
            let bytes = bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            String::from_utf8(bytes.clone()).unwrap_or_else(|error| panic!("{error}"))
        };
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 2, "{output}");
        for (line, expected_level) in lines.iter().zip(["info", "warn"]) {
            let value: serde_json::Value =
                serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(value["log"]["level"], serde_json::json!(expected_level));
            assert!(
                value["log"]["timestamp_epoch_ms"].is_number(),
                "missing timestamp: {line}"
            );
        }
    }

    #[test]
    fn trace_maps_to_debug_and_fields_cannot_override_metadata() {
        use tracing_subscriber::layer::SubscriberExt;

        let bytes = Arc::new(Mutex::new(Vec::new()));
        let layer = AfdataLayer::new(LogFormat::Json, crate::Redactor::new()).with_writer(
            MemoryMakeWriter {
                bytes: Arc::clone(&bytes),
            },
        );
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(level = "error", timestamp_epoch_ms = 1_i64, "trace event");
        });

        let output = {
            let bytes = bytes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            String::from_utf8(bytes.clone()).unwrap_or_else(|error| panic!("{error}"))
        };
        let value: serde_json::Value =
            serde_json::from_str(output.trim()).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(value["log"]["level"], "debug");
        assert_eq!(value["log"]["message"], "trace event");
        assert_ne!(value["log"]["timestamp_epoch_ms"], 1);
    }
}
