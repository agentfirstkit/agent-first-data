//! JSON format backend (via serde_json).

use crate::document::{DocumentError, DocumentResult, Value};

/// Lossless JSON source document. The parsed node tree stores byte spans for
/// every value/member, so editors can replace only the requested span while
/// retaining untouched ordering, whitespace, escapes, and number spelling.
#[derive(Debug)]
pub struct JsonDocument<'a> {
    source: &'a str,
    root: Node,
}

impl<'a> JsonDocument<'a> {
    pub fn parse(source: &'a str) -> DocumentResult<Self> {
        let mut parser = Parser::new(source);
        let root = parser.parse_value(0)?;
        if parser.skip_ws(root.end) != source.len() {
            return Err(DocumentError::ParseError {
                format: "JSON".to_string(),
                detail: "trailing bytes after root value".to_string(),
            });
        }
        Ok(Self { source, root })
    }

    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    #[must_use]
    pub(crate) fn root(&self) -> &Node {
        &self.root
    }
}

/// Set a JSON value at `path` while retaining every byte outside the edited
/// region. JSON has no comments, but this still preserves key order,
/// whitespace, line endings, escapes, and untouched number spellings.
///
/// Matches the in-memory [`crate::document::set_path`] capability:
/// - an existing value (scalar *or* collection) is replaced in place;
/// - a missing leaf, and any missing intermediate parent objects, are created
///   under the deepest existing ancestor object.
pub fn set_preserving(content: &str, path: &str, value: &Value) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    let document = JsonDocument::parse(content)?;
    let root = document.root();
    let mut output = content.to_string();
    match resolve(root, &segments, 0) {
        Some(target) => {
            // Replace the whole existing node span — scalar or collection. A
            // replaced collection is re-rendered compactly; everything outside
            // its span (and key order) is untouched.
            let replacement = compact_json(value)?;
            output.replace_range(target.start..target.end, &replacement);
        }
        None => {
            // The full path is absent. Find the deepest existing ancestor
            // object, then splice one new member holding the remaining path
            // chain wrapped around the leaf value — creating every missing
            // intermediate object along the way.
            let (insert_at, tail) = deepest_existing_object(root, &segments).ok_or_else(|| {
                DocumentError::UnsupportedOperation {
                    format: "JSON".to_string(),
                    operation: "set".to_string(),
                    detail: "cannot insert a new key into a non-object JSON value".to_string(),
                }
            })?;
            let NodeKind::Object(entries) = &insert_at.kind else {
                unreachable!("deepest_existing_object only returns object nodes");
            };
            let (member_key, rest) = tail.split_first().ok_or(DocumentError::EmptyPath)?;
            let new_key =
                serde_json::to_string(member_key).map_err(|error| DocumentError::ParseError {
                    format: "JSON".to_string(),
                    detail: error.to_string(),
                })?;
            let (position, fragment) = match entries.last() {
                Some((_, last_node)) => {
                    let anchor = last_node.member_start.unwrap_or(last_node.start);
                    let (indent, multiline) = member_indent(content.as_bytes(), anchor);
                    let member_value = nested_member_value(rest, value, multiline, &indent)?;
                    let fragment = if multiline {
                        format!(",\n{indent}{new_key}: {member_value}")
                    } else {
                        format!(", {new_key}: {member_value}")
                    };
                    (last_node.end, fragment)
                }
                None => {
                    // Empty object: no sibling to mirror, so stay inline compact.
                    let member_value = nested_member_value(rest, value, false, "")?;
                    (insert_at.start + 1, format!("{new_key}: {member_value}"))
                }
            };
            output.insert_str(position, &fragment);
        }
    }
    serde_json::from_str::<serde_json::Value>(&output).map_err(|error| {
        DocumentError::ParseError {
            format: "JSON".to_string(),
            detail: error.to_string(),
        }
    })?;
    Ok(output)
}

