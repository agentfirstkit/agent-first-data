<!-- Canonical focused reference for the installed Agent-First Data skill. -->

# CLI, protocol, help, and logging

Read this reference for protocol envelopes, output routing, CLI helpers,
structured help, version output, and logging.

## Contents

- [Protocol v1](#protocol-v1)
- [Channel policy](#channel-policy)
- [CLI implementation](#cli-implementation)
- [Version is on demand](#version-is-on-demand)
- [Help v1 and progressive discovery](#help-v1-and-progressive-discovery)
- [Logging](#logging)
- [Review checklist](#review-checklist)

## Protocol v1

Every event uses `kind`, one same-named payload, and top-level `trace`:

```json
{"kind":"log","log":{"message":"startup","level":"info","event":"startup"},"trace":{}}
{"kind":"progress","progress":{"message":"syncing","current":3,"total":10},"trace":{"duration_ms":5}}
{"kind":"result","result":{"code":"ok","items":3},"trace":{"duration_ms":12}}
{"kind":"error","error":{"code":"not_found","message":"item not found","retryable":false},"trace":{"duration_ms":8}}
```

- `result` and `error` are terminal.
- `error.code` and `error.message` are non-empty. Strict events also include
  `error.retryable`.
- Log and progress payload fields are tool-defined.
- Strict validation requires object-valued `trace` on every event.
- Keep this envelope across CLI, HTTP JSON, MCP, SSE, and JSONL streams when
  claiming protocol compatibility.

Validate exact envelope rules with `protocol-v1.schema.json` or
`afdata validate --strict`.

## Channel policy

Choose routing by how output is consumed:

- Finite one-shot command: `result` goes to stdout; `error`, `progress`, and
  `log` go to stderr.
- Ordered event stream: every event, including terminal error, stays on one
  destination so ordering survives.
- `--output json|yaml|plain` selects serialization.
- `--output-to split|stdout|stderr` selects destination policy. `stdout` and
  `stderr` collapse the whole event stream onto one destination.
- Raw-scalar document readers are intrinsically split and reject a non-default
  `--output-to`.

Do not write diagnostics to stderr ad hoc. Route them through the emitter so
redaction, framing, and stream selection remain correct. Native panic or
traceback bytes may remain raw stderr.

Use emitter `finish`/`finish_result` helpers for terminal output. They preserve
the requested exit code, map a broken pipe to success, and map other write
failures to the runtime's output-failure code.

## CLI implementation

Use the shared runtime helpers instead of custom output parsing or envelopes:

- `cli_parse_output`
- `cli_parse_log_filters`
- `build_cli_error`
- `build_cli_version`
- `cli_handle_version_or_continue`
- `CliEmitter`

Rust adds output-aware Clap help through
`cli_handle_version_or_help_or_continue`. Call it once before `try_parse`, pass
the caller's own `clap::Command`, and use `HelpConfig::output_aware()`.
Separate version/help helpers remain lifecycle escape hatches.

Use `try_parse`, not a parser that exits with raw usage text. Convert usage
failures into `kind:"error"` with `code:"cli_error"`, a useful hint,
`retryable:false`, and `trace:{}`.

## Version is on demand

Handle top-level `--version` before the parser's built-in text exit.
Return a protocol result:

```json
{"kind":"result","result":{"code":"version","name":"tool","version":"1.2.3"},"trace":{}}
```

Optional fields are `display_name` and opaque `build`. Explicit `--output` or
`--json` wins; otherwise inherit the command's declared output default.

Only recognize version before the first positional/subcommand. A malformed
request such as `--version --output xml` returns a structured CLI error.

Help advertises `--version` when supported but never embeds version or
library values. Agents request them only when needed.

## Help v1 and progressive discovery

Scope and format are independent:

- `--help` describes the selected command and one subcommand level.
- `--help --recursive` emits a compact selected-subtree index.
- `--output plain|json|yaml|markdown` chooses the representation.
- Omitted output inherits the selected command's declared `--output` default,
  with the nearest selected/ancestor declaration winning.
- A fixed-format CLI without `--output` passes its normal format through
  `HelpConfig::output_aware_with_fallback(...)`; a JSON-only CLI uses
  `HelpFormat::Json`.
- A bare `--recursive` falls through to the application parser.

Plain is conventional terminal help. Markdown is the complete documentation
export. JSON/YAML is one protocol-v1 result whose `result.code` is `help` and
whose model is under `result.help`:

```json
{"kind":"result","result":{"code":"help","help":{"scope":"one_level","command_path":"tool sync","name":"sync","about":"Synchronize records","usage":"[OPTIONS]","arguments":[{"name":"--limit","value":"COUNT","help":"Maximum records"}]}},"trace":{}}
```

The exact schema is `cli-help-v1.schema.json`. The help model permits:

- root-only `scope`, `command_path`, and `inherited_arguments_from`;
- command fields `name`, `about`, `usage`, `arguments`, `subcommands`;
- argument fields `name`, `short`, `help`, `required:true`, `global:true`,
  `repeatable:true`, `value`/`values`, and `default`/`defaults`.

Copy `command_path` when invoking the tool. Root `name` is a label and may be
the CLI's branded display name rather than its binary name.

Omit empty arrays, empty strings, false booleans, and absent defaults. A
positional's `name` is already its placeholder, so do not duplicate it in
`value`. Redact secret defaults to `***`.

Do not add nested `help.code`, `versions`, raw formatted help, repeated command
paths on descendants, or generic `description`. Keep long-form Markdown out of
compact structured output; a future explicit full mode must call the field
`description_markdown`.

Recursive output includes each command and argument once. Do not repeat global
arguments on every descendant. Scoped structured help lists their defining
ancestor command paths in `inherited_arguments_from`; query those paths only
when the shared options are relevant. Scoped plain help includes inherited
globals as conventional terminal help does. One-level child entries are
summaries; recursive child entries may include their own usage, arguments, and
children.

## Logging

One-shot programs emit `kind:"log"` through the same emitter. Long-running
Rust services using `tracing` initialize once with
`afdata_tracing::try_init`; other runtimes emit log events explicitly or
integrate their structured logger.

Redaction depends on field names. Log `api_key_secret` as a structured field;
do not hide it inside Debug output or interpolated prose. Tool-defined log
payloads commonly include `message`, `level`, `event`, and span fields, but
protocol v1 does not reserve them.

## Review checklist

1. Validate representative terminal and non-terminal events in strict mode.
2. Confirm finite versus ordered-stream routing matches the consumer.
3. Confirm usage failures and early version/help failures stay structured.
4. Confirm help-v1 uses the exact schema and command path.
5. Confirm version values appear only in version output.
6. Confirm logs expose secret-bearing values only under redacted field names.
