# agent-first-data

**Agent-First Data (AFDATA)** — Suffix-driven output formatting and protocol templates for AI agents.

The field name is the schema. Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact, no external schema needed.

## Installation

```bash
npm install agent-first-data
```

## Quick Example

A backup tool invoked from the CLI — flags, env vars, and config all use the same suffixes:

```bash
API_KEY_SECRET=sk-1234 cloudback --timeout-s 30 --max-file-size-bytes 10737418240 /data/backup.tar.gz
```

For CLI diagnostics, enable log categories explicitly:

```bash
--log startup,request,progress,retry,redirect
--verbose   # shorthand for all categories
```

Without these flags, startup diagnostics should stay off by default.

The tool reads env vars, flags, and config — all with AFDATA suffixes — and can emit a startup diagnostic event:

```typescript
import { buildJson, outputJson, outputYaml, outputPlain } from "agent-first-data";

const startup = buildJson(
  "log",
  {
    event: "startup",
    config: { timeout_s: 30, max_file_size_bytes: 10737418240 },
    args: { input_path: "/data/backup.tar.gz" },
    env: { API_KEY_SECRET: process.env.API_KEY_SECRET ?? null },
  },
);
```

Three output formats, same data:

```
JSON:  {"code":"log","event":"startup","args":{"input_path":"/data/backup.tar.gz"},"config":{"max_file_size_bytes":10737418240,"timeout_s":30},"env":{"API_KEY_SECRET":"***"}}
YAML:  code: "log"
       event: "startup"
       args:
         input_path: "/data/backup.tar.gz"
       config:
         max_file_size: "10.0GB"
         timeout: "30s"
       env:
         API_KEY: "***"
Plain: args.input_path=/data/backup.tar.gz code=log event=startup config.max_file_size=10.0GB config.timeout=30s env.API_KEY=***
```

`--timeout-s` → `timeout_s` → `timeout: 30s`. `API_KEY_SECRET` → `API_KEY: "***"`. The suffix is the schema.

## API Reference

Total: **12 public APIs and 1 type** + **AFDATA logging** (3 protocol builders + 3 output functions + 1 internal + 1 utility + 4 CLI helpers + `OutputFormat`)

### Protocol Builders (returns JsonValue)

Build AFDATA protocol structures. Return JSON-serializable objects for API responses.

```typescript
type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };

// Success (result)
buildJsonOk(result: JsonValue, trace?: JsonValue): JsonValue

// Error (simple message)
buildJsonError(message: string, trace?: JsonValue): JsonValue

// Generic (any code + fields)
buildJson(code: string, fields: JsonValue, trace?: JsonValue): JsonValue
```

**Use case:** API responses (frameworks like Express automatically serialize)

**Example:**
```typescript
import { buildJsonOk, buildJsonError, buildJson } from "agent-first-data";

// Startup
const startup = buildJson(
  "log",
  {
    event: "startup",
    config: { api_key_secret: "sk-123", timeout_s: 30 },
    args: { config_path: "config.yml" },
    env: { RUST_LOG: "info" },
  },
);

// Success (always include trace)
const response = buildJsonOk(
  { user_id: 123 },
  { duration_ms: 150, source: "db" },
);

// Error
const error = buildJsonError("user not found", { duration_ms: 5 });

// Specific error code
const notFound = buildJson(
  "not_found",
  { resource: "user", id: 123 },
  { duration_ms: 8 },
);
```

### CLI/Log Output (returns string)

Format values for CLI output and logs. **All formats redact `_secret` fields.** YAML and Plain also strip suffixes from keys and format values for human readability.

```typescript
outputJson(value: JsonValue): string   // Single-line JSON, original keys, for programs/logs
outputYaml(value: JsonValue): string   // Multi-line YAML, keys stripped, values formatted
outputPlain(value: JsonValue): string  // Single-line logfmt, keys stripped, values formatted
```

