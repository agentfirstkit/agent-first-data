//! Markdown mode: read a CommonMark file as a tree of heading sections.
//! Read-only — Markdown prose is never written back.
//!
//! A heading owns everything under it until the next heading of its own level
//! or shallower, so the document becomes a tree. Each section reports its
//! heading `text` and `level`, its own `blocks` in source order, the
//! `paragraph` subset of those, and its child sections under `h2`/`h3`/…
//! Anything before the first heading is `preamble`, which carries the same
//! `blocks`/`paragraph` pair.
//!
//! ```text
//! preamble.blocks.0   → { type: "frontmatter", format: "toml", text: "" }
//! preamble.paragraph.0 → { type: "paragraph", text: "CI" }       (a badge line)
//! h1.0.text           → "Real"
//! h1.0.paragraph.0    → { type: "paragraph", text: "The lead." }
//! h1.0.blocks.1       → { type: "code", language: "bash", text: "…" }
//! h1.0.h2.0.text      → "A quick look"
//! ```
//!
//! Segments are matched by content as well as by index (see
//! [`Format::array_rule`](crate::document::Format::array_rule)): `h2.look`
//! finds the section headed "A quick look", and matching several sections is an
//! error rather than a guess. That is what makes an address survive editing —
//! `h2.3` moves the moment a section is inserted above it, and a top-level
//! index moves the moment a badge line appears above the title.
//!
//! Sections are the reason the tree is not flat. A heading level is the one
//! piece of hierarchy CommonMark states outright, and folding on it costs
//! nothing while giving every address a stable frame: content is addressed
//! relative to the heading it lives under, not to the top of the file.
//!
//! What this backend owns is the *specification* layer — block identification,
//! block boundaries, and the heading nesting that follows from levels — and
//! nothing above it. Where a document's title lives, whether the first
//! paragraph is a synopsis, which blockquote carries a prompt: those are one
//! project's layout conventions, and they stay with the caller that holds them.
//! The distinction matters because block boundaries are precisely the part a
//! hand-rolled line scanner gets wrong: setext headings, the seven HTML-block
//! start conditions, four-space indented code, and the rule that an ATX heading
//! interrupts a paragraph are each a rule a `^# ` regex does not have.
//!
//! Detection is never automatic: a `.md` path resolves to no format on its own
//! and the caller must ask for `--input-format markdown`. The same file is
//! legitimately readable as `yaml-frontmatter`, and a reader that guesses
//! between two valid readings of one file is the shape-guessing AFDATA avoids.
//!
//! Deliberately out of scope, so that what this does return is exact:
//!
//! - **No dialects.** No GFM tables, footnotes, task lists, or strikethrough.
//!   A table's rows parse as a paragraph, which is the specification's answer,
//!   not a defect. (A leading `+++`/`---` metadata block *is* recognised, as
//!   its own block kind — see [`options`] for why that is not a dialect.)
//! - **No parsing or copying of frontmatter fields.** It is reported as one
//!   block with `format: "toml"|"yaml"` and empty `text`;
//!   `--input-format toml-frontmatter` is the reading that turns it into
//!   values. Omitting the raw metadata also prevents this structural reading
//!   from becoming a way around field-name-based secret redaction.
//! - **No recursion into a block's children.** A blockquote or list reports
//!   flattened `text`; its inner block structure is not exposed. (Heading
//!   sections *are* nested — that hierarchy comes from the level, not from
//!   walking inside a block.)
//! - **No byte offsets or columns.** Every block does carry 1-based inclusive
//!   `source_start_line` / `source_end_line`. Every section carries its whole
//!   source range plus `heading_end_line` for the
//!   heading alone. These are enough to splice whole Markdown blocks without
//!   making every consumer implement UTF-8 byte indexing.
//! - **No Markdown-to-Markdown transformation.** Rewriting a document in place
//!   needs source-preserving serialization, which is a separate design, not a
//!   rider on a reader.

use std::{collections::BTreeMap, ops::Range};

use pulldown_cmark::{CodeBlockKind, Event, MetadataBlockKind, Options, Parser, Tag};

use crate::document::{DocumentError, DocumentResult, Value};

/// Parse `content` as CommonMark into the section tree described above.
///
/// Never fails on content: CommonMark has no invalid-input production — every
/// UTF-8 string is some sequence of blocks — so the result is `Ok`, including
/// for an empty string (which yields an empty `preamble` and no sections).
pub fn load(content: &str) -> DocumentResult<Value> {
    reject_lone_carriage_return(content)?;
    let events: Vec<SpannedEvent<'_>> = Parser::new_ext(content, options())
        .into_offset_iter()
        .collect();
    let lines = LineIndex::new(content);
    let mut cursor = 0;
    Ok(fold_sections(read_blocks(&events, &lines, &mut cursor)))
}

type SpannedEvent<'a> = (Event<'a>, Range<usize>);

/// Byte offsets of logical line starts, used only inside the parser boundary.
///
/// Public consumers see line numbers, not byte offsets. LF and CRLF input use
/// the same `\n`-based numbering as common line-oriented tools; a lone CR is
/// rejected before this index is constructed.
struct LineIndex<'a> {
    content: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    /// Index the line starts of `content`, counting a line as everything up to
    /// the next `\n`.
    ///
    /// Not CommonMark's rule, deliberately. CommonMark also treats a bare `\r`
    /// as a line ending, and these numbers exist to be handed to something
    /// else — `sed -n '5,10p'`, `head -n`, an editor jump, a `git diff` — all
    /// of which split on `\n` alone. A number that only afdata can interpret
    /// is worse than no number: it looks usable and silently points one line
    /// off. CRLF agrees with both rules, so the two definitions differ only
    /// for a bare `\r`, which [`reject_lone_carriage_return`] refuses outright
    /// rather than reporting a line number nothing else can act on.
    fn new(content: &'a str) -> Self {
        let mut starts = vec![0];
        for (offset, byte) in content.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(offset + 1);
            }
        }
        Self { content, starts }
    }

    fn number_at(&self, offset: usize) -> i64 {
        let line = self.starts.partition_point(|start| *start <= offset);
        i64::try_from(line).unwrap_or(i64::MAX)
    }

    /// The 1-based inclusive line range a source span covers.
    ///
    /// The span's trailing whitespace is dropped first. Most block kinds end
    /// at their own last character, but pulldown-cmark runs a list's span on
    /// to where the next block begins, so it carries the blank lines that
    /// separate them. A range is a splice instruction — a consumer cutting one
    /// out must not take the separator with it, or the blocks on either side
    /// merge (a lead paragraph absorbed into the setext heading below it).
    fn range(&self, source: &Range<usize>) -> SourceLines {
        let content_end = self
            .content
            .get(..source.end)
            .map_or(source.end, |head| head.trim_end().len())
            .max(source.start);
        let end_offset = content_end.saturating_sub(1).max(source.start);
        SourceLines {
            start: self.number_at(source.start),
            end: self.number_at(end_offset),
        }
    }
}

