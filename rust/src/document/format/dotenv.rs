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
        let raw_line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line).trim_start();
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
fn logical_lines(content: &str) -> DocumentResult<Vec<(usize, &str)>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut first_line = 1;
    let mut line_number = 1;
    let mut quote = None;
    let mut quote_opened_at = 0;
    let mut escaped = false;
    let mut value_may_open = false;
    let mut assignment_started = false;
    let mut in_comment = false;
    let mut at_line_start = true;

    for (offset, character) in content.char_indices() {
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
                    continue;
                }
                if value_may_open && (character == '\'' || character == '"') {
                    quote = Some(character);
                    quote_opened_at = line_number;
                    value_may_open = false;
                } else if character == '=' && !assignment_started {
                    value_may_open = true;
                    assignment_started = true;
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
            lines.push((first_line, &content[start..offset + 1]));
            start = offset + 1;
            first_line = line_number + 1;
        }
        if character == '\n' {
            line_number += 1;
            if quote.is_none() {
                value_may_open = false;
                assignment_started = false;
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
    if start < content.len() || content.is_empty() {
        lines.push((first_line, &content[start..]));
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
    // The path is at fault here, not the file: a parse error would send the
    // caller looking for a malformed line that does not exist.
    if !valid_key(key) {
        let operation = if replacement.is_some() {
            "set"
        } else {
            "unset"
        };
        return Err(unsupported(
            operation,
            &format!("`{key}` is not a dotenv variable name"),
        ));
    }
    let document = DotenvDocument::parse(content).map_err(|error| with_path(error, key))?;
    let mut output = String::with_capacity(content.len());
    let found = document.has_key(key);
    for (line_number, line) in logical_lines(content)? {
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
                let prefix_len = bare.len() - assignment.len();
                let prefix = &body[..prefix_len];
                let (name, old_value) = assignment.split_once('=').unwrap_or((key, ""));
                let old_trimmed = old_value.trim_start();
                let spacing = &old_value[..old_value.len() - old_trimmed.len()];
                let suffix = value_suffix(old_trimmed, line_number)?;
                output.push_str(prefix);
                output.push_str(name);
                output.push('=');
                output.push_str(spacing);
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
            let newline = if content.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            if !output.is_empty() && !output.ends_with('\n') {
                output.push_str(newline);
            }
            output.push_str(key);
            output.push('=');
            output.push_str(value);
            if content.ends_with('\n') || !output.ends_with('\n') {
                output.push_str(newline);
            }
            return Ok(output);
        }
        return Err(DocumentError::PathNotFound {
            path: key.to_string(),
        });
    }
    Ok(output)
}

/// Encode a value so the file reads back the same here and when a shell
/// sources it (`set -a; . ./.env`): bare only for characters no shell treats
/// specially, plain double quotes when nothing inside would expand or need an
/// escape, single quotes (no expansion at all) when the value allows them, and
/// double quotes with `$` and `` ` `` escaped otherwise.
///
/// Newlines and tabs are written literally inside the quotes, since a shell
/// reads `\n` in double quotes as two characters. A carriage return is the
/// one exception: written as `\r`, so editors and line-ending conversion
/// cannot silently change the value, at the cost of a shell seeing `\r`.
fn encode_value(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|c| {
            c.is_alphanumeric()
                || matches!(c, '_' | '.' | '/' | ':' | '@' | '%' | '+' | ',' | '=' | '-')
        })
    {
        return value.to_string();
    }
    // Double quotes need nothing escaped here, and nothing in them expands.
    if !value.contains(['$', '`', '\\', '"', '\r']) {
        return format!("\"{value}\"");
    }
    // The reader treats `\'` inside single quotes as an escaped quote, so a
    // trailing backslash would swallow the closing quote.
    if !value.contains(['\'', '\r']) && !value.ends_with('\\') {
        return format!("'{value}'");
    }
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
            .replace('\r', "\\r")
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
        Some('\'') => parse_quoted(input, '\'', line_number).map(|(value, _)| value),
        Some('"') => parse_quoted(input, '"', line_number).map(|(value, _)| value),
        _ => parse_unquoted(input),
    }
}

fn parse_quoted(input: &str, quote: char, line_number: usize) -> DocumentResult<(String, usize)> {
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
                ('"', '$') => output.push('$'),
                ('"', '`') => output.push('`'),
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
            return Ok((output, remainder_index));
        }
        output.push(character);
    }

    Err(parse_error(line_number, "unterminated quoted value"))
}

