# agent-first-data

**Agent-First Data (AFD)** — Suffix-driven output formatting and protocol templates for AI agents.

The field name is the schema. Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact, no external schema needed.

## Installation

```bash
cargo add agent-first-data
```

## Quick Example

A backup tool invoked from the CLI — flags, env vars, and config all use the same suffixes:

```bash
API_KEY_SECRET=sk-1234 cloudback --timeout-s 30 --max-file-size-bytes 10737418240 /data/backup.tar.gz
```

The tool reads env vars, flags, and config — all with AFD suffixes — and emits a startup message:

```rust
use agent_first_data::*;
use serde_json::json;

let startup = build_json_startup(
    json!({"timeout_s": 30, "max_file_size_bytes": 10737418240u64}),
    json!({"input_path": "/data/backup.tar.gz"}),
    json!({"API_KEY_SECRET": std::env::var("API_KEY_SECRET").ok()}),
);
```

Three output formats, same data:

```
JSON:  {"code":"startup","args":{"input_path":"/data/backup.tar.gz"},"config":{"max_file_size_bytes":10737418240,"timeout_s":30},"env":{"API_KEY_SECRET":"***"}}
YAML:  code: "startup"
       args:
         input_path: "/data/backup.tar.gz"
       config:
         max_file_size: "10.0GB"
         timeout: "30s"
       env:
         API_KEY: "***"
Plain: args.input_path=/data/backup.tar.gz code=startup config.max_file_size=10.0GB config.timeout=30s env.API_KEY=***
```

`--timeout-s` → `timeout_s` → `timeout: 30s`. `API_KEY_SECRET` → `API_KEY: "***"`. The suffix is the schema.

## API Reference

Total: **9 public APIs** + optional **AFD tracing** (4 protocol builders + 3 output functions + 1 internal + 1 utility)

### Protocol Builders (returns JSON Value)

Build AFD protocol structures. Return `serde_json::Value` objects for API responses.

```rust
// Startup (configuration)
build_json_startup(config: Value, args: Value, env: Value) -> Value

// Success (result)
build_json_ok(result: Value, trace: Option<Value>) -> Value

// Error (simple message)
build_json_error(message: &str, trace: Option<Value>) -> Value

// Generic (any code + fields)
build_json(code: &str, fields: Value, trace: Option<Value>) -> Value
```

**Use case:** API responses (frameworks like axum automatically serialize)

**Example:**
```rust
use agent_first_data::*;
use serde_json::json;

// Startup
let startup = build_json_startup(
    json!({"api_key_secret": "sk-123", "timeout_s": 30}),
    json!({"config_path": "config.yml"}),
    json!({"RUST_LOG": "info"})
);

// Success (always include trace)
let response = build_json_ok(
    json!({"user_id": 123}),
    Some(json!({"duration_ms": 150, "source": "db"}))
);

// Error
let error = build_json_error(
    "user not found",
    Some(json!({"duration_ms": 5}))
);

// Specific error code
let not_found = build_json(
    "not_found",
    json!({"resource": "user", "id": 123}),
    Some(json!({"duration_ms": 8}))
);
```

### CLI/Log Output (returns String)

Format values for CLI output and logs. **All formats redact `_secret` fields.** YAML and Plain also strip suffixes from keys and format values for human readability.

```rust
output_json(value: &Value) -> String   // Single-line JSON, original keys, for programs/logs
output_yaml(value: &Value) -> String   // Multi-line YAML, keys stripped, values formatted
output_plain(value: &Value) -> String  // Single-line logfmt, keys stripped, values formatted
```

**Example:**
```rust
use agent_first_data::*;
use serde_json::json;

let data = json!({
    "user_id": 123,
    "api_key_secret": "sk-1234567890abcdef",
    "created_at_epoch_ms": 1738886400000i64,
    "file_size_bytes": 5242880
});

// JSON (secrets redacted, original keys, raw values)
println!("{}", output_json(&data));
// {"api_key_secret":"***","created_at_epoch_ms":1738886400000,"file_size_bytes":5242880,"user_id":123}

// YAML (keys stripped, values formatted, secrets redacted)
println!("{}", output_yaml(&data));
// ---
// api_key: "***"
// created_at: "2025-02-07T00:00:00.000Z"
// file_size: "5.0MB"
// user_id: 123

// Plain logfmt (keys stripped, values formatted, secrets redacted)
println!("{}", output_plain(&data));
// api_key=*** created_at=2025-02-07T00:00:00.000Z file_size=5.0MB user_id=123
```

### Internal Tools

```rust
internal_redact_secrets(value: &mut Value)  // Manually redact secrets in-place
```

