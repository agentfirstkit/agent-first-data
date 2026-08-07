---
name: agent-first-data
description: Apply and review Agent-First Data (AFDATA) for structured field names, unit suffixes, secret and URL redaction, JSON/YAML/plain rendering, protocol events, logs, agent-facing CLI output, help, and value sources, safe dot-path edits to JSON/TOML/YAML/dotenv/INI or Markdown frontmatter, read-only Markdown structure, and AFDATA-style Bash scripts. Use proactively for configs, logs, transport payloads, database or wire fields, public/persistent names, CLI design, and structured-data shell work.
---

<!-- Canonical source: skills/agent-first-data/SKILL.md. -->

# Agent-First Data

Apply AFDATA at structured-data boundaries. Field names carry units, formats,
and sensitivity so an agent can interpret values without extra prose.

## Start with the smallest source

Read only the route needed for the task:

| Task | Read |
|---|---|
| Name or review fields, configs, logs, database columns, wire/API data, redaction, or rendering | [naming-output.md](references/naming-output.md), then use [registry.json](references/registry.json) for exact suffix metadata |
| Build or review protocol events, CLI output, logging, version/help behavior, value-source declarations, or stream routing | [cli-protocol.md](references/cli-protocol.md) and [protocol-v1.schema.json](references/protocol-v1.schema.json) |
| Build or validate a closed-world CLI or structured help | [cli-protocol.md](references/cli-protocol.md), [cli-spec-v1.schema.json](references/cli-spec-v1.schema.json), and [cli-help-v2.schema.json](references/cli-help-v2.schema.json) |
| Read or safely mutate JSON/TOML/YAML/dotenv/INI or Markdown frontmatter, or read a Markdown file's heading sections | [documents.md](references/documents.md) |
| Author an AFDATA-style Bash 3.2+ executable | [bash.md](references/bash.md) |

In a repository checkout, the formal contract is `spec/agent-first-data.md`.
Use it only when changing the contract or resolving an ambiguity; use
`spec/registry.json`, `spec/protocol-v1.schema.json`,
`spec/cli-spec-v1.schema.json`, and `spec/cli-help-v2.schema.json` for exact
machine-readable CLI constraints. The
focused references above are the offline installed-skill equivalents.

If a README or example conflicts with the formal spec or schema, follow the
formal source and report the discrepancy. Do not invent suffix meanings,
protocol fields, or help fields.

## Core decisions

- Add a registered suffix when a number's unit, a string's format, or a value's
  sensitivity is otherwise ambiguous.
- Name secret values or subtrees `_secret`; render them as `***`.
- Treat `_secret` as a structured-output guarantee only. Keep live secrets out
  of argv; use `redact_argv` before recording an invocation.
- Name whole URL values `_url`; scrub userinfo passwords and explicitly
  secret-named query parameters.
- Configure exact `url_names` for legacy URL fields; collection values recurse
  without guessing unrelated field names.
- Do not scan arbitrary prose for secrets. Rename the field or configure an
  exact secret name at the serialization boundary; use the explicit
  `redact_urls_in_text` helper only for complete scheme URLs embedded in prose.
- Keep JSON and YAML schema-preserving. Use plain output only as the lossy,
  human-readable form.
- Register legal invocation combinations; do not infer legality from parser
  permissiveness, output differences, or post-parse ignored flags.
- Declare accepted value sources on the argument. Classify them before domain
  I/O, read credentials through `read_secret()`, and keep host schemes distinct
  from the built-in grammar.
- Use one protocol envelope shape across CLI, HTTP, MCP, SSE, and logs when
  claiming AFDATA protocol compatibility.
- Before renaming a public API, wire, database, or persistent field, explain
  the compatibility impact and obtain approval.

## Implementation workflow

1. Identify the serialization boundary and whether the output is a finite
   result or an ordered event stream.
2. Inspect existing public and persistent names before changing them.
3. Use the runtime library instead of reimplementing suffix formatting,
   redaction, event envelopes, or CLI errors.
4. Redact before serialization. For HTTP/MCP/SSE paths that bypass `render`,
   call `redacted_value`; use `redact_url_secrets` for one URL or
   `redact_urls_in_text` for scheme URLs already embedded in prose.
5. For new CLIs, construct one `CliSpec`, register every executable
   `Combination`, resolve to `CliOutcome`, and dispatch only by `action_id`.
   Parser, combination matching, typed values, output planning, and help must
   all come from that registry.
6. Validate examples and emitted data with the checks below.

Runtime API names follow native casing. In a checkout, consult only the relevant
`rust/README.md`, `go/README.md`, `python/README.md`, or
`typescript/README.md` for imports and signatures.

## CLI checks

Use `afdata` when it is available:

```bash
afdata lint payload.json
afdata validate events.jsonl --strict
afdata render payload.json --output yaml
afdata skill validate skills/example-skill
```

- Run `lint` for JSON/JSONL samples, JSON Schema, MCP schemas, configs, or
  serialized output.
- Run `validate` for protocol events and finite event streams.
- Use `render` to apply AFDATA redaction and formatting.
- If a required check cannot run, state that explicitly.

## Review exit criteria

- Ambiguous units and strict string formats use registered suffixes and valid
  JSON types.
- Secrets redact on every output path, including defaults shown in help.
- JSON/YAML retain original keys and scalar types.
- Protocol events use `kind`, the same-named payload, and top-level `trace`.
- Finite output and event streams use the correct channel policy.
- Every application argument belongs to at least one combination, combinations
  do not overlap, and unregistered mixtures fail before domain I/O.
- Value-source sets are non-empty and unambiguous, malformed or unaccepted
  sources fail before I/O, and credential reads return a redacting type.
- Structured help validates against help-v2, gives one minimal generated
  template per combination, and contains no embedded version values or
  long-form Markdown.
- Config/document edits preserve unrelated source text and avoid exposing
  secrets in argv.
- No compatibility-sensitive name changed without explicit approval.