#[derive(Clone, Copy)]
struct SourceLines {
    start: i64,
    end: i64,
}

/// Refuse a document containing a bare `\r` — a carriage return not followed
/// by a newline.
///
/// This is the single case where CommonMark's line rule and every other tool's
/// disagree: CommonMark ends a line there, `sed`/`awk`/`wc`/`head`/`git` do
/// not. A reported line number is only useful if the thing the caller hands it
/// to counts the same way, and for such a file no single number can satisfy
/// both — two blocks would share one line, and a splice by that number would
/// cut the wrong text with nothing to notice it by.
///
/// So the file is refused instead of answered wrongly. It is the classic Mac
/// (pre-2002) line ending; CRLF and LF, which every current tool produces, are
/// unaffected because both rules agree on them.
fn reject_lone_carriage_return(content: &str) -> DocumentResult<()> {
    let bytes = content.as_bytes();
    for (offset, byte) in bytes.iter().enumerate() {
        if *byte == b'\r' && bytes.get(offset + 1) != Some(&b'\n') {
            return Err(DocumentError::SourceRefused {
                format: "Markdown".to_string(),
                detail: format!(
                    "a bare carriage return at byte {offset} ends a line for CommonMark but not \
                     for the tools these line numbers are meant to feed; convert the file to LF \
                     or CRLF line endings"
                ),
            });
        }
    }
    Ok(())
}

/// Pure CommonMark, plus the two metadata-block rules and nothing else.
///
/// The extensions that stay off are *dialects* — GFM tables, footnotes, task
/// lists, strikethrough each re-read text CommonMark already assigns a meaning
/// to, so enabling one is a guess about which flavour a file was written in.
///
/// A leading `+++`/`---` block is not that. afdata already treats frontmatter
/// as a first-class thing to read — `Format::TomlFrontmatter` and
/// `Format::YamlFrontmatter` exist — so a Markdown reader that took the same
/// bytes for prose would contradict the crate's own model, and did: a Zola
/// `_index.md` arrived with its `title = "…"` and `[extra]` lines flattened
/// into two paragraphs of the body. The block is only recognised at the very
/// start of the file, so it cannot reinterpret a `---` anywhere else (a setext
/// underline or a thematic break stays what it is).
fn options() -> Options {
    Options::ENABLE_YAML_STYLE_METADATA_BLOCKS | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
}

/// One heading and everything it owns, while the tree is still being built.
struct Section {
    level: i64,
    text: String,
    heading_lines: SourceLines,
    blocks: Vec<Value>,
    children: Vec<Section>,
}

/// Fold a flat block sequence into the section tree.
///
/// A heading closes every open section at its own level or deeper, then opens
/// its own — the standard reading of heading levels, and the only hierarchy
/// CommonMark itself states. A level may be skipped (an `h3` directly under an
/// `h1`); the `h3` simply becomes a child of whatever section is open, since
/// there is no `h2` to hold it.
fn fold_sections(blocks: Vec<Value>) -> Value {
    let mut preamble = Vec::new();
    let mut roots: Vec<Section> = Vec::new();
    let mut open: Vec<Section> = Vec::new();

    for block in blocks {
        let heading_level = (block.get("type").and_then(Value::as_str) == Some("heading"))
            .then(|| block.get("level").and_then(Value::as_integer))
            .flatten();
        match heading_level {
            Some(level) => {
                while open.last().is_some_and(|section| section.level >= level) {
                    close_section(&mut open, &mut roots);
                }
                open.push(Section {
                    level,
                    text: block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    heading_lines: SourceLines {
                        start: block
                            .get("source_start_line")
                            .and_then(Value::as_integer)
                            .unwrap_or_default(),
                        end: block
                            .get("source_end_line")
                            .and_then(Value::as_integer)
                            .unwrap_or_default(),
                    },
                    blocks: Vec::new(),
                    children: Vec::new(),
                });
            }
            // Content before the first heading belongs to no section. It is
            // kept rather than dropped: "this file opens with a generated-file
            // comment" is a fact, and whether that is acceptable is the
            // caller's rule to apply, not this reader's to enforce by omission.
            None => match open.last_mut() {
                Some(section) => section.blocks.push(block),
                None => preamble.push(block),
            },
        }
    }
    while !open.is_empty() {
        close_section(&mut open, &mut roots);
    }

    // `preamble` carries the same two views a section does, so "the first
    // paragraph" is one address in both. As a bare array it was not: a `+++`
    // block or a badge line above the prose shifted every index, which is the
    // problem sections exist to remove.
    let mut root = BTreeMap::from([("preamble".to_string(), Value::Object(block_views(preamble)))]);
    insert_sections(&mut root, roots);
    Value::Object(root)
}

/// Pop the deepest open section and file it under its parent, or under the
/// document root when it has none.
fn close_section(open: &mut Vec<Section>, roots: &mut Vec<Section>) {
    let Some(done) = open.pop() else { return };
    match open.last_mut() {
        Some(parent) => parent.children.push(done),
        None => roots.push(done),
    }
}

/// File `sections` into `target` under `h1`/`h2`/… by their own level, each
/// group in source order.
fn insert_sections(target: &mut BTreeMap<String, Value>, sections: Vec<Section>) {
    for section in sections {
        let key = format!("h{}", section.level);
        let group = target
            .entry(key)
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(items) = group.as_array_mut() {
            items.push(section.into_value());
        }
    }
}

