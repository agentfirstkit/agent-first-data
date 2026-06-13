---
name: agent-first-data
description: Apply Agent-First Data naming and output conventions when writing structured data, configs, logs, transport payloads, or CLI output in any language.
disable-model-invocation: true
allowed-tools: Bash, Read, Edit, Write, Glob, Grep
---

# Agent-First Data

This skill content is tool-agnostic. It can be used by any AI coding agent workflow.
The frontmatter keys at the top are metadata for skill runners and do not change the AFDATA conventions.

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
| `_rfc3339` | RFC 3339 date-time string | `expires_rfc3339: "2026-02-14T10:30:00Z"` |

### Strict string formats

| Suffix | Format | Example |
|:-------|:-------|:--------|
| `_bcp47` | BCP-47 language tag string | `language_bcp47: "zh-CN"` |
| `_utc_offset` | fixed UTC offset string | `timezone_utc_offset: "+08:00"` |
| `_rfc3339_date` | RFC 3339 full-date string | `invoice_due_rfc3339_date: "2026-06-13"` |
| `_rfc3339_time` | RFC 3339 partial-time string | `market_open_rfc3339_time: "09:30:00"` |

`*_bcp47` identifies a BCP-47 language tag string. AFDATA does not implement the full BCP-47 registry; tools may validate tags when needed.

`*_utc_offset` identifies a fixed UTC offset. Canonical persisted and structured output values are `"UTC"` or `±HH:MM`, with `HH` in `00..23` and `MM` in `00..59`; zero offsets normalize to `"UTC"`. This is not an IANA timezone name, DST rule, or timezone database field.

`*_rfc3339_date` identifies an RFC 3339 `full-date` string (`YYYY-MM-DD`). It is a calendar date, not an instant, and has no time, offset, or timezone.

`*_rfc3339_time` identifies an RFC 3339 `partial-time` string (`HH:MM:SS[.fraction]`). It is a time-of-day, not an instant, and MUST NOT include `Z`, `±HH:MM`, IANA timezone names, or other timezone annotations. Use `_rfc3339` or `_epoch_*` for instants.

Do not create companion timezone-name fields as an AFDATA core pattern. If a future tool needs IANA timezone semantics with a timestamp, prefer a self-contained standard value such as RFC 9557.

Avoid magic string sentinels such as `"auto"` inside strict-format fields. If a tool needs auto/default behavior, define it in that tool's own config semantics, not as an AFDATA-wide rule.

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

Fiat — `_{iso4217}_cents` for currencies with 1/100 subdivision, `_{iso4217}` for currencies without. Generic `_{code}_cents` matches only 3-4 ASCII letters:

| Suffix | Example |
|:-------|:--------|
| `_usd_cents` | `price_usd_cents: 999` |
| `_eur_cents` | `price_eur_cents: 850` |
| `_jpy` | `price_jpy: 1500` |
| `_usdt_cents` | `deposit_usdt_cents: 1000` |

### Sensitive

| Suffix | Handling | Example |
|:-------|:---------|:--------|
| `_secret` | redact the entire value/subtree to `***` | `api_key_secret: "sk-or-v1-abc..."` |
| `_url` | scrub secrets *inside* the URL value, keep the rest | `callback_url: "https://h/cb?code_secret=..."` |

All CLI output formats (JSON, YAML, Plain) automatically redact `_secret` fields. Matching recognizes `_secret` and `_SECRET` only — no mixed case. The entire `_secret` value/subtree becomes `***`, including objects and arrays. For legacy fields that cannot be renamed, configure `OutputOptions.redaction` with `secret_names`/`secretNames` such as `["api_key", "authorization"]`; names match exact field names at any nesting level; no trim, case folding, hyphen/underscore normalization, globs, regex, or substring matching. Callers that need schema-preserving YAML/plain rendering can pass `OutputOptions` with the `Raw` output style.

