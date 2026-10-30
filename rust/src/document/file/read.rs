//! Handle-based bounded UTF-8 reads and atomic no-follow policy.
use super::SymlinkPolicy;
use crate::document::{DocumentError, DocumentResult};
use std::fs::{File, OpenOptions};
use std::io::Read as _;
use std::path::Path;

pub(super) fn open_read_handle(path: &Path, symlink_policy: SymlinkPolicy) -> DocumentResult<File> {
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
    {
        if symlink_policy == SymlinkPolicy::NoFollow {
            return Err(DocumentError::UnsupportedOperation {
                format: "filesystem".to_string(),
                operation: "open".to_string(),
                detail: "atomic no-follow reads require Cargo feature `libc` on unix".to_string(),
            });
        }
        // Without `O_NONBLOCK`, opening a FIFO blocks until a writer appears,
        // before handle metadata can reject it. A path check first is racy
        // but keeps the ordinary case from hanging; the handle is still
        // checked after the open.
        if let Ok(metadata) = std::fs::metadata(path)
            && !metadata.is_file()
        {
            return Err(DocumentError::IoError {
                detail: format!("`{}` is not a regular file", path.display()),
            });
        }
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

pub(super) fn read_capped_source(
    file: File,
    path: &Path,
    max_bytes: u64,
) -> DocumentResult<String> {
    inspect_capped_source(&file, path, max_bytes)?;
    read_capped_contents(file, path, max_bytes)
}

pub(super) fn inspect_capped_source(
    file: &File,
    path: &Path,
    max_bytes: u64,
) -> DocumentResult<()> {
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

pub(super) fn read_capped_contents(
    file: File,
    path: &Path,
    max_bytes: u64,
) -> DocumentResult<String> {
    let read_limit = max_bytes.saturating_add(1);
    // A budget is a ceiling, not an allocation hint: small configuration files
    // must not allocate a megabyte merely because that much would be allowed.
    let mut bytes = Vec::new();
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