/// Compact one-line JSON rendering of a document value.
fn compact_json(value: &Value) -> DocumentResult<String> {
    serde_json::to_string(&serde_json::Value::from(value.clone())).map_err(|error| {
        DocumentError::UnsupportedOperation {
            format: "JSON".to_string(),
            operation: "set".to_string(),
            detail: error.to_string(),
        }
    })
}

/// The deepest node reachable along `segments` that is an object, paired with
/// the still-missing tail below it. Returns `None` if that deepest existing
/// node is not an object (so a child key cannot be created under it).
fn deepest_existing_object<'a>(
    root: &'a Node,
    segments: &'a [String],
) -> Option<(&'a Node, &'a [String])> {
    // `segments` itself does not resolve (checked by the caller), so start one
    // level up and walk toward the root; the root (empty prefix) always exists.
    for split in (0..segments.len()).rev() {
        if let Some(node) = resolve(root, &segments[..split], 0) {
            return match node.kind {
                NodeKind::Object(_) => Some((node, &segments[split..])),
                _ => None,
            };
        }
    }
    None
}

/// Render the value for a spliced member: the leaf `value` wrapped in an object
/// for each remaining `tail` segment (`["a","b"] , v` → `{"a":{"b":v}}`).
/// Multiline output is re-indented to sit at `base_indent`.
fn nested_member_value(
    tail: &[String],
    value: &Value,
    multiline: bool,
    base_indent: &str,
) -> DocumentResult<String> {
    let mut nested = serde_json::Value::from(value.clone());
    for segment in tail.iter().rev() {
        let mut object = serde_json::Map::new();
        object.insert(segment.clone(), nested);
        nested = serde_json::Value::Object(object);
    }
    if multiline {
        let pretty =
            serde_json::to_string_pretty(&nested).map_err(|error| DocumentError::ParseError {
                format: "JSON".to_string(),
                detail: error.to_string(),
            })?;
        Ok(pretty.replace('\n', &format!("\n{base_indent}")))
    } else {
        serde_json::to_string(&nested).map_err(|error| DocumentError::ParseError {
            format: "JSON".to_string(),
            detail: error.to_string(),
        })
    }
}

/// Indentation of the member whose key begins at `anchor`, and whether that
/// member sits on its own line (so a spliced sibling should too).
fn member_indent(bytes: &[u8], anchor: usize) -> (String, bool) {
    match bytes[..anchor].iter().rposition(|byte| *byte == b'\n') {
        Some(newline) => {
            let indent = bytes[newline + 1..anchor]
                .iter()
                .take_while(|byte| matches!(byte, b' ' | b'\t'))
                .map(|byte| *byte as char)
                .collect();
            (indent, true)
        }
        None => (String::new(), false),
    }
}

