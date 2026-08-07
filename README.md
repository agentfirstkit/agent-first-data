# Agent-First Data

A naming convention that lets AI agents understand your data without being told what it means, plus a CLI and library for reading Markdown structure and safely editing structured JSON, TOML, YAML, dotenv, and INI documents.

> **Ask your agent:** "Apply the Agent-First Data convention across my project's fields, config, and logs."

## The problem: data doesn't say what it means

An agent reads `{"timeout": 5000}` from a tool. Seconds or milliseconds? It guesses — and a 5-second timeout silently becomes 83 minutes. The same trap is everywhere: `{"price": 1200}` gets charged as $1,200 instead of $12.00; `{"created": 1738886400}` is treated as an ID instead of a date.

It reads `{"api_key": "sk-live-abc123"}` and writes that line straight into a log file, because nothing marked the value as a secret.

Then it moves to the next tool, which calls the same value `elapsed` instead of `duration` — so what the agent learned about one tool tells it nothing about the next.

None of this is carelessness. The data never says what it means, so the meaning has to live somewhere else — documentation, a schema, a prompt. That copy goes stale, gets lost, or was never written.

## What it does: put the meaning into the field name

Agent-First Data puts the meaning into the field name itself. Call the field `timeout_ms` and there is nothing left to guess — the name says milliseconds. Call it `api_key_secret` and any AFDATA output boundary that follows the convention hides it automatically.

It is a convention, not a framework — a small set of name endings, plus a tiny library in four languages that reads and formats them.

- **Names carry meaning.** Endings like `_ms`, `_bytes`, `_secret`, `_usd_cents`, and `_percent` put units and intent directly into the field name.
- **One set of data, three ways to show it.** The same fields render as JSON or YAML — both keep original keys and types, for machines — or as a single human log line with units formatted for scanning. Secrets are removed in every form.
- **Structured secrets are redacted.** Anything ending in `_secret` is hidden when a structured value passes through an AFDATA redactor or renderer. A `_url` field keeps its address but scrubs the userinfo password and secret-named query parameters. Legacy secret and URL fields can be protected with exact `secret_names` and `url_names` lists.
- **Logging agents can read.** Structured logs that follow the same rules, with request-scoped fields.
- **One contract in four languages.** Rust, Go, Python, and TypeScript share
  wire behavior and fixtures while keeping idiomatic native API shapes.

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
kind=log log.args.api_key=*** log.args.timeout=30s log.db_url="postgres://user:***@db/app?token_secret=***" log.event=startup trace.duration=1.28s
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

Fiat suffixes use signed integer units: negative values represent refunds,
credits, reversals, or deltas and receive the same Plain currency formatting.

JSON and YAML keep suffixes and raw values; Plain strips duration/size/currency/timestamp suffixes after formatting the value, and never strips `_url`/`_bcp47`/`_utc_offset`/`_rfc3339_date`/`_rfc3339_time`.

## Redaction boundary

AFDATA redaction is intentionally field-name based:

- `_secret` / `_SECRET` redacts the whole value or subtree to `***`.
- Legacy names such as `api_key` are redacted only when the caller passes an explicit `secret_names` list; matching is exact field-name equality.
- `_url` fields scrub the userinfo password and query parameters whose names end in `_secret` or appear in `secret_names`; broad names such as `api_key`, `token`, or `password` are not hidden by default.
- Legacy URL fields such as `url` or `relays` receive the same treatment only when listed exactly in `url_names`. Arrays and nested collections under that field are walked recursively; unrelated fields and ordinary non-URL strings are unchanged.
- Free-form strings are not scanned for arbitrary secrets. Use the explicit `redact_urls_in_text` helper when prose may contain complete scheme URLs; it redacts only those URL spans and leaves the surrounding prose alone.

The suffix protects structured fields only after they pass through an AFDATA
redactor or renderer. It cannot remove a live secret from process argv, shell
history, `/proc`, a parent process, or third-party logs. Avoid putting secrets
in argv; if a tool records its invocation, pass argv through `redact_argv`
before logging it.

There are no named redaction profiles. Use the default policy (`All`), an explicit `secret_names` list, or the documented scoped policies (`TraceOnly`, `Off`) for deliberate exceptions.

## Reading and editing config documents

Beyond emitting AFDATA, the library and `afdata` CLI read and safely edit structured documents — JSON, TOML, YAML, dotenv, and INI — by dot-path:

```bash
afdata get config.toml server.port                    # one value as an AFDATA record (secrets redacted)
host=$(afdata value config.toml server.host)          # raw scalar, for shell substitution
afdata set config.toml server.port 8080 --value-type number
```

Use `get` when the next step must preserve JSON types and structure; `value`
deliberately turns one scalar into raw shell bytes. `set` creates missing object
parents along the requested dot-path, but refuses to traverse an existing
scalar or incompatible container.

