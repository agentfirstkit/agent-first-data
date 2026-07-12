//! Optional process stdout/stderr file redirection.
//!
//! This is a CLI/deployment helper, not an AFDATA protocol formatter. It
//! redirects stdout and/or stderr to caller-provided files without converting
//! diagnostics such as Rust panic output into JSON.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Canonical CLI argument for redirecting stdout.
pub const STDOUT_FILE_ARG: &str = "--stdout-file";

/// Canonical CLI argument for redirecting stderr.
pub const STDERR_FILE_ARG: &str = "--stderr-file";

/// Resolved process stream redirection configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamRedirectConfig {
    /// File receiving stdout bytes, if stdout redirection is enabled.
    pub stdout_file: Option<PathBuf>,
    /// File receiving stderr bytes, if stderr redirection is enabled.
    pub stderr_file: Option<PathBuf>,
}

impl StreamRedirectConfig {
    /// Build a config from explicit stdout/stderr file paths.
    pub fn new(
        stdout_file: Option<impl Into<PathBuf>>,
        stderr_file: Option<impl Into<PathBuf>>,
    ) -> io::Result<Option<Self>> {
        let stdout_file = stdout_file.map(Into::into);
        let stderr_file = stderr_file.map(Into::into);
        validate_optional_file(STDOUT_FILE_ARG, stdout_file.as_deref())?;
        validate_optional_file(STDERR_FILE_ARG, stderr_file.as_deref())?;
        if stdout_file.is_none() && stderr_file.is_none() {
            return Ok(None);
        }
        Ok(Some(Self {
            stdout_file,
            stderr_file,
        }))
    }
}

/// Guard for installed stream redirection.
///
/// Keep this value alive for as long as stdout/stderr should stay redirected.
/// On drop it restores the original process fds.
#[cfg_attr(not(unix), derive(Clone, Debug, PartialEq, Eq))]
pub struct InstalledStreamRedirect {
    /// File receiving stdout bytes, if stdout redirection is enabled.
    pub stdout_file: Option<PathBuf>,
    /// File receiving stderr bytes, if stderr redirection is enabled.
    pub stderr_file: Option<PathBuf>,
    #[cfg(unix)]
    stdout_restore: Option<std::os::fd::OwnedFd>,
    #[cfg(unix)]
    stderr_restore: Option<std::os::fd::OwnedFd>,
}

#[cfg(unix)]
impl std::fmt::Debug for InstalledStreamRedirect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstalledStreamRedirect")
            .field("stdout_file", &self.stdout_file)
            .field("stderr_file", &self.stderr_file)
            .finish_non_exhaustive()
    }
}

#[cfg(unix)]
impl Drop for InstalledStreamRedirect {
    fn drop(&mut self) {
        let _ = io::stdout().flush();

        if let Some(stdout_restore) = &self.stdout_restore {
            let _ = unix::redirect_fd(libc::STDOUT_FILENO, stdout_restore.as_raw_fd());
        }
        if let Some(stderr_restore) = &self.stderr_restore {
            let _ = unix::redirect_fd(libc::STDERR_FILENO, stderr_restore.as_raw_fd());
        }

        self.stdout_restore.take();
        self.stderr_restore.take();
        unix::mark_uninstalled();
    }
}

#[cfg(unix)]
use std::os::fd::AsRawFd;

/// Resolve canonical CLI arguments into a config.
pub fn config_from_cli_args(
    stdout_file_arg: Option<PathBuf>,
    stderr_file_arg: Option<PathBuf>,
) -> io::Result<Option<StreamRedirectConfig>> {
    StreamRedirectConfig::new(stdout_file_arg, stderr_file_arg)
}

/// Resolve canonical raw CLI arguments into a config.
///
/// This parser intentionally has no `clap` dependency so callers can install
/// redirection before help/version handling emits early output. It recognizes
/// `--stdout-file VALUE`, `--stdout-file=VALUE`, `--stderr-file VALUE`, and
/// `--stderr-file=VALUE`.
pub fn config_from_raw_args<I, S>(args: I) -> io::Result<Option<StreamRedirectConfig>>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let raw = parse_raw_args(args)?;
    config_from_cli_args(raw.stdout_file, raw.stderr_file)
}

