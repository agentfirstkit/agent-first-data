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
//! use agent_first_data::afdata_tracing;
//! use tracing_subscriber::EnvFilter;
//!
//! afdata_tracing::try_init(EnvFilter::new("info"), LogFormat::Json, Redactor::new())?;
//! ```

use std::io::{self, Write};

use tracing::field::{Field, Visit};
use tracing::span;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::TryInitError;

/// Output format for the AFDATA tracing layer.
#[derive(Clone, Copy, Debug)]
pub enum LogFormat {
    Json,
    Plain,
    /// Structure-preserving YAML.
    Yaml,
}

/// A tracing Layer that outputs AFDATA-compliant log lines to stdout.
pub struct AfdataLayer {
    format: LogFormat,
    redactor: crate::Redactor,
}

/// Try to initialize tracing with AFDATA output.
///
/// Returns `Err` if a global tracing subscriber is already initialized. This is
/// the single entry point for tracing initialization; pass your desired format
/// and redactor configuration.
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
        .with(AfdataLayer { format, redactor })
        .try_init()
}

impl AfdataLayer {
    fn output_options(&self) -> crate::OutputOptions {
        crate::OutputOptions {
            redaction: self.redactor.clone(),
            style: crate::PlainStyle::Readable,
        }
    }

    fn format_value(&self, value: &serde_json::Value) -> String {
        let options = self.output_options();
        match self.format {
            LogFormat::Json => crate::render(value, crate::OutputFormat::Json, &options),
            LogFormat::Plain => crate::render(value, crate::OutputFormat::Plain, &options),
            LogFormat::Yaml => crate::render(value, crate::OutputFormat::Yaml, &options),
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

        map.insert("level".into(), serde_json::Value::String(level.to_string()));

        // Append event-level structured fields. Logs no longer use top-level
        // protocol code; code may be a tool-defined field inside the log
        // payload.
        for (k, v) in visitor.fields {
            map.insert(k, v);
        }

        // Normalize the tracing adapter's conventional message and level
        // fields. These are adapter output conventions, not protocol fields.
        let message = map
            .remove("message")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "(no message)".to_string());
        let level_str = map
            .remove("level")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "info".to_string());
        let level = match level_str.as_str() {
            "debug" => crate::LogLevel::Debug,
            "info" => crate::LogLevel::Info,
            "warn" => crate::LogLevel::Warn,
            "error" => crate::LogLevel::Error,
            _ => crate::LogLevel::Info,
        };

        map.insert(
            "level".to_string(),
            serde_json::Value::String(level.as_str().to_string()),
        );
        map.insert("message".to_string(), serde_json::Value::String(message));
        let builder = crate::json_log(serde_json::Value::Object(map));
        let value = builder.build();

        // Format using the library's own output functions.
        let line = self.format_value(value.as_value());

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
            let layer = AfdataLayer {
                format,
                redactor: redactor.clone(),
            };
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
        let layer = AfdataLayer {
            format: LogFormat::Json,
            redactor: crate::Redactor::new(),
        };

        let line = layer.format_value(value.as_value());
        assert!(
            line.contains("\"authorization\":\"Bearer visible\""),
            "{line}"
        );
    }
}
