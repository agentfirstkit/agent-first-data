//! Mutation guards and atomic replacement or creation of document files.
use super::{CreateMode, CreateOptions};
use crate::document::{DocumentError, DocumentResult};
use std::fs;
use std::path::Path;

/// Reject mutation of a symlink or (on unix) a hardlinked file. Returns the
/// target's metadata on success so callers that also need to write can reuse
/// it (e.g. to preserve permissions) without a second syscall.
pub(super) fn guard_mutation(path: &Path, operation: &str) -> DocumentResult<fs::Metadata> {
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

/// Turn a failed atomic installation into this module's error type. The step
/// names the failure; `operation` names what the caller was doing when it
/// happened, so a `save` and a `create` read differently for the same fault.
fn document_io_error(operation: &str, error: crate::atomic_file::AtomicError) -> DocumentError {
    DocumentError::IoError {
        detail: format!("{operation} {error}"),
    }
}

/// Write `bytes` to `path` atomically: guard, same-directory temp file,
/// fsync, permission preservation, then rename over the target.
pub(super) fn write_atomic(path: &Path, bytes: &[u8], operation: &str) -> DocumentResult<()> {
    let metadata = guard_mutation(path, operation)?;
    crate::atomic_file::install(
        path,
        crate::atomic_file::AtomicInstall::replacing(bytes)
            .with_permissions(Some(metadata.permissions())),
    )
    .map_err(|error| document_io_error(operation, error))
}

pub(super) fn write_atomic_create(
    path: &Path,
    bytes: &[u8],
    options: CreateOptions,
) -> DocumentResult<()> {
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

    // A replace with no explicit mode keeps the target's own permissions,
    // exactly as `save()` does; only a first creation falls back to 0o600.
    let unix_mode = options.effective_unix_mode(existing_permissions.is_some());
    let preserved = unix_mode
        .is_none()
        .then(|| existing_permissions.clone())
        .flatten();
    let mut request = crate::atomic_file::AtomicInstall::replacing(bytes)
        .with_permissions(preserved)
        .with_unix_mode(unix_mode);
    if options.mode == CreateMode::NewOnly {
        request = request.new_only();
    }
    crate::atomic_file::install(path, request).map_err(|error| {
        // `NewOnly` installs with a link, so losing the race for a name that
        // was absent at preflight is the same answer as finding it there.
        if error.target_exists() {
            return DocumentError::AlreadyExists {
                path: path.display().to_string(),
            };
        }
        document_io_error(operation, error)
    })
}