/// Install stdout/stderr redirection from canonical CLI inputs.
pub fn install_from_cli_args(
    stdout_file_arg: Option<PathBuf>,
    stderr_file_arg: Option<PathBuf>,
) -> io::Result<Option<InstalledStreamRedirect>> {
    match config_from_cli_args(stdout_file_arg, stderr_file_arg)? {
        Some(config) => install(&config).map(Some),
        None => Ok(None),
    }
}

/// Install stdout/stderr redirection from canonical raw CLI inputs.
pub fn install_from_raw_args<I, S>(args: I) -> io::Result<Option<InstalledStreamRedirect>>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    match config_from_raw_args(args)? {
        Some(config) => install(&config).map(Some),
        None => Ok(None),
    }
}

/// Install stdout/stderr redirection for a resolved config.
#[cfg(unix)]
pub fn install(config: &StreamRedirectConfig) -> io::Result<InstalledStreamRedirect> {
    unix::install(config)
}

/// Install stdout/stderr redirection for a resolved config.
#[cfg(not(unix))]
pub fn install(config: &StreamRedirectConfig) -> io::Result<InstalledStreamRedirect> {
    let _ = config;
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stream redirection is only supported on Unix platforms",
    ))
}

fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn validate_optional_file(arg_name: &str, path: Option<&Path>) -> io::Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if path.as_os_str() == OsStr::new("") {
        return Err(invalid_input(&format!("{arg_name} must not be empty")));
    }
    Ok(())
}

fn open_append(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                let file_type = metadata.file_type();
                if file_type.is_symlink() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "stream redirection target must not be a symbolic link",
                    ));
                }
                if !file_type.is_file() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "stream redirection target must be a regular file",
                    ));
                }
                OpenOptions::new()
                    .append(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(path)
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => OpenOptions::new()
                .append(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path),
            Err(err) => Err(err),
        }
    }
    #[cfg(not(unix))]
    {
        OpenOptions::new().create(true).append(true).open(path)
    }
}

#[derive(Debug, Default)]
struct RawStreamRedirectArgs {
    stdout_file: Option<PathBuf>,
    stderr_file: Option<PathBuf>,
}

fn parse_raw_args<I, S>(args: I) -> io::Result<RawStreamRedirectArgs>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut parsed = RawStreamRedirectArgs::default();
    let mut iter = args.into_iter().map(Into::into).peekable();

    while let Some(arg) = iter.next() {
        if arg == OsStr::new("--") {
            break;
        }
        let Some(arg_str) = arg.to_str() else {
            continue;
        };

        if arg_str == STDOUT_FILE_ARG {
            parsed.stdout_file = Some(PathBuf::from(take_raw_value(&mut iter, STDOUT_FILE_ARG)?));
        } else if let Some(value) = arg_str.strip_prefix("--stdout-file=") {
            parsed.stdout_file = Some(PathBuf::from(value));
        } else if arg_str == STDERR_FILE_ARG {
            parsed.stderr_file = Some(PathBuf::from(take_raw_value(&mut iter, STDERR_FILE_ARG)?));
        } else if let Some(value) = arg_str.strip_prefix("--stderr-file=") {
            parsed.stderr_file = Some(PathBuf::from(value));
        }
    }

    Ok(parsed)
}

fn take_raw_value<I>(iter: &mut std::iter::Peekable<I>, arg_name: &str) -> io::Result<OsString>
where
    I: Iterator<Item = OsString>,
{
    let Some(value) = iter.next() else {
        return Err(invalid_input(&format!("{arg_name} requires a value")));
    };
    if value
        .to_str()
        .is_some_and(|value| value.starts_with("--") && value != "--")
    {
        return Err(invalid_input(&format!("{arg_name} requires a value")));
    }
    Ok(value)
}

#[cfg(unix)]
mod unix {
    use super::{InstalledStreamRedirect, PathBuf, StreamRedirectConfig, Write, io, open_append};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::sync::atomic::{AtomicBool, Ordering};

    const STDOUT_FD: RawFd = libc::STDOUT_FILENO;
    const STDERR_FD: RawFd = libc::STDERR_FILENO;

    static INSTALLED: AtomicBool = AtomicBool::new(false);

