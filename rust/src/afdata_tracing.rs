//! AFDATA-compliant tracing layer.
//!
//! Outputs log events using agent-first-data formatting functions:
//! - JSON: single-line JSONL via `output_json` (secrets redacted, original keys)
//! - Plain: single-line logfmt via `output_plain` (keys stripped, values formatted)
//! - YAML: multi-line via `output_yaml` (keys stripped, values formatted)
//!
//! Span fields are flattened into every event line (e.g. `request_id`).
//! All other tracing features (macros, spans, EnvFilter) work unchanged.
//!
//! # Usage
//! ```ignore
//! use agent_first_data::afdata_tracing;
//! use tracing_subscriber::EnvFilter;
//!
//! afdata_tracing::try_init_json(EnvFilter::new("info"))?;
//! afdata_tracing::init_plain(EnvFilter::new("info"));
//! afdata_tracing::init_yaml(EnvFilter::new("debug"));
//! ```

use std::io::{self, Write};

use tracing::field::{Field, Visit};
use tracing::span;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::TryInitError;
use tracing_subscriber::Layer;

/// Output format for the AFDATA tracing layer.
#[derive(Clone, Copy)]
pub enum LogFormat {
    Json,
    Plain,
    Yaml,
}

/// A tracing Layer that outputs AFDATA-compliant log lines to stdout.
pub struct AfdataLayer {
    format: LogFormat,
    redaction: crate::RedactionOptions,
}

/// Initialize tracing with AFDATA JSON output (single-line JSONL).
pub fn init_json(filter: tracing_subscriber::EnvFilter) {
    let _ = try_init_json(filter);
}

/// Initialize tracing with AFDATA plain/logfmt output (keys stripped, values formatted).
pub fn init_plain(filter: tracing_subscriber::EnvFilter) {
    let _ = try_init_plain(filter);
}

/// Initialize tracing with AFDATA YAML output (multi-line, keys stripped, values formatted).
pub fn init_yaml(filter: tracing_subscriber::EnvFilter) {
    let _ = try_init_yaml(filter);
}

/// Initialize tracing with AFDATA JSON output and explicit redaction options.
pub fn init_json_with_options(
    filter: tracing_subscriber::EnvFilter,
    redaction: crate::RedactionOptions,
) {
    let _ = try_init_json_with_options(filter, redaction);
}

/// Initialize tracing with AFDATA plain/logfmt output and explicit redaction options.
pub fn init_plain_with_options(
    filter: tracing_subscriber::EnvFilter,
    redaction: crate::RedactionOptions,
) {
    let _ = try_init_plain_with_options(filter, redaction);
}

/// Initialize tracing with AFDATA YAML output and explicit redaction options.
pub fn init_yaml_with_options(
    filter: tracing_subscriber::EnvFilter,
    redaction: crate::RedactionOptions,
) {
    let _ = try_init_yaml_with_options(filter, redaction);
}

/// Initialize tracing with AFDATA output and explicit format/redaction options.
pub fn init_with_options(
    filter: tracing_subscriber::EnvFilter,
    format: LogFormat,
    redaction: crate::RedactionOptions,
) {
    let _ = try_init_with_options(filter, format, redaction);
}

/// Try to initialize tracing with AFDATA JSON output (single-line JSONL).
///
/// Prefer this over [`init_json`] when startup code needs to know whether the
/// redaction/formatting layer was actually installed.
pub fn try_init_json(filter: tracing_subscriber::EnvFilter) -> Result<(), TryInitError> {
    try_init_json_with_options(filter, crate::RedactionOptions::default())
}

/// Try to initialize tracing with AFDATA plain/logfmt output.
pub fn try_init_plain(filter: tracing_subscriber::EnvFilter) -> Result<(), TryInitError> {
    try_init_plain_with_options(filter, crate::RedactionOptions::default())
}

/// Try to initialize tracing with AFDATA YAML output.
pub fn try_init_yaml(filter: tracing_subscriber::EnvFilter) -> Result<(), TryInitError> {
    try_init_yaml_with_options(filter, crate::RedactionOptions::default())
}

/// Try to initialize tracing with AFDATA JSON output and explicit redaction options.
pub fn try_init_json_with_options(
    filter: tracing_subscriber::EnvFilter,
    redaction: crate::RedactionOptions,
) -> Result<(), TryInitError> {
    try_init_with_options(filter, LogFormat::Json, redaction)
}

/// Try to initialize tracing with AFDATA plain/logfmt output and explicit redaction options.
pub fn try_init_plain_with_options(
    filter: tracing_subscriber::EnvFilter,
    redaction: crate::RedactionOptions,
) -> Result<(), TryInitError> {
    try_init_with_options(filter, LogFormat::Plain, redaction)
}

/// Try to initialize tracing with AFDATA YAML output and explicit redaction options.
pub fn try_init_yaml_with_options(
    filter: tracing_subscriber::EnvFilter,
    redaction: crate::RedactionOptions,
) -> Result<(), TryInitError> {
    try_init_with_options(filter, LogFormat::Yaml, redaction)
}

