//! Schema definitions and documentation rendering (feature = "schema").

use crate::document::Value;

/// Field definition for documentation and schema generation.
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub path: &'static str,
    pub type_name: &'static str,
    pub description: &'static str,
    pub default: Option<&'static str>,
    pub example: Option<&'static str>,
    pub required: bool,
    pub secret: bool,
}

/// Trait for providing config schema and defaults.
pub trait CliSchema {
    fn fields() -> &'static [FieldDef];
    fn default_value() -> Value;
}

/// Render field definitions as markdown reference.
pub fn render_doc_markdown(title: &str, fields: &[FieldDef]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {} Configuration Reference\n\n", title));

    // Group by section (first segment of path)
    let mut sections: std::collections::BTreeMap<&str, Vec<&FieldDef>> =
        std::collections::BTreeMap::new();
    for field in fields {
        let section = field.path.split('.').next().unwrap_or("_");
        sections.entry(section).or_default().push(field);
    }

    for (section, section_fields) in sections {
        out.push_str(&format!("## {}\n\n", section));
        out.push_str("| Key | Type | Default | Description |\n");
        out.push_str("|-----|------|---------|-------------|\n");

        for field in section_fields {
            let key = if field.secret {
                format!("`{}` 🔒", field.path)
            } else {
                format!("`{}`", field.path)
            };

            let type_str = field.type_name;
            let default_str = field.default.unwrap_or("—");
            let required_marker = if field.required { " **Required**" } else { "" };
            let example = field
                .example
                .map(|value| format!(" Example: `{value}`."))
                .unwrap_or_default();
            let desc = format!("{}{}{}", field.description, required_marker, example);

            out.push_str(&format!(
                "| {} | `{}` | `{}` | {} |\n",
                key, type_str, default_str, desc
            ));
        }
        out.push('\n');
    }

    out
}

/// Render annotated TOML with inline comments (if toml_edit feature available).
#[cfg(feature = "toml")]
pub fn render_annotated_toml(value: &Value, fields: &[FieldDef]) -> String {
    // Renders application-provided defaults as a fresh, commented TOML template
    // (not a source-preserving edit of an existing file — this is a doc/scaffold
    // helper). Each scalar/array is emitted as a validated fragment and nested
    // objects as `[table]` sections; the output round-trips through toml_edit
    // (see tests). It is deliberately not wired to the standalone CLI.
    let mut out = String::new();
    let field_map: std::collections::HashMap<&str, &FieldDef> =
        fields.iter().map(|f| (f.path, f)).collect();

    render_toml_with_comments(value, &mut out, "", &field_map, 0);
    out
}

#[cfg(not(feature = "toml"))]
pub fn render_annotated_toml(_value: &Value, _fields: &[FieldDef]) -> String {
    "# (TOML feature required for annotated output)".to_string()
}

#[cfg(feature = "toml")]
fn render_toml_with_comments(
    value: &Value,
    out: &mut String,
    path: &str,
    field_map: &std::collections::HashMap<&str, &FieldDef>,
    indent: usize,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    let indent_str = " ".repeat(indent);
    for (key, value) in object {
        let current_path = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        if value.is_object() {
            continue;
        }
        let Some(fragment) = toml_fragment(value) else {
            continue;
        };
        if let Some(field) = field_map.get(current_path.as_str()) {
            out.push_str(&indent_str);
            out.push_str("# ");
            out.push_str(&field.description.replace('\n', " "));
            out.push('\n');
            out.push_str(&indent_str);
            out.push_str(&format!(
                "# Type: {} | Default: {}\n",
                field.type_name,
                field.default.unwrap_or("—")
            ));
        }
        out.push_str(&indent_str);
        out.push_str(&toml_key(key));
        out.push_str(" = ");
        out.push_str(&fragment);
        out.push('\n');
    }
    for (key, value) in object {
        let Some(child) = value.as_object() else {
            continue;
        };
        let current_path = if path.is_empty() {
            key.clone()
        } else {
            format!("{path}.{key}")
        };
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push('[');
        out.push_str(&toml_table_path(&current_path));
        out.push_str("]\n");
        render_toml_with_comments(
            &Value::Object(child.clone()),
            out,
            &current_path,
            field_map,
            indent,
        );
    }
}

