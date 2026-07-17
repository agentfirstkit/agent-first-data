<!-- Generated. Do not edit by hand. Regenerate: afdata --help --recursive --output markdown -->

# afdata CLI Reference

# afdata - Validate, lint, and render Agent-First Data JSON.

```text
Usage: afdata [OPTIONS] <COMMAND>

Commands:
  lint      Lint ordinary JSON, JSONL, or JSON Schema for deterministic AFDATA issues
  validate  Validate one protocol event or a finite protocol event stream
  render    Render JSON or JSONL through AFDATA output formatting and redaction
  skill     Validate an Agent Skill or manage the bundled Agent Skill
  show      Show a document as a full AFDATA record
  get       Get the value at a dot-path as an AFDATA record
  value     Get the value at a dot-path as raw scalar bytes on stdout, with no AFDATA envelope
  set       Set a scalar value at a dot-path, preserving the document's source formatting
  add       Add an element to a keyed list (an array of objects addressed by a slug field)
  remove    Remove an element from a keyed list by slug
  unset     Remove one entry from a document entirely

Options:
      --output <OUTPUT>
          Output format: json, yaml, or plain (help also accepts markdown)

          [default: json]

      --stdout-file <PATH>
          Redirect stdout to this file

      --stderr-file <PATH>
          Redirect stderr to this file

      --input-format <FORMAT>
          Document format override for `show`/`get`/`value`/`set`/`add`/`remove`/`unset`: json, toml, yaml, yml, dotenv, env, or ini.

          Overrides file-extension detection for a FILE/--input-file argument, and overrides the JSON default when a read command falls back to stdin.

      --secret-name <FIELD>
          Extra field name to redact (beyond the `_secret` suffix convention) in `show`/`get`/`value` output. Repeatable

  -h, --help
          Print help. Add --recursive to expand every nested subcommand; add --output json|yaml|markdown to render this help in another format.

  -V, --version
          Print version

AFDATA: 0.17.3
```

## afdata lint - Lint ordinary JSON, JSONL, or JSON Schema for deterministic AFDATA issues

```text
Usage: lint [INPUT]

Arguments:
  [INPUT]
          Input file; stdin is used when omitted

Options:
  -h, --help
          Print help
```

## afdata validate - Validate one protocol event or a finite protocol event stream

```text
Usage: validate [OPTIONS] [INPUT]

Arguments:
  [INPUT]
          Input file; stdin is used when omitted

Options:
      --strict
          Enforce the recommended strict protocol profile

      --event
          Validate each input value as an independent event, without stream lifecycle rules

  -h, --help
          Print help
```

## afdata render - Render JSON or JSONL through AFDATA output formatting and redaction

```text
Usage: render [INPUT]

Arguments:
  [INPUT]
          Input file; stdin is used when omitted

Options:
  -h, --help
          Print help
```

## afdata skill - Validate an Agent Skill or manage the bundled Agent Skill

```text
Usage: skill [OPTIONS] <ACTION> [INPUT]

Arguments:
  <ACTION>
          Action: validate, status, install, or uninstall

  [INPUT]
          SKILL.md file or skill directory for the validate action; stdin when omitted

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
          Overwrite or remove a skill this tool did not manage

  -h, --help
          Print help
```

## afdata show - Show a document as a full AFDATA record

Reads FILE (or stdin when omitted) and emits `{"code":"document","format":...,"value":...}`. `_secret`-suffixed fields (and any `--secret-name`) are redacted to `"***"` anywhere in the document.

```text
Usage: show [FILE]

Arguments:
  [FILE]
          Document file path; stdin is used when omitted

Options:
  -h, --help
          Print help
```

## afdata get - Get the value at a dot-path as an AFDATA record

Emits `{"code":"document_value","format":...,"key":...,"value":...}`. If `KEY`'s leaf field name is a secret (the `_secret` suffix convention or `--secret-name`), the value is redacted to `"***"` even though it was explicitly targeted — use `value --reveal-secret` to read a secret's real value.

```text
Usage: get <KEY> [FILE]

Arguments:
  <KEY>
          Dot-separated key path (`\.` escapes a literal dot, `\\` a backslash)

  [FILE]
          Document file path; stdin is used when omitted

Options:
  -h, --help
          Print help
```

## afdata value - Get the value at a dot-path as raw scalar bytes on stdout, with no AFDATA envelope

Only scalars (string/bool/integer/float/null) are supported; arrays and objects are rejected, as are non-finite floats. A secret-named leaf is rejected unless `--reveal-secret` is passed.

```text
Usage: value [OPTIONS] <KEY> [FILE]

Arguments:
  <KEY>
          Dot-separated key path

  [FILE]
          Document file path; stdin is used when omitted

Options:
      --reveal-secret
          Print a secret-named scalar instead of erroring

  -h, --help
          Print help
```

## afdata set - Set a scalar value at a dot-path, preserving the document's source formatting

```text
Usage: set [OPTIONS] --input-file <PATH> <KEY> [VALUES]...

Arguments:
  <KEY>
          Dot-separated key path

  [VALUES]...
          Value(s) to set (multiple arguments become an array)

Options:
      --value-secret <VALUE_SECRET>
          Secret scalar value (visible to process observers such as `ps`)

      --value-secret-stdin
          Read one secret scalar from stdin to EOF

      --value-secret-prompt
          Read one secret scalar from the controlling terminal

      --value-secret-fd <FD>
          Read one secret scalar from an inherited Unix file descriptor

      --input-file <PATH>
          Document file to mutate in place (required; mutation never reads stdin)

  -h, --help
          Print help
```

## afdata add - Add an element to a keyed list (an array of objects addressed by a slug field)

```text
Usage: add --slug-field <SLUG_FIELD> --input-file <PATH> <KEY> <SLUG> [FIELD=VALUE]...

Arguments:
  <KEY>
          Dot-path to the keyed list

  <SLUG>
          Slug/ID for the new element

  [FIELD=VALUE]...
          Additional `FIELD=VALUE` pairs to set on the new element

Options:
      --slug-field <SLUG_FIELD>
          Field name that identifies each element (the slug field)

      --input-file <PATH>
          Document file to mutate in place (required; mutation never reads stdin)

  -h, --help
          Print help
```

## afdata remove - Remove an element from a keyed list by slug

```text
Usage: remove --slug-field <SLUG_FIELD> --input-file <PATH> <KEY> <SLUG>

Arguments:
  <KEY>
          Dot-path to the keyed list

  <SLUG>
          Slug/ID of the element to remove

Options:
      --slug-field <SLUG_FIELD>
          Field name that identifies each element (the slug field)

      --input-file <PATH>
          Document file to mutate in place (required; mutation never reads stdin)

  -h, --help
          Print help
```

## afdata unset - Remove one entry from a document entirely

```text
Usage: unset --input-file <PATH> <KEY>

Arguments:
  <KEY>
          Dot-path to the entry to remove

Options:
      --input-file <PATH>
          Document file to mutate in place (required; mutation never reads stdin)

  -h, --help
          Print help
```
