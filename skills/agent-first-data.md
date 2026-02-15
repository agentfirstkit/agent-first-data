---
name: agent-first-data
description: Apply Agent-First Data naming and output conventions when writing structured data, configs, logs, API responses, or CLI output in any language.
disable-model-invocation: true
allowed-tools: Bash, Read, Edit, Write, Glob, Grep
---

# Agent-First Data

Two independent conventions that work together:

1. **Naming** — encode units and semantics in field names so agents parse structured data without external schemas
2. **Output** — a JSONL protocol with lifecycle phases and multi-format rendering

You can adopt either one alone.

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
| `_epoch_ms` | milliseconds since Unix epoch | `tasted_epoch_ms: 1707868800000` |
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

All output (logs, CLI, API responses) MUST redact `_secret` fields to `"***"`. Case-insensitive matching.

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

## Part 2: Output Protocol

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
```

Include `trace` for execution context: duration, token counts, cost, data source.

### Output formats

`--output json|yaml|plain`

- **json** — canonical, lossless, JSONL to stdout
- **yaml** — `---` separated, strings always quoted, values as-is
- **plain** — suffix-driven human formatting (lossy):
  - `_ms` >= 1000 → seconds: `1280` → `1.28s`
  - `_epoch_ms` → RFC 3339: `2026-01-31T16:00:00.000Z` (negative = pre-1970)
  - `_bytes` → human-readable: `456789` → `446.1KB`, `-5242880` → `-5.0MB`
  - `_percent` → append `%`: `85` → `85%`
  - `_usd_cents` → dollars: `999` → `$9.99` (negative falls through)
  - `_secret` → `***`
  - `_bytes`/`_epoch_*` require integer; `_usd_cents`/`_jpy` require non-negative integer; duration/`_percent`/Bitcoin accept any number; float/bool fall through

YAML and plain sort keys by UTF-16 code unit order (JCS, RFC 8785). For ASCII keys this equals byte-order sorting.

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

| Function | What it does |
|:---------|:-------------|
| `to_plain` / `to_yaml` | Render JSON value as plain or YAML string |
| `redact_secrets` | Walk tree, replace `_secret` string values with `"***"` |
| `parse_size` | Parse `_size` string to bytes: `"10M"` → `10485760` |
| `ok` / `ok_trace` | Build `{"code": "ok", "result": ...}` |
| `error` / `error_trace` | Build `{"code": "error", "error": ...}` |
| `startup` | Build `{"code": "startup", "config": ..., "args": ..., "env": ...}` |
| `status` | Build `{"code": "<custom>", ...fields}` |
| `OutputFormat` | Enum: `json` / `yaml` / `plain` with `.format(value)` |

### Rust

```rust
use agent_first_data::{to_plain, to_yaml, redact_secrets, parse_size, ok, ok_trace, error, error_trace, startup, status, OutputFormat};
```

### Python

```python
from agent_first_data import to_plain, to_yaml, redact_secrets, parse_size, ok, ok_trace, error, error_trace, startup, status, OutputFormat
```

### TypeScript

```typescript
import { toPlain, toYaml, redactSecrets, parseSize, ok, okTrace, error, errorTrace, startup, status, OutputFormat } from "agent-first-data";
```

### Go

```go
import afd "github.com/cmnspore/agent-first-data/go"

afd.ToPlain(value)
afd.RedactSecrets(value)
afd.ParseSize("10M")
afd.Ok(result)
```

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
