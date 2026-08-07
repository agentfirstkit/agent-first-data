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

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use crate::document::{Addressing, DocumentError, DocumentResult, Format, KeyedList, Value};

/// Whether a capped file read may follow a symbolic link.
///
/// [`NoFollow`](SymlinkPolicy::NoFollow) is implemented with the operating
/// system's atomic `O_NOFOLLOW` open on unix when Cargo feature `libc` is
/// enabled. Builds without that feature, and other platforms, return
/// [`DocumentError::UnsupportedOperation`] rather than pretending a
/// check-before-open sequence provides the same guarantee.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SymlinkPolicy {
    /// Follow a symbolic link in the same way as [`File::open`].
    #[default]
    Follow,
    /// Refuse a symbolic-link final path component atomically.
    ///
    /// This requires Cargo feature `libc` on unix.
    NoFollow,
}

/// Whether [`DocumentFile::create_atomic`] may replace an existing target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CreateMode {
    /// Fail with `document_target_exists` if the target already exists.
    #[default]
    NewOnly,
    /// Atomically replace an existing target, or create it when absent.
    Replace,
}

/// Options for a safe first commit with [`DocumentFile::create_atomic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateOptions {
    mode: CreateMode,
    unix_mode: Option<u32>,
}

impl CreateOptions {
    /// Create with no-clobber semantics and unix mode `0o600` for a new file.
    ///
    /// Replacing an existing target preserves that target's mode unless
    /// [`unix_mode`](Self::unix_mode) asks for a specific one.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            mode: CreateMode::NewOnly,
            unix_mode: None,
        }
    }

    /// Set the target's unix permission bits.
    ///
    /// The value must contain only the low `0o777` permission bits. It is
    /// ignored on non-unix platforms. Setting it explicitly also applies the
    /// mode when replacing an existing file, which otherwise keeps its own.
    #[must_use]
    pub const fn unix_mode(mut self, unix_mode: u32) -> Self {
        self.unix_mode = Some(unix_mode);
        self
    }

    /// Explicitly allow an existing target to be atomically replaced.
    #[must_use]
    pub const fn replace(mut self) -> Self {
        self.mode = CreateMode::Replace;
        self
    }

    /// The configured target-existence behavior.
    #[must_use]
    pub const fn mode(&self) -> CreateMode {
        self.mode
    }

    /// The explicitly configured unix permission bits, if any.
    ///
    /// `None` means "`0o600` when creating, and keep the existing mode when
    /// replacing".
    #[must_use]
    pub const fn configured_unix_mode(&self) -> Option<u32> {
        self.unix_mode
    }

    /// The mode to apply, given whether the target already exists.
    const fn effective_unix_mode(&self, target_exists: bool) -> Option<u32> {
        match self.unix_mode {
            Some(mode) => Some(mode),
            // Replacing must not silently re-permission a file the caller did
            // not ask about: `save()` preserves the original, and a second
            // write path with the opposite rule would quietly widen a 0600
            // secrets file to the create default, or narrow a shared one.
            None if target_exists => None,
            None => Some(0o600),
        }
    }
}

