# Agent-First Data for Python

```bash
pip install agent-first-data
```

```python
from agent_first_data import json_result, render, OutputFormat

value = json_result(
    {
        "api_key_secret": "sk-123",
        "latency_ms": 1280,
        "db_url": "postgres://user:p@ss@db/app?token_secret=abc",
    }
).build()

print(render(value, OutputFormat.JSON))
print(render(value, OutputFormat.PLAIN))
```

Useful names use Python casing: `render` (the single value x format x options -> str entry point; takes an `OutputFormat` and an optional keyword-only `options=`), `redacted_value`, `redact_url_secrets`, `redact_urls_in_text`, `redact_argv`, `normalize_utc_offset`, `is_valid_rfc3339_date`, `is_valid_rfc3339_time`, `is_valid_rfc3339`, `is_valid_bcp47`, `cli_parse_output`, `build_cli_error`, `build_cli_version`, `cli_parse_log_filters`, `CliEmitter`, `decode_protocol_event`.

## Behavior Notes

- `OutputOptions(reveal_names=...)` reveals exact structured field names at the selected output boundary. Children still pass through redaction; URL parameters and argv receive no exemption.
- The protocol reader accepts base-profile events. An absent `error.retryable` remains nullable; strict validation is a separate opt-in.
- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed, and internal whitespace or a `://` that is not the start of a clean URL redacts the whole field, where whitespace is the explicit Unicode `White_Space` set the spec enumerates, not the language's native predicate.
- `OutputOptions.url_names` applies URL treatment to exact legacy field names
  and recurses through collections. `redact_urls_in_text` is explicit and scans
  only complete scheme URLs, never arbitrary prose secrets.
- YAML keeps original keys and values (structure-preserving, like JSON), sorting keys by UTF-16 code unit order and quoting/escaping unsafe keys and string scalars. Plain strips formatting suffixes, formats values, sorts the same way, and renders nested objects/arrays as canonical JSON; it quotes a value only when it contains ASCII space, tab, newline, carriage return, form feed, vertical tab, NBSP, `=`, `"`, or `\`, and escapes `\`, `"`, newline, carriage return, tab, form feed, and vertical tab inside the quotes.
- Logging records use `kind:"log"` with a nested `log` payload and a separate `level` field, so error-level logs are not terminal protocol errors.
- `build_cli_error(message, hint?)` returns a strict-ready CLI error with `error.retryable:false` and `trace:{}`.
- Do not add a raw version/help pre-parser. This SDK provides emitters and readers; argv and lifecycle parsing belong to the application. Claim help-v2 or closed-world compatibility only when using a registry compiler.

## Reference

- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data/SKILL.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data/SKILL.md)