Name URL-valued fields `_url` so the userinfo password and any `_secret`/`secret_names` query parameter inside them are scrubbed automatically (the rest of the URL is preserved; the suffix is not stripped). For a URL inside a free-form message, redact it with `redact_url_secrets` before interpolating — `_url` only fires on whole-URL field values, never on prose.

**`_url` scrubs the userinfo password and suffix-named params only — not arbitrary credential params.** Common parameters like `?access_token=`, `?api_key=`, `?code=`, `?sig=` are NOT redacted unless their name ends in `_secret` or is passed in `secret_names`. Rename params you own to the suffix (`?access_token_secret=`); list the rest in `secret_names`. A `_url` value that is not a clean scheme-prefixed URL but carries internal whitespace or an `@` credential sigil (e.g. a schemeless `user:pass@host/db`) is redacted wholesale to `***` (fail-closed).

### Environment variables

Same suffixes, `UPPER_SNAKE_CASE`:

```
DATABASE_URL_SECRET=postgres://user:pass@host/db
CACHE_TTL_S=3600
TOKEN_VALIDITY_HOURS=24
```

### No suffix needed

Fields whose meaning is obvious: `redb_path`, `proof_count`, `search_enabled`, `method`, `domain`, `model`. (URL fields are the exception — use `_url` so embedded secrets are scrubbed.)

### Database columns

Use suffixes on generic types (`INTEGER`, `BIGINT`, `TEXT`). Native types that carry semantics (`TIMESTAMPTZ`, `INTERVAL`) don't need suffixes.

| Column | Type | Suffix? | Why |
|:-------|:-----|:--------|:----|
| `created_at` | `TIMESTAMPTZ` | no | type says timestamp |
| `duration_ms` | `INTEGER` | yes | integer is ambiguous |
| `api_key_secret` | `TEXT` | yes | enables auto-redaction |
| `retry_count` | `INTEGER` | no | meaning obvious |

ORM struct fields preserve the suffix: `duration_ms: i64`, not `duration: i64`.

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

Three output formats. Default YAML and Plain apply key stripping + value formatting.

### Formats

- **JSON** — single-line, original keys, raw values, no sorting (machine-readable), secrets redacted
- **YAML** — multi-line, formatting suffixes stripped, values formatted, secrets redacted by default
- **Plain** — single-line logfmt, formatting suffixes stripped, values formatted, secrets redacted by default

### Key stripping (YAML and Plain)

Remove recognized formatting suffix from key. Longest match first, exact lowercase or uppercase only:

1. `_epoch_ms`, `_epoch_s`, `_epoch_ns`
2. `_usd_cents`, `_eur_cents`, `_{code}_cents` (`code` is 3-4 ASCII letters)
3. `_rfc3339`, `_minutes`, `_hours`, `_days`
4. `_msats`, `_sats`, `_bytes`, `_percent`, `_secret`
5. `_btc`, `_jpy`, `_ns`, `_us`, `_ms`, `_s`

`_size`, `_bcp47`, `_utc_offset`, `_rfc3339_date`, and `_rfc3339_time` are NOT stripped (pass through). If two keys collide after stripping, both revert to original key AND raw value (no formatting). Redaction runs before collision handling, so fallback never restores a secret.

### Value formatting (YAML and Plain)

- `_ms` with absolute value < 1000 → `{n}ms`; absolute value ≥ 1000 → seconds (`1280` → `1.28s`, `-1500` → `-1.5s`)
- `_s`, `_ns`, `_us` → append unit (`3600s`, `450000ns`, `830μs`)
- `_minutes`, `_hours`, `_days` → append unit (`30 minutes`)
- `_epoch_ms`/`_epoch_s`/`_epoch_ns` → RFC 3339 (negative = pre-1970)
- `_rfc3339` → pass through
- `_bytes` → human-readable (`456789` → `446.1KB`, `-5242880` → `-5.0MB`)
- `_size` → pass through
- `_percent` → append `%`
- `_msats` → `{n}msats`, `_sats` → `{n}sats`, `_btc` → `{n} BTC`
- `_usd_cents` → `$X.XX`, `_eur_cents` → `€X.XX`, `_jpy` → `¥X,XXX`, `_{code}_cents` → `X.XX CODE` where `code` is 3-4 ASCII letters
- `_secret` → `***` (the redaction phase already replaced the subtree)
- `_bcp47`, `_utc_offset`, `_rfc3339_date`, `_rfc3339_time` → pass through unchanged

