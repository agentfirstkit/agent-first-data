<!-- Canonical source: skills/agent-first-data/references/rules.md.
     Mirrored byte-for-byte into go/python/typescript assets/ by scripts/sync_offline_assets.py.
     Edit only here; run that script (checked by scripts/test.sh) to propagate. -->

# Agent-First Data detailed rules

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
| `_epoch_ns` | nanoseconds since Unix epoch as a decimal string | `created_epoch_ns: "1707868800000000000"` |
| `_rfc3339` | RFC 3339 date-time string | `expires_rfc3339: "2026-02-14T10:30:00Z"` |

Integer vs string is not uniform across siblings: `_epoch_s` and `_epoch_ms` are JSON integers, but `_epoch_ns` is a decimal **string** because nanoseconds exceed the 2⁵³−1 safe-integer range. The same rule applies to large `_sats`/`_msats` (integer when safe, decimal string when not). Writing `_epoch_ns` as a bare JSON number silently loses precision — `afdata lint` rejects it (`suffix_type_mismatch`).

`_rfc3339` needs a **mandatory** offset: `YYYY-MM-DDThh:mm:ss[.fff]` followed by `Z` or `±HH:MM`. A bare `2026-02-14T10:30:00` (no offset), a space instead of `T`, or a trailing IANA name (`...Asia/Shanghai`) is rejected by `afdata lint`. `afdata lint` also type-checks the numeric suffixes (durations `_s`/`_ms`/…, currency `_cents`/`_micro`/`_jpy`) and rejects a `_url` with internal whitespace or bare `user:pass@host` credentials.

A `null` value is exempt from every suffix type constraint above: `null` means the field is absent/unset, not present-with-the-wrong-type, so `law_repeal_at_epoch_s: null` is not a `suffix_type_mismatch`. Absence may be written as an omitted key or as an explicit `null` — both are valid. The constraint applies only once a value is present and non-null.

### Strict string formats

| Suffix | Format | Example |
|:-------|:-------|:--------|
| `_bcp47` | BCP-47 language tag string | `language_bcp47: "zh-CN"` |
| `_utc_offset` | fixed UTC offset string | `timezone_utc_offset: "+08:00"` |
| `_rfc3339_date` | RFC 3339 full-date string | `invoice_due_rfc3339_date: "2026-06-13"` |
| `_rfc3339_time` | RFC 3339 partial-time string | `market_open_rfc3339_time: "09:30:00"` |

`*_bcp47` identifies a BCP-47 language tag string. AFDATA validates structure (hyphen-separated subtags, 2–3 letter primary language), so the POSIX form `zh_CN` and an over-long primary like `chinese` are rejected; use `zh-CN`. It does not check the IANA registry, so a well-formed-but-unregistered tag like `zz-ZZ` still passes. `is_valid_bcp47` and `afdata lint` apply this check.

`*_utc_offset` identifies a fixed UTC offset. Canonical persisted and structured output values are `"UTC"` or `±HH:MM`, with `HH` in `00..23` and `MM` in `00..59`; zero offsets normalize to `"UTC"`. This is not an IANA timezone name, DST rule, or timezone database field.

`*_rfc3339_date` identifies an RFC 3339 `full-date` string (`YYYY-MM-DD`). It is a calendar date, not an instant, and has no time, offset, or timezone.

`*_rfc3339_time` identifies an RFC 3339 `partial-time` string (`HH:MM:SS[.fraction]`). It is a time-of-day, not an instant, and MUST NOT include `Z`, `±HH:MM`, IANA timezone names, or other timezone annotations. Use `_rfc3339` or `_epoch_*` for instants.

Do not create companion timezone-name fields as an AFDATA core pattern. If a future tool needs IANA timezone semantics with a timestamp, prefer a self-contained standard value such as RFC 9557.

Avoid magic string sentinels such as `"auto"` inside strict-format fields. If a tool needs auto/default behavior, define it in that tool's own config semantics, not as an AFDATA-wide rule.

### Size

| Suffix | Example |
|:-------|:--------|
| `_bytes` | `payload_bytes: 456789` (non-negative integer) |

Byte sizes are always integer `_bytes`, in config and output alike. There is no unit-in-value size string: never write `buffer_size: "10MiB"`. Encode the unit in the key (`buffer_bytes: 10485760`), the same way durations use `timeout_s: 30`.

### Percentage

