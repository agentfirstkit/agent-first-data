//! INI Core v1: a deliberately small, deterministic INI dialect.
//!
//! Entries before the first `[section]` header belong to the document root and
//! are addressed by their bare key. Refusing them was a smaller dialect but not
//! a truthful one: a flat `key=value` file — phoenixd's `phoenix.conf`, most of
//! `/etc/*.conf`, php.ini's preamble — is the shape people most often need one
//! value out of, and having no reader for it pushed callers back to `grep | cut`,
//! which is what this module exists to replace.
//!
//! The cost is one ambiguity, and it is refused rather than resolved: a root key
//! and a section of the same name would both address the same top-level name, so
//! a document containing both is a parse error.

use crate::document::{DocumentError, DocumentResult, Value};
use std::collections::BTreeMap;

const MAX_INI_VALUE_BYTES: usize = 1024 * 1024;

/// The section name entries before the first header carry. Not a real section:
/// it is how "at the document root" is spelled inside the lexer.
const ROOT: &str = "";

#[derive(Debug, Clone, Copy)]
struct IniEntry<'a> {
    section: &'a str,
    key: &'a str,
}

/// Parsed INI Core v1 source document. The lexer used for semantic loading and
/// source editing is intentionally shared so both paths enforce the same
/// section/key/duplicate rules.
#[derive(Debug)]
pub struct IniDocument<'a> {
    source: &'a str,
    entries: Vec<IniEntry<'a>>,
    sections: BTreeMap<&'a str, usize>,
}

impl<'a> IniDocument<'a> {
    pub fn parse(source: &'a str) -> DocumentResult<Self> {
        let mut entries = Vec::new();
        let mut current: Option<&str> = None;
        let mut sections = BTreeMap::<&str, usize>::new();
        for (index, raw) in source.lines().enumerate() {
            let line_number = index + 1;
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }
            if trimmed.starts_with('[') {
                let Some(name) = trimmed.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
                    return parse_error(line_number, 1, "invalid section header");
                };
                let name = name.trim();
                if name.is_empty() || name.contains(['[', ']']) {
                    return parse_error(line_number, 1, "section name must be non-empty");
                }
                if sections.insert(name, line_number).is_some() {
                    return parse_error(line_number, 1, "duplicate section");
                }
                if entries
                    .iter()
                    .any(|entry: &IniEntry<'_>| entry.section == ROOT && entry.key == name)
                {
                    return parse_error(
                        line_number,
                        1,
                        "section name collides with a root entry of the same name",
                    );
                }
                current = Some(name);
                continue;
            }
            // `""` is the root: entries before the first header. Addressed by
            // bare key, which is why a section may not take a root key's name.
            let section = current.unwrap_or(ROOT);
            let Some((key, value)) = line.split_once('=') else {
                return parse_error(line_number, 1, "expected key=value entry");
            };
            let key = key.trim();
            if key.is_empty() || key.contains(['[', ']']) {
                return parse_error(line_number, 1, "key must be non-empty");
            }
            if value.trim().len() > MAX_INI_VALUE_BYTES {
                return parse_error(
                    line_number,
                    line.find('=').unwrap_or(0) + 2,
                    "value exceeds 1 MiB",
                );
            }
            if entries
                .iter()
                .any(|entry: &IniEntry<'_>| entry.section == section && entry.key == key)
            {
                return parse_error(
                    line_number,
                    line.find(key).unwrap_or(0) + 1,
                    "duplicate key",
                );
            }
            entries.push(IniEntry { section, key });
        }
        Ok(Self {
            source,
            entries,
            sections,
        })
    }

    fn has_entry(&self, section: &str, key: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.section == section && entry.key == key)
    }

    fn has_section(&self, section: &str) -> bool {
        self.sections.contains_key(section)
    }

    fn to_value(&self) -> Value {
        let mut root = BTreeMap::<String, Value>::new();
        let mut current: Option<String> = None;
        for raw in self.source.lines() {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }
            if let Some(name) = trimmed.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                let name = name.trim().to_string();
                root.insert(name.clone(), Value::Object(BTreeMap::new()));
                current = Some(name);
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_string();
            let value = Value::String(value.trim().to_string());
            match current.as_ref() {
                // Inside a section, the entry is a member of that object.
                Some(section) => {
                    if let Some(Value::Object(entries)) = root.get_mut(section) {
                        entries.insert(key, value);
                    }
                }
                // Before any header, the entry *is* a top-level member.
                None => {
                    root.insert(key, value);
                }
            }
        }
        Value::Object(root)
    }
}

