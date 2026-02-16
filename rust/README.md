# agent-first-data

**Agent-First Data (AFD)** — Suffix-driven output formatting and protocol templates for AI agents.

The field name is the schema. Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact, no external schema needed.

## Installation

```bash
cargo add agent-first-data
```

## API Reference

Total: **9 public APIs** (4 protocol builders + 3 output functions + 1 internal + 1 utility)

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
