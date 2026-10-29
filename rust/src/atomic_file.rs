//! One atomic file installation, shared by every writer in this crate.
//!
//! Installing a file so that a crash can never expose a half-written one is a
//! fixed sequence — private same-directory temporary file, write, permissions,
//! fsync the file, atomically install it over the target, fsync the directory
//! entry — and each step has a way to be got subtly wrong. Writing that
//! sequence a second time somewhere else is how one copy ends up durable and
//! the other only looks it: a `rename` that is never followed by a parent
//! directory fsync returns success while the new directory entry may still be
//! lost to a power cut.
//!
//! So the sequence lives here once, and callers bring only what is genuinely
//! theirs: the pre-write guard (what may be overwritten is a policy question,
//! and different writers answer it differently) and the error type. Failures
//! name the step that failed through [`AtomicError`], whose `Display` is the
//! tail of a caller's own message — `write` + `atomic replace \`p\`: …`.

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// How the finished temporary file becomes the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallMode {
    /// `rename` over the target. Replaces whatever is at the path — including a
    /// symbolic link, which is replaced rather than followed.
    Replace,
    /// `hard_link`, which refuses to install over an existing target. The
    /// caller distinguishes that refusal by [`AtomicError::target_exists`].
    NewOnly,
}

/// What to install, and how.
pub(crate) struct AtomicInstall<'a> {
    /// The complete new contents.
    pub bytes: &'a [u8],
    /// How to install the finished file.
    pub mode: InstallMode,
    /// Permissions to re-apply, normally the replaced file's own.
    pub permissions: Option<fs::Permissions>,
    /// Explicit unix permission bits, applied after `permissions`.
    pub unix_mode: Option<u32>,
}

impl<'a> AtomicInstall<'a> {
    /// Replace whatever is at the path, leaving permissions to the platform.
    pub(crate) fn replacing(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            mode: InstallMode::Replace,
            permissions: None,
            unix_mode: None,
        }
    }

    /// Re-apply these permissions to the new file.
    pub(crate) fn with_permissions(mut self, permissions: Option<fs::Permissions>) -> Self {
        self.permissions = permissions;
        self
    }

    /// Apply these unix permission bits to the new file.
    pub(crate) fn with_unix_mode(mut self, unix_mode: Option<u32>) -> Self {
        self.unix_mode = unix_mode;
        self
    }

    /// Install with `hard_link`, refusing an existing target.
    pub(crate) fn new_only(mut self) -> Self {
        self.mode = InstallMode::NewOnly;
        self
    }
}

/// The step of an atomic installation that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomicStep {
    /// The target has no parent directory to write the temporary file into.
    NoParent,
    /// The target's own file name is not valid UTF-8.
    NonUtf8Name,
    /// Creating the private temporary file.
    Temp,
    /// Every candidate temporary name was taken.
    TempExhausted,
    /// Writing the new bytes.
    Write,
    /// Re-applying the replaced file's permissions.
    PreservePermissions,
    /// Applying explicitly requested permission bits. Unix only, because the
    /// bits themselves are: the step does not exist on a platform that has no
    /// mode to set, and a variant no code can construct is a step a reader
    /// would look for and never find.
    #[cfg(unix)]
    SetPermissions,
    /// Flushing the temporary file to storage.
    Sync,
    /// Installing the temporary file over the target.
    Install,
    /// Recording the new directory entry.
    SyncParent,
}

/// A failed atomic installation: which step, which path, and the OS error.
#[derive(Debug)]
pub(crate) struct AtomicError {
    /// The step that failed.
    pub step: AtomicStep,
    /// The path that step was about — the target, its temporary file, or the
    /// parent directory, depending on the step.
    pub path: PathBuf,
    /// The underlying OS error, absent only where the step had none.
    pub source: Option<std::io::Error>,
}

impl AtomicError {
    fn at(step: AtomicStep, path: &Path, source: std::io::Error) -> Self {
        Self {
            step,
            path: path.to_path_buf(),
            source: Some(source),
        }
    }

    /// Whether this is [`InstallMode::NewOnly`] refusing an existing target,
    /// which is a caller-visible outcome rather than an I/O fault.
    pub(crate) fn target_exists(&self) -> bool {
        self.step == AtomicStep::Install
            && self
                .source
                .as_ref()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists)
    }

    /// Whether the target may already carry the new bytes.
    ///
    /// Only the final directory fsync can fail after the installation itself
    /// succeeded: the file is complete and in place, but its survival across a
    /// crash is unconfirmed. Every other step fails before the install and
    /// leaves the original untouched.
    pub(crate) fn commit_uncertain(&self) -> bool {
        self.step == AtomicStep::SyncParent
    }
}

