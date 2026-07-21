# Agent-First Data

A naming convention that lets AI agents understand your data without being told what it means, plus a CLI and library for reading and safely editing structured JSON, TOML, YAML, dotenv, and INI documents.

> **Ask your agent:** "Apply the Agent-First Data convention across my project's fields, config, and logs."

## The problem: data doesn't say what it means

An agent reads `{"timeout": 5000}` from a tool. Seconds or milliseconds? It guesses — and a 5-second timeout silently becomes 83 minutes. The same trap is everywhere: `{"price": 1200}` gets charged as $1,200 instead of $12.00; `{"created": 1738886400}` is treated as an ID instead of a date.

It reads `{"api_key": "sk-live-abc123"}` and writes that line straight into a log file, because nothing marked the value as a secret.

Then it moves to the next tool, which calls the same value `elapsed` instead of `duration` — so what the agent learned about one tool tells it nothing about the next.

None of this is carelessness. The data never says what it means, so the meaning has to live somewhere else — documentation, a schema, a prompt. That copy goes stale, gets lost, or was never written.

## What it does: put the meaning into the field name

Agent-First Data puts the meaning into the field name itself. Call the field `timeout_ms` and there is nothing left to guess — the name says milliseconds. Call it `api_key_secret` and any tool that follows the convention hides it automatically.

It is a convention, not a framework — a small set of name endings, plus a tiny library in four languages that reads and formats them.

- **Names carry meaning.** Endings like `_ms`, `_bytes`, `_secret`, `_usd_cents`, and `_percent` put units and intent directly into the field name.
- **One set of data, three ways to show it.** The same fields render as JSON or YAML — both keep original keys and types, for machines — or as a single human log line with units formatted for scanning. Secrets are removed in every form.
- **Secrets stay secret.** Anything ending in `_secret` is hidden automatically, in output and in logs. A `_url` field keeps its address but scrubs the userinfo password and secret-named query parameters. Legacy names like `api_key` can be protected by passing an explicit secret-name list.
- **Logging agents can read.** Structured logs that follow the same rules, with request-scoped fields.
- **The same in four languages.** One identical API across Rust, Go, Python, and TypeScript.

## A quick look

One record — a log event with a timeout, an API key, and a database URL — rendered three ways. Nothing is configured; the field names carry everything.

```json
{"kind":"log","log":{"event":"startup","args":{"timeout_s":30,"api_key_secret":"sk-123"},"db_url":"postgres://user:p@ss@db/app?token_secret=abc"},"trace":{"duration_ms":1280}}
```

**JSON and YAML** keep original keys and types (structure-preserving) and only redact secrets:

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

**Plain** is the one human renderer — it strips unit suffixes and formats values for scanning:

```text
kind=log log.args.api_key=*** log.args.timeout=30s log.db_url=postgres://user:***@db/app?token_secret=*** log.event=startup trace.duration=1.28s
```

## Supported suffixes

| Category | Suffixes |
|:--|:--|
| Duration | `_ns`, `_us`, `_ms`, `_s`, `_minutes`, `_hours`, `_days` |
| Timestamps | `_epoch_ns`, `_epoch_ms`, `_epoch_s`, `_rfc3339` |
| Size | `_bytes` (integer everywhere — config and output alike) |
| Currency | `_msats`, `_sats`, `_usd_cents`, `_eur_cents`, `_jpy`, `_{code}_cents`, `_{code}_micro` (`code` is 3–4 ASCII letters) |
| Strict strings | `_bcp47`, `_utc_offset`, `_rfc3339_date`, `_rfc3339_time` |
| Other | `_percent`, `_secret`, `_url` |

JSON and YAML keep suffixes and raw values; Plain strips duration/size/currency/timestamp suffixes after formatting the value, and never strips `_url`/`_bcp47`/`_utc_offset`/`_rfc3339_date`/`_rfc3339_time`.

## Redaction boundary

AFDATA redaction is intentionally field-name based:

- `_secret` / `_SECRET` redacts the whole value or subtree to `***`.
- Legacy names such as `api_key` are redacted only when the caller passes an explicit `secret_names` list; matching is exact field-name equality.
- `_url` fields scrub the userinfo password and query parameters whose names end in `_secret` or appear in `secret_names`; broad names such as `api_key`, `token`, or `password` are not hidden by default.
- Free-form strings are not scanned for arbitrary secrets. If a secret URL is embedded in prose, redact the URL first with `redact_url_secrets`.

There are no named redaction profiles. Use the default policy (`All`), an explicit `secret_names` list, or the documented scoped policies (`TraceOnly`, `Off`) for deliberate exceptions.

## Reading and editing config documents

Beyond emitting AFDATA, the library and `afdata` CLI read and safely edit structured documents — JSON, TOML, YAML, dotenv, and INI — by dot-path:

