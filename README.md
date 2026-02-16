# Agent-First Data

**The field name is the schema.** Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact — no external schema needed.

Agent-First Data (AFD) is a convention for self-describing structured data:

1. **Naming** — Encode units and semantics in field name suffixes (`_ms`, `_bytes`, `_secret`, ...)
2. **Output** — Three formats (JSON/YAML/Plain) with automatic key stripping, value formatting, and secret redaction
3. **Protocol** — Optional structured templates (`ok`, `error`, `startup`) with `trace` for execution context

See the full [specification](spec/agent-first-data.md).

## Installation

```bash
cargo add agent-first-data        # Rust
pip install agent-first-data       # Python
npm install agent-first-data       # TypeScript
go get github.com/cmnspore/agent-first-data/go  # Go
```

## Quick Example

Input JSON:
```json
{
  "created_at_epoch_ms": 1738886400000,
  "file_size_bytes": 5242880,
  "cache_ttl_s": 3600,
  "api_key_secret": "sk-1234567890abcdef",
  "user_name": "alice",
  "count": 42
}
```

**JSON** (single-line, secrets redacted, original keys):
```
{"api_key_secret":"***","cache_ttl_s":3600,"count":42,"created_at_epoch_ms":1738886400000,"file_size_bytes":5242880,"user_name":"alice"}
```

**YAML** (keys stripped, values formatted):
```yaml
---
api_key: "***"
cache_ttl: "3600s"
count: 42
created_at: "2025-02-07T00:00:00.000Z"
file_size: "5.0MB"
user_name: "alice"
```

**Plain** (single-line logfmt, keys stripped):
```
api_key=*** cache_ttl=3600s count=42 created_at=2025-02-07T00:00:00.000Z file_size=5.0MB user_name=alice
```

## API (9 functions, same across all languages)

| Function | Returns | Description |
|:---------|:--------|:------------|
| `build_json_startup` | JSON | `{code: "startup", config, args, env}` |
| `build_json_ok` | JSON | `{code: "ok", result, trace?}` |
| `build_json_error` | JSON | `{code: "error", error, trace?}` |
| `build_json` | JSON | `{code: "<custom>", ...fields, trace?}` |
| `output_json` | String | Single-line JSON, secrets redacted |
| `output_yaml` | String | Multi-line YAML, keys stripped, values formatted |
| `output_plain` | String | Single-line logfmt, keys stripped, values formatted |
| `internal_redact_secrets` | void | Redact `_secret` fields in-place |
| `parse_size` | int | Parse `"10M"` → bytes |

## Supported Suffixes

| Category | Suffixes | Example |
|:---------|:---------|:--------|
| **Duration** | `_ns`, `_us`, `_ms`, `_s`, `_minutes`, `_hours`, `_days` | `latency_ms: 1280` → `latency: 1.28s` |
| **Timestamps** | `_epoch_ns`, `_epoch_ms`, `_epoch_s`, `_rfc3339` | `created_at_epoch_ms: 1738886400000` → `created_at: 2025-02-07T00:00:00.000Z` |
| **Size** | `_bytes` (output), `_size` (config input) | `file_size_bytes: 5242880` → `file_size: 5.0MB` |
| **Currency** | `_msats`, `_sats`, `_btc`, `_usd_cents`, `_eur_cents`, `_jpy`, `_{code}_cents` | `price_usd_cents: 999` → `price: $9.99` |
| **Other** | `_percent`, `_secret` | `cpu_percent: 85` → `cpu: 85%` |

## Language Documentation

- **[Rust](rust/)** — Full API reference and examples
- **[Go](go/)** — Full API reference and examples
- **[Python](python/)** — Full API reference and examples
- **[TypeScript](typescript/)** — Full API reference and examples

## License

MIT
