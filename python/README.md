# Agent-First Data for Python

```bash
pip install agent-first-data
```

```python
from agent_first_data import output_json, output_plain

value = {
    "code": "ok",
    "result": {
        "api_key_secret": "sk-123",
        "latency_ms": 1280,
        "db_url": "postgres://user:p@ss@db/app?token_secret=abc",
    },
}

print(output_json(value))
print(output_plain(value))
```

Useful names use Python casing: `output_json`, `output_yaml`, `output_plain`, `output_json_with_options`, `redacted_value`, `redact_secrets_in_place`, `redact_url_secrets`, `parse_size`, `normalize_utc_offset`, `is_valid_rfc3339_date`, `is_valid_rfc3339_time`, `cli_parse_output`, `cli_output`, `build_cli_error`, `build_cli_version`, and `cli_handle_version_or_continue`.

Logging is available through `init_logging_json`, `init_logging_plain`, `init_logging_yaml`, `span`, and `get_logger`.

```python
from agent_first_data import init_logging_json

init_logging_json("INFO", secret_names=("authorization",))
```

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- YAML/plain quote and escape keys as well as values, sort by UTF-16 code unit order, and render nested objects in arrays as canonical JSON.
- Logging records use `code: "log"` plus a separate `level` field, so error-level logs are not terminal protocol errors.
- `build_cli_error(message, hint?)` returns `{code:"error", error: message, hint?}` only.
- Use `cli_handle_version_or_continue()` before argument parsing so bare `--version` stays conventional and `--version --output json|yaml|plain` stays structured.

## Reference

- Full convention and API groups: [docs/overview.md](https://github.com/agentfirstkit/agent-first-data/blob/main/docs/overview.md)
- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data.md)