pub fn load(content: &str) -> DocumentResult<Value> {
    Ok(IniDocument::parse(content)?.to_value())
}

pub fn save(value: &Value) -> DocumentResult<String> {
    let Value::Object(sections) = value else {
        return Err(DocumentError::UnsupportedOperation {
            format: "INI".to_string(),
            operation: "save".to_string(),
            detail: "INI requires a section object".to_string(),
        });
    };
    let mut output = String::new();
    // Root entries first, and not by preference: a `key=value` written after a
    // header belongs to that section, so emitting them later would silently
    // move them.
    for (key, value) in sections {
        let Value::String(value) = value else {
            continue;
        };
        output.push_str(key);
        output.push('=');
        output.push_str(value);
        output.push('\n');
    }
    for (section, value) in sections {
        let Value::Object(entries) = value else {
            // Already emitted above as a root entry; anything else has no INI
            // spelling at all.
            if matches!(value, Value::String(_)) {
                continue;
            }
            return Err(DocumentError::UnsupportedOperation {
                format: "INI".to_string(),
                operation: "save".to_string(),
                detail: format!("`{section}` must be a section object or a root string"),
            });
        };
        output.push('[');
        output.push_str(section);
        output.push_str("]\n");
        for (key, value) in entries {
            let Value::String(value) = value else {
                return Err(DocumentError::UnsupportedOperation {
                    format: "INI".to_string(),
                    operation: "save".to_string(),
                    detail: format!("entry `{section}.{key}` must remain a string"),
                });
            };
            output.push_str(key);
            output.push('=');
            output.push_str(value);
            output.push('\n');
        }
    }
    Ok(output)
}

/// Replace an existing INI scalar without reordering sections or entries.
pub fn set_preserving(content: &str, path: &str, value: &Value) -> DocumentResult<String> {
    let Value::String(value) = value else {
        return Err(unsupported("set", "INI values are strings"));
    };
    edit_entry(content, path, Some(value))
}

/// Remove an existing INI entry without rewriting the document.
pub fn unset_preserving(content: &str, path: &str) -> DocumentResult<String> {
    edit_entry(content, path, None)
}

/// The section name a line declares, or `None` when it is not a header.
fn section_header(line: &str) -> Option<String> {
    let trimmed = line.trim_end_matches(['\n', '\r']).trim();
    (trimmed.starts_with('[') && trimmed.ends_with(']'))
        .then(|| trimmed[1..trimmed.len() - 1].trim().to_string())
}

/// End `output` with a line terminator so the next line starts on its own.
fn ensure_terminated(output: &mut String, newline: &str) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push_str(newline);
    }
}