#[cfg(feature = "toml")]
fn toml_key(key: &str) -> String {
    if !key.is_empty()
        && key.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
    {
        key.to_string()
    } else {
        format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

#[cfg(feature = "toml")]
fn toml_table_path(path: &str) -> String {
    path.split('.').map(toml_key).collect::<Vec<_>>().join(".")
}

#[cfg(feature = "toml")]
fn toml_fragment(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(value) => Some(value.to_string()),
        Value::Integer(value) => Some(value.to_string()),
        Value::Unsigned(value) => i64::try_from(*value).ok().map(|value| value.to_string()),
        Value::Float(value) if value.is_finite() => Some(value.to_string()),
        Value::Float(_) => None,
        Value::Number(text) => Some(text.clone()),
        Value::String(value) => Some(format!("\"{}\"", value.escape_default())),
        Value::Array(values) => {
            let fragments = values
                .iter()
                .map(toml_fragment)
                .collect::<Option<Vec<_>>>()?;
            Some(format!("[{}]", fragments.join(", ")))
        }
        Value::Object(values) => {
            let fragments = values
                .iter()
                .map(|(key, value)| Some(format!("{} = {}", toml_key(key), toml_fragment(value)?)))
                .collect::<Option<Vec<_>>>()?;
            Some(format!("{{ {} }}", fragments.join(", ")))
        }
    }
}

/// Render annotated YAML with inline comments.
pub fn render_annotated_yaml(value: &Value, fields: &[FieldDef]) -> String {
    let mut out = String::new();
    let field_map: std::collections::HashMap<&str, &FieldDef> =
        fields.iter().map(|f| (f.path, f)).collect();

    render_yaml_with_comments(value, &mut out, "", &field_map, 0);
    out
}

fn render_yaml_with_comments(
    value: &Value,
    out: &mut String,
    path: &str,
    field_map: &std::collections::HashMap<&str, &FieldDef>,
    indent: usize,
) {
    let indent_str = " ".repeat(indent);

    if let Some(obj) = value.as_object() {
        for (key, val) in obj {
            let current_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{}.{}", path, key)
            };

            if let Some(field) = field_map.get(current_path.as_str()) {
                out.push_str(&indent_str);
                out.push_str(&format!("# {}\n", field.description));
                if let Some(default) = field.default {
                    out.push_str(&indent_str);
                    out.push_str(&format!("# Default: {}\n", default));
                }
            }

            out.push_str(&indent_str);
            out.push_str(&yaml_key(key));
            out.push_str(": ");

            match val {
                Value::String(s) => out.push_str(&format!("\"{}\"", s.escape_default())),
                Value::Integer(i) => out.push_str(&i.to_string()),
                Value::Unsigned(i) => out.push_str(&i.to_string()),
                Value::Float(f) => out.push_str(&f.to_string()),
                Value::Number(text) => out.push_str(text),
                Value::Bool(b) => out.push_str(&b.to_string()),
                Value::Null => out.push_str("null"),
                Value::Array(a) => {
                    out.push('\n');
                    render_yaml_sequence(a, out, &current_path, field_map, indent + 2);
                }
                Value::Object(_) => {
                    out.push('\n');
                    render_yaml_with_comments(val, out, &current_path, field_map, indent + 2);
                    continue;
                }
            }

            out.push('\n');
        }
    }
}

fn yaml_key(key: &str) -> String {
    if !key.is_empty()
        && key.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | ' ')
        })
        && !key.starts_with([
            '-', '?', ':', '!', '&', '*', '#', '{', '}', '[', ']', ',', '|', '>', '@', '`',
        ])
    {
        key.to_string()
    } else {
        format!("\"{}\"", key.escape_default())
    }
}

