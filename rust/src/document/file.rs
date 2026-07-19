//! Format-neutral document editing: an in-memory [`Document`] with
//! source-preserving edits, and a [`DocumentFile`] that adds the file boundary.
//!
//! [`Document`] holds the source text, its parsed [`Value`], and the
//! [`Format`]. Its verbs — [`set`](Document::set) / [`unset`](Document::unset)
//! / [`add`](Document::add) / [`remove`](Document::remove) — edit the source in
//! place (comments, ordering, and untouched formatting survive) and update the
//! parsed value alongside; [`source`](Document::source) reads the result and
//! [`encode`](Document::encode) re-renders a fresh, non-preserving copy from the
//! value. No file, no I/O, no guards — just editing.
//!
//! [`DocumentFile`] is a [`Document`] plus a path, reachable through
//! [`Deref`](std::ops::Deref): read and edit exactly as above, then commit with
//! [`save`](DocumentFile::save) or the [`edit`](DocumentFile::edit) closure.
//! Every write refuses a symlink/hardlinked target and goes to a
//! same-directory temp file that is fsynced, has the original permissions
//! re-applied, and is atomically renamed over the target — so a crash mid-write
//! never leaves a partial file.
//!
//! This module never redacts values — it reads and writes raw values as-is;
//! redaction is the caller's responsibility.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::document::{DocumentError, DocumentResult, Format, KeyedList, Value};

/// A format-neutral in-memory document: the original source text, its parsed
/// [`Value`], and the [`Format`] both came from. Has no file coupling —
/// construct it from a string or any [`std::io::Read`] the caller supplies,
/// edit it source-preservingly with [`set`](Document::set) /
/// [`unset`](Document::unset) / [`add`](Document::add) /
/// [`remove`](Document::remove), and read the result back with
/// [`source`](Document::source). [`DocumentFile`] is just this plus a path.
#[derive(Debug, Clone)]
pub struct Document {
    source: String,
    value: Value,
    format: Format,
}

impl Document {
    /// Parse `source` in the given `format`.
    ///
    /// Named `parse` (not `from_str`) deliberately: this takes an explicit
    /// `format` argument, so it is not the single-argument `std::str::FromStr`
    /// contract that `from_str` would imply.
    pub fn parse(source: &str, format: Format) -> DocumentResult<Document> {
        let value = format.load(source)?;
        Ok(Document {
            source: source.to_string(),
            value,
            format,
        })
    }

    /// Read `reader` fully to a `String`, then parse it in the given
    /// `format`.
    ///
    /// Reads only from the supplied `reader` — never touches the process's
    /// own stdin.
    pub fn from_reader<R: std::io::Read>(
        mut reader: R,
        format: Format,
    ) -> DocumentResult<Document> {
        let mut source = String::new();
        reader.read_to_string(&mut source)?;
        Document::parse(&source, format)
    }

    /// Borrow the parsed value (reflects the last successful edit).
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Borrow the current source text — the original bytes with every
    /// source-preserving edit applied. This is what [`DocumentFile::save`]
    /// writes.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The format this document was parsed from.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Resolve a dotted `path` against the parsed document and return the value
    /// at that address.
    pub fn value_at(&self, path: &str) -> DocumentResult<Value> {
        crate::document::get_path(&self.value, path, &[])
    }

    /// [`value_at`](Document::value_at) that also asserts the value at `path`
    /// satisfies `expected`, returning a [`DocumentError::TypeMismatch`]
    /// otherwise.
    pub fn value_at_typed(
        &self,
        path: &str,
        expected: crate::document::ValueType,
    ) -> DocumentResult<Value> {
        let value = self.value_at(path)?;
        if crate::document::value_matches_type(&value, expected) {
            Ok(value)
        } else {
            Err(DocumentError::TypeMismatch {
                path: path.to_string(),
                expected: expected.name().to_string(),
                got: value.kind_name().to_string(),
                hint: None,
            })
        }
    }