/// The two views every block container reports: `blocks` in source order, and
/// the `paragraph` subset of them.
///
/// The subset is duplicated out of `blocks` on purpose. Prose is what a
/// container is usually read for, and "the first paragraph" must not shift
/// because a frontmatter block, a code fence, or a thematic break happened to
/// land above it. Badge syntax and a GFM-looking pipe table are paragraphs
/// under the deliberately enabled CommonMark grammar, so they remain in this
/// view. `blocks` stays the one place that shows what the container actually
/// looks like, in order.
fn block_views(blocks: Vec<Value>) -> BTreeMap<String, Value> {
    let of_kind = |kind: &str| {
        Value::Array(
            blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some(kind))
                .cloned()
                .collect::<Vec<Value>>(),
        )
    };
    let paragraph = of_kind("paragraph");
    let blockquote = of_kind("blockquote");
    BTreeMap::from([
        ("paragraph".to_string(), paragraph),
        ("blockquote".to_string(), blockquote),
        ("blocks".to_string(), Value::Array(blocks)),
    ])
}

impl Section {
    fn into_value(self) -> Value {
        let source_end_line = self.source_end_line();
        let mut fields = block_views(self.blocks);
        fields.insert("level".to_string(), Value::Integer(self.level));
        fields.insert("text".to_string(), Value::String(self.text));
        fields.insert(
            "source_start_line".to_string(),
            Value::Integer(self.heading_lines.start),
        );
        fields.insert(
            "source_end_line".to_string(),
            Value::Integer(source_end_line),
        );
        fields.insert(
            "heading_end_line".to_string(),
            Value::Integer(self.heading_lines.end),
        );
        insert_sections(&mut fields, self.children);
        Value::Object(fields)
    }

    fn source_end_line(&self) -> i64 {
        self.blocks
            .iter()
            .filter_map(|block| block.get("source_end_line"))
            .filter_map(Value::as_integer)
            .chain(self.children.iter().map(Section::source_end_line))
            .max()
            .unwrap_or(self.heading_lines.end)
    }
}

