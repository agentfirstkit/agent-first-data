# Agent-First Data for TypeScript

```bash
npm install agent-first-data
```

```typescript
import { outputJson, outputPlain } from "agent-first-data";

const value = {
  code: "ok",
  result: {
    api_key_secret: "sk-123",
    latency_ms: 1280,
    db_url: "postgres://user:p@ss@db/app?token_secret=abc",
  },
};

console.log(outputJson(value));
console.log(outputPlain(value));
```

Useful names use TypeScript casing: `outputJson`, `outputYaml`, `outputPlain`, `outputJsonWithOptions`, `redactedValue`, `redactSecretsInPlace`, `redactUrlSecrets`, `parseSize`, `normalizeUtcOffset`, `isValidRfc3339Date`, `isValidRfc3339Time`, `cliParseOutput`, `cliOutput`, `buildCliError`, `buildCliVersion`, and `cliHandleVersionOrContinue`.

Logging is available through `initJson`, `initPlain`, `initYaml`, `span`, and `log`.

```typescript
import { initJson } from "agent-first-data";

initJson({ secretNames: ["authorization"] });
```

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- YAML/plain quote and escape keys as well as values, sort by UTF-16 code unit order, and render nested objects in arrays as canonical JSON.
- Logging records use `code: "log"` plus a separate `level` field, so error-level logs are not terminal protocol errors.
- `build_cli_error(message, hint?)` returns `{code:"error", error: message, hint?}` only.
- Use `cliHandleVersionOrContinue()` before argument parsing so bare `--version` stays conventional and `--version --output json|yaml|plain` stays structured.

## Reference

- Full convention and API groups: [../docs/overview.md](../docs/overview.md)
- Formal cross-language contract: [../spec/agent-first-data.md](../spec/agent-first-data.md)
- Conformance fixtures: [../spec/fixtures](../spec/fixtures)
- Agent skill: [../skills/agent-first-data.md](../skills/agent-first-data.md)