| Suffix | Example |
|:-------|:--------|
| `_percent` | `cpu_percent: 85` |

Value is in units of percent: `1` = 1%, `0.2` = 0.2%, `85` = 85%. Decimals, negatives, and values >100 are all valid; no fixed range. `%` is exactly 0.01, so `_percent` is never a 0–1 fraction — if the underlying ratio is `0.999`, multiply by 100 and write `success_percent: 99.9`. Writing `0.85` for 85% is a producer-side conversion bug, not a convention ambiguity.

### Currency

Bitcoin:

| Suffix | Example |
|:-------|:--------|
| `_msats` | `balance_msats: 97900` |
| `_sats` | `withdrawn_sats: 1234` |

Do not use a floating `_btc` suffix. Use integer `_sats` or `_msats` instead; values outside the JSON safe-integer range should be decimal strings.

Fiat — `_{iso4217}_cents` for currencies with 1/100 subdivision, `_{iso4217}` for currencies without. Generic `_{code}_cents` matches only 3-4 ASCII letters:

| Suffix | Example |
|:-------|:--------|
| `_usd_cents` | `price_usd_cents: 999` |
| `_eur_cents` | `price_eur_cents: 850` |
| `_jpy` | `price_jpy: 1500` |
| `_usdt_cents` | `deposit_usdt_cents: 1000` |

Sub-cent precision — `_{code}_micro`, integer millionths (10⁻⁶) of the major unit (`cost_usd_micro: 170000` = $0.17). The fiat analog of `_msats`: when cents are too coarse (per-token pricing, metered costs), use a smaller integer unit, never decimal cents. `_{code}_cents` for user-facing amounts, `_{code}_micro` for high-precision internal accounting.

### Sensitive

| Suffix | Handling | Example |
|:-------|:---------|:--------|
| `_secret` | redact the entire value/subtree to `***` | `api_key_secret: "sk-or-v1-abc..."` |
| `_url` | scrub secrets *inside* the URL value, keep the rest | `callback_url: "https://h/cb?code_secret=..."` |

All CLI output formats (JSON, YAML, Plain) automatically redact `_secret` fields. Matching recognizes `_secret` and `_SECRET` only — no mixed case. The entire `_secret` value/subtree becomes `***`, including objects and arrays. For legacy fields that cannot be renamed, configure `OutputOptions.redaction` with `secret_names`/`secretNames` such as `["api_key", "authorization"]`; names match exact field names at any nesting level; no trim, case folding, hyphen/underscore normalization, globs, regex, or substring matching. AFDATA does not define named redaction profiles; use the default policy `All`, `secret_names`, `TraceOnly`, or `Off` deliberately at the serialization boundary. YAML is always schema-preserving, like JSON. Callers that need schema-preserving Plain rendering can pass `OutputOptions` with the `Raw` output style.

`***` means only one thing: AFDATA redacted a sensitive value. Do not use it for serialization failure, truncation, unsupported types, or “maybe secret” guesses.

Name URL-valued fields `_url` so the userinfo password and any `_secret`/`secret_names` query parameter inside them are scrubbed automatically (the rest of the URL is preserved; the suffix is not stripped). For a URL inside a free-form message, redact it with `redact_url_secrets` before interpolating — `_url` only fires on whole-URL field values, never on prose. AFDATA does not scan arbitrary free text for secrets.

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
| `size: 456789` | `payload_bytes: 456789` | bytes? KiB? |
| `price: 999` | `price_usd_cents: 999` | what currency? what unit? |
| `latency: 142` | `latency_ms: 142` | seconds? milliseconds? |
| `api_key: "sk-..."` | `api_key_secret: "sk-..."` | won't be auto-redacted |
| `cpu: 85` | `cpu_percent: 85` | 85 what? |

---

## Part 2: Output Processing

Three output formats via one entry point `render(value, format, options)` (`format` = json/yaml/plain). JSON and YAML are both **structure-preserving**: original keys, scalar types, and numeric semantics survive after redaction. Plain is the one **lossy/human** renderer — it strips formatting suffixes and reformats values for scanning. `options` is an optional keyword arg in Python/TS; Rust/Go pass `OutputOptions` explicitly.

### Formats

- **JSON** — single-line, original keys, raw values, no sorting (machine-readable), secrets redacted
- **YAML** — multi-line, original keys, raw values (same semantics as JSON), keys sorted, secrets redacted by default
- **Plain** — single-line logfmt, formatting suffixes stripped, values formatted, secrets redacted by default

