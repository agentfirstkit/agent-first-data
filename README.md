# Agent-First Data

**The field name is the schema.** Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact — no external schema needed.

Agent-First Data (AFDATA) is a convention for self-describing structured data:

1. **Naming** — Encode units and semantics in field name suffixes (`_ms`, `_bytes`, `_secret`, ...)
2. **Output** — Three formats (JSON/YAML/Plain) with automatic key stripping, value formatting, and secret redaction
3. **Protocol** — Optional structured templates (`ok`, `error`, `log`) with `trace` for execution context
4. **Logging** — AFDATA-compliant structured logging with span support (per-language integration)
5. **Channel discipline** — machine-readable protocol/log events use `stdout` only; `stderr` is not a protocol channel

See the full [specification](spec/agent-first-data.md) and the [agent skill](skills/agent-first-data.md) for AI-assisted development.

## Installation

```bash
cargo add agent-first-data        # Rust
pip install agent-first-data       # Python
npm install agent-first-data       # TypeScript
go get github.com/cmnspore/agent-first-data/go  # Go
```

## Quick Example

A backup tool invoked from the CLI — flags, env vars, and config all use the same suffixes:

```bash
API_KEY_SECRET=sk-1234 cloudback --timeout-s 30 --max-file-size-bytes 10737418240 --log startup /data/backup.tar.gz
```

The tool reads env vars (`UPPER_SNAKE_CASE`), flags (`--kebab-case`), and config (`snake_case`) — all with AFDATA suffixes. When `startup` logging is enabled, it emits a startup log event. Three output formats, same data:

**JSON** (secrets redacted, original keys, for machines):
```json
{"code":"log","event":"startup","args":{"input_path":"/data/backup.tar.gz"},"config":{"max_file_size_bytes":10737418240,"timeout_s":30},"env":{"API_KEY_SECRET":"***"}}
```

**YAML** (suffixes stripped from keys, values formatted, for humans):
```yaml
---
code: "log"
event: "startup"
args:
  input_path: "/data/backup.tar.gz"
config:
  max_file_size: "10.0GB"
  timeout: "30s"
env:
  API_KEY: "***"
```

**Plain** (single-line logfmt, keys stripped, for log scanning):
```
args.input_path=/data/backup.tar.gz code=log event=startup config.max_file_size=10.0GB config.timeout=30s env.API_KEY=***
```

`--timeout-s` → `timeout_s` → `timeout: 30s`. `API_KEY_SECRET` → `API_KEY: "***"`. Same suffixes flow through env vars, CLI flags, JSON, and formatted output — the suffix is the schema.

CLI logging flags:

```bash
--log startup,request,progress,retry,redirect
--verbose   # shorthand for all log categories
```

## API (13 functions + 2 types, same across all languages)

| Function / Type | Returns | Description |
|:----------------|:--------|:------------|
| `build_json_ok` | JSON | `{code: "ok", result, trace?}` |
| `build_json_error` | JSON | `{code: "error", error, hint?, trace?}` |
| `build_json` | JSON | `{code: "<custom>", ...fields, trace?}` |
| `output_json` | String | Single-line JSON, secrets redacted |
| `output_json_with` | String | Single-line JSON with explicit redaction policy |
| `output_yaml` | String | Multi-line YAML, keys stripped, values formatted |
| `output_plain` | String | Single-line logfmt, keys stripped, values formatted |
| `internal_redact_secrets` | void | Redact `_secret` fields in-place |
| `parse_size` | int | Parse `"10M"` → bytes; invalid/overflow returns language-specific invalid result |
| `OutputFormat` | type | `"json"` / `"yaml"` / `"plain"` enum/type |
| `RedactionPolicy` | type | `RedactionTraceOnly` / `RedactionNone` |
| `cli_parse_output` | OutputFormat | Parse `--output` flag; error on unknown value |
| `cli_parse_log_filters` | String[] | Normalize `--log` entries: trim, lowercase, dedup, remove empty |
| `cli_output` | String | Dispatch to `output_json` / `output_yaml` / `output_plain` |
| `build_cli_error` | JSON | `{code:"error", error_code:"invalid_request", hint?, retryable:false, trace:{duration_ms:0}}` |

