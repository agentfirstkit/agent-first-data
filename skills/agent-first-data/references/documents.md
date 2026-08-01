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

`get` preserves JSON types and structure inside its result envelope. `value`
deliberately stringifies one scalar into raw shell bytes. Use `get` (or the
typed library API) for read-modify-write work; use `value` only when the next
consumer explicitly wants a scalar string. For a CLI round trip, take the
typed value from `get` and write it with the matching `set --value-type`
(`json` for a container); never feed `value` output back when type matters.

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

`--value-type json` is the only way to write an object or array. Overwriting
anything that is not already a string requires an explicit value type — a
differently-typed scalar, and also an existing array or object, where a bare
value would discard the whole container. The error names both ways out: keep
what is there (`--value-type json` for a container, its own type for a scalar),
or replace it deliberately with `--value-type string`.

`set` creates missing object parents along the dot-path. It does not replace an
existing scalar or incompatible container in the middle of the path; that is a
path/type error. Treat parent creation as part of the requested mutation and
review the full path before writing.

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
- `document_format_unknown`
- `document_write_would_corrupt`
- `document_io_failed`
- `document_not_scalar`
- `document_not_container`
- `document_secret_redacted`

`document_format_unknown` means the file's extension named no format, so
nothing was parsed — pass `--input-format`. It is distinct from
`document_parse_failed`, which means a parser read the file and rejected it.

`document_write_would_corrupt` means the edit was rendered, read back, and
found unparseable, so it was refused before reaching disk. The file is
unchanged. Treat it as a bug in the tool, not as something to retry.

No error message quotes document content: a parse failure reports the format
and position, a type mismatch reports the path and the type it found. Branch on
the code and the path, never on a value you expect to see echoed back.

Malformed invocation is `document_usage_error` with exit 2; runtime document
errors exit 1. A successful mutation result includes the path written.

## Exit criteria

1. The chosen format and dot-path target are explicit.
2. Reads cannot leak a secret.
3. Secret writes avoid argv.
4. Value type is explicit whenever it is not a string.
5. Unrelated source text remains unchanged.
6. The result path or stable error code is checked.
