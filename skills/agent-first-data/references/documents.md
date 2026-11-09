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
- [Building JSON in a shell](#building-json-in-a-shell)
- [Keyed lists](#keyed-lists)
- [Addressing an array element by content](#addressing-an-array-element-by-content)
- [Markdown](#markdown)
- [Property lists](#property-lists)
- [Mutation safety and errors](#mutation-safety-and-errors)
- [Exit criteria](#exit-criteria)

## Select the command

| Need | Command |
|---|---|
| Whole document or subtree as an AFDATA record | `get FILE [KEY]` |
| One raw scalar for shell use | `value FILE KEY` |
| Whole document or subtree as bare compact JSON | `json FILE` or `json FILE KEY [--default JSON]` |
| Whether a key exists | `has FILE KEY` |
| How many direct children a container has | `count FILE [KEY [--missing-ok]]` |
| Build a JSON object or array from arguments | `object [FIELD=VALUE]... [--json FIELD=JSON]...`, `array [VALUE]... [--json]` |
| Many scalars from a single parse | `values FILE KEY... [--missing-ok] [--null]` |
| Immediate children as reusable dot-paths | `paths FILE [KEY]` |
| Immediate children as literal external names | `keys FILE [KEY]` |
| Write one value | `set FILE KEY VALUE` |
| Insert into a slug-keyed object array | `add FILE KEY SLUG FIELD=VALUE...` |
| Remove a slug-keyed item | `remove FILE KEY SLUG` |
| Remove one dot-path | `unset FILE KEY` |

The first positional is always the file. Reads accept `-` for explicit stdin,
parsed as JSON unless `--input-format` names another format; mutations reject
stdin.

## Read safely

`get` always redacts a directly targeted `_secret` leaf to `***`. Pass
`--secret-name FIELD` for every exact legacy secret name.
An absent (`null`) secret stays null. Targeting an `_url` field, or a child of
an `_url` collection, retains the same URL scrubbing as whole-document reads.

`value` writes a scalar with no envelope and no trailing newline: the output is
the exact bytes, so two calls in a terminal run together. Use `values` when you
want one value per line. It rejects containers. On failure its stdout is empty and the structured error goes to
stderr, so shell substitution cannot capture an error as data:

```bash
port="$(afdata value config.toml server.port --default 8080)"
```

Secret leaves and URL values containing credentials require the explicit,
auditable `--reveal-secret` option. Credential-free URLs remain readable.

Build a path from free text (URLs, file paths, versions) with
`afdata path -- SEGMENT...`, never with a hand-written replacement: it escapes
`\`, `.` and `*` exactly as the path parser reads them back, and prints the
path with no trailing newline. Assign it first, then read:
`p="$(afdata path -- "$url")"; afdata value urls.json -- "$p"`. Zero segments is a usage
error (exit 2); an empty segment is `document_invalid_argument`.

`get` preserves JSON types and structure inside its result envelope. `value`
deliberately stringifies one scalar into raw shell bytes. Use `get` (or the
typed library API) for read-modify-write work; use `value` only when the next
consumer explicitly wants a scalar string. For a CLI round trip, take the
typed value from `get` and write it with the matching `set --value-type`
(`json` for a container); never feed `value` output back when type matters.

CLI document file reads and edits (and `lint`) accept regular files up to
16 MiB by default. When a `document_too_large` error names a file you mean to
read, pass `--max-file-bytes BYTES`; stdin (`-`) is not capped.

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

### Existence and null

`has` answers "is the key there"; `--default` and `--missing-ok` answer "is there a
readable value".

`has` prints `true` or `false` (no trailing newline) and exits 0 either way:
`present="$(afdata has config.toml server.port)"; [ "$present" = true ]`. Assign
first: inside `[ "$(...)" = true ]` an unreadable file would read as "absent"
and the script would go on. A key holding `null`
exists (`has` prints `true`), but it is not a readable value: `count`, `keys`,
`paths` and a `*` container read it as empty only with `--missing-ok`, and
without it fail with `document_null_value`. A missing segment, or a path that runs
through a scalar, is `false`.
Only an unreadable or unparseable document, or a malformed path, is an error
(empty stdout, exit 1). Do not test existence with `get ... || ...`, which
reads every failure as "absent".

### Defaults

`--default VALUE` applies when the path leads to no value (absent, running through
a scalar such as `b.version` when `b` is a string, or null); an empty string is a
real value. Without `--default` that same path is `document_type_mismatch`.

`json` writes the value at KEY (the whole document without KEY) as one line of
compact JSON and a newline, with no envelope: the shell equivalent of
`jq '.key'`, and it converts TOML, YAML and the other readable formats to JSON.
Redaction matches `get`; `--reveal-secret` lifts it. Reach for `json` when the
next consumer wants a document or subtree as JSON, `value` when it wants a
scalar's bytes. A missing KEY is `document_path_not_found`; `--default JSON`
prints the given JSON (re-serialized compact) for a missing, scalar-blocked or null KEY instead,
so a missing subtree can read as `[]` in one call. An invalid default is
`json_parse_failed`, and an ambiguous match never falls back. `value` never
prints a container, so the string `{}` and the object `{}` stay distinguishable.

### Counting

`count` prints how many direct children an object or array has, as a decimal
integer with no trailing newline: `n="$(afdata count config.toml servers)"`. It
does not recurse, a scalar is `document_not_container`, and a missing KEY is
`document_path_not_found` unless `--missing-ok`, which answers `0`. Without
KEY it counts the document root: `n="$(cmd | afdata count -)"`.

### Iteration

`values` takes `--missing-ok` when a `*` container (or any segment before it)
may be absent: that KEY then contributes no records. It never forgives a
missing leaf; that stays with `--default`. `--null` separates records with NUL,
so a value holding a newline stays one record.

`paths` emits root-relative grammar-escaped paths that feed back into every read
and write verb — `get`/`value`/`set`/`unset` — unchanged. Pass the same
`--slug-field` you listed with, or a content-addressed path arrives at a verb
that cannot resolve it. `keys` emits raw immediate names for external tools.
They differ when a key contains a dot or space.

Consume either line stream with `while IFS= read -r`; never use command
substitution in a `for` loop. Path grammar escapes a literal dot as `\.` and a
literal backslash as `\\`, and a literal star as `\*`. Unknown escapes are
errors. Empty segments address empty-string keys (`packages..version`); an
empty-string key at the document root has no standalone path spelling.
`keys` and `paths` refuse secret subtrees. Names containing a newline require
`--null`; NUL-containing names cannot be emitted by either framing. An
unrepresentable name returns `document_unframeable_name` with empty stdout.

#### Iterating an array

Prefer one `values` call per field with a `*` segment. `*` expands to every
child in index order, so each field comes back as a column and line N is
element N. With `--default ''` a missing or null field is an empty line:

```bash
afdata values Cargo.toml 'bin.*.name' 'bin.*.path' --default ''
```

With several patterns, all lines of the first pattern come first, then all of
the second; they are not interleaved per element.

Never assign a column to a variable and feed the loop with `<<<`. `$(...)`
strips every trailing newline, so trailing empty values disappear, and `<<<`
adds one back, so an empty result still runs the loop once. Columns of
different length, or a phantom empty element, arrive with exit 0.

In a script that sources the kit, read each column into a Bash array with
`afdata_into` and loop over the index. Entries are NUL-framed, so empty values
and values holding a newline stay intact, and an empty array runs zero times:

```bash
afdata_into names values config.json 'items.*.name'
afdata_into emails values config.json 'items.*.email' --default ''
i=0
while [ "$i" -lt "${#names[@]}" ]; do
  printf '%s %s\n' "${names[$i]}" "${emails[$i]}"
  i=$((i + 1))
done
```

Without the kit, write each column to a file and redirect the file into the
loop. `afdata values … > "$file"` is a plain command, so its failure reaches
`set -e`:

```bash
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
afdata values config.json 'items.*.name' > "$tmp/names"
afdata values config.json 'items.*.email' --default '' > "$tmp/emails"
while IFS= read -r name <&3 && IFS= read -r email <&4; do
  printf '%s %s\n' "$name" "$email"
done 3< "$tmp/names" 4< "$tmp/emails"
```

A value holding a newline cannot be one line: `values` fails with
`document_multiline_value` and names the path. Use `afdata_into`, or add
`--null` and read with `read -r -d ''`.

Boundaries. If an element is a scalar where the path needs to go deeper
(`items.*.name.first` with `name` a string), that is `document_type_mismatch`
without `--default`; with it, that element yields the default, the same as an
absent or `null` one. Only a pattern that ends on a collection (`items.*` where
an element is an object) is not covered: it is `document_not_scalar` either way.

Second choice, when the loop must stop early on a condition: take the indices
from `keys`, never from probing for a missing field, and read each element with
one `values` call. Data arriving on stdin can be read only once, so write it to
a file first:

```bash
afdata_mktemp file
cat > "$file"   # stdin is consumed once; keep it for every pass below
afdata_into indices keys "$file" identities
for index in ${indices[@]+"${indices[@]}"}; do
  afdata_into fields values "$file" "identities.$index.id" "identities.$index.email" --default ''
  printf '%s %s\n' "${fields[0]}" "${fields[1]}"
done
```

To find an element by one of its field values instead of walking all of them,
see [Addressing an array element by content](#addressing-an-array-element-by-content).

## Write exact types

A bare `set` value and each bare `FIELD=VALUE` is always a string. Use:

```bash
afdata set config.toml server.port 8080 --value-type number
afdata set config.toml feature.enabled true --value-type bool
afdata set config.json cache.value --value-type null
afdata set config.toml routes '[{"path":"/"}]' --value-type json
```

`--value-type null` takes no VALUE and is not supported by TOML.
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

For an existing array, `set FILE features.- c` appends one typed element while
preserving existing comments. On an array, `-` always names that append
position and never an element, even under `--slug-field` when an element's
slug is `-` (address that one by index): it is valid only as the last segment
of a `set` path, and `get`, `value`, `unset` or a mid-path `-` fail with
`document_invalid_path`. Object key `-` is ordinary; out-of-range numeric
indices still fail. A YAML flow sequence (`features: [a, b]`) is refused like
any non-empty YAML flow collection; rewrite it as a block sequence first.
Missing object parents are created, including YAML block mappings. No array
parents or holes are created.

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
`stdin` and `fd:<N>` store the bytes exactly, a trailing newline included, so
feed them with `printf %s`, not `echo`.

## Building JSON in a shell

Do not splice shell variables into a `printf '{"k":"%s"}'` template: a quote,
backslash or newline in the value yields invalid JSON. `object` and `array`
build the text from arguments and escape it. Each prints one line of compact
JSON with a trailing newline, so the result can go straight to `> file`, a
JSONL stream, or `afdata json -`. Both are raw-output verbs: no `--output` or
`--output-to`, and no redaction, because every byte came from your own argv.

`object` takes `FIELD=VALUE` strings (split at the first `=`; the key is
literal, a `.` is not a path) and `--json FIELD=JSON` for anything that is not
a string. Invalid JSON text is `json_parse_failed`; a repeated key is an error;
no fields prints `{}`.

```bash
# a config file
afdata object name="$name" root="$dir" --json port=8080 --json debug=false > config.json

# a result event payload
payload="$(afdata object path="$file" --json bytes="$size")"
afdata_result --json "$payload"

# a string array from the positional parameters
files="$(afdata array -- "$@")"
afdata object --json files="$files"
```

When a positional argument comes from a variable (`"$@"`, a file name, a URL, a
version), write `--` before it: flags may appear between positionals and apply
to all of them, so a value that happens to be `--json` or `-x` would otherwise
change the result or fail. `object`'s `FIELD=VALUE` begins with a literal field
name and needs no `--`.

`array` makes every VALUE a string; with `--json` every VALUE is parsed as JSON
text (do not mix the two). No elements prints `[]`. A `_secret` key is accepted
as given, the same as `set` with a bare VALUE, but argv is visible to other
processes: write secrets with `set --secret-from`.

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

### Enumerate first, then address by name

`--slug-field` alone only declares how a slug segment on the path is matched;
`keys` and `paths` on an array still list indices. Add `--slugs` (accepted only
together with `--slug-field`) and `keys` lists each element's slug instead,
while `paths` prints the escaped path that `value`/`get` accept back (with the
same `--slug-field`). `--slugs` is all-or-nothing: an element without a string
in that field is `document_type_mismatch` (the message names the index), a
repeated slug is `document_ambiguous_match` (with the indices), and `paths` on
an empty or all-digit slug is `document_unaddressable_key` (`keys` lists it);
stdout stays empty in every case. Predicates stay in the shell:

```bash
afdata_into listed paths runtimes.json runtimes --slug-field name --slugs
for path in ${listed[@]+"${listed[@]}"}; do
  case "$path" in runtimes.iOS\ 27*) ;; *) continue ;; esac
  available="$(afdata value runtimes.json --slug-field name -- "$path.isAvailable")"
  [ "$available" = true ] && echo "$path"
done
```

Without `--slugs`, `keys`/`paths` list indices, and Markdown's own array
rule does not switch them to slugs.

## Markdown

A `.md` file has several valid readings, so none of them is ever auto-detected —
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
frontmatter block, and `ordered` on a list. Prose `text` is flattened plain
text: emphasis unwrapped, code spans reduced to their content, links to their
text, images to their alt text, inline HTML tags dropped, and line breaks
joined. Code and HTML blocks remain verbatim, while frontmatter text is empty.
Block kinds: `paragraph`, `code`, `blockquote`, `list`, `item`, `html`, `rule`,
`frontmatter`.

What flattening drops is kept in `inlines`, on every paragraph, heading, and
section: an ordered tree of `text`, `code`, `html`, `line_break`, and
`emphasis`/`strong`/`link`, which nest their own `inlines`. `link` and `image`
carry `destination_url`; an image's alt text is its flattened `text`. Each
node has a `text`, so a word addresses a link too:

```bash
afdata value README.md 'h1.0.h2.bash.paragraph.kit guide.inlines.guide.destination_url' --input-format markdown
```

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
sections nest by level; a blockquote, a list, and each list `item` also report
their child blocks under `blocks`, so walk `blocks` recursively to find every
`code` fence, including one inside a list item.

`paragraph` means exactly what CommonMark parsed as a paragraph. Badge syntax
and GFM-looking table rows are therefore paragraphs, not magically skipped
decoration. If your layout says the first paragraph is a synopsis, enforce that
badge/table syntax does not occupy that position.

## Property lists

`.plist` files are detected from the extension; `-` reads stdin with
`--input-format plist`. The format is **read-only**: `set`/`unset`/`add`/`remove` reject
`--input-format plist` while parsing argv (`cli_invalid_argument_value`, exit 2);
`lint` reads it. A write verb on a plist detected by extension fails at runtime
with `document_unsupported_operation`. Values map one to one:

| plist | value |
|---|---|
| `dict` | object |
| `array` | array |
| `string` | string |
| `integer` | integer |
| `real` | float |
| `true` / `false` | bool |
| `date` | RFC 3339 string in UTC, e.g. `2026-11-08T03:00:00Z` |
| `data` | standard base64 string |

XML and binary property lists (`bplist` magic) are both read, with the same
value mapping. A binary plist read as another format fails with
`document_parse_failed` and a message pointing at `--input-format plist`.

## Mutation safety and errors

Mutations are atomic and source-preserving. They refuse symlink targets and,
on Unix, multi-link hardlink targets. No partial file is observable, and every
failure before atomic installation leaves the original untouched. A
parent-directory fsync can fail after a complete replacement was installed;
that error means durability is unconfirmed, so reopen the path to determine
the committed state before retrying. Saves are last-writer-wins: if two handles
edit the same file, the later save replaces the earlier one. Callers must
serialize concurrent edits themselves.

Rust `document::write_atomic(path, bytes)` exposes the same durable atomic file
replacement primitive: existing regular-file permissions survive, new files
start owner-only on Unix (Windows inherits the directory ACL), and
symlinks/special files are refused. Unlike the document mutations it does not
check hard links, so a hard-linked target is split from its other links; write
through `DocumentFile` when that must be refused. Parent directories
must exist; inspect `AtomicError::commit_uncertain()` after a directory-sync
failure because installation has already happened.

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
- `document_not_scalar` — `value` was pointed at a collection; list its
  children with `keys` or read it whole with `get`.
- `document_not_container` — `keys`/`paths` was pointed at a scalar; read it
  with `value`.
- `document_secret_redacted`
- `document_multiline_value` — `values` met a value holding a newline; read that
  path with `value`, or use `--null`.
- `document_unframeable_value` — `values --null` met a value holding a NUL byte;
  read that path with `value`.
- `document_unaddressable_key` — `paths` would print a path that cannot be read
  back (an empty or all-digit slug, or an empty-string key at the root); use
  `keys`.
- `document_empty` — the file or stdin was empty, so there was nothing to parse.

These codes describe different things that all read as "it did not parse",
and the difference is what to fix:

- `document_format_unknown` — the file's extension named no format, so nothing
  was parsed. Pass `--input-format`.
- `document_parse_failed` — a parser read the file and rejected it. The file is
  malformed; the message carries the format and position, never the text.
- `document_invalid_path` — the *address* is malformed (a bad escape, a trailing
  `\`, a bare `*`, an index beyond the platform's range). The file is fine; fix
  the path.
- `document_source_refused` — the file parses, and afdata declines to answer
  about it anyway. Markdown containing a bare `\r` is one case: CommonMark
  ends a line there and `sed`/`awk`/`git` do not, so no single
  `source_start_line` would be right for both; convert the file to LF or CRLF.
  Markdown blocks nested more than 128 levels deep is the other.

`document_unsupported_operation` means the format cannot express the edit —
a read-only backend, an escaped or keyed-list route through YAML, a YAML
path that resolves through an alias (editing it would rewrite the anchor, a
different key than the one named), or an INI name or value that would not read
back unchanged (INI has no quoting for a newline, edge whitespace, or a
delimiter in a name). A TOML array of tables is refused because the editor has
no explicit element-identity policy. It is not retryable as-is;
change the verb, the path, or the format.

`document_target_exists` is the expected no-clobber result from
`create_atomic`; decide explicitly whether the existing file wins or a
subsequent call may opt into replacement.

`document_too_large` is a capped read finding more bytes than the caller
allowed. It is separate from `document_io_failed` so a size budget can be
enforced without matching on a message: missing, unreadable, and not-a-regular
-file stay `document_io_failed`, and only the cap reports this. The file was not
parsed. Rust callers choose an explicit cap; CLI callers raise the 16 MiB
default with `--max-file-bytes`.

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
errors exit 1. A `cli_*` code (also exit 2) means the arguments did not parse
into a registered command shape — an unknown flag, a missing positional — and
nothing was read. `document_usage_error` means the shape parsed but a value is
unusable for the command: `-` given to a mutation, an invalid
`--max-file-bytes`, a missing `--secret-from` source, or a bare value that would
overwrite a non-string without `--value-type`. A successful mutation result
includes the path written.

## Exit criteria

1. The chosen format and dot-path target are explicit.
2. Reads cannot leak a secret.
3. Secret writes avoid argv.
4. Value type is explicit whenever it is not a string.
5. Unrelated source text remains unchanged.
6. The result path or stable error code is checked.