<!-- Generated. Do not edit by hand. Regenerate: afdata --help --recursive --output markdown -->

# afdata CLI Reference

# afdata - A naming convention that lets AI agents understand your data without being told what it means, plus a CLI and library for reading and safely editing structured JSON, TOML, YAML, dotenv, and INI documents.

Commands are grouped into two families: protocol tools that operate on AFDATA protocol-v1 JSON (lint, validate, render, emit), and document tools that read and edit JSON/TOML/YAML/dotenv/INI documents by dot-path (get, value, paths, keys, set, unset, add, remove). `shell bash` exports the sourceable Bash authoring kit. Every data command's first positional is its input; `-` reads stdin. Mutation commands (set/unset/add/remove) never read stdin.

```text
afdata [OPTIONS] <COMMAND>
```

| Argument | Description |
| --- | --- |
| `--output <OUTPUT>` | Output format: json, yaml, or plain (help also accepts markdown) *(global, default: `json`)* |
| `--output-to <OUTPUT_TO>` | Where protocol events go: split (default), stdout, or stderr *(global, default: `split`)* |
| `--stdout-file <PATH>` | Redirect stdout to this file *(global)* |
| `--stderr-file <PATH>` | Redirect stderr to this file *(global)* |
| `--help` | Print help. Add --recursive to expand every nested subcommand; add --output plain\|json\|yaml\|markdown to choose the format. |
| `--version` | Print version |

| Command | Summary |
| --- | --- |
| `afdata lint` | Lint a JSON/JSONL stream, a JSON Schema, or a document for deterministic AFDATA issues |
| `afdata validate` | Validate one protocol event or a finite protocol event stream (JSON only) |
| `afdata render` | Render JSON or JSONL through AFDATA output formatting and redaction (JSON only) |
| `afdata emit` | Emit one AFDATA event from shell-safe scalar arguments |
| `afdata shell` | Export a sourceable shell authoring kit |
| `afdata skill` | Validate an Agent Skill, or manage the bundled Agent Skill |
| `afdata get` | Read a document as a whole, or the value at a dot-path |
| `afdata value` | Read the scalar at a dot-path as raw bytes on stdout — no AFDATA envelope |
| `afdata paths` | List a container's child dot-paths, one per line — feeds back into afdata |
| `afdata keys` | List a container's child key names or array indices, one per line — for external tools |
| `afdata set` | Set a value at a dot-path, preserving the document's source formatting |
| `afdata unset` | Remove one entry from a document entirely |
| `afdata add` | Add an element to a keyed list (an array of objects addressed by a slug field) |
| `afdata remove` | Remove an element from a keyed list by slug |

## afdata lint - Lint a JSON/JSONL stream, a JSON Schema, or a document for deterministic AFDATA issues

JSON/JSONL input (the default when no document format is detected) keeps its existing dual-mode behavior: a single JSON value, or one value per line. `--input-format toml|yaml|yml|dotenv|env|ini` (or a recognized file extension) lints a document as a single value instead — the AFDATA naming/suffix rules apply equally there. `toml-frontmatter`/`yaml-frontmatter` address only the `+++`/`---` metadata block of a Markdown file, leaving its body untouched (never auto-detected — the format must be named explicitly).

```text
afdata lint [OPTIONS] <INPUT>
```

| Argument | Description |
| --- | --- |
| `INPUT` | Input file, or `-` for stdin *(required)* |
| `--input-format <FORMAT>` | Document format override; unset means JSON/JSONL unless the file extension names a document format |

## afdata validate - Validate one protocol event or a finite protocol event stream (JSON only)

```text
afdata validate [OPTIONS] <INPUT>
```

| Argument | Description |
| --- | --- |
| `INPUT` | Input file, or `-` for stdin *(required)* |
| `--strict` | Enforce the recommended strict protocol profile |
| `--per-event` | Validate each input value as an independent event, without stream lifecycle rules |

## afdata render - Render JSON or JSONL through AFDATA output formatting and redaction (JSON only)

