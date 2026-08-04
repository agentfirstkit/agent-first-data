<!-- Canonical focused reference for the installed Agent-First Data skill. -->

# CLI, protocol, help, and logging

## Protocol v1

Every event uses `kind`, one same-named payload, and top-level `trace`:

```json
{"kind":"log","log":{"message":"startup","level":"info","event":"startup"},"trace":{}}
{"kind":"progress","progress":{"message":"syncing","current":3,"total":10},"trace":{"duration_ms":5}}
{"kind":"result","result":{"code":"ok","items":3},"trace":{"duration_ms":12}}
{"kind":"error","error":{"code":"not_found","message":"item not found","retryable":false},"trace":{"duration_ms":8}}
```

- `result` and `error` are terminal.
- Strict events have object-valued `trace`; strict errors also have non-empty
  `code`/`message` and boolean `retryable`.
- Keep the same envelope across CLI, HTTP, MCP, SSE, and logs.
- Validate exact rules with `protocol-v1.schema.json` or
  `afdata validate --strict`.

## Channel policy

- Finite command: result to stdout; error/progress/log to stderr.
- Ordered stream: keep every event on one destination.
- `--output json|yaml|plain` chooses serialization.
- `--output-to split|stdout|stderr` chooses destination.
- Raw actions have raw success output and a strict JSON domain-error emitter;
  they do not accept protocol format or destination options.

Resolved output plans, emitters, and raw sinks come from AFDATA. Do not write
diagnostics ad hoc or reparse output flags in the application.

In Rust, `OutputFormat`, `OutputTo`, and
`afdata_tracing::LogFormat` are closed enums with `FromStr`, `Display`, and
serde support. Keep them typed in configuration; an unknown value must fail
parsing rather than silently selecting a fallback.

## Closed-world CLI implementation

Build one `CliSpec` and register:

- one explicit `CommandSpec` for root and each command path;
- command-local `ArgSpec`s with canonical long names or positional indexes;
- named `Combination`s containing fixed enum values, required arguments,
  optional arguments, an `action_id`, and an `OutputSpec`.

Every application argument must appear in at least one combination. Fixed,
required, and optional sets are disjoint. A combination's optional arguments
may appear in any subset; conditional optional relationships require separate
combinations. `CliSpec::build()` rejects overlap, invalid defaults, uncovered
arguments, invalid output contracts, and duplicate identities.

`ResolvedInvocation::required` returns `Option` so a misspelled caller-side id
cannot masquerade as a valid flag value. `BoundCliSpec::execute` likewise
returns `None` for an invocation resolved by an independently built registry;
never dispatch an invocation through a different binding.

An invocation must match exactly one combination. Known arguments in an
unregistered mixture return:

```json
{"kind":"error","error":{"code":"cli_unregistered_combination","message":"arguments do not match a registered CLI combination","retryable":false,"hint":"run `tool sync --help` and choose one registered combination"},"trace":{}}
```

All CLI errors use exit 2 and raw JSON on stderr before domain I/O. The `code`
names the failure, so the same rule covers CLI and document errors alike:
branch on `error.code`, never on message text.

`cli_unknown_command`, `cli_unknown_argument`, `cli_missing_argument_value`,
`cli_invalid_argument_value`, `cli_duplicate_argument`,
`cli_unexpected_positional`, `cli_unregistered_combination`,
`cli_invalid_utf8`. A CLI not compiled from a registry reports the generic
`cli_error`.

`message` identifies a safe argument spelling or the failure category and
`hint` gives the command to run next; neither ever carries a raw value.

The full command path comes first. There are no application globals, shorts,
abbreviations, optional-value options, compatibility no-ops, rule priorities,
or post-parse combination callbacks. Reserved built-ins are reserved only where
AFDATA parses them: `--help`, `--output`, `--output-to`, `--stdout-file`, and
`--stderr-file` at every command; `--version` and `--docs` at the root command
only, so a subcommand may declare its own (`tool release --version 1.2.0`).

Output tokens do not select or distinguish business combinations. Select one
application shape first, then validate output against only that combination.
Parse/match errors do not trust unresolved output tokens.

After `CliSpec::build()` and `bind_actions`, resolve argv to `CliOutcome` and
dispatch the selected `action_id`. The application owns its process lifecycle:
finite commands, long-running event streams, raw bytes, custom redaction, and
pre-resolution security checks do not share one execution host. Use the
resolved `OutputPlan` with AFDATA emitters and sinks rather than reparsing
output flags.