impl std::fmt::Display for AtomicError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let path = self.path.display();
        let phrase = match self.step {
            AtomicStep::NoParent => {
                return write!(formatter, "has no parent directory for `{path}`");
            }
            AtomicStep::NonUtf8Name => {
                return write!(formatter, "path is not valid UTF-8: `{path}`");
            }
            AtomicStep::TempExhausted => {
                return write!(formatter, "could not allocate temporary file in `{path}`");
            }
            AtomicStep::Temp => "temporary file in",
            AtomicStep::Write => "write",
            AtomicStep::PreservePermissions => "preserve permissions",
            #[cfg(unix)]
            AtomicStep::SetPermissions => "set permissions on",
            AtomicStep::Sync => "fsync",
            AtomicStep::Install => "install",
            AtomicStep::SyncParent => "fsync parent directory",
        };
        write!(formatter, "{phrase} `{path}`")?;
        match &self.source {
            Some(source) => write!(formatter, ": {source}"),
            None => Ok(()),
        }
    }
}

/// The smallest `NAME_MAX` across the platforms this runs on.
const MAX_NAME_BYTES: usize = 255;

/// Compose a temporary file name that stays within one path component's limit.
///
/// The marker, pid, and attempt add about 35 bytes. A target whose own name is
/// close to `NAME_MAX` would push the composed name past it, and the failure
/// would surface at install time — after the caller had already done its work,
/// on a file it opened successfully. Truncating the stem keeps the write
/// possible; uniqueness still comes from the pid and attempt, with `create_new`
/// retrying the rare collision.
fn temp_file_name(file_name: &str, pid: u32, attempt: u32) -> String {
    let suffix = format!(".afdata.{pid}.{attempt}.tmp");
    // One byte for the leading dot that hides the temporary file.
    let budget = MAX_NAME_BYTES.saturating_sub(suffix.len() + 1);
    let stem = &file_name[..file_name.floor_char_boundary(budget)];
    format!(".{stem}{suffix}")
}

/// Split a target into the directory the temporary file must share with it and
/// the target's own name. Sharing the directory is what makes the final install
/// a rename within one filesystem, which is the atomic operation this relies on.
fn parent_and_name(path: &Path) -> Result<(&Path, String), AtomicError> {
    let parent = match path.parent() {
        Some(parent) if parent.as_os_str().is_empty() => Path::new("."),
        Some(parent) => parent,
        None => {
            return Err(AtomicError {
                step: AtomicStep::NoParent,
                path: path.to_path_buf(),
                source: None,
            });
        }
    };
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AtomicError {
            step: AtomicStep::NonUtf8Name,
            path: path.to_path_buf(),
            source: None,
        })?
        .to_string();
    Ok((parent, file_name))
}

fn allocate_private_temp(parent: &Path, file_name: &str) -> Result<(PathBuf, File), AtomicError> {
    let pid = std::process::id();
    for attempt in 0..32_u32 {
        let candidate = parent.join(temp_file_name(file_name, pid, attempt));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            // Nothing may read the new contents through the temporary name
            // before the caller's own permissions are applied below.
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(AtomicError::at(AtomicStep::Temp, parent, error)),
        }
    }
    Err(AtomicError {
        step: AtomicStep::TempExhausted,
        path: parent.to_path_buf(),
        source: None,
    })
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<(), AtomicError> {
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| AtomicError::at(AtomicStep::SyncParent, parent, error))
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<(), AtomicError> {
    // Rust does not expose a portable directory handle on non-unix targets.
    // The file itself is still synced before its atomic installation.
    Ok(())
}

fn write_temp(
    mut temp_file: File,
    temp_path: &Path,
    target: &Path,
    request: &AtomicInstall<'_>,
) -> Result<(), AtomicError> {
    temp_file
        .write_all(request.bytes)
        .map_err(|error| AtomicError::at(AtomicStep::Write, target, error))?;
    if let Some(permissions) = request.permissions.clone() {
        temp_file
            .set_permissions(permissions)
            .map_err(|error| AtomicError::at(AtomicStep::PreservePermissions, target, error))?;
    }
    #[cfg(unix)]
    if let Some(unix_mode) = request.unix_mode {
        use std::os::unix::fs::PermissionsExt as _;
        temp_file
            .set_permissions(fs::Permissions::from_mode(unix_mode))
            .map_err(|error| AtomicError::at(AtomicStep::SetPermissions, target, error))?;
    }
    temp_file
        .sync_all()
        .map_err(|error| AtomicError::at(AtomicStep::Sync, temp_path, error))
}

