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
    event, _ := afdata.NewJSONResult(map[string]any{
        "api_key_secret": "sk-123",
        "latency_ms": 1280,
        "db_url": "postgres://user:p@ss@db/app?token_secret=abc",
    }).Build()
    value := event.Value()

    fmt.Println(afdata.OutputJson(value))
    fmt.Println(afdata.OutputPlain(value))
}
```

Useful names use Go casing: `OutputJson`, `OutputYaml`, `OutputPlain`, `OutputJsonWithOptions`, `OutputOptionsForPolicy`, `RedactedValue`, `RedactURLSecrets`, `NormalizeUTCOffset`, `IsValidRFC3339Date`, `IsValidRFC3339Time`, `IsValidRFC3339`, `IsValidBCP47`, `CliParseOutput`, `CliOutput`, `BuildCliError`, `BuildCliVersion`, `CliHandleVersionOrContinue`, and `DecodeProtocolEvent`.

Skill admin (`RunSkillAdmin`, `SkillSpec`, ...) lives in the `github.com/agentfirstkit/agent-first-data/go/skill` subpackage; stdout/stderr file redirection (`ParseStreamRedirectArgs`, `InstallStreamRedirectFromArgs`, ...) lives in `github.com/agentfirstkit/agent-first-data/go/streamredirect`.

Scoped redaction and extra secret names use the `Redactor` struct:

```go
r := afdata.Redactor{SecretNames: []string{"authorization"}}
fmt.Println(afdata.OutputJson(r.Value(value)))
fmt.Println(r.URL("https://api.example.com/?authorization=abc"))
```

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- YAML keeps original keys and values (structure-preserving, like JSON), sorting keys by UTF-16 code unit order and quoting/escaping unsafe keys and string scalars. Plain strips formatting suffixes, formats values, sorts the same way, and renders nested objects/arrays as canonical JSON.
- Logging records use `kind:"log"` with a nested `log` payload and a separate `level` field, so error-level logs are not terminal protocol errors.
- `build_cli_error(message, hint?)` returns a strict-ready CLI error with `error.retryable:false` and `trace:{}`.
- Use `CliHandleVersionOrContinue()` before argument parsing so `--version`/`-V` always answers with a structured protocol-v1 `kind:"result"` version event — JSON by default, or `--output yaml|plain`/`--json` for another format; there is no conventional bare-text form. Pass your own value-taking global flag names so their value is not mistaken for the subcommand boundary.
- Use `streamredirect.InstallStreamRedirectFromArgs()` before version/help handling if a CLI exposes `--stdout-file` or `--stderr-file`; stderr is redirected as native diagnostics, not JSON.

## Reference

- Full convention and API groups: [docs/overview.md](https://github.com/agentfirstkit/agent-first-data/blob/main/docs/overview.md)
- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data/SKILL.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data/SKILL.md)