Edits are **source-preserving and atomic**: comments, key order, and formatting
survive; a write is read back before it lands, so an edit that would leave a
file its own parser rejects fails instead of succeeding; no partial file is
observable, and failures before atomic installation leave the original
untouched. A rare parent-directory fsync failure after installation is
commit-uncertain, so reopen the path before retrying. Values are **never
guessed** — a bare value is always a string, and an exact type is asked for,
not inferred. A name marks its whole subtree, so a `_secret` node stays
redacted however you address it, and revealing it is an auditable opt-in.
Errors carry stable codes and never quote the document, because an error event
is the thing an agent logs.

TOML edits support scalars, arrays, array elements, inline tables, and ordinary
tables while preserving surrounding decor, comments, order, trailing commas,
and untouched datetime syntax. Arrays of tables are refused because this editor
does not define an element-identity policy; afdata never guesses which repeated
table an object should replace.

INI accepts both section entries (`section.key`) and flat `key=value` entries
before the first section header (addressed as a bare `key`). A root key and a
section cannot share a name. When a config filename such as `phoenix.conf`
does not identify its parser, a Rust value source can say so explicitly:
`file+ini:PATH#DOT_PATH`.

A Markdown file has three valid readings, none of them guessed at: name
`toml-frontmatter` or `yaml-frontmatter` to edit its metadata block with the body
frozen, or `markdown` to read the body as a tree of heading sections. The last is
read-only, and reports structure rather than what a project means by it — whether
the H1 is the title is a layout convention, and stays with the caller that holds
it.

```bash
afdata value README.md h1.0.paragraph.0.text --input-format markdown  # the synopsis
afdata value README.md h1.0.h2.suffixes.text --input-format markdown  # a section by name
afdata value README.md h1.0.paragraph.0.source_end_line \
  --input-format markdown                                             # source cut point
```

An array element can be addressed by content rather than position, which is what
makes an address survive an edit above it: a Markdown section matches a word of
its heading, and any other document declares the field to match with
`--slug-field` (`identities.me.email --slug-field identity`). Matching several
elements is an error reporting their indices, never their document text and
never the first one.

Every Markdown block reports 1-based inclusive `source_start_line` and
`source_end_line` — the lines `sed`, `awk`, `head`, and `git diff` count, so a
range can be handed straight to them without Markdown reserialization or UTF-8
byte indexing. A section's range covers the whole section; it additionally
reports `heading_end_line`, where its own heading ends, which is what tells a
setext heading's two lines from the body below. A recognised frontmatter block
reports `type: "frontmatter"` and `format: "toml"|"yaml"` with empty `text` —
read its fields through the matching frontmatter mode, where normal secret
redaction applies.

Flags and exit codes are in [`docs/cli.md`](docs/cli.md) and `afdata <command>
--help`; the error codes and recovery rules are in the
[skill reference](skills/agent-first-data/references/documents.md). The Rust
library is `agent_first_data::document` (`Document` / `DocumentFile`).
`DocumentFile::open_capped` limits bytes read from one verified file handle;
`create_atomic` safely performs a first no-clobber commit; `decode` and
`edit_and_validate` put typed serde validation on the same transaction boundary.
On Unix, the default-on `libc` feature adds nonblocking special-file rejection
and atomic `SymlinkPolicy::NoFollow`; `stream-redirect` enables it automatically.

## Token-efficient CLI discovery

Help is a normal result, not an exception to the output contract, and it answers
in one round trip. `afdata --help` returns a protocol-v1 result whose help-v2
payload carries every legal shape of the command, each complete. `afdata set
--help`, for example, shows the distinct `set-value`, `set-null`, and
`set-secret` shapes instead of an argument catalog that leaves an agent to solve
conflicts. There is no second level to ask for: what it could omit — the
optional arguments — would be registered but undiscoverable to a caller that
stopped at the first. `--output plain` renders the same catalog for humans, and
`--docs` renders the whole registry as Markdown. One `cli-spec-v1` registry
generates argv parsing, typed invocations, combination validation, output plans,
help, and that reference. Contracts:
[`cli-spec-v1`](spec/cli-spec-v1.schema.json) and
[`cli-help-v2`](spec/cli-help-v2.schema.json).

## Writing AFDATA-style Bash scripts

The CLI embeds a sourceable Bash 3.2+ authoring kit:

