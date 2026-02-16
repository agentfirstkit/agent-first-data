---
name: agent-first-data
description: Apply Agent-First Data naming and output conventions when writing structured data, configs, logs, API responses, or CLI output in any language.
disable-model-invocation: true
allowed-tools: Bash, Read, Edit, Write, Glob, Grep
---

# Agent-First Data

Three parts:

1. **Naming** — encode units and semantics in field names so agents parse structured data without external schemas
2. **Output** — suffix-driven formatting with key stripping, value formatting, and automatic secret redaction
3. **Protocol** — optional JSONL protocol with `code` (required) and `trace` (recommended)

---

## Part 1: Naming Convention

The field name is the schema. Always encode units and semantics in the field name.

### Duration

| Suffix | Unit | Example |
|:-------|:-----|:--------|
| `_ns` | nanoseconds | `gc_pause_ns: 450000` |
| `_us` | microseconds | `query_us: 830` |
| `_ms` | milliseconds | `latency_ms: 142` |
| `_s` | seconds | `dns_ttl_s: 3600` |
| `_minutes` | minutes | `session_timeout_minutes: 30` |
| `_hours` | hours | `token_validity_hours: 24` |
| `_days` | days | `cert_validity_days: 365` |

### Timestamps

| Suffix | Format | Example |
|:-------|:-------|:--------|
| `_epoch_ms` | milliseconds since Unix epoch | `created_at_epoch_ms: 1707868800000` |
| `_epoch_s` | seconds since Unix epoch | `cached_epoch_s: 1707868800` |
| `_epoch_ns` | nanoseconds since Unix epoch | `created_epoch_ns: 1707868800000000000` |
| `_rfc3339` | RFC 3339 string | `expires_rfc3339: "2026-02-14T10:30:00Z"` |

### Size

| Suffix | Example |
|:-------|:--------|
| `_bytes` | `payload_bytes: 456789` (always numeric) |
| `_size` | `buffer_size: "10M"` (config files only, human-readable) |

`_size` parsing rules (binary): `B`=1, `K`=1024, `M`=1024², `G`=1024³, `T`=1024⁴. Case-insensitive.

`parse_size("10M")` → `10485760`. Returns null for invalid or negative input.

### Percentage

| Suffix | Example |
|:-------|:--------|
| `_percent` | `cpu_percent: 85` |

### Currency

Bitcoin:

| Suffix | Example |
|:-------|:--------|
| `_msats` | `balance_msats: 97900` |
| `_sats` | `withdrawn_sats: 1234` |
| `_btc` | `reserve_btc: 0.5` |

Fiat — `_{iso4217}_cents` for currencies with 1/100 subdivision, `_{iso4217}` for currencies without:

| Suffix | Example |
|:-------|:--------|
| `_usd_cents` | `price_usd_cents: 999` |
| `_eur_cents` | `price_eur_cents: 850` |
| `_jpy` | `price_jpy: 1500` |
| `_usdt_cents` | `deposit_usdt_cents: 1000` |

### Sensitive

| Suffix | Handling | Example |
|:-------|:---------|:--------|
| `_secret` | redact to `***` | `api_key_secret: "sk-or-v1-abc..."` |

All CLI output formats (JSON, YAML, Plain) automatically redact `_secret` fields. Matching recognizes `_secret` and `_SECRET` only — no mixed case.

### Environment variables

Same suffixes, `UPPER_SNAKE_CASE`:

```
DATABASE_URL_SECRET=postgres://user:pass@host/db
CACHE_TTL_S=3600
TOKEN_VALIDITY_HOURS=24
```

### No suffix needed

Fields whose meaning is obvious: `callback_url`, `redb_path`, `proof_count`, `search_enabled`, `method`, `domain`, `model`.

### Common mistakes

| Bad | Good | Why |
|:----|:-----|:----|
| `timeout: 30` | `timeout_s: 30` | 30 what? seconds? ms? |
| `timestamp: 1707868800` | `cached_epoch_s: 1707868800` | what unit? what event? |
| `size: 456789` | `payload_bytes: 456789` | bytes? KB? |
| `price: 999` | `price_usd_cents: 999` | what currency? what unit? |
| `latency: 142` | `latency_ms: 142` | seconds? milliseconds? |
| `api_key: "sk-..."` | `api_key_secret: "sk-..."` | won't be auto-redacted |
| `cpu: 85` | `cpu_percent: 85` | 85 what? |
| `buffer: "10M"` | `buffer_size: "10M"` | only `_size` gets parsed |

