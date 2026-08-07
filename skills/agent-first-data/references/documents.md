<!-- Canonical focused reference for the installed Agent-First Data skill. -->

# Safe structured-document workflow

Use `afdata` document commands instead of `sed`, regex replacement, or a
generic reserializer when reading or editing JSON, TOML, YAML, dotenv, INI, or
Markdown frontmatter, and when reading Markdown structure. Mutating commands
preserve unrelated comments, order, quoting, anchors, and body text.

## Contents

- [Select the command](#select-the-command)
- [Read safely](#read-safely)
- [Write exact types](#write-exact-types)
- [Keyed lists](#keyed-lists)
- [Addressing an array element by content](#addressing-an-array-element-by-content)
- [Markdown](#markdown)
- [Mutation safety and errors](#mutation-safety-and-errors)
- [Exit criteria](#exit-criteria)

## Select the command

| Need | Command |
|---|---|
| Whole document or subtree as an AFDATA record | `get FILE [KEY]` |
| One raw scalar for shell use | `value FILE KEY` |
| Many scalars from a single parse | `values FILE KEY...` |
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

`paths` emits root-relative grammar-escaped paths that feed back into every read
and write verb — `get`/`value`/`set`/`unset` — unchanged. Pass the same
`--slug-field` you listed with, or a content-addressed path arrives at a verb
that cannot resolve it. `keys` emits raw immediate names for external tools.
They differ when a key contains a dot or space.

Consume either line stream with `while IFS= read -r`; never use command
substitution in a `for` loop. Path grammar escapes a literal dot as `\.` and a
literal backslash as `\\`. Unknown escapes and empty segments are errors.

For untrusted or secret-bearing paths in Rust, prefer
`DocumentFile::open_capped`. It opens once, checks regular-file metadata on
that handle, and limits the actual read to one byte beyond the cap so a path
replacement or later growth cannot bypass the limit. Choose
`SymlinkPolicy::NoFollow` when the final path component must not be a symlink;
on Unix this requires the default-on Cargo feature `libc`, and
`stream-redirect` enables that feature automatically. Builds without `libc`
and platforms without an atomic no-follow open refuse that policy instead of
simulating it with a racy precheck. Without `libc`, a normal `Follow` read
retains the single-handle and byte-cap guarantees, but opening a special file
can block before its type is rejected.

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

TOML source-preserving edits support arrays, numeric array elements, inline
tables, and ordinary tables. They retain surrounding comments, decor, key
order, trailing-comma style, and untouched datetime syntax. Arrays of tables
are explicitly refused because this editor has no caller-declared element
identity policy; do not fall back to a generic TOML reserializer.

INI entries before the first `[section]` header are document-root strings and
use a bare path (`http-password`); section entries use `section.key`. A root key
and section cannot share a name, because both would occupy the same top-level
address. For an extensionless or `.conf` file used as a value source, name the
format explicitly: `file+ini:PATH#DOT_PATH`.

In Rust, `Document::decode<T>()` deserializes the complete document, and
`DocumentFile::edit_and_validate<T>(...)` stages edits on a clone, decodes the
typed model, then commits once. An edit or type failure rolls back both the
in-memory handle and disk state.

For secrets, do not put values in argv. Use
`--secret-from stdin|prompt|fd:<N>|env:<VAR>`. There is no inline argv form.

## Keyed lists

`add` and `remove` operate on arrays of objects addressed by a slug and always
require `--slug-field FIELD`; identity is never inferred. Keyed editing is
implemented for JSON and YAML only.

An existing slug on `add` is `document_slug_exists`; a missing slug on
`remove` is `document_slug_not_found`. Wrap the command deliberately if
idempotence is required. If `remove` finds the same exact slug more than once,
it reports `document_ambiguous_match` and leaves the document unchanged rather
than guessing or removing several elements.

`KEY` is the dot-path to the array. If the document root is itself the keyed
array, pass an explicit empty argument: `afdata add FILE "" SLUG ...` or
`afdata remove FILE "" SLUG ...`.

## Addressing an array element by content

A non-empty ASCII-decimal path segment is always an index (and overflow is an
error, never a slug fallback). Anything else has to be declared, because
nothing in `identities.me.email` tells afdata which field of an `identities`
element `me` is supposed to match:

```bash
afdata value config.json identities.me.email --slug-field identity
```

`--slug-field` works on every **addressed** read form (`get`, `value`, `values`,
`keys`, `paths`) and on every write (`set`, `unset`, `add`, `remove`), so an
address that reads also writes. Whole-document `get` and root-level
`keys`/`paths` reject it because no address exists for the option to affect.
Without it a non-numeric segment is `document_path_not_found`. A slug that
matches nothing is `document_slug_not_found` — never a scan for something close,
and on `unset` an error rather than a silent "nothing to remove". If an external
document contains the same exact slug more than once, the read *or write* is
`document_ambiguous_match` with candidate indices; afdata never takes the first,
so no edit lands on a guessed element.
Slug text uses the normal path grammar: for example, `case.add` is addressed as
`items.case\.add`. An ASCII-decimal segment is always an array index, even when
the declared field contains the same digits.

Markdown needs no declaration: its value is a tree afdata built, so the format
states its own rule (see below). Passing `--slug-field` anyway overrides that
rule and switches to exact matching.

## Markdown

A `.md` file has three valid readings, so none of them is ever auto-detected —
name the one you want:

- `--input-format toml-frontmatter` edits a `+++` metadata block;
- `--input-format yaml-frontmatter` edits a `---` metadata block;
- `--input-format markdown` reads the body's CommonMark blocks.

The two frontmatter readings edit only the metadata block and leave the body
byte-for-byte unchanged.

`markdown` is **read-only**, and a mutating verb does not accept the value at
all: `set`/`unset`/`add`/`remove` reject `--input-format markdown` while parsing
argv (`cli_invalid_argument_value`, exit 2). `lint` rejects it too — it judges
field names against the naming convention, and these names are afdata's own.

A heading owns everything under it until the next heading of its own level or
shallower, so the document reads as a tree of sections. Anything before the
first heading is `preamble`.

```
preamble.blocks    everything before the first heading, in source order
preamble.paragraph its CommonMark paragraph blocks (skips non-paragraph blocks)
preamble.blockquote its blockquote blocks only
h1.0.text          the first level-1 heading's text
h1.0.level         1
h1.0.source_start_line / source_end_line the whole section's inclusive range
h1.0.heading_end_line the inclusive end of the heading itself
h1.0.paragraph     that section's paragraphs only
h1.0.blockquote    that section's blockquotes only
h1.0.blocks        that section's blocks, all kinds, in source order
h1.0.h2.0          its first level-2 subsection
```

A leading `+++`/`---` frontmatter block is recognised as
`{type: "frontmatter", format: "toml"|"yaml", text: ""}` rather than read as
prose. Its raw metadata is deliberately not copied into this structural view:
use `--input-format toml-frontmatter` or `yaml-frontmatter` to read its fields
with normal secret redaction.

Each block carries `type`, `text`, and 1-based inclusive `source_start_line` /
`source_end_line`, plus `language` on a code block, `format` on a
frontmatter block, and `ordered` on a list. Prose is
flattened (emphasis unwrapped, links reduced to text, wrapped lines joined);
code and HTML remain verbatim, while frontmatter text is empty. Block kinds:
`paragraph`, `code`, `blockquote`, `list`, `html`, `rule`, `frontmatter`.

Section `source_start_line` / `source_end_line` cover the heading and everything
the section owns, including child sections. The heading begins at the same
`source_start_line`; `heading_end_line` marks its inclusive end and therefore
includes both lines of a setext heading. These ranges use source lines rather
than byte offsets so Bash, Python, and JavaScript consumers can splice complete
blocks safely for UTF-8 under LF and CRLF input. A file containing a lone CR is
refused, as described below. afdata still does no Markdown-to-Markdown
serialization.

**Address by content, not position.** A non-numeric segment matches an
element's `text` as a case-insensitive substring, so a memorable word out of a
heading is a far steadier address than an index that moves:

```bash
afdata value README.md h1.0.text --input-format markdown              # the title
afdata value README.md h1.0.paragraph.0.text --input-format markdown  # the synopsis
afdata value README.md h1.0.h2.suffixes.text --input-format markdown  # a section by name
```

Matching several elements is `document_ambiguous_match`, which reports the
candidate indices without copying document text into an error — pick a word
that separates. Matching none is `document_slug_not_found`.

**Line numbers are 1-based, inclusive, and count `\n`** — the same lines `sed`,
`awk`, `head`, `git diff`, and an editor count, so a range goes straight to
them:

```bash
end=$(afdata value README.md h1.0.paragraph.0.source_end_line --input-format markdown)
sed -n "1,${end}p" README.md      # the title and synopsis, exactly
```

A file containing a bare `\r` (classic pre-2002 Mac line endings) is refused
rather than numbered: CommonMark ends a line there and those tools do not, so
no single number would be right for both readers. LF and CRLF are unaffected —
both rules agree on them.

Apply your own layout convention on top; do not assume one is built in. "The
title is the H1" and "the synopsis is the first paragraph" are your project's
rules, so assert them. A README opening with a badge line puts that badge in
`preamble` and still has its title at `h1.0` — but a file whose first heading
is an `h2`, or which has no heading at all, has no `h1` key, and reading
`h1.0.text` fails loudly rather than inventing one.

Pure CommonMark only: no GFM tables, footnotes, task lists, or strikethrough
(table rows read as a paragraph, which is the specification's answer). Heading
sections nest, but blocks do not — a blockquote or list reports its flattened
text, not its inner structure.

`paragraph` means exactly what CommonMark parsed as a paragraph. Badge syntax
and GFM-looking table rows are therefore paragraphs, not magically skipped
decoration. If your layout says the first paragraph is a synopsis, enforce that
badge/table syntax does not occupy that position.

## Mutation safety and errors

Mutations are atomic and source-preserving. They refuse symlink targets and,
on Unix, multi-link hardlink targets. No partial file is observable, and every
failure before atomic installation leaves the original untouched. A
parent-directory fsync can fail after a complete replacement was installed;
that error means durability is unconfirmed, so reopen the path to determine
the committed state before retrying.

For a Rust library caller's first write, use
`DocumentFile::create_atomic(path, document, CreateOptions::new())`. It accepts
only an already-parsed `Document`, creates a private same-directory temporary
file, fsyncs it, and defaults to no-clobber mode with Unix permissions `0o600`.
Choose `.unix_mode(...)` deliberately when different permissions are required,
and `.replace()` only when replacing an existing target is intended. The raw
atomic text writer is not public.

Branch on stable `error.code`, not message text. Common runtime codes include:

- `document_path_not_found`
- `document_invalid_path`
- `document_type_mismatch`
- `document_slug_not_found`
- `document_slug_exists`
- `document_ambiguous_match`
- `document_parse_failed`
- `document_source_refused`
- `document_format_unknown`
- `document_unsupported_operation`
- `document_invalid_argument`
- `document_target_exists`
- `document_too_large`
- `document_write_would_corrupt`
- `document_io_failed`
- `document_not_scalar`
- `document_not_container`
- `document_secret_redacted`

Four codes describe four different things that all read as "it did not parse",
and the difference is what to fix:

- `document_format_unknown` — the file's extension named no format, so nothing
  was parsed. Pass `--input-format`.
- `document_parse_failed` — a parser read the file and rejected it. The file is
  malformed; the message carries the format and position, never the text.
- `document_invalid_path` — the *address* is malformed (a bad escape, a trailing
  `\`, a bare `*`, an index beyond the platform's range). The file is fine; fix
  the path.
- `document_source_refused` — the file parses, and afdata declines to answer
  about it anyway because it cannot do so honestly. Markdown containing a bare
  `\r` is the one case: CommonMark ends a line there and `sed`/`awk`/`git` do
  not, so no single `source_start_line` would be right for both. The message
  names the way out; convert the file to LF or CRLF.

`document_unsupported_operation` means the format cannot express the edit —
a read-only backend, an escaped or keyed-list route through YAML, or a YAML
path that resolves through an alias (editing it would rewrite the anchor, a
different key than the one named). A TOML array of tables is refused because
the editor has no explicit element-identity policy. It is not retryable as-is;
change the verb, the path, or the format.

`document_target_exists` is the expected no-clobber result from
`create_atomic`; decide explicitly whether the existing file wins or a
subsequent call may opt into replacement.

`document_too_large` is a capped read finding more bytes than the caller
allowed. It is separate from `document_io_failed` so a size budget can be
enforced without matching on a message: missing, unreadable, and not-a-regular
-file stay `document_io_failed`, and only the cap reports this. The file was not
parsed; raise the cap or refuse the input.

`create_atomic` also refuses a document whose format disagrees with the path
(`document_unsupported_operation`), because installing it would produce a file
the next `open` rejects. Replacing an existing file keeps that file's
permissions unless a mode is named explicitly.

Writing a whole sequence or mapping into YAML *is* supported, including inside
Markdown frontmatter, and keeps every byte outside that value: surrounding
keys, their comments, key order, quote styles, and the body. What it cannot
keep are comments and blank lines *inside* the collection being replaced —
those go with the value they annotated. Editing a single element (`tags.1`) or
appending one leaves its neighbours, and their comments, untouched.

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
