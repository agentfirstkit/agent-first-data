//! `afdata guard <TYPE> <VALUE> [--under ROOT]`: validate a value that is
//! about to flow into a destructive Bash verb (`rm -rf`, `mv`, …) and print
//! its normalized absolute path on success.
//!
//! This module is deliberately CLI-only: it is not `mod`-included from
//! `rust/src/lib.rs`, so nothing here reaches the four-language SDK surface
//! or the `api-surface` manifest. See the design note this implements for why
//! that boundary is a decision, not an oversight.
//!
//! Every public-ish item here takes its environment (`cwd`, `$HOME`, the
//! system temp roots) as plain arguments rather than reading it directly, so
//! unit tests can supply fabricated values without mutating this process's
//! real `$PWD`/env — both of which are global, shared, and unsafe to mutate
//! from parallel `#[test]`s. The one place that reads the real environment is
//! [`guard_from_environment`], which the CLI dispatch calls exactly once.

use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuardType {
    Path,
    TmpPath,
    CwdPath,
}

impl GuardType {
    /// Parse a `TYPE` positional already validated by the closed-world
    /// registry against exactly these three spellings. Returns `None` for
    /// anything else so the caller can report a dispatch-table defect instead
    /// of guessing a fallback type.
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "path" => Some(Self::Path),
            "tmp_path" => Some(Self::TmpPath),
            "cwd_path" => Some(Self::CwdPath),
            _ => None,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::TmpPath => "tmp_path",
            Self::CwdPath => "cwd_path",
        }
    }
}

/// Which containment requirement a verdict is about. A `TYPE` names one whose
/// root the guard derives for itself; `--under` names one the caller supplied.
/// They are checked identically, and an invocation carrying both must satisfy
/// both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Containment {
    Type(GuardType),
    Under,
}

impl Containment {
    const fn label(self) -> &'static str {
        match self {
            Self::Type(guard_type) => guard_type.name(),
            Self::Under => "--under",
        }
    }
}

/// Why `guard` refused `VALUE`. Each variant is one stable, catalogued error
/// code — never a message-matching contract.
#[derive(Debug)]
pub(crate) enum GuardError {
    EmptyValue,
    ContainsNewline,
    /// `--under` was given a root that is not usable as one: empty, blank, or
    /// carrying a newline. An unset shell variable landing here would
    /// otherwise silently mean "under the current directory".
    InvalidRoot,
    /// `VALUE` spells a `..` segment. Normalization would resolve it away and
    /// hand back a lawful-looking path one or more levels up, which is exactly
    /// how a containment root gets escaped without anything looking wrong: a
    /// caller guarding `"$dir/$name"` under a root above `$dir` sees `..` walk
    /// out of `$dir` and still land inside the root. A destructive operand is
    /// never legitimately addressed by traversal, so it is refused before the
    /// path machinery can launder it.
    TraversalSegment,
    /// Hit the built-in reject set shared by every type: the filesystem
    /// root, `$HOME` itself or one of its ancestors, or the current
    /// directory itself or one of its ancestors.
    Rejected {
        normalized: PathBuf,
    },
    /// Normalized cleanly, but a containment requirement wants it strictly
    /// inside a root and it is not.
    OutsideContainment {
        containment: Containment,
        normalized: PathBuf,
    },
    /// Sits inside the containment root, but its final segment is a symlink
    /// pointing outside — so a verb that follows symlinks would act beyond
    /// the root the type promised.
    SymlinkEscapesContainment {
        containment: Containment,
        normalized: PathBuf,
        target: PathBuf,
    },
    /// The real filesystem could not be consulted (e.g. `$PWD` itself is
    /// unreadable). Distinct from the other variants: this is an environment
    /// fault, not a verdict about `VALUE`.
    Io(io::Error),
}

impl GuardError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::EmptyValue => "guard_empty_value",
            Self::ContainsNewline => "guard_contains_newline",
            Self::InvalidRoot => "guard_invalid_root",
            Self::TraversalSegment => "guard_traversal_segment",
            Self::Rejected { .. } => "guard_rejected_target",
            Self::OutsideContainment { .. } => "guard_outside_containment",
            Self::SymlinkEscapesContainment { .. } => "guard_symlink_escapes_containment",
            Self::Io(_) => "guard_target_unreadable",
        }
    }
}

impl std::fmt::Display for GuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyValue => write!(f, "VALUE must not be empty or blank"),
            Self::ContainsNewline => write!(f, "VALUE must not contain a newline"),
            Self::InvalidRoot => write!(
                f,
                "--under ROOT must not be empty, blank, or contain a newline"
            ),
            Self::TraversalSegment => write!(
                f,
                "VALUE must not contain a `..` segment; name the target directly, because \
                 resolving `..` would silently move the operand out of the directory it appears \
                 to sit in"
            ),
            Self::Rejected { normalized } => write!(
                f,
                "refusing to guard `{}`: it is the filesystem root, the home directory, the \
                 current directory, or an ancestor of one of those",
                normalized.display()
            ),
            Self::OutsideContainment {
                containment,
                normalized,
            } => write!(
                f,
                "refusing to guard `{}`: it is not strictly inside the {} containment root",
                normalized.display(),
                containment.label()
            ),
            Self::SymlinkEscapesContainment {
                containment,
                normalized,
                target,
            } => write!(
                f,
                "refusing to guard `{}`: its final segment is a symlink to `{}`, outside the {} \
                 containment root, so a verb that follows symlinks would act outside it",
                normalized.display(),
                target.display(),
                containment.label()
            ),
            Self::Io(err) => write!(
                f,
                "could not resolve VALUE against the real filesystem: {err}"
            ),
        }
    }
}

/// The real-environment entry point: reads `$PWD`, `$HOME`/`%USERPROFILE%`,
/// and the platform's fixed temp-area structure, then delegates to
/// [`evaluate`]. This is the only function in the module that touches actual
/// process environment.
pub(crate) fn guard_from_environment(
    guard_type: GuardType,
    value: &str,
    under: Option<&str>,
) -> Result<PathBuf, GuardError> {
    let cwd = std::env::current_dir().map_err(GuardError::Io)?;
    // Canonicalized for the same reason `cwd` is: `$HOME` is routinely handed
    // out through a symlink (a CI or sandbox home under a symlinked temp area,
    // a site where `/home` is a link), and comparing a canonical target
    // against a non-canonical home would silently drop the home entry from the
    // reject set. Falling back to the raw value keeps a home that cannot be
    // resolved at all in the set rather than dropping it.
    let home = home_dir().map(|home| resolve_missing_ok(&home).unwrap_or(home));
    let temp_roots = candidate_temp_roots();
    evaluate(guard_type, value, under, &cwd, home.as_deref(), &temp_roots)
}