Most users don't need this. Output functions automatically protect secrets.

### Utility Functions

```rust
parse_size(s: &str) -> Option<u64>  // Parse "10M" → bytes
```

**Example:**
```rust
use agent_first_data::*;

assert_eq!(parse_size("10M"), Some(10485760));
assert_eq!(parse_size("1.5K"), Some(1536));
assert_eq!(parse_size("512"), Some(512));
```

## Usage Examples

### Example 1: REST API

```rust
use agent_first_data::*;
use axum::{Json, http::StatusCode};
use serde_json::json;

async fn get_user(id: i64) -> (StatusCode, Json<Value>) {
    let response = build_json_ok(
        json!({"user_id": id, "name": "alice"}),
        Some(json!({"duration_ms": 150, "source": "db"}))
    );
    // API returns raw JSON — no output processing, no key stripping
    (StatusCode::OK, Json(response))
}
```

### Example 2: CLI Tool (Complete Lifecycle)

```rust
use agent_first_data::*;
use serde_json::json;

fn main() {
    // 1. Startup
    let startup = build_json_startup(
        json!({"api_key_secret": "sk-sensitive-key", "timeout_s": 30}),
        json!({"input_path": "data.json"}),
        json!({"RUST_LOG": "info"})
    );
    println!("{}", output_yaml(&startup));
    // ---
    // code: "startup"
    // args:
    //   input_path: "data.json"
    // config:
    //   api_key: "***"
    //   timeout: "30s"
    // env:
    //   RUST_LOG: "info"

    // 2. Progress
    let progress = build_json(
        "progress",
        json!({"current": 3, "total": 10, "message": "processing"}),
        Some(json!({"duration_ms": 1500}))
    );
    println!("{}", output_plain(&progress));
    // code=progress current=3 message=processing total=10 trace.duration=1.5s

    // 3. Result
    let result = build_json_ok(
        json!({
            "records_processed": 10,
            "file_size_bytes": 5242880,
            "created_at_epoch_ms": 1738886400000i64
        }),
        Some(json!({"duration_ms": 3500, "source": "file"}))
    );
    println!("{}", output_yaml(&result));
    // ---
    // code: "ok"
    // result:
    //   created_at: "2025-02-07T00:00:00.000Z"
    //   file_size: "5.0MB"
    //   records_processed: 10
    // trace:
    //   duration: "3.5s"
    //   source: "file"
}
```

### Example 3: JSONL Output

```rust
use agent_first_data::*;
use serde_json::json;

fn process_request() {
    let result = build_json_ok(
        json!({"status": "success"}),
        Some(json!({"duration_ms": 250, "api_key_secret": "sk-123"}))
    );

    // Print JSONL to stdout (secrets redacted, one JSON object per line)
    println!("{}", output_json(&result));
    // {"code":"ok","result":{"status":"success"},"trace":{"api_key_secret":"***","duration_ms":250}}
}
```

## Complete Suffix Example

```rust
use agent_first_data::*;
use serde_json::json;

let data = json!({
    "created_at_epoch_ms": 1738886400000i64,
    "request_timeout_ms": 5000,
    "cache_ttl_s": 3600,
    "file_size_bytes": 5242880,
    "payment_msats": 50000000,
    "price_usd_cents": 9999,
    "success_rate_percent": 95.5,
    "api_key_secret": "sk-1234567890abcdef",
    "user_name": "alice",
    "count": 42
});

// YAML output (keys stripped, values formatted, secrets redacted)
println!("{}", output_yaml(&data));
// ---
// api_key: "***"
// cache_ttl: "3600s"
// count: 42
// created_at: "2025-02-07T00:00:00.000Z"
// file_size: "5.0MB"
// payment: "50000000msats"
// price: "$99.99"
// request_timeout: "5.0s"
// success_rate: "95.5%"
// user_name: "alice"

// Plain logfmt output (same transformations, single line)
println!("{}", output_plain(&data));
// api_key=*** cache_ttl=3600s count=42 created_at=2025-02-07T00:00:00.000Z file_size=5.0MB payment=50000000msats price=$99.99 request_timeout=5.0s success_rate=95.5% user_name=alice
```

## AFD Tracing (optional feature)

AFD-compliant structured logging via the `tracing` ecosystem. Enable with:

```bash
cargo add agent-first-data --features tracing
```

Every log line is formatted using the library's own `output_json`/`output_plain`/`output_yaml` functions. Span fields are automatically flattened into each event line, solving the concurrent-request log interleaving problem.

### API