impl Default for CreateOptions {
    fn default() -> Self {
        Self::new()
    }
}

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

    /// How a non-numeric path segment resolves against an array in this
    /// document: whatever its format declares (see [`Format::array_rule`]),
    /// with no caller-declared keyed lists.
    ///
    /// Use [`addressing_keyed`](Document::addressing_keyed) to add those.
    #[must_use]
    pub fn addressing(&self) -> Addressing<'static> {
        Addressing::INDEX_ONLY.with_array_rule(self.format.array_rule())
    }

    /// [`addressing`](Document::addressing) plus the caller's keyed lists,
    /// which take precedence over the format rule for the arrays they name.
    #[must_use]
    pub fn addressing_keyed<'a>(&self, keyed_lists: &'a [KeyedList<'a>]) -> Addressing<'a> {
        Addressing::keyed(keyed_lists).with_array_rule(self.format.array_rule())
    }

    /// Resolve a dotted `path` against the parsed document and return the value
    /// at that address.
    ///
    /// A non-empty ASCII-decimal segment against an array is an index;
    /// anything else goes through the format's own rule, so a Markdown document answers
    /// `h2.look` while a JSON one refuses `deps.foo`. See
    /// [`addressing`](Document::addressing).
    pub fn value_at(&self, path: &str) -> DocumentResult<Value> {
        crate::document::get_path(&self.value, path, self.addressing())
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

    /// Deserialize the complete document into a typed serde model.
    ///
    /// This is the convenience form of
    /// [`crate::document::from_value(self.value(), "")`](crate::document::from_value).
    /// Type errors are returned as content-redactable [`DocumentError`] values.
    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> DocumentResult<T> {
        crate::document::from_value(self.value(), "")
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

    /// Refuse `operation` when this document's format has no writer at all.
    ///
    /// A read-only format (see [`Format::is_read_only`]) is refused here, at
    /// the top of every mutating verb, rather than at the backend dispatch
    /// below it. The verbs stage an edit against the parsed value first, so a
    /// later refusal would report whichever backend step happened to notice —
    /// naming the wrong reason for a document that was never writable.
    fn ensure_writable(&self, operation: &str) -> DocumentResult<()> {
        if self.format.is_read_only() {
            return Err(self.format.read_only_error(operation));
        }
        Ok(())
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
/// [`std::ops::Deref`]/[`std::ops::DerefMut`]; `DocumentFile` adds only the file
/// boundary — reading on [`open`](DocumentFile::open) and an atomic,
/// symlink-guarded commit on [`save`](DocumentFile::save) /
/// [`edit`](DocumentFile::edit).
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
        let format = resolve_format(&path, format_override)?;
        let source = fs::read_to_string(&path).map_err(|error| DocumentError::IoError {
            detail: format!("read `{}`: {error}", path.display()),
        })?;
        Ok(DocumentFile {
            doc: Document::parse(&source, format)?,
            path,
        })
    }

    /// Open and parse `path` like [`DocumentFile::open`], but reject any
    /// non-regular file and limit the actual read to `max_bytes + 1`.
    ///
    /// Use this over [`open`](DocumentFile::open) when reading untrusted or
    /// secret-bearing config, where an unbounded read of an arbitrary path is
    /// a denial-of-service risk. The file is opened exactly once; both metadata
    /// and contents come from that same handle, so replacing or growing the
    /// path cannot bypass the cap.
    ///
    /// On unix, Cargo feature `libc` also opens with `O_NONBLOCK`, so a special
    /// file such as a FIFO cannot block before metadata rejects it. Without
    /// that feature, the single-handle and byte-cap guarantees still hold, but
    /// opening a special file can block before its type is inspected.
    pub fn open_capped(
        path: impl AsRef<Path>,
        format_override: Option<Format>,
        max_bytes: u64,
    ) -> DocumentResult<DocumentFile> {
        Self::open_capped_with_policy(path, format_override, max_bytes, SymlinkPolicy::Follow)
    }

    /// [`open_capped`](DocumentFile::open_capped) with an explicit symbolic-link
    /// policy.
    ///
    /// [`SymlinkPolicy::NoFollow`] requires Cargo feature `libc` on unix and is
    /// unsupported on other platforms.
    pub fn open_capped_with_policy(
        path: impl AsRef<Path>,
        format_override: Option<Format>,
        max_bytes: u64,
        symlink_policy: SymlinkPolicy,
    ) -> DocumentResult<DocumentFile> {
        let path = path.as_ref().to_path_buf();
        let format = resolve_format(&path, format_override)?;
        let file = open_read_handle(&path, symlink_policy)?;
        let source = read_capped_source(file, &path, max_bytes)?;
        Ok(DocumentFile {
            doc: Document::parse(&source, format)?,
            path,
        })
    }

    /// Safely create and commit a new file-backed document.
    ///
    /// Safely create and commit a new file-backed document.
    ///
    /// The document must already have passed a supported parser through
    /// [`Document::parse`]. Its source is parsed once more before any write,
    /// then written to a private same-directory temporary file, fsynced, and
    /// atomically installed. The default [`CreateOptions`] never replaces an
    /// existing path; replacement must be explicitly requested.
    ///
    /// The document's format must match what the path resolves to, so a commit
    /// cannot produce a file [`open`](DocumentFile::open) would then reject.
    pub fn create_atomic(
        path: impl AsRef<Path>,
        document: Document,
        options: CreateOptions,
    ) -> DocumentResult<DocumentFile> {
        let path = path.as_ref().to_path_buf();
        document.ensure_writable("create")?;
        // Resolving the path's own format is what `open` does; disagreeing here
        // would install a document that only this call can read back.
        let path_format = resolve_format(&path, None)?;
        if path_format != document.format() {
            return Err(DocumentError::UnsupportedOperation {
                format: document.format().name().to_string(),
                operation: "create".to_string(),
                detail: format!(
                    "path resolves to {}, so the created file could not be reopened",
                    path_format.name()
                ),
            });
        }
        validate_source_for_write(&document)?;
        validate_create_options(options)?;
        write_atomic_create(&path, document.source().as_bytes(), options)?;
        Ok(DocumentFile {
            doc: document,
            path,
        })
    }

    /// The file path this document was opened from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Preflight-check that this document format is writable and this file is
    /// safe to mutate — not a symlink, and on unix not hardlinked — without
    /// performing any write.
    ///
    /// [`save`](DocumentFile::save) runs this same guard before it writes, so
    /// calling it directly is only useful to front-run a *separate* side effect
    /// with the same guarantee — e.g. a CLI reading a secret from stdin for a
    /// `set` should refuse an unsafe target before consuming that input.
    pub fn ensure_mutable(&self, operation: &str) -> DocumentResult<()> {
        self.doc.ensure_writable(operation)?;
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
    /// YAML replaces collection blocks while preserving bytes outside the
    /// replaced block; TOML updates arrays, inline tables, and ordinary tables
    /// in place. Arrays of tables are refused because this editor has no
    /// explicit element-identity policy. Backends return
    /// [`DocumentError::UnsupportedOperation`] when an edit cannot be expressed
    /// source-preservingly.
    pub fn set(&mut self, key: &str, value: Value) -> DocumentResult<()> {
        let addressing = self.addressing();
        self.set_addressed(key, value, addressing)
    }

    /// [`set`](Document::set) with the caller's own addressing, so a path that
    /// names an array element by its content — `identities.me.email` — can be
    /// written and not merely read.
    ///
    /// The address is canonicalized to indices first (see
    /// [`crate::document::resolve_path`]); everything below this point, the
    /// source-preserving backends included, sees only `identities.0.email`.
    pub fn set_addressed(
        &mut self,
        key: &str,
        value: Value,
        addressing: Addressing<'_>,
    ) -> DocumentResult<()> {
        self.ensure_writable("set")?;
        let key = &crate::document::resolve_path(&self.value, key, addressing)?;
        let mut new_doc = self.value.clone();
        crate::document::set_path(&mut new_doc, key, &value, Addressing::INDEX_ONLY)?;
        let target = crate::document::get_path(&new_doc, key, Addressing::INDEX_ONLY)?;
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
    /// the source document. An empty `key` targets the document root when the
    /// root is itself the keyed array.
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
        self.ensure_writable("add")?;
        let mut value = self.value.clone();
        let keyed_lists = [KeyedList {
            prefix: key,
            slug_field,
        }];
        crate::document::add_keyed(&mut value, key, slug, &keyed_lists, None, fields)?;
        let array = if key.is_empty() {
            &value
        } else {
            crate::document::get_path_ref(&value, key, self.addressing_keyed(&keyed_lists))?
        };
        let item = array
            .as_array()
            .and_then(|items| items.last())
            .ok_or_else(|| DocumentError::UnsupportedOperation {
                format: self.format.name().to_string(),
                operation: "add".to_string(),
                detail: "keyed list did not produce an array item".to_string(),
            })?;
        // The catch-all covers the formats with no source-preserving keyed
        // editor. A build with only JSON has none left to catch, which makes it
        // unreachable there and reachable everywhere else — the arm stays.
        #[allow(unreachable_patterns)]
        let output: String = match self.format {
            Format::Json => crate::document::format::json::append_array_item_preserving(
                &self.source,
                key,
                item,
            )?,
            #[cfg(feature = "yaml")]
            Format::Yaml => crate::document::format::yaml::append_array_item_preserving(
                &self.source,
                key,
                item,
            )?,
            // Same editor as `set` reaches through frontmatter: the delimited
            // block is an ordinary YAML document, and the body is spliced back
            // untouched. Without this arm a keyed list was editable in a `.yaml`
            // file and refused in the frontmatter of a `.md` one.
            #[cfg(feature = "yaml")]
            Format::YamlFrontmatter => {
                let parts = crate::document::format::frontmatter::split(
                    &self.source,
                    crate::document::format::frontmatter::Delimiter::Dash,
                )?;
                let new_fm = crate::document::format::yaml::append_array_item_preserving(
                    parts.frontmatter,
                    key,
                    item,
                )?;
                format!("{}{}{}", parts.pre, new_fm, parts.post)
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
    /// list at `key`. Preserves the rest of the source document. An empty
    /// `key` targets the document root when the root is itself the keyed array.
    ///
    /// Only JSON and YAML backends implement a source-preserving
    /// keyed-collection editor today; other formats return
    /// [`DocumentError::UnsupportedOperation`].
    pub fn remove(&mut self, key: &str, slug: &str, slug_field: &str) -> DocumentResult<()> {
        self.ensure_writable("remove")?;
        let mut value = self.value.clone();
        let keyed_lists = [KeyedList {
            prefix: key,
            slug_field,
        }];
        let removed_index = crate::document::remove_keyed(&mut value, key, slug, &keyed_lists)?;
        // The catch-all covers the formats with no source-preserving keyed
        // editor. A build with only JSON has none left to catch, which makes it
        // unreachable there and reachable everywhere else — the arm stays.
        #[allow(unreachable_patterns)]
        let output: String = match self.format {
            Format::Json => crate::document::format::json::remove_array_item_preserving(
                &self.source,
                key,
                removed_index,
            )?,
            #[cfg(feature = "yaml")]
            Format::Yaml => crate::document::format::yaml::remove_array_item_preserving(
                &self.source,
                key,
                removed_index,
            )?,
            #[cfg(feature = "yaml")]
            Format::YamlFrontmatter => {
                let parts = crate::document::format::frontmatter::split(
                    &self.source,
                    crate::document::format::frontmatter::Delimiter::Dash,
                )?;
                let new_fm = crate::document::format::yaml::remove_array_item_preserving(
                    parts.frontmatter,
                    key,
                    removed_index,
                )?;
                format!("{}{}{}", parts.pre, new_fm, parts.post)
            }
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
    /// returns `Ok(false)` when there was nothing at `key` to remove (nothing
    /// is staged), and `Ok(true)` when it was removed.
    ///
    /// "Nothing there" does not depend on how deep the path is. A missing leaf
    /// and a missing ancestor are the same fact — `a.b.c` is absent whether
    /// `a.b` exists or not — so both answer `Ok(false)`. Only a path that is
    /// *malformed* stays an error: bad syntax, an index into a non-array, or a
    /// segment that tries to traverse through a scalar. Those describe a caller
    /// asking something incoherent, not a document that already lacks the key.
    ///
    /// A read-only format is the one case that errors *before* the idempotent
    /// answer: "nothing to remove" would report success for a document this
    /// verb can never edit.
    ///
    /// A content-addressed segment that matches no element is also an error
    /// ([`DocumentError::SlugNotFound`]), not `Ok(false)`. It is not the same
    /// fact as an absent key: the caller named an element and the document has
    /// none by that name, which is how a mistyped slug looks, and answering
    /// "removed nothing, all good" would swallow it.
    pub fn unset(&mut self, key: &str) -> DocumentResult<bool> {
        let addressing = self.addressing();
        self.unset_addressed(key, addressing)
    }

    /// [`unset`](Document::unset) with the caller's own addressing, so an
    /// element named by its content can be removed and not merely read. See
    /// [`set_addressed`](Document::set_addressed) for why the address is
    /// canonicalized before anything below sees it.
    pub fn unset_addressed(
        &mut self,
        key: &str,
        addressing: Addressing<'_>,
    ) -> DocumentResult<bool> {
        self.ensure_writable("unset")?;
        let key = &crate::document::resolve_path(&self.value, key, addressing)?;
        let segments = crate::document::parse_path(key)?;
        let (leaf, parents) = segments.split_last().ok_or(DocumentError::EmptyPath)?;
        let parent = if parents.is_empty() {
            &self.value
        } else {
            let parent_path = crate::document::join_path(parents);
            match crate::document::get_path_ref(&self.value, &parent_path, Addressing::INDEX_ONLY) {
                Ok(parent) => parent,
                // The ancestor is absent, so the leaf below it is too.
                Err(DocumentError::UnknownSegment { .. }) => return Ok(false),
                Err(error) => return Err(error),
            }
        };
        match parent {
            Value::Object(object) => {
                if !object.contains_key(leaf) {
                    return Ok(false);
                }
            }
            Value::Array(array) => {
                let index =
                    leaf.parse::<usize>()
                        .map_err(|_| DocumentError::UnregisteredArray {
                            path: crate::document::join_path(parents),
                        })?;
                if index >= array.len() {
                    return Err(DocumentError::IndexOutOfBounds {
                        path: crate::document::join_path(parents),
                        index,
                        len: array.len(),
                    });
                }
            }
            value => {
                return Err(DocumentError::NotTraversable {
                    path: crate::document::join_path(parents),
                    got: value.kind_name().to_string(),
                });
            }
        }
        let mut value = self.value.clone();
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
    /// closure and pre-install failures reach neither this handle nor disk,
    /// while a successful commit updates both. As with [`save`](Self::save), a
    /// parent-directory fsync error after installation is commit-uncertain and
    /// callers should reopen the path.
    pub fn edit<F>(&mut self, edit: F) -> DocumentResult<()>
    where
        F: FnOnce(&mut Document) -> DocumentResult<()>,
    {
        let mut draft = self.doc.clone();
        edit(&mut draft)?;
        self.save_document(&draft)?;
        self.doc = draft;
        Ok(())
    }

    /// Transactionally edit, deserialize, and validate the complete document
    /// before committing it.
    ///
    /// The closure works on a clone. Editing, typed decoding, and every
    /// pre-install write failure leave both this handle and the file in their
    /// original state. No partial file is observable. If the final parent
    /// directory fsync fails after the rename, a complete new file may already
    /// be installed even though durability could not be confirmed; reopen the
    /// file after that error. On success the decoded model is returned so
    /// callers need not deserialize a second time.
    pub fn edit_and_validate<T>(
        &mut self,
        edit: impl FnOnce(&mut Document) -> DocumentResult<()>,
    ) -> DocumentResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut draft = self.doc.clone();
        edit(&mut draft)?;
        let decoded = draft.decode::<T>()?;
        self.save_document(&draft)?;
        self.doc = draft;
        Ok(decoded)
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
    /// `rename` it over the target. No partial write is ever observable.
    /// Failures before the rename leave the original untouched. A failure
    /// syncing the parent directory after the rename means the complete new
    /// file may already be installed, but its crash durability could not be
    /// confirmed.
    ///
    /// Crate-internal write seam behind the public [`save`](DocumentFile::save);
    /// it is not exported, so callers cannot write arbitrary raw text that
    /// bypasses the parse/edit path.
    pub(crate) fn save_atomic(&self, new_source: &str) -> DocumentResult<()> {
        // A read-only format never reaches disk, even when nothing was staged
        // and the bytes would be identical: a `save` that succeeds is a claim
        // this file is under afdata's control for writing, and it is not.
        self.ensure_writable("save")?;
        // Read back what is about to be written, before writing it. A
        // source-preserving edit splices text, and a splice that lands in the
        // wrong place can produce a file this very parser rejects — an INI key
        // added to a section that had already closed, for one. Of the three
        // possible outcomes, reporting success and leaving an unreadable file
        // behind is the only unrecoverable one.
        validate_source_text_for_write(new_source, self.format)?;
        write_atomic(&self.path, new_source.as_bytes(), "write")
    }

    fn save_document(&self, document: &Document) -> DocumentResult<()> {
        document.ensure_writable("save")?;
        validate_source_for_write(document)?;
        write_atomic(&self.path, document.source().as_bytes(), "write")
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

fn resolve_format(path: &Path, format_override: Option<Format>) -> DocumentResult<Format> {
    match format_override {
        Some(format) => Ok(format),
        // A `.toml` file read by a build without the `toml` feature is not an
        // unknown format — it is a known one this binary cannot read, and
        // saying so names the fix. Only `Format::unavailable` can tell the two
        // apart, because `detect` answers `None` for both.
        None => match Format::detect(path) {
            Some(format) => Ok(format),
            None => Err(match Format::unavailable(path) {
                Some(feature) => DocumentError::UnsupportedOperation {
                    format: feature.to_string(),
                    operation: "open".to_string(),
                    detail: format!("requires Cargo feature `{feature}`"),
                },
                None => DocumentError::FormatUnknown {
                    path: path.display().to_string(),
                },
            }),
        },
    }
}

fn open_read_handle(path: &Path, symlink_policy: SymlinkPolicy) -> DocumentResult<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(all(unix, feature = "libc"))]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Opening a FIFO for reading can otherwise block before handle
        // metadata gets the chance to reject it as non-regular. O_NONBLOCK has
        // no effect on ordinary regular-file reads.
        let mut flags = libc::O_NONBLOCK;
        if symlink_policy == SymlinkPolicy::NoFollow {
            flags |= libc::O_NOFOLLOW;
        }
        options.custom_flags(flags);
    }
    #[cfg(all(unix, not(feature = "libc")))]
    if symlink_policy == SymlinkPolicy::NoFollow {
        return Err(DocumentError::UnsupportedOperation {
            format: "filesystem".to_string(),
            operation: "open".to_string(),
            detail: "atomic no-follow reads require Cargo feature `libc` on unix".to_string(),
        });
    }
    #[cfg(not(unix))]
    if symlink_policy == SymlinkPolicy::NoFollow {
        return Err(DocumentError::UnsupportedOperation {
            format: "filesystem".to_string(),
            operation: "open".to_string(),
            detail: "atomic no-follow reads are unavailable on this platform".to_string(),
        });
    }
    options.open(path).map_err(|error| DocumentError::IoError {
        detail: format!("read `{}`: {error}", path.display()),
    })
}

fn read_capped_source(file: File, path: &Path, max_bytes: u64) -> DocumentResult<String> {
    inspect_capped_source(&file, path, max_bytes)?;
    read_capped_contents(file, path, max_bytes)
}

fn inspect_capped_source(file: &File, path: &Path, max_bytes: u64) -> DocumentResult<()> {
    let metadata = file.metadata().map_err(|error| DocumentError::IoError {
        detail: format!("inspect `{}`: {error}", path.display()),
    })?;
    if !metadata.is_file() {
        return Err(DocumentError::IoError {
            detail: format!("`{}` is not a regular file", path.display()),
        });
    }
    if metadata.len() > max_bytes {
        return Err(DocumentError::TooLarge {
            path: path.display().to_string(),
            max_bytes,
        });
    }
    Ok(())
}

fn read_capped_contents(file: File, path: &Path, max_bytes: u64) -> DocumentResult<String> {
    let read_limit = max_bytes.saturating_add(1);
    let initial_capacity = usize::try_from(max_bytes.min(1024 * 1024)).unwrap_or(1024 * 1024);
    let mut bytes = Vec::with_capacity(initial_capacity);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| DocumentError::IoError {
            detail: format!("read `{}`: {error}", path.display()),
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
        return Err(DocumentError::TooLarge {
            path: path.display().to_string(),
            max_bytes,
        });
    }
    String::from_utf8(bytes).map_err(|error| DocumentError::IoError {
        detail: format!(
            "read `{}`: document is not UTF-8 (valid through byte {})",
            path.display(),
            error.utf8_error().valid_up_to()
        ),
    })
}

fn validate_source_for_write(document: &Document) -> DocumentResult<()> {
    validate_source_text_for_write(document.source(), document.format())
}

fn validate_source_text_for_write(source: &str, format: Format) -> DocumentResult<()> {
    Document::parse(source, format).map_err(|error| DocumentError::WriteWouldCorrupt {
        format: format.name().to_string(),
        detail: error.redacted_message(),
    })?;
    Ok(())
}

fn validate_create_options(options: CreateOptions) -> DocumentResult<()> {
    if let Some(unix_mode) = options.unix_mode
        && unix_mode & !0o777 != 0
    {
        return Err(DocumentError::InvalidArgument {
            detail: format!("unix mode {unix_mode:o} contains bits outside 0o777"),
        });
    }
    Ok(())
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

/// Compose a temporary file name that stays within one path component's limit.
///
/// The marker, pid, and attempt add about 35 bytes. A target whose own name is
/// close to `NAME_MAX` would push the composed name past it, and the failure
/// would surface at `save()` — after the caller had already made their edits, on
/// a file they opened successfully. Truncating the stem keeps the write
/// possible; uniqueness still comes from the pid and attempt, with `create_new`
/// retrying the rare collision.
fn temp_file_name(file_name: &str, pid: u32, attempt: u32) -> String {
    // The smallest NAME_MAX across the platforms this runs on.
    const MAX_NAME_BYTES: usize = 255;
    let suffix = format!(".afdata-document.{pid}.{attempt}.tmp");
    // One byte for the leading dot that hides the temporary file.
    let budget = MAX_NAME_BYTES.saturating_sub(suffix.len() + 1);
    let mut stem = file_name;
    if stem.len() > budget {
        let mut cut = budget;
        while cut > 0 && !stem.is_char_boundary(cut) {
            cut -= 1;
        }
        stem = &stem[..cut];
    }
    format!(".{stem}{suffix}")
}

fn allocate_private_temp(
    parent: &Path,
    file_name: &str,
    operation: &str,
) -> DocumentResult<(PathBuf, File)> {
    let pid = std::process::id();
    for attempt in 0..32_u32 {
        let candidate = parent.join(temp_file_name(file_name, pid, attempt));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(DocumentError::IoError {
                    detail: format!(
                        "{operation} temporary file in `{}`: {error}",
                        parent.display()
                    ),
                });
            }
        }
    }
    Err(DocumentError::IoError {
        detail: format!(
            "{operation} could not allocate temporary file in `{}`",
            parent.display()
        ),
    })
}

fn atomic_parent_and_name<'a>(
    path: &'a Path,
    operation: &str,
) -> DocumentResult<(&'a Path, String)> {
    let parent = match path.parent() {
        Some(parent) if parent.as_os_str().is_empty() => Path::new("."),
        Some(parent) => parent,
        None => {
            return Err(DocumentError::IoError {
                detail: format!(
                    "{operation} has no parent directory for `{}`",
                    path.display()
                ),
            });
        }
    };
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DocumentError::IoError {
            detail: format!("{operation} path is not valid UTF-8: `{}`", path.display()),
        })?
        .to_string();
    Ok((parent, file_name))
}

#[cfg(unix)]
fn sync_parent(parent: &Path, operation: &str) -> DocumentResult<()> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| DocumentError::IoError {
            detail: format!(
                "{operation} fsync parent directory `{}`: {error}",
                parent.display()
            ),
        })
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path, _operation: &str) -> DocumentResult<()> {
    // Rust does not expose a portable directory handle on non-unix targets.
    // The file itself is still synced before its atomic installation.
    Ok(())
}

fn write_temp_bytes(
    mut temp_file: File,
    temp_path: &Path,
    target_path: &Path,
    bytes: &[u8],
    operation: &str,
    permissions: Option<fs::Permissions>,
    unix_mode: Option<u32>,
) -> DocumentResult<()> {
    temp_file
        .write_all(bytes)
        .map_err(|error| DocumentError::IoError {
            detail: format!("{operation} write `{}`: {error}", target_path.display()),
        })?;
    if let Some(permissions) = permissions {
        temp_file
            .set_permissions(permissions)
            .map_err(|error| DocumentError::IoError {
                detail: format!(
                    "{operation} preserve permissions `{}`: {error}",
                    target_path.display()
                ),
            })?;
    }
    #[cfg(unix)]
    if let Some(unix_mode) = unix_mode {
        use std::os::unix::fs::PermissionsExt as _;
        temp_file
            .set_permissions(fs::Permissions::from_mode(unix_mode))
            .map_err(|error| DocumentError::IoError {
                detail: format!(
                    "{operation} set permissions on `{}`: {error}",
                    target_path.display()
                ),
            })?;
    }
    #[cfg(not(unix))]
    let _ = unix_mode;
    temp_file
        .sync_all()
        .map_err(|error| DocumentError::IoError {
            detail: format!("{operation} fsync `{}`: {error}", temp_path.display()),
        })
}

/// Write `bytes` to `path` atomically: guard, same-directory temp file,
/// fsync, permission preservation, then rename over the target.
fn write_atomic(path: &Path, bytes: &[u8], operation: &str) -> DocumentResult<()> {
    let metadata = guard_mutation(path, operation)?;
    let (parent, file_name) = atomic_parent_and_name(path, operation)?;
    let (temp_path, temp_file) = allocate_private_temp(parent, &file_name, operation)?;
    let result = (|| -> DocumentResult<()> {
        write_temp_bytes(
            temp_file,
            &temp_path,
            path,
            bytes,
            operation,
            Some(metadata.permissions()),
            None,
        )?;
        fs::rename(&temp_path, path).map_err(|error| DocumentError::IoError {
            detail: format!("{operation} atomic replace `{}`: {error}", path.display()),
        })?;
        sync_parent(parent, operation)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn write_atomic_create(path: &Path, bytes: &[u8], options: CreateOptions) -> DocumentResult<()> {
    let operation = "create";
    let mut existing_permissions = None;
    match fs::symlink_metadata(path) {
        Ok(_) => match options.mode {
            CreateMode::NewOnly => {
                return Err(DocumentError::AlreadyExists {
                    path: path.display().to_string(),
                });
            }
            CreateMode::Replace => {
                let metadata = guard_mutation(path, operation)?;
                existing_permissions = Some(metadata.permissions());
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(DocumentError::IoError {
                detail: format!("create preflight `{}`: {error}", path.display()),
            });
        }
    }

    let (parent, file_name) = atomic_parent_and_name(path, operation)?;
    let (temp_path, temp_file) = allocate_private_temp(parent, &file_name, operation)?;
    let result = (|| -> DocumentResult<()> {
        // A replace with no explicit mode keeps the target's own permissions,
        // exactly as `save()` does; only a first creation falls back to 0o600.
        let unix_mode = options.effective_unix_mode(existing_permissions.is_some());
        let preserved = unix_mode
            .is_none()
            .then(|| existing_permissions.clone())
            .flatten();
        write_temp_bytes(
            temp_file, &temp_path, path, bytes, operation, preserved, unix_mode,
        )?;
        match options.mode {
            CreateMode::NewOnly => match fs::hard_link(&temp_path, path) {
                Ok(()) => {
                    // The document is installed from here on. Failing to unlink
                    // the temporary link leaves a stray file, but reporting an
                    // error would tell the caller the commit did not happen —
                    // and a retry would then get `document_target_exists` for a
                    // write that in fact succeeded.
                    let _ = fs::remove_file(&temp_path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(DocumentError::AlreadyExists {
                        path: path.display().to_string(),
                    });
                }
                Err(error) => {
                    return Err(DocumentError::IoError {
                        detail: format!("create install `{}`: {error}", path.display()),
                    });
                }
            },
            CreateMode::Replace => {
                fs::rename(&temp_path, path).map_err(|error| DocumentError::IoError {
                    detail: format!("create atomic replace `{}`: {error}", path.display()),
                })?;
            }
        }
        sync_parent(parent, operation)?;
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

        // Over the cap: rejected without parsing, under its own code so a
        // caller enforcing a size budget need not match on the message.
        let err = DocumentFile::open_capped(&path, None, 4).unwrap_err();
        assert_eq!(err.code(), "document_too_large");

        // A directory is not a regular file, and that is a different failure.
        let dir_err = DocumentFile::open_capped(dir.path(), Some(Format::Json), 1024).unwrap_err();
        assert_eq!(dir_err.code(), "document_io_failed");

        // Missing is different again — the three must stay distinguishable.
        let missing =
            DocumentFile::open_capped(dir.path().join("absent.json"), None, 1024).unwrap_err();
        assert_ne!(missing.code(), "document_too_large");
    }

    #[cfg(unix)]
    #[test]
    fn capped_read_uses_the_open_handle_when_the_path_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let original = r#"{"source":"original"}"#;
        let path = write_temp(dir.path(), "config.json", original);
        let handle = open_read_handle(&path, SymlinkPolicy::Follow).unwrap();
        inspect_capped_source(&handle, &path, 64).unwrap();

        fs::rename(&path, dir.path().join("original.json")).unwrap();
        fs::write(&path, r#"{"source":"replacement"}"#).unwrap();

        let source = read_capped_contents(handle, &path, 64).unwrap();
        assert_eq!(source, original);
    }

    #[cfg(unix)]
    #[test]
    fn capped_read_rechecks_the_actual_bytes_after_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(dir.path(), "config.json", "{}");
        let handle = open_read_handle(&path, SymlinkPolicy::Follow).unwrap();
        inspect_capped_source(&handle, &path, 4).unwrap();

        let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
        writer.write_all(b"123").unwrap();
        writer.sync_all().unwrap();

        let error = read_capped_contents(handle, &path, 4).unwrap_err();
        assert_eq!(error.code(), "document_too_large");
    }

    #[cfg(all(unix, feature = "libc"))]
    #[test]
    fn open_capped_can_atomically_refuse_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let target = write_temp(dir.path(), "target.json", r#"{"k": "v"}"#);
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(
            DocumentFile::open_capped_with_policy(&link, None, 1024, SymlinkPolicy::Follow).is_ok()
        );
        let error =
            DocumentFile::open_capped_with_policy(&link, None, 1024, SymlinkPolicy::NoFollow)
                .unwrap_err();
        assert_eq!(error.code(), "document_io_failed");
    }

    #[test]
    fn create_atomic_is_no_clobber_and_returns_a_file_handle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let document = Document::parse(r#"{"port": 993}"#, Format::Json).unwrap();

        let created = DocumentFile::create_atomic(&path, document, CreateOptions::new()).unwrap();
        assert_eq!(created.path(), path);
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"port": 993}"#);

        let replacement = Document::parse(r#"{"port": 1024}"#, Format::Json).unwrap();
        let error =
            DocumentFile::create_atomic(&path, replacement, CreateOptions::new()).unwrap_err();
        assert_eq!(error.code(), "document_target_exists");
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"port": 993}"#);
    }

    #[test]
    fn create_atomic_requires_explicit_replace() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(dir.path(), "config.json", r#"{"port": 993}"#);
        let replacement = Document::parse(r#"{"port": 1024}"#, Format::Json).unwrap();

        let created =
            DocumentFile::create_atomic(&path, replacement, CreateOptions::new().replace())
                .unwrap();

        assert_eq!(created.value_at("port").unwrap(), Value::Integer(1024));
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"port": 1024}"#);
    }

    /// Two write paths must not disagree about permissions. `save()` preserves
    /// the target's mode, so a replace does too unless the caller names one —
    /// otherwise committing through the other verb silently re-permissions a
    /// file, which on a secrets file means widening it.
    #[cfg(unix)]
    #[test]
    fn create_atomic_replace_preserves_the_targets_mode_unless_told_otherwise() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let mode_of = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;

        for original in [0o644, 0o600, 0o640] {
            let path = write_temp(dir.path(), &format!("m{original:o}.json"), r#"{"a": 1}"#);
            fs::set_permissions(&path, fs::Permissions::from_mode(original)).unwrap();
            let replacement = Document::parse(r#"{"a": 2}"#, Format::Json).unwrap();
            DocumentFile::create_atomic(&path, replacement, CreateOptions::new().replace())
                .unwrap();
            assert_eq!(
                mode_of(&path),
                original,
                "replace must keep the file's mode"
            );
        }

        // An explicit mode still wins.
        let path = write_temp(dir.path(), "explicit.json", r#"{"a": 1}"#);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let replacement = Document::parse(r#"{"a": 2}"#, Format::Json).unwrap();
        DocumentFile::create_atomic(
            &path,
            replacement,
            CreateOptions::new().replace().unix_mode(0o600),
        )
        .unwrap();
        assert_eq!(mode_of(&path), 0o600);

        // A first creation still defaults to owner-only.
        let fresh = dir.path().join("fresh.json");
        let document = Document::parse(r#"{"a": 1}"#, Format::Json).unwrap();
        DocumentFile::create_atomic(&fresh, document, CreateOptions::new()).unwrap();
        assert_eq!(mode_of(&fresh), 0o600);
    }

    /// A "safe first commit" that installs a file `open` would reject is not
    /// safe. The format the path resolves to and the document's own format have
    /// to agree before anything is written.
    #[cfg(feature = "toml")]
    #[test]
    fn create_atomic_refuses_a_document_the_path_could_not_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mismatch.toml");
        let document = Document::parse(r#"{"port": 993}"#, Format::Json).unwrap();

        let error = DocumentFile::create_atomic(&path, document, CreateOptions::new()).unwrap_err();

        assert_eq!(error.code(), "document_unsupported_operation");
        assert!(
            !path.exists(),
            "nothing may be written when the check fails"
        );
    }

    #[test]
    fn bare_relative_atomic_paths_use_the_current_directory() {
        let (parent, file_name) =
            atomic_parent_and_name(Path::new("config.json"), "write").unwrap();

        assert_eq!(parent, Path::new("."));
        assert_eq!(file_name, "config.json");
    }

    #[cfg(unix)]
    #[test]
    fn create_atomic_applies_requested_private_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let document = Document::parse(r#"{"port": 993}"#, Format::Json).unwrap();

        DocumentFile::create_atomic(&path, document, CreateOptions::new().unix_mode(0o640))
            .unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    #[test]
    fn create_atomic_rejects_invalid_permission_bits_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let document = Document::parse(r#"{"port": 993}"#, Format::Json).unwrap();

        let error =
            DocumentFile::create_atomic(&path, document, CreateOptions::new().unix_mode(0o1600))
                .unwrap_err();

        assert_eq!(error.code(), "document_invalid_argument");
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn create_atomic_replace_refuses_symlinks_and_hardlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = write_temp(dir.path(), "target.json", r#"{"port": 993}"#);
        let symlink = dir.path().join("symlink.json");
        std::os::unix::fs::symlink(&target, &symlink).unwrap();
        let replacement = Document::parse(r#"{"port": 1024}"#, Format::Json).unwrap();

        let symlink_error = DocumentFile::create_atomic(
            &symlink,
            replacement.clone(),
            CreateOptions::new().replace(),
        )
        .unwrap_err();
        assert_eq!(symlink_error.code(), "document_unsupported_operation");
        assert_eq!(fs::read_to_string(&target).unwrap(), r#"{"port": 993}"#);

        let hardlink = dir.path().join("hardlink.json");
        fs::hard_link(&target, &hardlink).unwrap();
        let hardlink_error =
            DocumentFile::create_atomic(&hardlink, replacement, CreateOptions::new().replace())
                .unwrap_err();
        assert_eq!(hardlink_error.code(), "document_unsupported_operation");
        assert_eq!(fs::read_to_string(&target).unwrap(), r#"{"port": 993}"#);
    }

    #[cfg(unix)]
    #[test]
    fn create_atomic_replace_refuses_a_dangling_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.json");
        let symlink = dir.path().join("dangling.json");
        std::os::unix::fs::symlink(&missing, &symlink).unwrap();
        let replacement = Document::parse(r#"{"port": 1024}"#, Format::Json).unwrap();

        let error =
            DocumentFile::create_atomic(&symlink, replacement, CreateOptions::new().replace())
                .unwrap_err();

        assert_eq!(error.code(), "document_unsupported_operation");
        assert!(
            fs::symlink_metadata(&symlink)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!missing.exists());
    }

    #[test]
    fn edit_rolls_back_memory_and_disk_when_the_closure_fails() {
        let dir = tempfile::tempdir().unwrap();
        let original = r#"{"port": 993}"#;
        let path = write_temp(dir.path(), "config.json", original);
        let mut document = DocumentFile::open(&path, None).unwrap();

        let error = document
            .edit(|draft| {
                draft.set("port", Value::Integer(1024))?;
                Err(DocumentError::InvalidArgument {
                    detail: "validation failed".to_string(),
                })
            })
            .unwrap_err();

        assert_eq!(error.code(), "document_invalid_argument");
        assert_eq!(document.source(), original);
        assert_eq!(document.value_at("port").unwrap(), Value::Integer(993));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
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

    #[test]
    fn decode_and_edit_and_validate_share_one_typed_boundary() {
        #[derive(Debug, serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Config {
            port: u16,
        }

        let dir = tempfile::tempdir().unwrap();
        let original = r#"{"port": 8080}"#;
        let path = write_temp(dir.path(), "config.json", original);
        let mut document = DocumentFile::open(&path, None).unwrap();

        assert_eq!(document.decode::<Config>().unwrap().port, 8080);

        let error = document
            .edit_and_validate::<Config>(|draft| {
                draft.set("port", Value::String("invalid".to_string()))
            })
            .unwrap_err();
        assert_eq!(error.code(), "document_type_mismatch");
        assert_eq!(document.source(), original);
        assert_eq!(fs::read_to_string(&path).unwrap(), original);

        let config = document
            .edit_and_validate::<Config>(|draft| draft.set("port", Value::Unsigned(9090)))
            .unwrap();
        assert_eq!(config.port, 9090);
        assert_eq!(document.value_at("port").unwrap(), Value::Unsigned(9090));
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

    /// A comment documents the value it was written beside. Growing an array
    /// must not copy a neighbour's comment onto a value the author never wrote
    /// it for, and shrinking must not leave a removed element's comment
    /// documenting a survivor. Both are content this module invented.
    #[cfg(feature = "toml")]
    #[test]
    fn toml_array_edits_never_invent_or_misattribute_a_comment() {
        let contents = "paths = [\n  \"one\", # first\n  \"two\", # second\n]\n";

        let mut grown = Document::parse(contents, Format::Toml).unwrap();
        grown
            .set(
                "paths",
                Value::Array(vec![
                    Value::String("a".into()),
                    Value::String("b".into()),
                    Value::String("c".into()),
                ]),
            )
            .unwrap();
        assert_eq!(
            grown.source(),
            "paths = [\n  \"a\", # first\n  \"b\",\n  \"c\", # second\n]\n",
            "an appended element must carry no comment of its own"
        );

        let mut shrunk = Document::parse(contents, Format::Toml).unwrap();
        shrunk
            .set("paths", Value::Array(vec![Value::String("only".into())]))
            .unwrap();
        assert_eq!(
            shrunk.source(),
            "paths = [\n  \"only\", # first\n]\n",
            "the surviving element keeps its own comment, not the removed one's"
        );

        // Same length: every comment stays exactly where the author put it.
        let mut replaced = Document::parse(contents, Format::Toml).unwrap();
        replaced
            .set(
                "paths",
                Value::Array(vec![Value::String("x".into()), Value::String("y".into())]),
            )
            .unwrap();
        assert_eq!(
            replaced.source(),
            "paths = [\n  \"x\", # first\n  \"y\", # second\n]\n"
        );

        // Growing a single-line array keeps the bracket spacing.
        let mut inline = Document::parse("paths = [ \"one\" ]\n", Format::Toml).unwrap();
        inline
            .set(
                "paths",
                Value::Array(vec![
                    Value::String("one".into()),
                    Value::String("two".into()),
                ]),
            )
            .unwrap();
        assert_eq!(inline.source(), "paths = [ \"one\", \"two\" ]\n");
    }

    #[cfg(feature = "toml")]
    #[test]
    fn set_toml_array_preserves_single_line_decor() {
        let contents = "# before\npaths = [ \"old\", 'second', ] # keep this\nother = 42\n";
        let mut document = Document::parse(contents, Format::Toml).unwrap();

        document
            .set(
                "paths",
                Value::Array(vec![
                    Value::String("new".to_string()),
                    Value::String("next".to_string()),
                ]),
            )
            .unwrap();

        assert_eq!(
            document.source(),
            "# before\npaths = [ \"new\", \"next\", ] # keep this\nother = 42\n"
        );
    }

    #[cfg(feature = "toml")]
    #[test]
    fn set_toml_array_preserves_multiline_comments_and_trailing_comma() {
        let contents = "paths = [\n  \"one\", # first\n  \"two\", # second\n]\nother = 42\n";
        let mut document = Document::parse(contents, Format::Toml).unwrap();

        document
            .set(
                "paths",
                Value::Array(vec![
                    Value::String("uno".to_string()),
                    Value::String("dos".to_string()),
                ]),
            )
            .unwrap();

        assert_eq!(
            document.source(),
            "paths = [\n  \"uno\", # first\n  \"dos\", # second\n]\nother = 42\n"
        );
    }

    #[cfg(feature = "toml")]
    #[test]
    fn set_toml_array_element_preserves_its_neighbors() {
        let contents = "paths = [\n  \"one\", # first\n  \"two\", # second\n]\n";
        let mut document = Document::parse(contents, Format::Toml).unwrap();

        document
            .set("paths.1", Value::String("changed".to_string()))
            .unwrap();

        assert_eq!(
            document.source(),
            "paths = [\n  \"one\", # first\n  \"changed\", # second\n]\n"
        );
    }

    #[cfg(feature = "toml")]
    #[test]
    fn set_toml_array_can_become_empty_without_touching_neighbors() {
        let contents = "before = 1\npaths = [ \"one\", ] # list\nafter = 2\n";
        let mut document = Document::parse(contents, Format::Toml).unwrap();

        document.set("paths", Value::Array(Vec::new())).unwrap();

        assert_eq!(
            document.source(),
            "before = 1\npaths = [ ] # list\nafter = 2\n"
        );
    }

    #[cfg(feature = "toml")]
    #[test]
    fn set_toml_inline_table_preserves_layout_and_comments() {
        let contents = "cache = { ttl_s = 1, enabled = true } # cache\nother = 42\n";
        let mut document = Document::parse(contents, Format::Toml).unwrap();
        let replacement = Value::from(serde_json::json!({
            "enabled": false,
            "ttl_s": 60
        }));

        document.set("cache", replacement).unwrap();

        assert_eq!(
            document.source(),
            "cache = { ttl_s = 60, enabled = false } # cache\nother = 42\n"
        );
    }

    #[cfg(feature = "toml")]
    #[test]
    fn set_toml_ordinary_table_preserves_header_and_unrelated_section() {
        let contents = "# lead\n[cache] # cache header\nttl_s = 1 # ttl\nenabled = true\n\n[next]\nvalue = 9\n";
        let mut document = Document::parse(contents, Format::Toml).unwrap();
        let replacement = Value::from(serde_json::json!({
            "enabled": false,
            "ttl_s": 60
        }));

        document.set("cache", replacement).unwrap();

        assert_eq!(
            document.source(),
            "# lead\n[cache] # cache header\nttl_s = 60 # ttl\nenabled = false\n\n[next]\nvalue = 9\n"
        );
    }

    #[cfg(feature = "toml")]
    #[test]
    fn set_toml_collection_preserves_unchanged_datetime_syntax() {
        let contents = "[cache]\nexpires_at = 2026-08-04T12:30:00Z\npaths = [\"one\"]\n";
        let mut document = Document::parse(contents, Format::Toml).unwrap();
        let replacement = document.value_at("cache").unwrap();

        document.set("cache", replacement).unwrap();

        assert_eq!(document.source(), contents);
    }

    #[cfg(feature = "toml")]
    #[test]
    fn set_toml_array_of_tables_is_explicitly_refused() {
        let contents = "[[servers]]\nname = \"one\"\n[[servers]]\nname = \"two\"\n";
        let mut document = Document::parse(contents, Format::Toml).unwrap();
        let replacement = document.value_at("servers").unwrap();

        let error = document.set("servers", replacement).unwrap_err();

        assert_eq!(error.code(), "document_unsupported_operation");
        assert_eq!(document.source(), contents);
    }

    #[cfg(feature = "ini")]
    #[test]
    fn save_refuses_source_its_own_parser_rejects() {
        // The read-back guard, exercised directly: no backend should be able to
        // splice text into this shape any more, so the corrupt source is
        // supplied by hand rather than provoked through a verb.
        let dir = tempfile::tempdir().unwrap();
        let original = "[db]\nhost=localhost\n";
        let path = write_temp(dir.path(), "config.ini", original);
        let doc = DocumentFile::open(&path, None).unwrap();

        let error = doc
            .save_atomic("[db]\nhost=localhost\n\n[db]\nport=5432\n")
            .unwrap_err();
        assert_eq!(error.code(), "document_write_would_corrupt");
        // The whole point is that the guard runs before any bytes land.
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[cfg(feature = "ini")]
    #[test]
    fn save_writes_source_the_parser_accepts() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(dir.path(), "config.ini", "[db]\nhost=localhost\n");
        let mut doc = DocumentFile::open(&path, None).unwrap();

        doc.set("db.port", Value::String("5432".to_string()))
            .unwrap();
        doc.save().unwrap();

        // A key added to a section that is not the file's last one used to be
        // appended at end of file, re-opening a section that had closed.
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[db]\nhost=localhost\nport=5432\n"
        );
        assert!(DocumentFile::open(&path, None).is_ok());
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

    #[test]
    fn unset_is_false_for_anything_already_absent() {
        let mut doc = Document::parse(
            r#"{"service":{"host":"example","ports":[80]}}"#,
            Format::Json,
        )
        .unwrap();

        // Absent is absent, at any depth — the answer must not change with the
        // number of segments the caller had to write to name the same nothing.
        assert!(!doc.unset("service.missing").unwrap());
        assert!(!doc.unset("missing.parent").unwrap());
        assert!(!doc.unset("missing.deeply.nested").unwrap());

        // Still errors: these describe a path no document could satisfy, not a
        // document that happens not to carry the key.
        assert!(doc.unset("service.host.child").is_err()); // through a scalar
        assert!(doc.unset("service.ports.9").is_err()); // index out of range
        assert!(doc.unset(r"service\q").is_err()); // malformed syntax
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn every_markdown_write_verb_is_refused() {
        let source = "# Title\n\nThe lead.\n";
        let mut doc = Document::parse(source, Format::Markdown).unwrap();

        // Reading is the whole point, and works — including by content, which
        // `value_at` gets from the format's own rule.
        assert_eq!(
            doc.value_at("h1.0.text").unwrap(),
            Value::String("Title".to_string())
        );
        assert_eq!(
            doc.value_at("h1.Tit.paragraph.0.text").unwrap(),
            Value::String("The lead.".to_string())
        );

        // Every mutating verb refuses, including the two that would otherwise
        // answer before reaching a backend: `unset` on an absent path (which
        // is `Ok(false)` for a writable format) and `encode`.
        let refusals: Vec<DocumentError> = vec![
            doc.set("h1.0.text", Value::String("New".to_string()))
                .unwrap_err(),
            doc.add("preamble", "x", "type", &[]).unwrap_err(),
            doc.remove("preamble", "x", "type").unwrap_err(),
            doc.unset("h1.0").unwrap_err(),
            doc.unset("nothing.here").unwrap_err(),
            doc.encode().unwrap_err(),
        ];
        for error in refusals {
            assert_eq!(error.code(), "document_unsupported_operation");
            assert!(
                error.to_string().contains("read-only"),
                "refusal must name the reason: {error}"
            );
        }

        // Nothing was staged by any of them.
        assert_eq!(doc.source(), source);
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn markdown_save_never_reaches_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(dir.path(), "README.md", "# Title\n");
        // `.md` resolves to no format, so the reading has to be named.
        assert!(DocumentFile::open(&path, None).is_err());

        let doc = DocumentFile::open(&path, Some(Format::Markdown)).unwrap();
        // Even an unmodified save, whose bytes would be identical, is refused.
        let error = doc.save().unwrap_err();
        assert_eq!(error.code(), "document_unsupported_operation");
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Title\n");
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn yaml_write_rejects_cst_ambiguous_mapping_segments() {
        let mut numeric = Document::parse("\"123\": value\n", Format::Yaml).unwrap();
        assert!(
            numeric
                .set("123", Value::String("changed".to_string()))
                .is_err()
        );
        assert!(numeric.unset("123").is_err());

        let mut bracketed = Document::parse("\"a[0]\": value\n", Format::Yaml).unwrap();
        assert!(
            bracketed
                .set("a[0]", Value::String("changed".to_string()))
                .is_err()
        );
    }
}
