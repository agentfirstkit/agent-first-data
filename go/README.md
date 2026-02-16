# agent-first-data

**Agent-First Data (AFD)** — Suffix-driven output formatting and protocol templates for AI agents.

The field name is the schema. Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact, no external schema needed.

## Installation

```bash
go get github.com/cmnspore/agent-first-data/go
```

## API Reference

Total: **9 public APIs** (4 protocol builders + 3 output functions + 1 internal + 1 utility)

### Protocol Builders (returns map[string]any)

Build AFD protocol structures. Return JSON-serializable objects for API responses.

```go
// Startup (configuration)
BuildJsonStartup(config, args, env any) map[string]any

// Success (result)
BuildJsonOk(result any, trace any) map[string]any

// Error (simple message)
BuildJsonError(message string, trace any) map[string]any

// Generic (any code + fields)
BuildJson(code string, fields any, trace any) map[string]any
```

**Use case:** API responses (frameworks like net/http or gin serialize to JSON)

**Example:**
```go
import afd "github.com/cmnspore/agent-first-data/go"

// Startup
startup := afd.BuildJsonStartup(
    map[string]any{"api_key_secret": "sk-123", "timeout_s": 30},
    map[string]any{"config_path": "config.yml"},
    map[string]any{"RUST_LOG": "info"},
)

// Success (always include trace)
response := afd.BuildJsonOk(
    map[string]any{"user_id": 123},
    map[string]any{"duration_ms": 150, "source": "db"},
)

// Error
err := afd.BuildJsonError("user not found", map[string]any{"duration_ms": 5})

// Specific error code
notFound := afd.BuildJson(
    "not_found",
    map[string]any{"resource": "user", "id": 123},
    map[string]any{"duration_ms": 8},
)
```

### CLI/Log Output (returns string)

Format values for CLI output and logs. **All formats redact `_secret` fields.** YAML and Plain also strip suffixes from keys and format values for human readability.

```go
OutputJson(value any) string   // Single-line JSON, original keys, for programs/logs
OutputYaml(value any) string   // Multi-line YAML, keys stripped, values formatted
OutputPlain(value any) string  // Single-line logfmt, keys stripped, values formatted
```

**Example:**
```go
import afd "github.com/cmnspore/agent-first-data/go"

data := map[string]any{
    "user_id":              123,
    "api_key_secret":       "sk-1234567890abcdef",
    "created_at_epoch_ms":  int64(1738886400000),
    "file_size_bytes":      5242880,
}

// JSON (secrets redacted, original keys, raw values)
fmt.Println(afd.OutputJson(data))
// {"api_key_secret":"***","created_at_epoch_ms":1738886400000,"file_size_bytes":5242880,"user_id":123}

// YAML (keys stripped, values formatted, secrets redacted)
fmt.Println(afd.OutputYaml(data))
// ---
// api_key: "***"
// created_at: "2025-02-07T00:00:00.000Z"
// file_size: "5.0MB"
// user_id: 123

// Plain logfmt (keys stripped, values formatted, secrets redacted)
fmt.Println(afd.OutputPlain(data))
// api_key=*** created_at=2025-02-07T00:00:00.000Z file_size=5.0MB user_id=123
```

### Internal Tools

```go
InternalRedactSecrets(value any)  // Manually redact secrets in-place
```

Most users don't need this. Output functions automatically protect secrets.

### Utility Functions

```go
ParseSize(s string) (uint64, bool)  // Parse "10M" → bytes
```

**Example:**
```go
import afd "github.com/cmnspore/agent-first-data/go"

size, _ := afd.ParseSize("10M")   // 10485760
size, _ = afd.ParseSize("1.5K")   // 1536
size, _ = afd.ParseSize("512")    // 512
```

## Usage Examples

### Example 1: REST API

```go
import afd "github.com/cmnspore/agent-first-data/go"

func getUserHandler(w http.ResponseWriter, r *http.Request) {
    response := afd.BuildJsonOk(
        map[string]any{"user_id": 123, "name": "alice"},
        map[string]any{"duration_ms": 150, "source": "db"},
    )
    // API returns raw JSON — no output processing, no key stripping
    json.NewEncoder(w).Encode(response)
}
```

### Example 2: CLI Tool (Complete Lifecycle)

```go
import afd "github.com/cmnspore/agent-first-data/go"

func main() {
    // 1. Startup
    startup := afd.BuildJsonStartup(
        map[string]any{"api_key_secret": "sk-sensitive-key", "timeout_s": 30},
        map[string]any{"input_path": "data.json"},
        map[string]any{"RUST_LOG": "info"},
    )
    fmt.Println(afd.OutputYaml(startup))
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
    progress := afd.BuildJson(
        "progress",
        map[string]any{"current": 3, "total": 10, "message": "processing"},
        map[string]any{"duration_ms": 1500},
    )
    fmt.Println(afd.OutputPlain(progress))
    // code=progress current=3 message=processing total=10 trace.duration=1.5s

    // 3. Result
    result := afd.BuildJsonOk(
        map[string]any{
            "records_processed":    10,
            "file_size_bytes":      5242880,
            "created_at_epoch_ms":  int64(1738886400000),
        },
        map[string]any{"duration_ms": 3500, "source": "file"},
    )
    fmt.Println(afd.OutputYaml(result))
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

```go
import afd "github.com/cmnspore/agent-first-data/go"

func processRequest() {
    result := afd.BuildJsonOk(
        map[string]any{"status": "success"},
        map[string]any{"duration_ms": 250, "api_key_secret": "sk-123"},
    )

    // Print JSONL to stdout (secrets redacted, one JSON object per line)
    fmt.Println(afd.OutputJson(result))
    // {"code":"ok","result":{"status":"success"},"trace":{"api_key_secret":"***","duration_ms":250}}
}
```

## Complete Suffix Example

```go
import afd "github.com/cmnspore/agent-first-data/go"

data := map[string]any{
    "created_at_epoch_ms":   int64(1738886400000),
    "request_timeout_ms":    5000,
    "cache_ttl_s":           3600,
    "file_size_bytes":       5242880,
    "payment_msats":         50000000,
    "price_usd_cents":       9999,
    "success_rate_percent":  95.5,
    "api_key_secret":        "sk-1234567890abcdef",
    "user_name":             "alice",
    "count":                 42,
}

// YAML output (keys stripped, values formatted, secrets redacted)
fmt.Println(afd.OutputYaml(data))
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
fmt.Println(afd.OutputPlain(data))
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