```bash
#!/usr/bin/env bash
set -euo pipefail
_AFDATA_BASH_SOURCE="$("${AFDATA_BIN:-afdata}" shell bash)"
source /dev/stdin <<<"$_AFDATA_BASH_SOURCE"
unset _AFDATA_BASH_SOURCE

afdata_args_begin "build.sh [OPTIONS] PACKAGE [-- CARGO_ARG ...]"
afdata_args_flag release --release "Build release artifacts"
afdata_args_positional package PACKAGE "Package to build"
afdata_args_rest CARGO_ARG "Arguments forwarded to cargo"
afdata_args_parse "$@"

afdata_log info "Building ${package}"
if [ "$release" = true ]; then
  afdata_run cargo build --release ${AFDATA_ARGS_REST[@]+"${AFDATA_ARGS_REST[@]}"}
else
  afdata_run cargo build ${AFDATA_ARGS_REST[@]+"${AFDATA_ARGS_REST[@]}"}
fi
afdata_result "Build complete"
```

The helpers handle long-form argument parsing, `--help`, AFDATA output flags,
raw config reads, and structured `log`/`result`/`error` events. `afdata_run`
keeps child stdin/stdout/stderr and TTY interaction untouched; only the Bash
script's own lifecycle is structured, with a terminal error on child failure.
Use `afdata_call` when an AFDATA Bash parent invokes an AFDATA Bash child and
must remain the sole owner of the final result. See the complete
[Bash authoring kit guide](docs/bash.md).

## Where to use it: CLI flags, config files, logs, and API responses

- **Building a CLI tool an agent will call** — your output is understood correctly the first time, with no extra schema to ship. Results land on `stdout` and errors on `stderr` by default, so a shell capture or pipe never mistakes a failure for data; a single `--output-to stdout` collapses everything onto one stream when a consumer would rather branch on `kind`.
- **Writing a config file** — keys like `timeout_s` or `db_password_secret` make settings self-explanatory to whoever edits them, and secrets stay hidden when the config is printed back.
- **Adding logs to a service** — the same lines stay readable for a person and parseable for an agent.
- **Designing an API response or event payload** — units and sensitivity travel *with* the data, across every boundary it crosses.
- **Auditing for leaked secrets** — one naming rule (`_secret`) makes redaction automatic instead of case-by-case.

## One shared contract, four languages

The shared core surface ships in Rust, Go, Python, and TypeScript (each in its own casing):

- **Protocol builders** `json_result` / `json_error` / `json_progress` / `json_log` → `.build()` → an event; **reader** `decode_protocol_event(text)` → a typed decoded event.
- **Output** `render(value, format, options)` — the single value × format × options → string entry point.
- **Redaction** `redacted_value` / `redact_url_secrets` / `redact_urls_in_text` for paths that bypass `render`; output options accept exact legacy `secret_names` and `url_names`.
- **CLI primitives** `cli_parse_output`, `cli_parse_log_filters`, `build_cli_error`, `build_cli_version`, and the `CliEmitter`.

The Rust crate is the reference implementation of `CliSpec`, closed-world
combination resolution, version, and help-v2; the whole CLI surface sits behind
the default-on `cli` feature. Argv is only ever parsed through a registry —
there is no raw pre-parser. A resolved `OutputPlan` carries typed
`OutputFormat` / `OutputTo` values; applications retain control of command
lifetime and output policy while using `CliEmitter`, `write_raw`, and the
optional stream-redirection module.

Rust arguments may also declare `SourceSet::config()` or
`SourceSet::stream()`, so help, validation, and the host agree on
`env:`/`file:`/`stdin`/`fd:`/`prompt` value sources. `ValueSource::read_secret`
returns a `SecretString` that redacts under `Debug` and `Display`; the host must
call `expose_secret` only where the credential is consumed.

Rust also provides `ErrorSpec` / `ErrorCatalog` for stable public domain errors,
in-process `lint_value` and assertion helpers, and a composable
`afdata_tracing::AfdataLayer` with an injectable writer and
`StructuredLogHandle` for nested JSON. The shared surface is enumerated in
[`spec/api-surface.json`](spec/api-surface.json); other SDK compilers consume
the same serialized CLI spec and fixtures as they migrate.

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
- [CLI reference](docs/cli.md) — discovery and registered-combination examples
- [CLI spec v1](spec/cli-spec-v1.schema.json) and [help v2](spec/cli-help-v2.schema.json) — the closed invocation registry and token-efficient help contracts
- [Protocol v1](docs/protocol-v1.md) and [transport mappings](docs/transport-mappings.md) — the event envelope across CLI, HTTP, MCP, and SSE
- [Bash authoring kit](docs/bash.md) — arguments, config reads, events, and transparent child processes
- [Agent Skill](skills/agent-first-data/SKILL.md) — for AI-assisted development, with the [document reference](skills/agent-first-data/references/documents.md) covering addressing, error codes, and recovery
- Per-language API reference: [Rust](rust) · [Go](go) · [Python](python) · [TypeScript](typescript)

## License

MIT