/// The pure core: given an already-resolved `cwd`, an optional home
/// directory, and a set of (not-yet-canonicalized) temp-root candidates,
/// decide whether `value` may pass, and if so return its normalized absolute
/// path.
pub(crate) fn evaluate(
    guard_type: GuardType,
    value: &str,
    under: Option<&str>,
    cwd: &Path,
    home: Option<&Path>,
    temp_root_candidates: &[PathBuf],
) -> Result<PathBuf, GuardError> {
    if value.is_empty() {
        return Err(GuardError::EmptyValue);
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(GuardError::ContainsNewline);
    }
    // An all-whitespace value is the same accident as an empty one — a
    // variable filled from a command that printed only blanks — and no
    // destructive operand is legitimately named by it.
    if value.trim().is_empty() {
        return Err(GuardError::EmptyValue);
    }

    // Checked before normalization, because normalization is precisely what
    // would hide it: `..` resolves away and yields a lawful-looking absolute
    // path somewhere above where the caller was looking. Only VALUE is held to
    // this. ROOT is a position to compare against rather than an operand any
    // verb acts on, it has no blast radius to preserve, and it is resolved in
    // full — so traversal inside it changes nothing about what is protected.
    if Path::new(value)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(GuardError::TraversalSegment);
    }

    let normalized = normalize_target(value, cwd).map_err(GuardError::Io)?;
    let canonical_cwd = resolve_missing_ok(cwd).map_err(GuardError::Io)?;

    if is_rejected(&normalized, &canonical_cwd, home) {
        return Err(GuardError::Rejected { normalized });
    }

    let requirements =
        containment_requirements(guard_type, under, &canonical_cwd, temp_root_candidates, cwd)?;
    if requirements.is_empty() {
        return Ok(normalized);
    }

    // Only consulted once, and only where a containment root exists to escape
    // from: `normalize_target` deliberately leaves a final-segment symlink
    // unresolved so a verb acts on the link itself, which is right for `rm`
    // and `mv` but not for the verbs that follow it (`>`, `chmod`, `cp`).
    let link_target = symlink_target(&normalized)?;

    for (containment, roots) in &requirements {
        if !roots
            .iter()
            .any(|root| is_strictly_under(&normalized, root))
        {
            return Err(GuardError::OutsideContainment {
                containment: *containment,
                normalized,
            });
        }
        if let Some(target) = &link_target
            && !roots.iter().any(|root| is_strictly_under(target, root))
        {
            return Err(GuardError::SymlinkEscapesContainment {
                containment: *containment,
                normalized,
                target: target.clone(),
            });
        }
    }

    Ok(normalized)
}

/// Every containment root set this invocation must satisfy: the one its `TYPE`
/// names (none, for `path`), plus the one `--under` supplies. Each entry is a
/// set of alternatives — being strictly under any one of them satisfies that
/// requirement — but every entry must be satisfied.
fn containment_requirements(
    guard_type: GuardType,
    under: Option<&str>,
    canonical_cwd: &Path,
    temp_root_candidates: &[PathBuf],
    cwd: &Path,
) -> Result<Vec<(Containment, Vec<PathBuf>)>, GuardError> {
    let mut requirements: Vec<(Containment, Vec<PathBuf>)> = Vec::new();
    match guard_type {
        GuardType::Path => {}
        GuardType::CwdPath => requirements.push((
            Containment::Type(guard_type),
            vec![canonical_cwd.to_path_buf()],
        )),
        GuardType::TmpPath => requirements.push((
            Containment::Type(guard_type),
            // A candidate that does not resolve on this machine (a per-user
            // directory that does not exist here) is dropped, not an error.
            temp_root_candidates
                .iter()
                .filter_map(|candidate| resolve_missing_ok(candidate).ok())
                .collect(),
        )),
    }
    if let Some(root) = under {
        requirements.push((Containment::Under, vec![resolve_root(root, cwd)?]));
    }
    Ok(requirements)
}

/// Normalize an `--under` root. Unlike `VALUE`, a root *is* fully resolved
/// including its final segment: it is a place to compare against, never an
/// operand a verb will act on, so there is no blast radius to preserve — and
/// leaving it unresolved would make a symlinked root (macOS's `/var`) fail to
/// contain its own real children.
fn resolve_root(root: &str, cwd: &Path) -> Result<PathBuf, GuardError> {
    if root.trim().is_empty() || root.contains('\n') || root.contains('\r') {
        return Err(GuardError::InvalidRoot);
    }
    let candidate = Path::new(root);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    };
    resolve_missing_ok(&absolute).map_err(GuardError::Io)
}

/// Where `path`'s final segment points, if it is a symlink: `None` when there
/// is nothing there or it is not a link. A dangling link still reports its
/// target — a verb that creates through it (`>`) would land exactly there.
fn symlink_target(path: &Path) -> Result<Option<PathBuf>, GuardError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        // A target that does not exist is explicitly allowed to pass; any
        // other stat failure means the guard cannot classify what it is about
        // to hand over, which is an environment fault, not a pass.
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(GuardError::Io(err)),
    };
    if !metadata.file_type().is_symlink() {
        return Ok(None);
    }
    let link = std::fs::read_link(path).map_err(GuardError::Io)?;
    let absolute = if link.is_absolute() {
        link
    } else {
        match path.parent() {
            Some(parent) => parent.join(link),
            None => return Ok(None),
        }
    };
    resolve_missing_ok(&absolute)
        .map(Some)
        .map_err(GuardError::Io)
}

