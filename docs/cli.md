<!-- Generated. Do not edit by hand. Regenerate: afdata --help --recursive --output markdown -->

# afdata CLI Reference

# afdata - A naming convention that lets AI agents understand your data without being told what it means, plus a CLI and library for reading and safely editing structured JSON, TOML, YAML, dotenv, and INI documents.

Commands are grouped into two families: protocol tools that operate on AFDATA protocol-v1 JSON (lint, validate, render), and document tools that read and edit JSON/TOML/YAML/dotenv/INI documents by dot-path (get, value, paths, keys, set, unset, add, remove). Every command's first positional is its input; `-` reads stdin. Mutation commands (set/unset/add/remove) never read stdin.

```text
Usage: afdata [OPTIONS] <COMMAND>

Commands:
  lint      Lint a JSON/JSONL stream, a JSON Schema, or a document for deterministic AFDATA issues
  validate  Validate one protocol event or a finite protocol event stream (JSON only)
  render    Render JSON or JSONL through AFDATA output formatting and redaction (JSON only)
  skill     Validate an Agent Skill, or manage the bundled Agent Skill
  get       Read a document as a whole, or the value at a dot-path
  value     Read the scalar at a dot-path as raw bytes on stdout — no AFDATA envelope
  paths     List a container's child dot-paths, one per line — feeds back into afdata
  keys      List a container's child key names or array indices, one per line — for external tools
  set       Set a value at a dot-path, preserving the document's source formatting
  unset     Remove one entry from a document entirely
  add       Add an element to a keyed list (an array of objects addressed by a slug field)
  remove    Remove an element from a keyed list by slug

Options:
      --output <OUTPUT>
          Output format: json, yaml, or plain (help also accepts markdown)

          [default: json]

      --output-to <OUTPUT_TO>
          Where protocol events go: split (default), stdout, or stderr.

          `split` (default, finite one-shot mode) sends `result` to stdout and `error`/`progress`/`log` to stderr, so a shell capture or pipe never mistakes a failure for data. `stdout`/`stderr` (event-stream mode) collapse every event, including `error`, onto that one stream for a consumer that reads it in order and branches on `kind`. Orthogonal to `--output` (which selects format, not destination). A file sink is `--output-to stdout` plus `--stdout-file <PATH>`.

          [default: split]

      --stdout-file <PATH>
          Redirect stdout to this file

      --stderr-file <PATH>
          Redirect stderr to this file

  -h, --help
          Print help. Add --recursive to expand every nested subcommand; add --output json|yaml|markdown to render this help in another format.

  -V, --version
          Print version

AFDATA: 0.22.0
```

## afdata lint - Lint a JSON/JSONL stream, a JSON Schema, or a document for deterministic AFDATA issues

JSON/JSONL input (the default when no document format is detected) keeps its existing dual-mode behavior: a single JSON value, or one value per line. `--input-format toml|yaml|yml|dotenv|env|ini` (or a recognized file extension) lints a document as a single value instead — the AFDATA naming/suffix rules apply equally there. `toml-frontmatter`/`yaml-frontmatter` address only the `+++`/`---` metadata block of a Markdown file, leaving its body untouched (never auto-detected — the format must be named explicitly).

```text
Usage: lint [OPTIONS] <INPUT>

Arguments:
  <INPUT>
          Input file, or `-` for stdin

Options:
      --input-format <FORMAT>
          Document format override; unset means JSON/JSONL unless the file extension names a document format

  -h, --help
          Print help
```

## afdata validate - Validate one protocol event or a finite protocol event stream (JSON only)

```text
Usage: validate [OPTIONS] <INPUT>

Arguments:
  <INPUT>
          Input file, or `-` for stdin

Options:
      --strict
          Enforce the recommended strict protocol profile

      --per-event
          Validate each input value as an independent event, without stream lifecycle rules

  -h, --help
          Print help
```

## afdata render - Render JSON or JSONL through AFDATA output formatting and redaction (JSON only)

```text
Usage: render [OPTIONS] <INPUT>

Arguments:
  <INPUT>
          Input file, or `-` for stdin

Options:
      --secret-name <FIELD>
          Extra field name to redact (beyond the `_secret` suffix convention). Repeatable

  -h, --help
          Print help
```

## afdata skill - Validate an Agent Skill, or manage the bundled Agent Skill

```text
Usage: skill <COMMAND>

Commands:
  validate   Validate a SKILL.md file or skill directory against the Agent Skills spec
  status     Report whether the bundled Agent Skill is installed for each target agent
  install    Install the bundled Agent Skill for each target agent
  uninstall  Uninstall the bundled Agent Skill for each target agent

Options:
  -h, --help
          Print help
```