**Example:**
```typescript
import { outputJson, outputYaml, outputPlain } from "agent-first-data";

const data = {
  user_id: 123,
  api_key_secret: "sk-1234567890abcdef",
  created_at_epoch_ms: 1738886400000,
  file_size_bytes: 5242880,
};

// JSON (secrets redacted, original keys, raw values)
console.log(outputJson(data));
// {"api_key_secret":"***","created_at_epoch_ms":1738886400000,"file_size_bytes":5242880,"user_id":123}

// YAML (keys stripped, values formatted, secrets redacted)
console.log(outputYaml(data));
// ---
// api_key: "***"
// created_at: "2025-02-07T00:00:00.000Z"
// file_size: "5.0MB"
// user_id: 123

// Plain logfmt (keys stripped, values formatted, secrets redacted)
console.log(outputPlain(data));
// api_key=*** created_at=2025-02-07T00:00:00.000Z file_size=5.0MB user_id=123
```

### Internal Tools

```typescript
internalRedactSecrets(value: JsonValue): void  // Manually redact secrets in-place
```

Most users don't need this. Output functions automatically protect secrets.

### Utility Functions

```typescript
parseSize(s: string): number | null  // Parse "10M" → bytes
```

**Example:**
```typescript
import { parseSize } from "agent-first-data";

parseSize("10M");  // 10485760
parseSize("1.5K"); // 1536
parseSize("512");  // 512
```

### CLI Helpers (for tools built on AFDATA)

Shared helpers that prevent flag-parsing drift between CLI tools. Use these instead of reimplementing `--output` and `--log` handling in each tool.

```typescript
type OutputFormat = "json" | "yaml" | "plain"

cliParseOutput(s: string): OutputFormat             // Parse --output flag; throws on unknown
cliParseLogFilters(entries: string[]): string[]     // Normalize --log: trim, lowercase, dedup, remove empty
cliOutput(value: JsonValue, format: OutputFormat): string  // Dispatch to outputJson/Yaml/Plain
buildCliError(message: string): JsonValue           // {code:"error", error_code:"invalid_request", retryable:false, trace:{duration_ms:0}}
```

**Canonical pattern** — parse all flags before doing work, emit JSONL errors to stdout:

```typescript
import {
  type OutputFormat, cliParseOutput, cliParseLogFilters,
  cliOutput, buildCliError, outputJson,
} from "agent-first-data";

let fmt: OutputFormat;
try {
  fmt = cliParseOutput(outputArg);
} catch (e) {
  console.log(outputJson(buildCliError((e as Error).message)));
  process.exit(2);
}

const log = cliParseLogFilters(logArg ? logArg.split(",") : []);
// ... do work ...
console.log(cliOutput(result, fmt));
```

See `examples/agent_cli.ts` for the complete working example (`npx tsx --test examples/agent_cli.ts`).

## Usage Examples

### Example 1: REST API

```typescript
import { buildJsonOk } from "agent-first-data";
import express from "express";

const app = express();

app.get("/users/:id", (req, res) => {
  const response = buildJsonOk(
    { user_id: Number(req.params.id), name: "alice" },
    { duration_ms: 150, source: "db" },
  );
  // API returns raw JSON — no output processing, no key stripping
  res.json(response);
});
```

### Example 2: CLI Tool (Complete Lifecycle)

