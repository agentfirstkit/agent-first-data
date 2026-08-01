# AFDATA protocol v1 decision record

Status: accepted.

AFDATA protocol v1 uses one discriminator field, `kind`, and one payload field
whose name is identical to `kind`.

```json
{"kind":"result","result":{"rows":12}}
{"kind":"error","error":{"code":"file_not_found","message":"missing input","hint":"check --input"}}
{"kind":"progress","progress":{"percent":50}}
{"kind":"log","log":{"event":"startup"}}
```

Top-level fields are closed:

- `kind`
- the payload field matching `kind`
- optional `trace`

`trace`, when present, must be a JSON object. Its internal fields are owned by
the emitting tool. Normal recursive AFDATA redaction still applies when the
event is formatted.

`result`, `progress`, and `log` may carry any valid JSON value. AFDATA does not
define business payload structure for these event kinds.

`error` must be a JSON object with:

- `code`: non-empty string
- `message`: non-empty string
- optional `hint`: string

Tools may add extension fields directly to the `error` object. There is no
required `details` wrapper.

Finite structured CLI event streams follow this lifecycle:

```text
(log | progress)* -> exactly one (result | error) -> end
```

Canonical CLIs do not need `--stream` or `--result-only` mode switches. The
default finite execution emits exactly one terminal `result` or `error` event.
Diagnostic `log` or `progress` events are opt-in, for example through an
explicit `--log ...` filter or `--verbose` shorthand. TTY detection,
redirection, and pipe targets must not change this event policy.

`result` maps to process exit code `0`. `error` maps to a non-zero process exit
code. AFDATA does not define a global detailed exit-code table.

Cancellation is represented as an ordinary tool-defined `error` event when the
tool can observe the cancellation and still write a terminal event. For example,
a tool may use `error.code: "cancelled"`, but AFDATA does not reserve that code.
If stdout or the transport is already closed and the terminal event cannot be
written, the outcome is a transport interruption with unknown business result.

A CLI resolution failure names itself in `code`, the same way a document
failure does: `cli_unknown_argument`, `cli_unknown_command`,
`cli_missing_argument_value`, `cli_invalid_argument_value`,
`cli_duplicate_argument`, `cli_unexpected_positional`,
`cli_unregistered_combination`, `cli_invalid_utf8`. A CLI that is not compiled
from a registry — anything built with `build_cli_error` — reports the generic
`cli_error`.

There is one place to look. An earlier draft put the classification in a
separate `rule` field beside a generic `cli_error`, alongside `command_path`
and `argument_names`; that asked an agent to learn a second branching key for
one family of errors while `document_*` already spelled its classification into
the code, and it cost a closed enum kept in step across four files and four
mirrored schemas. `message` names the offending argument, `hint` gives the
command to run next, and neither ever quotes a raw value. Every such error is
`retryable: false` and exits 2.

The machine-readable event schema is:

- `spec/protocol-v1.schema.json`
- `$id`: `https://agentfirstkit.org/schemas/agent-first-data/protocol-v1.schema.json`

The shared fixtures proving constructor, validation, and lifecycle behavior are:

- `spec/fixtures/protocol.json`
- `spec/fixtures/protocol_streams.json`