    /// Build a value from the CLI string `raw` per an explicit
    /// [`ValueType`](crate::document::ValueType) and [`set`](Document::set) it.
    pub fn set_typed(
        &mut self,
        key: &str,
        raw: Option<&str>,
        value_type: crate::document::ValueType,
    ) -> DocumentResult<()> {
        let value = crate::document::value_from_type(value_type, raw)?;
        self.set(key, value)
    }

    /// Re-render the current value in its format via [`Format::save`].
    ///
    /// This is a fresh, non-source-preserving render: comments and original
    /// formatting are not retained. Use [`source`](Document::source) after
    /// source-preserving edits to keep the original formatting.
    pub fn encode(&self) -> DocumentResult<String> {
        self.format.save(&self.value)
    }
}

/// A file-backed [`Document`]: the in-memory document plus the path it was
/// read from.
///
/// All reads and source-preserving edits come from [`Document`] through
/// [`Deref`]/[`DerefMut`]; `DocumentFile` adds only the file boundary — reading
/// on [`open`](DocumentFile::open) and an atomic, symlink-guarded commit on
/// [`save`](DocumentFile::save) / [`edit`](DocumentFile::edit).
#[derive(Debug, Clone)]
pub struct DocumentFile {
    doc: Document,
    path: PathBuf,
}

impl DocumentFile {
    /// Open and parse `path`.
    ///
    /// `format_override` takes precedence; otherwise the format is detected
    /// from the file extension via [`Format::detect`]. Reading is always
    /// allowed — this does not run the mutation guard.
    pub fn open(
        path: impl AsRef<Path>,
        format_override: Option<Format>,
    ) -> DocumentResult<DocumentFile> {
        let path = path.as_ref().to_path_buf();
        let format = match format_override {
            Some(format) => format,
            None => Format::detect(&path).ok_or_else(|| DocumentError::ParseError {
                format: "format".to_string(),
                detail: format!(
                    "cannot detect format from file extension `{}`; pass an explicit format",
                    path.display()
                ),
            })?,
        };
        let source = fs::read_to_string(&path).map_err(|error| DocumentError::IoError {
            detail: format!("read `{}`: {error}", path.display()),
        })?;
        Ok(DocumentFile {
            doc: Document::parse(&source, format)?,
            path,
        })
    }

    /// Open and parse `path` like [`DocumentFile::open`], but first reject any
    /// non-regular file, or any file larger than `max_bytes`, without reading
    /// its contents.
    ///
    /// Use this over [`open`](DocumentFile::open) when reading untrusted or
    /// secret-bearing config, where an unbounded read of an arbitrary path is
    /// a denial-of-service risk.
    pub fn open_capped(
        path: impl AsRef<Path>,
        format_override: Option<Format>,
        max_bytes: u64,
    ) -> DocumentResult<DocumentFile> {
        let path = path.as_ref();
        let metadata = fs::metadata(path).map_err(|error| DocumentError::IoError {
            detail: format!("read `{}`: {error}", path.display()),
        })?;
        if !metadata.is_file() {
            return Err(DocumentError::IoError {
                detail: format!("`{}` is not a regular file", path.display()),
            });
        }
        if metadata.len() > max_bytes {
            return Err(DocumentError::IoError {
                detail: format!(
                    "`{}` exceeds the {max_bytes}-byte read limit",
                    path.display()
                ),
            });
        }
        DocumentFile::open(path, format_override)
    }

    /// The file path this document was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Preflight-check that this file is safe to mutate — not a symlink, and
    /// on unix not hardlinked — without performing any write.
    ///
    /// [`save`](DocumentFile::save) runs this same guard before it writes, so
    /// calling it directly is only useful to front-run a *separate* side effect
    /// with the same guarantee — e.g. a CLI reading a secret from stdin for a
    /// `set` should refuse an unsafe target before consuming that input.
    pub fn ensure_mutable(&self, operation: &str) -> DocumentResult<()> {
        guard_mutation(&self.path, operation)?;
        Ok(())
    }
}

