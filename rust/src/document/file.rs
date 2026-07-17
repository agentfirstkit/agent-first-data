//! Format-neutral in-memory document value, plus a file-backed document
//! facade with safe, source-preserving edits.
//!
//! [`DocumentFile`] lifts the file-safety and source-preserving edit
//! orchestration that previously lived only in a CLI binary: mutation
//! methods refuse to write through a symlink or (on unix) a hardlinked
//! file, and every write goes to a same-directory temp file that is
//! fsynced, has the original file's permissions re-applied, and is
//! atomically renamed over the target — so a crash or error mid-write never
//! leaves a partial file.
//!
//! This module never redacts values on decode/encode/save/edit — it reads
//! and writes raw values as-is; redaction is the caller's responsibility.
//!
//! # Capability matrix
//!
//! - [`Document`] (in-memory): [`Document::value_mut`] allows arbitrary
//!   in-memory edits; [`Document::encode`] re-renders the value fresh from
//!   scratch — formatting and comments are NOT preserved. No file, no atomic
//!   write.
//! - [`DocumentFile`] (file-backed): reads via [`DocumentFile::value`] (paired
//!   with the free function [`crate::document::get_path`]), and
//!   source-preserving typed write verbs [`DocumentFile::set`]/
//!   [`DocumentFile::unset`]/[`DocumentFile::add`]/[`DocumentFile::remove`];
//!   every write is atomic (symlink/hardlink-guarded temp file + fsync +
//!   permission-preserving rename). There is no `value_mut` — edits go
//!   through the verbs above so the original source's formatting survives.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::document::{DocumentError, DocumentResult, Format, KeyedList, Value};

/// A format-neutral in-memory document: a parsed [`Value`] plus the
/// [`Format`] it was parsed from. Has no file or stdin coupling — construct
/// it from a string or any [`std::io::Read`] the caller supplies.
#[derive(Debug, Clone)]
pub struct Document {
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
        Ok(Document { value, format })
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

    /// Borrow the parsed value.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// Mutably borrow the parsed value.
    pub fn value_mut(&mut self) -> &mut Value {
        &mut self.value
    }

    /// The format this document was parsed from.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Re-render the current value in its format via [`Format::save`].
    ///
    /// This is a fresh, non-source-preserving render: comments and original
    /// formatting are not retained. Use [`DocumentFile`] when the original
    /// source's formatting must survive an edit.
    pub fn encode(&self) -> DocumentResult<String> {
        self.format.save(&self.value)
    }
}

/// A file-backed document: owns the path, format, original source text, and
/// parsed value.
///
/// Reading (`open`) is always allowed. Mutation methods guard against unsafe
/// targets (symlinks, hardlinked files) and write through a same-directory
/// temp file that is atomically renamed over the original — see
/// [`DocumentFile::save_atomic`].
#[derive(Debug)]
pub struct DocumentFile {
    path: PathBuf,
    format: Format,
    source: String,
    value: Value,
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
        let value = format.load(&source)?;
        Ok(DocumentFile {
            path,
            format,
            source,
            value,
        })
    }

    /// The file path this document was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Borrow the currently parsed value (reflects the last successful
    /// edit, if any).
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The format this file was opened as.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Borrow the current source text (reflects the last successful edit,
    /// if any).
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Preflight-check that this file is safe to mutate — not a symlink, and
    /// on unix not hardlinked — without performing any write.
    ///
    /// Every mutation method ([`DocumentFile::set`] and friends) already
    /// runs this same guard itself before writing, so calling it directly is
    /// only useful when a caller must front-run a *separate* side effect with
    /// the same guarantee — for example, a CLI that reads a secret from stdin
    /// for `set` should refuse an unsafe target before consuming that input,
    /// not after.
    pub fn ensure_mutable(&self, operation: &str) -> DocumentResult<()> {
        guard_mutation(&self.path, operation)?;
        Ok(())
    }

    /// Set `key` to the typed `value`, preserving the rest of the source
    /// document.
    pub fn set(&mut self, key: &str, value: Value) -> DocumentResult<()> {
        guard_mutation(&self.path, "set")?;
        let mut new_doc = self.value.clone();
        self.format.ensure_writable("set")?;
        crate::document::set_path(&mut new_doc, key, &value, &[])?;
        let target = crate::document::get_path(&new_doc, key, &[])?;
        #[allow(unreachable_patterns)]
        let output = match self.format {
            #[cfg(feature = "toml")]
            Format::Toml => {
                crate::document::format::toml::set_scalar_preserving(&self.source, key, &target)?
            }
            #[cfg(feature = "yaml")]
            Format::Yaml => {
                crate::document::format::yaml::set_scalar_preserving(&self.source, key, &target)?
            }
            Format::Json => {
                crate::document::format::json::set_scalar_preserving(&self.source, key, &target)?
            }
            #[cfg(feature = "dotenv")]
            Format::Dotenv => {
                crate::document::format::dotenv::set_scalar_preserving(&self.source, key, &target)?
            }
            #[cfg(feature = "ini")]
            Format::Ini => {
                crate::document::format::ini::set_scalar_preserving(&self.source, key, &target)?
            }
            _ => self.format.save(&new_doc)?,
        };
        self.save_atomic(&output)?;
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
        guard_mutation(&self.path, "add")?;
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
                    format: format_name(self.format).to_string(),
                    operation: "add".to_string(),
                    detail: "keyed collection source editor is not implemented for this backend"
                        .to_string(),
                });
            }
        };
        self.save_atomic(&output)?;
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
        guard_mutation(&self.path, "remove")?;
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
                    format: format_name(self.format).to_string(),
                    operation: "remove".to_string(),
                    detail: "keyed collection source editor is not implemented for this backend"
                        .to_string(),
                });
            }
        };
        self.save_atomic(&output)?;
        self.source = output;
        self.value = value;
        Ok(())
    }

    /// Remove the entry at `key` entirely. Preserves the rest of the source
    /// document.
    pub fn unset(&mut self, key: &str) -> DocumentResult<()> {
        guard_mutation(&self.path, "unset")?;
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
            _ => self.format.save(&value)?,
        };
        self.save_atomic(&output)?;
        self.source = output;
        self.value = value;
        Ok(())
    }

    /// Atomically replace the file's contents with `new_source`: guard
    /// against symlinks/hardlinked files, write to a same-directory temp
    /// file, fsync it, re-apply the original file's permissions, then
    /// `rename` it over the target. No partial write is ever observable —
    /// on any failure the temp file is removed and the original file is
    /// untouched.
    ///
    /// This does not update the in-memory [`DocumentFile::source`] /
    /// [`DocumentFile::value`] — the mutation methods do that themselves
    /// after a successful write.
    ///
    /// Crate-internal: this is the raw-string write seam the typed verbs
    /// (`set`/`unset`/`add`/`remove`) route through after computing a
    /// source-preserving rendering; it is not part of the public API, so
    /// callers cannot bypass the typed verbs to write arbitrary raw text.
    pub(crate) fn save_atomic(&self, new_source: &str) -> DocumentResult<()> {
        write_atomic(&self.path, new_source.as_bytes(), "write")
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

fn format_name(format: Format) -> &'static str {
    match format {
        Format::Json => "JSON",
        Format::Toml => "TOML",
        Format::Yaml => "YAML",
        Format::Dotenv => "dotenv",
        Format::Ini => "INI",
    }
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

        // Mutating through it is not.
        let err = doc.set("port", Value::Integer(1024)).unwrap_err();
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
}