---

## Part 2: Output Processing

Three output formats. YAML and Plain apply key stripping + value formatting.

### Formats

- **JSON** — single-line, original keys, raw values, no sorting (machine-readable), secrets redacted
- **YAML** — multi-line, keys stripped, values formatted, secrets redacted
- **Plain** — single-line logfmt, keys stripped, values formatted, secrets redacted

### Key stripping (YAML and Plain)

Remove recognized suffix from key. Longest match first, exact lowercase or uppercase only:

1. `_epoch_ms`, `_epoch_s`, `_epoch_ns`
2. `_usd_cents`, `_eur_cents`, `_{code}_cents`
3. `_rfc3339`, `_minutes`, `_hours`, `_days`
4. `_msats`, `_sats`, `_bytes`, `_percent`, `_secret`
5. `_btc`, `_jpy`, `_ns`, `_us`, `_ms`, `_s`

`_size` is NOT stripped (pass through). If two keys collide after stripping, both revert to original key AND raw value (no formatting).

### Value formatting (YAML and Plain)

- `_ms` < 1000 → `{n}ms`; ≥ 1000 → seconds (`1280` → `1.28s`, `5000` → `5.0s`)
- `_s`, `_ns`, `_us` → append unit (`3600s`, `450000ns`, `830μs`)
- `_minutes`, `_hours`, `_days` → append unit (`30 minutes`)
- `_epoch_ms`/`_epoch_s`/`_epoch_ns` → RFC 3339 (negative = pre-1970)
- `_rfc3339` → pass through
- `_bytes` → human-readable (`456789` → `446.1KB`, `-5242880` → `-5.0MB`)
- `_size` → pass through
- `_percent` → append `%`
- `_msats` → `{n}msats`, `_sats` → `{n}sats`, `_btc` → `{n} BTC`
- `_usd_cents` → `$X.XX`, `_eur_cents` → `€X.XX`, `_jpy` → `¥X,XXX`, `_{code}_cents` → `X.XX CODE`
- `_secret` → `***`

**Type constraints**: `_bytes`/`_epoch_*` require integer. `_usd_cents`/`_eur_cents`/`_jpy`/`_{code}_cents` require non-negative integer. Duration/Bitcoin/`_percent` accept any number. Wrong type → raw value + original key.

### Plain logfmt details

- Nested keys use dot notation: `trace.duration=1.28s`
- Values with spaces are quoted: `message="uploading chunks"`
- Arrays comma-joined: `fields=email,age`
- Null → empty value: `RUST_LOG=`
- Sort by full dot path (JCS / UTF-16 code unit order)

### Key ordering

YAML and Plain sort keys (after stripping) by UTF-16 code unit order (JCS, RFC 8785). For ASCII keys this equals byte-order sorting.

---

## Part 3: Protocol Template (Optional)

Every output line carries a `code` field:

| `code` | When |
|:-------|:-----|
| `"startup"` | Program start — config, args, env |
| tool-defined | Status/progress (`"request"`, `"progress"`, `"sync"`, etc.) |
| `"ok"` | Success result |
| `"error"` | Error result |

### Templates

```json
{"code": "startup", "config": {...}, "args": {...}, "env": {...}}
{"code": "ok", "result": {...}, "trace": {"duration_ms": 12, "source": "redb"}}
{"code": "error", "error": "message", "trace": {"duration_ms": 3}}
{"code": "not_found", "resource": "user", "id": 123, "trace": {"duration_ms": 8}}
```

Always include `trace` for execution context: duration, token counts, cost, data source.

### Same structure, any transport

| Transport | Format |
|:----------|:-------|
| CLI stdout | JSONL |
| REST API | JSON body |
| MCP tool | JSON |
| SSE stream | JSONL |

All use `code` / `result` / `error` / `trace`.

---

## Using the Library

9 public APIs (same across all languages):