/// The built-in reject set shared by every type (design's "所有类型共享的
///基础检查" item 4): the filesystem/drive/UNC root, `$HOME` itself or an
/// ancestor of it, and the current directory itself or an ancestor of it.
///
/// A filesystem root is universally `path.parent().is_none()` — true for
/// Unix `/`, a Windows drive root (`C:\`), and a Windows UNC share root
/// (`\\server\share`) alike, so no per-platform special case is needed here.
fn is_rejected(normalized: &Path, canonical_cwd: &Path, home: Option<&Path>) -> bool {
    if normalized.parent().is_none() {
        return true;
    }
    if canonical_cwd.starts_with(normalized) {
        return true;
    }
    if let Some(home) = home
        && home.starts_with(normalized)
    {
        return true;
    }
    false
}

/// Whether `target` sits strictly inside `root`: `root` itself does not
/// count (design: "包含根本身也被拒绝"), and containment is decided
/// component-wise via [`Path::starts_with`], never by string prefix.
fn is_strictly_under(target: &Path, root: &Path) -> bool {
    target != root && target.starts_with(root)
}

/// Normalize `value` (already known non-empty and newline-free) into an
/// absolute path.
///
/// The parent is canonicalized — `..`, relative segments, and parent-level
/// symlinks are all resolved against the real filesystem — but the final
/// path segment never is, so a symlink named by `VALUE` is guarded (and later
/// acted on) as itself, not as whatever it points to. A destructive verb
/// resolving through the guard's own output must delete the link, not its
/// target; resolving the final segment here would make the guard the thing
/// that upgrades the blast radius.
///
/// A raw last component of `.` is stripped before this split runs (it names
/// the same entry as no trailing `.` at all, so it must not force resolution
/// through a symlink any more than the bare form would); `..` is not
/// stripped, because unlike `.` it asks to leave the named entry, which does
/// require resolving it as a real, followed directory to find what it is the
/// parent of.
fn normalize_target(value: &str, cwd: &Path) -> io::Result<PathBuf> {
    let candidate = Path::new(value);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    };

    let mut components: Vec<Component<'_>> = absolute.components().collect();
    while matches!(components.last(), Some(Component::CurDir)) {
        components.pop();
    }

    if matches!(components.last(), Some(Component::Normal(_))) {
        let Some(Component::Normal(name)) = components.pop() else {
            unreachable!("just matched Some(Component::Normal(_)) above");
        };
        let parent: PathBuf = components.iter().collect();
        let canonical_parent = resolve_missing_ok(&parent)?;
        spell_existing_leaf(&canonical_parent, name)
    } else {
        // Ends in `..`, is bare root/prefix, or emptied out entirely by the
        // `.` strip above (impossible in practice: `cwd` is always absolute,
        // so it contributes at least a root component) — either way there is
        // no unresolved leaf name to protect, so the whole thing resolves.
        let whole: PathBuf = components.iter().collect();
        resolve_missing_ok(&whole)
    }
}

/// Join `name` onto `parent` the way the filesystem spells it, without
/// following a final symlink.
///
/// On a case-insensitive filesystem (APFS, NTFS) `/Users/ROGER` names the home
/// directory as surely as `/Users/roger` does, and Windows also answers to 8.3
/// short names. Every comparison after this one is component-wise, so a leaf
/// kept as typed would walk a protected directory straight past the reject
/// set. A real entry is canonicalized; a symlink is looked up among its
/// parent's entries instead, so the link itself — not its target — is what
/// comes back. A missing leaf has no other spelling and is kept as typed.
///
/// The symlink lookup matches the entry by what it is, not by how its name
/// compares: a filesystem may answer to a spelling that no simple string rule
/// relates to the stored one (APFS finds a link stored in decomposed form when
/// asked in composed form). On Unix the entry is the one with the same device
/// and inode as the typed path; elsewhere names are compared, exactly and then
/// case-insensitively.
///
/// A symlink is never a component of a canonical path, and `parent` already is
/// one, so the leaf's spelling cannot move a symlink into or out of the reject
/// set or a containment root — those are canonical too. When the parent cannot
/// be listed (an execute-only directory), the typed spelling is therefore kept
/// rather than refusing a link the filesystem has already shown exists.
fn spell_existing_leaf(parent: &Path, name: &std::ffi::OsStr) -> io::Result<PathBuf> {
    let joined = parent.join(name);
    let metadata = match std::fs::symlink_metadata(&joined) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(joined),
        Err(err) => return Err(err),
    };
    if !metadata.file_type().is_symlink() {
        return std::fs::canonicalize(&joined).map(plain_form);
    }
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => return Ok(joined),
        Err(err) => return Err(err),
    };
    let mut matches = Vec::new();
    for entry in entries {
        let entry = entry?;
        // The entry stored under exactly the typed name is the one that was
        // just stat'd, on every platform.
        if entry.file_name() == name {
            return Ok(joined);
        }
        if names_same_entry(&entry, name, &metadata)? {
            matches.push(entry.file_name());
        }
    }
    match matches.as_slice() {
        [only] => Ok(parent.join(only)),
        // No unique entry means the guard cannot say what it is handing over,
        // which is a refusal rather than a pass.
        _ => Err(io::Error::other(format!(
            "cannot determine the on-disk spelling of `{}`",
            joined.display()
        ))),
    }
}

/// Whether directory `entry` is the symlink `wanted` describes: the same
/// device and inode, read without following the entry.
#[cfg(unix)]
fn names_same_entry(
    entry: &std::fs::DirEntry,
    _typed: &std::ffi::OsStr,
    wanted: &std::fs::Metadata,
) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    // `DirEntry::metadata` does not traverse a symlink on Unix.
    let found = match entry.metadata() {
        Ok(found) => found,
        // Removed between the listing and the stat: it is not the link.
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    Ok(found.dev() == wanted.dev() && found.ino() == wanted.ino())
}

/// Whether directory `entry` is the symlink typed as `typed`. Without a stable
/// file identity in std here, the name is compared case-insensitively; the
/// caller has already taken an exact match.
#[cfg(not(unix))]
fn names_same_entry(
    entry: &std::fs::DirEntry,
    typed: &std::ffi::OsStr,
    _wanted: &std::fs::Metadata,
) -> io::Result<bool> {
    let entry_name = entry.file_name();
    Ok(entry_name.to_string_lossy().to_lowercase() == typed.to_string_lossy().to_lowercase())
}

