# agent-first-data

**Agent-First Data (AFDATA)** — Suffix-driven output formatting and protocol templates for AI agents.

The field name is the schema. Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact, no external schema needed.

## Installation

```bash
go get github.com/cmnspore/agent-first-data/go
```

## Quick Example

A backup tool invoked from the CLI — flags, env vars, and config all use the same suffixes:

```bash
API_KEY_SECRET=sk-1234 cloudback --timeout-s 30 --max-file-size-bytes 10737418240 /data/backup.tar.gz
```

For CLI diagnostics, enable log categories explicitly:

```bash
--log startup,request,progress,retry,redirect
--verbose   # shorthand for all categories
```

Without these flags, startup diagnostics should stay off by default.

The tool reads env vars, flags, and config — all with AFDATA suffixes — and can emit a startup diagnostic event:

```go
import afdata "github.com/cmnspore/agent-first-data/go"

startup := afdata.BuildJson(
    "log",
    map[string]any{
        "event":  "startup",
        "config": map[string]any{"timeout_s": 30, "max_file_size_bytes": 10737418240},
        "args":   map[string]any{"input_path": "/data/backup.tar.gz"},
        "env":    map[string]any{"API_KEY_SECRET": os.Getenv("API_KEY_SECRET")},
    },
    nil,
)
```

Three output formats, same data:

```
JSON:  {"code":"log","event":"startup","args":{"input_path":"/data/backup.tar.gz"},"config":{"max_file_size_bytes":10737418240,"timeout_s":30},"env":{"API_KEY_SECRET":"***"}}
YAML:  code: "log"
       event: "startup"
       args:
         input_path: "/data/backup.tar.gz"
       config:
         max_file_size: "10.0GB"
         timeout: "30s"
       env:
         API_KEY: "***"
Plain: args.input_path=/data/backup.tar.gz code=log event=startup config.max_file_size=10.0GB config.timeout=30s env.API_KEY=***
```

`--timeout-s` → `timeout_s` → `timeout: 30s`. `API_KEY_SECRET` → `API_KEY: "***"`. The suffix is the schema.

## API Reference

Total: **8 public APIs** + **AFDATA logging** (3 protocol builders + 3 output functions + 1 internal + 1 utility)

### Protocol Builders (returns map[string]any)

Build AFDATA protocol structures. Return JSON-serializable objects for API responses.

```go
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
import afdata "github.com/cmnspore/agent-first-data/go"

// Startup
startup := afdata.BuildJson(
    "log",
    map[string]any{
        "event":  "startup",
        "config": map[string]any{"api_key_secret": "sk-123", "timeout_s": 30},
        "args":   map[string]any{"config_path": "config.yml"},
        "env":    map[string]any{"RUST_LOG": "info"},
    },
    nil,
)

// Success (always include trace)
response := afdata.BuildJsonOk(
    map[string]any{"user_id": 123},
    map[string]any{"duration_ms": 150, "source": "db"},
)

// Error
err := afdata.BuildJsonError("user not found", map[string]any{"duration_ms": 5})

// Specific error code
notFound := afdata.BuildJson(
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
import afdata "github.com/cmnspore/agent-first-data/go"

data := map[string]any{
    "user_id":              123,
    "api_key_secret":       "sk-1234567890abcdef",
    "created_at_epoch_ms":  int64(1738886400000),
    "file_size_bytes":      5242880,
}

// JSON (secrets redacted, original keys, raw values)
fmt.Println(afdata.OutputJson(data))
// {"api_key_secret":"***","created_at_epoch_ms":1738886400000,"file_size_bytes":5242880,"user_id":123}

// YAML (keys stripped, values formatted, secrets redacted)
fmt.Println(afdata.OutputYaml(data))
// ---
// api_key: "***"
// created_at: "2025-02-07T00:00:00.000Z"
// file_size: "5.0MB"
// user_id: 123

// Plain logfmt (keys stripped, values formatted, secrets redacted)
fmt.Println(afdata.OutputPlain(data))
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
import afdata "github.com/cmnspore/agent-first-data/go"

size, _ := afdata.ParseSize("10M")   // 10485760
size, _ = afdata.ParseSize("1.5K")   // 1536
size, _ = afdata.ParseSize("512")    // 512
```