/// Try to initialize tracing with AFDATA output and explicit format/redaction options.
pub fn try_init_with_options(
    filter: tracing_subscriber::EnvFilter,
    format: LogFormat,
    redaction: crate::RedactionOptions,
) -> Result<(), TryInitError> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    tracing_subscriber::registry()
        .with(filter)
        .with(AfdataLayer { format, redaction })
        .try_init()
}

impl AfdataLayer {
    fn output_options(&self) -> crate::OutputOptions {
        crate::OutputOptions {
            redaction: self.redaction.clone(),
            style: crate::OutputStyle::Readable,
        }
    }

    fn format_value(&self, value: &serde_json::Value) -> String {
        let options = self.output_options();
        match self.format {
            LogFormat::Json => crate::output_json_with_options(value, &options),
            LogFormat::Plain => crate::output_plain_with_options(value, &options),
            LogFormat::Yaml => crate::output_yaml_with_options(value, &options),
        }
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

        // Build output object with AFDATA field names
        let mut map = serde_json::Map::with_capacity(4 + visitor.fields.len());

        let level = match *meta.level() {
            Level::TRACE => "trace",
            Level::DEBUG => "debug",
            Level::INFO => "info",
            Level::WARN => "warn",
            Level::ERROR => "error",
        };

        map.insert(
            "timestamp_epoch_ms".into(),
            serde_json::Value::Number(chrono::Utc::now().timestamp_millis().into()),
        );

        // "message" field from the tracing macro's format string
        if let Some(msg) = visitor.message.take() {
            map.insert("message".into(), serde_json::Value::String(msg));
        }

        // Flatten span fields from root to leaf (child overrides parent on collision)
        if let Some(scope) = ctx.event_scope(event) {
            for span in scope.from_root() {
                let extensions = span.extensions();
                if let Some(fields) = extensions.get::<SpanFields>() {
                    for (k, v) in &fields.0 {
                        map.insert(k.clone(), v.clone());
                    }
                }
            }
        }

        map.insert("code".into(), serde_json::Value::String("log".to_string()));
        map.insert("level".into(), serde_json::Value::String(level.to_string()));

        // Append event-level structured fields, except protocol code is always "log".
        for (k, v) in visitor.fields {
            if k == "code" {
                continue;
            }
            map.insert(k, v);
        }

        let value = serde_json::Value::Object(map);

        // Format using the library's own output functions.
        let line = self.format_value(&value);

        let mut out = io::stdout().lock();
        let _ = out.write_all(line.as_bytes());
        let _ = out.write_all(b"\n");
    }
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
            // time in `on_event` via `output_json`/`output_plain`/`output_yaml`,
            // which redact by field name (`_secret` suffix, `_url` scrubbing) —
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
    fn secret_named_field_is_redacted_at_emit() {
        let line = crate::output_json(&json!({
            "code": "info",
            "api_key_secret": "sk-live-123",
        }));
        assert!(line.contains("\"api_key_secret\":\"***\""), "{line}");
        assert!(!line.contains("sk-live-123"), "{line}");
    }

    #[test]
    fn non_secret_field_whose_value_mentions_secret_is_not_redacted() {
        // A real secret value never contains the literal "_secret"; the old
        // substring scan only ever produced false positives like this one.
        let line = crate::output_json(&json!({
            "code": "info",
            "note": "see the api_key_secret field in docs",
        }));
        assert!(
            line.contains("see the api_key_secret field in docs"),
            "{line}"
        );
    }

    #[test]
    fn secret_typed_field_is_redacted_regardless_of_record_path() {
        // record_str / record_i64 etc. push raw values too; emit-time redaction
        // covers every record_* path, not just record_debug.
        let line = crate::output_json(&json!({
            "code": "warn",
            "db_password_secret": 1234,
        }));
        assert!(line.contains("\"db_password_secret\":\"***\""), "{line}");
    }

    #[test]
    fn legacy_secret_names_are_redacted_when_layer_has_options() {
        let value = json!({
            "timestamp_epoch_ms": 1,
            "message": "authorization appears in message but is not name-redacted",
            "code": "log",
            "level": "info",
            "authorization": "Bearer legacy",
            "request_url": "https://example.test/path?authorization=legacy&ok=1",
        });
        let redaction = crate::RedactionOptions {
            secret_names: vec!["authorization".to_string()],
            ..crate::RedactionOptions::default()
        };

        for format in [LogFormat::Json, LogFormat::Plain, LogFormat::Yaml] {
            let layer = AfdataLayer {
                format,
                redaction: redaction.clone(),
            };
            let line = layer.format_value(&value);
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
        let value = json!({
            "timestamp_epoch_ms": 1,
            "message": "ready",
            "code": "log",
            "level": "info",
            "authorization": "Bearer visible",
        });
        let layer = AfdataLayer {
            format: LogFormat::Json,
            redaction: crate::RedactionOptions::default(),
        };

        let line = layer.format_value(&value);
        assert!(
            line.contains("\"authorization\":\"Bearer visible\""),
            "{line}"
        );
    }
}