impl Document {
    /// Set `key` to the typed `value`, preserving the rest of the source
    /// document. The edit is staged in memory — call
    /// [`save`](DocumentFile::save) to persist it.
    ///
    /// Backend capability mirrors [`crate::document::set_path`] where the
    /// source editor allows it: the JSON backend replaces an existing value
    /// (scalar or collection) and creates missing intermediate parent objects;
    /// the TOML backend creates missing parent tables. Backends that cannot
    /// express an edit source-preserving (e.g. YAML collection mutation) return
    /// [`DocumentError::UnsupportedOperation`].
    pub fn set(&mut self, key: &str, value: Value) -> DocumentResult<()> {
        let mut new_doc = self.value.clone();
        self.format.ensure_writable("set")?;
        crate::document::set_path(&mut new_doc, key, &value, &[])?;
        let target = crate::document::get_path(&new_doc, key, &[])?;
        #[allow(unreachable_patterns)]
        let output = match self.format {
            #[cfg(feature = "toml")]
            Format::Toml => {
                crate::document::format::toml::set_preserving(&self.source, key, &target)?
            }
            #[cfg(feature = "yaml")]
            Format::Yaml => {
                crate::document::format::yaml::set_preserving(&self.source, key, &target)?
            }
            Format::Json => {
                crate::document::format::json::set_preserving(&self.source, key, &target)?
            }
            #[cfg(feature = "dotenv")]
            Format::Dotenv => {
                crate::document::format::dotenv::set_preserving(&self.source, key, &target)?
            }
            #[cfg(feature = "ini")]
            Format::Ini => {
                crate::document::format::ini::set_preserving(&self.source, key, &target)?
            }
            #[cfg(feature = "toml")]
            Format::TomlFrontmatter => {
                let parts = crate::document::format::frontmatter::split(
                    &self.source,
                    crate::document::format::frontmatter::Delimiter::Plus,
                )?;
                let new_fm =
                    crate::document::format::toml::set_preserving(parts.frontmatter, key, &target)?;
                format!("{}{}{}", parts.pre, new_fm, parts.post)
            }
            #[cfg(feature = "yaml")]
            Format::YamlFrontmatter => {
                let parts = crate::document::format::frontmatter::split(
                    &self.source,
                    crate::document::format::frontmatter::Delimiter::Dash,
                )?;
                let new_fm =
                    crate::document::format::yaml::set_preserving(parts.frontmatter, key, &target)?;
                format!("{}{}{}", parts.pre, new_fm, parts.post)
            }
            _ => self.format.save(&new_doc)?,
        };
        self.source = output;
        self.value = new_doc;
        Ok(())
    }

    /// Add a new element to the keyed list at `key`, identified by
    /// `slug`/`slug_field`, with the given `fields`. Preserves the rest of
    /// the source document.
    ///
    /// Only JSON and YAML backends implement a source-preserving
    /// keyed-collection editor today; other formats return
    /// [`DocumentError::UnsupportedOperation`].
    pub fn add(
        &mut self,
        key: &str,
        slug: &str,
        slug_field: &str,
        fields: &[(String, Value)],
    ) -> DocumentResult<()> {
        let mut value = self.value.clone();
        self.format.ensure_writable("add")?;
        let keyed_lists = [KeyedList {
            prefix: key,
            slug_field,
        }];
        crate::document::add_keyed(&mut value, key, slug, &keyed_lists, None, fields)?;
        let output: String = match self.format {
            Format::Json => {
                let array = crate::document::get_path(&value, key, &keyed_lists)?;
                let item = array
                    .as_array()
                    .and_then(|items| items.last())
                    .ok_or_else(|| DocumentError::UnsupportedOperation {
                        format: "JSON".to_string(),
                        operation: "add".to_string(),
                        detail: "keyed list did not produce an array item".to_string(),
                    })?;
                crate::document::format::json::append_array_item_preserving(
                    &self.source,
                    key,
                    item,
                )?
            }
            #[cfg(feature = "yaml")]
            Format::Yaml => {
                let array = crate::document::get_path(&value, key, &keyed_lists)?;
                let item = array
                    .as_array()
                    .and_then(|items| items.last())
                    .ok_or_else(|| DocumentError::UnsupportedOperation {
                        format: "YAML".to_string(),
                        operation: "add".to_string(),
                        detail: "keyed list did not produce an array item".to_string(),
                    })?;
                crate::document::format::yaml::append_array_item_preserving(
                    &self.source,
                    key,
                    item,
                )?
            }
            _ => {
                return Err(DocumentError::UnsupportedOperation {
                    format: self.format.name().to_string(),
                    operation: "add".to_string(),
                    detail: "keyed collection source editor is not implemented for this backend"
                        .to_string(),
                });
            }
        };
        self.source = output;
        self.value = value;
        Ok(())
    }