## AFDATA Logging

AFDATA-compliant structured logging. Log output is formatted using the library's own `output_json`/`output_plain`/`output_yaml` functions — same suffix processing, key stripping, and secret redaction as the core output API. Span fields are automatically flattened into each event line, solving concurrent request interleaving.

Each language integrates with its native logging ecosystem:

| Language | Integration | Span Mechanism | Output Formats |
|:---------|:------------|:---------------|:---------------|
| **Rust** | `tracing` Layer (feature `"tracing"`) | tracing spans | `init_json` / `init_plain` / `init_yaml` |
| **Go** | `log/slog` Handler | `WithAttrs` / `WithSpan(ctx)` | `InitJson` / `InitPlain` / `InitYaml` |
| **Python** | `logging` Handler | `contextvars` | `init_logging_json` / `init_logging_plain` / `init_logging_yaml` |
| **TypeScript** | Built-in logger | `AsyncLocalStorage` | `initJson` / `initPlain` / `initYaml` |

Minimum envelope contract across languages:
- Required fields: `timestamp_epoch_ms`, `message`, `code`
- Optional fields: `target` and tool-specific structured fields

**JSON output** (production — secrets redacted, original keys):
```json
{"timestamp_epoch_ms":1739000000000,"message":"Processing","request_id":"abc-123","code":"info"}
```

**Plain output** (development — keys stripped, values formatted):
```
code=info message=Processing request_id=abc-123 timestamp_epoch_ms=1739000000000
```

**Rust:**
```rust
use agent_first_data::afdata_tracing;
afdata_tracing::init_json(EnvFilter::new("info"));   // or init_plain / init_yaml

let span = info_span!("request", request_id = %uuid);
let _guard = span.enter();
info!("Processing");
```

**Go:**
```go
afdata.InitJson()   // or InitPlain / InitYaml

ctx := afdata.WithSpan(ctx, map[string]any{"request_id": uuid})
afdata.LoggerFromContext(ctx).Info("Processing")
```

**Python:**
```python
from agent_first_data import init_logging_json, span  # or init_logging_plain / init_logging_yaml

init_logging_json("INFO")
with span(request_id=uuid):
    logger.info("Processing")
```

**TypeScript:**
```typescript
import { log, span, initJson } from "agent-first-data";  // or initPlain / initYaml

await span({ request_id: uuid }, async () => {
  log.info("Processing");
});
```

## Supported Suffixes

| Category | Suffixes | Example |
|:---------|:---------|:--------|
| **Duration** | `_ns`, `_us`, `_ms`, `_s`, `_minutes`, `_hours`, `_days` | `latency_ms: 1280` → `latency: 1.28s` |
| **Timestamps** | `_epoch_ns`, `_epoch_ms`, `_epoch_s`, `_rfc3339` | `created_at_epoch_ms: 1738886400000` → `created_at: 2025-02-07T00:00:00.000Z` |
| **Size** | `_bytes` (output), `_size` (config input) | `file_size_bytes: 5242880` → `file_size: 5.0MB` |
| **Currency** | `_msats`, `_sats`, `_btc`, `_usd_cents`, `_eur_cents`, `_jpy`, `_{code}_cents` | `price_usd_cents: 999` → `price: $9.99` |
| **Other** | `_percent`, `_secret` | `cpu_percent: 85` → `cpu: 85%` |

## Language Documentation

- **[Rust](rust/)** — Full API reference, examples, and AFDATA tracing
- **[Go](go/)** — Full API reference, examples, and AFDATA logging
- **[Python](python/)** — Full API reference, examples, and AFDATA logging
- **[TypeScript](typescript/)** — Full API reference, examples, and AFDATA logging

## License

MIT
