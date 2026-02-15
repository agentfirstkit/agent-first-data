# Agent-First Data

Agent-First Data is two independent conventions that work together:

1. **Naming** — encode units and semantics in field names so agents parse structured data without external schemas
2. **Output** — a JSONL protocol with lifecycle phases and multi-format rendering

You can adopt either one alone. Using both together gives agents a complete, self-describing data interface.

---

# Part 1: Naming Convention

**The field name is the schema.**

An agent reading `latency_ms: 142` knows this is a duration in milliseconds. An agent reading `api_key_secret: "sk-..."` knows this value must be redacted. No lookup required.

This convention applies to all structured data: JSON, YAML, TOML, environment variables, config files, database columns, API responses, log fields.

## Design rules

1. If a numeric value has a unit, encode the unit in the field name suffix.
2. If a value is sensitive, end the field name with `_secret`.
3. If the meaning is obvious from the name alone, no suffix is needed.
4. Never rely on external metadata, companion fields, or documentation to convey what a field contains.

## Suffixes

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
| `_epoch_ns` | nanoseconds since Unix epoch | `created_epoch_ns: 1707868800000000000` |
| `_epoch_ms` | milliseconds since Unix epoch | `tasted_epoch_ms: 1707868800000` |
| `_epoch_s` | seconds since Unix epoch | `cached_epoch_s: 1707868800` |
| `_rfc3339` | RFC 3339 string | `expires_rfc3339: "2026-02-14T10:30:00Z"` |

> **Precision note**: `_epoch_ns` values near the current era (~1.7×10¹⁸) exceed JavaScript's safe integer range (2⁵³ ≈ 9×10¹⁵). JSON parsed by JavaScript will silently lose nanosecond precision. Use `BigInt` or a custom JSON parser when nanosecond accuracy matters.

### Size

| Suffix | Value type | Example |
|:-------|:-----------|:--------|
| `_bytes` | numeric (bytes) | `payload_bytes: 456789` |
| `_size` | human-readable string | `buffer_size: "10M"` |

`_bytes` is always a number. Used in output, API responses, and anywhere agents need to compute on the value.

`_size` is a human-readable string. Used **only in config files** where humans write values. Programs parse `_size` to bytes at load time. Never use `_size` in output — always use `_bytes`.

Parsing rules for `_size` (binary, consistent with nginx/systemd/docker):

| Unit | Multiplier |
|:-----|:-----------|
| `B` or bare number | 1 |
| `K` | 1024 |
| `M` | 1024^2 |
| `G` | 1024^3 |
| `T` | 1024^4 |

The library provides `parse_size` to convert `_size` strings to bytes:

```
parse_size("10M")   → 10485760
parse_size("1.5K")  → 1536
parse_size("512B")  → 512
parse_size("1024")  → 1024
parse_size("")      → null
parse_size("-10M")  → null
```

Case-insensitive unit letter. Trims whitespace. Returns null/None for invalid or negative input.

### Percentage

| Suffix | Unit | Example |
|:-------|:-----|:--------|
| `_percent` | percentage | `cpu_percent: 85` |

### Currency

Bitcoin:

| Suffix | Unit | Example |
|:-------|:-----|:--------|
| `_msats` | millisatoshis | `balance_msats: 97900` |
| `_sats` | satoshis | `withdrawn_sats: 1234` |
| `_btc` | bitcoin | `reserve_btc: 0.5` |

Fiat — `_{iso4217}_cents` for currencies with 1/100 subdivision, `_{iso4217}` for currencies without (JPY). Always integers:

| Suffix | Unit | Example |
|:-------|:-----|:--------|
| `_usd_cents` | US dollar cents | `price_usd_cents: 999` |
| `_eur_cents` | euro cents | `price_eur_cents: 850` |
| `_thb_cents` | Thai baht 1/100 | `fare_thb_cents: 15050` |
| `_jpy` | Japanese yen (no minor unit) | `price_jpy: 1500` |

Stablecoins pegged to fiat follow the same pattern as their peg:

| Suffix | Unit | Example |
|:-------|:-----|:--------|
| `_usdt_cents` | USDT cents | `deposit_usdt_cents: 1000` |
| `_usdc_cents` | USDC cents | `payout_usdc_cents: 500` |

### Sensitive