/// Remove one existing JSON member or array item while preserving the rest of
/// the source layout. Removing the root value is intentionally unsupported.
pub fn unset_preserving(content: &str, path: &str) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    let document = JsonDocument::parse(content)?;
    let root = document.root();
    let target = resolve(root, &segments, 0).ok_or_else(|| DocumentError::PathNotFound {
        path: path.to_string(),
    })?;
    if target.member_start.is_none()
        && matches!(target.kind, NodeKind::Scalar)
        && segments.is_empty()
    {
        return Err(DocumentError::UnsupportedOperation {
            format: "JSON".to_string(),
            operation: "unset".to_string(),
            detail: "cannot remove the JSON root value".to_string(),
        });
    }
    let mut start = target.member_start.unwrap_or(target.start);
    if target.member_start.is_some()
        && let Some(newline) = content.as_bytes()[..start]
            .iter()
            .rposition(|byte| *byte == b'\n')
    {
        let candidate = newline + 1;
        if content.as_bytes()[candidate..start]
            .iter()
            .all(|byte| byte.is_ascii_whitespace())
        {
            start = candidate;
        }
    }
    let end = target.end;
    let after = skip_ws_bytes(content.as_bytes(), end);
    // Removing a last/only member also removes the comma *before* it — but that
    // search must stay inside the target's own parent container. A comma
    // belonging to an enclosing object/array (e.g. a sibling earlier in the
    // document) is not ours to take, or we splice across a container boundary.
    let parent_body_start =
        resolve(root, &segments[..segments.len() - 1], 0).map_or(0, |parent| parent.start + 1);
    let (remove_start, mut remove_end, remove_following_line) =
        if content.as_bytes().get(after) == Some(&b',') {
            (start, after + 1, true)
        } else if let Some(offset) = content.as_bytes()[parent_body_start..start]
            .iter()
            .rposition(|byte| *byte == b',')
        {
            (parent_body_start + offset, end, false)
        } else {
            (start, end, false)
        };
    if remove_following_line {
        if content.as_bytes().get(remove_end) == Some(&b'\r') {
            remove_end += 1;
        }
        if content.as_bytes().get(remove_end) == Some(&b'\n') {
            remove_end += 1;
        }
    }
    let mut output = content.to_string();
    output.replace_range(remove_start..remove_end, "");
    serde_json::from_str::<serde_json::Value>(&output).map_err(|error| {
        DocumentError::ParseError {
            format: "JSON".to_string(),
            detail: error.to_string(),
        }
    })?;
    Ok(output)
}

/// Append one JSON array item while retaining the existing array's trailing
/// whitespace and all bytes outside the insertion point.
pub fn append_array_item_preserving(
    content: &str,
    path: &str,
    item: &Value,
) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    let mut parser = Parser::new(content);
    let root = parser.parse_value(0)?;
    let target = resolve(&root, &segments, 0).ok_or_else(|| DocumentError::PathNotFound {
        path: path.to_string(),
    })?;
    let NodeKind::Array(items) = &target.kind else {
        return Err(DocumentError::UnsupportedOperation {
            format: "JSON".to_string(),
            operation: "add".to_string(),
            detail: "target is not an array".to_string(),
        });
    };
    let fragment =
        serde_json::to_string(&serde_json::Value::from(item.clone())).map_err(|error| {
            DocumentError::UnsupportedOperation {
                format: "JSON".to_string(),
                operation: "add".to_string(),
                detail: error.to_string(),
            }
        })?;
    let close = target
        .end
        .checked_sub(1)
        .ok_or_else(|| DocumentError::ParseError {
            format: "JSON".to_string(),
            detail: "invalid array span".to_string(),
        })?;
    let whitespace_start = content.as_bytes()[target.start + 1..close]
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map(|index| target.start + 2 + index)
        .unwrap_or(target.start + 1);
    let insertion = if items.is_empty() {
        fragment
    } else {
        format!(", {fragment}")
    };
    let mut output = content.to_string();
    output.insert_str(whitespace_start, &insertion);
    serde_json::from_str::<serde_json::Value>(&output).map_err(|error| {
        DocumentError::ParseError {
            format: "JSON".to_string(),
            detail: error.to_string(),
        }
    })?;
    Ok(output)
}