/// Read sibling blocks until the enclosing container's `End` event, or the end
/// of the stream at top level. Leaves `cursor` *on* that `End`, for the caller
/// that opened the container to consume.
fn read_blocks(
    events: &[SpannedEvent<'_>],
    lines: &LineIndex<'_>,
    cursor: &mut usize,
) -> Vec<Value> {
    let mut blocks = Vec::new();
    while let Some((event, source)) = events.get(*cursor) {
        match event {
            Event::End(_) => break,
            Event::Rule => {
                let source = source.clone();
                *cursor += 1;
                blocks.push(block("rule", String::new(), &source, lines, vec![]));
            }
            Event::Start(tag) if !is_inline_tag(tag) => {
                let source = source.clone();
                *cursor += 1;
                blocks.extend(read_block(tag, &source, events, lines, cursor));
            }
            // Inline content standing where a block belongs: a tight list's
            // item holds its text directly, with no paragraph around it.
            // Reading it as an implicit paragraph is what makes a tight and a
            // loose list report the same items.
            _ => {
                let first = *cursor;
                let text = read_loose_inline(events, cursor);
                if !text.is_empty() {
                    let source = covered_range(events, first, *cursor);
                    blocks.push(block("paragraph", text, &source, lines, vec![]));
                }
            }
        }
    }
    blocks
}

/// Whether `tag` wraps text inside a block rather than opening one.
fn is_inline_tag(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

/// Read the one block opened by `tag`, consuming through its own `End`.
fn read_block(
    tag: &Tag<'_>,
    source: &Range<usize>,
    events: &[SpannedEvent<'_>],
    lines: &LineIndex<'_>,
    cursor: &mut usize,
) -> Option<Value> {
    match tag {
        Tag::Paragraph => Some(block(
            "paragraph",
            read_inline(events, cursor),
            source,
            lines,
            vec![],
        )),
        Tag::Heading { level, .. } => Some(block(
            "heading",
            read_inline(events, cursor),
            source,
            lines,
            vec![("level", Value::Integer(*level as i64))],
        )),
        Tag::CodeBlock(kind) => {
            // CommonMark's info string is everything after the opening fence;
            // in practice it is the language. An indented block has none.
            let info = match kind {
                CodeBlockKind::Fenced(info) => info.trim().to_string(),
                CodeBlockKind::Indented => String::new(),
            };
            Some(block(
                "code",
                read_verbatim(events, cursor),
                source,
                lines,
                // `language`, not CommonMark's `info`: a field must say what
                // it holds without its neighbours (spec/agent-first-data.md
                // rule 5, "Self-contained"). Sharing one name with the
                // frontmatter block below made `info: "toml"` mean either a
                // fence written in TOML or a `+++` delimiter, tellable apart
                // only by also reading `type`.
                vec![("language", Value::String(info))],
            ))
        }
        Tag::HtmlBlock => Some(block(
            "html",
            read_verbatim(events, cursor),
            source,
            lines,
            vec![],
        )),
        // The leading `+++`/`---` block, reported as its own kind without
        // copying its potentially secret-bearing source into `text`: which
        // fields it holds is TOML's or YAML's answer, and
        // `--input-format toml-frontmatter` is the reading that gives it.
        // `format` names the delimiter's dialect, matching the
        // `--input-format` token that reads its fields.
        Tag::MetadataBlock(kind) => {
            // Advance without collecting the metadata into a second String.
            // The source parser already borrows it, and this view deliberately
            // exposes neither a copy nor a flattened form.
            skip_subtree(events, cursor);
            Some(block(
                "frontmatter",
                String::new(),
                source,
                lines,
                vec![(
                    "format",
                    Value::String(
                        match kind {
                            MetadataBlockKind::PlusesStyle => "toml",
                            MetadataBlockKind::YamlStyle => "yaml",
                        }
                        .to_string(),
                    ),
                )],
            ))
        }
        Tag::BlockQuote(_) => Some(block(
            "blockquote",
            read_container(events, lines, cursor),
            source,
            lines,
            vec![],
        )),
        Tag::List(first_number) => Some(block(
            "list",
            read_container(events, lines, cursor),
            source,
            lines,
            vec![("ordered", Value::Bool(first_number.is_some()))],
        )),
        // An item exists only as a list's child, and the list flattens it away
        // into its own `text`, so this shape never reaches a caller. It is a
        // block here so that the generic container walk can reach an item's
        // own children at all.
        Tag::Item => Some(block(
            "item",
            read_container(events, lines, cursor),
            source,
            lines,
            vec![],
        )),
        // Tables, footnote definitions, definition lists, and metadata blocks
        // require options this backend does not enable, so none of them can
        // occur. Skipping the subtree keeps that a fact rather than a
        // half-formed block if the option set ever widens.
        _ => {
            skip_subtree(events, cursor);
            None
        }
    }
}

/// Flattened text of a container block — its child blocks' `text` joined by a
/// newline — consuming through the container's own `End`.
///
/// Block boundaries survive as newlines because they are structure; a wrapped
/// line inside one paragraph does not, because it is presentation (see
/// [`read_inline`]). Children that flatten to nothing, such as a thematic
/// break, contribute nothing rather than a blank line.
fn read_container(
    events: &[SpannedEvent<'_>],
    lines: &LineIndex<'_>,
    cursor: &mut usize,
) -> String {
    let children = read_blocks(events, lines, cursor);
    // `read_blocks` stops on the container's `End` rather than past it.
    *cursor += 1;
    children
        .iter()
        .filter_map(|child| child.get("text").and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Flattened plain text of a leaf block's inline content, consuming through
/// that block's own `End`.
///
/// Emphasis and strong are unwrapped, a code span keeps its content, a link
/// keeps its text and drops its URL, an image keeps its alt text, raw inline
/// HTML tags are dropped, and every line break inside the block — soft or hard
/// — becomes a single space. The result is one line of plain prose, which is
/// what a name, a synopsis, or a prompt is.
fn read_inline(events: &[SpannedEvent<'_>], cursor: &mut usize) -> String {
    let text = read_loose_inline(events, cursor);
    if matches!(events.get(*cursor), Some((Event::End(_), _))) {
        *cursor += 1;
    }
    text
}

/// [`read_inline`] without an owning block: stops *before* the next block
/// boundary instead of consuming a closing `End`. This is the form a tight
/// list item needs, whose text has no paragraph of its own to end.
fn read_loose_inline(events: &[SpannedEvent<'_>], cursor: &mut usize) -> String {
    let mut text = String::new();
    let mut depth = 0usize;
    while let Some((event, _)) = events.get(*cursor) {
        match event {
            Event::Start(tag) if depth == 0 && !is_inline_tag(tag) => break,
            Event::End(_) if depth == 0 => break,
            Event::Rule if depth == 0 => break,
            Event::Start(_) => {
                depth += 1;
                *cursor += 1;
            }
            Event::End(_) => {
                depth -= 1;
                *cursor += 1;
            }
            Event::Text(chunk) | Event::Code(chunk) => {
                text.push_str(chunk);
                *cursor += 1;
            }
            Event::SoftBreak | Event::HardBreak => {
                text.push(' ');
                *cursor += 1;
            }
            _ => *cursor += 1,
        }
    }
    text.trim().to_string()
}

/// Literal content of a code or HTML block, consuming through its `End`.
///
/// Nothing is flattened here — the content is not prose. The single trailing
/// newline every such block carries (the line break before its closing fence,
/// or ending its last line) is dropped; everything else is verbatim.
fn read_verbatim(events: &[SpannedEvent<'_>], cursor: &mut usize) -> String {
    let mut text = String::new();
    while let Some((event, _)) = events.get(*cursor) {
        *cursor += 1;
        match event {
            Event::End(_) => break,
            Event::Text(chunk) | Event::Html(chunk) => text.push_str(chunk),
            _ => {}
        }
    }
    let unterminated = text.strip_suffix('\n').unwrap_or(text.as_str());
    unterminated
        .strip_suffix('\r')
        .unwrap_or(unterminated)
        .to_string()
}

/// Consume an unhandled container's whole subtree, including its own `End`.
fn skip_subtree(events: &[SpannedEvent<'_>], cursor: &mut usize) {
    let mut depth = 0usize;
    while let Some((event, _)) = events.get(*cursor) {
        *cursor += 1;
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
}

/// The smallest source range covering events in `start..end`.
///
/// Top-level CommonMark blocks carry their full range on the opening event.
/// This fallback exists for inline text in tight containers, where there is no
/// paragraph `Start` event to lend us one.
fn covered_range(events: &[SpannedEvent<'_>], start: usize, end: usize) -> Range<usize> {
    let mut covered = events
        .get(start)
        .map(|(_, source)| source.clone())
        .unwrap_or(0..0);
    for (_, source) in events.get(start..end).unwrap_or_default() {
        covered.start = covered.start.min(source.start);
        covered.end = covered.end.max(source.end);
    }
    covered
}

/// Assemble one block: the `type`, flattened `text`, and 1-based inclusive
/// source-line range every block carries, plus kind-specific fields.
fn block(
    kind: &str,
    text: String,
    source: &Range<usize>,
    lines: &LineIndex<'_>,
    extra: Vec<(&str, Value)>,
) -> Value {
    let source = lines.range(source);
    let mut fields = BTreeMap::from([
        ("type".to_string(), Value::String(kind.to_string())),
        ("text".to_string(), Value::String(text)),
        (
            "source_start_line".to_string(),
            Value::Integer(source.start),
        ),
        ("source_end_line".to_string(), Value::Integer(source.end)),
    ]);
    for (name, value) in extra {
        fields.insert(name.to_string(), value);
    }
    Value::Object(fields)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]
    use super::*;
    use crate::document::{Addressing, Format, get_path};

    /// Read one address exactly as a caller would, through the same traversal
    /// and the same format rule the CLI uses.
    fn at(source: &str, path: &str) -> DocumentResult<Value> {
        get_path(
            &load(source).unwrap(),
            path,
            Addressing::INDEX_ONLY.with_array_rule(Format::Markdown.array_rule()),
        )
    }

    fn text(source: &str, path: &str) -> String {
        at(source, path)
            .unwrap_or_else(|error| panic!("{path}: {error}"))
            .as_str()
            .unwrap_or_else(|| panic!("{path} is not a string"))
            .to_string()
    }

    fn integer(source: &str, path: &str) -> i64 {
        at(source, path)
            .unwrap_or_else(|error| panic!("{path}: {error}"))
            .as_integer()
            .unwrap_or_else(|| panic!("{path} is not an integer"))
    }

    /// `(type, text)` of a block array, which is what a caller reads.
    fn shape(source: &str, path: &str) -> Vec<(String, String)> {
        at(source, path)
            .unwrap_or_else(|error| panic!("{path}: {error}"))
            .as_array()
            .unwrap_or_else(|| panic!("{path} is not an array"))
            .iter()
            .map(|block| {
                let field = |name: &str| {
                    block
                        .get(name)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                (field("type"), field("text"))
            })
            .collect()
    }

    fn types(source: &str, path: &str) -> Vec<String> {
        shape(source, path).into_iter().map(|(k, _)| k).collect()
    }

    // ---- the eight probes -------------------------------------------------
    //
    // These are the cases a hand-rolled line scanner answered wrong — all eight
    // of them. Each names the CommonMark rule it turns on, and each is the
    // classification a caller's layout policy then reads.

    #[test]
    fn probe_1_setext_heading_is_a_heading() {
        // An underlined title is a heading; `^# ` never sees it.
        let source = "Title\n=====\n\nThe lead.\n";
        assert_eq!(text(source, "h1.0.text"), "Title");
        assert_eq!(text(source, "h1.0.paragraph.0.text"), "The lead.");
    }

    #[test]
    fn probe_2_html_comment_before_the_heading_is_its_own_block() {
        // A generated-file banner is an HTML block. It lands in `preamble`,
        // where a caller can see it, and does not shift the title.
        let source = "<!-- generated -->\n\n# Real\n\nThe lead.\n";
        assert_eq!(
            shape(source, "preamble.blocks"),
            [("html".to_string(), "<!-- generated -->".to_string())]
        );
        assert_eq!(text(source, "h1.0.text"), "Real");
    }

    #[test]
    fn probe_3_badge_line_before_the_heading_is_a_paragraph() {
        // The most common README opening there is. A flat reading would make
        // this block 0 and publish `CI` as the project's name; here it is
        // preamble and the title is still `h1.0`.
        let source = "[![CI](a.svg)](b)\n\n# Real\n\nThe lead.\n";
        assert_eq!(
            shape(source, "preamble.blocks"),
            [("paragraph".to_string(), "CI".to_string())]
        );
        assert_eq!(text(source, "h1.0.text"), "Real");
        assert_eq!(text(source, "h1.0.paragraph.0.text"), "The lead.");
    }

    #[test]
    fn probe_4_fenced_code_in_the_lead_position_is_code() {
        // The section's first *block* is code, so `paragraph` is empty — a
        // caller asking for a synopsis gets a failure, not the code.
        let source = "# Real\n\n```bash\nafdata get x\n```\n";
        assert_eq!(
            shape(source, "h1.0.blocks"),
            [("code".to_string(), "afdata get x".to_string())]
        );
        assert_eq!(shape(source, "h1.0.paragraph"), []);
        assert_eq!(
            at(source, "h1.0.paragraph.0").unwrap_err().code(),
            "document_path_not_found"
        );
        assert_eq!(
            at(source, "h1.0.blocks.0.language").unwrap(),
            Value::String("bash".to_string())
        );
    }

    #[test]
    fn probe_5_leading_fence_swallows_the_heading_inside_it() {
        // `# Real` is code here, not a heading — so there is no section at all
        // and the whole file is preamble.
        let source = "```\n# Real\n```\n";
        assert_eq!(
            shape(source, "preamble.blocks"),
            [("code".to_string(), "# Real".to_string())]
        );
        assert_eq!(
            at(source, "h1.0").unwrap_err().code(),
            "document_path_not_found"
        );
    }

    #[test]
    fn probe_6_atx_heading_interrupts_a_paragraph() {
        // The rule that looks backwards and is not: a `# ` line ends the
        // paragraph it appears in rather than joining it. It also opens a
        // sibling section, so the lead stops there too.
        let source = "# Real\n\nThe lead.\n# looks like heading\nmore.\n";
        assert_eq!(text(source, "h1.0.text"), "Real");
        assert_eq!(
            shape(source, "h1.0.paragraph"),
            [("paragraph".to_string(), "The lead.".to_string())]
        );
        assert_eq!(text(source, "h1.1.text"), "looks like heading");
        assert_eq!(text(source, "h1.1.paragraph.0.text"), "more.");
    }

    #[test]
    fn probe_7_four_space_indent_is_code_not_a_heading() {
        // The worst of the eight: an indented `# Title` reads as a title to a
        // scanner that strips leading whitespace, and is code to CommonMark.
        let source = "    # Indented\n\nAfter.\n";
        assert_eq!(
            types(source, "preamble.blocks"),
            ["code".to_string(), "paragraph".to_string()]
        );
        assert_eq!(
            at(source, "h1.0").unwrap_err().code(),
            "document_path_not_found"
        );
    }

    #[test]
    fn probe_8_whole_paragraph_emphasis_is_unwrapped() {
        // Flattening gives this for free; a scanner needs a special case for
        // it, and still leaks the inner markers of the partial-emphasis form.
        assert_eq!(
            text("# Real\n\n**A bold tagline.**\n", "h1.0.paragraph.0.text"),
            "A bold tagline."
        );
        // Partial emphasis, a code span, and a link all flatten to plain text
        // too — the scanner leaked every one of these into published metadata.
        assert_eq!(
            text(
                "# T\n\nA **bold** `span` and a [link](https://example.com).\n",
                "h1.0.paragraph.0.text"
            ),
            "A bold span and a link."
        );
    }

    // ---- section folding --------------------------------------------------

    #[test]
    fn headings_nest_by_level() {
        let source = "# A\n\na.\n\n## B\n\nb.\n\n### C\n\nc.\n\n## D\n\nd.\n\n# E\n";
        assert_eq!(text(source, "h1.0.text"), "A");
        assert_eq!(text(source, "h1.0.paragraph.0.text"), "a.");
        assert_eq!(text(source, "h1.0.h2.0.text"), "B");
        assert_eq!(text(source, "h1.0.h2.0.h3.0.text"), "C");
        assert_eq!(text(source, "h1.0.h2.0.h3.0.paragraph.0.text"), "c.");
        // `D` closes `B` (same level) and `C` with it.
        assert_eq!(text(source, "h1.0.h2.1.text"), "D");
        assert_eq!(
            at(source, "h1.0.h2.1.h3.0").unwrap_err().code(),
            "document_path_not_found"
        );
        assert_eq!(text(source, "h1.1.text"), "E");
    }

    #[test]
    fn a_skipped_level_keeps_its_own_name() {
        // `### C` under `# A` with no `## ` between: C is A's child, filed by
        // its own level rather than promoted to one it does not have.
        let source = "# A\n\n### C\n\nc.\n";
        assert_eq!(text(source, "h1.0.h3.0.text"), "C");
        assert_eq!(
            at(source, "h1.0.h2.0").unwrap_err().code(),
            "document_path_not_found"
        );
    }

    #[test]
    fn a_document_opening_below_h1_has_no_h1() {
        // No invented level-1 wrapper: `h2` is simply where the sections are,
        // and a caller demanding `h1.0` fails loudly.
        let source = "## Only\n\ntext.\n";
        assert_eq!(text(source, "h2.0.text"), "Only");
        assert_eq!(
            at(source, "h1.0").unwrap_err().code(),
            "document_path_not_found"
        );
    }

    #[test]
    fn preamble_is_always_present_and_empty_for_a_clean_file() {
        assert_eq!(shape("# T\n\nlead\n", "preamble.blocks"), []);
        assert_eq!(shape("", "preamble.blocks"), []);
    }

    // ---- addressing by content --------------------------------------------

    #[test]
    fn a_section_is_addressable_by_a_word_of_its_heading() {
        let source = "# T\n\n## A Quick Look\n\ninside look.\n\n## Supported suffixes\n\ns.\n";
        // Case-insensitive substring: the memorable word, not the full text.
        assert_eq!(text(source, "h1.0.h2.look.text"), "A Quick Look");
        assert_eq!(
            text(source, "h1.0.h2.look.paragraph.0.text"),
            "inside look."
        );
        assert_eq!(text(source, "h1.0.h2.SUFFIX.text"), "Supported suffixes");
        // Content and index address the same section.
        assert_eq!(
            at(source, "h1.0.h2.look.text").unwrap(),
            at(source, "h1.0.h2.0.text").unwrap()
        );
        // Matching a body paragraph does not leak in: `h2` holds only headings,
        // which is the whole reason the tree is not flat.
        assert_eq!(
            at(source, "h1.0.h2.inside").unwrap_err().code(),
            "document_slug_not_found"
        );
    }

    #[test]
    fn an_empty_segment_is_not_an_address() {
        // `contains("")` is true for everything, so before this an empty
        // interpolated name resolved to whichever element happened to be
        // alone in the array — a confident wrong answer that only turned into
        // an error once a second element existed.
        let one = "# T\n\n## Only One\n\na\n";
        let two = "# T\n\n## A\n\na\n\n## B\n\nb\n";
        for source in [one, two] {
            assert_eq!(
                at(source, "h1.0.h2..text").unwrap_err().code(),
                "document_slug_not_found"
            );
        }
        // Named addressing still works.
        assert_eq!(text(two, "h1.0.h2.A.text"), "A");
    }

    #[test]
    fn a_word_matching_several_sections_is_refused() {
        let source = "# T\n\n## Quick look\n\na.\n\n## Another look\n\nb.\n";
        let error = at(source, "h1.0.h2.look").unwrap_err();
        assert_eq!(error.code(), "document_ambiguous_match");
        // The refusal reports structural indices, never matched document text.
        let message = error.to_string();
        assert!(message.contains("indices 0, 1"), "{message}");
        assert!(!message.contains("Quick look"), "{message}");
        assert!(!message.contains("Another look"), "{message}");
        // A word that separates them resolves.
        assert_eq!(text(source, "h1.0.h2.Another.text"), "Another look");
    }

    #[test]
    fn content_addressing_lowercases_unicode() {
        let source = "# T\n\n## Überblick\n\ninside.\n";
        assert_eq!(text(source, "h1.0.h2.ÜBER.text"), "Überblick");
    }

    #[test]
    fn the_ask_prompt_blockquote_is_addressable_by_its_opening_words() {
        // The real use: this blockquote sits at no fixed index in any README,
        // and its position moves with every edit above it.
        let source = "# T\n\nThe lead.\n\n> **Ask your agent:** \"Do the thing.\"\n";
        assert_eq!(
            text(source, "h1.0.blocks.Ask your agent.text"),
            "Ask your agent: \"Do the thing.\""
        );
    }

    // ---- inline flattening and block kinds --------------------------------

    #[test]
    fn wrapped_paragraph_joins_onto_one_line() {
        assert_eq!(
            text("# T\n\nLead line one\nline two.\n", "h1.0.paragraph.0.text"),
            "Lead line one line two."
        );
    }

    #[test]
    fn heading_level_is_reported_for_every_depth() {
        let source = "# a\n\n## b\n\n###### f\n";
        assert_eq!(at(source, "h1.0.level").unwrap(), Value::Integer(1));
        assert_eq!(at(source, "h1.0.h2.0.level").unwrap(), Value::Integer(2));
        assert_eq!(
            at(source, "h1.0.h2.0.h6.0.level").unwrap(),
            Value::Integer(6)
        );
    }

    #[test]
    fn blockquote_flattens_its_paragraphs() {
        // One wrapped paragraph is one line, as the prompt convention needs.
        assert_eq!(
            shape(
                "> **Ask your agent:** \"Wrapped across\n> two lines.\"\n",
                "preamble.blocks"
            ),
            [(
                "blockquote".to_string(),
                "Ask your agent: \"Wrapped across two lines.\"".to_string()
            )]
        );
        // Two paragraphs are two blocks, and that boundary is structure, so it
        // survives as a newline rather than dissolving into a space.
        assert_eq!(
            shape("> first\n>\n> second\n", "preamble.blocks"),
            [("blockquote".to_string(), "first\nsecond".to_string())]
        );
    }

    #[test]
    fn list_reports_its_items_and_whether_it_is_ordered() {
        let bullet = at("- one\n- two\n", "preamble.blocks.0").unwrap();
        assert_eq!(bullet.get("text").and_then(Value::as_str), Some("one\ntwo"));
        assert_eq!(bullet.get("ordered"), Some(&Value::Bool(false)));

        assert_eq!(
            at("1. one\n2. two\n", "preamble.blocks.0")
                .unwrap()
                .get("ordered"),
            Some(&Value::Bool(true))
        );

        // A loose list wraps each item in a paragraph and a tight one does
        // not. That is a rendering difference, not a content one, so the two
        // must read the same.
        assert_eq!(
            at("- one\n\n- two\n", "preamble.blocks.0")
                .unwrap()
                .get("text")
                .and_then(Value::as_str),
            Some("one\ntwo")
        );

        // A nested list is a block inside its item, and flattens with it.
        assert_eq!(
            at("- one\n  - inner\n- two\n", "preamble.blocks.0")
                .unwrap()
                .get("text")
                .and_then(Value::as_str),
            Some("one\ninner\ntwo")
        );
    }

    #[test]
    fn a_leading_metadata_block_is_its_own_kind() {
        // A Zola `_index.md`: the `+++` block used to flatten into two
        // paragraphs of the body, so `preamble` reported prose that was really
        // metadata and a caller reading "the first paragraph" got TOML.
        let toml = "+++\ntitle = \"T\"\n\n[extra]\ntagline = \"x\"\n+++\n\n# Real\n\nThe lead.\n";
        assert_eq!(types(toml, "preamble.blocks"), ["frontmatter".to_string()]);
        assert_eq!(
            at(toml, "preamble.blocks.0.format").unwrap(),
            Value::String("toml".to_string())
        );
        // Deliberately not copied into `text`: which fields it holds is TOML's
        // answer, and `--input-format toml-frontmatter` is how you get them.
        // This structural view must not bypass field-name-based redaction.
        assert_eq!(text(toml, "preamble.blocks.0.text"), "");
        // The body reads exactly as it would without the block.
        assert_eq!(text(toml, "h1.0.text"), "Real");
        assert_eq!(text(toml, "h1.0.paragraph.0.text"), "The lead.");

        let yaml = "---\ntitle: T\n---\n\n# Real\n";
        assert_eq!(types(yaml, "preamble.blocks"), ["frontmatter".to_string()]);
        assert_eq!(
            at(yaml, "preamble.blocks.0.format").unwrap(),
            Value::String("yaml".to_string())
        );
        assert_eq!(text(yaml, "h1.0.text"), "Real");
    }

    #[test]
    fn dashes_away_from_the_start_keep_their_commonmark_meaning() {
        // The metadata rule applies only at the very start of the file, so it
        // cannot reinterpret a `---` elsewhere. Both of these would break if
        // it did.
        assert_eq!(text("Setext\n---\n\nbody\n", "h2.0.text"), "Setext");
        assert_eq!(
            types("# T\n\na\n\n---\n\nb\n", "h1.0.blocks"),
            [
                "paragraph".to_string(),
                "rule".to_string(),
                "paragraph".to_string()
            ]
        );
    }

    #[test]
    fn a_source_range_stops_at_the_block_it_names() {
        // pulldown-cmark runs a list's span on to where the next block starts,
        // so it carries the blank separator. A range is a splice instruction:
        // taking the separator out merges the blocks on either side. Here that
        // made `Lead.` and `Next Title\n====` one setext heading, swallowing
        // the lead paragraph into a title.
        let source = "- a\n  - b\n\n\nAfter.\n";
        assert_eq!(
            at(source, "preamble.blocks.0.type").unwrap(),
            Value::String("list".to_string())
        );
        assert_eq!(
            at(source, "preamble.blocks.0.source_start_line").unwrap(),
            Value::Integer(1)
        );
        assert_eq!(
            at(source, "preamble.blocks.0.source_end_line").unwrap(),
            Value::Integer(2)
        );
        assert_eq!(
            at(source, "preamble.blocks.1.source_start_line").unwrap(),
            Value::Integer(5)
        );

        // Every other kind already ended at its own last line; assert they
        // still do, so the trim cannot over-correct.
        let mixed = "# H\n\npara\n\n```\ncode\n```\n\n> quote\n\n---\n\ntail\n";
        for (address, start, end) in [
            ("h1.0.blocks.0", 3, 3),
            ("h1.0.blocks.1", 5, 7),
            ("h1.0.blocks.2", 9, 9),
            ("h1.0.blocks.3", 11, 11),
            ("h1.0.blocks.4", 13, 13),
        ] {
            assert_eq!(
                at(mixed, &format!("{address}.source_start_line")).unwrap(),
                Value::Integer(start),
                "{address} start"
            );
            assert_eq!(
                at(mixed, &format!("{address}.source_end_line")).unwrap(),
                Value::Integer(end),
                "{address} end"
            );
        }
    }

    #[test]
    fn gfm_table_rows_are_a_paragraph() {
        // No extensions: a table is not a block kind here, and the pipe rows
        // are a paragraph. That is the specification's reading, not a gap.
        assert_eq!(
            shape("| a | b |\n|---|---|\n| 1 | 2 |\n", "preamble.blocks"),
            [(
                "paragraph".to_string(),
                "| a | b | |---|---| | 1 | 2 |".to_string()
            )]
        );
        assert_eq!(
            shape("| a | b |\n|---|---|\n| 1 | 2 |\n", "preamble.paragraph"),
            [(
                "paragraph".to_string(),
                "| a | b | |---|---| | 1 | 2 |".to_string()
            )]
        );
    }

    #[test]
    fn badge_syntax_remains_in_the_paragraph_view() {
        let source = "# T\n\n[![CI](a.svg)](b)\n\nThe lead.\n";
        assert_eq!(
            shape(source, "h1.0.paragraph"),
            [
                ("paragraph".to_string(), "CI".to_string()),
                ("paragraph".to_string(), "The lead.".to_string()),
            ]
        );
    }

    // ---- source line ranges ----------------------------------------------

    #[test]
    fn atx_heading_and_blocks_report_inclusive_source_lines() {
        let source = "# Title\n\nLead line one\nline two.\n\n```rs\nfn main() {}\n```\n";
        assert_eq!(integer(source, "h1.0.source_start_line"), 1);
        assert_eq!(integer(source, "h1.0.source_end_line"), 8);
        assert_eq!(integer(source, "h1.0.source_start_line"), 1);
        assert_eq!(integer(source, "h1.0.heading_end_line"), 1);
        assert_eq!(integer(source, "h1.0.paragraph.0.source_start_line"), 3);
        assert_eq!(integer(source, "h1.0.paragraph.0.source_end_line"), 4);
        assert_eq!(integer(source, "h1.0.blocks.1.source_start_line"), 6);
        assert_eq!(integer(source, "h1.0.blocks.1.source_end_line"), 8);
    }

    #[test]
    fn setext_heading_range_includes_its_underline() {
        let source = "My Project\n==========\n\nThe synopsis.\n\n## Install\n";
        assert_eq!(integer(source, "h1.0.source_start_line"), 1);
        assert_eq!(integer(source, "h1.0.heading_end_line"), 2);
        assert_eq!(integer(source, "h1.0.source_end_line"), 6);
        assert_eq!(integer(source, "h1.0.h2.0.source_start_line"), 6);
        assert_eq!(integer(source, "h1.0.h2.0.source_end_line"), 6);
        assert_eq!(integer(source, "h1.0.paragraph.0.source_start_line"), 4);
        assert_eq!(integer(source, "h1.0.paragraph.0.source_end_line"), 4);
    }

    #[test]
    fn line_ranges_are_utf8_safe_and_newline_style_independent() {
        let source = "# 中文标题\r\n\r\n这是首段，\r\n也是首段。\r\n\r\n## 安装";
        assert_eq!(integer(source, "h1.0.source_start_line"), 1);
        assert_eq!(integer(source, "h1.0.heading_end_line"), 1);
        assert_eq!(integer(source, "h1.0.paragraph.0.source_start_line"), 3);
        assert_eq!(integer(source, "h1.0.paragraph.0.source_end_line"), 4);
        assert_eq!(integer(source, "h1.0.h2.0.source_start_line"), 6);
        assert_eq!(integer(source, "h1.0.h2.0.heading_end_line"), 6);
    }

    #[test]
    fn a_bare_carriage_return_is_refused_rather_than_numbered() {
        // CommonMark ends a line at a bare `\r`; sed, awk, wc, head, and git
        // do not. These numbers exist to be handed to those, so for such a
        // file no single number is right for both readers — here `Lead` and
        // `## End` would share a line, and a splice by that number would cut
        // the wrong text silently.
        let error = load("# T\r\rLead\rcontinued\r\r## End").unwrap_err();
        // Its own code, not `document_parse_failed`: the file is valid
        // CommonMark and afdata is declining to number it, so a reader told
        // "parse failed" would go hunting for a syntax error that is not there.
        assert_eq!(error.code(), "document_source_refused");
        assert!(error.to_string().contains("carriage return"), "{error}");
        // The way out survives redaction. This detail is written here, about
        // the file's line endings, and holds no document text — dropping it
        // left `failed to parse Markdown` and nothing to act on.
        assert!(
            error.redacted_message().contains("CRLF line endings"),
            "{}",
            error.redacted_message()
        );

        // The endings every current tool writes are unaffected, and both
        // rules agree on them.
        let crlf = "# T\r\n\r\nLead\r\ncontinued\r\n\r\n## End\r\n";
        assert_eq!(integer(crlf, "h1.0.paragraph.0.source_start_line"), 3);
        assert_eq!(integer(crlf, "h1.0.paragraph.0.source_end_line"), 4);
        assert_eq!(integer(crlf, "h1.0.h2.0.source_start_line"), 6);

        // No final newline still numbers its last line.
        let bare = "# T\n\nLead";
        assert_eq!(integer(bare, "h1.0.paragraph.0.source_end_line"), 3);
    }

    #[test]
    fn frontmatter_range_includes_both_delimiters() {
        let source = "---\ntitle: T\nnested:\n  token_secret: hidden\n---\n\n# T\n";
        assert_eq!(integer(source, "preamble.blocks.0.source_start_line"), 1);
        assert_eq!(integer(source, "preamble.blocks.0.source_end_line"), 5);
        assert_eq!(text(source, "preamble.blocks.0.text"), "");
    }

    #[test]
    fn thematic_break_is_a_block_with_no_text() {
        assert_eq!(
            shape("# T\n\na\n\n---\n\nb\n", "h1.0.blocks"),
            [
                ("paragraph".to_string(), "a".to_string()),
                ("rule".to_string(), String::new()),
                ("paragraph".to_string(), "b".to_string()),
            ]
        );
        // The paragraph view skips it, so "the second paragraph" stays the
        // second paragraph.
        assert_eq!(
            shape("# T\n\na\n\n---\n\nb\n", "h1.0.paragraph"),
            [
                ("paragraph".to_string(), "a".to_string()),
                ("paragraph".to_string(), "b".to_string()),
            ]
        );
    }

    #[test]
    fn a_byte_order_mark_makes_the_first_block_a_paragraph() {
        // CommonMark does not strip a BOM, so `\u{feff}# Title` is not a
        // heading. Pinned because a Windows editor writes one silently: the
        // file then has no section at all, which fails loudly — the outcome to
        // keep, against publishing "\u{feff}Title" as a name.
        assert_eq!(
            types("\u{feff}# Title\n\nlead\n", "preamble.blocks"),
            ["paragraph".to_string(), "paragraph".to_string()]
        );
    }

    #[test]
    fn empty_document_has_no_blocks() {
        assert_eq!(shape("", "preamble.blocks"), []);
        assert_eq!(shape("\n\n   \n", "preamble.blocks"), []);
    }

    #[test]
    fn code_block_keeps_its_lines_and_drops_one_trailing_newline() {
        assert_eq!(
            shape("```\nline one\nline two\n```\n", "preamble.blocks"),
            [("code".to_string(), "line one\nline two".to_string())]
        );
    }
}