    /// Remove the element identified by `slug`/`slug_field` from the keyed
    /// list at `key`. Preserves the rest of the source document.
    ///
    /// Only JSON and YAML backends implement a source-preserving
    /// keyed-collection editor today; other formats return
    /// [`DocumentError::UnsupportedOperation`].
    pub fn remove(&mut self, key: &str, slug: &str, slug_field: &str) -> DocumentResult<()> {
        let mut value = self.value.clone();
        self.format.ensure_writable("remove")?;
        let keyed_lists = [KeyedList {
            prefix: key,
            slug_field,
        }];
        let original_array = crate::document::get_path(&value, key, &keyed_lists)?;
        let removed_index = original_array
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .position(|item| item.get(slug_field).and_then(Value::as_str) == Some(slug))
            })
            .ok_or_else(|| DocumentError::SlugNotFound {
                prefix: key.to_string(),
                slug: slug.to_string(),
            })?;
        #[cfg(not(feature = "yaml"))]
        let _ = removed_index;
        crate::document::remove_keyed(&mut value, key, slug, &keyed_lists)?;
        let output: String = match self.format {
            Format::Json => crate::document::format::json::remove_array_item_preserving(
                &self.source,
                key,
                slug,
                slug_field,
            )?,
            #[cfg(feature = "yaml")]
            Format::Yaml => crate::document::format::yaml::remove_array_item_preserving(
                &self.source,
                key,
                removed_index,
            )?,
            _ => {
                return Err(DocumentError::UnsupportedOperation {
                    format: self.format.name().to_string(),
                    operation: "remove".to_string(),
                    detail: "keyed collection source editor is not implemented for this backend"
                        .to_string(),
                });
            }
        };
        self.source = output;
        self.value = value;
        Ok(())
    }

    /// Remove the entry at `key` entirely, preserving the rest of the source
    /// document. The edit is staged in memory — call
    /// [`DocumentFile::save`] to persist it.
    ///
    /// Idempotent, like [`HashSet::remove`](std::collections::HashSet::remove):
    /// returns `Ok(false)` when `key` is already absent (nothing is staged) and
    /// `Ok(true)` when it was removed.
    pub fn unset(&mut self, key: &str) -> DocumentResult<bool> {
        if self.value_at(key).is_err() {
            return Ok(false);
        }
        let mut value = self.value.clone();
        self.format.ensure_writable("unset")?;
        crate::document::unset_path(&mut value, key)?;
        #[allow(unreachable_patterns)]
        let output = match self.format {
            Format::Json => crate::document::format::json::unset_preserving(&self.source, key)?,
            #[cfg(feature = "toml")]
            Format::Toml => crate::document::format::toml::unset_preserving(&self.source, key)?,
            #[cfg(feature = "yaml")]
            Format::Yaml => crate::document::format::yaml::unset_preserving(&self.source, key)?,
            #[cfg(feature = "dotenv")]
            Format::Dotenv => crate::document::format::dotenv::unset_preserving(&self.source, key)?,
            #[cfg(feature = "ini")]
            Format::Ini => crate::document::format::ini::unset_preserving(&self.source, key)?,
            #[cfg(feature = "toml")]
            Format::TomlFrontmatter => {
                let parts = crate::document::format::frontmatter::split(
                    &self.source,
                    crate::document::format::frontmatter::Delimiter::Plus,
                )?;
                let new_fm =
                    crate::document::format::toml::unset_preserving(parts.frontmatter, key)?;
                format!("{}{}{}", parts.pre, new_fm, parts.post)
            }
            #[cfg(feature = "yaml")]
            Format::YamlFrontmatter => {
                let parts = crate::document::format::frontmatter::split(
                    &self.source,
                    crate::document::format::frontmatter::Delimiter::Dash,
                )?;
                let new_fm =
                    crate::document::format::yaml::unset_preserving(parts.frontmatter, key)?;
                format!("{}{}{}", parts.pre, new_fm, parts.post)
            }
            _ => self.format.save(&value)?,
        };
        self.source = output;
        self.value = value;
        Ok(true)
    }
}