/// Install `request` at `path` atomically.
///
/// The caller has already decided that this target may be written; this only
/// performs the write. Every failure before the install leaves the original
/// untouched and removes the temporary file. A failure at the final parent
/// directory fsync — the one [`AtomicError::commit_uncertain`] reports — means
/// the new file may already be in place without confirmed durability.
pub(crate) fn install(path: &Path, request: AtomicInstall<'_>) -> Result<(), AtomicError> {
    let (parent, file_name) = parent_and_name(path)?;
    let (temp_path, temp_file) = allocate_private_temp(parent, &file_name)?;
    let result = (|| -> Result<(), AtomicError> {
        write_temp(temp_file, &temp_path, path, &request)?;
        match request.mode {
            InstallMode::Replace => {
                fs::rename(&temp_path, path)
                    .map_err(|error| AtomicError::at(AtomicStep::Install, path, error))?;
            }
            InstallMode::NewOnly => {
                fs::hard_link(&temp_path, path)
                    .map_err(|error| AtomicError::at(AtomicStep::Install, path, error))?;
                // The file is installed from here on. Failing to unlink the
                // temporary link leaves a stray file, but reporting an error
                // would tell the caller the commit did not happen — and a retry
                // would then refuse a target that this call in fact created.
                let _ = fs::remove_file(&temp_path);
            }
        }
        sync_parent(parent)?;
        Ok(())
    })();
    if let Err(error) = &result {
        // A parent-directory fsync failure comes after the install, where the
        // temporary name no longer refers to anything to clean up.
        if !error.commit_uncertain() {
            let _ = fs::remove_file(&temp_path);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]
    use super::*;

    #[test]
    fn temp_name_stays_within_one_path_component() {
        let long = "x".repeat(400);
        let name = temp_file_name(&long, 1234, 0);
        assert!(name.len() <= MAX_NAME_BYTES, "{} bytes", name.len());
        assert!(name.starts_with('.'));
        assert!(name.ends_with(".tmp"));
    }

    #[test]
    fn temp_name_truncates_on_a_character_boundary() {
        let long = "é".repeat(300);
        let name = temp_file_name(&long, 1234, 0);
        assert!(name.is_char_boundary(name.len()));
        assert!(name.len() <= MAX_NAME_BYTES);
    }

    #[test]
    fn bare_relative_paths_use_the_current_directory() {
        let (parent, file_name) = parent_and_name(Path::new("config.json")).unwrap();

        assert_eq!(parent, Path::new("."));
        assert_eq!(file_name, "config.json");
    }

    #[test]
    fn replace_installs_new_bytes_and_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"old").unwrap();

        install(&path, AtomicInstall::replacing(b"new")).unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"new");
        let strays: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(strays.is_empty(), "temporary files left behind");
    }

    #[test]
    fn new_only_refuses_an_existing_target_without_touching_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, b"original").unwrap();

        let error = install(&path, AtomicInstall::replacing(b"new").new_only()).unwrap_err();

        assert!(error.target_exists());
        assert_eq!(fs::read(&path).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[test]
    fn replace_swaps_a_symlink_rather_than_following_it() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.txt");
        fs::write(&outside, b"untouched").unwrap();
        let link = dir.path().join("link.txt");
        symlink(&outside, &link).unwrap();

        install(&link, AtomicInstall::replacing(b"new")).unwrap();

        assert_eq!(fs::read(&outside).unwrap(), b"untouched");
        assert!(
            !fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&link).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn explicit_unix_mode_wins_over_the_private_temporary_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readable.md");

        install(
            &path,
            AtomicInstall::replacing(b"body").with_unix_mode(Some(0o644)),
        )
        .unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }

    #[test]
    fn a_missing_parent_directory_fails_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent").join("config.json");

        let error = install(&path, AtomicInstall::replacing(b"new")).unwrap_err();

        assert_eq!(error.step, AtomicStep::Temp);
        assert!(!error.commit_uncertain());
        assert!(!path.exists());
    }
}
