# Agent-First Data

Two independent conventions that work together:

1. **Naming** — encode units and semantics in field names so agents parse structured data without external schemas
2. **Output** — a JSONL protocol with lifecycle phases and multi-format rendering

You can adopt either one alone. Using both together gives agents a complete, self-describing data interface.

`latency_ms: 142` → agents know milliseconds. `api_key_secret: "sk-..."` → agents know to redact. No lookup required.

## Install

```bash
# Rust
cargo add agent-first-data

# Python
pip install agent-first-data

# TypeScript
npm install agent-first-data

# Go
go get github.com/cmnspore/agent-first-data/go
```

## Quick start

**Rust**
```rust
use agent_first_data::{to_plain, redact_secrets, ok};

let result = ok(serde_json::json!({"balance_msats": 97900}));
println!("{}", to_plain(&result));
// code: ok
// result:
//   balance_msats: 97900msats
```

**Python**
```python
from agent_first_data import to_plain, redact_secrets, ok

result = ok({"balance_msats": 97900})
print(to_plain(result))
```

**TypeScript**
```typescript
import { toPlain, redactSecrets, ok } from "agent-first-data";

const result = ok({ balance_msats: 97900 });
console.log(toPlain(result));
```

**Go**
```go
import afd "github.com/cmnspore/agent-first-data/go"

result := afd.Ok(map[string]any{"balance_msats": 97900})
fmt.Println(afd.ToPlain(result))
```

## Part 1: Naming Convention

The field name is the schema. Encode units and semantics in the name itself:

| Suffix | Meaning | Plain output |
|:-------|:--------|:-------------|
| `_ms` | milliseconds | `42ms` or `1.28s` |
| `_s`, `_ns`, `_us` | seconds / nano / micro | `3600s` |
| `_epoch_ms` | unix timestamp ms | `2026-01-31T16:00:00.000Z` |
| `_bytes` | size in bytes | `446.1KB` |
| `_msats`, `_sats` | bitcoin units | `2056msats` |
| `_percent` | percentage | `85%` |
| `_usd_cents` | US dollar cents | `$9.99` |
| `_secret` | sensitive, redact | `***` |

Full suffix list: [spec/agent-first-data.md](spec/agent-first-data.md)

## Part 2: Output Protocol

A JSONL protocol where every line carries a `code` field:

```json
{"code": "startup", "config": {...}, "args": {...}, "env": {...}}
{"code": "progress", "current": 3, "total": 10}
{"code": "ok", "result": {...}, "trace": {"duration_ms": 12}}
{"code": "error", "error": "not found", "trace": {"duration_ms": 3}}
```

Three output formats via `--output json|yaml|plain`:

- **JSON** — canonical, lossless, JSONL-compatible
- **YAML** — quoted strings, `---` separated, values as-is
- **Plain** — suffix-driven human formatting (lossy)

Same structure works across CLI stdout, REST API, MCP tools, and SSE streams.

## Skill

`skills/agent-first-data.md` is a [Claude Code skill](https://docs.anthropic.com/en/docs/claude-code) that teaches agents to apply AFD conventions when writing structured data, configs, logs, API responses, or CLI output. Copy it into your project's `.claude/skills/` directory to enable it.

## License

MIT