Rust gates YAML output behind the `yaml` Cargo feature (on by default, along with the rest of the default feature set). With `default-features = false` and `yaml` not re-enabled, `OutputFormat::Yaml` does not exist and `--output yaml` is rejected with a feature-requirement error instead of silently falling back to another format.

### Key stripping (Plain only)

Remove recognized formatting suffix from key. Longest match first, exact lowercase or uppercase only:

1. `_epoch_ms`, `_epoch_s`, `_epoch_ns`
2. `_usd_cents`, `_eur_cents`, `_{code}_cents`, `_{code}_micro` (`code` is 3-4 ASCII letters)
3. `_rfc3339`, `_minutes`, `_hours`, `_days`
4. `_msats`, `_sats`, `_bytes`, `_percent`, `_secret`
5. `_jpy`, `_ns`, `_us`, `_ms`, `_s`

`_bcp47`, `_utc_offset`, `_rfc3339_date`, and `_rfc3339_time` are NOT stripped (pass through). If two keys collide after stripping, both revert to original key AND raw value (no formatting). Redaction runs before collision handling, so fallback never restores a secret. JSON and YAML never strip suffixes — every key renders exactly as written.

### Value formatting (Plain only)

- `_ms` with absolute value < 1000 → `{n}ms`; absolute value ≥ 1000 → seconds (`1280` → `1.28s`, `-1500` → `-1.5s`)
- `_s`, `_ns`, `_us` → append unit (`3600s`, `450000ns`, `830μs`)
- `_minutes`, `_hours`, `_days` → append unit (`30 minutes`)
- `_epoch_ms`/`_epoch_s`/decimal-string `_epoch_ns` → RFC 3339 (negative = pre-1970)
- `_rfc3339` → pass through
- `_bytes` → human-readable (`456789` → `446.1KiB`); negative and fractional byte values fall through raw
- `_percent` → append `%`
- `_msats` → `{n}msats`, `_sats` → `{n}sats`
- `_usd_cents` → `$X.XX`, `_eur_cents` → `€X.XX`, `_jpy` → `¥X,XXX`, `_{code}_cents` → `X.XX CODE`, `_{code}_micro` → `X.XXXXXX CODE` where `code` is 3-4 ASCII letters
- `_secret` → `***` (the redaction phase already replaced the subtree)
- `_bcp47`, `_utc_offset`, `_rfc3339_date`, `_rfc3339_time` → pass through unchanged

**Type constraints**: `_bytes`/`_epoch_*` require integer. `_usd_cents`/`_eur_cents`/`_jpy`/`_{code}_cents`/`_{code}_micro` require non-negative integer. Duration/Bitcoin/`_percent` accept any number. Wrong type → raw value + original key. JSON and YAML never format values — a value renders exactly as its JSON type (numbers stay numbers, strings stay quoted strings, no unit/date/currency formatting).

### Plain logfmt details