```text
afdata render [OPTIONS] <INPUT>
```

| Argument | Description |
| --- | --- |
| `INPUT` | Input file, or `-` for stdin *(required)* |
| `--secret-name <FIELD>...` | Extra field name to redact (beyond the `_secret` suffix convention). Repeatable |

## afdata emit - Emit one AFDATA event from shell-safe scalar arguments

```text
afdata emit <COMMAND>
```

| Command | Summary |
| --- | --- |
| `afdata emit log` | Emit a diagnostic log event (stderr under the default split) |
| `afdata emit result` | Emit a terminal result event (stdout under the default split) |
| `afdata emit error` | Emit a terminal error event and exit with status 1 |

### afdata emit log - Emit a diagnostic log event (stderr under the default split)

```text
afdata emit log <LEVEL> <MESSAGE>
```

| Argument | Description |
| --- | --- |
| `LEVEL` | Log level: debug, info, warn, or error *(required)* |
| `MESSAGE` | Human-readable message; do not place secrets in free text *(required)* |

### afdata emit result - Emit a terminal result event (stdout under the default split)

```text
afdata emit result <MESSAGE>
```

| Argument | Description |
| --- | --- |
| `MESSAGE` | Human-readable result message *(required)* |

### afdata emit error - Emit a terminal error event and exit with status 1

```text
afdata emit error [OPTIONS] <CODE> <MESSAGE>
```

| Argument | Description |
| --- | --- |
| `CODE` | Stable machine-readable error code *(required)* |
| `MESSAGE` | Human-readable error message *(required)* |
| `--hint <HINT>` | Suggested corrective action |
| `--retryable` | Mark the failure as safe to retry |

## afdata shell - Export a sourceable shell authoring kit

```text
afdata shell <COMMAND>
```

| Command | Summary |
| --- | --- |
| `afdata shell bash` | Print the sourceable Bash authoring kit as raw Bash on stdout |

### afdata shell bash - Print the sourceable Bash authoring kit as raw Bash on stdout

```text
afdata shell bash
```

## afdata skill - Validate an Agent Skill, or manage the bundled Agent Skill

```text
afdata skill <COMMAND>
```

| Command | Summary |
| --- | --- |
| `afdata skill validate` | Validate a SKILL.md file or skill directory against the Agent Skills spec |
| `afdata skill status` | Report whether the bundled Agent Skill is installed for each target agent |
| `afdata skill install` | Install the bundled Agent Skill for each target agent |
| `afdata skill uninstall` | Uninstall the bundled Agent Skill for each target agent |

### afdata skill validate - Validate a SKILL.md file or skill directory against the Agent Skills spec

```text
afdata skill validate <INPUT>
```

| Argument | Description |
| --- | --- |
| `INPUT` | SKILL.md file or skill directory, or `-` for SKILL.md text on stdin *(required)* |

### afdata skill status - Report whether the bundled Agent Skill is installed for each target agent

```text
afdata skill status [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--agent <AGENT>` | Agent target: all, codex, claude-code, opencode, or hermes *(default: `all`)* |
| `--scope <SCOPE>` | Skill scope: personal or workspace *(default: `personal`)* |
| `--skills-dir <SKILLS_DIR>` | Explicit skills directory; requires a single concrete --agent |

### afdata skill install - Install the bundled Agent Skill for each target agent

```text
afdata skill install [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--agent <AGENT>` | Agent target: all, codex, claude-code, opencode, or hermes *(default: `all`)* |
| `--scope <SCOPE>` | Skill scope: personal or workspace *(default: `personal`)* |
| `--skills-dir <SKILLS_DIR>` | Explicit skills directory; requires a single concrete --agent |
| `--force` | Overwrite a skill this tool did not manage |

### afdata skill uninstall - Uninstall the bundled Agent Skill for each target agent

```text
afdata skill uninstall [OPTIONS]
```

