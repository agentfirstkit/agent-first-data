---
name: agent-first-data
description: Apply and review the formal Agent-First Data specification using the Rust/Go/Python/TypeScript libraries or afdata CLI for naming, redaction, formatting, protocol envelopes, logging, linting, validation, and reading or safely editing structured JSON/TOML/YAML/dotenv/INI documents (and the +++/--- frontmatter block of a Markdown page) by dot-path. Use proactively when writing or reviewing structured data, configs, logs, transport payloads, database/wire fields, CLI output, compatibility-sensitive public and persistent field names, or when reading or editing a config/document/Markdown-frontmatter file from the shell.
---

<!-- Canonical source: skills/agent-first-data/SKILL.md.
     Mirrored byte-for-byte into go/python/typescript assets/ by scripts/sync_offline_assets.py.
     Edit only here; run that script (checked by scripts/test.sh) to propagate. -->

# Agent-First Data

Use AFDATA when naming fields or reviewing structured output. The contract is naming-based: suffixes communicate units, formatting, and redaction semantics without relying on free-text inference.

## Specification workflow

Treat the formal specification as normative, not the skill summary or an
example payload. In a repository checkout, read these spore-root-relative
sources as needed:

- `spec/agent-first-data.md` — complete cross-language naming, formatting,
  redaction, protocol, logging, and CLI contract.
- `spec/registry.json` — exact suffix metadata.
- `spec/protocol-v1.schema.json` — protocol event schema.
- `spec/fixtures/` — cross-language conformance cases.

When only the installed skill is available, use the bundled equivalents:

- `references/rules.md` — detailed suffix rules, output formatting, protocol templates, logging, CLI flags, and review checklist.
- `references/registry.json` — machine-readable suffix registry; use this as the exact source for suffix metadata.
- `references/protocol-v1.schema.json` — protocol event schema for `kind:"result"`, `kind:"error"`, `kind:"progress"`, and `kind:"log"` envelopes.

If an example or README conflicts with the formal spec, follow the spec and
report the discrepancy. Do not invent suffix meanings or reduce the protocol
schema.

## Library workflow

Use the library at serialization boundaries so redaction and formatting are
not reimplemented ad hoc. Rust applications can build a typed protocol event
and render it in the required output format:

```bash
cargo add agent-first-data --no-default-features
pip install agent-first-data
npm install agent-first-data
go get github.com/agentfirstkit/agent-first-data/go
```

```rust
use agent_first_data::{json_result, render, OutputFormat, OutputOptions};
use serde_json::json;

let event = json_result(json!({
    "latency_ms": 1280,
    "api_key_secret": "sk-live-example",
})).build();
let rendered = render(event.as_value(), OutputFormat::Json, &OutputOptions::default());
assert!(rendered.contains("\"api_key_secret\":\"***\""));
```

For a finite CLI execution implemented in Rust, create one `CliEmitter`, select
`OutputFormat` through `cli_parse_output`, enable strict protocol, and emit one
terminal result or error. Handle version before clap with
`cli_handle_version_or_continue(raw_args, name, version)`.
For non-CLI HTTP/MCP/SSE serialization, call `redacted_value()` or
`redact_url_secrets()` explicitly at the boundary.

The same library contract is available for Go, Python, and TypeScript. Use the
runtime's native names while preserving the same builders, output formats,
redaction behavior, registry, schema, and conformance fixtures; consult its
root-level `go/README.md`, `python/README.md`, or `typescript/README.md` when
working in that runtime.

## Naming and redaction decisions

- Add unit suffixes to ambiguous numeric fields: `_ms`, `_s`, `_bytes`, `_percent`, `_sats`, `_msats`, currency suffixes, and timestamp suffixes.
- Use `_secret` for values/subtrees that must redact to `***` in every output format.
- Use `_url` only for whole URL values whose userinfo password or suffix-named secret query params should be scrubbed.
- Do not scan arbitrary free text for secrets. Rename fields or configure explicit secret names at the serialization boundary.
- Before changing public API, wire, database, or persistent field names, report the compatibility impact and get approval.

## Evaluation rules