/// Remove a keyed object from a JSON array without rebuilding the document.
pub fn remove_array_item_preserving(
    content: &str,
    path: &str,
    slug: &str,
    slug_field: &str,
) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    let mut parser = Parser::new(content);
    let root = parser.parse_value(0)?;
    let target = resolve(&root, &segments, 0).ok_or_else(|| DocumentError::PathNotFound {
        path: path.to_string(),
    })?;
    let NodeKind::Array(items) = &target.kind else {
        return Err(DocumentError::UnsupportedOperation {
            format: "JSON".to_string(),
            operation: "remove".to_string(),
            detail: "target is not an array".to_string(),
        });
    };
    let item = items
        .iter()
        .find(|item| {
            let Ok(value) =
                serde_json::from_str::<serde_json::Value>(&content[item.start..item.end])
            else {
                return false;
            };
            value.get(slug_field).and_then(serde_json::Value::as_str) == Some(slug)
        })
        .ok_or_else(|| DocumentError::SlugNotFound {
            prefix: path.to_string(),
            slug: slug.to_string(),
        })?;
    let mut start = item.start;
    if let Some(newline) = content.as_bytes()[..start]
        .iter()
        .rposition(|byte| *byte == b'\n')
    {
        let candidate = newline + 1;
        if content.as_bytes()[candidate..start]
            .iter()
            .all(|byte| byte.is_ascii_whitespace())
        {
            start = candidate;
        }
    }
    let after = skip_ws_bytes(content.as_bytes(), item.end);
    let (mut remove_start, mut remove_end, remove_following_line) =
        if content.as_bytes().get(after) == Some(&b',') {
            (start, after + 1, true)
        } else if let Some(comma) = content.as_bytes()[target.start..start]
            .iter()
            .rposition(|byte| *byte == b',')
            .map(|offset| target.start + offset)
        {
            // Scoped to the array's own span (`target.start..start`), not
            // the whole document: an unscoped backward search can walk past
            // the array's opening `[` and land on an unrelated preceding
            // sibling's comma (e.g. `{"a":1,"items":[{"id":"x"}]}` removing
            // the sole item), corrupting the document instead of just
            // collapsing the array to `[]`.
            (comma, item.end, false)
        } else {
            (start, item.end, false)
        };
    if remove_following_line {
        if content.as_bytes().get(remove_end) == Some(&b'\r') {
            remove_end += 1;
        }
        if content.as_bytes().get(remove_end) == Some(&b'\n') {
            remove_end += 1;
        }
    }
    if remove_start > remove_end {
        std::mem::swap(&mut remove_start, &mut remove_end);
    }
    let mut output = content.to_string();
    output.replace_range(remove_start..remove_end, "");
    serde_json::from_str::<serde_json::Value>(&output).map_err(|error| {
        DocumentError::ParseError {
            format: "JSON".to_string(),
            detail: error.to_string(),
        }
    })?;
    Ok(output)
}

fn skip_ws_bytes(source: &[u8], mut position: usize) -> usize {
    while source
        .get(position)
        .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
    {
        position += 1;
    }
    position
}

#[derive(Debug, Clone)]
pub(crate) struct Node {
    start: usize,
    end: usize,
    member_start: Option<usize>,
    kind: NodeKind,
}

#[derive(Debug, Clone)]
enum NodeKind {
    Scalar,
    Object(Vec<(String, Node)>),
    Array(Vec<Node>),
}

fn resolve<'a>(node: &'a Node, segments: &[String], index: usize) -> Option<&'a Node> {
    if index == segments.len() {
        return Some(node);
    }
    match &node.kind {
        NodeKind::Object(entries) => entries
            .iter()
            .rev()
            .find(|(key, _)| key == &segments[index])
            .and_then(|(_, child)| resolve(child, segments, index + 1)),
        NodeKind::Array(items) => segments[index]
            .parse::<usize>()
            .ok()
            .and_then(|item| items.get(item))
            .and_then(|child| resolve(child, segments, index + 1)),
        NodeKind::Scalar => None,
    }
}