fn render_yaml_sequence(
    values: &[Value],
    out: &mut String,
    path: &str,
    field_map: &std::collections::HashMap<&str, &FieldDef>,
    indent: usize,
) {
    for value in values {
        let indent_str = " ".repeat(indent);
        out.push_str(&indent_str);
        out.push_str("- ");
        match value {
            Value::String(value) => out.push_str(&format!("\"{}\"", value.escape_default())),
            Value::Integer(value) => out.push_str(&value.to_string()),
            Value::Unsigned(value) => out.push_str(&value.to_string()),
            Value::Float(value) => out.push_str(&value.to_string()),
            Value::Number(text) => out.push_str(text),
            Value::Bool(value) => out.push_str(&value.to_string()),
            Value::Null => out.push_str("null"),
            Value::Object(_) | Value::Array(_) => {
                out.push('\n');
                if let Value::Object(_) = value {
                    render_yaml_with_comments(value, out, path, field_map, indent + 2);
                } else if let Value::Array(values) = value {
                    render_yaml_sequence(values, out, path, field_map, indent + 2);
                }
                continue;
            }
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn markdown_marks_required_secret_and_example() {
        let fields = [FieldDef {
            path: "service.api_key_secret",
            type_name: "string",
            description: "API key",
            default: None,
            example: Some("demo"),
            required: true,
            secret: true,
        }];
        let markdown = render_doc_markdown("Demo", &fields);
        assert!(markdown.contains("🔒"));
        assert!(markdown.contains("Required"));
        assert!(markdown.contains("demo"));
    }

    #[test]
    fn schema_renderer_is_application_metadata_not_json_schema() {
        let fields = [FieldDef {
            path: "service.api_key_secret",
            type_name: "string",
            description: "API key",
            default: Some("required-at-runtime"),
            example: Some("example-only"),
            required: true,
            secret: true,
        }];
        let markdown = render_doc_markdown("Demo", &fields);
        assert!(markdown.contains("required-at-runtime"));
        assert!(markdown.contains("example-only"));
        assert!(markdown.contains("🔒"));
        assert!(!markdown.contains("$schema"));
    }

    #[cfg(feature = "toml")]
    #[test]
    fn annotated_toml_is_valid_for_nested_and_array_values() {
        let value = Value::Object(BTreeMap::from([
            ("name".to_string(), Value::String("demo".to_string())),
            (
                "ports".to_string(),
                Value::Array(vec![Value::Integer(80), Value::Integer(443)]),
            ),
            (
                "database".to_string(),
                Value::Object(BTreeMap::from([(
                    "host".to_string(),
                    Value::String("localhost".to_string()),
                )])),
            ),
        ]));
        let rendered = render_annotated_toml(&value, &[]);
        assert!(
            rendered.parse::<toml_edit::DocumentMut>().is_ok(),
            "{rendered}"
        );
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn annotated_yaml_is_valid_for_nested_and_array_values() {
        let value = Value::Object(BTreeMap::from([
            ("name".to_string(), Value::String("demo".to_string())),
            (
                "ports".to_string(),
                Value::Array(vec![Value::Integer(80), Value::Integer(443)]),
            ),
            (
                "database".to_string(),
                Value::Object(BTreeMap::from([(
                    "host".to_string(),
                    Value::String("localhost".to_string()),
                )])),
            ),
        ]));
        let rendered = render_annotated_yaml(&value, &[]);
        assert!(
            noyalib::from_str::<noyalib::Value>(&rendered).is_ok(),
            "{rendered}"
        );
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn annotated_yaml_handles_mixed_arrays_and_escaping() {
        let value = Value::Object(BTreeMap::from([(
            "items".to_string(),
            Value::Array(vec![Value::Object(BTreeMap::from([
                (
                    "key:with:colon".to_string(),
                    Value::String("a\nb".to_string()),
                ),
                ("enabled".to_string(), Value::Bool(true)),
            ]))]),
        )]));
        let rendered = render_annotated_yaml(&value, &[]);
        let parsed = noyalib::from_str::<noyalib::Value>(&rendered).unwrap();
        assert!(matches!(parsed, noyalib::Value::Mapping(_)));
    }

    #[cfg(feature = "toml")]
    #[test]
    fn annotated_toml_handles_inline_tables() {
        let value = Value::Object(BTreeMap::from([(
            "items".to_string(),
            Value::Array(vec![Value::Object(BTreeMap::from([(
                "name".to_string(),
                Value::String("demo".to_string()),
            )]))]),
        )]));
        let rendered = render_annotated_toml(&value, &[]);
        assert!(
            rendered.parse::<toml_edit::DocumentMut>().is_ok(),
            "{rendered}"
        );
    }
}
