<!-- Canonical focused reference for the installed Agent-First Data skill. -->

# Safe structured-document workflow

Use `afdata` document commands instead of `sed`, regex replacement, or a
generic reserializer when reading or editing JSON, TOML, YAML, dotenv, INI, or
Markdown frontmatter. These commands preserve unrelated comments, order,
quoting, anchors, and body text.

## Contents

- [Select the command](#select-the-command)
- [Read safely](#read-safely)
- [Write exact types](#write-exact-types)
- [Keyed lists](#keyed-lists)
- [Markdown frontmatter](#markdown-frontmatter)
- [Mutation safety and errors](#mutation-safety-and-errors)
- [Exit criteria](#exit-criteria)

## Select the command

| Need | Command |
|---|---|
| Whole document or subtree as an AFDATA record | `get FILE [KEY]` |
| One raw scalar for shell use | `value FILE KEY` |
| Immediate children as reusable dot-paths | `paths FILE [KEY]` |
| Immediate children as literal external names | `keys FILE [KEY]` |
| Write one value | `set FILE KEY VALUE` |
| Insert into a slug-keyed object array | `add FILE KEY SLUG FIELD=VALUE...` |
| Remove a slug-keyed item | `remove FILE KEY SLUG` |
| Remove one dot-path | `unset FILE KEY` |

The first positional is always the file. Reads accept `-` for explicit stdin;
mutations reject stdin.

## Read safely

`get` always redacts a directly targeted `_secret` leaf to `***`. Pass
`--secret-name FIELD` for every exact legacy secret name.

`value` writes a scalar with no envelope and no forced newline. It rejects
containers. On failure its stdout is empty and the structured error goes to
stderr, so shell substitution cannot capture an error as data:

```bash
port="$(afdata value config.toml server.port --default 8080)"
```

Secret leaves require the explicit, auditable `--reveal-secret` option.
`--default VALUE` applies only when the path is absent or null; an empty string
is a real value.

`paths` emits root-relative grammar-escaped paths that feed back into
`get`/`value`/`unset`. `keys` emits raw immediate names for external tools.
They differ when a key contains a dot or space.

Consume either line stream with `while IFS= read -r`; never use command
substitution in a `for` loop. Path grammar escapes a literal dot as `\.` and a
literal backslash as `\\`. Unknown escapes and empty segments are errors.

## Write exact types

A bare `set` value and each bare `FIELD=VALUE` is always a string. Use:

```bash
afdata set config.toml server.port 8080 --value-type number
afdata set config.toml feature.enabled true --value-type bool
afdata set config.toml cache.value ignored --value-type null
afdata set config.toml routes '[{"path":"/"}]' --value-type json
```

`--value-type json` is the only way to write an object or array. Replacing an
existing scalar with a different kind requires an explicit value type.

For secrets, do not put values in argv. Use
`--secret-from stdin|prompt|fd:<N>|env:<VAR>`. There is no inline argv form.

## Keyed lists

`add` and `remove` operate on arrays of objects addressed by a slug and always
require `--slug-field FIELD`; identity is never inferred. Keyed editing is
implemented for JSON and YAML only.

Existing or missing slugs are explicit errors, not silent no-ops. Wrap the
command deliberately if idempotence is required.

## Markdown frontmatter

Name the format explicitly:

- `--input-format toml-frontmatter` for a `+++` block;
- `--input-format yaml-frontmatter` for a `---` block.

Frontmatter is never auto-detected. The same dot-path commands then edit only
the metadata block and leave the Markdown body byte-for-byte unchanged.

## Mutation safety and errors

Mutations are atomic and source-preserving. They refuse symlink targets and,
on Unix, multi-link hardlink targets. Treat a failed mutation as leaving the
original untouched, then reread before retrying.

Branch on stable `error.code`, not message text. Common runtime codes include:

- `document_path_not_found`
- `document_type_mismatch`
- `document_slug_not_found`
- `document_slug_exists`
- `document_parse_failed`
- `document_io_failed`
- `document_not_scalar`
- `document_not_container`
- `document_secret_redacted`

Malformed invocation is `document_usage_error` with exit 2; runtime document
errors exit 1. A successful mutation result includes the path written.

## Exit criteria

1. The chosen format and dot-path target are explicit.
2. Reads cannot leak a secret.
3. Secret writes avoid argv.
4. Value type is explicit whenever it is not a string.
5. Unrelated source text remains unchanged.
6. The result path or stable error code is checked.
