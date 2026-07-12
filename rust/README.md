# Agent-First Data for Rust

```bash
cargo add agent-first-data
# for tracing integration:
cargo add agent-first-data --features tracing
```

```rust
use agent_first_data::{json_result, output_json, output_plain};
use serde_json::json;

fn main() {
    let value = json_result(json!({
        "api_key_secret": "sk-123",
        "latency_ms": 1280,
        "db_url": "postgres://user:p@ss@db/app?token_secret=abc"
    }))
    .build()
    .expect("valid afdata event");

    println!("{}", output_json(&value));
    println!("{}", output_plain(&value));
}
```

Useful names use Rust casing: `output_json`, `output_yaml`, `output_plain`, `output_json_with_options`, `redacted_value`, `redact_secrets_in_place`, `redact_url_secrets`, `parse_size`, `normalize_utc_offset`, `is_valid_rfc3339_date`, `is_valid_rfc3339_time`, `cli_parse_output`, `cli_output`, `build_cli_error`, `build_cli_version`, `cli_handle_version_or_continue`, and `decode_protocol_event`.

Tracing integration is behind the `tracing` feature: `afdata_tracing::try_init_json`, `try_init_plain`, and `try_init_yaml` return initialization errors; the older `init_*` helpers remain for fire-and-forget setup. CLI help rendering is behind `cli-help`; skill administration is behind `skill-admin`; stdout/stderr file redirection is behind `stream-redirect`.

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

Stream redirection is opt-in and should be installed before version/help handling, tracing/logging setup, and other early output:

```rust
#[cfg(feature = "stream-redirect")]
let _stream_redirect = agent_first_data::stream_redirect::install_from_raw_args(std::env::args())?;
```

Canonical flags are `--stdout-file` and `--stderr-file`. They redirect the corresponding stream to an append-only file; stdout keeps the selected AFDATA output format, and stderr keeps native diagnostics such as panics and backtraces.

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- YAML/plain quote and escape keys as well as values, sort by UTF-16 code unit order, and render nested objects in arrays as canonical JSON.
- Logging records use `kind:"log"` with a nested `log` payload and a separate `level` field, so error-level logs are not terminal protocol errors.
- Prefer `try_init_*` for Rust tracing startup so failures, such as another global subscriber already being installed, are visible to the caller.
- `build_cli_error(message, hint?)` returns a strict-ready CLI error with `error.retryable:false` and `trace:{}`.
- Use `cli_handle_version_or_continue()` before clap parsing so `--version --output json|yaml|plain` stays structured; use `VersionConfig::conventional_default()` so bare `--version` stays human text while explicit `--output` remains structured.
- `stream-redirect` is Unix fd-level redirection where supported. It is stream destination control, not a second AFDATA protocol stream, and it does not implement rotation.

## Reference

- Full convention and API groups: [docs/overview.md](https://github.com/agentfirstkit/agent-first-data/blob/main/docs/overview.md)
- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data/SKILL.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data/SKILL.md)
