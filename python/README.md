# agent-first-data

**Agent-First Data (AFD)** — Suffix-driven output formatting and protocol templates for AI agents.

The field name is the schema. Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact, no external schema needed.

## Installation

```bash
pip install agent-first-data
```

## Quick Example

A backup tool invoked from the CLI — flags, env vars, and config all use the same suffixes:

```bash
API_KEY_SECRET=sk-1234 cloudback --timeout-s 30 --max-file-size-bytes 10737418240 /data/backup.tar.gz
```

The tool reads env vars, flags, and config — all with AFD suffixes — and emits a startup message:

```python
from agent_first_data import *
import os

startup = build_json_startup(
    {"timeout_s": 30, "max_file_size_bytes": 10737418240},
    {"input_path": "/data/backup.tar.gz"},
    {"API_KEY_SECRET": os.environ.get("API_KEY_SECRET")},
)
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

Total: **9 public APIs** + **AFD logging** (4 protocol builders + 3 output functions + 1 internal + 1 utility)

### Protocol Builders (returns dict)

Build AFD protocol structures. Return dict objects for API responses.

```python
# Startup (configuration)
build_json_startup(config: Any, args: Any, env: Any) -> dict

# Success (result)
build_json_ok(result: Any, trace: Any = None) -> dict

# Error (simple message)
build_json_error(message: str, trace: Any = None) -> dict

# Generic (any code + fields)
build_json(code: str, fields: Any, trace: Any = None) -> dict
```

**Use case:** API responses (frameworks like FastAPI automatically serialize)

**Example:**
```python
from agent_first_data import *

# Startup
startup = build_json_startup(
    {"api_key_secret": "sk-123", "timeout_s": 30},
    {"config_path": "config.yml"},
    {"RUST_LOG": "info"},
)

# Success (always include trace)
response = build_json_ok(
    {"user_id": 123},
    trace={"duration_ms": 150, "source": "db"},
)

# Error
err = build_json_error("user not found", trace={"duration_ms": 5})

# Specific error code
not_found = build_json(
    "not_found",
    {"resource": "user", "id": 123},
    trace={"duration_ms": 8},
)
```

### CLI/Log Output (returns str)

Format values for CLI output and logs. **All formats redact `_secret` fields.** YAML and Plain also strip suffixes from keys and format values for human readability.

```python
output_json(value: Any) -> str   # Single-line JSON, original keys, for programs/logs
output_yaml(value: Any) -> str   # Multi-line YAML, keys stripped, values formatted
output_plain(value: Any) -> str  # Single-line logfmt, keys stripped, values formatted
```

**Example:**
```python
from agent_first_data import *

data = {
    "user_id": 123,
    "api_key_secret": "sk-1234567890abcdef",
    "created_at_epoch_ms": 1738886400000,
    "file_size_bytes": 5242880,
}

# JSON (secrets redacted, original keys, raw values)
print(output_json(data))
# {"api_key_secret":"***","created_at_epoch_ms":1738886400000,"file_size_bytes":5242880,"user_id":123}

# YAML (keys stripped, values formatted, secrets redacted)
print(output_yaml(data))
# ---
# api_key: "***"
# created_at: "2025-02-07T00:00:00.000Z"
# file_size: "5.0MB"
# user_id: 123

# Plain logfmt (keys stripped, values formatted, secrets redacted)
print(output_plain(data))
# api_key=*** created_at=2025-02-07T00:00:00.000Z file_size=5.0MB user_id=123
```

### Internal Tools

```python
internal_redact_secrets(value: Any) -> None  # Manually redact secrets in-place
```

Most users don't need this. Output functions automatically protect secrets.

### Utility Functions

```python
parse_size(s: str) -> int | None  # Parse "10M" → bytes
```

**Example:**
```python
from agent_first_data import *

assert parse_size("10M") == 10485760
assert parse_size("1.5K") == 1536
assert parse_size("512") == 512
```

## Usage Examples

### Example 1: REST API

```python
from agent_first_data import *
from fastapi import FastAPI

app = FastAPI()

@app.get("/users/{user_id}")
async def get_user(user_id: int):
    response = build_json_ok(
        {"user_id": user_id, "name": "alice"},
        trace={"duration_ms": 150, "source": "db"},
    )
    # API returns raw JSON — no output processing, no key stripping
    return response
```

### Example 2: CLI Tool (Complete Lifecycle)

```python
from agent_first_data import *

# 1. Startup
startup = build_json_startup(
    {"api_key_secret": "sk-sensitive-key", "timeout_s": 30},
    {"input_path": "data.json"},
    {"RUST_LOG": "info"},
)
print(output_yaml(startup))
# ---
# code: "startup"
# args:
#   input_path: "data.json"
# config:
#   api_key: "***"
#   timeout: "30s"
# env:
#   RUST_LOG: "info"

# 2. Progress
progress = build_json(
    "progress",
    {"current": 3, "total": 10, "message": "processing"},
    trace={"duration_ms": 1500},
)
print(output_plain(progress))
# code=progress current=3 message=processing total=10 trace.duration=1.5s

# 3. Result
result = build_json_ok(
    {
        "records_processed": 10,
        "file_size_bytes": 5242880,
        "created_at_epoch_ms": 1738886400000,
    },
    trace={"duration_ms": 3500, "source": "file"},
)
print(output_yaml(result))
# ---
# code: "ok"
# result:
#   created_at: "2025-02-07T00:00:00.000Z"
#   file_size: "5.0MB"
#   records_processed: 10
# trace:
#   duration: "3.5s"
#   source: "file"
```

### Example 3: JSONL Output

```python
from agent_first_data import *