**Type constraints**: `_bytes`/`_epoch_*` require integer. `_usd_cents`/`_eur_cents`/`_jpy`/`_{code}_cents` require non-negative integer. Duration/Bitcoin/`_percent` accept any number. Wrong type → raw value + original key.

### Plain logfmt details

- Nested keys use dot notation: `trace.duration=1.28s`
- Keys and values with ASCII space, tab/newline, VT, FF, NBSP, `=`, `"`, or `\` are quoted/escaped so each record stays one physical line
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
| `"log"` | Diagnostic event (`event` field identifies startup/request/progress/retry/redirect) |
| tool-defined | Status/progress (`"request"`, `"progress"`, `"sync"`, etc.) |
| `"ok"` | Success result |
| `"error"` | Error result |

Channel policy:
- `stdout` is the only protocol/log stream for machine-readable events
- runtime protocol events MUST NOT be emitted on `stderr`
- `stderr` is reserved for unrecoverable pre-protocol startup failures only

Recommended enforcement:
- Rust: enable clippy `print_stderr = "deny"` and disallow `std::eprintln` / `std::io::stderr`
- Go/Python/TypeScript: add source-policy tests that fail if runtime sources reference stderr APIs

### Templates

```json
{"code": "log", "event": "startup", "version": "0.1.0", "argv": ["tool", "--log", "startup"], "config": {...}, "args": {...}, "env": {...}}
{"code": "ok", "result": {...}, "trace": {"duration_ms": 12, "source": "redb"}}
{"code": "error", "error": "message", "trace": {"duration_ms": 3}}
{"code": "not_found", "resource": "user", "id": 123, "trace": {"duration_ms": 8}}
```

Always include `trace` for execution context: duration, token counts, cost, data source.
Startup payload fields are tool-defined; `config` is recommended, while `version`/`argv`/`args`/`env` are optional.

### Same structure, any transport

| Transport | Format |
|:----------|:-------|
| CLI stdout | JSONL |
| REST API | JSON body |
| MCP tool | JSON |
| SSE stream | JSONL |

All use `code` / `result` / `error` / `trace`. Do not split protocol events across `stdout` and `stderr`.

---

## Library Usage

Use the local language README for installation and the full overview/spec for API reference. Keep this skill focused on naming, output, protocol, logging, and review rules rather than duplicating import snippets that drift across languages.

Required cross-language behavior to rely on:

- Output helpers redact before formatting.
- Use `redacted_value` for raw HTTP/MCP/SSE serialization paths that bypass `output_json`.
- Use `redact_secrets_in_place` only when mutating an existing JSON value is intentional; otherwise prefer copy-returning redactors.
- Use `cli_parse_output`, `cli_parse_log_filters`, `cli_output`, `build_cli_error`, and the version helper for CLI tools instead of custom parsing/error envelopes.
- `build_cli_error(message, hint?)` returns `{code:"error", error: message, hint?}` only.
- Rust CLIs should call `cli_handle_version_or_continue()` before clap parsing so `--version --output json|yaml|plain` emits `{code:"version", version}` instead of clap's plain text. Use `VersionConfig::conventional_default()` so bare `--version` stays human text while explicit `--output` remains structured.

## AFDATA Logging

Structured logging that outputs via the library's own `output_json`/`output_plain`/`output_yaml`. Each language integrates with its native logging ecosystem. All three formats apply the same suffix processing, key stripping, and secret redaction as the core output API.

### Init (pick one format per process)

| Format | Rust | Go | Python | TypeScript |
|:-------|:-----|:---|:-------|:-----------|
| **JSON** | `afdata_tracing::init_json(filter)` | `afdata.InitJson()` | `init_logging_json("INFO")` | `initJson()` |
| **Plain** | `afdata_tracing::init_plain(filter)` | `afdata.InitPlain()` | `init_logging_plain("INFO")` | `initPlain()` |
| **YAML** | `afdata_tracing::init_yaml(filter)` | `afdata.InitYaml()` | `init_logging_yaml("INFO")` | `initYaml()` |

Rust requires `cargo add agent-first-data --features tracing`.

### Spans (add fields to all log events in scope)

```rust
// Rust — tracing spans
let span = info_span!("request", request_id = %uuid);
let _guard = span.enter();
```

```go
// Go — context-based
ctx := afdata.WithSpan(ctx, map[string]any{"request_id": uuid})
logger := afdata.LoggerFromContext(ctx)
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

