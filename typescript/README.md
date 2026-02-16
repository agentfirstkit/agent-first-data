# agent-first-data

**Agent-First Data (AFD)** — Suffix-driven output formatting and protocol templates for AI agents.

The field name is the schema. Agents read `latency_ms` and know milliseconds, `api_key_secret` and know to redact, no external schema needed.

## Installation

```bash
npm install agent-first-data
```

## API Reference

Total: **9 public APIs** (4 protocol builders + 3 output functions + 1 internal + 1 utility)

### Protocol Builders (returns JsonValue)

Build AFD protocol structures. Return JSON-serializable objects for API responses.

```typescript
type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };

// Startup (configuration)
buildJsonStartup(config: JsonValue, args: JsonValue, env: JsonValue): JsonValue

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
import { buildJsonStartup, buildJsonOk, buildJsonError, buildJson } from "agent-first-data";

// Startup
const startup = buildJsonStartup(
  { api_key_secret: "sk-123", timeout_s: 30 },
  { config_path: "config.yml" },
  { RUST_LOG: "info" },
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
import { buildJsonStartup, buildJsonOk, buildJson, outputYaml, outputPlain } from "agent-first-data";

// 1. Startup
const startup = buildJsonStartup(
  { api_key_secret: "sk-sensitive-key", timeout_s: 30 },
  { input_path: "data.json" },
  { RUST_LOG: "info" },
);
console.log(outputYaml(startup));
// ---
// code: "startup"
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

## License

MIT
