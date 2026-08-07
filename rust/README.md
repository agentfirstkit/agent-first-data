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
`redact_urls_in_text`, `redact_argv`, `normalize_utc_offset`, `is_valid_rfc3339_date`,
`is_valid_rfc3339_time`, `is_valid_rfc3339`, `is_valid_bcp47`, `CliSpec`,
`CommandSpec`, `ArgSpec`, `Combination`, `OutputSpec`, `SourceSet`,
`SourceScheme`, `ValueSource`, `ErrorSpec`, `lint_value`,
`build_afdata_cli`, and `decode_protocol_event`.

The general-purpose features below are on by default; `--no-default-features`
opts out. Tracing integration is behind `tracing`:
`afdata_tracing::AfdataLayer` is composable and accepts an injected writer,
while `try_init` is the global-subscriber convenience entry point. The whole
CLI surface — `CliSpec`, help-v2, emitters, and the AFDATA adapter — is behind
`cli`; skill administration is behind `skill-admin`; stdout/stderr file
redirection is behind `stream-redirect`. Unix hardened document opens are behind
`libc`, which is on by default and is also enabled by `stream-redirect`.
`render` and `OutputFormat` stay available with every feature off.

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

For a closed-world CLI, bind exact action handlers and resolve argv to
`CliOutcome`. The application owns the process lifecycle because a finite
command, a long-running event stream, and raw-byte output need different
execution and output policies. Use the resolved `OutputPlan` with
`CliEmitter::finish`, `write_raw`, and, when enabled, `stream_redirect`.
[`rust/examples/agent_cli.rs`](examples/agent_cli.rs) shows the finite-command
case.

An `ArgSpec` can declare indirect values with
`sources(SourceSet::config())` (`env:NAME`, `file:PATH#DOT_PATH`, or
`file+FORMAT:PATH#DOT_PATH`) or `SourceSet::stream()` (also `stdin`, `fd:N`,
and `prompt`). Parse the resolved string with that same set, then call
`ValueSource::read()` for an ordinary value or `read_secret()` for a
credential. The latter returns
`agent_first_data::value_source::SecretString`; its `Debug` and `Display`
render `***`, and only `expose_secret()` reveals it at the boundary that needs
the credential. Host-only schemes are declared with `host_scheme` and read by
the host. Empty strings remain values, reads are capped, and an `fd:N` read
does not close the caller-owned descriptor.

Canonical flags are `--stdout-file` and `--stderr-file`. They redirect the corresponding stream to an append-only file; stdout keeps the selected AFDATA output format, and stderr keeps native diagnostics such as panics and backtraces.

## Behavior Notes

- Default redaction replaces every `_secret` or configured secret-name subtree with `***`, including objects and arrays.
- `_url` fields scrub userinfo passwords and secret-named query parameters; surrounding whitespace is trimmed and internal whitespace redacts the whole field.
- `Redactor::url_names` applies that URL treatment to exact legacy field names
  and recurses through collections. `redact_urls_in_text` is the explicit
  scheme-URL-only prose helper; ordinary structured redaction never scans prose.
- YAML/plain quote and escape keys as well as values, sort by UTF-16 code unit order, and render nested objects in arrays as canonical JSON.
- Logging records use `kind:"log"` with a nested `log` payload and a separate `level` field, so error-level logs are not terminal protocol errors.
- Use `AfdataLayer::new(...).with_writer(...)` when composing tracing or
  retaining a `StructuredLogHandle`; use `try_init` when a global subscriber is
  the desired convenience. Tracing `TRACE` maps to AFDATA `debug`; adapter-owned
  `level`, `message`, and `timestamp_epoch_ms` cannot be overridden by span or
  event fields.
- Build new CLIs from one `CliSpec`. Register each command-local argument and
  every legal `Combination`, call `build()`, bind exact `action_id` handlers,
  and resolve to `CliOutcome`. Parsing, typed values, output planning, and
  help-v2 come from the registry; the application retains its command lifetime,
  redaction options, emission strategy, and exit policy.
- `ResolvedInvocation::required` returns `Option`, and
  `BoundCliSpec::execute` returns `None` for an invocation produced by another
  independently built registry. Callers must handle both programming errors
  without fabricating a value or dispatching the wrong handler.
- `CliError` carries a closed rule plus caller-safe `message` and `hint` —
  never raw argument values. `cli_error_event` renders it as a strict protocol
  event; `exit_code()` is the process status.
- `synthetic_invocations()` generates type-correct argv for every combination
  and every fixed `one_of` member, so the same registered shapes can be
  round-trip tested independently of help placeholders.
- Use `document::DocumentFile::open_capped` for bounded untrusted reads,
  `create_atomic` for a parsed first commit, and `edit_and_validate` for a
  typed transactional edit. TOML collections are source-preserving except
  arrays of tables, which require an explicit identity and are refused.
- Feature `libc` gives `open_capped` nonblocking special-file rejection and
  atomic `SymlinkPolicy::NoFollow` on unix. Without it, `NoFollow` is
  unsupported and opening a special file may block before its type is checked.
- `ErrorSpec` / `ErrorCatalog` keep stable public errors separate from runtime
  diagnostics. `lint_value`, strict-event assertions, and redaction-canary
  assertions test final serialized boundaries in-process.
- `stream-redirect` enables `libc` and provides Unix fd-level redirection where supported. It is stream destination control, not a second AFDATA protocol stream, and it does not implement rotation.

## Reference

- Formal cross-language contract: [spec/agent-first-data.md](https://github.com/agentfirstkit/agent-first-data/blob/main/spec/agent-first-data.md)
- Conformance fixtures: [spec/fixtures](https://github.com/agentfirstkit/agent-first-data/tree/main/spec/fixtures)
- Agent skill: [skills/agent-first-data/SKILL.md](https://github.com/agentfirstkit/agent-first-data/blob/main/skills/agent-first-data/SKILL.md)
