---
name: agent-first-data
description: Apply Agent-First Data naming, redaction, formatting, protocol envelope, logging, and CLI lint/validate workflows when writing or reviewing structured data, configs, logs, transport payloads, database/wire fields, or CLI output in any language. Use proactively before adding, renaming, serializing, or changing public API, wire, database, or persistent field names that may need AFDATA suffixes.
---

<!-- Canonical source: skills/agent-first-data/SKILL.md.
     Mirrored byte-for-byte into go/python/typescript assets/ by scripts/sync_offline_assets.py.
     Edit only here; run that script (checked by scripts/test.sh) to propagate. -->

# Agent-First Data

Use AFDATA when naming fields or reviewing structured output. The contract is naming-based: suffixes communicate units, formatting, and redaction semantics without relying on free-text inference.

## Read references as needed

- `references/rules.md` — detailed suffix rules, output formatting, protocol templates, logging, CLI flags, and review checklist.
- `references/registry.json` — machine-readable suffix registry; use this as the exact source for suffix metadata.
- `references/protocol-v1.schema.json` — protocol event schema for `kind:"result"`, `kind:"error"`, `kind:"progress"`, and `kind:"log"` envelopes.

## Naming and redaction decisions

- Add unit suffixes to ambiguous numeric fields: `_ms`, `_s`, `_bytes`, `_percent`, `_sats`, `_msats`, currency suffixes, and timestamp suffixes.
- Use `_secret` for values/subtrees that must redact to `***` in every output format.
- Use `_url` only for whole URL values whose userinfo password or suffix-named secret query params should be scrubbed.
- Do not scan arbitrary free text for secrets. Rename fields or configure explicit secret names at the serialization boundary.
- Before changing public API, wire, database, or persistent field names, report the compatibility impact and get approval.

## Evaluation rules

- Trigger this skill for field naming, structured output, configs, logs, CLI output, protocol events, MCP/HTTP/SSE payloads, database columns, wire fields, or persisted JSON. Do not wait for the user to say "AFDATA" when the work changes those surfaces.
- Treat `references/registry.json` and `references/protocol-v1.schema.json` as authoritative. Do not replace them with invented suffix meanings, a reduced schema, or free-text interpretation.
- For secrets, fix the field name or serializer configuration. Use `_secret` or configured exact secret names; do not rely on scanning arbitrary strings or object debug output.
- Run `afdata lint` on JSON/JSONL examples, JSON Schema, MCP schemas, or serialized samples when the CLI is available. Run `afdata validate` on finite protocol events/streams. If a check cannot run, report that explicitly.
- Before modifying an external project's public API, wire format, database schema, or persisted field names, stop and report the compatibility impact; request approval before applying the change.

## Redaction behavior contract

Fields named `_secret` or `_url` and values passing through AFDATA output formatters (`output_json`, `output_yaml`, `output_plain`) are automatically redacted: secrets become `***`, URL userinfo passwords and suffix-named query parameters are scrubbed. When serializing outside output formatters (HTTP, MCP, SSE), call `redacted_value()` or `redact_url_secrets()` at the serialization boundary. PII redaction, domain-specific privacy policies (header allowlists, API scope sensitivity), and per-field secret naming are each spore's responsibility; the library provides field-name-based mechanics only.

## Logging behavior contract

One-time CLI output uses `json_log()` or `CliEmitter` for a single event; serialization via `cli_output()` applies output formatting. Long-running services or processes depending on structured logging (tonic, sqlx, hyper, etc. via tracing) initialize with `afdata_tracing::try_init()` (Rust only) to wire process-wide logging; other languages emit log events explicitly via builders or integrate their own structured logging.

## CLI workflow

When the project has the `afdata` CLI available, use it for deterministic checks:

```bash
afdata lint <file-or-stdin>
afdata validate <file-or-stdin>
```

- Run `afdata lint` for ordinary JSON/JSONL, JSON Schema, or MCP input/output schemas.
- Run `afdata validate` for AFDATA protocol events or finite event streams.
- Treat findings as contract issues unless the owning tool intentionally documents a non-AFDATA field.

## Review checklist

1. Numeric fields with units carry the right suffix and safe integer/string representation.
2. Timestamps use `_epoch_s`, `_epoch_ms`, decimal-string `_epoch_ns`, or `_rfc3339`.
3. Secrets use `_secret` or configured exact secret names; no fallback path leaks them.
4. CLI output uses AFDATA helpers for JSON/YAML/plain formatting and structured errors.
5. Logs use AFDATA structured logging or equivalent field-name redaction.
6. Transport payloads preserve the protocol envelope when AFDATA compliance is claimed.