## Version

Version is a root-only lifecycle combination generated from `CliSpec`:

```json
{"kind":"result","result":{"code":"version","name":"tool","version":"1.2.3"},"trace":{}}
```

It uses `CliSpec.lifecycle_output`. Past the root the name is the
application's: a subcommand that declares `--version` gets its own argument,
and one that does not rejects it as an unregistered combination.

## Help v2

`tool [command] --help` answers in one round trip, with every legal shape of
that command, each complete:

```json
{"kind":"result","result":{"code":"help","help":{"schema":"cli-help-v2","command_path":"tool sync","about":"Copy records between stores","shapes":[{"id":"sync","usage":"tool sync --source <SOURCE> [--dry-run] [--output <json|yaml|plain>]"}],"notes":{"--source":"Store to read from","--dry-run":"Report what would change"},"defaults":{"--output":"json"}}},"trace":{}}
```

- JSON is the default; `--output plain` is the human catalog.
- Each `shapes[]` entry is generated from the registry and carries a complete
  `usage`: fixed, required, optional, and output arguments, with placeholders
  rather than secret values. A shape whose fixed argument is already satisfied
  by that argument's default is shown bracketed, because it may be omitted.
- An argument with a closed value set names those values inline
  (`--output <json|yaml|plain>`), so no error is needed to discover them.
- `notes` and `defaults` are keyed by the spelling `usage` uses, so a caller
  looks up the token it just read.
- There is no second help level: what it could omit — the optional arguments —
  would be registered but undiscoverable to a caller that stopped at the first.
- There is no recursive argument catalog in v2. `--docs` renders the whole
  registry as Markdown.
- `cli-spec-v1` is the authoring contract, not a runtime export: it describes
  the registry a tool is compiled from. No CLI emits one. Read `--docs` for the
  whole registry, `--help` for one command.
- Help describes invocation shapes and arguments. Domain idempotency, runtime
  errors, side effects, and recovery behavior belong in the owning tool's
  focused documentation and tests, not in `CliSpec` or help-v2.

Validate with `cli-help-v2.schema.json`.

A `CliEmitter` owns one logical finite invocation or ordered event stream. A
multiplex transport owns one emitter/lifecycle state per request; AFDATA does
not provide a global keyed-emitter registry.

## Public domain errors

Declare caller-safe Rust errors with `ErrorSpec` and collect them in an
`ErrorCatalog` when lookup by code is useful. The declaration contains only
stable `code`, `message`, optional `hint`, and `retryable`. Parser, database,
network, or other third-party diagnostics must be logged separately; no catalog
API accepts or automatically formats a diagnostic into the public event.

## Logging

One-shot programs emit `kind:"log"` through the same emitter. Long-running
Rust services compose `afdata_tracing::AfdataLayer::new(format, redactor)` into
their subscriber and may inject a test or application sink with `with_writer`.
Keep its `StructuredLogHandle` when a nested `serde_json::Value` must retain
objects and arrays; ordinary tracing and direct nested events then share the
same writer, redactor, format, lock, and order. `try_init` remains the convenient
global-subscriber shortcut, not the only integration path. Other runtimes emit
events explicitly or integrate their structured logger.

Redaction depends on field names. Log `api_key_secret` as a structured field;
do not hide it inside Debug output or interpolated prose.

For in-process Rust tests, run `lint_value` on the real serialized value and
combine `assert_no_lint_findings`, `assert_strict_event`, and
`assert_redaction_canary_absent` as appropriate. This tests the boundary
without temporary files or an `afdata` subprocess.

## Review checklist

1. Every application argument belongs to at least one combination.
2. The build rejects overlap, including fixed values satisfied by defaults.
3. Usage failures happen before config, secret-source, network, or domain I/O.
4. Every CLI error names its failure in `code`, exits 2, leaves stdout empty,
   writes strict JSON to stderr, and carries no secret values.
5. Output contracts are selected after the business combination.
6. Help examples and detail usage are generated from the registry and validate
   against help-v2.
7. Version values appear only in version output.
8. Public errors contain catalogued fields only; runtime diagnostics stay in logs.
9. Transport bodies claiming AFDATA protocol compatibility are redacted and
   strict before serialization.
