# Overview

**The field name is the schema.** Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact, and `callback_url` and know URL credentials must be scrubbed.

Agent-First Data (AFDATA) is a convention plus four small libraries:

1. **Naming** - encode units and sensitivity in field names (`_ms`, `_bytes`, `_secret`, `_url`, ...)
2. **Output** - render the same data as structure-preserving JSON or YAML, or as plain logfmt with deterministic unit/date/currency formatting
3. **Protocol** - optional JSONL objects with `kind`, a matching payload field, and optional `trace`
4. **Logging** - structured logs that use the same redaction and suffix formatting rules
5. **Channel discipline** - machine-readable events go to `stdout`; `stderr` is not a protocol stream
6. **Stream redirection** - optional CLI helper to send stdout and stderr to separate files without changing their formats
7. **Documents** - read and safely edit structured JSON, TOML, YAML, dotenv, and INI files by dot-path, source-preserving (CLI commands plus the Rust `agent_first_data::document` library)

See the full [specification](../spec/agent-first-data.md) and the [agent skill](../skills/agent-first-data/SKILL.md).

## Installation

```bash
cargo add agent-first-data        # Rust
pip install agent-first-data      # Python
npm install agent-first-data      # TypeScript
go get github.com/agentfirstkit/agent-first-data/go  # Go
```

## Quick Example

Input data:

```json
{"kind":"log","log":{"event":"startup","args":{"timeout_s":30,"api_key_secret":"sk-123"},"db_url":"postgres://user:p@ss@db/app?token_secret=abc"},"trace":{"duration_ms":1280}}
```

JSON and YAML both keep original keys and raw values (structure-preserving), and only redact secrets:

```json
{"kind":"log","log":{"event":"startup","args":{"timeout_s":30,"api_key_secret":"***"},"db_url":"postgres://user:***@db/app?token_secret=***"},"trace":{"duration_ms":1280}}
```

```yaml
---
kind: "log"
log:
  args:
    api_key_secret: "***"
    timeout_s: 30
  db_url: "postgres://user:***@db/app?token_secret=***"
  event: "startup"
trace:
  duration_ms: 1280
```

Plain is the one lossy/human renderer: it strips formatting suffixes and formats values for scanning:

```text
kind=log log.args.api_key=*** log.args.timeout=30s log.db_url=postgres://user:***@db/app?token_secret=*** log.event=startup trace.duration=1.28s
```

## Current API Surface

Language names follow each ecosystem's casing. The shared contract is:

| Group | APIs |
|:--|:--|
| Protocol builders | `json_result`, `json_error`, `json_progress`, `json_log` (fluent builders; `.build()` → Event) |
| Protocol reader | `decode_protocol_event(text)` → typed DecodedResult \| DecodedError \| DecodedProgress \| DecodedLog; raises EventDecodeError on invalid input |
| Output | `render(value, format, options)` — the single value × format × options → String entry point (options optional in Python/TS; Rust/Go pass `OutputOptions` explicitly) |
| Redaction | `redacted_value`, `redact_url_secrets` (defaults; options are keyword args in Python/TS, a configured `Redactor` value in Rust/Go: `.value()`/`.url()`) |
| CLI helpers | `normalize_utc_offset`, `is_valid_rfc3339_date`, `is_valid_rfc3339_time`, `is_valid_rfc3339`, `is_valid_bcp47`, `cli_parse_output`, `cli_parse_log_filters`, `render`, `build_cli_error`, `build_cli_version`, `cli_handle_version_or_continue`, `CliEmitter` |
| Types | `OutputFormat`, `RedactionPolicy`, `PlainStyle`, `OutputOptions`, `LogFilters` |
| Skill admin & stream redirect | Moved to submodules: Python `agent_first_data.skill` / `agent_first_data.stream_redirect`, TS `agent-first-data/skill` / `agent-first-data/stream-redirect`, Go `go/skill` / `go/streamredirect`, Rust feature-gated (on by default; opt out with `default-features = false`) |
| Logging init | Rust only: `afdata_tracing::try_init(filter, format, redactor)` |

