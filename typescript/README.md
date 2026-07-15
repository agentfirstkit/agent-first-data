# Agent-First Data for TypeScript

```bash
npm install agent-first-data
```

```typescript
import { jsonResult, outputJson, outputPlain } from "agent-first-data";

const event = jsonResult({
  api_key_secret: "sk-123",
  latency_ms: 1280,
  db_url: "postgres://user:p@ss@db/app?token_secret=abc",
}).build();

console.log(outputJson(event.toJSON()));
console.log(outputPlain(event.toJSON()));
```

Useful names use TypeScript casing: `outputJson`, `outputYaml`, `outputPlain`, `outputOptionsForPolicy`, `redactedValue`, `redactUrlSecrets`, `normalizeUtcOffset`, `isValidRfc3339Date`, `isValidRfc3339Time`, `decodeProtocolEvent`, `cliParseOutput`, `cliParseLogFilters`, `cliOutput`, `buildCliError`, `buildCliVersion`, and `cliHandleVersionOrContinue`.

Skill admin and stream redirection are not re-exported from the package root; import them from their own subpaths:

```typescript
import { runSkillAdmin } from "agent-first-data/skill";
import { installStreamRedirectFromRawArgs } from "agent-first-data/stream-redirect";
```

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- YAML/plain quote and escape keys as well as values, sort by UTF-16 code unit order, and render nested objects in arrays as canonical JSON.
- Logging records use `kind:"log"` with a nested `log` payload and a separate `level` field, so error-level logs are not terminal protocol errors.
- `buildCliError(message, hint?)` returns a strict-ready CLI error with `error.retryable:false` and `trace:{}`.
- Use `cliHandleVersionOrContinue()` before argument parsing so bare `--version` stays conventional and `--version --output json|yaml|plain` stays structured.
- Use `installStreamRedirectFromRawArgs()` (from `agent-first-data/stream-redirect`) before version/help handling if a CLI exposes `--stdout-file` or `--stderr-file`; stderr is redirected as native diagnostics, not JSON. Node fd-level redirection is process-lifetime because the standard library does not expose `dup2` restoration.

## Reference

- Full convention and API groups: [docs/overview.md](https://github.com/agentfirstkit/agent-first-data/blob/main/docs/overview.md)
- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data/SKILL.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data/SKILL.md)