| Argument | Description |
| --- | --- |
| `--agent <AGENT>` | Agent target: all, codex, claude-code, opencode, or hermes *(default: `all`)* |
| `--scope <SCOPE>` | Skill scope: personal or workspace *(default: `personal`)* |
| `--skills-dir <SKILLS_DIR>` | Explicit skills directory; requires a single concrete --agent |
| `--force` | Remove a skill this tool did not manage |

## afdata get - Read a document as a whole, or the value at a dot-path

With no KEY, emits `{"code":"document","format":...,"value":...}` — the whole document. With KEY, adds `"key"` and narrows `"value"` to that dot-path. `_secret`-suffixed fields (and any `--secret-name`) are redacted to `"***"` anywhere in the output, including a directly-targeted secret leaf — use `value --reveal-secret` to read a secret's real value.

```text
afdata get [OPTIONS] <FILE> [KEY]
```

| Argument | Description |
| --- | --- |
| `FILE` | Document file, or `-` for stdin *(required)* |
| `KEY` | Dot-separated key path (`\.` escapes a literal dot, `\\` a backslash); omit for the whole document |
| `--input-format <FORMAT>` | Document format override; unset means extension detection |
| `--secret-name <FIELD>...` | Extra field name to redact (beyond the `_secret` suffix convention). Repeatable |

## afdata value - Read the scalar at a dot-path as raw bytes on stdout — no AFDATA envelope

Only scalars (string/bool/integer/float/null) are supported; arrays and objects are rejected, as are non-finite floats. A secret-named leaf is rejected unless `--reveal-secret` is passed. On failure, stdout is always empty — the error envelope goes to stderr instead (so `x=$(afdata value f k)` never captures a JSON error as data).

```text
afdata value [OPTIONS] <FILE> <KEY>
```

| Argument | Description |
| --- | --- |
| `FILE` | Document file, or `-` for stdin *(required)* |
| `KEY` | Dot-separated key path *(required)* |
| `--reveal-secret` | Print a secret-named scalar instead of erroring |
| `--default <VALUE>` | Print this instead of erroring when KEY's path does not exist or its value is null (an empty string is a real value and does not trigger the default) |
| `--input-format <FORMAT>` | Document format override; unset means extension detection |
| `--secret-name <FIELD>...` | Extra field name to redact (beyond the `_secret` suffix convention). Repeatable |

## afdata paths - List a container's child dot-paths, one per line — feeds back into afdata

With no KEY, enumerates the document's top-level children. Each line is a full dot-path from the root (grammar-escaped), so it can be piped straight back into `get`/`value`/`unset`/… or extended with `"$p.field"`. A scalar leaf (nothing to enumerate) is an error, the dual of `value`. On failure, stdout is always empty (same contract as `value`). Rejects `--output json` — read a container's structured JSON via `get` instead.

```text
afdata paths [OPTIONS] <FILE> [KEY]
```

| Argument | Description |
| --- | --- |
| `FILE` | Document file, or `-` for stdin *(required)* |
| `KEY` | Dot-separated key path to the container; omit for the top level |
| `--input-format <FORMAT>` | Document format override; unset means extension detection |
| `--missing-ok` | Empty output + exit 0 when KEY's path does not exist (other errors still fail) |
| `--null` | Separate lines with NUL instead of newline (for `xargs -0`/`read -d ''`) |

## afdata keys - List a container's child key names or array indices, one per line — for external tools

The dual of `paths`: raw, unescaped, unprefixed key names/indices — exactly what a package manager or another tool expects (`lodash.merge`, not `dependencies.lodash\.merge`). Never feed this back into afdata's own dot-path arguments; use `paths` for that. Otherwise identical contract to `paths` (KEY, `--input-format`, `--missing-ok`, `-0`/`--null`, scalar-leaf error, empty stdout on failure, rejects `--output json`).

```text
afdata keys [OPTIONS] <FILE> [KEY]
```

