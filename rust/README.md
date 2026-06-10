# Agent-First Data for Rust

```bash
cargo add agent-first-data
# for tracing integration:
cargo add agent-first-data --features tracing
```

```rust
use agent_first_data::{output_json, output_plain};
use serde_json::json;

fn main() {
    let value = json!({
        "code": "ok",
        "result": {
            "api_key_secret": "sk-123",
            "latency_ms": 1280,
            "db_url": "postgres://user:p@ss@db/app?token_secret=abc"
        }
    });

    println!("{}", output_json(&value));
    println!("{}", output_plain(&value));
}
```

Useful names use Rust casing: `output_json`, `output_yaml`, `output_plain`, `output_json_with_options`, `redacted_value`, `redact_secrets_in_place`, `redact_url_secrets`, `parse_size`, `normalize_utc_offset`, `cli_parse_output`, `cli_output`, and `build_cli_error`.

Tracing integration is behind the `tracing` feature: `afdata_tracing::try_init_json`, `try_init_plain`, and `try_init_yaml` return initialization errors; the older `init_*` helpers remain for fire-and-forget setup. CLI help rendering is behind `cli-help`; skill administration is behind `skill-admin`.

```rust
use agent_first_data::{afdata_tracing, RedactionOptions};
use tracing_subscriber::EnvFilter;

fn init_logging() -> Result<(), tracing_subscriber::util::TryInitError> {
    afdata_tracing::try_init_json_with_options(
        EnvFilter::new("info"),
        RedactionOptions {
            secret_names: vec!["authorization".to_string()],
            ..RedactionOptions::default()
        },
    )
}
```

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- YAML/plain quote and escape keys as well as values, sort by UTF-16 code unit order, and render nested objects in arrays as canonical JSON.
- Logging records use `code: "log"` plus a separate `level` field, so error-level logs are not terminal protocol errors.
- Prefer `try_init_*` for Rust tracing startup so failures, such as another global subscriber already being installed, are visible to the caller.
- `build_cli_error(message, hint?)` returns `{code:"error", error: message, hint?}` only.

## Reference

- Full convention and API groups: [../docs/overview.md](../docs/overview.md)
- Formal cross-language contract: [../spec/agent-first-data.md](../spec/agent-first-data.md)
- Conformance fixtures: [../spec/fixtures](../spec/fixtures)
- Agent skill: [../skills/agent-first-data.md](../skills/agent-first-data.md)