/// `realpath`-with-missing-components-allowed: canonicalize (resolve `..`,
/// `.`, and symlinks against the real filesystem) the longest existing
/// prefix of `path`, then lexically apply whatever raw components remain —
/// there is no real filesystem left to consult for them, so nothing beyond
/// simple `.`/`..` bookkeeping is lost by doing that lexically.
///
/// `path` must already be absolute. Walking forward from the root (rather
/// than shrinking from the end) is both simpler and cannot overshoot: a path
/// cannot exist unless its parent does, so the first component that fails to
/// canonicalize is exactly the start of the non-existent tail.
fn resolve_missing_ok(path: &Path) -> io::Result<PathBuf> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Ok(plain_form(canonical));
    }

    let components: Vec<Component<'_>> = path.components().collect();
    let root_len = components
        .iter()
        .take_while(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
        .count();
    let root: PathBuf = components[..root_len].iter().collect();
    let mut resolved = std::fs::canonicalize(&root)?;
    let mut existing_len = root_len;

    for extended_len in (root_len + 1)..=components.len() {
        let candidate: PathBuf = components[..extended_len].iter().collect();
        match std::fs::canonicalize(&candidate) {
            Ok(canonical) => {
                resolved = canonical;
                existing_len = extended_len;
            }
            Err(_) => break,
        }
    }

    for component in &components[existing_len..] {
        match component {
            Component::Normal(name) => resolved.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Ok(plain_form(resolved))
}

/// Canonicalization on Windows answers in the `\\?\` verbatim form. That is a
/// real path — the shell's file tests and `rm` both take it — but it is not the
/// spelling this command exists to hand out. The value printed here is the
/// operand a caller passes to a verb, and Windows' own shell item API refuses
/// the verbatim form outright, while `.NET` reads its leading backslashes as a
/// UNC share. A guarded operand that the platform's own APIs will not accept is
/// not a guarded operand, so the drive form is restored.
///
/// Only `\\?\C:\…` is rewritten. `\\?\UNC\server\share` and device paths keep
/// their prefix, because for those the plain spelling does not name the same
/// thing. Paths beyond `MAX_PATH` lose verbatim addressing, which is the one
/// cost of this and is not a real loss: the APIs this exists to feed cannot
/// address them either way.
#[cfg(windows)]
fn plain_form(path: PathBuf) -> PathBuf {
    use std::ffi::OsString;
    use std::path::Prefix;

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };
    let Prefix::VerbatimDisk(letter) = prefix.kind() else {
        return path;
    };
    let mut plain = OsString::from(format!("{}:", letter as char));
    // Everything after the prefix, starting with the root separator, so this
    // stays byte-exact rather than going through a lossy string conversion.
    plain.push(components.as_path().as_os_str());
    PathBuf::from(plain)
}

#[cfg(not(windows))]
fn plain_form(path: PathBuf) -> PathBuf {
    path
}

/// `$HOME` on Unix, `%USERPROFILE%` on Windows. Read from the environment —
/// unlike the temp-area roots below, there is no portable non-env system API
/// for "the current user's home directory" without pulling in `getpwuid`
/// (Unix) or a known-folder call (Windows) for what the design treats as a
/// best-effort denylist entry, not the structural per-type guarantee that
/// motivates `tmp_path` refusing to read `TMPDIR`/`TMP`/`TEMP`. See the
/// implementation notes this ships with for the reasoning.
#[cfg(unix)]
fn home_dir() -> Option<PathBuf> {
    non_empty_env("HOME")
}

#[cfg(windows)]
fn home_dir() -> Option<PathBuf> {
    non_empty_env("USERPROFILE")
}

fn non_empty_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|value| !value.as_os_str().is_empty())
}

/// `tmp_path`'s containment roots, in the platform's own fixed structure or
/// via a system API — never `TMPDIR`/`TMP`/`TEMP`. Candidates that do not
/// resolve on this machine (a per-user directory that does not exist here)
/// are silently dropped by [`evaluate`]'s caller, not treated as errors.
#[cfg(target_os = "macos")]
fn candidate_temp_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/private/tmp"),
        PathBuf::from("/private/var/tmp"),
    ];
    if let Some(per_user) = darwin_user_temp_dir() {
        roots.push(per_user);
    }
    roots
}

#[cfg(all(target_os = "macos", feature = "libc"))]
fn darwin_user_temp_dir() -> Option<PathBuf> {
    // SAFETY: a null buffer with length 0 only asks `confstr` for the
    // required buffer size per `man confstr`; it writes nothing.
    let needed = unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, std::ptr::null_mut(), 0) };
    if needed == 0 {
        return None;
    }
    let mut buffer = vec![0u8; needed];
    // SAFETY: `buffer` has exactly the `needed` bytes just queried above.
    let written = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if written == 0 || written > buffer.len() {
        return None;
    }
    // `written` (per `man confstr`) includes the trailing NUL.
    buffer.truncate(written.saturating_sub(1));
    let path = PathBuf::from(String::from_utf8(buffer).ok()?);
    let path = resolve_missing_ok(&path).ok()?;
    // Some runtimes return TMPDIR through confstr too. An API result outside
    // the platform's per-user temp layout must not turn an arbitrary directory
    // into a trusted cleanup root. Resolve first so a symlink cannot bypass it.
    let relative = path.strip_prefix("/private/var/folders").ok()?;
    (relative.components().count() == 3 && relative.file_name() == Some(std::ffi::OsStr::new("T")))
        .then_some(path)
}

#[cfg(all(target_os = "macos", not(feature = "libc")))]
fn darwin_user_temp_dir() -> Option<PathBuf> {
    None
}

#[cfg(all(unix, not(target_os = "macos")))]
fn candidate_temp_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/tmp"), PathBuf::from("/var/tmp")];
    if let Some(uid) = current_uid() {
        roots.push(PathBuf::from(format!("/run/user/{uid}")));
    }
    roots
}

#[cfg(all(unix, not(target_os = "macos"), feature = "libc"))]
fn current_uid() -> Option<u32> {
    // SAFETY: `getuid` takes no arguments and cannot fail.
    Some(unsafe { libc::getuid() })
}

#[cfg(all(unix, not(target_os = "macos"), not(feature = "libc")))]
fn current_uid() -> Option<u32> {
    None
}

#[cfg(windows)]
fn candidate_temp_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from(r"C:\Windows\Temp")];
    if let Some(local_app_data) = windows_known_folder::local_app_data() {
        roots.push(local_app_data.join("Temp"));
    }
    roots
}