result = build_json_ok(
    {"status": "success"},
    trace={"duration_ms": 250, "api_key_secret": "sk-123"},
)

# Print JSONL to stdout (secrets redacted, one JSON object per line)
print(output_json(result))
# {"code":"ok","result":{"status":"success"},"trace":{"api_key_secret":"***","duration_ms":250}}
```

## Complete Suffix Example

```python
from agent_first_data import *

data = {
    "created_at_epoch_ms": 1738886400000,
    "request_timeout_ms": 5000,
    "cache_ttl_s": 3600,
    "file_size_bytes": 5242880,
    "payment_msats": 50000000,
    "price_usd_cents": 9999,
    "success_rate_percent": 95.5,
    "api_key_secret": "sk-1234567890abcdef",
    "user_name": "alice",
    "count": 42,
}

# YAML output (keys stripped, values formatted, secrets redacted)
print(output_yaml(data))
# ---
# api_key: "***"
# cache_ttl: "3600s"
# count: 42
# created_at: "2025-02-07T00:00:00.000Z"
# file_size: "5.0MB"
# payment: "50000000msats"
# price: "$99.99"
# request_timeout: "5.0s"
# success_rate: "95.5%"
# user_name: "alice"

# Plain logfmt output (same transformations, single line)
print(output_plain(data))
# api_key=*** cache_ttl=3600s count=42 created_at=2025-02-07T00:00:00.000Z file_size=5.0MB payment=50000000msats price=$99.99 request_timeout=5.0s success_rate=95.5% user_name=alice
```

## AFD Logging

AFD-compliant structured logging via Python's `logging` module. Every log line is formatted using the library's own `output_json`/`output_plain`/`output_yaml` functions. Span fields are carried via `contextvars` (async-safe), automatically flattened into each log line.

### API

```python
from agent_first_data import init_logging_json, init_logging_plain, init_logging_yaml
from agent_first_data.afd_logging import AfdHandler, get_logger, span

# Convenience initializers — set up the root logger with AFD output to stdout
init_logging_json(level="INFO")    # Single-line JSONL (secrets redacted, original keys)
init_logging_plain(level="INFO")   # Single-line logfmt (keys stripped, values formatted)
init_logging_yaml(level="INFO")    # Multi-line YAML (keys stripped, values formatted)

# Low-level — create a handler for custom logger stacks
AfdHandler(format="json")  # format: "json" | "plain" | "yaml"

# Logger with default fields (returns logging.LoggerAdapter)
get_logger(name, **fields)

# Span context manager — adds fields to all log events within the block
span(**fields)
```

### Setup

```python
from agent_first_data import init_logging_json, init_logging_plain, init_logging_yaml

# JSON output for production (one JSONL line per event, secrets redacted)
init_logging_json("INFO")

# Plain logfmt for development (keys stripped, values formatted)
init_logging_plain("DEBUG")

# YAML for detailed inspection (multi-line, keys stripped, values formatted)
init_logging_yaml("DEBUG")
```

### Log Output

Standard `logging` calls work unchanged. Output format depends on the init function used.

```python
import logging
logger = logging.getLogger("myapp")

logger.info("Server started")
# JSON:  {"timestamp_epoch_ms":1739000000000,"message":"Server started","target":"myapp","code":"info"}
# Plain: code=info message="Server started" target=myapp timestamp_epoch_ms=1739000000000
# YAML:  ---
#        code: "info"
#        message: "Server started"
#        target: "myapp"
#        timestamp_epoch_ms: 1739000000000

logger.warning("DNS lookup failed")
# JSON:  {"timestamp_epoch_ms":...,"message":"DNS lookup failed","target":"myapp","code":"warn"}
```

### Span Support

Use the `span` context manager to add fields to all log events within the block. Spans nest and work with both sync and async code.

```python
from agent_first_data import span

with span(request_id="abc-123"):
    logger.info("Processing")
    # {"timestamp_epoch_ms":...,"message":"Processing","target":"myapp","request_id":"abc-123","code":"info"}

    with span(step="validate"):
        logger.info("Validating input")
        # {"timestamp_epoch_ms":...,"message":"Validating input","target":"myapp","request_id":"abc-123","step":"validate","code":"info"}
```

### Logger with Default Fields

Use `get_logger` for per-component fields that appear on every log line:

```python
from agent_first_data import get_logger

logger = get_logger("myapp.auth", component="auth")
logger.info("Token verified")
# {"timestamp_epoch_ms":...,"message":"Token verified","target":"myapp.auth","component":"auth","code":"info"}
```

### Custom Code Override

The `code` field defaults to the log level. Override with an explicit field:

```python
from agent_first_data import get_logger

logger = get_logger("myapp")
logger.info("Server ready", extra={"code": "startup"})
# {"timestamp_epoch_ms":...,"message":"Server ready","target":"myapp","code":"startup"}
```

### Output Fields

Every log line contains:

| Field | Type | Description |
|:------|:-----|:------------|
| `timestamp_epoch_ms` | number | Unix milliseconds |
| `message` | string | Log message |
| `target` | string | Logger name |
| `code` | string | Level (debug/info/warn/error) or explicit override |
| *span fields* | any | From `span()` context manager |
| *event fields* | any | From `extra=` or `get_logger` fields |

### Log Output Formats

All three formats use the library's own output functions, so AFD suffix processing applies to log fields too:

| Format | Function | Keys | Values | Use case |
|:-------|:---------|:-----|:-------|:---------|
| **JSON** | `init_logging_json` | original (with suffix) | raw | production, log aggregation |
| **Plain** | `init_logging_plain` | stripped | formatted | development, compact scanning |
| **YAML** | `init_logging_yaml` | stripped | formatted | debugging, detailed inspection |

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