    pub(super) fn install(config: &StreamRedirectConfig) -> io::Result<InstalledStreamRedirect> {
        INSTALLED
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "stream redirection already installed",
                )
            })?;

        match install_once(config) {
            Ok(installed) => Ok(installed),
            Err(err) => {
                INSTALLED.store(false, Ordering::SeqCst);
                Err(err)
            }
        }
    }

    fn install_once(config: &StreamRedirectConfig) -> io::Result<InstalledStreamRedirect> {
        let mut stdout_target = prepare_target(STDOUT_FD, config.stdout_file.as_ref())?;
        let mut stderr_target = prepare_target(STDERR_FD, config.stderr_file.as_ref())?;

        let _ = io::stdout().flush();

        if let Some(target) = &stdout_target {
            redirect_fd(STDOUT_FD, target.file.as_raw_fd())?;
        }
        if let Some(target) = &stderr_target
            && let Err(err) = redirect_fd(STDERR_FD, target.file.as_raw_fd())
        {
            if let Some(stdout_target) = &stdout_target {
                let _ = redirect_fd(STDOUT_FD, stdout_target.restore.as_raw_fd());
            }
            return Err(err);
        }

        Ok(InstalledStreamRedirect {
            stdout_file: config.stdout_file.clone(),
            stderr_file: config.stderr_file.clone(),
            stdout_restore: stdout_target.take().map(|target| target.restore),
            stderr_restore: stderr_target.take().map(|target| target.restore),
        })
    }

    struct PreparedTarget {
        file: std::fs::File,
        restore: OwnedFd,
    }

    fn prepare_target(
        target_fd: RawFd,
        path: Option<&PathBuf>,
    ) -> io::Result<Option<PreparedTarget>> {
        let Some(path) = path else {
            return Ok(None);
        };
        let file = open_append(path)?;
        let restore = dup_fd(target_fd)?;
        Ok(Some(PreparedTarget { file, restore }))
    }

    fn dup_fd(fd: RawFd) -> io::Result<OwnedFd> {
        let duped = unsafe { libc::dup(fd) };
        if duped < 0 {
            return Err(io::Error::last_os_error());
        }
        let owned = unsafe { OwnedFd::from_raw_fd(duped) };
        set_cloexec(owned.as_raw_fd())?;
        Ok(owned)
    }

    pub(super) fn redirect_fd(target_fd: RawFd, replacement_fd: RawFd) -> io::Result<()> {
        let rc = unsafe { libc::dup2(replacement_fd, target_fd) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub(super) fn mark_uninstalled() {
        INSTALLED.store(false, Ordering::SeqCst);
    }

    fn set_cloexec(fd: RawFd) -> io::Result<()> {
        let current = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if current < 0 {
            return Err(io::Error::last_os_error());
        }
        let rc = unsafe { libc::fcntl(fd, libc::F_SETFD, current | libc::FD_CLOEXEC) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    #![allow(clippy::expect_used)]

    use super::*;
    #[cfg(unix)]
    use std::{
        env, fs,
        os::unix::fs::{PermissionsExt, symlink},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn config_builds_optional_paths() {
        let config =
            StreamRedirectConfig::new(Some("/tmp/afdata-out.jsonl"), Some("/tmp/afdata.err"))
                .expect("valid config")
                .expect("redirection should be enabled");
        assert_eq!(
            config.stdout_file,
            Some(PathBuf::from("/tmp/afdata-out.jsonl"))
        );
        assert_eq!(config.stderr_file, Some(PathBuf::from("/tmp/afdata.err")));
    }

    #[test]
    fn config_without_files_disables_redirection() {
        let config = StreamRedirectConfig::new(None::<PathBuf>, None::<PathBuf>)
            .expect("valid empty config");
        assert_eq!(config, None);
    }

    #[test]
    fn raw_args_support_space_separated_values() {
        let config = config_from_raw_args([
            "agent-cli",
            "--stdout-file",
            "/tmp/agent-cli.out",
            "--stderr-file",
            "/tmp/agent-cli.err",
            "ping",
        ])
        .expect("valid raw args")
        .expect("stream redirection should be enabled");
        assert_eq!(
            config.stdout_file,
            Some(PathBuf::from("/tmp/agent-cli.out"))
        );
        assert_eq!(
            config.stderr_file,
            Some(PathBuf::from("/tmp/agent-cli.err"))
        );
    }

    #[test]
    fn raw_args_support_equals_values() {
        let config = config_from_raw_args([
            "agent-cli",
            "--stdout-file=/tmp/agent-cli.out",
            "--stderr-file=/tmp/agent-cli.err",
            "ping",
        ])
        .expect("valid raw args")
        .expect("stream redirection should be enabled");
        assert_eq!(
            config.stdout_file,
            Some(PathBuf::from("/tmp/agent-cli.out"))
        );
        assert_eq!(
            config.stderr_file,
            Some(PathBuf::from("/tmp/agent-cli.err"))
        );
    }

    #[test]
    fn raw_args_accept_single_stream() {
        let config = config_from_raw_args(["agent-cli", "--stderr-file", "/tmp/agent-cli.err"])
            .expect("valid raw args")
            .expect("stderr-only redirection should be enabled");
        assert_eq!(config.stdout_file, None);
        assert_eq!(
            config.stderr_file,
            Some(PathBuf::from("/tmp/agent-cli.err"))
        );
    }

    #[test]
    fn raw_args_reject_missing_values() {
        assert!(config_from_raw_args(["agent-cli", "--stdout-file"]).is_err());
        assert!(config_from_raw_args(["agent-cli", "--stderr-file", "--help"]).is_err());
    }

    #[test]
    fn raw_args_disable_redirection_without_file_flags() {
        assert_eq!(
            config_from_raw_args(["agent-cli", "ping"]).expect("valid raw args without file flags"),
            None
        );
    }

    #[cfg(not(unix))]
    #[test]
    fn install_reports_unsupported_on_non_unix() {
        let config = StreamRedirectConfig::new(Some("stdout.log"), None::<PathBuf>)
            .expect("valid config")
            .expect("redirection should be enabled");
        let err = install(&config).expect_err("non-unix install must be unsupported");
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert!(err.to_string().contains("only supported on Unix"));
    }

    #[cfg(unix)]
    #[test]
    fn install_redirects_stdout_and_stderr_in_child_process() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "afdata-stream-redirect-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp directory");
        let stdout_file = dir.join("stdout.log");
        let stderr_file = dir.join("stderr.log");
        fs::write(&stdout_file, "existing stdout\n").expect("prewrite stdout file");

        let status = Command::new(env::current_exe().expect("current test executable"))
            .arg("--exact")
            .arg("stream_redirect::tests::stream_redirect_child_writes_to_files")
            .arg("--nocapture")
            .env("AFDATA_STREAM_REDIRECT_CHILD", "1")
            .env("AFDATA_STREAM_REDIRECT_STDOUT", &stdout_file)
            .env("AFDATA_STREAM_REDIRECT_STDERR", &stderr_file)
            .status()
            .expect("run child test process");
        assert!(status.success(), "child test process failed: {status}");

        assert_eq!(
            fs::read_to_string(&stdout_file).expect("read stdout file"),
            "existing stdout\nstdout bytes\n"
        );
        assert_eq!(
            fs::read_to_string(&stderr_file).expect("read stderr file"),
            "stderr bytes\n"
        );
        assert_eq!(
            fs::metadata(&stderr_file)
                .expect("stderr metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn install_rejects_symbolic_link_targets() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "afdata-stream-redirect-symlink-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp directory");
        let real_file = dir.join("real.log");
        let symlink_file = dir.join("stdout.log");
        fs::write(&real_file, "").expect("create real file");
        symlink(&real_file, &symlink_file).expect("create symlink");

        let err = install_from_cli_args(Some(symlink_file), None::<PathBuf>)
            .expect_err("symlink target must be rejected");
        assert!(
            err.to_string().contains("symbolic link")
                || err.to_string().contains("Too many levels"),
            "{err}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn install_drop_flushes_and_restores_stdout_in_child_process() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "afdata-stream-redirect-restore-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp directory");
        let stdout_file = dir.join("stdout.log");

        let output = Command::new(env::current_exe().expect("current test executable"))
            .arg("--exact")
            .arg("stream_redirect::tests::stream_redirect_child_restores_stdout_after_drop")
            .arg("--nocapture")
            .env("AFDATA_STREAM_REDIRECT_RESTORE_CHILD", "1")
            .env("AFDATA_STREAM_REDIRECT_STDOUT", &stdout_file)
            .output()
            .expect("run child test process");
        assert!(
            output.status.success(),
            "child test process failed: status={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        assert_eq!(
            fs::read_to_string(&stdout_file).expect("read stdout file"),
            "redirected before drop\n"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("stdout after restore\n"),
            "restored stdout should reach parent capture: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn install_reports_existing_redirect_and_recovers_after_drop_in_child_process() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "afdata-stream-redirect-reinstall-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp directory");
        let stdout_file = dir.join("stdout.log");
        let stderr_file = dir.join("stderr.log");

        let output = Command::new(env::current_exe().expect("current test executable"))
            .arg("--exact")
            .arg("stream_redirect::tests::stream_redirect_child_reinstalls_after_drop")
            .arg("--nocapture")
            .env("AFDATA_STREAM_REDIRECT_REINSTALL_CHILD", "1")
            .env("AFDATA_STREAM_REDIRECT_STDOUT", &stdout_file)
            .env("AFDATA_STREAM_REDIRECT_STDERR", &stderr_file)
            .output()
            .expect("run child test process");
        assert!(
            output.status.success(),
            "child test process failed: status={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );

        assert_eq!(
            fs::read_to_string(&stdout_file).expect("read stdout file"),
            "first redirect still usable\n"
        );
        assert_eq!(
            fs::read_to_string(&stderr_file).expect("read stderr file"),
            "stderr after reinstall\n"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn stream_redirect_child_writes_to_files() {
        if env::var_os("AFDATA_STREAM_REDIRECT_CHILD").is_none() {
            return;
        }
        let stdout_file =
            PathBuf::from(env::var_os("AFDATA_STREAM_REDIRECT_STDOUT").expect("stdout path"));
        let stderr_file =
            PathBuf::from(env::var_os("AFDATA_STREAM_REDIRECT_STDERR").expect("stderr path"));
        let _redirect = install_from_cli_args(Some(stdout_file), Some(stderr_file))
            .expect("install stream redirect")
            .expect("stream redirect enabled");
        io::stdout()
            .write_all(b"stdout bytes\n")
            .expect("write stdout bytes");
        io::stderr()
            .write_all(b"stderr bytes\n")
            .expect("write stderr bytes");
    }

    #[cfg(unix)]
    #[test]
    fn stream_redirect_child_restores_stdout_after_drop() {
        if env::var_os("AFDATA_STREAM_REDIRECT_RESTORE_CHILD").is_none() {
            return;
        }
        let stdout_file =
            PathBuf::from(env::var_os("AFDATA_STREAM_REDIRECT_STDOUT").expect("stdout path"));
        let redirect = install_from_cli_args(Some(stdout_file), None::<PathBuf>)
            .expect("install stream redirect")
            .expect("stream redirect enabled");
        io::stdout()
            .write_all(b"redirected before drop\n")
            .expect("write redirected stdout bytes");
        drop(redirect);
        io::stdout()
            .write_all(b"stdout after restore\n")
            .expect("write restored stdout bytes");
        io::stdout().flush().expect("flush restored stdout");
    }

    #[cfg(unix)]
    #[test]
    fn stream_redirect_child_reinstalls_after_drop() {
        if env::var_os("AFDATA_STREAM_REDIRECT_REINSTALL_CHILD").is_none() {
            return;
        }
        let stdout_file =
            PathBuf::from(env::var_os("AFDATA_STREAM_REDIRECT_STDOUT").expect("stdout path"));
        let stderr_file =
            PathBuf::from(env::var_os("AFDATA_STREAM_REDIRECT_STDERR").expect("stderr path"));
        let first = install_from_cli_args(Some(stdout_file), None::<PathBuf>)
            .expect("install first stream redirect")
            .expect("first stream redirect enabled");
        io::stdout()
            .write_all(b"first redirect still usable\n")
            .expect("write first redirected stdout bytes");

        let err = install_from_cli_args(None::<PathBuf>, Some(stderr_file.clone()))
            .expect_err("second install must report an active redirect");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            err.to_string().contains("already installed"),
            "unexpected error message: {err}"
        );

        drop(first);
        let second = install_from_cli_args(None::<PathBuf>, Some(stderr_file))
            .expect("install second stream redirect after drop")
            .expect("second stream redirect enabled");
        io::stderr()
            .write_all(b"stderr after reinstall\n")
            .expect("write reinstalled stderr bytes");
        drop(second);
    }
}