- Trigger this skill for field naming, structured output, configs, logs, CLI output, protocol events, MCP/HTTP/SSE payloads, database columns, wire fields, or persisted JSON. Do not wait for the user to say "AFDATA" when the work changes those surfaces.
- In a repository checkout, treat `spec/agent-first-data.md`,
  `spec/registry.json`, and `spec/protocol-v1.schema.json` as authoritative; in
  an installed skill, use their bundled `references/` equivalents. Do not
  replace them with invented suffix meanings, a reduced schema, or free-text
  interpretation.
- For secrets, fix the field name or serializer configuration. Use `_secret` or configured exact secret names; do not rely on scanning arbitrary strings or object debug output.
- Run `afdata lint` on JSON/JSONL examples, JSON Schema, MCP schemas, or serialized samples when the CLI is available. Run `afdata validate` on finite protocol events/streams. If a check cannot run, report that explicitly.
- Before modifying an external project's public API, wire format, database schema, or persisted field names, stop and report the compatibility impact; request approval before applying the change.

## Redaction behavior contract

Fields named `_secret` or `_url` and values passing through AFDATA output rendering (`render`, in any format) are automatically redacted: secrets become `***`, URL userinfo passwords and suffix-named query parameters are scrubbed. When serializing outside output formatters (HTTP, MCP, SSE), call `redacted_value()` or `redact_url_secrets()` at the serialization boundary. PII redaction, domain-specific privacy policies (header allowlists, API scope sensitivity), and per-field secret naming are each spore's responsibility; the library provides field-name-based mechanics only.

## Logging behavior contract

One-time CLI output uses `json_log()` or `CliEmitter` for a single event; serialization via `render()` applies output formatting. Construct the emitter in finite mode for a one-shot command (`result`→stdout, `error`/`progress`/`log`→stderr) or stream mode for an interleaved event stream (all events on one destination) — see the Output stream contract. Long-running services or processes depending on structured logging (tonic, sqlx, hyper, etc. via tracing) initialize with `afdata_tracing::try_init()` (Rust only) to wire process-wide logging; other languages emit log events explicitly via builders or integrate their own structured logging.

## CLI workflow

Install `afdata` when a shell workflow needs validation, formatting, or skill
administration without embedding a library:

```bash
cargo install agent-first-data
```

Use it for deterministic checks:

```bash
afdata lint payload.json
afdata validate events.jsonl
afdata render payload.json --output yaml
afdata skill validate skills/example-skill
```

- Every command's first positional is its input; `-` reads stdin explicitly.
  There is no implicit stdin fallback, and omitting the input is always a
  usage error that names the fix — never a hang.
- Run `afdata lint` for ordinary JSON/JSONL, JSON Schema, MCP input/output
  schemas, or a structured document (`--input-format`/extension detection
  picks TOML/YAML/dotenv/INI; JSON/JSONL stays the default dual-mode input).
- Run `afdata validate` for AFDATA protocol events or finite event streams.
- Run `afdata render` to apply AFDATA redaction/formatting to arbitrary JSON
  or JSONL. `--output` accepts `json`, `yaml`, or `plain`; JSON and YAML keep
  original keys/values (structure-preserving), Plain is the lossy human
  renderer.
- Treat findings as contract issues unless the owning tool intentionally
  documents a non-AFDATA field.

## Output stream contract

AFDATA CLIs route protocol events by consumption mode, not by event shape.

- A **finite one-shot command** (the default) puts `kind:"result"` on `stdout`
  and `kind:"error"`/`progress`/`log` on `stderr`, so `x=$(tool …)` never
  captures a failure as data, `tool … >/dev/null` never swallows a diagnostic,
  and `tool … | next` never pipes an error envelope in as input. The exit
  code — not the stream — is set by the terminal kind; routing follows `kind`,
  not the exit code.
- An **event stream** (interleaved events consumed in order) keeps every event,
  including `error`, on one stream so ordering survives.
- `afdata` and any CLI built on the library expose `--output-to
  <split|stdout|stderr>` (default `split`). `stdout`/`stderr` collapse the whole
  stream — errors included — onto one destination, for a consumer that would
  rather read one stream and branch on `kind`. `--output` picks the format,
  `--output-to` picks the destination. Send an event stream to a file by
  collapsing it (`--output-to stdout`) and redirecting (`--stdout-file PATH`).