```rust
use agent_first_data::afd_tracing;
use tracing_subscriber::EnvFilter;

// Convenience initializers — set up the default tracing subscriber with AFD output
afd_tracing::init_json(filter: EnvFilter)   // Single-line JSONL (secrets redacted, original keys)
afd_tracing::init_plain(filter: EnvFilter)  // Single-line logfmt (keys stripped, values formatted)
afd_tracing::init_yaml(filter: EnvFilter)   // Multi-line YAML (keys stripped, values formatted)

// Low-level — create a tracing Layer for custom subscriber stacks
AfdLayer { format: LogFormat }  // implements tracing_subscriber::Layer
LogFormat::Json | LogFormat::Plain | LogFormat::Yaml
```

### Setup

```rust
use agent_first_data::afd_tracing;
use tracing_subscriber::EnvFilter;

// JSON output for production (one JSONL line per event, secrets redacted)
afd_tracing::init_json(EnvFilter::new("info"));

// Plain logfmt for development (keys stripped, values formatted)
afd_tracing::init_plain(EnvFilter::new("debug"));

// YAML for detailed inspection (multi-line, keys stripped, values formatted)
afd_tracing::init_yaml(EnvFilter::new("debug"));
```

### Log Output

Standard `tracing` macros work unchanged. Output format depends on the init function used.

```rust
use tracing::{info, warn, info_span};

info!("Server started");
// JSON:  {"timestamp_epoch_ms":1739000000000,"message":"Server started","target":"myapp","code":"info"}
// Plain: code=info message="Server started" target=myapp timestamp_epoch_ms=1739000000000
// YAML:  ---
//        code: "info"
//        message: "Server started"
//        target: "myapp"
//        timestamp_epoch_ms: 1739000000000

warn!(latency_ms = 1280, domain = %domain, "DNS lookup failed");
// JSON:  {"timestamp_epoch_ms":...,"message":"DNS lookup failed","target":"myapp","domain":"example.com","latency_ms":1280,"code":"warn"}
// Plain: code=warn domain=example.com latency=1.28s message="DNS lookup failed" target=myapp ...
```

### Span Support

Span fields are flattened into every event inside the span. Child spans override parent fields on collision.

```rust
let span = info_span!("request", request_id = %uuid);
let _guard = span.enter();

info!("Processing");
// {"timestamp_epoch_ms":...,"message":"Processing","target":"myapp","request_id":"abc-123","code":"info"}

warn!(error = "not found", "Failed");
// {"timestamp_epoch_ms":...,"message":"Failed","target":"myapp","request_id":"abc-123","error":"not found","code":"warn"}
```

### Custom Code Override

The `code` field defaults to the log level (trace/debug/info/warn/error). Override with an explicit `code` field:

```rust
info!(code = "startup", "Server ready");
// {"timestamp_epoch_ms":...,"message":"Server ready","target":"myapp","code":"startup"}
```

### Output Fields

Every log line contains:

| Field | Type | Description |
|:------|:-----|:------------|
| `timestamp_epoch_ms` | number | Unix milliseconds |
| `message` | string | Log message |
| `target` | string | Source module path |
| `code` | string | Level (trace/debug/info/warn/error) or explicit override |
| *span fields* | any | Flattened from root span to leaf span |
| *event fields* | any | Structured fields from the log macro |

### Log Output Formats

All three formats use the library's own output functions, so AFD suffix processing applies to log fields too:

| Format | Function | Keys | Values | Use case |
|:-------|:---------|:-----|:-------|:---------|
| **JSON** | `init_json` | original (with suffix) | raw | production, log aggregation |
| **Plain** | `init_plain` | stripped | formatted | development, compact scanning |
| **YAML** | `init_yaml` | stripped | formatted | debugging, detailed inspection |

All formats automatically redact `_secret` fields in log output.

## Output Formats

Three output formats for different use cases:

| Format | Structure | Keys | Values | Use case |
|:-------|:----------|:-----|:-------|:---------|
| **JSON** | single-line | original (with suffix) | raw | programs, logs |
| **YAML** | multi-line | stripped | formatted | human inspection |
| **Plain** | single-line logfmt | stripped | formatted | compact scanning |

All formats automatically redact `_secret` fields.

## Supported Suffixes

- **Duration**: `_ms`, `_s`, `_ns`, `_us`, `_minutes`, `_hours`, `_days`
- **Timestamps**: `_epoch_ms`, `_epoch_s`, `_epoch_ns`, `_rfc3339`
- **Size**: `_bytes` (auto-scales to KB/MB/GB/TB), `_size` (config input, pass through)
- **Currency**: `_msats`, `_sats`, `_btc`, `_usd_cents`, `_eur_cents`, `_jpy`, `_{code}_cents`
- **Other**: `_percent`, `_secret` (auto-redacted in all formats)

## License

MIT
