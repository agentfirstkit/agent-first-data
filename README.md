# Agent-First Data

A naming convention that lets AI agents understand your data without being told what it means.

> **Ask your agent:** "Apply the Agent-First Data convention across my project's fields, config, and logs."

## The problem: data doesn't say what it means

An agent reads `{"timeout": 5000}` from a tool. Seconds or milliseconds? It guesses — and a 5-second timeout silently becomes 83 minutes. The same trap is everywhere: `{"price": 1200}` gets charged as $1,200 instead of $12.00; `{"created": 1738886400}` is treated as an ID instead of a date.

It reads `{"api_key": "sk-live-abc123"}` and writes that line straight into a log file, because nothing marked the value as a secret.

Then it moves to the next tool, which calls the same value `elapsed` instead of `duration` — so what the agent learned about one tool tells it nothing about the next.

None of this is carelessness. The data never says what it means, so the meaning has to live somewhere else — documentation, a schema, a prompt. That copy goes stale, gets lost, or was never written.

## What it does: put the meaning into the field name

Agent-First Data puts the meaning into the field name itself. Call the field `timeout_ms` and there is nothing left to guess — the name says milliseconds. Call it `api_key_secret` and any tool that follows the convention hides it automatically.

It is a convention, not a framework — a small set of name endings, plus a tiny library in four languages that reads and formats them.

- **Names carry meaning.** Endings like `_ms`, `_bytes`, `_secret`, `_usd_cents`, and `_percent` put units and intent directly into the field name.
- **One set of data, three ways to show it.** The same fields render as JSON for machines, YAML for people, or a single log line for scanning — units formatted, secrets removed.
- **Secrets stay secret.** Anything ending in `_secret` is hidden automatically, in output and in logs. A `_url` field keeps its address but scrubs the userinfo password and secret-named query parameters. Legacy names like `api_key` can be protected by passing an explicit secret-name list.
- **Logging agents can read.** Structured logs that follow the same rules, with request-scoped fields.
- **The same in four languages.** One identical API across Rust, Go, Python, and TypeScript.

## Where to use it: CLI flags, config files, logs, and API responses

- **Building a CLI tool an agent will call** — your output is understood correctly the first time, with no extra schema to ship.
- **Writing a config file** — keys like `timeout_s` or `db_password_secret` make settings self-explanatory to whoever edits them, and secrets stay hidden when the config is printed back.
- **Adding logs to a service** — the same lines stay readable for a person and parseable for an agent.
- **Designing an API response or event payload** — units and sensitivity travel *with* the data, across every boundary it crosses.
- **Auditing for leaked secrets** — one naming rule (`_secret`) makes redaction automatic instead of case-by-case.

## Adopt it: hand the convention to your coding agent

Agent-First Data is a convention, not a dependency you wire in by hand — and adopting a convention is exactly the kind of work you now hand to an agent. There's even an [Agent Skill](skills/agent-first-data.md) for exactly that — the convention in a form an agent reads and applies directly. Paste this to your coding agent:

> Learn the Agent-First Data convention: read https://agentfirstkit.com/agent-first-data/docs/overview and https://agentfirstkit.com/agent-first-data/docs/agent-skill. Then look at the codebase we're working in and tell me whether adopting the convention would help it — and if so, how: which fields and config keys to rename, and where the output and logging helpers fit.

The library, if you want it:

```bash
cargo add agent-first-data       # Rust
pip install agent-first-data     # Python
npm install agent-first-data     # TypeScript
go get github.com/agentfirstkit/agent-first-data/go   # Go
```

## Docs

- [Overview](docs/overview.md) — the full guide: examples, the complete API, every supported suffix, and logging
- [Specification](spec/agent-first-data.md) — the formal convention
- [Agent Skill](skills/agent-first-data.md) — for AI-assisted development
- Per-language API reference: [Rust](rust) · [Go](go) · [Python](python) · [TypeScript](typescript)

## License

MIT
