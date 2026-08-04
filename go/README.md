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
    event := afdata.NewJSONResult(map[string]any{
        "api_key_secret": "sk-123",
        "latency_ms": 1280,
        "db_url": "postgres://user:p@ss@db/app?token_secret=abc",
    }).Build()
    value := event.Value()

    options := afdata.OutputOptions{}
    fmt.Println(afdata.Render(value, afdata.OutputFormatJson, options))
    fmt.Println(afdata.Render(value, afdata.OutputFormatPlain, options))
}
```

Useful names use Go casing: `Render` (the single `value × format × options → string` entry point), `OutputFormat`, `OutputTo`, `OutputOptions`, `OutputOptionsForPolicy`, `RedactedValue`, `RedactURLSecrets`, `RedactURLsInText`, `RedactArgv`, `NormalizeUTCOffset`, `IsValidRFC3339Date`, `IsValidRFC3339Time`, `IsValidRFC3339`, `IsValidBCP47`, `CliParseOutput`, `CliParseLogFilters`, `ParseOutputTo`, `CliEmitter`, `BuildCLIError`, `BuildCliVersion`, `CliRenderVersion`, `ValidateProtocolEvent`, and `DecodeProtocolEvent`.

Scoped redaction and extra secret names use the `Redactor` struct:

```go
r := afdata.Redactor{
    SecretNames: []string{"authorization"},
    URLNames: []string{"url", "relays"},
}
fmt.Println(afdata.Render(r.Value(value), afdata.OutputFormatJson, afdata.OutputOptions{}))
fmt.Println(r.URL("https://api.example.com/?authorization=abc"))
fmt.Println(r.URLsInText("see https://api.example.com/?authorization=abc"))
```

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- `Redactor.URLNames` applies URL treatment to exact legacy field names and
  recurses through collections. `RedactURLsInText` is explicit and scans only
  complete scheme URLs, never arbitrary prose secrets.
- YAML keeps original keys and values (structure-preserving, like JSON), sorting keys by UTF-16 code unit order and quoting/escaping unsafe keys and string scalars. Plain strips formatting suffixes, formats values, sorts the same way, and renders nested objects/arrays as canonical JSON.
- Logging records use `kind:"log"` with a nested `log` payload and a separate `level` field, so error-level logs are not terminal protocol errors.
- `build_cli_error(message, hint?)` returns a strict-ready CLI error with `error.retryable:false` and `trace:{}`.
- Do not add a raw version/help pre-parser. Until the Go `CliSpec` compiler lands, applications should keep lifecycle parsing in their own parser rather than claim AFDATA closed-world help compatibility.

## Reference

- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data/SKILL.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data/SKILL.md)