fn edit_entry(content: &str, path: &str, replacement: Option<&str>) -> DocumentResult<String> {
    let segments = crate::document::parse_path(path)?;
    // One segment is a root entry, two are `section.key`. Deeper has no INI
    // spelling: a section cannot nest.
    let (section, key) = match segments.as_slice() {
        [key] => (ROOT.to_string(), key.clone()),
        [section, key] => (section.clone(), key.clone()),
        _ => return Err(unsupported("edit", "INI paths must be key or section.key")),
    };
    let document = IniDocument::parse(content).map_err(|error| with_path(error, path))?;
    let section = &section;
    let key = &key;
    let found = document.has_entry(section, key);
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let lines: Vec<&str> = content.split_inclusive('\n').collect();

    if found {
        let mut current = ROOT.to_string();
        let mut output = String::with_capacity(content.len());
        for line in &lines {
            if let Some(name) = section_header(line) {
                current = name;
            }
            let body = line.strip_suffix('\n').unwrap_or(line);
            let bare = body.strip_suffix('\r').unwrap_or(body);
            let matches = current == section.as_str()
                && bare
                    .trim()
                    .split_once('=')
                    .is_some_and(|(name, _)| name.trim() == key);
            if !matches {
                output.push_str(line);
                continue;
            }
            // `unset` drops the line entirely; `set` rewrites only the value,
            // keeping the key's own spacing.
            let Some(value) = replacement else { continue };
            let eq = bare.find('=').unwrap_or(bare.len());
            output.push_str(&bare[..eq + 1]);
            output.push_str(
                &bare[eq + 1..]
                    .chars()
                    .take_while(|character| character.is_whitespace())
                    .collect::<String>(),
            );
            output.push_str(value);
            if body.ends_with('\r') {
                output.push('\r');
            }
            if line.ends_with('\n') {
                output.push('\n');
            }
        }
        return Ok(output);
    }

    let Some(value) = replacement else {
        return Err(DocumentError::PathNotFound {
            path: path.to_string(),
        });
    };
    if section == ROOT && document.has_section(key) {
        return Err(unsupported(
            "edit",
            "an INI root key cannot replace a section of the same name",
        ));
    }
    if section != ROOT && document.has_entry(ROOT, section) {
        return Err(unsupported(
            "edit",
            "an INI section cannot replace a root key of the same name",
        ));
    }

    // A new key belongs at the end of its own section's block. Appending it at
    // end of file instead re-opens a section that already closed, and the
    // duplicate header leaves the document unparseable by the same parser that
    // just wrote it — a successful `set` that destroys the file.
    let mut section_seen = false;
    let mut next_header = None;
    for (index, line) in lines.iter().enumerate() {
        let Some(name) = section_header(line) else {
            continue;
        };
        if section_seen {
            next_header = Some(index);
            break;
        }
        if name == *section {
            section_seen = true;
        }
    }

    let entry = format!("{key}={value}{newline}");
    let mut output = String::with_capacity(content.len() + entry.len());

    // A new root entry goes before the first header. At end of file it would
    // land inside whatever section closes the document — the same silent move
    // `save` avoids, and here it would be a `set` that writes to the wrong key.
    if section == ROOT {
        let first_header = lines
            .iter()
            .position(|line| section_header(line).is_some())
            .unwrap_or(lines.len());
        for line in &lines[..first_header] {
            output.push_str(line);
        }
        ensure_terminated(&mut output, newline);
        output.push_str(&entry);
        for line in &lines[first_header..] {
            output.push_str(line);
        }
        return Ok(output);
    }

    match (section_seen, next_header) {
        (true, Some(mut at)) => {
            // Step back over the blank lines separating the two sections, so
            // the entry lands inside its own block rather than after the gap.
            while at > 0 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            for line in &lines[..at] {
                output.push_str(line);
            }
            ensure_terminated(&mut output, newline);
            output.push_str(&entry);
            for line in &lines[at..] {
                output.push_str(line);
            }
        }
        (true, None) => {
            output.push_str(content);
            ensure_terminated(&mut output, newline);
            output.push_str(&entry);
        }
        (false, _) => {
            output.push_str(content);
            if !output.is_empty() {
                ensure_terminated(&mut output, newline);
                output.push_str(newline);
            }
            output.push_str(&format!("[{section}]{newline}"));
            output.push_str(&entry);
        }
    }
    Ok(output)
}

fn unsupported(operation: &str, detail: &str) -> DocumentError {
    DocumentError::UnsupportedOperation {
        format: "INI".to_string(),
        operation: operation.to_string(),
        detail: detail.to_string(),
    }
}

fn parse_error<T>(line: usize, column: usize, detail: &str) -> DocumentResult<T> {
    Err(DocumentError::ParseError {
        format: "INI Core v1".to_string(),
        detail: format!("line {line}, column {column}: {detail}"),
    })
}

fn with_path(error: DocumentError, path: &str) -> DocumentError {
    match error {
        DocumentError::ParseError { format, detail } => DocumentError::ParseError {
            format,
            detail: format!("path `{path}`: {detail}"),
        },
        other => other,
    }
}