```typescript
import { buildJsonOk, buildJson, outputYaml, outputPlain } from "agent-first-data";

// 1. Startup
const startup = buildJson(
  "log",
  {
    event: "startup",
    config: { api_key_secret: "sk-sensitive-key", timeout_s: 30 },
    args: { input_path: "data.json" },
    env: { RUST_LOG: "info" },
  },
);
console.log(outputYaml(startup));
// ---
// code: "log"
// event: "startup"
// args:
//   input_path: "data.json"
// config:
//   api_key: "***"
//   timeout: "30s"
// env:
//   RUST_LOG: "info"

// 2. Progress
const progress = buildJson(
  "progress",
  { current: 3, total: 10, message: "processing" },
  { duration_ms: 1500 },
);
console.log(outputPlain(progress));
// code=progress current=3 message=processing total=10 trace.duration=1.5s

// 3. Result
const result = buildJsonOk(
  {
    records_processed: 10,
    file_size_bytes: 5242880,
    created_at_epoch_ms: 1738886400000,
  },
  { duration_ms: 3500, source: "file" },
);
console.log(outputYaml(result));
// ---
// code: "ok"
// result:
//   created_at: "2025-02-07T00:00:00.000Z"
//   file_size: "5.0MB"
//   records_processed: 10
// trace:
//   duration: "3.5s"
//   source: "file"
```

### Example 3: JSONL Output

```typescript
import { buildJsonOk, outputJson } from "agent-first-data";

const result = buildJsonOk(
  { status: "success" },
  { duration_ms: 250, api_key_secret: "sk-123" },
);

// Print JSONL to stdout (secrets redacted, one JSON object per line)
// Channel policy: machine-readable protocol/log events must not use stderr.
console.log(outputJson(result));
// {"code":"ok","result":{"status":"success"},"trace":{"api_key_secret":"***","duration_ms":250}}
```

## Complete Suffix Example

```typescript
import { outputYaml, outputPlain } from "agent-first-data";

const data = {
  created_at_epoch_ms: 1738886400000,
  request_timeout_ms: 5000,
  cache_ttl_s: 3600,
  file_size_bytes: 5242880,
  payment_msats: 50000000,
  price_usd_cents: 9999,
  success_rate_percent: 95.5,
  api_key_secret: "sk-1234567890abcdef",
  user_name: "alice",
  count: 42,
};

// YAML output (keys stripped, values formatted, secrets redacted)
console.log(outputYaml(data));
// ---
// api_key: "***"
// cache_ttl: "3600s"
// count: 42
// created_at: "2025-02-07T00:00:00.000Z"
// file_size: "5.0MB"
// payment: "50000000msats"
// price: "$99.99"
// request_timeout: "5.0s"
// success_rate: "95.5%"
// user_name: "alice"

// Plain logfmt output (same transformations, single line)
console.log(outputPlain(data));
// api_key=*** cache_ttl=3600s count=42 created_at=2025-02-07T00:00:00.000Z file_size=5.0MB payment=50000000msats price=$99.99 request_timeout=5.0s success_rate=95.5% user_name=alice
```

## AFDATA Logging

AFDATA-compliant structured logging. Every log line is formatted using the library's own `outputJson`/`outputPlain`/`outputYaml` functions. Span fields are carried via `AsyncLocalStorage` (async-safe), automatically flattened into each log line. Zero dependencies beyond Node.js built-ins.

### API

```typescript
import { log, span, initJson, initPlain, initYaml } from "agent-first-data";

// Format selectors — set the output format for all subsequent log calls
initJson()    // Single-line JSONL (secrets redacted, original keys) — default
initPlain()   // Single-line logfmt (keys stripped, values formatted)
initYaml()    // Multi-line YAML (keys stripped, values formatted)

// Logger — each method outputs a single log line to stdout
log.trace(msg, fields?)
log.debug(msg, fields?)
log.info(msg, fields?)
log.warn(msg, fields?)
log.error(msg, fields?)

// Span — run fn with additional fields on all log events
span<T>(fields, fn: () => T): T  // works with sync and async functions
```

### Setup

```typescript
import { log, initJson, initPlain, initYaml } from "agent-first-data";

// JSON output for production (one JSONL line per event, secrets redacted)
initJson();  // default, can be omitted

// Plain logfmt for development (keys stripped, values formatted)
initPlain();

// YAML for detailed inspection (multi-line, keys stripped, values formatted)
initYaml();
```

### Log Output

Output format depends on the init function used.