Every log line contains `timestamp_epoch_ms`, `message`, `code: "log"`, `level` (debug/info/warn/error), plus span fields and event fields. Do not use the log level as `code`; `code:"error"` is reserved for terminal protocol errors.

Log redaction is **by field name** (the same `_secret`/`_url` rule as all output), applied when the line is emitted. So name the secret field — `info!(api_key_secret = %key)` — rather than logging a whole object by its `Debug`/string rendering, which hides the inner field names from redaction. For structured/nested secret-bearing data, build a value, redact it (`redact_secrets_in_place`), then emit via `output_*` — do not pass the struct to a `?`/`%`-rendered log field.

## CLI Flags

CLI tools that use AFDATA should support output and logging flags:

```
--output json|yaml|plain    # default is tool-defined (interactive → yaml, scripting/logging → json)
--log startup,request,progress,retry,redirect
--verbose                   # shorthand for all log categories
```

- Protocol output (`build_json_*` + `output_*`) follows `--output`
- Log format follows `--output` or a separate `--log-format` flag if independent control is needed
- Help scope and format are orthogonal: `--recursive` decides one-level vs recursive, `--output` decides plain/json/yaml/markdown. Human help is one-level `--help` (and scoped `myapp sub --help`); agents/docs use `--help --recursive` and add `--output json|yaml|markdown` for a recursive export. A bare `--recursive` without `--help` falls through to the app's own parser. `markdown` is help-only. Rust: use `cli_handle_help_or_continue()` / `cli_render_help_with_options()` from the `cli-help` feature; wrappers `cli_render_help()` and `cli_render_help_markdown()` remain available for recursive output.

## Review Checklist

When reviewing code that produces structured output:

1. Every numeric field with a unit has the correct suffix (`_ms`, `_bytes`, `_sats`, `_percent`, etc.)
2. Timestamps use `_epoch_ms` / `_epoch_s` / `_rfc3339`; date-only/time-only strings use `_rfc3339_date` / `_rfc3339_time`
3. Sensitive values end in `_secret` and are redacted in all output paths
4. Transport payloads / CLI output use `code` / `result` / `error` / `trace` structure
5. Config files use the same suffixes as output
6. No unit-less ambiguous fields (`timeout: 30` — 30 what?)
7. Config size values use `_size` suffix (`buffer_size: "10M"`, not `buffer: "10M"`)
8. Environment variables follow `UPPER_SNAKE_CASE` with the same suffixes
9. Logging uses AFDATA init functions (`init_json`/`init_plain`/`init_yaml`) — not raw `println!`/`fmt.Println`/`console.log` for structured output
10. Database columns use AFDATA suffixes on generic types (`duration_ms INTEGER`, not `duration INTEGER`); native types like `TIMESTAMPTZ` don't need suffixes
11. CLI flag parsing uses `cli_parse_output`/`cli_parse_log_filters`/`build_cli_error`/version helpers — not custom reimplementations; uses `try_parse()` not `parse()` in Rust so clap errors go to stdout as JSONL