| Suffix | Handling | Example |
|:-------|:---------|:--------|
| `_secret` | redact entire value to `***` | `api_key_secret: "sk-or-v1-abc..."` |

Any program output (logs, CLI, API responses) must redact `_secret` fields to `"***"`. Matching is case-insensitive — `_secret` and `_SECRET` are both redacted. Config files store the real value; everything else shows `"***"`.

### No suffix needed

Fields whose meaning is obvious from the name alone:

- URLs: `callback_url`, `homepage_url`
- Paths: `redb_path`, `config_path`
- Counts: `proof_count`, `relay_count`
- Booleans: `search_enabled`, `forward_pulse`
- Identifiers: `method`, `domain`, `model`, `backend`

### Environment variables

Same suffixes, `UPPER_SNAKE_CASE`:

```
DATABASE_URL_SECRET=postgres://user:pass@host/db
CACHE_TTL_S=3600
TOKEN_VALIDITY_HOURS=24
RUST_LOG=info
```

## Config files

Config files follow the same naming suffixes. Agents reading a config file can determine units, formats, and sensitivity without a separate schema.

### YAML

```yaml
openrouter:
  api_key_secret: "sk-or-v1-actual-key"
  model: "google/gemini-3-flash-preview"

storage:
  backend: redb
  postgres_url_secret: "postgres://user:pass@host/db"
  redb_path: "data.redb"

cache:
  dns_ttl_s: 3600
  cmn_ttl_s: 300

pricing:
  input_msats: 2
  output_msats: 12
```

### TOML

```toml
[cache]
dns_ttl_s = 3600
cmn_ttl_s = 300

[openrouter]
api_key_secret = "sk-or-v1-actual-key"
model = "google/gemini-3-flash-preview"
```

---

# Part 2: Output Protocol

A structured output format for programs. Every output is a JSON object with a `code` field that tells agents what phase of execution the program is in. Field names within the output follow Part 1 naming conventions.

## JSONL stream

Programs emit JSONL to stdout — one JSON object per line. Every line has a `code` field identifying its type:

| `code` | Meaning |
|:-------|:--------|
| `"startup"` | Program startup state |
| tool-defined | Status / progress / log |
| `"ok"` | Success result |
| `"error"` | Error result |

Three values are reserved: `startup`, `ok`, `error`. Status lines use any other value — the tool defines what makes sense (`"request"`, `"progress"`, `"sync"`, etc.).

Not all phases are required. A simple CLI tool may emit only a result line. A long-running service may never emit a result.

### Startup

`code: "startup"`. Optional. Emitted once at the beginning if the program has configuration.

```json
{"code": "startup", "config": {"api_key_secret": "***", "dns_ttl_s": 3600}, "args": {"config_path": "config.yml"}, "env": {"RUST_LOG": null, "DATABASE_URL_SECRET": "***"}}
```

- `config` — loaded configuration
- `args` — parsed CLI arguments
- `env` — environment variables the program reads (`null` if unset)

### Status

`code` is tool-defined. Content is tool-defined.

```json
{"code": "progress", "current": 3, "total": 10, "message": "indexing spores"}
```

```json
{"code": "request", "method": "POST", "path": "/v1/chat", "status": 200, "latency_ms": 42}
```

### Result

`code: "ok"` on success, `code: "error"` on failure. An agent watching a stream can treat either as the signal that the operation is complete. Include `trace` for execution context — data sources, processing steps, verification results, resource usage.

```json
{"code": "ok", "result": {"hash": "abc123", "size_bytes": 456789}, "trace": {"duration_ms": 1280, "tokens_input": 512, "tokens_output": 86, "cost_msats": 2056}}
```

```json
{"code": "error", "error": "config file not found", "trace": {"duration_ms": 3}}
```

### Agent consumption

1. Read `code` on every line.
2. `"startup"` → understand configuration.
3. `"ok"` or `"error"` → operation complete.
4. Anything else → status/progress, tool-specific.

## Output formats

```
--output json|yaml|plain
```

Default is tool-defined. Services default to `json`, interactive CLIs to `plain`.

JSON is the canonical format. YAML and plain are derived from it.

### yaml

Each JSON line becomes a YAML document, separated by `---`. Values preserved as-is. Strings always quoted to avoid YAML pitfalls (`no` → `false`, `3.0` → float):