impl DocumentFile {
    /// Run `edit` against the in-memory [`Document`], then commit once with
    /// [`save`](DocumentFile::save). The single-call form of stage-then-save:
    /// the edits either all land (on `Ok`) or none reach disk (on `Err`,
    /// nothing is written), and the commit can't be forgotten.
    pub fn edit<F>(&mut self, edit: F) -> DocumentResult<()>
    where
        F: FnOnce(&mut Document) -> DocumentResult<()>,
    {
        edit(&mut self.doc)?;
        self.save()
    }

    /// Persist the document — every edit staged since [`open`](DocumentFile::open)
    /// — to its path in a single atomic write.
    ///
    /// The mutation verbs (`set`/`unset`/`add`/`remove`) stage their
    /// source-preserving edit in memory and do **not** touch disk; this is the
    /// one commit point. That lets a caller apply several edits and inspect the
    /// result via [`value`](Document::value) (e.g. deserialize-and-validate)
    /// before any bytes are written, and makes a multi-edit change atomic —
    /// all edits land together or none do.
    pub fn save(&self) -> DocumentResult<()> {
        self.save_atomic(self.doc.source())
    }

    /// Atomically replace the file's contents with `new_source`: guard
    /// against symlinks/hardlinked files, write to a same-directory temp
    /// file, fsync it, re-apply the original file's permissions, then
    /// `rename` it over the target. No partial write is ever observable —
    /// on any failure the temp file is removed and the original file is
    /// untouched.
    ///
    /// Crate-internal write seam behind the public [`save`](DocumentFile::save);
    /// it is not exported, so callers cannot write arbitrary raw text that
    /// bypasses the parse/edit path.
    pub(crate) fn save_atomic(&self, new_source: &str) -> DocumentResult<()> {
        write_atomic(&self.path, new_source.as_bytes(), "write")
    }
}

impl std::ops::Deref for DocumentFile {
    type Target = Document;

    fn deref(&self) -> &Document {
        &self.doc
    }
}

impl std::ops::DerefMut for DocumentFile {
    fn deref_mut(&mut self) -> &mut Document {
        &mut self.doc
    }
}

/// Reject mutation of a symlink or (on unix) a hardlinked file. Returns the
/// target's metadata on success so callers that also need to write can reuse
/// it (e.g. to preserve permissions) without a second syscall.
fn guard_mutation(path: &Path, operation: &str) -> DocumentResult<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).map_err(|error| DocumentError::IoError {
        detail: format!("{operation} preflight `{}`: {error}", path.display()),
    })?;
    if metadata.file_type().is_symlink() {
        return Err(DocumentError::UnsupportedOperation {
            format: "filesystem".to_string(),
            operation: operation.to_string(),
            detail: format!("refusing to mutate symlink `{}`", path.display()),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Err(DocumentError::UnsupportedOperation {
                format: "filesystem".to_string(),
                operation: operation.to_string(),
                detail: format!("refusing to mutate hardlinked file `{}`", path.display()),
            });
        }
    }
    Ok(metadata)
}