### afdata skill validate - Validate a SKILL.md file or skill directory against the Agent Skills spec

```text
Usage: validate <INPUT>

Arguments:
  <INPUT>
          SKILL.md file or skill directory, or `-` for SKILL.md text on stdin

Options:
  -h, --help
          Print help
```

### afdata skill status - Report whether the bundled Agent Skill is installed for each target agent

```text
Usage: status [OPTIONS]

Options:
      --agent <AGENT>
          Agent target: all, codex, claude-code, opencode, or hermes

          [default: all]

      --scope <SCOPE>
          Skill scope: personal or workspace

          [default: personal]

      --skills-dir <SKILLS_DIR>
          Explicit skills directory; requires a single concrete --agent

  -h, --help
          Print help
```

### afdata skill install - Install the bundled Agent Skill for each target agent

```text
Usage: install [OPTIONS]

Options:
      --agent <AGENT>
          Agent target: all, codex, claude-code, opencode, or hermes

          [default: all]

      --scope <SCOPE>
          Skill scope: personal or workspace

          [default: personal]

      --skills-dir <SKILLS_DIR>
          Explicit skills directory; requires a single concrete --agent

      --force
          Overwrite a skill this tool did not manage

  -h, --help
          Print help
```

### afdata skill uninstall - Uninstall the bundled Agent Skill for each target agent

```text
Usage: uninstall [OPTIONS]

Options:
      --agent <AGENT>
          Agent target: all, codex, claude-code, opencode, or hermes

          [default: all]

      --scope <SCOPE>
          Skill scope: personal or workspace

          [default: personal]

      --skills-dir <SKILLS_DIR>
          Explicit skills directory; requires a single concrete --agent

      --force
          Remove a skill this tool did not manage

  -h, --help
          Print help
```

## afdata get - Read a document as a whole, or the value at a dot-path

With no KEY, emits `{"code":"document","format":...,"value":...}` — the whole document. With KEY, adds `"key"` and narrows `"value"` to that dot-path. `_secret`-suffixed fields (and any `--secret-name`) are redacted to `"***"` anywhere in the output, including a directly-targeted secret leaf — use `value --reveal-secret` to read a secret's real value.

```text
Usage: get [OPTIONS] <FILE> [KEY]

Arguments:
  <FILE>
          Document file, or `-` for stdin

  [KEY]
          Dot-separated key path (`\.` escapes a literal dot, `\\` a backslash); omit for the whole document

Options:
      --input-format <FORMAT>
          Document format override; unset means extension detection

      --secret-name <FIELD>
          Extra field name to redact (beyond the `_secret` suffix convention). Repeatable

  -h, --help
          Print help
```

## afdata value - Read the scalar at a dot-path as raw bytes on stdout — no AFDATA envelope

Only scalars (string/bool/integer/float/null) are supported; arrays and objects are rejected, as are non-finite floats. A secret-named leaf is rejected unless `--reveal-secret` is passed. On failure, stdout is always empty — the error envelope goes to stderr instead (so `x=$(afdata value f k)` never captures a JSON error as data).

```text
Usage: value [OPTIONS] <FILE> <KEY>

Arguments:
  <FILE>
          Document file, or `-` for stdin

  <KEY>
          Dot-separated key path

Options:
      --reveal-secret
          Print a secret-named scalar instead of erroring

      --default <VALUE>
          Print this instead of erroring when KEY's path does not exist or its value is null (an empty string is a real value and does not trigger the default)

      --input-format <FORMAT>
          Document format override; unset means extension detection

      --secret-name <FIELD>
          Extra field name to redact (beyond the `_secret` suffix convention). Repeatable

  -h, --help
          Print help
```

## afdata paths - List a container's child dot-paths, one per line — feeds back into afdata

With no KEY, enumerates the document's top-level children. Each line is a full dot-path from the root (grammar-escaped), so it can be piped straight back into `get`/`value`/`unset`/… or extended with `"$p.field"`. A scalar leaf (nothing to enumerate) is an error, the dual of `value`. On failure, stdout is always empty (same contract as `value`). Rejects `--output json` — read a container's structured JSON via `get` instead.

```text
Usage: paths [OPTIONS] <FILE> [KEY]

Arguments:
  <FILE>
          Document file, or `-` for stdin

  [KEY]
          Dot-separated key path to the container; omit for the top level

Options:
      --input-format <FORMAT>
          Document format override; unset means extension detection

      --missing-ok
          Empty output + exit 0 when KEY's path does not exist (other errors still fail)

  -0, --null
          Separate lines with NUL instead of newline (for `xargs -0`/`read -d ''`)

  -h, --help
          Print help
```