| Argument | Description |
| --- | --- |
| `FILE` | Document file, or `-` for stdin *(required)* |
| `KEY` | Dot-separated key path to the container; omit for the top level |
| `--input-format <FORMAT>` | Document format override; unset means extension detection |
| `--missing-ok` | Empty output + exit 0 when KEY's path does not exist (other errors still fail) |
| `--null` | Separate lines with NUL instead of newline (for `xargs -0`/`read -d ''`) |

## afdata set - Set a value at a dot-path, preserving the document's source formatting

A bare VALUE is always a string — zero coercion, so `007` or a leading-zero-bearing ID is never silently reinterpreted. Overwriting an *existing* scalar of a different type with a bare VALUE is an argument error (pass `--value-type` to keep the type, or `--value-type string` to convert explicitly); a brand-new key never needs `--value-type`. `--value-type json` is the only entry point for arrays, objects, and an exact-type scalar. Idempotency: setting an already-current value is not special-cased — it just writes the same value again.

```text
afdata set [OPTIONS] <FILE> <KEY> [VALUE]
```

| Argument | Description |
| --- | --- |
| `FILE` | Document file to mutate in place (never reads stdin; rejects `-`) *(required)* |
| `KEY` | Dot-separated key path *(required)* |
| `VALUE` | Value to write; interpreted per `--value-type` (default: string, zero coercion) |
| `--value-type <TYPE>` | Exact type for VALUE: string (default), number, bool, null, or json |
| `--secret-from <SRC>` | Read a secret string VALUE from stdin, the controlling terminal, an inherited file descriptor, or an environment variable: stdin\|prompt\|fd:<N>\|env:<VAR> |
| `--input-format <FORMAT>` | Document format override; unset means extension detection |

## afdata unset - Remove one entry from a document entirely

Idempotency: removing an absent KEY is an error (`document_path_not_found`), not a no-op — script around it with `afdata unset ... || true` if absence should be silent.

```text
afdata unset [OPTIONS] <FILE> <KEY>
```

| Argument | Description |
| --- | --- |
| `FILE` | Document file to mutate in place (never reads stdin; rejects `-`) *(required)* |
| `KEY` | Dot-path to the entry to remove *(required)* |
| `--input-format <FORMAT>` | Document format override; unset means extension detection |

## afdata add - Add an element to a keyed list (an array of objects addressed by a slug field)

Extra `FIELD=VALUE` pairs are always strings (the same zero-coercion rule as `set`'s bare VALUE — `add` does not invent its own type syntax; write an exact type afterwards with `set --value-type`). Idempotency: adding a SLUG that already exists is an error (`document_slug_exists`), not a no-op or overwrite.

```text
afdata add [OPTIONS] --slug-field <SLUG_FIELD> <FILE> <KEY> <SLUG> [FIELD=VALUE]...
```

| Argument | Description |
| --- | --- |
| `FILE` | Document file to mutate in place (never reads stdin; rejects `-`) *(required)* |
| `KEY` | Dot-path to the keyed list *(required)* |
| `SLUG` | Slug/ID for the new element *(required)* |
| `--slug-field <SLUG_FIELD>` | Field name that identifies each element (the slug field) *(required)* |
| `FIELD=VALUE...` | Additional `FIELD=VALUE` pairs to set on the new element (always strings) |
| `--input-format <FORMAT>` | Document format override; unset means extension detection |

## afdata remove - Remove an element from a keyed list by slug

Idempotency: removing a SLUG that does not exist is an error (`document_slug_not_found`), not a no-op.

```text
afdata remove [OPTIONS] --slug-field <SLUG_FIELD> <FILE> <KEY> <SLUG>
```

| Argument | Description |
| --- | --- |
| `FILE` | Document file to mutate in place (never reads stdin; rejects `-`) *(required)* |
| `KEY` | Dot-path to the keyed list *(required)* |
| `SLUG` | Slug/ID of the element to remove *(required)* |
| `--slug-field <SLUG_FIELD>` | Field name that identifies each element (the slug field) *(required)* |
| `--input-format <FORMAT>` | Document format override; unset means extension detection |