Built-in redaction applies to `_secret` (whole value → `***`) and `_url` (scrub userinfo password and secret-named query parameters). Field-based redaction is the only mechanism: custom sensitive names are explicit exact-name lists configured at the redactor or output boundary. `RedactionPolicy` is `{ All, TraceOnly, Off }`; the default is `All` (full redaction), with `TraceOnly` and `Off` as explicit overrides.

AFDATA does not provide named redaction profiles and does not scan arbitrary prose for secrets. Custom sensitive names are an explicit exact-name list (`secret_names` / `SecretNames` / `secretNames`). Broad URL query names such as `token`, `api_key`, or `password` are not hidden unless they end in `_secret` or are listed. When a value bypasses output formatters (HTTP/MCP/SSE serialization), apply `redacted_value()` or `redact_url_secrets()` at the serialization boundary before writing to transport.

`build_cli_error(message, hint?)` returns a strict-ready protocol v1 error event: `{kind:"error", error:{code:"cli_error", message, hint?, retryable:false}, trace:{}}`.

Version helpers should run before the app parser so bare `--version` stays conventional and `--version --output json|yaml|plain` emits a structured `kind:"result"` event with `result.version` instead of being intercepted by parser built-ins.

Canonical CLIs default to one terminal protocol event. They do not add
`--stream` or `--result-only`; extra `log`/`progress` events appear only when
requested through explicit diagnostics such as `--log ...` or `--verbose`.
TTY detection and stdout/stderr redirection do not change that policy.

The Rust `cli-help`, `skill`, `skill-admin`, `stream-redirect`, and `tracing` features are all on by default, so the published `afdata` binary and `cargo install agent-first-data` are full-featured, and `cargo add agent-first-data` pulls every helper. `skill` provides strict `SKILL.md` validation, while `skill-admin` includes it and adds install/uninstall/status operations. They are intentionally separate from the cross-language AFDATA formatting contract; a consumer that wants only that core surface disables them with `default-features = false`, and language README files point back here instead of duplicating the full reference.

Optional stream redirection uses canonical CLI names:

```text
--stdout-file <PATH>
--stderr-file <PATH>
```

When enabled, stdout bytes are appended to the stdout file and stderr bytes are appended to the stderr file. This is a stream destination override, not a second protocol stream: stdout keeps the selected AFDATA format, and stderr keeps native diagnostics such as Rust panics, Python tracebacks, or runtime errors. Rotation is left to external tooling.

## Documents

Beyond emitting AFDATA, `afdata` reads and safely edits structured documents — JSON, TOML, YAML, dotenv, and INI — by dot-path. A spore embeds the library for generic config access without a per-field dispatch table; a shell or another-language CLI gets one tool for one-off reads and edits.

Read with `get <FILE> [KEY]` (the whole document with no KEY, or one value as an AFDATA record with KEY — secrets still redacted), `value <FILE> <KEY>` (the raw scalar, for shell substitution), or `paths`/`keys <FILE> [KEY]` (a container's children, one per line — `paths` emits full dot-paths that feed back into afdata, `keys` emits raw names for external tools). Edit in place with `set`/`unset` (any format) and, for JSON or YAML, `add`/`remove` on a keyed list:

```bash
afdata get config.toml server.port
host=$(afdata value config.toml server.host)      # FILE then KEY; scalars only
afdata set config.toml server.port 8080 --value-type number
```

- **One input model.** Every command's first positional is FILE, or `-` for stdin (read commands only — mutations reject `-`). There is no implicit stdin fallback. Format is inferred from the file extension and overridable with `--input-format`.
- **Source-preserving and atomic.** Edits keep comments, key order, and unrelated formatting; a failed write leaves the original untouched, and the CLI refuses to write through a symlink or hardlink. A mutation's result carries the `path` actually written.
- **One dot-path grammar.** A literal dot in a key is `\.` and a literal backslash is `\\`; an unrecognized escape is an error, never a guess. `add`/`remove` operate on a keyed list and require an explicit `--slug-field`.
- **Bare values are strings; exact types are explicit.** `set`'s bare VALUE (and `add`'s `FIELD=VALUE`) is always a plain string — no shape-guessing, so `007` never becomes `7`. `--value-type string|number|bool|null|json` writes an exact type; `json` is the only entry point for an array or object. Overwriting an *existing* scalar of a different kind with a bare VALUE is an argument error naming both escape hatches, not a silent type rewrite.
- **Number literals round-trip exactly.** `get`/`set` preserve a number's original digits — an integer beyond `u64` or a high-precision float is never routed through `f64` and reformatted; `value` is otherwise type-lossy (every scalar becomes plain text) but stays digit-faithful for numbers.
- **Secrets stay closed.** A `_secret` leaf (or an exact `--secret-name`) is redacted even on a directly targeted `get`; `value --reveal-secret` is the explicit, auditable opt-in. `value` on a container errors (`document_not_scalar`) — use `get` for a subtree. `value`'s failure envelope goes to stderr, never stdout, so shell substitution never captures a JSON error as data; `--default VAL` covers the common "missing or null -> fall back" case in one call.
- **Decidable errors.** Runtime document errors use stable `error.code`s (`document_path_not_found`, `document_type_mismatch`, `document_slug_not_found`, `document_slug_exists`, `document_parse_failed`, `document_io_failed`, and a few narrower ones) and exit 1; a malformed invocation is always `document_usage_error` at exit 2.