/// Write `bytes` to `path` atomically: guard, same-directory temp file,
/// fsync, permission preservation, then rename over the target.
fn write_atomic(path: &Path, bytes: &[u8], operation: &str) -> DocumentResult<()> {
    let metadata = guard_mutation(path, operation)?;

    let parent = path.parent().ok_or_else(|| DocumentError::IoError {
        detail: format!(
            "{operation} has no parent directory for `{}`",
            path.display()
        ),
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DocumentError::IoError {
            detail: format!("{operation} path is not valid UTF-8: `{}`", path.display()),
        })?;
    let pid = std::process::id();
    let mut temp_path = None;
    let mut temp_file = None;
    for attempt in 0..32_u32 {
        let candidate = parent.join(format!(".{file_name}.afdata-document.{pid}.{attempt}.tmp"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temp_path = Some(candidate);
                temp_file = Some(file);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(DocumentError::IoError {
                    detail: format!(
                        "{operation} create temporary file in `{}`: {error}",
                        parent.display()
                    ),
                });
            }
        }
    }
    let temp_path = temp_path.ok_or_else(|| DocumentError::IoError {
        detail: format!(
            "{operation} could not allocate temporary file in `{}`",
            parent.display()
        ),
    })?;
    let mut temp_file = temp_file.ok_or_else(|| DocumentError::IoError {
        detail: format!("{operation} temporary file handle missing"),
    })?;
    let result = (|| -> DocumentResult<()> {
        temp_file
            .write_all(bytes)
            .map_err(|error| DocumentError::IoError {
                detail: format!("{operation} write `{}`: {error}", path.display()),
            })?;
        temp_file
            .sync_all()
            .map_err(|error| DocumentError::IoError {
                detail: format!("{operation} fsync `{}`: {error}", path.display()),
            })?;
        drop(temp_file);
        fs::set_permissions(&temp_path, metadata.permissions()).map_err(|error| {
            DocumentError::IoError {
                detail: format!(
                    "{operation} preserve permissions `{}`: {error}",
                    path.display()
                ),
            }
        })?;
        fs::rename(&temp_path, path).map_err(|error| DocumentError::IoError {
            detail: format!("{operation} atomic replace `{}`: {error}", path.display()),
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]
    use super::*;
    use std::io::Cursor;

    fn write_temp(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn round_trip_open_json() {
        let dir = tempfile::tempdir().unwrap();
        let contents = r#"{"host": "example.com", "port": 993}"#;
        let path = write_temp(dir.path(), "config.json", contents);

        let doc = DocumentFile::open(&path, None).unwrap();

        assert_eq!(doc.format(), Format::Json);
        assert_eq!(
            doc.value().get("host").and_then(Value::as_str),
            Some("example.com")
        );
        assert_eq!(doc.source(), contents);
    }

    #[test]
    fn value_at_reads_a_nested_address() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(
            dir.path(),
            "config.json",
            r#"{"database": {"url": "postgres://x"}}"#,
        );
        let doc = DocumentFile::open(&path, None).unwrap();

        assert_eq!(
            doc.value_at("database.url").unwrap(),
            Value::String("postgres://x".to_string())
        );
        assert_eq!(
            doc.value_at("database.missing").unwrap_err().code(),
            "document_path_not_found"
        );
    }

    #[test]
    fn open_capped_enforces_size_and_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(dir.path(), "config.json", r#"{"k": "v"}"#);

        // Within the cap: opens normally.
        assert!(DocumentFile::open_capped(&path, None, 1024).is_ok());

        // Over the cap: rejected without parsing, as an io failure.
        let err = DocumentFile::open_capped(&path, None, 4).unwrap_err();
        assert_eq!(err.code(), "document_io_failed");
        assert!(err.to_string().contains("read limit"));

        // A directory is not a regular file.
        let dir_err = DocumentFile::open_capped(dir.path(), Some(Format::Json), 1024).unwrap_err();
        assert_eq!(dir_err.code(), "document_io_failed");
    }

    #[test]
    fn typed_get_and_set_enforce_the_stated_type() {
        use crate::document::ValueType;
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(dir.path(), "config.json", r#"{"port": 8080, "host": "x"}"#);
        let mut doc = DocumentFile::open(&path, None).unwrap();

        // Typed get: matching type returns, wrong type is a caught error, Json
        // matches anything.
        assert!(doc.value_at_typed("port", ValueType::Number).is_ok());
        assert_eq!(
            doc.value_at_typed("port", ValueType::String)
                .unwrap_err()
                .code(),
            "document_type_mismatch"
        );
        assert!(doc.value_at_typed("host", ValueType::Json).is_ok());

        // Typed set: the literal is validated against the stated type.
        doc.set_typed("port", Some("9090"), ValueType::Number)
            .unwrap();
        assert_eq!(
            doc.value_at("port").unwrap(),
            Value::from(serde_json::json!(9090))
        );
        assert_eq!(
            doc.set_typed("port", Some("not-a-number"), ValueType::Number)
                .unwrap_err()
                .code(),
            "document_parse_failed"
        );
    }

    #[cfg(feature = "toml")]
    #[test]
    fn round_trip_open_toml() {
        let dir = tempfile::tempdir().unwrap();
        let contents = "# leading comment\nhost = \"example.com\"\nport = 993\n";
        let path = write_temp(dir.path(), "config.toml", contents);

        let doc = DocumentFile::open(&path, None).unwrap();

        assert_eq!(doc.format(), Format::Toml);
        assert_eq!(
            doc.value().get("host").and_then(Value::as_str),
            Some("example.com")
        );
        assert_eq!(doc.source(), contents);
    }

    #[cfg(feature = "toml")]
    #[test]
    fn set_scalar_preserves_toml_comments_and_formatting() {
        let dir = tempfile::tempdir().unwrap();
        let contents = "# leading comment\nhost = \"example.com\"\nport = 993 # inline comment\n";
        let path = write_temp(dir.path(), "config.toml", contents);
        let mut doc = DocumentFile::open(&path, None).unwrap();

        doc.set("port", Value::Integer(1024)).unwrap();
        doc.save().unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# leading comment"));
        assert!(saved.contains("port = 1024"));
        assert_eq!(
            doc.value().get("port").and_then(Value::as_integer),
            Some(1024)
        );
        assert_eq!(doc.source(), saved);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_preserves_file_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(dir.path(), "config.json", r#"{"port": 993}"#);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let mut doc = DocumentFile::open(&path, None).unwrap();

        doc.set("port", Value::Integer(1024)).unwrap();
        doc.save().unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_target_is_rejected_for_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let target = write_temp(dir.path(), "target.json", r#"{"port": 993}"#);
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        // Reading through the symlink is fine.
        let mut doc = DocumentFile::open(&link, None).unwrap();

        // Editing in memory is fine; committing through the symlink is not.
        doc.set("port", Value::Integer(1024)).unwrap();
        let err = doc.save().unwrap_err();
        assert!(matches!(err, DocumentError::UnsupportedOperation { .. }));

        // The target file was never touched.
        let target_contents = fs::read_to_string(&target).unwrap();
        assert_eq!(target_contents, r#"{"port": 993}"#);
    }

    #[test]
    fn from_reader_parses_in_memory_cursor() {
        let cursor = Cursor::new(br#"{"host": "example.com"}"#.to_vec());

        let doc = Document::from_reader(cursor, Format::Json).unwrap();

        assert_eq!(
            doc.value().get("host").and_then(Value::as_str),
            Some("example.com")
        );
    }

    #[test]
    fn document_from_str_encode_round_trip() {
        let doc = Document::parse(r#"{"a": 1}"#, Format::Json).unwrap();
        let encoded = doc.encode().unwrap();
        let reparsed = Document::parse(&encoded, Format::Json).unwrap();
        assert_eq!(
            reparsed.value().get("a").and_then(Value::as_integer),
            Some(1)
        );
    }

    #[test]
    fn document_edits_source_in_memory_without_a_file() {
        // The point of the Document/DocumentFile split: source-preserving
        // editing with no file, no I/O, no guards.
        let mut doc = Document::parse("{\n  \"host\": \"old\"\n}\n", Format::Json).unwrap();
        doc.set("host", Value::String("new".to_string())).unwrap();
        doc.set("imap.port", Value::Integer(993)).unwrap(); // creates the parent

        assert_eq!(
            doc.source(),
            "{\n  \"host\": \"new\",\n  \"imap\": {\n    \"port\": 993\n  }\n}\n"
        );
        assert_eq!(
            doc.value_at("imap.port").unwrap(),
            Value::from(serde_json::json!(993))
        );
    }
}
