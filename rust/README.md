# Agent-First Data for Rust

```bash
cargo add agent-first-data
# data/protocol surface only — no CLI compiler, emitter, tracing, or skill admin:
cargo add agent-first-data --no-default-features
```

```rust
use agent_first_data::{OutputFormat, OutputOptions, json_result, render};
use serde_json::json;

fn main() {
    let event = json_result(json!({
        "api_key_secret": "sk-123",
        "latency_ms": 1280,
        "db_url": "postgres://user:p@ss@db/app?token_secret=abc"
    }))
    .build();

    let options = OutputOptions::default();
    println!("{}", render(event.as_value(), OutputFormat::Json, &options));
    println!("{}", render(event.as_value(), OutputFormat::Plain, &options));
}
```

Useful names use Rust casing: `render` (the single
`value × format × options → String` entry point), `OutputFormat`,
`OutputOptions`, `Redactor`, `redacted_value`, `redact_url_secrets`,
`redact_argv`, `normalize_utc_offset`, `is_valid_rfc3339_date`,
`is_valid_rfc3339_time`, `is_valid_rfc3339`, `is_valid_bcp47`, `CliSpec`,
`CommandSpec`, `ArgSpec`, `Combination`, `OutputSpec`, `CliOutcome`,
`build_afdata_cli`, and `decode_protocol_event`.

Every feature below is on by default; `--no-default-features` opts out. Tracing integration is behind `tracing`: `afdata_tracing::try_init` is the single entry point and returns an error if a global subscriber is already installed. The whole CLI surface — `CliSpec`, help-v2 and version resolution, the `CliEmitter`, and the AFDATA adapter — is behind `cli`; skill administration is behind `skill-admin`; stdout/stderr file redirection is behind `stream-redirect`. `render` and `OutputFormat` stay available with every feature off.

```rust
use agent_first_data::afdata_tracing::{self, LogFormat};
use agent_first_data::Redactor;
use tracing_subscriber::EnvFilter;

fn init_logging() -> Result<(), tracing_subscriber::util::TryInitError> {
    afdata_tracing::try_init(
        EnvFilter::new("info"),
        LogFormat::Json,
        Redactor::new().secret_names(["authorization"]),
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
- Prefer `afdata_tracing::try_init` for Rust tracing startup so failures, such as another global subscriber already being installed, are visible to the caller.
- Build new CLIs from one `CliSpec`. Register each command-local argument and
  every legal `Combination`, call `build()`, bind exact `action_id` handlers,
  then resolve to `CliOutcome`. Parsing, typed values, output planning, and
  help-v2 all come from that registry.
- `CliError` carries a closed rule, the selected command path, and normalized
  argument names — never raw values. `cli_error_event` renders it as a strict
  protocol event; `exit_code()` is the process status.
- `synthetic_invocations()` generates type-correct argv for every combination
  and every fixed `one_of` member, so the same registered shapes can be
  round-trip tested independently of help placeholders.
- `stream-redirect` is Unix fd-level redirection where supported. It is stream destination control, not a second AFDATA protocol stream, and it does not implement rotation.

## Reference

- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data/SKILL.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data/SKILL.md)
