//! Source-preserving dotenv backend.

use crate::document::{DocumentError, DocumentResult, Value};
use std::collections::BTreeMap;

const MAX_DOTENV_VALUE_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub struct DotenvDocument<'a> {
    source: &'a str,
    keys: Vec<String>,
}

impl<'a> DotenvDocument<'a> {
    pub fn parse(source: &'a str) -> DocumentResult<Self> {
        let value = parse_semantic(source)?;
        let keys = value
            .as_object()
            .map(|object| object.keys().cloned().collect())
            .unwrap_or_default();
        Ok(Self { source, keys })
    }

    fn has_key(&self, key: &str) -> bool {
        self.keys.iter().any(|candidate| candidate == key)
    }

    #[allow(dead_code)]
    fn source(&self) -> &'a str {
        self.source
    }
}

/// Parse dotenv without shell execution, environment lookup, or variable expansion.
pub fn load(content: &str) -> DocumentResult<Value> {
    parse_semantic(content)
}

fn parse_semantic(content: &str) -> DocumentResult<Value> {
    let mut values = BTreeMap::new();

    for (line_number, raw_line) in logical_lines(content)? {
        let line = raw_line
            .strip_suffix('\r')
            .unwrap_or(&raw_line)
            .trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let assignment = line
            .strip_prefix("export")
            .and_then(|rest| {
                rest.starts_with(char::is_whitespace)
                    .then(|| rest.trim_start())
            })
            .unwrap_or(line);
        let Some((raw_key, raw_value)) = assignment.split_once('=') else {
            return Err(parse_error(line_number, "expected KEY=value assignment"));
        };
        let key = raw_key.trim();
        if !valid_key(key) {
            return Err(parse_error(line_number, "invalid variable name"));
        }

        let value = parse_value(raw_value.trim_start(), line_number)?;
        if values.contains_key(key) {
            return Err(parse_error(line_number, "duplicate variable name"));
        }
        values.insert(key.to_string(), Value::String(value));
    }

    Ok(Value::Object(values))
}

/// Split `content` into logical lines, joining the physical lines a quoted
/// value spans.
///
/// A quote only opens a value where a value can start: after the `=`, before
/// any other non-whitespace character, and never inside a comment. Tracking
/// less than that made an ordinary English comment — `# set KEY=value if it
/// isn't already there` — open a quote at the apostrophe that no later line
/// closed, swallowing the rest of the file into one unparseable line and
/// yielding zero keys with no error at all.
fn logical_lines(content: &str) -> DocumentResult<Vec<(usize, String)>> {
    let mut lines = Vec::new();
    let mut buffer = String::new();
    let mut first_line = 1;
    let mut line_number = 1;
    let mut quote = None;
    let mut quote_opened_at = 0;
    let mut escaped = false;
    let mut value_may_open = false;
    let mut in_comment = false;
    let mut at_line_start = true;

    for character in content.chars() {
        if quote.is_none() {
            if at_line_start && !character.is_whitespace() {
                at_line_start = false;
                in_comment = character == '#';
            }
            if !in_comment {
                if value_may_open
                    && character.is_whitespace()
                    && character != '\n'
                    && character != '\r'
                {
                    buffer.push(character);
                    continue;
                }
                if value_may_open && (character == '\'' || character == '"') {
                    quote = Some(character);
                    quote_opened_at = line_number;
                    value_may_open = false;
                } else if character == '=' {
                    value_may_open = true;
                } else if !character.is_whitespace() {
                    // The value has already started unquoted, so a quote from
                    // here on is an ordinary character, not a delimiter.
                    value_may_open = false;
                }
            }
        } else if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if Some(character) == quote {
            quote = None;
        }
        if character == '\n' && quote.is_none() {
            lines.push((
                first_line,
                buffer.strip_suffix('\n').unwrap_or(&buffer).to_string(),
            ));
            buffer.clear();
            first_line = line_number + 1;
        } else {
            buffer.push(character);
        }
        if character == '\n' {
            line_number += 1;
            if quote.is_none() {
                value_may_open = false;
                in_comment = false;
                at_line_start = true;
            }
        }
    }
    if quote.is_some() {
        // Reaching end of input inside a quote means the file is malformed.
        // Reporting zero keys instead let `value --default` hand a caller its
        // fallback while the real value sat in the file, unread and unmentioned.
        return Err(parse_error(quote_opened_at, "unterminated quoted value"));
    }
    if !buffer.is_empty() || content.is_empty() {
        lines.push((first_line, buffer));
    }
    Ok(lines)
}

/// dotenv is intentionally read-only because the generic IR loses source formatting.
pub fn save(_value: &Value) -> DocumentResult<String> {
    Err(DocumentError::UnsupportedOperation {
        format: "dotenv".to_string(),
        operation: "save".to_string(),
        detail: "dotenv files are read-only because rewriting would lose comments, ordering, and quoting"
            .to_string(),
    })
}

/// Replace one existing assignment while preserving every other source byte.
pub fn set_preserving(content: &str, key: &str, value: &Value) -> DocumentResult<String> {
    let Value::String(value) = value else {
        return Err(unsupported("set", "dotenv values are strings"));
    };
    let replacement = encode_value(value);
    edit_line(content, key, Some(&replacement))
}

/// Remove one existing assignment while preserving every other source byte.
pub fn unset_preserving(content: &str, key: &str) -> DocumentResult<String> {
    edit_line(content, key, None)
}