- Nested keys use dot notation: `trace.duration=1.28s`
- Keys and values with ASCII space, tab/newline, VT, FF, NBSP, `=`, `"`, or `\` are quoted/escaped so each record stays one physical line
- Arrays comma-joined: `fields=email,age`
- Null → empty value: `RUST_LOG=`
- Sort by full dot path (JCS / UTF-16 code unit order)

### Key ordering

YAML sorts keys by UTF-16 code unit order (JCS, RFC 8785) without stripping suffixes. Plain sorts keys the same way, but after stripping. For ASCII keys this equals byte-order sorting.

### `PlainStyle` (Plain only)

`PlainStyle` (`Readable`/`Raw`) only affects Plain: `Readable` (default) strips suffixes and formats values; `Raw` keeps original keys/values (still redacted). YAML ignores `PlainStyle` entirely — it is always structure-preserving, matching JSON.

---

## Part 3: Protocol Template (Optional)

Every protocol event uses `kind` plus one same-named payload:

| `kind` | When |
|:-------|:-----|
| `"log"` | Diagnostic event; `log.event` may identify startup/request/retry/redirect |
| `"progress"` | Non-terminal status/progress |
| `"result"` | Terminal success |
| `"error"` | Terminal error with non-empty `error.code` and `error.message` |

Channel policy — the stream follows the consumption mode, not the event shape:
- **Finite one-shot command (default):** `result` → `stdout`; `error`/`progress`/`log` → `stderr`. Routing follows `kind`, not the exit code, so `stdout` carries only successful payloads (`x=$(tool …)`, `tool | next`, and `tool >/dev/null` all stay safe).
- **Event stream (interleaved events consumed in order):** every event, including `error`, stays on one stream so ordering survives; do not split it across `stdout`/`stderr`.
- `--output-to <split|stdout|stderr>` (default `split`) selects the mode: `stdout`/`stderr` collapse the whole stream onto one destination. `--output` selects format, `--output-to` selects destination.
- Raw-scalar readers (`value`/`paths`/`keys`) are intrinsically split and reject a non-default `--output-to`.
- Choose the mode by how the output is consumed, not reflexively: a plain one-shot shell command uses **finite**; a tool whose output is itself an event stream (streaming rows, interleaved progress/log) or that is agent-facing and wants one structured channel uses **stream/unified** (all on `stdout`, errors included — a stream's terminal error must stay on the stream). When building a CLI on the library, emit terminal events with the emitter's `finish`/`finish_result` helpers (success → the given code, broken pipe → 0, other write failure → a nonzero output-failure code); build errors with the `json_error` builder (the error *type*, carrying hint/retryable/fields) and pass the event to `finish` — the helpers work in either mode.

Optional stream redirection:
- use `--stdout-file <PATH>` / `--stderr-file <PATH>` to redirect a stream's bytes to a file (process-level, beneath the channel routing above)
- send an event stream to a file by collapsing it (`--output-to stdout`) and redirecting (`--stdout-file <PATH>`); there is no separate events-file flag
- native panics/tracebacks stay raw `stderr` bytes; in finite mode `stderr` also carries the formatted AFDATA `error`/`progress`/`log` envelopes, so a reader parses JSON lines and tolerates interleaved native bytes
- treat this as stream destination control, not a second protocol stream and not stream copying
- do not implement application-level rotation

Recommended enforcement:
- route every `stderr` write through the emitter's diagnostic/error path; no ad-hoc stderr that bypasses the formatter in runtime code
- Rust: keep native panic output as the only unstructured `stderr` (no stray `eprintln!` / `std::io::stderr`)
- Go/Python/TypeScript: source-policy tests that fail on ad-hoc stderr APIs in runtime code, with the emitter's own sink as the sanctioned exception

### Templates

```json
{"kind":"log","log":{"message":"startup","level":"info","event":"startup","config":{...}},"trace":{}}
{"kind":"progress","progress":{"message":"syncing","current":3,"total":10},"trace":{"duration_ms":5}}
{"kind":"result","result":{...},"trace":{"duration_ms":12,"source":"redb"}}
{"kind":"error","error":{"code":"not_found","message":"user not found","resource":"user","id":123,"retryable":false},"trace":{"duration_ms":8}}
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

All use `kind` plus the same-named payload and optional `trace`. An event stream keeps its events on one stream (never split); a finite one-shot command splits by `kind` (`result` → stdout, `error` → stderr) — see Channel policy.

Base validators enforce mandatory envelope/error/lifecycle rules. Strict validators additionally require object-valued `trace` on every event and `retryable` on every error. Log and progress payloads remain tool-defined in both profiles. Use strict validation before claiming compliance with all recommendations.

---

## Library Usage

Use the local language README for installation and the full overview/spec for API reference. Keep this skill focused on naming, output, protocol, logging, and review rules rather than duplicating import snippets that drift across languages.

Required cross-language behavior to rely on:

- The single output entry point `render(value, format, options)` redacts before formatting (with optional `options` in Python/TS).
- Use `redacted_value()` for raw HTTP/MCP/SSE serialization paths that bypass `render()`.
- Use `redact_url_secrets()` for URLs embedded in log messages or prose before interpolating them into output.
- Use `cli_parse_output()`, `cli_parse_log_filters()`, `render()`, `build_cli_error()`, and the version helper for CLI tools instead of custom parsing/error envelopes.
- `build_cli_error(message, hint?)` returns a strict-ready CLI error with `error.retryable:false` and `trace:{}`.
- Rust CLIs (feature `cli`/`cli-help`) should call `cli_handle_version_or_continue(raw_args, cmd, name, display_name, version, build)` before clap parsing so `--version`/`-V` always emits a structured `kind:"result"` event with `result.code == "version"`, `result.name`, and `result.version` (plus optional `result.display_name`/`result.build`) instead of clap's plain text — JSON by default, `--output yaml|plain` or `--json` for another format; there is no conventional bare-text form. Pass the caller's own `clap::Command` (e.g. `Cli::command()`) so any value-taking global flag it defines (e.g. `--stdout-file`) is recognized and its value is never mistaken for the subcommand boundary.
- Use the protocol reader `decode_protocol_event(text)` to parse and strict-validate a single JSON text line, receiving a typed decoded event.