| Function | What it does |
|:---------|:-------------|
| `build_json_startup` | Build `{code: "startup", config, args, env}` |
| `build_json_ok` | Build `{code: "ok", result, trace?}` |
| `build_json_error` | Build `{code: "error", error, trace?}` |
| `build_json` | Build `{code: "<custom>", ...fields, trace?}` |
| `output_json` | Single-line JSON, secrets redacted, original keys |
| `output_yaml` | Multi-line YAML, keys stripped, values formatted |
| `output_plain` | Single-line logfmt, keys stripped, values formatted |
| `internal_redact_secrets` | Redact `_secret` fields in-place |
| `parse_size` | Parse `"10M"` → bytes |

### Rust

```rust
use agent_first_data::{build_json_startup, build_json_ok, build_json_error, build_json, output_json, output_yaml, output_plain, internal_redact_secrets, parse_size};
```

### Python

```python
from agent_first_data import build_json_startup, build_json_ok, build_json_error, build_json, output_json, output_yaml, output_plain, internal_redact_secrets, parse_size
```

### TypeScript

```typescript
import { buildJsonStartup, buildJsonOk, buildJsonError, buildJson, outputJson, outputYaml, outputPlain, internalRedactSecrets, parseSize } from "agent-first-data";
```

### Go

```go
import afd "github.com/cmnspore/agent-first-data/go"

afd.BuildJsonStartup(config, args, env)
afd.OutputPlain(value)
afd.ParseSize("10M")
```

## AFD Logging

Structured logging that outputs via the library's own `output_json`/`output_plain`/`output_yaml`. Each language integrates with its native logging ecosystem. All three formats apply the same suffix processing, key stripping, and secret redaction as the core output API.

### Init (pick one format per process)

| Format | Rust | Go | Python | TypeScript |
|:-------|:-----|:---|:-------|:-----------|
| **JSON** | `afd_tracing::init_json(filter)` | `afd.InitJson()` | `init_logging_json("INFO")` | `initJson()` |
| **Plain** | `afd_tracing::init_plain(filter)` | `afd.InitPlain()` | `init_logging_plain("INFO")` | `initPlain()` |
| **YAML** | `afd_tracing::init_yaml(filter)` | `afd.InitYaml()` | `init_logging_yaml("INFO")` | `initYaml()` |

Rust requires `cargo add agent-first-data --features tracing`.

### Spans (add fields to all log events in scope)

```rust
// Rust — tracing spans
let span = info_span!("request", request_id = %uuid);
let _guard = span.enter();
```

```go
// Go — context-based
ctx := afd.WithSpan(ctx, map[string]any{"request_id": uuid})
logger := afd.LoggerFromContext(ctx)
```

```python
# Python — contextvars
with span(request_id=uuid):
    logger.info("Processing")
```

```typescript
// TypeScript — AsyncLocalStorage
await span({ request_id: uuid }, async () => {
  log.info("Processing");
});
```

### Output fields

Every log line contains: `timestamp_epoch_ms`, `message`, `code` (defaults to log level, overridable), plus span fields and event fields.

## CLI Flags

CLI tools that use AFD should support an output format flag:

```
--output json|yaml|plain    # default is tool-defined (interactive → yaml, scripting/logging → json)
```

- Protocol output (`build_json_*` + `output_*`) follows `--output`
- Log format follows `--output` or a separate `--log-format` flag if independent control is needed
- Document the default format and available options in `--help`

## Review Checklist

When reviewing code that produces structured output:

1. Every numeric field with a unit has the correct suffix (`_ms`, `_bytes`, `_sats`, `_percent`, etc.)
2. Timestamps use `_epoch_ms` / `_epoch_s` / `_rfc3339` — never bare `timestamp: 1707868800`
3. Sensitive values end in `_secret` and are redacted in all output paths
4. API responses / CLI output use `code` / `result` / `error` / `trace` structure
5. Config files use the same suffixes as output
6. No unit-less ambiguous fields (`timeout: 30` — 30 what?)
7. Config size values use `_size` suffix (`buffer_size: "10M"`, not `buffer: "10M"`)
8. Environment variables follow `UPPER_SNAKE_CASE` with the same suffixes
9. Logging uses AFD init functions (`init_json`/`init_plain`/`init_yaml`) — not raw `println!`/`fmt.Println`/`console.log` for structured output
