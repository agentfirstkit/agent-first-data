# Agent-First Data for TypeScript

```bash
npm install agent-first-data
```

```typescript
import { jsonResult, render } from "agent-first-data";

const event = jsonResult({
  api_key_secret: "sk-123",
  latency_ms: 1280,
  db_url: "postgres://user:p@ss@db/app?token_secret=abc",
}).build();

console.log(render(event.toJSON(), "json"));
console.log(render(event.toJSON(), "plain"));
```

Useful names use TypeScript casing: `render`, `outputOptionsForPolicy`, `redactedValue`, `redactUrlSecrets`, `redactUrlsInText`, `redactArgv`, `normalizeUtcOffset`, `isValidRfc3339Date`, `isValidRfc3339Time`, `isValidRfc3339`, `isValidBcp47`, `decodeProtocolEvent`, `cliParseOutput`, `cliParseLogFilters`, `buildCliError`, and `buildCliVersion`.

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- `OutputOptions.redaction.urlNames` applies URL treatment to exact legacy field names
  and recurses through collections. `redactUrlsInText` is explicit and scans
  only complete scheme URLs, never arbitrary prose secrets.
- YAML keeps original keys and values (structure-preserving, like JSON), sorting keys by UTF-16 code unit order and quoting/escaping unsafe keys and string scalars. Plain strips formatting suffixes, formats values, sorts the same way, and renders nested objects/arrays as canonical JSON.
- Logging records use `kind:"log"` with a nested `log` payload and a separate `level` field, so error-level logs are not terminal protocol errors.
- `buildCliError(message, hint?)` returns a strict-ready CLI error with `error.retryable:false` and `trace:{}`.
- Do not add a raw version/help pre-parser. Until the TypeScript `CliSpec` compiler lands, applications should keep lifecycle parsing in their own parser rather than claim AFDATA closed-world help compatibility.

## Reference

- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data/SKILL.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data/SKILL.md)