/// Keep only an actual trailing comment or whitespace, never a ` #` inside a
/// quoted value. The parser and editor share the quote boundary.
fn value_suffix(input: &str, line_number: usize) -> DocumentResult<&str> {
    if let Some(quote @ ('\'' | '"')) = input.chars().next() {
        let (_, end) = parse_quoted(input, quote, line_number)?;
        return Ok(&input[end..]);
    }
    let mut escaped = false;
    let mut whitespace = None;
    for (offset, character) in input.char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
            continue;
        } else if character == '#'
            && let Some(start) = whitespace
        {
            return Ok(&input[start..]);
        }
        if character.is_whitespace() {
            whitespace.get_or_insert(offset);
        } else {
            whitespace = None;
        }
    }
    Ok(&input[whitespace.unwrap_or(input.len())..])
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
    fn editing_a_multiline_value_replaces_its_whole_assignment() {
        let source = "# before\r\n export A = \"first # value\r\nB=inside\"\t# keep\r\nB=after\r\n";
        let value = Value::String("\"quoted\"\r\t\n".into());
        let edited = set_preserving(source, "A", &value).unwrap();
        assert!(edited.starts_with("# before\r\n export A = "));
        assert!(edited.ends_with("\t# keep\r\nB=after\r\n"));
        assert!(!edited.contains("B=inside"));
        let Value::Object(values) = load(&edited).unwrap() else {
            panic!("expected an object");
        };
        assert_eq!(values["A"], value);
        assert_eq!(values["B"], Value::String("after".into()));
        assert_eq!(
            unset_preserving(source, "A").unwrap(),
            "# before\r\nB=after\r\n"
        );
    }

    #[test]
    fn written_quotes_and_comment_like_values_round_trip() {
        for value in [
            "\"quoted\"",
            "'quoted'",
            "x=\"literal",
            "x # value",
            "a\r\nb\tc",
        ] {
            let value = Value::String(value.into());
            let edited = set_preserving("A=old # keep\n", "A", &value).unwrap();
            let Value::Object(values) = load(&edited).unwrap() else {
                panic!("expected an object");
            };
            assert_eq!(values["A"], value, "{edited}");
            assert!(edited.ends_with(" # keep\n"));
        }
        assert_eq!(
            set_preserving("A=\"old # value\"\n", "A", &Value::String("new".into())).unwrap(),
            "A=new\n"
        );
    }

    /// A written value must mean the same thing to a shell that sources the
    /// file as it does to this reader, and must never run anything there.
    #[cfg(unix)]
    #[test]
    fn written_values_are_inert_when_a_shell_sources_the_file() {
        let dir = std::env::temp_dir().join(format!("afdata-dotenv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.env");
        for value in [
            "x;touch pwned",
            "a $(touch pwned)",
            "`touch pwned`",
            "it's $HOME",
            "it's `x` \\ $y\\",
            "~/path",
            "a|b&c>d<e*f?g[h]",
            "plain-value_1.2",
            "line1\nline2\tend",
            "it's\nmulti $x",
            "a\\\nb",
        ] {
            let value = Value::String(value.into());
            let edited = set_preserving("", "A", &value).unwrap();
            let Value::Object(values) = load(&edited).unwrap() else {
                panic!("expected an object");
            };
            assert_eq!(values["A"], value, "{edited}");
            std::fs::write(&file, &edited).unwrap();
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg("set -a; . ./t.env; printf %s \"$A\"")
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(output.status.success(), "{edited}");
            assert_eq!(
                Value::String(String::from_utf8(output.stdout).unwrap()),
                value,
                "{edited}"
            );
            assert!(!dir.join("pwned").exists(), "{edited}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn later_equals_signs_cannot_open_an_unquoted_value() {
        let Value::Object(values) = load("A=query=\"literal\nB=after\n").unwrap() else {
            panic!("expected an object");
        };
        assert_eq!(values["A"], Value::String("query=\"literal".into()));
        assert_eq!(values["B"], Value::String("after".into()));
    }

    #[test]
    fn an_unterminated_quote_is_a_parse_error() {
        // Silence was the unacceptable answer here: an unreadable file has to
        // say so rather than present itself as an empty one.
        let error = load("A=\"unterminated\nB=after\n").unwrap_err();
        assert_eq!(error.code(), "document_parse_failed");
    }
}