## Usage Examples

### Example 1: REST API

```go
import afdata "github.com/cmnspore/agent-first-data/go"

func getUserHandler(w http.ResponseWriter, r *http.Request) {
    response := afdata.BuildJsonOk(
        map[string]any{"user_id": 123, "name": "alice"},
        map[string]any{"duration_ms": 150, "source": "db"},
    )
    // API returns raw JSON — no output processing, no key stripping
    json.NewEncoder(w).Encode(response)
}
```

### Example 2: CLI Tool (Complete Lifecycle)

```go
import afdata "github.com/cmnspore/agent-first-data/go"

func main() {
    // 1. Startup
    startup := afdata.BuildJson(
        "log",
        map[string]any{
            "event":  "startup",
            "config": map[string]any{"api_key_secret": "sk-sensitive-key", "timeout_s": 30},
            "args":   map[string]any{"input_path": "data.json"},
            "env":    map[string]any{"RUST_LOG": "info"},
        },
        nil,
    )
    fmt.Println(afdata.OutputYaml(startup))
    // ---
    // code: "log"
    // event: "startup"
    // args:
    //   input_path: "data.json"
    // config:
    //   api_key: "***"
    //   timeout: "30s"
    // env:
    //   RUST_LOG: "info"

    // 2. Progress
    progress := afdata.BuildJson(
        "progress",
        map[string]any{"current": 3, "total": 10, "message": "processing"},
        map[string]any{"duration_ms": 1500},
    )
    fmt.Println(afdata.OutputPlain(progress))
    // code=progress current=3 message=processing total=10 trace.duration=1.5s

    // 3. Result
    result := afdata.BuildJsonOk(
        map[string]any{
            "records_processed":    10,
            "file_size_bytes":      5242880,
            "created_at_epoch_ms":  int64(1738886400000),
        },
        map[string]any{"duration_ms": 3500, "source": "file"},
    )
    fmt.Println(afdata.OutputYaml(result))
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
import afdata "github.com/cmnspore/agent-first-data/go"

func processRequest() {
    result := afdata.BuildJsonOk(
        map[string]any{"status": "success"},
        map[string]any{"duration_ms": 250, "api_key_secret": "sk-123"},
    )

    // Print JSONL to stdout (secrets redacted, one JSON object per line)
    fmt.Println(afdata.OutputJson(result))
    // {"code":"ok","result":{"status":"success"},"trace":{"api_key_secret":"***","duration_ms":250}}
}
```

## Complete Suffix Example

```go
import afdata "github.com/cmnspore/agent-first-data/go"

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
fmt.Println(afdata.OutputYaml(data))
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
fmt.Println(afdata.OutputPlain(data))
// api_key=*** cache_ttl=3600s count=42 created_at=2025-02-07T00:00:00.000Z file_size=5.0MB payment=50000000msats price=$99.99 request_timeout=5.0s success_rate=95.5% user_name=alice
```

## AFDATA Logging

AFDATA-compliant structured logging via Go's `log/slog`. Every log line is formatted using the library's own `OutputJson`/`OutputPlain`/`OutputYaml` functions. Span fields are carried via `WithAttrs` / context, automatically flattened into each log line.

### API

```go
import afdata "github.com/cmnspore/agent-first-data/go"

// Convenience initializers — set up the default slog logger with AFDATA output to stdout
afdata.InitJson()    // Single-line JSONL (secrets redacted, original keys)
afdata.InitPlain()   // Single-line logfmt (keys stripped, values formatted)
afdata.InitYaml()    // Multi-line YAML (keys stripped, values formatted)

// Low-level — create a handler for custom logger stacks
afdata.NewAfdataHandler(w io.Writer, format LogFormat) *AfdataHandler  // implements slog.Handler
afdata.FormatJson | afdata.FormatPlain | afdata.FormatYaml

// Context-based spans for concurrent code
afdata.WithSpan(ctx context.Context, fields map[string]any) context.Context
afdata.LoggerFromContext(ctx context.Context) *slog.Logger

// Global span (non-concurrent, uses slog.SetDefault)
afdata.Span(fields map[string]any, fn func())
```