The library lives at `agent_first_data::document` (Rust): `Document` / `DocumentFile` plus the format backends, gated by the `toml` / `yaml` / `dotenv` / `ini` features (JSON is core). Consumers can use `Format::name()`, `Value::kind_name()`, and `DocumentError::code()` instead of exhaustively matching public enums for display labels and stable error classification. For reading a config that may hold secrets, `DocumentFile::open_capped(path, format, max_bytes)` rejects an oversized or non-regular file before reading it and `value_at(dot_path)` fetches a single address in one call; on a parse failure `DocumentError::redacted_message()` (and the raw `location()`) yield a content-free message that never echoes the source.

## Logging Contract

Long-running services or processes that depend on structured logging (tonic, sqlx, hyper, etc. via tracing) should use `afdata_tracing::try_init()` (Rust only) to capture the full process. This wires the logging ecosystem to emit through AFDATA output formatters.

One-time CLI output (single event) uses `json_log()` or the `CliEmitter` helper; `render()` handles the serialization.

Log payloads are tool-defined and have no required or reserved fields. Traditional logging adapters commonly add `message` and `level`, but AFDATA does not require them. `kind:"log"` distinguishes log events from terminal protocol events. Projects that need timestamps add them explicitly as `timestamp_epoch_ms`.

Example plain line:

```text
level=info message="Processing" request_id=abc-123 timestamp_epoch_ms=1739026400000
```

Name whole-value secret log fields explicitly (`api_key_secret`, `database_url_secret`) so redaction can see that the entire value is secret. Use `_url` only when the URL may remain visible after its userinfo and token-bearing query parameters are scrubbed. Any token-bearing query parameter must either be renamed to an `_secret` parameter such as `token_secret`, or listed in `secret_names` / `SecretNames` / `secretNames` when legacy names cannot change. Do not log a whole secret-bearing object as a pre-rendered debug string or free-form prose and expect AFDATA to find the inner secret. PII and domain-specific privacy policies (header names, API scopes) are owned by each spore; the library does not provide generic scanning or secret profiles.

## Supported Suffixes

| Category | Suffixes |
|:--|:--|
| Duration | `_ns`, `_us`, `_ms`, `_s`, `_minutes`, `_hours`, `_days` |
| Timestamps | `_epoch_ns`, `_epoch_ms`, `_epoch_s`, `_rfc3339` |
| Size | `_bytes` (integer, everywhere — config and output alike) |
| Currency | `_msats`, `_sats`, `_usd_cents`, `_eur_cents`, `_jpy`, `_{code}_cents`, `_{code}_micro` where `code` is 3-4 ASCII letters |
| Strict strings | `_bcp47`, `_utc_offset`, `_rfc3339_date`, `_rfc3339_time` |
| Other | `_percent`, `_secret`, `_url` |

YAML output sorts keys by UTF-16 code unit order without stripping suffixes. Plain output sorts keys the same way after key stripping, and escapes both keys and values so every record stays one physical line.

## Language Documentation

- [Rust](../rust)
- [Go](../go)
- [Python](../python)
- [TypeScript](../typescript)