```typescript
log.info("Server started");
// JSON:  {"timestamp_epoch_ms":1739000000000,"message":"Server started","code":"info"}
// Plain: code=info message="Server started" timestamp_epoch_ms=1739000000000
// YAML:  ---
//        code: "info"
//        message: "Server started"
//        timestamp_epoch_ms: 1739000000000

log.warn("DNS lookup failed", { error: "timeout", domain: "example.com" });
// JSON:  {"timestamp_epoch_ms":...,"message":"DNS lookup failed","error":"timeout","domain":"example.com","code":"warn"}
// Plain: code=warn domain=example.com error=timeout message="DNS lookup failed" ...
```

### Span Support

Use `span()` to add fields to all log events within the callback. Spans nest and work with both sync and async functions.

```typescript
import { log, span } from "agent-first-data";

await span({ request_id: "abc-123" }, async () => {
  log.info("Processing");
  // {"timestamp_epoch_ms":...,"message":"Processing","request_id":"abc-123","code":"info"}

  await span({ step: "validate" }, async () => {
    log.info("Validating input");
    // {"timestamp_epoch_ms":...,"message":"Validating input","request_id":"abc-123","step":"validate","code":"info"}
  });
});
```

### Custom Code Override

The `code` field defaults to the log level. Override with an explicit field:

```typescript
log.info("Server ready", { code: "log", event: "startup" });
// {"timestamp_epoch_ms":...,"message":"Server ready","code":"log","event":"startup"}
```

### Output Fields

Every log line contains:

| Field | Type | Description |
|:------|:-----|:------------|
| `timestamp_epoch_ms` | number | Unix milliseconds |
| `message` | string | Log message |
| `code` | string | Level (trace/debug/info/warn/error) or explicit override |
| *span fields* | any | From `span()` callback |
| *event fields* | any | From fields argument |

### Log Output Formats

All three formats use the library's own output functions, so AFDATA suffix processing applies to log fields too:

| Format | Function | Keys | Values | Use case |
|:-------|:---------|:-----|:-------|:---------|
| **JSON** | `initJson` | original (with suffix) | raw | production, log aggregation |
| **Plain** | `initPlain` | stripped | formatted | development, compact scanning |
| **YAML** | `initYaml` | stripped | formatted | debugging, detailed inspection |

All formats automatically redact `_secret` fields in log output.

## Output Formats

Three output formats for different use cases:

| Format | Structure | Keys | Values | Use case |
|:-------|:----------|:-----|:-------|:---------|
| **JSON** | single-line | original (with suffix) | raw | programs, logs |
| **YAML** | multi-line | stripped | formatted | human inspection |
| **Plain** | single-line logfmt | stripped | formatted | compact scanning |

All formats automatically redact `_secret` fields.

## Supported Suffixes

- **Duration**: `_ms`, `_s`, `_ns`, `_us`, `_minutes`, `_hours`, `_days`
- **Timestamps**: `_epoch_ms`, `_epoch_s`, `_epoch_ns`, `_rfc3339`
- **Size**: `_bytes` (auto-scales to KB/MB/GB/TB), `_size` (config input, pass through)
- **Currency**: `_msats`, `_sats`, `_btc`, `_usd_cents`, `_eur_cents`, `_jpy`, `_{code}_cents`
- **Other**: `_percent`, `_secret` (auto-redacted in all formats)

## Repository

This package is part of the [agent-first-data](https://github.com/cmnspore/agent-first-data) repository, which also contains:

- **`spec/`** — Full AFDATA specification with suffix definitions, protocol format rules, and cross-language test fixtures
- **`skills/`** — AI coding agent skill for working with AFDATA conventions

To run tests, clone the full repository (tests use shared cross-language fixtures from `spec/fixtures/`):

```bash
git clone https://github.com/cmnspore/agent-first-data
cd agent-first-data/typescript
npm test
```

## License

MIT
