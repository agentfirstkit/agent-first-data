//! AFD-compliant tracing layer.
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
//! use agent_first_data::afd_tracing;
//! use tracing_subscriber::EnvFilter;
//!
//! afd_tracing::init_json(EnvFilter::new("info"));
//! afd_tracing::init_plain(EnvFilter::new("info"));
//! afd_tracing::init_yaml(EnvFilter::new("debug"));
//! ```

use std::io::{self, Write};

use tracing::field::{Field, Visit};
use tracing::span;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// Output format for the AFD tracing layer.
#[derive(Clone, Copy)]
pub enum LogFormat {
    Json,
    Plain,
    Yaml,
}

/// A tracing Layer that outputs AFD-compliant log lines to stdout.
pub struct AfdLayer {
    format: LogFormat,
}

/// Initialize tracing with AFD JSON output (single-line JSONL).
pub fn init_json(filter: tracing_subscriber::EnvFilter) {
    init_with_format(filter, LogFormat::Json);
}

/// Initialize tracing with AFD plain/logfmt output (keys stripped, values formatted).
pub fn init_plain(filter: tracing_subscriber::EnvFilter) {
    init_with_format(filter, LogFormat::Plain);
}

/// Initialize tracing with AFD YAML output (multi-line, keys stripped, values formatted).
pub fn init_yaml(filter: tracing_subscriber::EnvFilter) {
    init_with_format(filter, LogFormat::Yaml);
}

fn init_with_format(filter: tracing_subscriber::EnvFilter, format: LogFormat) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    tracing_subscriber::registry()
        .with(filter)
        .with(AfdLayer { format })
        .init();
}

/// Stored in span extensions to carry structured fields.
struct SpanFields(Vec<(String, serde_json::Value)>);

impl<S> Layer<S> for AfdLayer
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

        // Build output object with AFD field names
        let mut map = serde_json::Map::with_capacity(4 + visitor.fields.len());

        // Default code from level; can be overridden by explicit code = "..." in the macro
        let default_code = match *meta.level() {
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

        map.insert(
            "target".into(),
            serde_json::Value::String(meta.target().to_string()),
        );

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

        // Append all event-level structured fields (override span fields on collision)
        let mut has_code = false;
        for (k, v) in visitor.fields {
            if k == "code" {
                has_code = true;
            }
            map.insert(k, v);
        }
        if !has_code {
            map.insert(
                "code".into(),
                serde_json::Value::String(default_code.to_string()),
            );
        }

        let value = serde_json::Value::Object(map);

        // Format using the library's own output functions
        let line = match self.format {
            LogFormat::Json => crate::output_json(&value),
            LogFormat::Plain => crate::output_plain(&value),
            LogFormat::Yaml => crate::output_yaml(&value),
        };

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
