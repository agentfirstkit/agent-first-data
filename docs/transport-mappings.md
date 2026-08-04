# AFDATA recommended transport mappings

These mappings are recommendations. A tool can be AFDATA-core compliant without
implementing any specific transport, and hosts may choose another reasonable
mapping when their protocol requires it.

## CLI

Structured CLI output uses AFDATA protocol v1 events. Finite commands split
result data from diagnostics; ordered event streams keep all events together.

JSON multi-event output is JSONL/NDJSON: one complete event per line.
Plain multi-event output is one display event per line. YAML multi-event output
uses an explicit `---` document boundary for every event. Agent-facing machine
input remains JSON; YAML and plain are display formats.

Finite CLI executions follow:

```text
(log | progress)* -> exactly one (result | error) -> end
```

The default finite routing is:

- `result` → stdout
- `error`, `log`, and `progress` → stderr

An ordered event stream routes every kind to one destination (stdout by
default), because splitting would lose order. `--output-to split|stdout|stderr`
selects finite split routing or a collapsed destination; an event-stream
command rejects `split`.

Routing follows `kind`, not process status. A `result` remains on the result
channel when a tool-defined condition uses a non-zero exit code, and an `error`
remains on the error channel. AFDATA reserves exit 2 for closed-world CLI usage
failures but does not define a global detailed exit-code table.

If a finite CLI observes cancellation before completion and its error
destination is still writable, it may emit a tool-defined `error` event such
as `error.code: "cancelled"` and exit non-zero. If the selected destination is
closed first, including a broken pipe on a collapsed stdout stream, the CLI
cannot reliably send a terminal AFDATA event; classify the run as transport
interruption with unknown business result. Broken pipes should not produce
panic, traceback, or stack diagnostics.

## HTTP

HTTP response bodies may use an AFDATA envelope.

Recommended status mapping:

- `result`: an appropriate `2xx`
- accepted asynchronous work: `202` with a `progress` body when useful
- `error`: an appropriate `4xx` or `5xx`

AFDATA does not define a global mapping from `error.code` to HTTP status, and
does not require RFC 9457 Problem Details.

## MCP

For MCP tools, place the final AFDATA envelope in
`CallToolResult.structuredContent`.

Recommended result mapping:

- `result`: `isError: false`, or omit `isError`
- `error`: `isError: true`

Keep `content` short and human-readable. JSON-RPC protocol errors are distinct
from tool execution errors. Intermediate progress should use MCP native
progress notifications rather than synthetic final envelopes.

## SSE

Each `data:` frame carries one complete AFDATA envelope.

The stream closes after a terminal `result` or `error`. Repeating the kind in
the SSE `event:` field is not required.

The HTTP `200` after connection establishment only means the stream is open.
The final business state is determined by the terminal `data.kind`.

If the connection closes before any terminal event is received, classify the
outcome as transport interruption with unknown business result.

## Explicitly out of scope

AFDATA transport recommendations do not define:

- transport-specific `SafeValue`
- MCP annotations or tasks
- HTTP Problem Details
- SSE event IDs, replay, or exactly-once delivery
- server log structure
- server process lifecycle; a server shutdown may either let active requests
  return a tool-defined error such as `server_shutting_down`, or interrupt the
  transport before a terminal event is available
