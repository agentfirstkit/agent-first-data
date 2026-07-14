# Agent-First Data

**Self-describing structured data for AI agents and humans.**

Field names encode units and semantics. Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact — no external schema needed.

## Overview

Agent-First Data has three parts:

1. **[Naming Convention](#part-1-naming-convention)** (required) — encode units and semantics in field names
2. **[Output Processing](#part-2-output-processing)** (required) — suffix-driven formatting and automatic secret protection
3. **[Protocol Template](#part-3-protocol-template-recommended-optional)** (optional) — structured format with `code` (required) and `trace` (recommended)

**Parts 1 and 2 are the core.** Part 3 is optional — a recommended structure that works well with Parts 1 and 2, but you can use AFDATA naming with any JSON structure (REST APIs, GraphQL, databases, etc.).

**Jump to:**
- [Quick Reference: All Suffixes](#quick-reference-all-suffixes)
- [Complete Example](#complete-example-cli-tool)

## Quick Reference: All Suffixes

| Category | Suffixes | YAML/Plain example |
|:---------|:---------|:-------------------|
| **Duration** | `_ns`, `_us`, `_ms`, `_s`, `_minutes`, `_hours`, `_days` | `latency_ms: 1280` → `latency: 1.28s` |
| **Timestamps** | `_epoch_ns`, `_epoch_ms`, `_epoch_s`, `_rfc3339` | `created_at_epoch_ms: 1707868800000` → `created_at: 2024-02-14T...` |
| **Size** | `_bytes` (output), `_size` (config input) | `file_size_bytes: 5242880` → `file_size: 5.0MiB` |
| **Currency** | `_msats`, `_sats`, `_usd_cents`, `_eur_cents`, `_jpy`, `_{code}_cents`, `_{code}_micro` | `price_usd_cents: 999` → `price: $9.99` |
| **String formats** | `_bcp47`, `_utc_offset`, `_rfc3339_date`, `_rfc3339_time` | `language_bcp47: "zh-CN"`, `invoice_due_rfc3339_date: "2026-06-13"` |
| **Other** | `_percent`, `_secret`, `_url` | `cpu_percent: 85` → `cpu: 85%` |

**In default YAML and Plain:** formatting suffixes are stripped from keys (value already encodes the unit) and values are formatted for readability. JSON preserves original keys and raw values. (`_url`, `_bcp47`, `_utc_offset`, `_rfc3339_date`, and `_rfc3339_time` are not stripped.)

**Secret protection:** All three formats automatically redact `_secret` fields and scrub secret components (userinfo password, secret-named query params) inside `_url` field values.

**Boundary:** AFDATA names communicate local field semantics. They do not replace schemas for required fields, enum values, numeric ranges, object shapes, or cross-field validation. Use JSON Schema, OpenAPI, database constraints, or typed APIs for those guarantees.

---

# Part 1: Naming Convention

Applies to all structured data: JSON, YAML, TOML, CLI arguments, environment variables, config files, database columns, HTTP payload fields, log fields.

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
| `_epoch_ns` | nanoseconds since Unix epoch as a decimal string | `created_epoch_ns: "1707868800000000000"` |
| `_epoch_ms` | milliseconds since Unix epoch | `created_at_epoch_ms: 1707868800000` |
| `_epoch_s` | seconds since Unix epoch | `cached_epoch_s: 1707868800` |
| `_rfc3339` | RFC 3339 date-time string | `expires_rfc3339: "2026-02-14T10:30:00Z"` |

`_epoch_s` and `_epoch_ms` use JSON integers. Current-era `_epoch_ns` values exceed JSON's cross-language safe integer range, so `_epoch_ns` uses a decimal string.

### Strict string formats

These suffixes identify strings with a strict external format. They are semantic field-name conventions, not YAML/Plain formatting suffixes: readable output keeps the full key and raw string value.

| Suffix | Format | Example |
|:-------|:-------|:--------|
| `_bcp47` | BCP-47 language tag string | `language_bcp47: "zh-CN"` |
| `_utc_offset` | fixed UTC offset string | `timezone_utc_offset: "+08:00"` |
| `_rfc3339_date` | RFC 3339 full-date string | `invoice_due_rfc3339_date: "2026-06-13"` |
| `_rfc3339_time` | RFC 3339 partial-time string | `market_open_rfc3339_time: "09:30:00"` |

`*_bcp47` names a field whose string value is a BCP-47 language tag, such as `language_bcp47: "zh-CN"` or `content_language_bcp47: "en-US"`. AFDATA does not implement the full BCP-47 registry; tools may validate tags when they need stronger guarantees.

`*_utc_offset` names a fixed offset from UTC. Canonical persisted and structured output values are `"UTC"` or `±HH:MM`, with `HH` in `00..23` and `MM` in `00..59`; zero offsets normalize to `"UTC"`. Examples: `timezone_utc_offset: "+08:00"`, `report_utc_offset: "-05:00"`. This is intentionally not an IANA timezone name: do not use `Asia/Shanghai`, `America/Los_Angeles`, DST rules, or timezone databases in this field.

`*_rfc3339_date` names an RFC 3339 `full-date` string: exactly `YYYY-MM-DD`, such as `invoice_due_rfc3339_date: "2026-06-13"`. It is a calendar date, not an instant, and it does not imply any time, offset, or timezone.

`*_rfc3339_time` names an RFC 3339 `partial-time` string: exactly `HH:MM:SS` with optional fractional seconds, such as `market_open_rfc3339_time: "09:30:00"` or `"09:30:00.123"`. It is a time-of-day, not an instant. It MUST NOT include `Z`, `±HH:MM`, an IANA timezone, or any other timezone annotation; a time without a date cannot be resolved through timezone/DST rules. Use `_rfc3339` or `_epoch_*` for instants.

AFDATA core does not define a companion timezone-name field. If a future tool needs to preserve IANA timezone semantics with a timestamp, prefer a self-contained standard value such as RFC 9557 rather than pairing a date/time field with a separate timezone-name field.

Tools should avoid magic string sentinels such as `"auto"` inside strict-format fields. If a tool needs auto/default behavior, define that in the tool's own config semantics rather than as an AFDATA-wide rule.

### Size

| Suffix | Value type | Usage | Example |
|:-------|:-----------|:------|:--------|
| `_bytes` | non-negative integer | Output, APIs | `payload_bytes: 456789` |
| `_size` | string with explicit unit | Config input | `buffer_size: "10MiB"` |

**Simple rule:**

- **Output/APIs** → use `_bytes` (numeric, agents compute on this)
- **Config files** → use `_size` (string like "10MiB" or "10MB", humans write this)

Programs parse `_size` at load time using `parse_size()` and convert to bytes for internal use.

**Parsing rules for `_size`:**

| Unit | Multiplier | Example |
|:-----|:-----------|:--------|
| `B` | 1 | `"512B"` → 512 |
| `kB`/`MB`/`GB`/`TB` | decimal powers of 1000 | `"10MB"` → 10000000 |
| `KiB`/`MiB`/`GiB`/`TiB` | binary powers of 1024 | `"10MiB"` → 10485760 |

Ambiguous `K/M/G/T` units and bare numbers are rejected. Supports decimals (`"1.5MiB"`). Returns null for invalid, negative, or overflow/unrepresentable input. To keep the helper byte-identical across all four ports, parsed sizes above JSON's safe integer ceiling (`2^53 - 1`) are rejected.

**Example config file:**

```json
{
  "shared_buffers_size": "128MiB",
  "max_wal_size": "1GiB",
  "archive_retention_size": "2TiB"
}
```

In YAML and Plain output, `_bytes` values auto-scale to human-readable format (5.0MiB, 2.0GiB).

### Percentage

| Suffix | Unit | Example |
|:-------|:-----|:--------|
| `_percent` | percentage | `cpu_percent: 85` |

### Currency

Bitcoin:

| Suffix | Unit | Example |
|:-------|:-----|:--------|
| `_msats` | millisatoshis as an integer or decimal integer string | `balance_msats: 97900` |
| `_sats` | satoshis as an integer or decimal integer string | `withdrawn_sats: 1234` |

AFDATA does not define a floating `_btc` suffix. Use integer `_sats` or `_msats` instead.

Fiat — `_{iso4217}_cents` for currencies with 1/100 subdivision, `_{iso4217}` for currencies without (JPY). Always integers:

| Suffix | Unit | Example |
|:-------|:-----|:--------|
| `_usd_cents` | US dollar cents | `price_usd_cents: 999` |
| `_eur_cents` | euro cents | `price_eur_cents: 850` |
| `_thb_cents` | Thai baht 1/100 | `fare_thb_cents: 15050` |
| `_jpy` | Japanese yen (no minor unit) | `price_jpy: 1500` |

Stablecoins follow the same `_{code}_cents` pattern: `deposit_usdt_cents: 1000`, `payout_usdc_cents: 500`.

Sub-cent precision — `_{code}_micro` for integer micro-units, one millionth (10⁻⁶) of the major unit:

| Suffix | Unit | Example |
|:-------|:-----|:--------|
| `_{code}_micro` | millionths of one major unit | `cost_usd_micro: 170000` (= $0.17) |

`_{code}_micro` is the fiat analog of `_msats`: when cents are too coarse (per-token LLM pricing, metered API costs, unit-economics accounting), do not switch to decimal cents — move to a smaller integer unit. Values are always integers. Use `_{code}_cents` for user-facing amounts and `_{code}_micro` for high-precision internal accounting.

### Sensitive

| Suffix | Handling | Example |
|:-------|:---------|:--------|
| `_secret` | redact the entire value/subtree to `***` | `api_key_secret: "sk-or-v1-abc..."` |
| `_url` | redact secret components **inside** the URL value (userinfo password, secret-named query params); the rest of the URL is preserved | `callback_url: "https://h/cb?code_secret=..."` |

All CLI output formats (JSON, YAML, Plain) automatically redact `_secret` fields. Any `_secret` value — scalar, object, or array — becomes the scalar string `***`, so a secret-marked container never leaks through JSON, YAML, Plain, or collision fallback. Matching recognizes `_secret` and `_SECRET` only. Config files always store the real value. For legacy payloads that cannot rename fields to `_secret`, use `OutputOptions.redaction.secret_names` (a configured `Redactor` in Rust/Go, keyword arguments in Python/TS) at serialization time; names match exact field names at any nesting level, with no trim, case folding, hyphen/underscore normalization, globs, regex, or substring matching. Secret-name lists only affect redaction; formatting suffix stripping is still controlled by AFDATA suffixes in the default readable style. AFDATA does not define named redaction profiles; use the default, `secret_names`, `RedactionTraceOnly`, or `RedactionNone` deliberately at the serialization boundary. Callers that need schema-preserving YAML/plain rendering can use `OutputOptions` with the `Raw` output style.

The marker `***` has exactly one meaning in AFDATA output: a value was redacted because its field name, URL query parameter name, or explicit `secret_names` entry made it sensitive. It is not used for serialization failures, truncation, unsupported types, or arbitrary “maybe secret” guesses.

#### Secrets inside URLs

Key-based redaction cannot reach a secret embedded **inside** a URL string — `token` in `wss://host/cdp?token=abc` is not its own field, and the URL often lives in a free-form `error` or log message that must stay readable. Implementations expose a URL-aware helper for this:

- `redact_url_secrets(url, *, secret_names=())` (Python) / `redactUrlSecrets(url, options?)` (TS) / `redact_url_secrets(url)` with `Redactor{secret_names}.url(url)` for custom names (Rust/Go) — returns `url` with its secret components redacted to `***`.

The same secret decision as everywhere else applies, **to the URL's query-parameter names**: a parameter is redacted iff its (form-decoded) name ends in `_secret`/`_SECRET`, or matches an exact entry in `secret_names`. No built-in list of "sensitive" parameter names exists — a legacy parameter such as `?token=` is redacted only when the caller passes `secret_names: ["token"]`, exactly as for legacy field names. Consumers that own the URL should instead rename the parameter to follow the suffix convention (`?token_secret=`).

> **⚠️ Common credential-bearing parameters are NOT redacted by default.** The userinfo password (`user:pass@host`) is always scrubbed structurally, but query parameters are matched by name only. Conventionally-named secret parameters such as `?access_token=`, `?api_key=`, `?code=`, `?id_token=`, `?sig=`, or `?sessionid=` pass through **unchanged** unless their name ends in `_secret` or is listed in `secret_names`. A `_url` field does not make an arbitrary URL safe to log — it scrubs the userinfo password and suffix-named/listed parameters, nothing more. When you own the URL, rename sensitive parameters to the `_secret` suffix (`?access_token_secret=`); when you do not, pass the parameter names via `secret_names`.

Independently of the parameter convention, the **userinfo password** component is always redacted as a structural rule: `scheme://user:pass@host` → `scheme://user:***@host` (the username is preserved; a userinfo with no `:` is left untouched).

**Input must be a single URL.** The standalone helper processes a string iff it begins with a scheme (`^[A-Za-z][A-Za-z0-9+.-]*://`) and contains no whitespace; any other string — including a URL embedded in surrounding prose — is returned unchanged. Callers that build messages around a URL redact the URL **before** interpolating it: `format("connect {}: {}", redact_url_secrets(url), err)`.

**Surgical replacement.** Only the secret spans (a secret parameter's value bytes after `=` up to the next `&`/`#`/end; the password bytes after the first `:` in userinfo up to the authority's last `@`) are replaced with the literal `***`. Every other byte — scheme, host, path, fragment, benign parameters, percent-encoding, ordering — is preserved exactly. Implementations parse with their URL library but must not re-serialize the whole URL (normalization differs across libraries and would break cross-language parity); output equals input outside the redacted spans.

**Automatic application via the `_url` suffix.** Redaction applies `redact_url_secrets` to the string value of any field whose name ends in `_url`/`_URL` — and **only** those fields. No payload string is scanned: the trigger is the field name, exactly like `_secret`. So `final_url` and `callback_url` are scrubbed automatically, while a free-form `error` or `message` field is never touched even if it contains a URL (redact such a URL with the helper before interpolating it). `RedactionNone` disables it along with all other redaction; `RedactionTraceOnly` scopes it to the `trace` subtree. A `_url` value with surrounding whitespace is trimmed before URL redaction. A `_url` value that cannot be parsed as a clean scheme-prefixed URL is replaced with `***` rather than silently passing through a likely malformed secret-bearing value when it carries either internal whitespace or an `@` credential sigil — for example a schemeless connection string `user:pass@host:5432/db`, which has no scheme anchor for the surgical span logic. A schemeless, `@`-free, whitespace-free value (e.g. a relative URL `/cb?page=2`) still passes through unchanged. The `secret_names` list applies to query-parameter names inside `_url` values as well. (A field carrying both meanings, e.g. `token_url_secret`, ends in `_secret` and so its whole value is redacted to `***`.)

### No suffix needed

Fields whose meaning is obvious from the name alone:

- Paths: `redb_path`, `config_path`
- Counts: `proof_count`, `relay_count`
- Booleans: `search_enabled`, `forward_pulse`
- Identifiers: `method`, `domain`, `model`, `backend`

(URL-valued fields are the exception: end them in `_url` so secrets inside the
URL are scrubbed — see the `_url` suffix above.)

### CLI arguments

Same suffixes, kebab-case. An agent reading `--help` output understands units and sensitivity without documentation:

```
--timeout-ms 5000          # milliseconds
--cache-ttl-s 3600         # seconds
--max-size-bytes 1048576   # bytes
--api-key-secret sk-xxx    # redact from logs and process listings
--buffer-size 10MiB        # human-readable config input (parse_size)
--port 8080                # no suffix needed — meaning obvious
--verbose                  # boolean flag — no suffix needed
```

**Long flags only.** Do not define single-letter short flags (`-s`, `-d`, `-l`). Short flags are ambiguous — `-s` could be `--synapse`, `--synopsis`, or `--source`. Agents parsing `--help` output cannot reliably interpret single-letter aliases. Always use the full `--kebab-case` form. The only exception is `-o` for `--output` and built-in flags like `-h`/`-V` from the argument parser.

**Kebab → snake mapping.** CLI flags map 1:1 to JSON field names by replacing hyphens with underscores. When a CLI tool emits a startup log event (Part 3), the `args` field uses the snake_case form:

```bash
myapp --cache-ttl-s 3600 --api-key-secret sk-xxx --max-size-bytes 1048576
```

```json
{"kind":"log","log":{"message":"startup","level":"info","event":"startup","args":{"cache_ttl_s":3600,"api_key_secret":"***","max_size_bytes":1048576}},"trace":{}}
```

```yaml
---
kind: "log"
log:
  args:
    api_key: "***"
    cache_ttl: "3600s"
    max_size: "1.0MiB"
  event: "startup"
  level: "info"
  message: "startup"
  timestamp: "2024-03-09T16:00:00.000Z"
trace: {}
```

The flag name, the JSON field name, and the formatted output all tell the same story. No mapping table, no `--help` prose explaining "timeout is in milliseconds" — the suffix is the documentation.

**Secret flags** (`--api-key-secret`, `--database-url-secret`) are automatically redacted in startup messages, logs, and YAML/Plain output. Tools should also consider redacting them from `/proc` process listings where possible.

**Human help vs export surface.** Help scope and help format are orthogonal. Scope is controlled by `--recursive`: `--help` is one-level (and `myapp sub --help` is one-level for that subcommand), while `--help --recursive` expands the selected command subtree. Format is controlled by `--output`: plain by default, or `json|yaml|markdown`. So human-facing CLIs use plain one-level `--help`; agent/doc flows use `--help --recursive` (recursive plain), `--help --recursive --output json|yaml` (recursive export), or `--help --recursive --output markdown` (recursive docs). A bare `--recursive` without `--help` is a no-op for help and MUST NOT be consumed by the help layer — it falls through to the application's own parser. Help `markdown` is help-only and SHOULD NOT become a general business output format.

**Version output.** Agent-first CLIs should handle `--version` before the argument parser's built-in plain-text exit. A bare `--version` should keep conventional human text, while `--version --output json|yaml|plain` MUST honor the requested AFDATA renderer. JSON version output uses `{"kind":"result","result":{"version":"<semver>"}}`. Compatibility wrappers may keep conventional bare text (for example `tool 1.2.3`) as long as an explicit structured `--output` is honored.

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
  manifest_ttl_s: 300

pricing:
  input_msats: 2
  output_msats: 12
```

### TOML

```toml
[cache]
dns_ttl_s = 3600
manifest_ttl_s = 300

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
| `payload_bytes` | `INTEGER` | yes | bytes vs KiB vs count |
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

**Queries**: Column aliases in views or query results should also follow AFDATA naming:

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

## Two Output Paths

### Path 1: Raw JSON Serialization

Return JSON values directly (for example via framework serializer or `serde_json::to_string`).

**No output processing.** Values are serialized as-is:

```json
{"user_id": 123, "api_key_secret": "sk-1234567890abcdef", "balance_msats": 50000}
```

### Path 2: CLI / Logs

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
--log startup,request,progress,retry,redirect
--verbose
```

Default is tool-defined. Interactive CLIs default to `yaml`, scripting/logging contexts to `json`.

JSON is the canonical format. YAML and plain are derived from it.

**All CLI output formats automatically redact `_secret` fields.** Matching recognizes `_secret` and `_SECRET` only. Any `_secret` value — scalar, object, or array — is replaced with `***`. Legacy field names can be protected by passing `OutputOptions.redaction.secret_names` at serialization time; this opt-in list is exact field-name equality. The `Raw` output style disables YAML/plain formatting suffix stripping while keeping the selected redaction policy.

**Format characteristics:**
- **JSON** — single-line, original keys, raw values, no sorting (machine-readable), secrets redacted
- **YAML** — multi-line, human-readable, formatting suffixes stripped, values formatted, secrets redacted by default
- **Plain** — single-line logfmt, human-readable, formatting suffixes stripped, values formatted, secrets redacted by default

### yaml

Each JSON line becomes a YAML document, separated by `---`. Strings always quoted to avoid YAML pitfalls (`no` → `false`, `3.0` → float). In the default readable style, **formatting suffixes are stripped from keys** (value already encodes the unit). **Secrets automatically redacted.**

```yaml
---
kind: "log"
log:
  args:
    config_path: "config.yml"
  config:
    api_key: "***"
    dns_ttl: "3600s"
  event: "startup"
---
kind: "result"
result:
  hash: "abc123"
  size: "446.1KiB"
trace:
  duration: "1.28s"
  cost: "2056msats"
```

### plain

Single-line [logfmt](https://brandur.org/logfmt) style. In the default readable style, **formatting suffixes are stripped from keys.** **Secrets automatically redacted.**

- Nested keys use dot notation: `trace.duration=1.28s`
- Values containing ASCII space, tab, newline, carriage return, form feed, vertical tab, NBSP, `=`, `"`, or `\` are quoted; `\`, `"`, newline, carriage return, tab, form feed, and vertical tab are escaped so each record stays one physical line
- Arrays are comma-joined: `fields=email,age`
- Null values are empty: `RUST_LOG=`

```
kind=log log.args.config_path=config.yml log.config.api_key=*** log.config.dns_ttl=3600s log.event=startup
kind=result result.hash=abc123 result.size=446.1KiB trace.cost=2056msats trace.duration=1.28s
```

### Suffix processing (yaml and plain)

YAML and plain apply two transformations:

**1. Key stripping** — remove the recognized formatting suffix from the key name. The formatted value already encodes the unit, so the suffix is redundant for human readers.

**Algorithm:** match the longest known suffix from the list below. Each suffix is recognized in two forms: lowercase (`_secret`) and uppercase (`_SECRET`). No other casing is matched. Remove the matched suffix from the key. If no suffix matches, keep the key unchanged. Match order (longest first):

1. `_epoch_ms`, `_epoch_s`, `_epoch_ns` (compound timestamp suffixes)
2. `_usd_cents`, `_eur_cents`, `_{code}_cents`, `_{code}_micro` (compound currency suffixes; `code` is 3-4 ASCII letters)
3. `_rfc3339`, `_minutes`, `_hours`, `_days` (multi-char suffixes)
4. `_msats`, `_sats`, `_bytes`, `_percent`, `_secret` (single-unit suffixes)
5. `_jpy`, `_ns`, `_us`, `_ms`, `_s` (short suffixes, matched last to avoid false positives)

Strict string suffixes (`_bcp47`, `_utc_offset`, `_rfc3339_date`, `_rfc3339_time`) are not key-stripping suffixes. They keep the field's format contract visible in readable output.

**Collision:** if two keys in the same object produce the same stripped key (e.g., `response_ms` and `response_bytes` both → `response`), revert both to their original key AND raw value (no formatting). Redaction happens before this step, so collision fallback can never restore a secret value.

| JSON key | YAML/Plain key | Why |
|:---------|:---------------|:----|
| `duration_ms` | `duration` | value shows `1.28s` |
| `size_bytes` | `size` | value shows `446.1KiB` |
| `created_at_epoch_ms` | `created_at` | value shows `2025-02-07T...` |
| `expires_rfc3339` | `expires` | value passes through |
| `api_key_secret` | `api_key` | value shows `***` |
| `cpu_percent` | `cpu` | value shows `85%` |
| `balance_msats` | `balance` | value shows `50000msats` |
| `price_usd_cents` | `price` | value shows `$9.99` |
| `DATABASE_URL_SECRET` | `DATABASE_URL` | uppercase `_SECRET` matched |
| `CACHE_TTL_S` | `CACHE_TTL` | uppercase `_S` matched |
| `buffer_size` | `buffer_size` | `_size` passes through, key unchanged |
| `language_bcp47` | `language_bcp47` | strict string format, key unchanged |
| `timezone_utc_offset` | `timezone_utc_offset` | fixed-offset string, key unchanged |
| `invoice_due_rfc3339_date` | `invoice_due_rfc3339_date` | RFC 3339 full-date string, key unchanged |
| `market_open_rfc3339_time` | `market_open_rfc3339_time` | RFC 3339 partial-time string, key unchanged |
| `config_path` | `config_path` | no suffix, unchanged |
| `user_id` | `user_id` | no suffix, unchanged |

**2. Value formatting** — transform the value for human readability. Same suffix matching as key stripping (lowercase or uppercase only):

- `_ns`, `_us`, `_ms`, `_s` → append unit (`450000ns`, `830μs`, `42ms`, `3600s`)
- `_ms` with absolute value ≥ 1000 → convert to seconds (`1280` → `1.28s`, `-1500` → `-1.5s`)
- `_minutes`, `_hours`, `_days` → append unit (`30 minutes`, `24 hours`)
- `_epoch_ms` / `_epoch_s` / decimal-string `_epoch_ns` → RFC 3339 (`2024-02-14T00:00:00.000Z`), negative values produce pre-1970 dates
- `_rfc3339` → pass through
- `_bytes` → human-readable (`456789` → `446.1KiB`); negative and fractional byte values fall through as raw values
- `_size` → pass through (config input string, e.g. `"10MiB"` stays `"10MiB"`)
- `_percent` → append `%` (`85` → `85%`, `99.9` → `99.9%`)
- `_msats` → append unit (`2056msats`)
- `_sats` → append unit (`1234sats`)
- `_usd_cents` → dollars (`999` → `$9.99`), negative falls through
- `_eur_cents` → euros (`850` → `€8.50`), negative falls through
- other `_{code}_cents` → major unit with code (`15050` → `150.50 THB`), where `code` is 3-4 ASCII letters; negative falls through
- `_{code}_micro` → major unit with six decimals and code (`170000` → `0.170000 USD`), where `code` is 3-4 ASCII letters; negative falls through
- `_jpy` → yen (`1500` → `¥1,500`), negative falls through
- `_secret` → `***` (already applied by the redaction phase; the formatter does not perform a second, divergent redaction pass)

Strict string fields such as `_bcp47`, `_utc_offset`, `_rfc3339_date`, and `_rfc3339_time` are not value-formatting suffixes; their string values pass through unchanged.

A `_url` field value is preserved byte-for-byte in YAML and plain except for the redacted secret spans (userinfo password, `_secret`-suffixed/`secret_names` query parameters): the `_url` key is not stripped, and formatting suffixes that appear *inside* the URL — `?timeout_ms=5000`, `?size_bytes=1048576` — are **not** reformatted (`5s`, `1.0MiB`) or stripped, because the URL must round-trip to its server exactly. URL key-stripping/value-formatting applies to JSON object keys, never to query parameters inside a string value. This is pinned by the `url_params_redacted_not_reformatted` case in [`spec/fixtures/output_formats.json`](fixtures/output_formats.json).

**Type constraints**: `_bytes` and `_epoch_*` require integer values. `_usd_cents`, `_eur_cents`, `_jpy`, `_{code}_cents`, and `_{code}_micro` require non-negative integers. Duration, Bitcoin, and `_percent` suffixes accept any number. When the value type doesn't match, formatting falls through to the raw value with the original key preserved. An **integral-valued float** counts as an integer for the integer-required suffixes (`3.0` is treated as `3`): a JSON number's value, not its lexical form, decides, because JavaScript cannot distinguish `3` from `3.0` after parsing.

**Number rendering**: a number is rendered for YAML/plain by the shared fixture-defined decimal form: integral-valued floats drop their trailing `.0` (`3.0` → `3`), exponent markers use lowercase `e`, and exponent signs/leading zeroes are normalized (`1e-07` → `1e-7`). Integers beyond 2⁵³ are preserved exactly by Rust, Go, and Python; JavaScript loses precision on them (see the `_epoch_ns` precision note above).

### Key ordering

YAML and plain output sort keys (after stripping) by UTF-16 code unit order (JCS, [RFC 8785](https://www.rfc-editor.org/rfc/rfc8785) §3.2.3). For ASCII keys — the common case — this equals simple byte-order sorting.

In plain logfmt, nested keys are flattened to dot notation before sorting. Sort by the full dot path: `args.input_path` < `code` < `config.api_key` < `trace.duration`.

JSON output is unordered per the JSON specification. YAML and plain sort for deterministic, cross-language-consistent output.

## Using AFDATA Without Part 3

Parts 1 and 2 (naming + output processing) work with any JSON structure — no protocol template needed:

```json
{"user_id": 123, "created_at_epoch_ms": 1738886400000, "balance_msats": 50000000, "api_key_secret": "sk-..."}
```

Plain: `api_key=*** balance=50000000msats created_at=2025-02-07T00:00:00.000Z user_id=123`

This works with REST APIs, GraphQL, database results, config files — anywhere you have structured data. Just use AFDATA naming and let output processing handle the rest.

---

# Part 3: Protocol Template (Recommended, Optional)

A recommended structure for program output. This part is **optional** — adopt it when you want consistent structure across CLI tools, streaming output, or internal protocols.

## Core Fields

**Required:**
- `kind` — protocol discriminator: `"result"`, `"error"`, `"progress"`, or `"log"`
- a payload field whose name matches `kind`

**Recommended:**
- `trace` — execution context (duration, source, resource usage)

`trace`, when present, is a JSON object. `result`, `progress`, and `log`
payloads are tool-defined valid JSON values. `error` is a JSON object with
required non-empty `code` and `message`, optional `hint`, and tool-defined
extension fields.

## CLI Event Framing

Structured CLI programs emit complete AFDATA protocol v1 events to stdout.
Framing depends on `--output`:

- JSON multi-event output is JSONL/NDJSON: one complete event per line.
- Plain multi-event output is one display event per line.
- YAML multi-event output uses an explicit `---` document boundary for every event.

Agent-facing machine input remains JSON. YAML and plain are display formats.

Channel policy:
- `stdout` is the only protocol/log stream for machine-readable events
- runtime protocol events MUST NOT be emitted on `stderr`
- `stderr` may be used only for unrecoverable pre-protocol startup failures where structured output cannot be produced

Optional stream redirection:
- CLI tools and services MAY expose `--stdout-file <PATH>` and `--stderr-file <PATH>`
- unset file flags leave the corresponding stream unchanged
- when enabled, stdout bytes are appended to the `--stdout-file` path instead of the original stdout destination
- when enabled, stderr bytes are appended to the `--stderr-file` path instead of the original stderr destination
- `--output` continues to select stdout format (`json`, `yaml`, `plain`, and help-specific `markdown`); it does not select stream destinations
- implementations SHOULD install stream redirection before version/help handling, logging/tracing initialization, and other early output
- startup failures to create/open the files SHOULD fail startup with a structured stdout error when stdout is still available
- stderr MUST NOT be converted to AFDATA JSON; native diagnostics such as Rust panics, Python tracebacks, and runtime errors remain stderr bytes
- no application-level rotation is implied; rotate with external tooling
- this is stream redirection, not a second AFDATA protocol channel and not stream copying

Recommended enforcement:
- Rust: clippy `print_stderr = "deny"` plus disallow `std::eprintln` / `std::io::stderr`
- Go/Python/TypeScript: source-policy tests or lint rules that fail on stderr API usage in runtime code

Finite structured CLI event streams follow:

```text
(log | progress)* -> exactly one (result | error) -> end
```

Log and progress payloads are tool-defined JSON values with no required or reserved payload fields. Traditional logging adapters commonly add `message` and `level`; progress producers may add a human-readable `message`. These are conventions, not protocol requirements. Projects that need timestamps add `timestamp_epoch_ms` explicitly.

Log fields are redacted **by field name** at emit time — the same `_secret`/`_url` rule as all other output, applied by the formatter, not by scanning rendered values. Emit secrets as named fields (`api_key_secret`) so the rule can see them. Logging a whole object pre-rendered to a single string (e.g. a language's debug/inspect form) defeats redaction, because the inner field names are no longer visible: build a structured value and redact it before logging instead.

The top-level `kind` values are reserved: `log`, `progress`, `result`, and `error`. Tool-defined codes belong inside the corresponding payload.

**Error payload codes:** Use specific codes instead of generic `"error"`:
- `"not_found"`, `"unauthorized"`, `"validation_error"`, `"rate_limit"`, `"internal_error"`, etc.
- Generic `"error"` is supported but specific codes are preferred

Progress and log payloads may add tool-defined fields such as `event: "request"` or `phase: "sync"`.

Not all phases are required. A simple CLI tool may emit only a result line. A long-running service may never emit a result.

### Startup Diagnostic Event

`kind: "log"` with `log.event: "startup"`. Optional. Emitted once at the beginning if diagnostic logging is enabled.

```json
{"kind":"log","log":{"message":"startup","level":"info","event":"startup","version":"0.1.0","argv":["tool","--log","startup"],"config":{"api_key_secret":"***","dns_ttl_s":3600},"args":{"config_path":"config.yml"},"env":{"RUST_LOG":null,"DATABASE_URL_SECRET":"***"}},"trace":{}}
```

Startup payload fields are tool-defined. Common fields:
- `version` — tool version string
- `argv` — raw CLI argv array
- `config` — resolved configuration (recommended)
- `args` — parsed CLI arguments (optional)
- `env` — environment variables the program reads (`null` if unset, optional)

### Status

`kind` is the protocol discriminator. `progress` and `log` payload content is tool-defined. Include `trace` for execution context when it helps debugging.

```json
{"kind": "progress", "progress": {"current": 3, "total": 10, "message": "indexing spores"}, "trace": {"duration_ms": 500}}
```

```json
{"kind":"log","log":{"message":"POST /v1/chat completed","level":"info","event":"request","method":"POST","path":"/v1/chat","http_status":200},"trace":{"latency_ms":42}}
```

### Result

`kind:"result"` MUST be emitted only when the command intent was completely
fulfilled. Any incomplete fulfillment, including partial completion, MUST emit
`kind:"error"`. An agent watching a finite stream can treat either as the
unique terminal event.

**Always include `trace`** for execution context — duration, data sources, resource usage, query details.

**Success:**
```json
{"kind": "result", "result": {"hash": "abc123", "size_bytes": 456789}, "trace": {"duration_ms": 1280, "tokens_input": 512}}
```

**Error:**

Simple message:
```json
{"kind": "error", "error": {"code": "config_not_found", "message": "config file not found", "retryable": false}, "trace": {"duration_ms": 3}}
```

With actionable hint:
```json
{"kind": "error", "error": {"code": "connection_refused", "message": "connection refused", "retryable": false, "hint": "check --host/--port or PGHOST/PGPORT environment variables"}, "trace": {"duration_ms": 3}}
```

The `hint` field is optional. When present, it provides an actionable suggestion for the user or agent to resolve the error. Omit `hint` when no specific remediation is available.

Error details are direct extension fields inside the error payload:
```json
{"kind": "error", "error": {"code": "not_found", "message": "user not found", "retryable": false, "resource": "user", "id": 123}, "trace": {"duration_ms": 8}}
```

More examples:
```json
{"kind": "error", "error": {"code": "validation_error", "message": "invalid fields", "retryable": false, "fields": ["email", "age"]}, "trace": {"duration_ms": 2}}
{"kind": "error", "error": {"code": "unauthorized", "message": "invalid token", "retryable": false}, "trace": {"duration_ms": 5}}
{"kind": "error", "error": {"code": "rate_limit", "message": "rate limited", "retryable": false, "retry_after_s": 60, "quota_remaining": 0}, "trace": {"duration_ms": 1}}
```

### Best Practices

**Always include `trace` field.** Even simple operations should report execution context:

- `duration_ms` — operation duration
- `source` — data source (db, cache, api, file)
- Resource usage — `tokens_input`, `tokens_output`, `cost_msats`, `memory_bytes`
- Metadata — `query`, `method`, `path`, `model`

**Good (with trace):**
```json
{"kind": "result", "result": {"count": 42}, "trace": {"duration_ms": 150, "source": "db"}}
{"kind": "error", "error": {"code": "not_found", "message": "not found", "retryable": false}, "trace": {"duration_ms": 5}}
```

**Also good:**
```json
{"kind": "result", "result": {"count": 42}, "trace": {"duration_ms": 150, "source": "db"}}
{"kind": "error", "error": {"code": "validation_error", "message": "invalid input", "retryable": false, "fields": [...]}, "trace": {"duration_ms": 2}}
```

**Avoid (missing trace):**
```jsonc
{"kind": "result", "result": {"count": 42}}
{"kind": "error", "error": {"code": "not_found", "message": "not found", "retryable": false}}
```

Missing `trace` makes debugging harder. Agents can't analyze performance, cost, or data flow without execution context.

### Validation profiles

The `validate_protocol_event(event, strict)` / `validate_protocol_stream(events, strict)` APIs enforce protocol compliance. With `strict=false`, only mandatory MUST rules are enforced: envelope shape, error payload requirements, and finite-stream lifecycle. With `strict=true` (the default in Python/TS), additional recommendations are required:

- every event includes an object-valued `trace`
- every error payload includes `retryable` as a boolean

Passing the base validator proves mandatory conformance only. Use the strict validator when claiming conformance with these recommendations. Structured version metadata may intentionally use the base profile when no execution trace exists.

### Agent consumption

1. Read `kind` on every line and read the same-named payload (`event[kind]`).
2. `kind:"log"` with `log.event:"startup"` describes resolved startup configuration.
3. `kind:"result"` or `kind:"error"` completes a finite operation.
4. `kind:"log"` and `kind:"progress"` are non-terminal events before that terminal event.

## Usage in HTTP Services

The protocol structure can be used in REST APIs. Choose output path explicitly:
- raw JSON serialization for untouched payloads
- formatter output (`json|yaml|plain`) when redaction/formatting is required

### REST API Examples

Response body follows the protocol structure:

**HTTP 200:**
```json
{"kind": "result", "result": {"balance_msats": 97900}, "trace": {"source": "redb", "duration_ms": 3}}
```

**HTTP 404:**
```json
{"kind": "error", "error": {"code": "not_found", "message": "user not found", "retryable": false, "resource": "user", "id": 123}, "trace": {"duration_ms": 5}}
```

**HTTP 402:**
```json
{"kind": "error", "error": {"code": "insufficient_balance", "message": "insufficient balance", "retryable": false, "balance_msats": 0, "required_msats": 2056}, "trace": {"source": "redb", "duration_ms": 2}}
```

### MCP Tool Response

Same structure, raw JSON:

```json
{"kind": "result", "result": {"files": ["src/main.rs"]}, "trace": {"source": "glob", "matched": 1, "duration_ms": 12}}
```

### Streaming (SSE)

JSONL stream, raw JSON per line:

```json
{"kind":"log","log":{"message":"startup","level":"info","event":"startup","config":{"model":"gpt-4","max_tokens":1024},"args":{},"env":{}},"trace":{}}
{"kind": "progress", "progress": {"current": 1, "total": 5, "message": "processing"}, "trace": {"duration_ms": 500}}
{"kind": "result", "result": {"answer": "..."}, "trace": {"tokens_input": 512, "duration_ms": 1280}}
```

### One Protocol, Multiple Contexts

| Context | Output | Secret Protection |
|:--------|:-------|:------------------|
| **CLI / Logs** | JSONL, one-line plain events, or YAML documents | ✅ Automatic |
| **HTTP body (raw path)** | JSON body (raw Value) | Use `redacted_value` before framework serialization |
| **MCP tool (raw path)** | JSON (raw Value) | Use `redacted_value` before SDK serialization |
| **SSE stream (raw path)** | JSONL (raw JSON) | Use `redacted_value` before emitting events |

All contexts can use the protocol structure from Part 3. `kind`, its matching
payload field, and optional object-valued `trace` are standardized. CLI/logs
apply output formatting and secret protection from Part 2. Raw-path serializers
return JSON values unchanged unless the program explicitly calls
`redacted_value`. For CLI/log protocol transport, use `stdout` only; do not
split protocol events across `stdout` and `stderr`.

---

# Complete Example: CLI Tool

A complete example showing all three parts working together. A backup tool that uploads files to cloud storage.

## CLI Invocation

```bash
cloudback --api-key-secret sk-1234567890abcdef --timeout-s 30 --max-file-size-bytes 10737418240 /data/backup.tar.gz
```

Flag names use AFDATA suffixes in kebab-case. An agent reading `--help` knows `--timeout-s` is seconds and `--api-key-secret` should be redacted — no documentation needed.

## Raw JSON (before output processing)

The tool converts CLI flags from kebab-case to snake_case and emits a startup diagnostic event when enabled:

```json
{
  "kind": "log",
  "log": {
    "timestamp_epoch_ms": 1710000000000,
    "message": "startup",
    "level": "info",
    "event": "startup",
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
  },
  "trace": {}
}
```

Field names encode semantics:
- `api_key_secret` → agent knows to redact
- `timeout_s` → 30 seconds
- `max_file_size_bytes` → 10GiB in bytes

## Output Formats (Part 2: Output Processing)

**JSON** (raw, for machines):
```json
{"kind":"log","log":{"message":"startup","level":"info","event":"startup","config":{"api_key_secret":"***","endpoint":"https://storage.example.com","timeout_s":30,"max_file_size_bytes":10737418240},"args":{"input_path":"/data/backup.tar.gz","compression_level":9}},"trace":{}}
```

**YAML** (structured, formatting suffixes stripped, for human inspection):
```yaml
---
kind: "log"
log:
  args:
    compression_level: 9
    input_path: "/data/backup.tar.gz"
  config:
    api_key: "***"
    endpoint: "https://storage.example.com"
    max_file_size: "10.0GiB"
    timeout: "30s"
  event: "startup"
  level: "info"
  message: "startup"
trace: {}
```

**Plain** (single-line logfmt, formatting suffixes stripped, for compact scanning):
```
kind=log log.args.compression_level=9 log.args.input_path=/data/backup.tar.gz log.config.api_key=*** log.config.endpoint=https://storage.example.com log.config.max_file_size=10.0GiB log.config.timeout=30s log.event=startup log.level=info log.message=startup
```

Note:
- **Key stripping**: formatting suffixes such as `api_key_secret` → `api_key`, `timeout_s` → `timeout`, `max_file_size_bytes` → `max_file_size`
- **Secret protection**: `api_key_secret` redacted in all three formats
- **Suffix formatting**: `_bytes` → `10.0GiB`, `_s` → `30s` in YAML and Plain

## Progress Update (Part 3: Protocol Template)

```json
{"kind":"progress","progress":{"current":3,"total":10,"message":"uploading chunks"},"trace":{"duration_ms":5420,"uploaded_bytes":3221225472}}
```

YAML:
```yaml
---
kind: "progress"
progress:
  current: 3
  message: "uploading chunks"
  total: 10
trace:
  duration: "5.42s"
  uploaded: "3.0GiB"
```

Plain:
```
kind=progress progress.current=3 progress.message="uploading chunks" progress.total=10 trace.duration=5.42s trace.uploaded=3.0GiB
```

## Final Result

```json
{"kind": "result", "result": {"backup_url": "https://storage.example.com/backup.tar.gz", "size_bytes": 10485760, "checksum": "sha256:abc123...", "uploaded_at_epoch_ms": 1738886400000}, "trace": {"duration_ms": 15300, "chunks": 10, "retries": 2}}
```

YAML:
```yaml
---
kind: "result"
result:
  backup_url: "https://storage.example.com/backup.tar.gz"
  checksum: "sha256:abc123..."
  size: "10.0MiB"
  uploaded_at: "2025-02-07T00:00:00.000Z"
trace:
  chunks: 10
  duration: "15.3s"
  retries: 2
```

Plain:
```
kind=result result.backup_url=https://storage.example.com/backup.tar.gz result.checksum=sha256:abc123... result.size=10.0MiB result.uploaded_at=2025-02-07T00:00:00.000Z trace.chunks=10 trace.duration=15.3s trace.retries=2
```

## What This Demonstrates

1. **Part 1 (Naming)**: Every field is self-describing — from CLI flags (`--timeout-s`, `--api-key-secret`) to JSON fields (`timeout_s`, `uploaded_at_epoch_ms`). Same suffixes, same semantics, kebab↔snake mapping

2. **Part 2 (Output Processing)**: Three formats for different needs
   - JSON: single-line, original keys, raw values, for programs and logs
   - YAML: multi-line, formatting suffixes stripped, values formatted, for human inspection
   - Plain: single-line logfmt, formatting suffixes stripped, values formatted, for compact scanning
   - All formats protect secrets automatically

3. **Part 3 (Protocol)**: Consistent structure across all output — `code` identifies message type, `trace` provides execution context, other fields flexible

**Key insight**: The same naming convention flows from CLI flag (`--timeout-s 30`) to JSON field (`timeout_s: 30`) to formatted output (`timeout: 30s`). An agent reading `--help`, JSON output, or YAML all gets the same self-describing semantics — no documentation needed at any layer.
