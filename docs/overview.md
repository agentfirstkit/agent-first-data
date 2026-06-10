# Overview

**The field name is the schema.** Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact, and `callback_url` and know URL credentials must be scrubbed.

Agent-First Data (AFDATA) is a convention plus four small libraries:

1. **Naming** - encode units and sensitivity in field names (`_ms`, `_bytes`, `_secret`, `_url`, ...)
2. **Output** - render the same data as JSON, YAML, or plain logfmt with deterministic formatting
3. **Protocol** - optional JSONL objects with `code`, `result`/`error`, and `trace`
4. **Logging** - structured logs that use the same redaction and suffix formatting rules
5. **Channel discipline** - machine-readable events go to `stdout`; `stderr` is not a protocol stream

See the full [specification](../spec/agent-first-data.md) and the [agent skill](../skills/agent-first-data.md).

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
{"code":"log","event":"startup","args":{"timeout_s":30,"api_key_secret":"sk-123"},"db_url":"postgres://user:p@ss@db/app?token_secret=abc","trace":{"duration_ms":1280}}
```

JSON keeps original keys and raw values, but redacts secrets:

```json
{"code":"log","event":"startup","args":{"timeout_s":30,"api_key_secret":"***"},"db_url":"postgres://user:***@db/app?token_secret=***","trace":{"duration_ms":1280}}
```

YAML and plain strip formatting suffixes and format values:

```yaml
---
args:
  api_key: "***"
  timeout: "30s"
code: "log"
db_url: "postgres://user:***@db/app?token_secret=***"
event: "startup"
trace:
  duration: "1.28s"
```

```text
args.api_key=*** args.timeout=30s code=log db_url=postgres://user:***@db/app?token_secret=*** event=startup trace.duration=1.28s
```

## Current API Surface

Language names follow each ecosystem's casing. The shared contract is:

| Group | APIs |
|:--|:--|
| Protocol builders | `build_json_ok`, `build_json_error`, `build_json` |
| Output | `output_json`, `output_json_with`, `output_json_with_options`, `output_yaml`, `output_yaml_with_options`, `output_plain`, `output_plain_with_options` |
| Redaction | `redacted_value`, `redacted_value_with`, `redacted_value_with_options`, `redact_secrets_in_place`, `redact_secrets_in_place_with_options` |
| URL redaction | `redact_url_secrets`, `redact_url_secrets_with_options` |
| CLI helpers | `parse_size`, `normalize_utc_offset`, `cli_parse_output`, `cli_parse_log_filters`, `cli_output`, `cli_output_with_options`, `build_cli_error` |
| Types | `OutputFormat`, `RedactionPolicy`, `RedactionOptions`, `OutputStyle`, `OutputOptions` |

`RedactionPolicy` has two explicit overrides: `RedactionTraceOnly` and `RedactionNone`. The default policy is full redaction: every `_secret` or configured secret-name field is replaced by `***`, including object and array subtrees. `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed, and internal whitespace causes the whole URL field to become `***`.

`build_cli_error(message, hint?)` returns only the protocol error shape: `{code:"error", error: message, hint?}`. It does not invent retry metadata or fake traces.

The Rust `cli-help` and `skill-admin` features are implementation utilities for spore binaries. They are intentionally separate from the cross-language AFDATA formatting contract; language README files point back here instead of duplicating the full reference.

## Logging Contract

Logging integrations emit structured records through the same output formatters.

Required log fields:

- `timestamp_epoch_ms`
- `message`
- `code: "log"`
- `level: "debug" | "info" | "warn" | "error"`

`code` is always `"log"`; the logging level lives in `level`. This prevents an error-level log line from being mistaken for a terminal protocol result with `code:"error"`.

Example plain line:

```text
code=log level=info message=Processing request_id=abc-123 timestamp=2025-02-08T07:33:20.000Z
```

Name secret log fields explicitly (`api_key_secret`, `db_url`) so redaction can see the field name. URL fields should end in `_url`; any token-bearing query parameter must either be renamed to an `_secret` parameter such as `token_secret`, or listed in `secret_names` / `SecretNames` / `secretNames` when legacy names cannot change. Do not log a whole secret-bearing object as a pre-rendered debug string.

## Supported Suffixes

| Category | Suffixes |
|:--|:--|
| Duration | `_ns`, `_us`, `_ms`, `_s`, `_minutes`, `_hours`, `_days` |
| Timestamps | `_epoch_ns`, `_epoch_ms`, `_epoch_s`, `_rfc3339` |
| Size | `_bytes` for numeric output, `_size` for config input strings |
| Currency | `_msats`, `_sats`, `_btc`, `_usd_cents`, `_eur_cents`, `_jpy`, `_{code}_cents` where `code` is 3-4 ASCII letters |
| Strict strings | `_bcp47`, `_utc_offset` |
| Other | `_percent`, `_secret`, `_url` |

YAML and plain output sort keys by UTF-16 code unit order after key stripping. Plain output escapes both keys and values so every record stays one physical line.

## Language Documentation

- [Rust](../rust)
- [Go](../go)
- [Python](../python)
- [TypeScript](../typescript)
