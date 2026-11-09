//! Property list reader (via the `plist` crate). Read-only.
//!
//! A plist is read into the same value tree every other format produces, with
//! one rule per plist type:
//!
//! | plist | value |
//! |---|---|
//! | `dict` | object |
//! | `array` | array |
//! | `string` | string |
//! | `integer` | integer |
//! | `real` | float |
//! | `true` / `false` | bool |
//! | `date` | RFC 3339 string in UTC, e.g. `2026-11-08T03:00:00Z` |
//! | `data` | standard base64 string |
//!
//! XML and binary property lists (`bplist` magic) map identically. A binary one
//! is recognised from its raw bytes by [`is_binary`] before any UTF-8 decoding,
//! parsed by [`load_binary`], and carries no source text because the format is
//! read-only.

use std::collections::BTreeMap;

use crate::document::{DocumentError, DocumentResult, Value};

const BINARY_MAGIC: &[u8] = b"bplist";

/// The text of the refusal for a binary property list read as another format.
/// It is afdata's own sentence and quotes nothing from the file, so
/// `DocumentError::redacted_message` keeps it whole instead of reducing the
/// parse failure to `failed to parse JSON`.
pub(crate) const WRONG_FORMAT_DETAIL: &str =
    "the input is a binary property list; read it with --input-format plist";

/// The same refusal for text that opens like an XML property list.
pub(crate) const WRONG_FORMAT_XML_DETAIL: &str =
    "the input is an XML property list; read it with --input-format plist";

/// Whether the raw bytes open like an XML property list: an XML declaration, a
/// `<plist` root, or a plist DOCTYPE, after an optional BOM and whitespace. Only
/// the opening bytes are looked at; this is a hint for a parse that already
/// failed on another format, not format sniffing.
pub(crate) fn looks_like_xml(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let head = &bytes[start..];
    head.starts_with(b"<?xml")
        || head.starts_with(b"<plist")
        || head.starts_with(b"<!DOCTYPE plist")
}

/// Whether the raw bytes start with the binary property list magic.
pub(crate) fn is_binary(bytes: &[u8]) -> bool {
    bytes.starts_with(BINARY_MAGIC)
}

/// The error for a binary property list read as `format` instead of plist.
pub(crate) fn wrong_format(format: &str) -> DocumentError {
    DocumentError::ParseError {
        format: format.to_string(),
        detail: WRONG_FORMAT_DETAIL.to_string(),
    }
}

/// The error for text that opens like an XML property list, read as `format`.
pub(crate) fn wrong_format_xml(format: &str) -> DocumentError {
    DocumentError::ParseError {
        format: format.to_string(),
        detail: WRONG_FORMAT_XML_DETAIL.to_string(),
    }
}

/// Parse a binary property list into a [`Value`].
pub(crate) fn load_binary(bytes: &[u8]) -> DocumentResult<Value> {
    let parsed = ::plist::Value::from_reader(std::io::Cursor::new(bytes)).map_err(|error| {
        DocumentError::ParseError {
            format: "plist".to_string(),
            detail: error.to_string(),
        }
    })?;
    convert(parsed)
}

/// Parse an XML property list into a [`Value`].
pub fn load(content: &str) -> DocumentResult<Value> {
    let parsed = ::plist::Value::from_reader_xml(content.as_bytes()).map_err(|error| {
        DocumentError::ParseError {
            format: "plist".to_string(),
            detail: error.to_string(),
        }
    })?;
    convert(parsed)
}

fn convert(value: ::plist::Value) -> DocumentResult<Value> {
    Ok(match value {
        ::plist::Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(convert)
                .collect::<DocumentResult<_>>()?,
        ),
        ::plist::Value::Dictionary(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| Ok((key, convert(value)?)))
                .collect::<DocumentResult<BTreeMap<_, _>>>()?,
        ),
        ::plist::Value::Boolean(value) => Value::Bool(value),
        ::plist::Value::Data(bytes) => Value::String(base64(&bytes)),
        ::plist::Value::Date(date) => Value::String(date.to_xml_format()),
        ::plist::Value::Real(value) => Value::Float(value),
        ::plist::Value::Integer(value) => match (value.as_signed(), value.as_unsigned()) {
            (Some(signed), _) => Value::Integer(signed),
            (None, Some(unsigned)) => Value::Unsigned(unsigned),
            (None, None) => return Err(unmapped("integer")),
        },
        ::plist::Value::String(value) => Value::String(value),
        ::plist::Value::Uid(uid) => Value::Unsigned(uid.get()),
        // `plist::Value` is non-exhaustive; a type added later has no mapping,
        // and reading it as null would hide that.
        _ => return Err(unmapped("value")),
    })
}

fn unmapped(kind: &str) -> DocumentError {
    DocumentError::ParseError {
        format: "plist".to_string(),
        detail: format!("plist {kind} has no value mapping"),
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        let sextet = |shift: u32| char::from(ALPHABET[((n >> shift) & 63) as usize]);
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads_like_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
    }
}