struct Parser<'a> {
    source: &'a [u8],
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source: source.as_bytes(),
        }
    }

    fn parse_value(&mut self, mut position: usize) -> DocumentResult<Node> {
        position = self.skip_ws(position);
        let start = position;
        let Some(byte) = self.source.get(position).copied() else {
            return self.error(position, "expected JSON value");
        };
        let kind = match byte {
            b'{' => self.parse_object(&mut position)?,
            b'[' => self.parse_array(&mut position)?,
            b'"' => {
                position = self.parse_string(position)?;
                NodeKind::Scalar
            }
            _ => {
                position = self.parse_scalar(position)?;
                NodeKind::Scalar
            }
        };
        Ok(Node {
            start,
            end: position,
            member_start: None,
            kind,
        })
    }

    fn parse_object(&mut self, position: &mut usize) -> DocumentResult<NodeKind> {
        *position += 1;
        let mut entries = Vec::new();
        loop {
            *position = self.skip_ws(*position);
            if self.source.get(*position) == Some(&b'}') {
                *position += 1;
                return Ok(NodeKind::Object(entries));
            }
            let key_start = *position;
            let key_end = self.parse_string(*position)?;
            let key = serde_json::from_slice::<String>(&self.source[key_start..key_end]).map_err(
                |error| DocumentError::ParseError {
                    format: "JSON".to_string(),
                    detail: error.to_string(),
                },
            )?;
            *position = self.skip_ws(key_end);
            if self.source.get(*position) != Some(&b':') {
                return self.error(*position, "expected `:` after object key");
            }
            *position += 1;
            let child = self.parse_value(*position)?;
            *position = child.end;
            let mut child = child;
            child.member_start = Some(key_start);
            entries.push((key, child));
            *position = self.skip_ws(*position);
            match self.source.get(*position) {
                Some(b',') => *position += 1,
                Some(b'}') => {
                    *position += 1;
                    return Ok(NodeKind::Object(entries));
                }
                _ => return self.error(*position, "expected `,` or `}` in object"),
            }
        }
    }

    fn parse_array(&mut self, position: &mut usize) -> DocumentResult<NodeKind> {
        *position += 1;
        let mut items = Vec::new();
        loop {
            *position = self.skip_ws(*position);
            if self.source.get(*position) == Some(&b']') {
                *position += 1;
                return Ok(NodeKind::Array(items));
            }
            let child = self.parse_value(*position)?;
            *position = child.end;
            items.push(child);
            *position = self.skip_ws(*position);
            match self.source.get(*position) {
                Some(b',') => *position += 1,
                Some(b']') => {
                    *position += 1;
                    return Ok(NodeKind::Array(items));
                }
                _ => return self.error(*position, "expected `,` or `]` in array"),
            }
        }
    }

    fn parse_string(&self, mut position: usize) -> DocumentResult<usize> {
        if self.source.get(position) != Some(&b'"') {
            return self.error(position, "expected JSON string");
        }
        position += 1;
        let mut escaped = false;
        while let Some(byte) = self.source.get(position).copied() {
            position += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return Ok(position);
            }
        }
        self.error(position, "unterminated JSON string")
    }

    fn parse_scalar(&self, mut position: usize) -> DocumentResult<usize> {
        let start = position;
        while let Some(byte) = self.source.get(position).copied() {
            if matches!(byte, b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t') {
                break;
            }
            position += 1;
        }
        if position == start {
            return self.error(position, "empty JSON scalar");
        }
        serde_json::from_slice::<serde_json::Value>(&self.source[start..position])
            .map_err(|error| DocumentError::ParseError {
                format: "JSON".to_string(),
                detail: error.to_string(),
            })
            .map(|_| position)
    }

    fn skip_ws(&self, mut position: usize) -> usize {
        while self
            .source
            .get(position)
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            position += 1;
        }
        position
    }

    fn error<T>(&self, position: usize, detail: &str) -> DocumentResult<T> {
        Err(DocumentError::ParseError {
            format: "JSON".to_string(),
            detail: format!("at byte {position}: {detail}"),
        })
    }
}

pub fn load(content: &str) -> DocumentResult<Value> {
    serde_json::from_str::<serde_json::Value>(content)
        .map(Value::from)
        .map_err(|e| DocumentError::ParseError {
            format: "JSON".to_string(),
            detail: e.to_string(),
        })
}

pub fn save(value: &Value) -> DocumentResult<String> {
    let json_val: serde_json::Value = value.clone().into();
    serde_json::to_string_pretty(&json_val).map_err(|e| DocumentError::ParseError {
        format: "JSON".to_string(),
        detail: e.to_string(),
    })
}
