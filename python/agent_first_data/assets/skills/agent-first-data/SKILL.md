---
name: agent-first-data
description: Apply and review the formal Agent-First Data specification using the Rust/Go/Python/TypeScript libraries or afdata CLI for naming, redaction, formatting, protocol envelopes, logging, linting, and validation. Use proactively when writing or reviewing structured data, configs, logs, transport payloads, database/wire fields, CLI output, or compatibility-sensitive public and persistent field names.
---

<!-- Canonical source: skills/agent-first-data/SKILL.md.
     Mirrored byte-for-byte into go/python/typescript assets/ by scripts/sync_offline_assets.py.
     Edit only here; run that script (checked by scripts/test.sh) to propagate. -->

# Agent-First Data

Use AFDATA when naming fields or reviewing structured output. The contract is naming-based: suffixes communicate units, formatting, and redaction semantics without relying on free-text inference.

## Specification workflow

Treat the formal specification as normative, not the skill summary or an
example payload. In a repository checkout, read these spore-root-relative
sources as needed:

- `spec/agent-first-data.md` — complete cross-language naming, formatting,
  redaction, protocol, logging, and CLI contract.
- `spec/registry.json` — exact suffix metadata.
- `spec/protocol-v1.schema.json` — protocol event schema.
- `spec/fixtures/` — cross-language conformance cases.

When only the installed skill is available, use the bundled equivalents:

- `references/rules.md` — detailed suffix rules, output formatting, protocol templates, logging, CLI flags, and review checklist.
- `references/registry.json` — machine-readable suffix registry; use this as the exact source for suffix metadata.
- `references/protocol-v1.schema.json` — protocol event schema for `kind:"result"`, `kind:"error"`, `kind:"progress"`, and `kind:"log"` envelopes.

If an example or README conflicts with the formal spec, follow the spec and
report the discrepancy. Do not invent suffix meanings or reduce the protocol
schema.

## Library workflow

Use the library at serialization boundaries so redaction and formatting are
not reimplemented ad hoc. Rust applications can build a typed protocol event
and render it in the required output format:

```bash
cargo add agent-first-data --no-default-features
pip install agent-first-data
npm install agent-first-data
go get github.com/agentfirstkit/agent-first-data/go
```

```rust
use agent_first_data::{json_result, output_json};
use serde_json::json;

let event = json_result(json!({
    "latency_ms": 1280,
    "api_key_secret": "sk-live-example",
})).build()?;
let rendered = output_json(event.as_value());
assert!(rendered.contains("\"api_key_secret\":\"***\""));
# Ok::<(), agent_first_data::BuildError>(())
```

For a finite CLI execution implemented in Rust, create one `CliEmitter`, select
`OutputFormat` through `cli_parse_output`, enable strict protocol, and emit one
terminal result or error. Handle version before clap with
`cli_handle_version_or_continue` and `VersionConfig::conventional_default()`.
For non-CLI HTTP/MCP/SSE serialization, call `redacted_value()` or
`redact_url_secrets()` explicitly at the boundary.

The same library contract is available for Go, Python, and TypeScript. Use the
runtime's native names while preserving the same builders, output formats,
redaction behavior, registry, schema, and conformance fixtures; consult its
root-level `go/README.md`, `python/README.md`, or `typescript/README.md` when
working in that runtime.

## Naming and redaction decisions

- Add unit suffixes to ambiguous numeric fields: `_ms`, `_s`, `_bytes`, `_percent`, `_sats`, `_msats`, currency suffixes, and timestamp suffixes.
- Use `_secret` for values/subtrees that must redact to `***` in every output format.
- Use `_url` only for whole URL values whose userinfo password or suffix-named secret query params should be scrubbed.
- Do not scan arbitrary free text for secrets. Rename fields or configure explicit secret names at the serialization boundary.
- Before changing public API, wire, database, or persistent field names, report the compatibility impact and get approval.

## Evaluation rules

- Trigger this skill for field naming, structured output, configs, logs, CLI output, protocol events, MCP/HTTP/SSE payloads, database columns, wire fields, or persisted JSON. Do not wait for the user to say "AFDATA" when the work changes those surfaces.
- In a repository checkout, treat `spec/agent-first-data.md`,
  `spec/registry.json`, and `spec/protocol-v1.schema.json` as authoritative; in
  an installed skill, use their bundled `references/` equivalents. Do not
  replace them with invented suffix meanings, a reduced schema, or free-text
  interpretation.
- For secrets, fix the field name or serializer configuration. Use `_secret` or configured exact secret names; do not rely on scanning arbitrary strings or object debug output.
- Run `afdata lint` on JSON/JSONL examples, JSON Schema, MCP schemas, or serialized samples when the CLI is available. Run `afdata validate` on finite protocol events/streams. If a check cannot run, report that explicitly.
- Before modifying an external project's public API, wire format, database schema, or persisted field names, stop and report the compatibility impact; request approval before applying the change.

## Redaction behavior contract

Fields named `_secret` or `_url` and values passing through AFDATA output formatters (`output_json`, `output_yaml`, `output_plain`) are automatically redacted: secrets become `***`, URL userinfo passwords and suffix-named query parameters are scrubbed. When serializing outside output formatters (HTTP, MCP, SSE), call `redacted_value()` or `redact_url_secrets()` at the serialization boundary. PII redaction, domain-specific privacy policies (header allowlists, API scope sensitivity), and per-field secret naming are each spore's responsibility; the library provides field-name-based mechanics only.

## Logging behavior contract

One-time CLI output uses `json_log()` or `CliEmitter` for a single event; serialization via `cli_output()` applies output formatting. Long-running services or processes depending on structured logging (tonic, sqlx, hyper, etc. via tracing) initialize with `afdata_tracing::try_init()` (Rust only) to wire process-wide logging; other languages emit log events explicitly via builders or integrate their own structured logging.

## CLI workflow

Install `afdata` when a shell workflow needs validation, formatting, or skill
administration without embedding a library:

```bash
cargo install agent-first-data
```

Use it for deterministic checks:

```bash
afdata lint payload.json
afdata validate events.jsonl
afdata format payload.json --output yaml
afdata skill validate skills/example-skill
```

- Run `afdata lint` for ordinary JSON/JSONL, JSON Schema, or MCP input/output schemas.
- Run `afdata validate` for AFDATA protocol events or finite event streams.
- Omit the input path to read stdin. `--output` accepts `json`, `yaml`, or
  `plain`.
- Treat findings as contract issues unless the owning tool intentionally documents a non-AFDATA field.

## Review checklist

1. Numeric fields with units carry the right suffix and safe integer/string representation.
2. Timestamps use `_epoch_s`, `_epoch_ms`, decimal-string `_epoch_ns`, or `_rfc3339`.
3. Secrets use `_secret` or configured exact secret names; no fallback path leaks them.
4. CLI output uses AFDATA helpers for JSON/YAML/plain formatting and structured errors.
5. Logs use AFDATA structured logging or equivalent field-name redaction.
6. Transport payloads preserve the protocol envelope when AFDATA compliance is claimed.