```yaml
---
code: "startup"
config:
  api_key_secret: "***"
  dns_ttl_s: 3600
args:
  config_path: "config.yml"
---
code: "ok"
result:
  hash: "abc123"
  size_bytes: 456789
trace:
  duration_ms: 1280
  cost_msats: 2056
```

### plain

No quotes, values transformed for human readability. Plain is lossy — only json is the lossless canonical format.

```
code: startup
config:
  api_key_secret: ***
  dns_ttl_s: 3600s
args:
  config_path: config.yml
---
code: ok
result:
  hash: abc123
  size_bytes: 446.1KB
trace:
  duration_ms: 1.28s
  cost_msats: 2056msats
```

Suffix-driven formatting rules for plain. Default: append the unit abbreviation. Special cases override:

- `_ns`, `_us`, `_ms`, `_s` → append unit (`450000ns`, `830μs`, `42ms`, `3600s`)
- `_ms` ≥ 1000 → convert to seconds (`1280` → `1.28s`)
- `_minutes`, `_hours`, `_days` → append unit (`30 minutes`, `24 hours`)
- `_epoch_ms` / `_epoch_s` / `_epoch_ns` → RFC 3339 (`2024-02-14T00:00:00.000Z`), negative values produce pre-1970 dates
- `_rfc3339` → pass through
- `_bytes` → human-readable (`456789` → `446.1KB`, `-5242880` → `-5.0MB`)
- `_percent` → append `%` (`85` → `85%`, `99.9` → `99.9%`)
- `_msats` → append unit (`2056msats`)
- `_sats` → append unit (`1234sats`)
- `_btc` → append unit (`0.5 BTC`)
- `_usd_cents` → dollars (`999` → `$9.99`), negative falls through
- `_eur_cents` → euros (`850` → `€8.50`), negative falls through
- other `_{code}_cents` → major unit with code (`15050` → `150.50 THB`), negative falls through
- `_jpy` → yen (`1500` → `¥1,500`), negative falls through
- `_secret` → `***`

**Type constraints**: `_bytes` and `_epoch_*` require integer values. `_usd_cents`, `_eur_cents`, `_jpy`, and `_{code}_cents` require non-negative integers. Duration, Bitcoin, and `_percent` suffixes accept any number. When the value type doesn't match (string, boolean, float where integer expected), formatting falls through to the raw value.

### Key ordering

YAML and plain output sort object keys by UTF-16 code unit order (JCS, [RFC 8785](https://www.rfc-editor.org/rfc/rfc8785) §3.2.3). For ASCII keys — the common case — this equals simple byte-order sorting. The difference matters only when keys contain supplementary Unicode characters (U+10000+), where UTF-16 surrogate code units (0xD800–0xDBFF) sort before BMP code points in the 0xE000–0xFFFF range.

JSON output is unordered per the JSON specification. YAML and plain sort for deterministic, cross-language-consistent output.

## Beyond CLI

The `code` / `result` / `error` / `trace` structure applies beyond CLI. An agent uses the same parsing logic regardless of transport.

### REST API

Response body follows the same structure. HTTP status code maps to `code`.

HTTP 200:
```json
{"code": "ok", "result": {"balance_msats": 97900}, "trace": {"source": "redb", "duration_ms": 3}}
```

HTTP 402:
```json
{"code": "error", "error": "insufficient balance", "trace": {"balance_msats": 0, "required_msats": 2056}}
```

### MCP tool response

```json
{"code": "ok", "result": {"files": ["src/main.rs"]}, "trace": {"source": "glob", "matched": 1, "duration_ms": 12}}
```

### Streaming (SSE)

```json
{"code": "startup", "config": {"model": "gpt-4", "max_tokens": 1024}}
{"code": "progress", "current": 1, "total": 5, "message": "processing"}
{"code": "ok", "result": {"answer": "..."}, "trace": {"tokens_input": 512, "duration_ms": 1280}}
```

### One protocol, any transport

| Transport | Format | Same structure |
|:----------|:-------|:---------------|
| CLI stdout | JSONL | `code` / `result` / `error` / `trace` |
| REST API | JSON body | `code` / `result` / `error` / `trace` |
| MCP tool | JSON | `code` / `result` / `error` / `trace` |
| SSE stream | JSONL | `code` / `result` / `error` / `trace` |
