# Agent-First Data for Go

```bash
go get github.com/agentfirstkit/agent-first-data/go
```

```go
package main

import (
    "fmt"

    afdata "github.com/agentfirstkit/agent-first-data/go"
)

func main() {
    value := map[string]any{
        "code": "ok",
        "result": map[string]any{
            "api_key_secret": "sk-123",
            "latency_ms": 1280,
            "db_url": "postgres://user:p@ss@db/app?token_secret=abc",
        },
    }

    fmt.Println(afdata.OutputJson(value))
    fmt.Println(afdata.OutputPlain(value))
}
```

Useful names use Go casing: `OutputJson`, `OutputYaml`, `OutputPlain`, `OutputJsonWithOptions`, `RedactedValue`, `RedactSecretsInPlace`, `RedactURLSecrets`, `ParseSize`, `NormalizeUTCOffset`, `IsValidRFC3339Date`, `IsValidRFC3339Time`, `CliParseOutput`, `CliOutput`, `BuildCliError`, `BuildCliVersion`, and `CliHandleVersionOrContinue`.

Logging is available through the `log/slog` integration: `InitJson`, `InitPlain`, `InitYaml`, `InitJsonWithOptions`, `WithSpan`, and `LoggerFromContext`.

```go
afdata.InitJsonWithOptions(afdata.RedactionOptions{
    SecretNames: []string{"authorization"},
})
```

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- YAML/plain quote and escape keys as well as values, sort by UTF-16 code unit order, and render nested objects in arrays as canonical JSON.
- Logging records use `code: "log"` plus a separate `level` field, so error-level logs are not terminal protocol errors.
- `build_cli_error(message, hint?)` returns `{code:"error", error: message, hint?}` only.
- Use `CliHandleVersionOrContinue()` before argument parsing so bare `--version` stays conventional and `--version --output json|yaml|plain` stays structured.

## Reference

- Full convention and API groups: [docs/overview.md](https://github.com/agentfirstkit/agent-first-data/blob/main/docs/overview.md)
- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data.md)