- The raw-scalar readers (`value`/`paths`/`keys`) are intrinsically split (raw
  data on `stdout`, error envelope on `stderr`) and reject a non-default
  `--output-to`.
- When building a CLI on the library, choose the mode by how the output is
  consumed — do not reflexively pick finite. A plain one-shot shell command uses
  **finite** mode (errors → stderr, so shell capture and pipelines stay safe). A
  tool whose output is itself an event stream (interleaved `progress`/`log` then
  a terminal `result`/`error` — e.g. streaming query rows), or that is
  agent-facing and wants the caller to read a single structured channel and
  branch on `kind`, uses **stream/unified** mode (every event, errors included,
  on `stdout`). A stream's terminal error MUST stay on the stream, so such a tool
  keeps even its usage errors on `stdout` for one consistent channel. Never write
  to `stderr` ad hoc — route every diagnostic through the emitter so redaction
  and framing still apply.
- Emit a terminal event with the emitter's `finish` / `finish_result` helpers,
  which map the write outcome to a broken-pipe-safe process exit code (success →
  the given code, a hung-up reader → 0, any other write failure → a nonzero
  output-failure code) instead of hand-rolling that dance — they work in either
  mode (the emitter's mode decides the stream; `finish` only maps the outcome).
  Construct an error with the error builder (`json_error(code, message)` with
  `hint`/`retryable`/extra fields as needed) — that builder *is* the error type —
  and pass the built event to `finish`; there is no separate error-emitting
  convenience.

## Document workflow

Use `get`/`value`/`paths`/`keys` to read a JSON, TOML, YAML, dotenv, or INI
document and `set`/`add`/`remove`/`unset` to edit one in place, instead of
`sed`, regular expressions, or a generic reserializer — those can reorder
keys, drop comments, or change quoting/anchors in the rest of the file. Every
document command's first positional is `FILE` (or `-` for stdin on a read
command); mutation commands never read stdin and reject `-` as a usage error.
To edit only the metadata block of a Markdown page, pass `--input-format
toml-frontmatter` (a `+++` block) or `yaml-frontmatter` (a `---` block): the
same read/mutate commands then address dot-paths inside the frontmatter and
leave the Markdown body byte-for-byte untouched. Frontmatter is never
auto-detected — the format must be named explicitly, since extension detection
only ever resolves to a whole-file format — so reach for it deliberately
instead of `sed`-ing a page's `+++`/`---` header.

- `get FILE` returns the whole document as one AFDATA record
  (`code:"document"`); `get FILE KEY` narrows it to one dot-path value in the
  same shape. Pass every known sensitive legacy field name with
  `--secret-name FIELD` (an exact field name, not a dot-path). A secret-named
  leaf (`_secret`/`_SECRET`, or an exact `--secret-name` match) is still
  redacted to `"***"` even when directly targeted — use `value
  --reveal-secret` when the real value is genuinely needed.
- `value FILE KEY` writes only the raw scalar to stdout, with no AFDATA
  envelope and no forced trailing newline, for direct use in shell
  substitution: `value=$(afdata value config.toml server.host)`. Only scalars
  are supported; a container errors (`document_not_scalar`) — use `get` for a
  subtree. A secret-named leaf errors (`document_secret_redacted`) unless
  `--reveal-secret` is passed. On any failure `value`'s stdout is always
  empty — the error envelope goes to stderr instead — so
  `port=$(afdata value config.toml server.port)` never captures a JSON error
  as data. `--default VAL` prints `VAL` instead of erroring when the path is
  absent or the value is `null` (an empty string is a real value and does not
  trigger it), collapsing the common "read or fall back" shell idiom into one
  call with no `2>/dev/null` guard needed.
