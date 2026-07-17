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

Useful names use Python casing: `render` (the single value x format x options -> str entry point; takes an `OutputFormat` and an optional keyword-only `options=`), `redacted_value`, `redact_url_secrets`, `normalize_utc_offset`, `is_valid_rfc3339_date`, `is_valid_rfc3339_time`, `is_valid_rfc3339`, `is_valid_bcp47`, `cli_parse_output`, `build_cli_error`, `build_cli_version`, `cli_handle_version_or_continue`, `decode_protocol_event`. Skill admin and stream-redirect helpers live in `agent_first_data.skill` and `agent_first_data.stream_redirect` (import the submodule directly).

Logging is available through `init_logging_json`, `init_logging_plain`, `init_logging_yaml`, `span`, and `get_logger`.

```python
from agent_first_data import init_logging_json

init_logging_json("INFO", secret_names=("authorization",))
```

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- YAML keeps original keys and values (structure-preserving, like JSON), sorting keys by UTF-16 code unit order and quoting/escaping unsafe keys and string scalars. Plain strips formatting suffixes, formats values, sorts the same way, and renders nested objects/arrays as canonical JSON.
- Logging records use `kind:"log"` with a nested `log` payload and a separate `level` field, so error-level logs are not terminal protocol errors.
- `build_cli_error(message, hint?)` returns a strict-ready CLI error with `error.retryable:false` and `trace:{}`.
- Use `cli_handle_version_or_continue()` before argument parsing so bare `--version` stays conventional and `--version --output json|yaml|plain` stays structured.
- Use `agent_first_data.stream_redirect.install_from_raw_args()` before version/help handling if a CLI exposes `--stdout-file` or `--stderr-file`; stderr is redirected as native diagnostics, not JSON.

## Reference

- Full convention and API groups: [docs/overview.md](https://github.com/agentfirstkit/agent-first-data/blob/main/docs/overview.md)
- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data/SKILL.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data/SKILL.md)