/// Minimal, dependency-free FFI for `SHGetKnownFolderPath`, so the Windows
/// per-user temp root comes from the known-folder API the design calls for
/// rather than from `%TMP%`/`%TEMP%` (or even `%LOCALAPPDATA%`, which those
/// two are commonly derived from on a default install).
#[cfg(windows)]
mod windows_known_folder {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    // FOLDERID_LocalAppData: {F1B32785-6FBA-4FCF-9D55-7B8E7F157091}
    const FOLDERID_LOCAL_APP_DATA: Guid = Guid {
        data1: 0xF1B3_2785,
        data2: 0x6FBA,
        data3: 0x4FCF,
        data4: [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
    };

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            rfid: *const Guid,
            flags: u32,
            token: *mut c_void,
            out_path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(pv: *mut c_void);
    }

    pub(super) fn local_app_data() -> Option<PathBuf> {
        let mut raw: *mut u16 = std::ptr::null_mut();
        // SAFETY: `raw` is an out-parameter; on a non-negative HRESULT the API
        // fills it with a `CoTaskMemAlloc`'d, NUL-terminated wide string that
        // we free unconditionally below.
        let hresult = unsafe {
            SHGetKnownFolderPath(&FOLDERID_LOCAL_APP_DATA, 0, std::ptr::null_mut(), &mut raw)
        };
        if hresult < 0 || raw.is_null() {
            return None;
        }
        // SAFETY: `raw` is non-null and NUL-terminated, per the API contract
        // on success; we only read up to that terminator.
        let len = unsafe {
            let mut len = 0usize;
            while *raw.add(len) != 0 {
                len += 1;
            }
            len
        };
        // SAFETY: `[raw, raw + len)` was just measured as the initialized,
        // in-bounds portion of the buffer above.
        let units = unsafe { std::slice::from_raw_parts(raw, len) };
        let text = std::ffi::OsString::from_wide(units);
        // SAFETY: `raw` was allocated by `SHGetKnownFolderPath` via
        // `CoTaskMemAlloc` and is freed exactly once, here.
        unsafe { CoTaskMemFree(raw.cast()) };
        Some(PathBuf::from(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().expect("failed to create tempdir fixture")
    }

    /// [`evaluate`] with no `--under`, which is what every case about the
    /// `TYPE` vocabulary itself is testing. The `--under` cases below call
    /// [`evaluate`] directly.
    fn evaluate_typed(
        guard_type: GuardType,
        value: &str,
        cwd: &Path,
        home: Option<&Path>,
        temp_root_candidates: &[PathBuf],
    ) -> Result<PathBuf, GuardError> {
        evaluate(guard_type, value, None, cwd, home, temp_root_candidates)
    }

    fn symlink_dir(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(target, link).unwrap();
    }

    #[test]
    fn empty_and_newline_values_are_rejected_before_touching_the_filesystem() {
        let cwd = temp();
        assert!(matches!(
            evaluate_typed(GuardType::Path, "", cwd.path(), None, &[]),
            Err(GuardError::EmptyValue)
        ));
        assert!(matches!(
            evaluate_typed(GuardType::Path, "a\nb", cwd.path(), None, &[]),
            Err(GuardError::ContainsNewline)
        ));
        assert!(matches!(
            evaluate_typed(GuardType::Path, "a\rb", cwd.path(), None, &[]),
            Err(GuardError::ContainsNewline)
        ));
    }

    #[test]
    fn filesystem_root_is_always_rejected() {
        let cwd = temp();
        #[cfg(unix)]
        let root = "/";
        #[cfg(windows)]
        let root = "C:\\";
        let result = evaluate_typed(GuardType::Path, root, cwd.path(), None, &[]);
        assert!(
            matches!(result, Err(GuardError::Rejected { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn home_itself_and_its_ancestors_are_rejected_for_every_type() {
        let cwd = temp();
        let home = temp();
        let home_path = resolve_missing_ok(home.path()).unwrap();
        let ancestor = home_path.parent().unwrap().to_path_buf();
        for guard_type in [GuardType::Path, GuardType::TmpPath, GuardType::CwdPath] {
            let home_result = evaluate_typed(
                guard_type,
                home_path.to_str().unwrap(),
                cwd.path(),
                Some(&home_path),
                std::slice::from_ref(&home_path),
            );
            assert!(
                matches!(home_result, Err(GuardError::Rejected { .. })),
                "{guard_type:?}: {home_result:?}"
            );
            let ancestor_result = evaluate_typed(
                guard_type,
                ancestor.to_str().unwrap(),
                cwd.path(),
                Some(&home_path),
                std::slice::from_ref(&ancestor),
            );
            assert!(
                matches!(ancestor_result, Err(GuardError::Rejected { .. })),
                "{guard_type:?}: {ancestor_result:?}"
            );
        }
    }

    #[test]
    fn cwd_itself_and_its_ancestors_are_rejected_for_every_type() {
        let root = temp();
        let cwd = root.path().join("nested");
        std::fs::create_dir(&cwd).unwrap();
        for guard_type in [GuardType::Path, GuardType::TmpPath, GuardType::CwdPath] {
            let cwd_result = evaluate_typed(guard_type, cwd.to_str().unwrap(), &cwd, None, &[]);
            assert!(
                matches!(cwd_result, Err(GuardError::Rejected { .. })),
                "{cwd_result:?}"
            );
            let ancestor_result =
                evaluate_typed(guard_type, root.path().to_str().unwrap(), &cwd, None, &[]);
            assert!(
                matches!(ancestor_result, Err(GuardError::Rejected { .. })),
                "{ancestor_result:?}"
            );
        }
    }

    #[test]
    fn plain_path_type_allows_any_real_directory_outside_the_reject_set() {
        let cwd = temp();
        let elsewhere = temp();
        let result = evaluate_typed(
            GuardType::Path,
            elsewhere.path().to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        );
        let expected = resolve_missing_ok(elsewhere.path()).unwrap();
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn cwd_path_rejects_targets_outside_cwd_and_accepts_children() {
        let cwd = temp();
        let elsewhere = temp();
        let outside = evaluate_typed(
            GuardType::CwdPath,
            elsewhere.path().to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        );
        assert!(
            matches!(outside, Err(GuardError::OutsideContainment { .. })),
            "{outside:?}"
        );

        let child = cwd.path().join("child");
        std::fs::create_dir(&child).unwrap();
        let inside = evaluate_typed(GuardType::CwdPath, "child", cwd.path(), None, &[]);
        assert_eq!(inside.unwrap(), resolve_missing_ok(&child).unwrap());
    }

    #[test]
    fn tmp_path_rejects_real_directories_outside_every_candidate_root() {
        let cwd = temp();
        let tmp_root = temp();
        let elsewhere = temp();
        let outside = evaluate_typed(
            GuardType::TmpPath,
            elsewhere.path().to_str().unwrap(),
            cwd.path(),
            None,
            &[tmp_root.path().to_path_buf()],
        );
        assert!(
            matches!(outside, Err(GuardError::OutsideContainment { .. })),
            "{outside:?}"
        );

        let child = tmp_root.path().join("work");
        std::fs::create_dir(&child).unwrap();
        let inside = evaluate_typed(
            GuardType::TmpPath,
            child.to_str().unwrap(),
            cwd.path(),
            None,
            &[tmp_root.path().to_path_buf()],
        );
        assert_eq!(inside.unwrap(), resolve_missing_ok(&child).unwrap());
    }

    #[test]
    fn tmp_path_containment_root_itself_is_rejected_not_just_ancestors() {
        let cwd = temp();
        let tmp_root = temp();
        let result = evaluate_typed(
            GuardType::TmpPath,
            tmp_root.path().to_str().unwrap(),
            cwd.path(),
            None,
            &[tmp_root.path().to_path_buf()],
        );
        assert!(
            matches!(result, Err(GuardError::OutsideContainment { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn nonexistent_target_passes_lexically_when_otherwise_lawful() {
        let cwd = temp();
        let missing = cwd.path().join("does-not-exist-yet");
        let result = evaluate_typed(
            GuardType::CwdPath,
            missing.to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        );
        let canonical_cwd = resolve_missing_ok(cwd.path()).unwrap();
        assert_eq!(result.unwrap(), canonical_cwd.join("does-not-exist-yet"));
    }

    #[test]
    fn nonexistent_target_under_a_nonexistent_parent_still_passes() {
        let cwd = temp();
        let missing = cwd.path().join("a").join("b").join("c");
        let result = evaluate_typed(
            GuardType::CwdPath,
            missing.to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        );
        let canonical_cwd = resolve_missing_ok(cwd.path()).unwrap();
        assert_eq!(result.unwrap(), canonical_cwd.join("a").join("b").join("c"));
    }

    /// The value handed back is the one a caller pastes into a verb, so it
    /// must be a spelling the platform's own APIs accept. Canonicalization
    /// answers in the `\\?\` verbatim form on Windows, which the shell's item
    /// API refuses outright and .NET reads as a UNC share — a guarded operand
    /// nothing downstream can use. Pinned here because the cost of losing it is
    /// paid on a platform most of this suite's runs never touch.
    #[cfg(windows)]
    #[test]
    fn windows_output_is_the_drive_form_not_the_verbatim_one() {
        let cwd = temp();
        let nested = cwd.path().join("out");
        std::fs::create_dir_all(&nested).unwrap();

        let resolved = resolve_missing_ok(&nested).unwrap();
        let printed = resolved.to_string_lossy();
        assert!(
            !printed.starts_with(r"\\?\"),
            "guard printed the verbatim form: {printed}"
        );
        let mut characters = printed.chars();
        assert!(characters.next().is_some_and(|c| c.is_ascii_alphabetic()));
        assert_eq!(characters.next(), Some(':'));

        // Still the same directory, not merely a shorter string.
        assert_eq!(
            std::fs::canonicalize(&resolved).unwrap(),
            std::fs::canonicalize(&nested).unwrap()
        );
    }

    /// Normalization still cancels `..`, and must: `--under ROOT` is resolved
    /// through the same machinery, and a root handed over via traversal has to
    /// name the right directory. What changed is that VALUE no longer reaches
    /// it — `evaluate` refuses a traversal segment first, so this laundering
    /// can no longer be aimed at a destructive operand.
    #[test]
    fn dot_dot_lexically_cancels_a_nonexistent_component() {
        let cwd = temp();
        let value = cwd.path().join("missing-dir").join("..").join("sibling");
        let normalized = normalize_target(value.to_str().unwrap(), cwd.path()).unwrap();
        let canonical_cwd = resolve_missing_ok(cwd.path()).unwrap();
        assert_eq!(normalized, canonical_cwd.join("sibling"));

        let refused = evaluate_typed(
            GuardType::CwdPath,
            value.to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        );
        assert!(
            matches!(refused, Err(GuardError::TraversalSegment)),
            "{refused:?}"
        );
    }

    #[test]
    fn embedded_dot_dot_through_a_real_symlink_resolves_the_true_parent() {
        // link -> child, so on a system that resolves the link before
        // applying `..`, `link/../sibling` lands next to `child`'s real
        // parent rather than textually next to `link`.
        let root = temp();
        let real_parent = root.path().join("real_parent");
        std::fs::create_dir(&real_parent).unwrap();
        let child = real_parent.join("child");
        std::fs::create_dir(&child).unwrap();
        let link = root.path().join("link");
        symlink_dir(&child, &link);

        let value = link.join("..").join("sibling");
        let cwd = temp();
        let normalized = normalize_target(value.to_str().unwrap(), cwd.path()).unwrap();
        // The two platforms genuinely disagree here, and the guard follows each
        // rather than imposing one. Windows collapses `..` lexically before the
        // filesystem is consulted at all, so `link/..` is the link's textual
        // parent; POSIX resolves the link first and lands in the target's real
        // parent. Either way the answer is the directory that platform's own
        // verbs would act in for the same string, which is the property the
        // guard owes its caller — so this is asserted per platform instead of
        // being flattened into one expectation that would be wrong somewhere.
        #[cfg(unix)]
        let expected = resolve_missing_ok(&real_parent).unwrap().join("sibling");
        #[cfg(windows)]
        let expected = resolve_missing_ok(root.path()).unwrap().join("sibling");
        assert_eq!(normalized, expected);

        // Same story as the lexical case: the resolution is still correct and
        // still needed for `--under`, but VALUE is refused before reaching it.
        let refused = evaluate_typed(
            GuardType::Path,
            value.to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        );
        assert!(
            matches!(refused, Err(GuardError::TraversalSegment)),
            "{refused:?}"
        );
    }

    #[test]
    fn final_symlink_segment_is_not_resolved_to_its_target() {
        let root = temp();
        let target_dir = root.path().join("target_dir");
        std::fs::create_dir(&target_dir).unwrap();
        let link = root.path().join("link_name");
        symlink_dir(&target_dir, &link);

        let cwd = temp();
        let result = evaluate_typed(
            GuardType::Path,
            link.to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        )
        .unwrap();
        let canonical_root = resolve_missing_ok(root.path()).unwrap();
        assert_eq!(result, canonical_root.join("link_name"));
        assert_ne!(result, resolve_missing_ok(&target_dir).unwrap());
    }

    /// A case-insensitive filesystem answers to any spelling of a protected
    /// directory; the guard has to recognise every one of them.
    #[test]
    fn a_differently_cased_spelling_of_a_protected_directory_is_rejected() {
        let cwd = temp();
        let home = temp();
        let home_path = resolve_missing_ok(&home.path().join("Home")).unwrap();
        std::fs::create_dir(&home_path).unwrap();
        let upper = home_path.with_file_name("HOME");
        if std::fs::symlink_metadata(&upper).is_err() {
            // Case-sensitive filesystem: `HOME` is a different, missing entry.
            return assert!(
                evaluate_typed(
                    GuardType::Path,
                    upper.to_str().unwrap(),
                    cwd.path(),
                    Some(&home_path),
                    &[]
                )
                .is_ok()
            );
        }
        let result = evaluate_typed(
            GuardType::Path,
            upper.to_str().unwrap(),
            cwd.path(),
            Some(&home_path),
            &[],
        );
        assert!(
            matches!(result, Err(GuardError::Rejected { ref normalized }) if normalized == &home_path),
            "{result:?}"
        );
    }

    #[test]
    fn a_differently_cased_symlink_is_spelled_as_the_link_not_its_target() {
        let root = temp();
        let target_dir = root.path().join("target_dir");
        std::fs::create_dir(&target_dir).unwrap();
        symlink_dir(&target_dir, &root.path().join("link_name"));
        let upper = root.path().join("LINK_NAME");
        if std::fs::symlink_metadata(&upper).is_err() {
            return;
        }
        let cwd = temp();
        let result = evaluate_typed(
            GuardType::Path,
            upper.to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        )
        .unwrap();
        assert_eq!(
            result,
            resolve_missing_ok(root.path()).unwrap().join("link_name")
        );
    }

    #[test]
    fn trailing_dot_behaves_exactly_like_the_bare_form() {
        let root = temp();
        let target_dir = root.path().join("target_dir");
        std::fs::create_dir(&target_dir).unwrap();
        let link = root.path().join("link_name");
        symlink_dir(&target_dir, &link);

        let cwd = temp();
        let bare = evaluate_typed(
            GuardType::Path,
            link.to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        )
        .unwrap();
        let with_dot = evaluate_typed(
            GuardType::Path,
            link.join(".").to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        )
        .unwrap();
        assert_eq!(bare, with_dot);
    }

    #[test]
    fn containment_root_symlink_is_resolved_before_comparison() {
        // Mirrors macOS's real `/var` -> `/private/var`: the root candidate
        // itself is a symlink, and a target reached through it must still be
        // recognized as contained once both sides are canonicalized.
        let root = temp();
        let real_root = root.path().join("real_root");
        std::fs::create_dir(&real_root).unwrap();
        let root_link = root.path().join("root_link");
        symlink_dir(&real_root, &root_link);

        let child = root_link.join("work");
        std::fs::create_dir(root.path().join("real_root").join("work")).unwrap();
        let cwd = temp();
        let result = evaluate_typed(
            GuardType::TmpPath,
            child.to_str().unwrap(),
            cwd.path(),
            None,
            std::slice::from_ref(&root_link),
        );
        let expected = resolve_missing_ok(&real_root).unwrap().join("work");
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn missing_temp_root_candidate_is_skipped_not_an_error() {
        let cwd = temp();
        let tmp_root = temp();
        let missing_root = tmp_root.path().join("does-not-exist-root");
        let child = tmp_root.path().join("work");
        std::fs::create_dir(&child).unwrap();
        let result = evaluate_typed(
            GuardType::TmpPath,
            child.to_str().unwrap(),
            cwd.path(),
            None,
            &[missing_root, tmp_root.path().to_path_buf()],
        );
        assert_eq!(result.unwrap(), resolve_missing_ok(&child).unwrap());
    }

    #[test]
    fn relative_value_resolves_against_the_supplied_cwd() {
        let cwd = temp();
        let child = cwd.path().join("child");
        std::fs::create_dir(&child).unwrap();
        let result = evaluate_typed(GuardType::CwdPath, "child", cwd.path(), None, &[]).unwrap();
        assert_eq!(result, resolve_missing_ok(&child).unwrap());
    }

    #[test]
    fn all_whitespace_value_is_rejected_like_an_empty_one() {
        let cwd = temp();
        for blank in ["   ", "\t", " \t "] {
            let result = evaluate_typed(GuardType::Path, blank, cwd.path(), None, &[]);
            assert!(matches!(result, Err(GuardError::EmptyValue)), "{result:?}");
        }
    }

    #[test]
    fn final_symlink_pointing_outside_the_containment_root_is_rejected() {
        let cwd = temp();
        let tmp_root = temp();
        let elsewhere = temp();
        let escape = elsewhere.path().join("real-target");
        std::fs::create_dir(&escape).unwrap();
        let link = tmp_root.path().join("escape_link");
        symlink_dir(&escape, &link);

        let result = evaluate_typed(
            GuardType::TmpPath,
            link.to_str().unwrap(),
            cwd.path(),
            None,
            &[tmp_root.path().to_path_buf()],
        );
        assert!(
            matches!(result, Err(GuardError::SymlinkEscapesContainment { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn final_symlink_staying_inside_the_containment_root_still_passes_unresolved() {
        let cwd = temp();
        let tmp_root = temp();
        let sibling = tmp_root.path().join("sibling");
        std::fs::create_dir(&sibling).unwrap();
        let link = tmp_root.path().join("inside_link");
        symlink_dir(&sibling, &link);

        let result = evaluate_typed(
            GuardType::TmpPath,
            link.to_str().unwrap(),
            cwd.path(),
            None,
            &[tmp_root.path().to_path_buf()],
        )
        .unwrap();
        // Contained, so it passes — and still as the link itself, so `rm`
        // removes the link rather than `sibling`.
        assert_eq!(
            result,
            resolve_missing_ok(tmp_root.path())
                .unwrap()
                .join("inside_link")
        );
    }

    #[test]
    fn a_dangling_final_symlink_is_judged_by_where_it_would_create() {
        let cwd = temp();
        let tmp_root = temp();
        let elsewhere = temp();
        let link = tmp_root.path().join("dangling");
        // Nothing exists at the target: `rm` would still only unlink, but `>`
        // would create the file out there, so containment must be decided on
        // the target's location, not on whether it exists yet.
        symlink_dir(&elsewhere.path().join("not-created-yet"), &link);

        let result = evaluate_typed(
            GuardType::TmpPath,
            link.to_str().unwrap(),
            cwd.path(),
            None,
            &[tmp_root.path().to_path_buf()],
        );
        assert!(
            matches!(result, Err(GuardError::SymlinkEscapesContainment { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn plain_path_type_has_no_root_so_a_symlink_is_never_an_escape() {
        let cwd = temp();
        let root = temp();
        let elsewhere = temp();
        let link = root.path().join("escape_link");
        symlink_dir(elsewhere.path(), &link);

        let result = evaluate_typed(
            GuardType::Path,
            link.to_str().unwrap(),
            cwd.path(),
            None,
            &[],
        );
        assert_eq!(
            result.unwrap(),
            resolve_missing_ok(root.path()).unwrap().join("escape_link")
        );
    }

    #[test]
    fn under_root_anchors_a_target_the_type_vocabulary_cannot_name() {
        let cwd = temp();
        let anchor = temp();
        let child = anchor.path().join("build");
        std::fs::create_dir(&child).unwrap();

        let inside = evaluate(
            GuardType::Path,
            child.to_str().unwrap(),
            Some(anchor.path().to_str().unwrap()),
            cwd.path(),
            None,
            &[],
        );
        assert_eq!(inside.unwrap(), resolve_missing_ok(&child).unwrap());

        let elsewhere = temp();
        let outside = evaluate(
            GuardType::Path,
            elsewhere.path().to_str().unwrap(),
            Some(anchor.path().to_str().unwrap()),
            cwd.path(),
            None,
            &[],
        );
        assert!(
            matches!(
                outside,
                Err(GuardError::OutsideContainment {
                    containment: Containment::Under,
                    ..
                })
            ),
            "{outside:?}"
        );
    }

    #[test]
    fn under_root_itself_is_rejected_like_every_other_containment_root() {
        let cwd = temp();
        let anchor = temp();
        let result = evaluate(
            GuardType::Path,
            anchor.path().to_str().unwrap(),
            Some(anchor.path().to_str().unwrap()),
            cwd.path(),
            None,
            &[],
        );
        assert!(
            matches!(result, Err(GuardError::OutsideContainment { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn a_blank_under_root_is_rejected_rather_than_meaning_the_current_directory() {
        let cwd = temp();
        let child = cwd.path().join("child");
        std::fs::create_dir(&child).unwrap();
        for blank in ["", "   ", "\n"] {
            let result = evaluate(
                GuardType::Path,
                child.to_str().unwrap(),
                Some(blank),
                cwd.path(),
                None,
                &[],
            );
            assert!(
                matches!(result, Err(GuardError::InvalidRoot)),
                "{blank:?}: {result:?}"
            );
        }
    }

    #[test]
    fn type_and_under_must_both_be_satisfied() {
        let cwd = temp();
        let tmp_root = temp();
        let inside_tmp = tmp_root.path().join("work");
        std::fs::create_dir(&inside_tmp).unwrap();
        let unrelated_anchor = temp();

        // Contained by `tmp_path`, but not by the caller's anchor.
        let result = evaluate(
            GuardType::TmpPath,
            inside_tmp.to_str().unwrap(),
            Some(unrelated_anchor.path().to_str().unwrap()),
            cwd.path(),
            None,
            &[tmp_root.path().to_path_buf()],
        );
        assert!(
            matches!(
                result,
                Err(GuardError::OutsideContainment {
                    containment: Containment::Under,
                    ..
                })
            ),
            "{result:?}"
        );

        // Contained by the anchor, but not by `tmp_path`.
        let outside_tmp = unrelated_anchor.path().join("work");
        std::fs::create_dir(&outside_tmp).unwrap();
        let result = evaluate(
            GuardType::TmpPath,
            outside_tmp.to_str().unwrap(),
            Some(unrelated_anchor.path().to_str().unwrap()),
            cwd.path(),
            None,
            &[tmp_root.path().to_path_buf()],
        );
        assert!(
            matches!(
                result,
                Err(GuardError::OutsideContainment {
                    containment: Containment::Type(GuardType::TmpPath),
                    ..
                })
            ),
            "{result:?}"
        );
    }

    #[test]
    fn under_root_is_resolved_through_its_own_symlink() {
        let cwd = temp();
        let root = temp();
        let real_anchor = root.path().join("real_anchor");
        std::fs::create_dir(&real_anchor).unwrap();
        let anchor_link = root.path().join("anchor_link");
        symlink_dir(&real_anchor, &anchor_link);
        let child = real_anchor.join("build");
        std::fs::create_dir(&child).unwrap();

        let result = evaluate(
            GuardType::Path,
            child.to_str().unwrap(),
            Some(anchor_link.to_str().unwrap()),
            cwd.path(),
            None,
            &[],
        );
        assert_eq!(result.unwrap(), resolve_missing_ok(&child).unwrap());
    }

    #[test]
    fn guard_type_parse_round_trips_the_closed_vocabulary() {
        assert_eq!(GuardType::parse("path"), Some(GuardType::Path));
        assert_eq!(GuardType::parse("tmp_path"), Some(GuardType::TmpPath));
        assert_eq!(GuardType::parse("cwd_path"), Some(GuardType::CwdPath));
        assert_eq!(GuardType::parse("bogus"), None);
    }
}