- `paths FILE [KEY]` and `keys FILE [KEY]` enumerate a container's immediate
  children, one per line, with the same empty-stdout-on-failure contract as
  `value`. **`paths` output feeds back into afdata** (full grammar-escaped
  dot-paths from the root — pipe straight into `value`/`get`/`unset` or
  extend with `"$p.field"`); **`keys` output goes to external tools** (raw,
  unescaped key names/array indices — what a package manager or another CLI
  expects). Never use `paths`'s output as a literal name, or `keys`'s output
  as a dot-path — they only diverge on a key containing a dot or space, so
  the wrong one works until it silently doesn't. A scalar target errors
  (`document_not_container`); `--missing-ok` turns a missing KEY specifically
  into empty output + exit 0 (other errors still fail). Path grammar does not
  escape spaces or other IFS characters, so the only correct consumption
  shape is a `while IFS= read -r` loop, never `for x in $(...)`:

  ```sh
  while IFS= read -r p; do
      afdata value config.toml "$p.slug"
  done < <(afdata paths config.toml extra.tools)
  ```
- Escape a literal dot as `\.` and a literal backslash as `\\` in a dot-path;
  every command shares this grammar (`paths`'s escaped output re-parses
  through the exact same rules it was emitted from), so an unrecognized
  escape or empty segment is always an error, never a guess.
- `add`/`remove` operate on a keyed list — an array of objects addressed by a
  slug — and always require an explicit `--slug-field FIELD`; the CLI never
  infers an identity field. Keyed editing is implemented for JSON and YAML
  only; TOML, dotenv, and INI return a structured "not implemented" error, so
  edit their list entries by writing the whole value another way. Adding a
  slug that already exists, or removing/unsetting one that does not, is
  always an error (`document_slug_exists`/`document_slug_not_found`/
  `document_path_not_found`) — never a silent no-op; wrap the call yourself
  when idempotence is wanted.
- `set`'s bare VALUE (and `add`'s `FIELD=VALUE`) is always a plain string —
  no shape-guessing, so a leading-zero ID or SHA never gets reinterpreted as
  a number. Writing an exact type goes through `--value-type
  string|number|bool|null|json` instead; `json` is the only entry point for
  an array or object. Overwriting an *existing* scalar of a different kind
  with a bare VALUE is a usage error that names both escape hatches (keep the
  type with `--value-type <kind>`, or convert explicitly with `--value-type
  string`) — a brand-new key never needs `--value-type`.
- For `set`, choose `--secret-from stdin|prompt|fd:<N>|env:<VAR>` instead of a
  positional value when the value must not land in shell history or `ps`:
  `stdin` (pipe), `prompt` (human, terminal echo off), `fd:N` (Unix
  automation that needs stdin for something else), or `env:VAR` (already in
  the process environment). There is no inline `--secret-from=VALUE`-style
  argv source — that would defeat the point.
- Mutations preserve the rest of the source document — comments, key order,
  unrelated formatting — and refuse to write through a symlink or (on Unix) a
  multi-link hardlink target. Treat a failed mutation as leaving the original
  file untouched rather than assuming a partial write, and reread before
  retrying. A format backend that isn't compiled in, or input outside the
  documented dotenv/INI dialect, is a structured error to report, not a cue to
  hand-edit the file. A successful mutation's result carries the `path`
  actually written, for confirming which file changed when scripting several.
- Branch on `error.code`, not the message text: document errors use stable
  codes (`document_path_not_found`, `document_type_mismatch`,
  `document_slug_not_found`, `document_slug_exists`, `document_parse_failed`,
  `document_io_failed`, and a few narrower ones), while a malformed
  invocation (`--input-format`, `--value-type`, `FIELD=VALUE`, a mutation
  given `-`, …) is always `document_usage_error` at exit 2 — distinct from
  every runtime document error, which exits 1.

## Review checklist

1. Numeric fields with units carry the right suffix and safe integer/string representation.
2. Timestamps use `_epoch_s`, `_epoch_ms`, decimal-string `_epoch_ns`, or `_rfc3339`.
3. Secrets use `_secret` or configured exact secret names; no fallback path leaks them.
4. CLI output uses AFDATA helpers for JSON/YAML/plain formatting and structured errors, and routes by consumption mode (finite: `result`→stdout, `error`→stderr; stream: one destination) rather than writing to stderr ad hoc.
5. Logs use AFDATA structured logging or equivalent field-name redaction.
6. Transport payloads preserve the protocol envelope when AFDATA compliance is claimed.