## AFDATA Logging

Long-running services and processes depending on structured logging (tonic, sqlx, hyper, etc. via tracing) use a logging init function to capture the full process. One-time CLI output uses `json_log()` + `render()` or `CliEmitter`.

### Init (Rust only; pick one format per process)

| Format | Rust |
|:-------|:-----|
| **Unified** | `afdata_tracing::try_init(filter, format, redactor)` |

Go, Python, and TypeScript have no built-in logging integration; emit log events via `json_log()` + output helpers or construct them explicitly.

Rust requires `cargo add agent-first-data --features tracing`.

### Spans (Rust only; add fields to all log events in scope)

```rust
// Rust — tracing spans
let span = info_span!("request", request_id = %uuid);
let _guard = span.enter();
```

For Go, Python, and TypeScript: emit span fields explicitly in each tool-defined log payload.

### Output fields

Log payloads have no required or reserved fields. Traditional logging adapters commonly add `message` and `level` (debug/info/warn/error), plus span and event fields, but those names and meanings are adapter conventions rather than AFDATA protocol requirements. Projects that need timestamps add `timestamp_epoch_ms` explicitly.

Log redaction is **by field name** (the same `_secret`/`_url` rule as all output), applied when the line is emitted. Name the secret field — `info!(api_key_secret = %key)` — rather than logging a whole object by its `Debug`/string rendering, which hides the inner field names from redaction. For structured/nested secret-bearing data, build a value, apply redaction through output formatters or redacted_value(), then emit via the standard formatting helpers — do not pass the struct to a `?`/`%`-rendered log field.

## CLI Flags

CLI tools that use AFDATA should support output and logging flags:

```
--output json|yaml|plain    # default is tool-defined (structured/scripting → json or yaml, human scanning → plain)
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
4. Protocol transport payloads and CLI events use `kind`, the same-named payload (`event[kind]`), and top-level `trace`; tool-defined codes live inside the payload
5. Config files use the same suffixes as output
6. No unit-less ambiguous fields (`timeout: 30` — 30 what?)

## Skill Evaluation Checklist

Use this checklist when the skill is evaluating a change, not just documenting conventions:

1. Trigger on any change to structured fields, configs, logs, CLI output, protocol envelopes, MCP/HTTP/SSE payloads, database columns, wire fields, or persisted JSON.
2. Use `registry.json` for suffix semantics and `protocol-v1.schema.json` for envelope validity. Do not invent substitute suffix rules or shrink the schema to fit a local implementation.
3. Require secret-bearing values to be named with `_secret` or configured exact secret names before serialization. Do not rely on arbitrary free-text scanning or `Debug`/string-rendered objects.
4. Run `afdata lint` for JSON/JSONL samples, JSON Schema, MCP schemas, or serialized outputs when available; run `afdata validate` for finite protocol events/streams. If unavailable, report the skipped check.
5. Before changing an external project's public API, wire format, database schema, or persisted field names, report the compatibility impact and request approval before editing.
6. Byte sizes use integer `_bytes` everywhere, config included (`buffer_bytes: 10485760`, never `buffer_size: "10MiB"`)
7. Environment variables follow `UPPER_SNAKE_CASE` with the same suffixes
8. One-time CLI output uses `json_log()`/`CliEmitter` + output helpers; long-running services use `afdata_tracing::try_init()` (Rust) to capture via tracing; other languages emit log events explicitly via builders
9. Database columns use AFDATA suffixes on generic types (`duration_ms INTEGER`, not `duration INTEGER`); native types like `TIMESTAMPTZ` don't need suffixes
10. CLI flag parsing uses `cli_parse_output()`/`cli_parse_log_filters()`/`build_cli_error()`/version helpers — not custom reimplementations; uses `try_parse()` not `parse()` in Rust so a clap/usage error becomes a structured `kind:"error"` envelope (routed to stderr under the default split, exit 2), never raw clap text
