# Agent-First Data

**Self-describing structured data for AI agents and humans.**

Field names encode units and semantics. Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact — no external schema needed.

## Overview

Agent-First Data has three parts:

1. **[Naming Convention](#part-1-naming-convention)** (required) — encode units and semantics in field names
2. **[Output Processing](#part-2-output-processing)** (required) — suffix-driven formatting and automatic secret protection
3. **[Protocol Template](#part-3-protocol-template-recommended-optional)** (optional) — structured format with `code` (required) and `trace` (recommended)

**Parts 1 and 2 are the core.** Part 3 is optional — a recommended structure that works well with Parts 1 and 2, but you can use AFD naming with any JSON structure (REST APIs, GraphQL, databases, etc.).

**Jump to:**
- [Quick Reference: All Suffixes](#quick-reference-all-suffixes)
- [Complete Example](#complete-example-cli-tool)

## Quick Reference: All Suffixes

| Category | Suffixes | YAML/Plain example |
|:---------|:---------|:-------------------|
| **Duration** | `_ns`, `_us`, `_ms`, `_s`, `_minutes`, `_hours`, `_days` | `latency_ms: 1280` → `latency: 1.28s` |
| **Timestamps** | `_epoch_ns`, `_epoch_ms`, `_epoch_s`, `_rfc3339` | `created_at_epoch_ms: 1707868800000` → `created_at: 2024-02-14T...` |
| **Size** | `_bytes` (output), `_size` (config input) | `file_size_bytes: 5242880` → `file_size: 5.0MB` |
| **Currency** | `_msats`, `_sats`, `_btc`, `_usd_cents`, `_eur_cents`, `_jpy`, `_{code}_cents` | `price_usd_cents: 999` → `price: $9.99` |
| **Other** | `_percent`, `_secret` | `cpu_percent: 85` → `cpu: 85%` |

**In YAML and Plain:** suffixes are stripped from keys (value already encodes the unit) and values are formatted for readability. JSON preserves original keys and raw values.

**Secret protection:** All three formats automatically redact `_secret` fields.

---

# Part 1: Naming Convention

Applies to all structured data: JSON, YAML, TOML, CLI arguments, environment variables, config files, database columns, API responses, log fields.

## Design rules

1. **Name conveys meaning.** A reader should understand the field's purpose from the name alone, without seeing surrounding context or documentation. `data` could be anything — `request_body`, `search_results`, `cached_response` say exactly what it contains.
2. **Unit in suffix.** If a numeric value has a unit, encode the unit in the field name suffix.
3. **Secrets marked.** If a value is sensitive, end the field name with `_secret`.
4. **Obvious needs no suffix.** If the meaning is obvious from the name alone, no suffix is needed.
5. **Self-contained.** Never rely on external metadata, companion fields, or documentation to convey what a field contains.

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
| `_epoch_ms` | milliseconds since Unix epoch | `created_at_epoch_ms: 1707868800000` |
| `_epoch_s` | seconds since Unix epoch | `cached_epoch_s: 1707868800` |
| `_rfc3339` | RFC 3339 string | `expires_rfc3339: "2026-02-14T10:30:00Z"` |

> **Precision note**: `_epoch_ns` values near the current era (~1.7×10¹⁸) exceed JavaScript's safe integer range (2⁵³ ≈ 9×10¹⁵). JSON parsed by JavaScript will silently lose nanosecond precision. Use `BigInt` or a custom JSON parser when nanosecond accuracy matters.

### Size

| Suffix | Value type | Usage | Example |
|:-------|:-----------|:------|:--------|
| `_bytes` | numeric | Output, APIs | `payload_bytes: 456789` |
| `_size` | string with unit | Config input | `buffer_size: "10M"` |

**Simple rule:**

- **Output/APIs** → use `_bytes` (numeric, agents compute on this)
- **Config files** → use `_size` (string like "10M", humans write this)

Programs parse `_size` at load time using `parse_size()` and convert to bytes for internal use.

**Parsing rules for `_size` (binary units):**

| Unit | Multiplier | Example |
|:-----|:-----------|:--------|
| `B` or bare number | 1 | `"512"` → 512 |
| `K` | 1024 | `"10K"` → 10240 |
| `M` | 1024² | `"10M"` → 10485760 |
| `G` | 1024³ | `"2G"` → 2147483648 |
| `T` | 1024⁴ | `"1T"` → 1099511627776 |

Case-insensitive. Supports decimals (`"1.5M"`). Returns null for invalid/negative input.

**Example config file:**

```json
{
  "shared_buffers_size": "128M",
  "max_wal_size": "1G",
  "archive_retention_size": "2T"
}
```

In YAML and Plain output, `_bytes` values auto-scale to human-readable format (5.0MB, 2.0GB).

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

Stablecoins follow the same `_{code}_cents` pattern: `deposit_usdt_cents: 1000`, `payout_usdc_cents: 500`.

### Sensitive

| Suffix | Handling | Example |
|:-------|:---------|:--------|
| `_secret` | redact entire value to `***` | `api_key_secret: "sk-or-v1-abc..."` |

All CLI output formats (JSON, YAML, Plain) automatically redact `_secret` fields to `***`. API responses return raw values — no redaction needed over secure channels. Matching recognizes `_secret` and `_SECRET` only. Config files always store the real value.

### No suffix needed

Fields whose meaning is obvious from the name alone:

- URLs: `callback_url`, `homepage_url`
- Paths: `redb_path`, `config_path`
- Counts: `proof_count`, `relay_count`
- Booleans: `search_enabled`, `forward_pulse`
- Identifiers: `method`, `domain`, `model`, `backend`

### CLI arguments

Same suffixes, kebab-case. An agent reading `--help` output understands units and sensitivity without documentation:

```
--timeout-ms 5000          # milliseconds
--cache-ttl-s 3600         # seconds
--max-size-bytes 1048576   # bytes
--api-key-secret sk-xxx    # redact from logs and process listings
--buffer-size 10M          # human-readable config input (parse_size)
--port 8080                # no suffix needed — meaning obvious
--verbose                  # boolean flag — no suffix needed
```

**Long flags only.** Do not define single-letter short flags (`-s`, `-d`, `-l`). Short flags are ambiguous — `-s` could be `--synapse`, `--synopsis`, or `--source`. Agents parsing `--help` output cannot reliably interpret single-letter aliases. Always use the full `--kebab-case` form. The only exception is `-o` for `--output` and built-in flags like `-h`/`-V` from the argument parser.

**Kebab → snake mapping.** CLI flags map 1:1 to JSON field names by replacing hyphens with underscores. When a CLI tool emits a `startup` message (Part 3), the `args` field uses the snake_case form:

```bash
myapp --cache-ttl-s 3600 --api-key-secret sk-xxx --max-size-bytes 1048576
```

```json
{"code": "startup", "args": {"cache_ttl_s": 3600, "api_key_secret": "***", "max_size_bytes": 1048576}}
```

```yaml
---
code: "startup"
args:
  api_key: "***"
  cache_ttl: "3600s"
  max_size: "1.0MB"
```

The flag name, the JSON field name, and the formatted output all tell the same story. No mapping table, no `--help` prose explaining "timeout is in milliseconds" — the suffix is the documentation.

**Secret flags** (`--api-key-secret`, `--database-url-secret`) are automatically redacted in startup messages, logs, and YAML/Plain output. Tools should also consider redacting them from `/proc` process listings where possible.

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

## Database schemas

Same suffixes in column names. Agents reading a table schema can determine units, formats, and sensitivity without external documentation.

**When the database type already carries semantics, no suffix is needed.** `TIMESTAMPTZ` says "timestamp with timezone" — adding `_epoch_ms` is redundant. Suffixes are for generic types (`BIGINT`, `INTEGER`, `TEXT`) where the type alone is ambiguous.

```sql
CREATE TABLE events (
    id TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL,   -- type says timestamp, no suffix needed
    duration_ms INTEGER,               -- INTEGER is ambiguous, suffix needed
    payload_bytes INTEGER,
    api_key_secret TEXT,
    retry_count INTEGER,               -- no suffix needed, meaning is obvious
    domain TEXT NOT NULL
);
```

| Column | Type | Suffix needed? | Why |
|:-------|:-----|:---------------|:----|
| `created_at` | `TIMESTAMPTZ` | no | type encodes semantics |
| `duration_ms` | `INTEGER` | yes | 142 what? ms vs s vs μs |
| `payload_bytes` | `INTEGER` | yes | bytes vs KB vs count |
| `api_key_secret` | `TEXT` | yes | enables auto-redaction |
| `retry_count` | `INTEGER` | no | meaning obvious from name |
| `expires_at` | `TIMESTAMPTZ` | no | type encodes semantics |
| `cached_epoch_ms` | `BIGINT` | yes | bare integer needs unit |

**ORM / struct mapping**: Keep the suffix in the struct field name. The suffix is part of the semantic name, not a display concern:

```rust
struct Event {
    created_at: DateTime<Utc>,   // native type — no suffix
    duration_ms: i64,            // integer — suffix preserves semantics
    // duration: i64,            // bad — 64-bit what? seconds? ms?
}
```

**Queries**: Column aliases in views or query results should also follow AFD naming:

```sql
SELECT
    duration_ms,
    payload_bytes,
    (cost_input_msats + cost_output_msats) AS total_cost_msats
FROM requests;
```

---

# Part 2: Output Processing

Transform JSON values for CLI/log output with suffix-driven formatting and automatic secret protection. This applies to any JSON data, regardless of structure.

## Two Usage Contexts

### Context 1: API Responses

Return JSON values directly. Web frameworks handle serialization.

**No output processing.** API responses are transmitted over secure channels (HTTPS). Return raw JSON:

```json
{"user_id": 123, "api_key_secret": "sk-1234567890abcdef", "balance_msats": 50000}
```

### Context 2: CLI / Logs

Format JSON values for terminal/log display.

**Automatic processing:** Suffix formatting + secret redaction.

**Input:**
```json
{"user_id": 123, "api_key_secret": "sk-1234567890abcdef", "balance_msats": 50000}
```

**JSON:** `{"api_key_secret":"***","balance_msats":50000,"user_id":123}`

**YAML:**
```yaml
---
api_key: "***"
balance: "50000msats"
user_id: 123
```

**Plain:** `api_key=*** balance=50000msats user_id=123`

## Output Formats

CLI tools should support multiple output formats:

```
--output json|yaml|plain
```

Default is tool-defined. Interactive CLIs default to `yaml`, scripting/logging contexts to `json`.

JSON is the canonical format. YAML and plain are derived from it.

**All CLI output formats automatically redact `_secret` fields.** Any field ending in `_secret` (case-insensitive) is replaced with `***` before display. API responses bypass Part 2 and return raw values (see [Two Usage Contexts](#two-usage-contexts)).

**Format characteristics:**
- **JSON** — single-line, original keys, raw values, no sorting (machine-readable), secrets redacted
- **YAML** — multi-line, human-readable, keys stripped, values formatted, secrets redacted
- **Plain** — single-line logfmt, human-readable, keys stripped, values formatted, secrets redacted

### yaml

Each JSON line becomes a YAML document, separated by `---`. Strings always quoted to avoid YAML pitfalls (`no` → `false`, `3.0` → float). **Suffixes stripped from keys** (value already encodes the unit). **Secrets automatically redacted.**

```yaml
---
code: "startup"
config:
  api_key: "***"
  dns_ttl: "3600s"
args:
  config_path: "config.yml"
---
code: "ok"
result:
  hash: "abc123"
  size: "446.1KB"
trace:
  duration: "1.28s"
  cost: "2056msats"
```

### plain

Single-line [logfmt](https://brandur.org/logfmt) style. **Suffixes stripped from keys.** **Secrets automatically redacted.**

- Nested keys use dot notation: `trace.duration=1.28s`
- Values containing spaces are quoted: `message="uploading chunks"`
- Arrays are comma-joined: `fields=email,age`
- Null values are empty: `RUST_LOG=`

```
args.config_path=config.yml code=startup config.api_key=*** config.dns_ttl=3600s
code=ok result.hash=abc123 result.size=446.1KB trace.cost=2056msats trace.duration=1.28s
```

### Suffix processing (yaml and plain)

YAML and plain apply two transformations:

**1. Key stripping** — remove the suffix from the key name. The formatted value already encodes the unit, so the suffix is redundant for human readers.

**Algorithm:** match the longest known suffix from the list below. Each suffix is recognized in two forms: lowercase (`_secret`) and uppercase (`_SECRET`). No other casing is matched. Remove the matched suffix from the key. If no suffix matches, keep the key unchanged. Match order (longest first):

1. `_epoch_ms`, `_epoch_s`, `_epoch_ns` (compound timestamp suffixes)
2. `_usd_cents`, `_eur_cents`, `_{code}_cents` (compound currency suffixes)
3. `_rfc3339`, `_minutes`, `_hours`, `_days` (multi-char suffixes)
4. `_msats`, `_sats`, `_bytes`, `_percent`, `_secret` (single-unit suffixes)
5. `_btc`, `_jpy`, `_ns`, `_us`, `_ms`, `_s` (short suffixes, matched last to avoid false positives)

**Collision:** if two keys in the same object produce the same stripped key (e.g., `download_bytes` and `download_size` both → `download`), revert both to their original key AND raw value (no formatting).

| JSON key | YAML/Plain key | Why |
|:---------|:---------------|:----|
| `duration_ms` | `duration` | value shows `1.28s` |
| `size_bytes` | `size` | value shows `446.1KB` |
| `created_at_epoch_ms` | `created_at` | value shows `2025-02-07T...` |
| `expires_rfc3339` | `expires` | value passes through |
| `api_key_secret` | `api_key` | value shows `***` |
| `cpu_percent` | `cpu` | value shows `85%` |
| `balance_msats` | `balance` | value shows `50000msats` |
| `price_usd_cents` | `price` | value shows `$9.99` |
| `DATABASE_URL_SECRET` | `DATABASE_URL` | uppercase `_SECRET` matched |
| `CACHE_TTL_S` | `CACHE_TTL` | uppercase `_S` matched |
| `buffer_size` | `buffer_size` | `_size` passes through, key unchanged |
| `config_path` | `config_path` | no suffix, unchanged |
| `user_id` | `user_id` | no suffix, unchanged |

**2. Value formatting** — transform the value for human readability. Same suffix matching as key stripping (lowercase or uppercase only):

- `_ns`, `_us`, `_ms`, `_s` → append unit (`450000ns`, `830μs`, `42ms`, `3600s`)
- `_ms` ≥ 1000 → convert to seconds (`1280` → `1.28s`)
- `_minutes`, `_hours`, `_days` → append unit (`30 minutes`, `24 hours`)
- `_epoch_ms` / `_epoch_s` / `_epoch_ns` → RFC 3339 (`2024-02-14T00:00:00.000Z`), negative values produce pre-1970 dates
- `_rfc3339` → pass through
- `_bytes` → human-readable (`456789` → `446.1KB`, `-5242880` → `-5.0MB`)
- `_size` → pass through (config input string, e.g. `"10M"` stays `"10M"`)
- `_percent` → append `%` (`85` → `85%`, `99.9` → `99.9%`)
- `_msats` → append unit (`2056msats`)
- `_sats` → append unit (`1234sats`)
- `_btc` → append unit (`0.5 BTC`)
- `_usd_cents` → dollars (`999` → `$9.99`), negative falls through
- `_eur_cents` → euros (`850` → `€8.50`), negative falls through
- other `_{code}_cents` → major unit with code (`15050` → `150.50 THB`), negative falls through
- `_jpy` → yen (`1500` → `¥1,500`), negative falls through
- `_secret` → `***`

**Type constraints**: `_bytes` and `_epoch_*` require integer values. `_usd_cents`, `_eur_cents`, `_jpy`, and `_{code}_cents` require non-negative integers. Duration, Bitcoin, and `_percent` suffixes accept any number. When the value type doesn't match, formatting falls through to the raw value with the original key preserved.

### Key ordering

YAML and plain output sort keys (after stripping) by UTF-16 code unit order (JCS, [RFC 8785](https://www.rfc-editor.org/rfc/rfc8785) §3.2.3). For ASCII keys — the common case — this equals simple byte-order sorting.

In plain logfmt, nested keys are flattened to dot notation before sorting. Sort by the full dot path: `args.input_path` < `code` < `config.api_key` < `trace.duration`.

JSON output is unordered per the JSON specification. YAML and plain sort for deterministic, cross-language-consistent output.

## Using AFD Without Part 3

Parts 1 and 2 (naming + output processing) work with any JSON structure — no protocol template needed:

```json
{"user_id": 123, "created_at_epoch_ms": 1738886400000, "balance_msats": 50000000, "api_key_secret": "sk-..."}
```

Plain: `api_key=*** balance=50000000msats created_at=2025-02-07T00:00:00.000Z user_id=123`

This works with REST APIs, GraphQL, database results, config files — anywhere you have structured data. Just use AFD naming and let output processing handle the rest.

---

# Part 3: Protocol Template (Recommended, Optional)

A recommended structure for program output. This part is **optional** — adopt it when you want consistent structure across CLI tools, streaming output, or internal protocols.

## Core Fields

**Required:**
- `code` — identifies the message type (`"startup"`, `"ok"`, `"error"`, or tool-defined)

**Recommended:**
- `trace` — execution context (duration, source, resource usage)

**Everything else is flexible.** Fields can be flat or nested. Both styles are valid. Examples below show both approaches.

## JSONL Stream

Programs emit JSONL to stdout — one JSON object per line. Every line has a `code` field identifying its type:

| `code` | Meaning |
|:-------|:--------|
| `"startup"` | Program startup state |
| `"ok"` | Success result |
| `"error"` | Generic error (prefer specific codes) |
| tool-defined | Status / errors / progress |

Three values are reserved: `startup`, `ok`, `error`. All other values are tool-defined.

**Error codes:** Use specific codes instead of generic `"error"`:
- `"not_found"`, `"unauthorized"`, `"validation_error"`, `"rate_limit"`, `"internal_error"`, etc.
- Generic `"error"` is supported but specific codes are preferred

**Status codes:** Progress, requests, custom events:
- `"request"`, `"progress"`, `"sync"`, etc.

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

`code` is tool-defined. Content is tool-defined. Include `trace` for execution context.

```json
{"code": "progress", "current": 3, "total": 10, "message": "indexing spores", "trace": {"duration_ms": 500}}
```

```json
{"code": "request", "method": "POST", "path": "/v1/chat", "http_status": 200, "trace": {"latency_ms": 42}}
```

### Result

`code: "ok"` on success, `code: "error"` or specific error code on failure. An agent watching a stream can treat any result code as the signal that the operation is complete.

**Always include `trace`** for execution context — duration, data sources, resource usage, query details.

**Success - both styles valid:**

Nested (structured):
```json
{"code": "ok", "result": {"hash": "abc123", "size_bytes": 456789}, "trace": {"duration_ms": 1280, "tokens_input": 512}}
```

Flat:
```json
{"code": "ok", "hash": "abc123", "size_bytes": 456789, "trace": {"duration_ms": 1280, "tokens_input": 512}}
```

**Error - both styles valid:**

Simple message:
```json
{"code": "error", "error": "config file not found", "trace": {"duration_ms": 3}}
```

Nested error details:
```json
{"code": "not_found", "error": {"resource": "user", "id": 123}, "trace": {"duration_ms": 8}}
```

Flat error details:
```json
{"code": "not_found", "resource": "user", "id": 123, "trace": {"duration_ms": 8}}
```

More examples (flat style):
```json
{"code": "validation_error", "fields": ["email", "age"], "trace": {"duration_ms": 2}}
{"code": "unauthorized", "message": "invalid token", "trace": {"duration_ms": 5}}
{"code": "rate_limit", "retry_after_s": 60, "quota_remaining": 0, "trace": {"duration_ms": 1}}
```

### Best Practices

**Always include `trace` field.** Even simple operations should report execution context:

- `duration_ms` — operation duration
- `source` — data source (db, cache, api, file)
- Resource usage — `tokens_input`, `tokens_output`, `cost_msats`, `memory_bytes`
- Metadata — `query`, `method`, `path`, `model`

**Good (with trace):**
```json
{"code": "ok", "count": 42, "trace": {"duration_ms": 150, "source": "db"}}
{"code": "error", "error": "not found", "trace": {"duration_ms": 5}}
```

**Also good (structured):**
```json
{"code": "ok", "result": {"count": 42}, "trace": {"duration_ms": 150, "source": "db"}}
{"code": "validation_error", "error": {"fields": [...]}, "trace": {"duration_ms": 2}}
```

**Avoid (missing trace):**
```json
{"code": "ok", "count": 42}
{"code": "error", "error": "not found"}
```

Missing `trace` makes debugging harder. Agents can't analyze performance, cost, or data flow without execution context.

### Agent consumption

1. Read `code` on every line.
2. `"startup"` → understand configuration.
3. `"ok"` or `"error"` → operation complete.
4. Anything else → status/progress, tool-specific.

## Usage in APIs

The protocol structure can be used in REST APIs. APIs return raw JSON — no output formatting, no secret redaction.

### REST API Examples

Response body follows the protocol structure:

**HTTP 200:**
```json
{"code": "ok", "result": {"balance_msats": 97900}, "trace": {"source": "redb", "duration_ms": 3}}
```

**HTTP 404:**
```json
{"code": "not_found", "error": {"resource": "user", "id": 123}, "trace": {"duration_ms": 5}}
```

**HTTP 402:**
```json
{"code": "insufficient_balance", "error": {"balance_msats": 0, "required_msats": 2056}, "trace": {"source": "redb", "duration_ms": 2}}
```

### MCP Tool Response

Same structure, raw JSON:

```json
{"code": "ok", "result": {"files": ["src/main.rs"]}, "trace": {"source": "glob", "matched": 1, "duration_ms": 12}}
```

### Streaming (SSE)

JSONL stream, raw JSON per line:

```json
{"code": "startup", "config": {"model": "gpt-4", "max_tokens": 1024}, "args": {}, "env": {}}
{"code": "progress", "current": 1, "total": 5, "message": "processing", "trace": {"duration_ms": 500}}
{"code": "ok", "result": {"answer": "..."}, "trace": {"tokens_input": 512, "duration_ms": 1280}}
```

### One Protocol, Multiple Contexts

| Context | Output | Secret Protection |
|:--------|:-------|:------------------|
| **CLI / Logs** | JSONL (json/yaml/plain formats) | ✅ Automatic |
| **REST API** | JSON body (raw Value) | ❌ None needed |
| **MCP tool** | JSON (raw Value) | ❌ None needed |
| **SSE stream** | JSONL (raw JSON) | ❌ None needed |

All contexts can use the protocol structure from Part 3. Only `code` (required) and `trace` (recommended) are standardized. Other fields can be flat or nested — both styles work. CLI/logs apply output formatting and secret protection from Part 2. APIs return raw JSON Values.

---

# Complete Example: CLI Tool

A complete example showing all three parts working together. A backup tool that uploads files to cloud storage.

## CLI Invocation

```bash
cloudback --api-key-secret sk-1234567890abcdef --timeout-s 30 --max-file-size-bytes 10737418240 /data/backup.tar.gz
```

Flag names use AFD suffixes in kebab-case. An agent reading `--help` knows `--timeout-s` is seconds and `--api-key-secret` should be redacted — no documentation needed.

## Raw JSON (before output processing)

The tool converts CLI flags from kebab-case to snake_case and emits a `startup` message:

```json
{
  "code": "startup",
  "config": {
    "api_key_secret": "sk-1234567890abcdef",
    "endpoint": "https://storage.example.com",
    "timeout_s": 30,
    "max_file_size_bytes": 10737418240
  },
  "args": {
    "input_path": "/data/backup.tar.gz",
    "compression_level": 9
  }
}
```

Field names encode semantics:
- `api_key_secret` → agent knows to redact
- `timeout_s` → 30 seconds
- `max_file_size_bytes` → 10GB in bytes

## Output Formats (Part 2: Output Processing)

**JSON** (raw, for machines):
```json
{"code":"startup","config":{"api_key_secret":"***","endpoint":"https://storage.example.com","timeout_s":30,"max_file_size_bytes":10737418240},"args":{"input_path":"/data/backup.tar.gz","compression_level":9}}
```

**YAML** (structured, keys stripped, for human inspection):
```yaml
---
code: "startup"
args:
  compression_level: 9
  input_path: "/data/backup.tar.gz"
config:
  api_key: "***"
  endpoint: "https://storage.example.com"
  max_file_size: "10.0GB"
  timeout: "30s"
```

**Plain** (single-line logfmt, keys stripped, for compact scanning):
```
args.compression_level=9 args.input_path=/data/backup.tar.gz code=startup config.api_key=*** config.endpoint=https://storage.example.com config.max_file_size=10.0GB config.timeout=30s
```

Note:
- **Key stripping**: `api_key_secret` → `api_key`, `timeout_s` → `timeout`, `max_file_size_bytes` → `max_file_size`
- **Secret protection**: `api_key_secret` redacted in all three formats
- **Suffix formatting**: `_bytes` → `10.0GB`, `_s` → `30s` in YAML and Plain

## Progress Update (Part 3: Protocol Template)

```json
{"code": "progress", "current": 3, "total": 10, "message": "uploading chunks", "trace": {"duration_ms": 5420, "uploaded_bytes": 3221225472}}
```

YAML:
```yaml
---
code: "progress"
current: 3
message: "uploading chunks"
total: 10
trace:
  duration: "5.42s"
  uploaded: "3.0GB"
```

Plain:
```
code=progress current=3 message="uploading chunks" total=10 trace.duration=5.42s trace.uploaded=3.0GB
```

## Final Result

```json
{"code": "ok", "result": {"url": "https://storage.example.com/backup.tar.gz", "size_bytes": 10485760, "checksum": "sha256:abc123...", "uploaded_at_epoch_ms": 1738886400000}, "trace": {"duration_ms": 15300, "chunks": 10, "retries": 2}}
```

YAML:
```yaml
---
code: "ok"
result:
  checksum: "sha256:abc123..."
  size: "10.0MB"
  uploaded_at: "2025-02-07T00:00:00.000Z"
  url: "https://storage.example.com/backup.tar.gz"
trace:
  chunks: 10
  duration: "15.3s"
  retries: 2
```

Plain:
```
code=ok result.checksum=sha256:abc123... result.size=10.0MB result.uploaded_at=2025-02-07T00:00:00.000Z result.url=https://storage.example.com/backup.tar.gz trace.chunks=10 trace.duration=15.3s trace.retries=2
```

## What This Demonstrates

1. **Part 1 (Naming)**: Every field is self-describing — from CLI flags (`--timeout-s`, `--api-key-secret`) to JSON fields (`timeout_s`, `uploaded_at_epoch_ms`). Same suffixes, same semantics, kebab↔snake mapping

2. **Part 2 (Output Processing)**: Three formats for different needs
   - JSON: single-line, original keys, raw values, for programs and logs
   - YAML: multi-line, keys stripped, values formatted, for human inspection
   - Plain: single-line logfmt, keys stripped, values formatted, for compact scanning
   - All formats protect secrets automatically

3. **Part 3 (Protocol)**: Consistent structure across all output — `code` identifies message type, `trace` provides execution context, other fields flexible

**Key insight**: The same naming convention flows from CLI flag (`--timeout-s 30`) to JSON field (`timeout_s: 30`) to formatted output (`timeout: 30s`). An agent reading `--help`, JSON output, or YAML all gets the same self-describing semantics — no documentation needed at any layer.