### Setup

```go
import afdata "github.com/cmnspore/agent-first-data/go"

// JSON output for production (one JSONL line per event, secrets redacted)
afdata.InitJson()

// Plain logfmt for development (keys stripped, values formatted)
afdata.InitPlain()

// YAML for detailed inspection (multi-line, keys stripped, values formatted)
afdata.InitYaml()
```

### Log Output

Standard `slog` functions work unchanged. Output format depends on the init function used.

```go
slog.Info("Server started")
// JSON:  {"timestamp_epoch_ms":1739000000000,"message":"Server started","code":"info"}
// Plain: code=info message="Server started" timestamp_epoch_ms=1739000000000
// YAML:  ---
//        code: "info"
//        message: "Server started"
//        timestamp_epoch_ms: 1739000000000

slog.Warn("DNS lookup failed", "error", err, "domain", domain)
// JSON:  {"timestamp_epoch_ms":...,"message":"DNS lookup failed","domain":"example.com","error":"timeout","code":"warn"}
// Plain: code=warn domain=example.com error=timeout message="DNS lookup failed" ...
```

### Span Support (WithAttrs)

Create child loggers with span-level fields. All log events from the child include the span fields.

```go
reqLogger := slog.Default().With("request_id", uuid)
reqLogger.Info("Processing")
// {"timestamp_epoch_ms":...,"message":"Processing","request_id":"abc-123","code":"info"}

reqLogger.Warn("Not found", "path", "/users/42")
// {"timestamp_epoch_ms":...,"message":"Not found","request_id":"abc-123","path":"/users/42","code":"warn"}
```

### Context-Based Spans

For concurrent code (goroutines), use context-based spans:

```go
ctx := afdata.WithSpan(ctx, map[string]any{"request_id": uuid})

// In handler or goroutine
logger := afdata.LoggerFromContext(ctx)
logger.Info("Handling request", "method", "GET")
// {"timestamp_epoch_ms":...,"message":"Handling request","request_id":"abc-123","method":"GET","code":"info"}
```

### Custom Code Override

The `code` field defaults to the log level. Override with an explicit field:

```go
slog.Info("Server ready", "code", "log", "event", "startup")
// {"timestamp_epoch_ms":...,"message":"Server ready","code":"log","event":"startup"}
```

### Output Fields

Every log line contains:

| Field | Type | Description |
|:------|:-----|:------------|
| `timestamp_epoch_ms` | number | Unix milliseconds |
| `message` | string | Log message |
| `code` | string | Level (trace/debug/info/warn/error) or explicit override |
| *span fields* | any | From `WithAttrs` / `WithSpan` |
| *event fields* | any | From `slog` call arguments |

### Log Output Formats

All three formats use the library's own output functions, so AFDATA suffix processing applies to log fields too:

| Format | Function | Keys | Values | Use case |
|:-------|:---------|:-----|:-------|:---------|
| **JSON** | `InitJson` | original (with suffix) | raw | production, log aggregation |
| **Plain** | `InitPlain` | stripped | formatted | development, compact scanning |
| **YAML** | `InitYaml` | stripped | formatted | debugging, detailed inspection |

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

## Repository

This package is part of the [agent-first-data](https://github.com/cmnspore/agent-first-data) repository, which also contains:

- **`spec/`** — Full AFDATA specification with suffix definitions, protocol format rules, and cross-language test fixtures
- **`skills/`** — AI coding agent skill for working with AFDATA conventions

To run tests, clone the full repository (tests use shared cross-language fixtures from `spec/fixtures/`):

```bash
git clone https://github.com/cmnspore/agent-first-data
cd agent-first-data/go
go test ./...
```

## License

MIT