fn edit_line(content: &str, key: &str, replacement: Option<&str>) -> DocumentResult<String> {
    if !valid_key(key) {
        return Err(parse_error(0, "invalid variable name"));
    }
    let document = DotenvDocument::parse(content).map_err(|error| with_path(error, key))?;
    let mut output = String::with_capacity(content.len());
    let found = document.has_key(key);
    for line in content.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let bare = body.strip_suffix('\r').unwrap_or(body);
        let trimmed = bare.trim_start();
        let assignment = trimmed
            .strip_prefix("export")
            .and_then(|rest| {
                rest.starts_with(char::is_whitespace)
                    .then(|| rest.trim_start())
            })
            .unwrap_or(trimmed);
        let matches = assignment
            .split_once('=')
            .is_some_and(|(name, _)| name.trim() == key);
        if matches {
            if let Some(value) = replacement {
                let prefix_len = body.len() - assignment.len();
                let prefix = &body[..prefix_len];
                let (name, old_value) = assignment.split_once('=').unwrap_or((key, ""));
                let suffix = old_value
                    .find(" #")
                    .map(|index| &old_value[index..])
                    .unwrap_or("");
                output.push_str(prefix);
                output.push_str(name);
                output.push('=');
                output.push_str(value);
                output.push_str(suffix);
                if body.ends_with('\r') {
                    output.push('\r');
                }
                if line.ends_with('\n') {
                    output.push('\n');
                }
            }
        } else {
            output.push_str(line);
        }
    }
    if !found {
        if let Some(value) = replacement {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push('\n');
            }
            output.push_str(key);
            output.push('=');
            output.push_str(value);
            if content.ends_with('\n') || !output.ends_with('\n') {
                output.push('\n');
            }
            return Ok(output);
        }
        return Err(DocumentError::PathNotFound {
            path: key.to_string(),
        });
    }
    Ok(output)
}

fn encode_value(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| !c.is_whitespace() && c != '#' && c != '\\')
    {
        return value.to_string();
    }
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

fn unsupported(operation: &str, detail: &str) -> DocumentError {
    DocumentError::UnsupportedOperation {
        format: "dotenv".to_string(),
        operation: operation.to_string(),
        detail: detail.to_string(),
    }
}

fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn parse_value(input: &str, line_number: usize) -> DocumentResult<String> {
    if input.len() > MAX_DOTENV_VALUE_BYTES {
        return Err(parse_error(line_number, "value exceeds 1 MiB"));
    }
    match input.chars().next() {
        Some('\'') => parse_quoted(input, '\'', line_number),
        Some('"') => parse_quoted(input, '"', line_number),
        _ => parse_unquoted(input),
    }
}

fn parse_quoted(input: &str, quote: char, line_number: usize) -> DocumentResult<String> {
    let mut output = String::new();
    let mut escaped = false;

    for (offset, character) in input[quote.len_utf8()..].char_indices() {
        if escaped {
            match (quote, character) {
                ('"', 'n') => output.push('\n'),
                ('"', 'r') => output.push('\r'),
                ('"', 't') => output.push('\t'),
                ('"', '\\') => output.push('\\'),
                ('"', '"') => output.push('"'),
                ('\'', '\'') => output.push('\''),
                (_, other) => {
                    output.push('\\');
                    output.push(other);
                }
            }
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if character == quote {
            let remainder_index = quote.len_utf8() + offset + character.len_utf8();
            let remainder = input[remainder_index..].trim_start();
            if !remainder.is_empty() && !remainder.starts_with('#') {
                return Err(parse_error(
                    line_number,
                    "unexpected content after quoted value",
                ));
            }
            return Ok(output);
        }
        output.push(character);
    }

    Err(parse_error(line_number, "unterminated quoted value"))
}

fn parse_unquoted(input: &str) -> DocumentResult<String> {
    let mut output = String::new();
    let mut escaped = false;

    for character in input.chars() {
        if escaped {
            output.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '#' && output.chars().last().is_some_and(char::is_whitespace) {
            break;
        } else {
            output.push(character);
        }
    }
    if escaped {
        output.push('\\');
    }
    Ok(output.trim_end().to_string())
}

fn parse_error(line_number: usize, detail: &str) -> DocumentError {
    DocumentError::ParseError {
        format: "dotenv".to_string(),
        detail: format!("line {line_number}: {detail}"),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comment_does_not_swallow_the_file() {
        // The apostrophe in "isn't" follows an `=` earlier on the same comment
        // line. Treating it as an opening quote consumed every later line, so
        // the file parsed to zero keys and reported success — a caller using
        // `--default` got its fallback while the real value sat in the file.
        let source =
            "# set KEY=value if it isn't already there\nAPI_HOST=example.com\nAPI_PORT=8080\n";
        let Value::Object(values) = load(source).unwrap() else {
            panic!("expected an object");
        };
        assert_eq!(values.len(), 2);
        assert_eq!(
            values.get("API_HOST"),
            Some(&Value::String("example.com".to_string()))
        );
    }

    #[test]
    fn a_quote_after_the_value_started_is_literal() {
        let Value::Object(values) = load("MESSAGE=it isn't quoted\n").unwrap() else {
            panic!("expected an object");
        };
        assert_eq!(
            values.get("MESSAGE"),
            Some(&Value::String("it isn't quoted".to_string()))
        );
    }

    #[test]
    fn quoted_values_still_span_lines() {
        let Value::Object(values) = load("A=\"line1\nline2\"\nB=after\n").unwrap() else {
            panic!("expected an object");
        };
        assert_eq!(
            values.get("A"),
            Some(&Value::String("line1\nline2".to_string()))
        );
        assert_eq!(values.get("B"), Some(&Value::String("after".to_string())));
    }

    #[test]
    fn an_unterminated_quote_is_a_parse_error() {
        // Silence was the unacceptable answer here: an unreadable file has to
        // say so rather than present itself as an empty one.
        let error = load("A=\"unterminated\nB=after\n").unwrap_err();
        assert_eq!(error.code(), "document_parse_failed");
    }
}