## afdata keys - List a container's child key names or array indices, one per line — for external tools

The dual of `paths`: raw, unescaped, unprefixed key names/indices — exactly what a package manager or another tool expects (`lodash.merge`, not `dependencies.lodash\.merge`). Never feed this back into afdata's own dot-path arguments; use `paths` for that. Otherwise identical contract to `paths` (KEY, `--input-format`, `--missing-ok`, `-0`/`--null`, scalar-leaf error, empty stdout on failure, rejects `--output json`).

```text
Usage: keys [OPTIONS] <FILE> [KEY]

Arguments:
  <FILE>
          Document file, or `-` for stdin

  [KEY]
          Dot-separated key path to the container; omit for the top level

Options:
      --input-format <FORMAT>
          Document format override; unset means extension detection

      --missing-ok
          Empty output + exit 0 when KEY's path does not exist (other errors still fail)

  -0, --null
          Separate lines with NUL instead of newline (for `xargs -0`/`read -d ''`)

  -h, --help
          Print help
```

## afdata set - Set a value at a dot-path, preserving the document's source formatting

A bare VALUE is always a string — zero coercion, so `007` or a leading-zero-bearing ID is never silently reinterpreted. Overwriting an *existing* scalar of a different type with a bare VALUE is an argument error (pass `--value-type` to keep the type, or `--value-type string` to convert explicitly); a brand-new key never needs `--value-type`. `--value-type json` is the only entry point for arrays, objects, and an exact-type scalar. Idempotency: setting an already-current value is not special-cased — it just writes the same value again.

```text
Usage: set [OPTIONS] <FILE> <KEY> [VALUE]

Arguments:
  <FILE>
          Document file to mutate in place (never reads stdin; rejects `-`)

  <KEY>
          Dot-separated key path

  [VALUE]
          Value to write; interpreted per `--value-type` (default: string, zero coercion)

Options:
      --value-type <TYPE>
          Exact type for VALUE: string (default), number, bool, null, or json

      --secret-from <SRC>
          Read a secret string VALUE from stdin, the controlling terminal, an inherited file descriptor, or an environment variable: stdin|prompt|fd:<N>|env:<VAR>

      --input-format <FORMAT>
          Document format override; unset means extension detection

  -h, --help
          Print help
```

## afdata unset - Remove one entry from a document entirely

Idempotency: removing an absent KEY is an error (`document_path_not_found`), not a no-op — script around it with `afdata unset ... || true` if absence should be silent.

```text
Usage: unset [OPTIONS] <FILE> <KEY>

Arguments:
  <FILE>
          Document file to mutate in place (never reads stdin; rejects `-`)

  <KEY>
          Dot-path to the entry to remove

Options:
      --input-format <FORMAT>
          Document format override; unset means extension detection

  -h, --help
          Print help
```

## afdata add - Add an element to a keyed list (an array of objects addressed by a slug field)

Extra `FIELD=VALUE` pairs are always strings (the same zero-coercion rule as `set`'s bare VALUE — `add` does not invent its own type syntax; write an exact type afterwards with `set --value-type`). Idempotency: adding a SLUG that already exists is an error (`document_slug_exists`), not a no-op or overwrite.

```text
Usage: add [OPTIONS] --slug-field <SLUG_FIELD> <FILE> <KEY> <SLUG> [FIELD=VALUE]...

Arguments:
  <FILE>
          Document file to mutate in place (never reads stdin; rejects `-`)

  <KEY>
          Dot-path to the keyed list

  <SLUG>
          Slug/ID for the new element

  [FIELD=VALUE]...
          Additional `FIELD=VALUE` pairs to set on the new element (always strings)

Options:
      --slug-field <SLUG_FIELD>
          Field name that identifies each element (the slug field)

      --input-format <FORMAT>
          Document format override; unset means extension detection

  -h, --help
          Print help
```

## afdata remove - Remove an element from a keyed list by slug

Idempotency: removing a SLUG that does not exist is an error (`document_slug_not_found`), not a no-op.

```text
Usage: remove [OPTIONS] --slug-field <SLUG_FIELD> <FILE> <KEY> <SLUG>

Arguments:
  <FILE>
          Document file to mutate in place (never reads stdin; rejects `-`)

  <KEY>
          Dot-path to the keyed list

  <SLUG>
          Slug/ID of the element to remove

Options:
      --slug-field <SLUG_FIELD>
          Field name that identifies each element (the slug field)

      --input-format <FORMAT>
          Document format override; unset means extension detection

  -h, --help
          Print help
```