```bash
afdata get config.toml server.port                    # one value as an AFDATA record (secrets redacted)
host=$(afdata value config.toml server.host)          # raw scalar, for shell substitution
afdata set config.toml server.port 8080 --value-type number
```

Edits are **source-preserving and atomic** — comments, key order, and formatting survive; a failed write leaves the original untouched, and the CLI refuses to write through a symlink. A bare value is always a string (`007` never becomes `7`); `--value-type string|number|bool|null|json` writes an exact type. `_secret` fields stay redacted even on a directly targeted `get` — `value --reveal-secret` is the auditable opt-in. Every command's first positional is the FILE (`-` reads stdin for reads only); errors carry stable `error.code`s (`document_path_not_found`, `document_type_mismatch`, …). The Rust library is `agent_first_data::document` (`Document` / `DocumentFile`); TOML/YAML/dotenv/INI are feature-gated, JSON is core.

## Where to use it: CLI flags, config files, logs, and API responses

- **Building a CLI tool an agent will call** — your output is understood correctly the first time, with no extra schema to ship. Results land on `stdout` and errors on `stderr` by default, so a shell capture or pipe never mistakes a failure for data; a single `--output-to stdout` collapses everything onto one stream when a consumer would rather branch on `kind`.
- **Writing a config file** — keys like `timeout_s` or `db_password_secret` make settings self-explanatory to whoever edits them, and secrets stay hidden when the config is printed back.
- **Adding logs to a service** — the same lines stay readable for a person and parseable for an agent.
- **Designing an API response or event payload** — units and sensitivity travel *with* the data, across every boundary it crosses.
- **Auditing for leaked secrets** — one naming rule (`_secret`) makes redaction automatic instead of case-by-case.

## One contract, four languages

The same surface ships identically in Rust, Go, Python, and TypeScript (each in its own casing):

- **Protocol builders** `json_result` / `json_error` / `json_progress` / `json_log` → `.build()` → an event; **reader** `decode_protocol_event(text)` → a typed decoded event.
- **Output** `render(value, format, options)` — the single value × format × options → string entry point.
- **Redaction** `redacted_value` / `redact_url_secrets` for paths that bypass `render`.
- **CLI helpers** `cli_parse_output`, `cli_parse_log_filters`, `build_cli_error`, `build_cli_version`, `cli_handle_version_or_continue`, and the `CliEmitter`.

Three tools are **Rust-only** and deliberately outside the cross-language contract: skill admin (`SKILL.md` validation plus install/uninstall/status), stream redirection (`--stdout-file` / `--stderr-file`), and tracing/logging init (`afdata_tracing::try_init`). The exact shared surface is enumerated in [`spec/api-surface.json`](spec/api-surface.json).

## Adopt it: hand the convention to your coding agent

Agent-First Data is a convention, not a dependency you wire in by hand — and adopting a convention is exactly the kind of work you now hand to an agent. There's even an [Agent Skill](skills/agent-first-data/SKILL.md) for exactly that — the convention in a form an agent reads and applies directly. Paste this to your coding agent:

> Learn the Agent-First Data convention: read https://agentfirstkit.com/agent-first-data/docs/specification and https://agentfirstkit.com/agent-first-data/docs/agent-skill. Then look at the codebase we're working in and tell me whether adopting the convention would help it — and if so, how: which fields and config keys to rename, and where the output and logging helpers fit.

## Install the Libraries

```bash
cargo add agent-first-data --no-default-features   # Rust library
pip install agent-first-data     # Python
npm install agent-first-data     # TypeScript
go get github.com/agentfirstkit/agent-first-data/go   # Go
```

## Install the CLI

The `afdata` CLI provides the same formatting, redaction, and protocol-event
helpers from any shell, with no toolchain required:

```bash
# prebuilt binary
brew install agentfirstkit/tap/afdata   # macOS / Linux
scoop bucket add agentfirstkit https://github.com/agentfirstkit/scoop-bucket && scoop install afdata   # Windows

# or from crates.io
cargo install agent-first-data
```

Prebuilt archives are also available from
[GitHub Releases](https://github.com/agentfirstkit/agent-first-data/releases).

## Validate an Agent Skill

`afdata skill validate` checks a `SKILL.md` against the official metadata
constraints with a strict YAML parser. Passing a directory also verifies that
its name matches the front-matter `name`. Use `afdata skill install`, `status`,
and `uninstall` to manage the bundled skill.

```bash
afdata skill validate skills/agent-first-data
```

## Docs

- [Specification](spec/agent-first-data.md) — the full convention: every suffix, output formats, protocol, and logging
- [Agent Skill](skills/agent-first-data/SKILL.md) — for AI-assisted development
- Per-language API reference: [Rust](rust) · [Go](go) · [Python](python) · [TypeScript](typescript)

## License

MIT
