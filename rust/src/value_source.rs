//! Reading a value that named where it is, and the policy that separates a
//! printable value from a credential.
//!
//! The grammar — which sources exist, which an argument accepts, how one value
//! is classified — belongs to the CLI core and lives in
//! [`crate::cli_spec::SourceSet`]. This is the other half: doing the read, and
//! deciding what may then be done with the result.
//!
//! # Mechanism there, policy here
//!
//! Nothing about a source is specific to secrets — reading a dot path out of a
//! config file is the same operation whether it yields a password or a port.
//! What differs is what may be done with the result, and that difference is
//! carried by the *return type* rather than by a flag someone can forget:
//!
//! - [`ValueSource::read`] answers a [`String`]. Its errors may quote the file
//!   and the parser's own complaint, because being helpful is the point.
//! - [`ValueSource::read_secret`] answers a
//!   [`SecretString`](crate::value_source::SecretString), which cannot be
//!   printed, logged, or serialized without saying `expose_secret` out loud.
//!   Its errors are stripped of anything that could echo what was read, and it
//!   refuses a non-string value outright — a credential is never a number.
//!
//! Both cap the read. An unbounded read of a caller-named path is a denial of
//! service regardless of what the bytes turn out to be.

use std::fmt;
use std::path::Path;

use crate::cli_spec::{SourceError, ValueSource};
use crate::document::{DocumentFile, Format, Value};

/// 16 MiB read limit for file-backed value sources, and the CLI's default
/// document budget (`--max-file-bytes`).
///
/// Library callers of [`DocumentFile::open_capped`] choose their own budget.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Credential streams and prompts retain their smaller bound.
const MAX_STREAM_BYTES: usize = 1024 * 1024;

type Result<T> = std::result::Result<T, SourceError>;

// ── SecretString ────────────────────────────────────────────────────────────

/// A string that cannot be printed by accident.
///
/// `Debug` and `Display` both render `***`, and there is deliberately no
/// `Serialize`: a payload that genuinely needs the value asks for it with
/// [`SecretString::expose_secret`], which is greppable in review in a way that
/// `format!("{value}")` is not.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value itself. Call this at the boundary that needs it — the header
    /// being signed, the connection being opened — and nowhere else.
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("***")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("***")
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl ValueSource {
    /// Read the value.
    ///
    /// Errors may quote the file and the parser's complaint: for an ordinary
    /// value, saying what was wrong with it is the whole point of the error.
    /// A scalar that is not a string — a port, a boolean — is rendered in its
    /// canonical spelling; a collection is refused.
    pub fn read(&self) -> Result<String> {
        read_with(self, Policy::Plain)
    }

    /// Read the value as a secret.
    ///
    /// The result cannot be printed without [`SecretString::expose_secret`],
    /// errors carry nothing that could echo what was read, and a non-string
    /// value is refused rather than coerced.
    pub fn read_secret(&self) -> Result<SecretString> {
        read_with(self, Policy::Secret).map(SecretString)
    }
}

fn read_with(source: &ValueSource, policy: Policy) -> Result<String> {
    match source {
        ValueSource::Literal(value) => Ok(value.clone()),
        ValueSource::Env(name) => std::env::var(name).map_err(|error| {
            let reason = match error {
                std::env::VarError::NotPresent => "is unset",
                std::env::VarError::NotUnicode(_) => "is not valid UTF-8",
            };
            SourceError::unreadable(format!("environment variable `{name}` {reason}"))
        }),
        ValueSource::File {
            path,
            dot_path,
            format,
        } => read_file(path, dot_path, format.as_deref(), policy),
        ValueSource::Stdin if policy == Policy::Secret => {
            read_stream(std::io::stdin().lock(), "stdin")
        }
        ValueSource::Stdin => {
            read_stream_with_limit(std::io::stdin().lock(), "stdin", MAX_FILE_BYTES as usize)
        }
        ValueSource::Fd(number) => read_fd(
            *number,
            if policy == Policy::Secret {
                MAX_STREAM_BYTES
            } else {
                MAX_FILE_BYTES as usize
            },
        ),
        ValueSource::Prompt => read_prompt(),
        ValueSource::Host { scheme, .. } => Err(SourceError::unreadable(format!(
            "`{scheme}` is a host-defined source; this crate cannot read it"
        ))),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    Plain,
    Secret,
}

fn read_file(
    path: &Path,
    dot_path: &str,
    named_format: Option<&str>,
    policy: Policy,
) -> Result<String> {
    // A named format only says how to parse a document, and a whole-file read
    // parses nothing; accepting one would silently ignore it.
    if let Some(name) = named_format
        && dot_path.is_empty()
    {
        return Err(SourceError::invalid(format!(
            "`file+{name}:` needs a `#DOT_PATH`; use `file:PATH` to read the whole file"
        )));
    }
    if let Some(name) = named_format
        && Format::from_cli_name(name).is_none()
    {
        return Err(SourceError::invalid(format!(
            "`file+{name}:` is not a format this build reads"
        )));
    }
    // No document address means the complete UTF-8 file, without parsing or
    // scalar conversion. This also supports documents passed to typed callers.
    if dot_path.is_empty() {
        return crate::document::file::read_capped_file(
            path,
            MAX_FILE_BYTES,
            crate::document::SymlinkPolicy::Follow,
        )
        .map_err(|error| SourceError::unreadable(error.redacted_message()));
    }
    // A named format is resolved here rather than in the grammar: the CLI core
    // that parses argv is not allowed to know what a document format is, so it
    // carries the caller's word and this is where the word becomes a parser.
    let format = match named_format {
        Some(name) => Format::from_cli_name(name).ok_or_else(|| {
            SourceError::invalid(format!("`file+{name}:` is not a format this build reads"))
        })?,
        None => Format::detect(path).ok_or_else(|| match Format::unavailable(path) {
            Some(feature) => SourceError::unreadable(format!(
                "cannot read {}: this build has no {feature} support",
                path.display()
            )),
            None => SourceError::invalid(format!(
                "cannot tell the config format of {} from its name; name it with \
                 file+FORMAT:{}#{dot_path}, or use a .json/.toml/.yaml/.env/.ini file",
                path.display(),
                path.display()
            )),
        })?,
    };
    // `open_capped` rejects a non-regular file before reading a byte and limits
    // the read, so naming a device or a huge file fails instead of hanging.
    let document = DocumentFile::open_capped(path, Some(format), MAX_FILE_BYTES).map_err(
        |error| match policy {
            // `redacted_message` drops the parser detail, which for a
            // secret-bearing file is the part that would quote the secret.
            Policy::Secret => SourceError::unreadable(format!(
                "cannot read {} config {}: {}",
                format.name(),
                path.display(),
                error.redacted_message()
            )),
            Policy::Plain => SourceError::unreadable(format!(
                "cannot read {} config {}: {error}",
                format.name(),
                path.display()
            )),
        },
    )?;
    let value = document.value_at(dot_path).map_err(|error| {
        if error.code() == "document_path_not_found" {
            SourceError::unreadable(format!("{dot_path} was not found in {}", path.display()))
        } else {
            SourceError::unreadable(format!("cannot resolve {dot_path} in {}", path.display()))
        }
    })?;
    scalar(value, path, dot_path, policy)
}

fn scalar(value: Value, path: &Path, dot_path: &str, policy: Policy) -> Result<String> {
    let refused = |kind: &str| {
        SourceError::unreadable(format!(
            "{dot_path} in {} is {kind}, which is not a value",
            path.display()
        ))
    };
    match value {
        Value::String(value) => Ok(value),
        // A credential is text. A number that resolves where a secret was
        // expected is a mis-addressed dot path, not a password.
        other if policy == Policy::Secret => Err(SourceError::unreadable(format!(
            "{dot_path} in {} is {}; a secret must be a string",
            path.display(),
            other.kind_name()
        ))),
        Value::Integer(value) => Ok(value.to_string()),
        Value::Unsigned(value) => Ok(value.to_string()),
        Value::Float(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Null => Err(refused("null")),
        Value::Array(_) => Err(refused("an array")),
        Value::Object(_) => Err(refused("an object")),
    }
}

/// Verbatim, including any trailing newline: a value may legitimately end in
/// whitespace, and this crate cannot tell that from a shell that added one.
/// Use `printf '%s'` rather than `echo`.
fn read_stream<R: std::io::Read>(reader: R, source: &str) -> Result<String> {
    read_stream_with_limit(reader, source, MAX_STREAM_BYTES)
}

fn read_stream_with_limit<R: std::io::Read>(
    reader: R,
    source: &str,
    max_bytes: usize,
) -> Result<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| SourceError::unreadable(format!("read from {source}: {error}")))?;
    if bytes.len() > max_bytes {
        return Err(SourceError::unreadable(format!(
            "{source} exceeds {max_bytes} bytes"
        )));
    }
    String::from_utf8(bytes)
        .map_err(|_| SourceError::unreadable(format!("{source} must carry valid UTF-8")))
}

#[cfg(unix)]
fn read_fd(number: i32, max_bytes: usize) -> Result<String> {
    #[cfg(feature = "libc")]
    let file = {
        use std::os::fd::FromRawFd;

        // Duplicate the caller-owned descriptor: `read(&self)` must not close
        // a handle it did not create, and leaving the number in `ValueSource`
        // after closing it could make a later call read an unrelated descriptor
        // that the process reused.
        // SAFETY: `dup` accepts any integer and reports an invalid descriptor as
        // `-1`. `from_raw_fd` is called only for the new descriptor it returned.
        let duplicated = unsafe { libc::dup(number) };
        if duplicated < 0 {
            return Err(SourceError::unreadable(format!(
                "open file descriptor {number}: {}",
                std::io::Error::last_os_error()
            )));
        }
        // SAFETY: `duplicated` is a fresh owned descriptor from successful
        // `dup`, transferred exactly once into `File`.
        unsafe { std::fs::File::from_raw_fd(duplicated) }
    };
    #[cfg(not(feature = "libc"))]
    let file = std::fs::File::open(format!("/dev/fd/{number}")).map_err(|error| {
        SourceError::unreadable(format!("open file descriptor {number}: {error}"))
    })?;
    read_stream_with_limit(file, "file descriptor", max_bytes)
}

#[cfg(not(unix))]
fn read_fd(_number: i32, _max_bytes: usize) -> Result<String> {
    Err(SourceError::unreadable(
        "the `fd` source is unsupported on this platform",
    ))
}

#[cfg(all(unix, feature = "libc"))]
fn read_prompt() -> Result<String> {
    use std::io::Write;

    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|error| {
            SourceError::unreadable(format!("open the controlling terminal: {error}"))
        })?;
    let restore_tty = tty.try_clone().map_err(|error| {
        SourceError::unreadable(format!("prepare terminal echo restoration: {error}"))
    })?;
    let original = disable_terminal_echo(&tty)
        .map_err(|error| SourceError::unreadable(format!("disable terminal echo: {error}")))?;
    let _echo = EchoGuard {
        tty: restore_tty,
        original,
    };
    write!(tty, "Value: ")
        .map_err(|error| SourceError::unreadable(format!("write the prompt: {error}")))?;
    let reader = std::io::BufReader::new(&mut tty);
    let value = read_prompt_line(reader);
    // The newline the person typed is the terminator, not part of the value.
    let _ = writeln!(tty);
    value
}

// Reading a secret with the echo still on is not a degraded read, it is the
// wrong one — the value ends up on the screen and in whatever recorded it. So a
// build that cannot turn echo off refuses the source instead.
#[cfg(all(unix, not(feature = "libc")))]
fn read_prompt() -> Result<String> {
    Err(SourceError::unreadable(
        "the `prompt` source needs Cargo feature `libc` to turn terminal echo off",
    ))
}

#[cfg(windows)]
fn read_prompt() -> Result<String> {
    use std::io::Write as _;

    // `CONIN$` and `CONOUT$` are this process's own console, the way `/dev/tty`
    // is its controlling terminal: they are the console whatever stdin and
    // stdout were redirected to, which is the whole reason a prompt does not
    // read stdin. Input has to be opened for writing as well — `SetConsoleMode`
    // is a write to the input buffer's settings, not a read of them.
    let input = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONIN$")
        .map_err(|error| SourceError::unreadable(format!("open the console input: {error}")))?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .open("CONOUT$")
        .map_err(|error| SourceError::unreadable(format!("open the console output: {error}")))?;
    let restore_console = input.try_clone().map_err(|error| {
        SourceError::unreadable(format!("prepare console echo restoration: {error}"))
    })?;
    let restore_output = output.try_clone().map_err(|error| {
        SourceError::unreadable(format!("prepare console echo restoration: {error}"))
    })?;
    let original = disable_console_echo(&input)
        .map_err(|error| SourceError::unreadable(format!("disable console echo: {error}")))?;
    let _echo = EchoGuard {
        console: restore_console,
        output: restore_output,
        original,
    };
    write!(output, "Value: ")
        .map_err(|error| SourceError::unreadable(format!("write the prompt: {error}")))?;
    let value = read_console_line(&input);
    // The newline the person typed is the terminator, not part of the value.
    // With echo off the console printed nothing for it either, so the cursor is
    // still on the prompt line until this is written.
    let _ = writeln!(output);
    value
}

#[cfg(all(not(unix), not(windows)))]
fn read_prompt() -> Result<String> {
    Err(SourceError::unreadable(
        "the `prompt` source is unsupported on this platform",
    ))
}

/// The console API this needs, declared rather than depended on.
///
/// Three functions from `kernel32`, which is already linked into every Windows
/// program. A crate wrapping the whole Win32 surface would be a dependency —
/// and a supply chain — for what fits here in fifteen lines, and this spore
/// already reaches the known-folder API the same way.
#[cfg(windows)]
mod windows_console {
    use std::ffi::c_void;

    /// Keystrokes are echoed as they are typed. Clearing this bit is the entire
    /// point of the module.
    pub(super) const ENABLE_ECHO_INPUT: u32 = 0x0004;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub(super) fn GetConsoleMode(console: *mut c_void, mode: *mut u32) -> i32;
        pub(super) fn SetConsoleMode(console: *mut c_void, mode: u32) -> i32;
        pub(super) fn ReadConsoleW(
            console: *mut c_void,
            buffer: *mut u16,
            units_to_read: u32,
            units_read: *mut u32,
            input_control: *mut c_void,
        ) -> i32;
    }
}

/// Turn echo off on the console, returning the mode to put back.
///
/// The unix note about not spawning `stty` applies here for the same reasons,
/// and so does the one about verifying: `SetConsoleMode` rejects the whole mode
/// word or accepts it, but what the console ends up with is still worth reading
/// back — `ENABLE_ECHO_INPUT` is only honoured alongside `ENABLE_LINE_INPUT`,
/// so a console already in raw mode can take the call and echo anyway.
#[cfg(windows)]
fn disable_console_echo(console: &std::fs::File) -> std::io::Result<u32> {
    let original = console_mode(console)?;
    set_console_mode(console, original & !windows_console::ENABLE_ECHO_INPUT)?;
    if console_mode(console)? & windows_console::ENABLE_ECHO_INPUT != 0 {
        // Put back what was there before refusing: a half-applied change is
        // still a change.
        let _ = set_console_mode(console, original);
        return Err(std::io::Error::other("console echo is still enabled"));
    }
    Ok(original)
}

#[cfg(windows)]
fn console_mode(console: &std::fs::File) -> std::io::Result<u32> {
    use std::os::windows::io::AsRawHandle as _;

    let mut mode = 0u32;
    // SAFETY: `console` is a live open handle for the duration of the call, and
    // `mode` is a writable `u32` the API fills in on success.
    let status = unsafe { windows_console::GetConsoleMode(console.as_raw_handle(), &mut mode) };
    if status == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(mode)
}

#[cfg(windows)]
fn set_console_mode(console: &std::fs::File, mode: u32) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle as _;

    // SAFETY: `console` is a live open handle for the duration of the call, and
    // `mode` is passed by value.
    let status = unsafe { windows_console::SetConsoleMode(console.as_raw_handle(), mode) };
    if status == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Read one line of UTF-16 from the console.
///
/// Deliberately not the `Read` implementation on the same handle: that goes
/// through `ReadFile`, which answers bytes in the console's current input code
/// page. On a console whose code page is not UTF-8 — the default on most
/// installs outside the US — a value with any character beyond ASCII comes back
/// as different bytes, and this crate would then either refuse it as invalid
/// UTF-8 or accept a silently different secret. `ReadConsoleW` answers UTF-16,
/// which converts losslessly.
#[cfg(windows)]
fn read_console_line(console: &std::fs::File) -> Result<String> {
    use std::os::windows::io::AsRawHandle as _;

    // Enough to hold any accepted value plus the shortest overlong one, so
    // exceeding the cap is detected here rather than truncated into a valid
    // read: a UTF-16 sequence is never longer in units than its UTF-8 encoding
    // is in bytes, and the extra units cover CRLF.
    let mut buffer = vec![0u16; MAX_STREAM_BYTES + 4];
    let units_to_read = u32::try_from(buffer.len())
        .map_err(|_| SourceError::unreadable("the console read buffer does not fit a request"))?;
    let mut units_read = 0u32;
    // SAFETY: `console` is a live open handle, `buffer` is a writable
    // allocation of exactly `units_to_read` `u16`s, and `units_read` is a
    // writable `u32`. The input-control argument is optional and null here.
    let status = unsafe {
        windows_console::ReadConsoleW(
            console.as_raw_handle(),
            buffer.as_mut_ptr(),
            units_to_read,
            &mut units_read,
            std::ptr::null_mut(),
        )
    };
    if status == 0 {
        return Err(SourceError::unreadable(format!(
            "read from the console: {}",
            std::io::Error::last_os_error()
        )));
    }
    let units = buffer
        .get(..units_read as usize)
        .ok_or_else(|| SourceError::unreadable("the console reported reading past its buffer"))?;
    let text = String::from_utf16(units)
        .map_err(|_| SourceError::unreadable("the console answered malformed UTF-16"))?;
    // The same cap and the same line ending handling as every other prompt.
    read_prompt_line(std::io::Cursor::new(text.as_bytes()))
}

/// Turn echo off on `tty`, returning the settings to put back.
///
/// This talks to the terminal directly rather than running `stty`. Spawning
/// anything here means resolving a name through `PATH` immediately before a
/// secret is typed, and it makes the one thing standing between the value and
/// the screen depend on a program being installed. `tcsetattr` also reports
/// success when it applied only some of what was asked, so what it did is read
/// back and checked rather than assumed.
#[cfg(all(unix, feature = "libc"))]
fn disable_terminal_echo(tty: &std::fs::File) -> std::io::Result<libc::termios> {
    let original = terminal_attributes(tty)?;
    let mut quiet = original;
    quiet.c_lflag &= !libc::ECHO;
    set_terminal_attributes(tty, &quiet)?;
    if terminal_attributes(tty)?.c_lflag & libc::ECHO != 0 {
        // Put back what was there before refusing: a half-applied change is
        // still a change.
        let _ = set_terminal_attributes(tty, &original);
        return Err(std::io::Error::other("terminal echo is still enabled"));
    }
    Ok(original)
}

#[cfg(all(unix, feature = "libc"))]
fn terminal_attributes(tty: &std::fs::File) -> std::io::Result<libc::termios> {
    use std::os::fd::AsRawFd as _;

    let mut attributes = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `tty` is a live open descriptor for the duration of the call, and
    // `tcgetattr` initializes the whole `termios` it is given on success.
    let status = unsafe { libc::tcgetattr(tty.as_raw_fd(), attributes.as_mut_ptr()) };
    if status != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `tcgetattr` reported success, so the value is initialized.
    Ok(unsafe { attributes.assume_init() })
}

#[cfg(all(unix, feature = "libc"))]
fn set_terminal_attributes(tty: &std::fs::File, attributes: &libc::termios) -> std::io::Result<()> {
    use std::os::fd::AsRawFd as _;

    // `TCSAFLUSH` discards input typed before the change took effect, so
    // keystrokes racing the switch cannot be echoed after it.
    // SAFETY: `tty` is a live open descriptor and `attributes` is an
    // initialized `termios` that outlives the call.
    let status = unsafe { libc::tcsetattr(tty.as_raw_fd(), libc::TCSAFLUSH, attributes) };
    if status != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(any(all(unix, feature = "libc"), windows, test))]
fn read_prompt_line<R: std::io::BufRead>(reader: R) -> Result<String> {
    use std::io::BufRead;

    // Leave room for CRLF beyond the value cap. `Take` bounds `read_line`'s
    // allocation even when the terminal never sends a newline.
    let mut limited = reader.take((MAX_STREAM_BYTES + 2) as u64);
    let mut value = String::new();
    let read = limited
        .read_line(&mut value)
        .map_err(|error| SourceError::unreadable(format!("read from the terminal: {error}")))?;
    if read == 0 {
        return Err(SourceError::unreadable(
            "the terminal closed before a value was entered",
        ));
    }
    let value = match value.strip_suffix('\n') {
        Some(line) => line.strip_suffix('\r').unwrap_or(line),
        None => &value,
    };
    if value.len() > MAX_STREAM_BYTES {
        return Err(SourceError::unreadable(format!(
            "prompt exceeds {MAX_STREAM_BYTES} bytes"
        )));
    }
    Ok(value.to_string())
}

/// Puts terminal echo back however the read ended, including on a panic.
#[cfg(all(unix, feature = "libc"))]
struct EchoGuard {
    tty: std::fs::File,
    original: libc::termios,
}

#[cfg(all(unix, feature = "libc"))]
impl Drop for EchoGuard {
    fn drop(&mut self) {
        use std::io::Write as _;

        if set_terminal_attributes(&self.tty, &self.original).is_ok() {
            return;
        }
        // The terminal was borrowed and must be handed back. Failing quietly
        // leaves the person typing into a shell that shows nothing and looks
        // broken, with no clue why. Said on the terminal itself, which is where
        // the damage is, rather than through the caller's structured output.
        let _ = writeln!(
            &mut self.tty,
            "\nwarning: could not restore terminal echo; run `stty echo` to fix this terminal"
        );
    }
}

/// Puts console echo back however the read ended, including on a panic.
///
/// Two handles rather than the unix one: `CONIN$` carries the mode to restore
/// and cannot be written to, `CONOUT$` is where anything a person should read
/// has to go.
#[cfg(windows)]
struct EchoGuard {
    console: std::fs::File,
    output: std::fs::File,
    original: u32,
}

#[cfg(windows)]
impl Drop for EchoGuard {
    fn drop(&mut self) {
        use std::io::Write as _;

        if set_console_mode(&self.console, self.original).is_ok() {
            return;
        }
        // As on unix: the console was borrowed and must be handed back, and a
        // console that shows nothing while a person types looks broken rather
        // than quiet. Windows has no `stty echo` to name — the mode belongs to
        // this console's input buffer and goes when the console does.
        let _ = writeln!(
            &mut self.output,
            "\nwarning: could not restore console echo; close this console window to get it back"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_file_sources_preserve_text_and_accept_large_documents() {
        let directory = tempfile::tempdir().expect("temporary source directory");
        let path = directory.path().join("request.json");
        let text = format!("{{\"text\":\"{}\"}}\n", "x".repeat(1300 * 1024));
        std::fs::write(&path, &text).expect("write source");
        let source = ValueSource::File {
            path,
            dot_path: String::new(),
            format: None,
        };
        assert_eq!(source.read().expect("read complete file"), text);
        assert_eq!(
            read_stream_with_limit(text.as_bytes(), "stdin", MAX_FILE_BYTES as usize)
                .expect("read stream"),
            text
        );
        let directory_source = ValueSource::File {
            path: directory.path().to_owned(),
            dot_path: String::new(),
            format: None,
        };
        assert!(directory_source.read().is_err());
    }

    #[cfg(all(unix, feature = "libc"))]
    #[test]
    fn terminal_echo_control_goes_to_the_terminal_not_to_a_program() {
        use std::io::Write as _;

        // A regular file is not a terminal. `tcsetattr` answers ENOTTY, which
        // is an answer only the terminal API can give: a spawned `stty`
        // resolved through `PATH` would report a child's exit status instead,
        // and whatever `PATH` happened to name would already have run.
        let path = std::env::temp_dir().join(format!(
            "afdata_not_a_tty_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let mut file = match std::fs::File::create(&path) {
            Ok(file) => file,
            Err(_) => return,
        };
        let _ = file.write_all(b"not a terminal");

        let error = disable_terminal_echo(&file)
            .err()
            .map(|error| error.raw_os_error());

        let _ = std::fs::remove_file(&path);
        assert_eq!(
            error,
            Some(Some(libc::ENOTTY)),
            "echo control must fail as a terminal call, not as a missing program"
        );
    }

    #[cfg(all(unix, feature = "libc"))]
    #[test]
    fn a_failed_restore_is_reported_rather_than_swallowed() {
        // The guard writes its warning to the terminal it was handed. Dropping
        // one over a non-terminal exercises the failure path end to end: it
        // must neither panic nor hang, and the file it wrote to shows that the
        // failure was announced instead of ignored.
        let path = std::env::temp_dir().join(format!(
            "afdata_echo_guard_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let Ok(file) = std::fs::File::options()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&path)
        else {
            return;
        };
        // SAFETY: a zeroed `termios` is a valid value to hand back to
        // `tcsetattr`; this descriptor is not a terminal, so it never reaches
        // one.
        let original = unsafe { std::mem::zeroed::<libc::termios>() };
        drop(EchoGuard {
            tty: file,
            original,
        });

        let announced = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = std::fs::remove_file(&path);
        assert!(
            announced.contains("could not restore terminal echo"),
            "a terminal left without echo must say so: {announced:?}"
        );
    }

    /// The Windows counterpart of the two tests above, asserting the same two
    /// properties against the console API.
    #[cfg(windows)]
    mod windows_echo {
        use super::super::{EchoGuard, disable_console_echo};

        /// `ERROR_INVALID_HANDLE`. A regular file is not a console, and this is
        /// the console API saying so — a spawned helper resolved through `PATH`
        /// would report a child's exit status instead, and whatever `PATH`
        /// happened to name would already have run.
        const ERROR_INVALID_HANDLE: i32 = 6;

        fn scratch_file(label: &str) -> Option<(std::path::PathBuf, std::fs::File)> {
            let path = std::env::temp_dir().join(format!(
                "afdata_{label}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            let file = std::fs::File::options()
                .create(true)
                .truncate(true)
                .read(true)
                .write(true)
                .open(&path)
                .ok()?;
            Some((path, file))
        }

        #[test]
        fn echo_control_goes_to_the_console_not_to_a_program() {
            let Some((path, file)) = scratch_file("not_a_console") else {
                return;
            };

            let error = disable_console_echo(&file)
                .err()
                .and_then(|error| error.raw_os_error());

            drop(file);
            let _ = std::fs::remove_file(&path);
            assert_eq!(
                error,
                Some(ERROR_INVALID_HANDLE),
                "echo control must fail as a console call, not as a missing program"
            );
        }

        #[test]
        fn a_failed_restore_is_reported_rather_than_swallowed() {
            let Some((path, file)) = scratch_file("console_echo_guard") else {
                return;
            };
            let Ok(output) = file.try_clone() else {
                let _ = std::fs::remove_file(&path);
                return;
            };

            // Restoring over a non-console fails, which is the path under test:
            // it must neither panic nor hang, and must leave the warning behind.
            drop(EchoGuard {
                console: file,
                output,
                original: 0,
            });

            let announced = std::fs::read_to_string(&path).unwrap_or_default();
            let _ = std::fs::remove_file(&path);
            assert!(
                announced.contains("could not restore console echo"),
                "a console left without echo must say so: {announced:?}"
            );
        }
    }

    use crate::cli_spec::SourceSet;
    use std::path::PathBuf;

    fn temp_config(name: &str, extension: &str, content: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "afdata-value-source-{name}-{}.{extension}",
            std::process::id()
        ));
        std::fs::write(&path, content).expect("write test config");
        path
    }

    /// Every format this build can parse. The list is assembled by feature
    /// because `scripts/test.sh unit` also runs `--no-default-features
    /// --features cli`, where a `.toml` file is not a format this binary knows
    /// — and a source that answers "no toml support" there is correct
    /// behavior, not a failure to assert against.
    fn readable_formats() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
        // In the `cli`-only test build none of the conditional pushes compile,
        // while every normal build needs the mutability.
        #[allow(unused_mut)]
        let mut cases: Vec<(&str, &str, &str, &str)> =
            vec![("json", "json", r#"{"a":{"b":" v "}}"#, "a.b")];
        #[cfg(feature = "toml")]
        cases.push(("toml", "toml", "[a]\nb = ' v '\n", "a.b"));
        #[cfg(feature = "yaml")]
        cases.push(("yaml", "yaml", "a:\n  b: ' v '\n", "a.b"));
        #[cfg(feature = "dotenv")]
        cases.push(("dotenv", "env", "A_B=' v '\n", "A_B"));
        cases
    }

    #[test]
    fn a_file_source_reads_one_address_out_of_every_format() {
        for (name, extension, content, dot_path) in readable_formats() {
            let path = temp_config(name, extension, content);
            let source = ValueSource::File {
                path: path.clone(),
                dot_path: dot_path.to_string(),
                format: None,
            };
            let read = source.read();
            let secret = source.read_secret();
            std::fs::remove_file(&path).expect("remove test config");
            // Verbatim: surrounding space is part of the value.
            assert_eq!(read.as_deref(), Ok(" v "), "{name}");
            assert_eq!(
                secret.expect("secret read").expose_secret(),
                " v ",
                "{name}"
            );
        }
    }

    #[test]
    fn an_empty_string_is_still_a_value() {
        let path = temp_config("empty", "json", r#"{"empty":""}"#);
        let source = ValueSource::File {
            path: path.clone(),
            dot_path: "empty".to_string(),
            format: None,
        };
        assert_eq!(source.read().as_deref(), Ok(""));
        let secret = source.read_secret().expect("empty secret remains explicit");
        assert!(secret.is_empty());
        std::fs::remove_file(&path).expect("remove test config");
    }

    /// A filename does not always say what a file is. `phoenix.conf`,
    /// `/etc/*.conf`, an extensionless credential file — refusing those sent
    /// callers back to `grep | cut`, which is the thing a source replaces.
    ///
    /// INI-gated: without that parser this build genuinely cannot read the
    /// file, and refusing to is the correct answer rather than a failure.
    #[cfg(feature = "ini")]
    #[test]
    fn a_named_format_reads_a_file_whose_name_cannot_say_what_it_is() {
        let path = temp_config("named", "conf", "http-password=abc123\nauto-liquidity=2m\n");
        let named = ValueSource::File {
            path: path.clone(),
            dot_path: "http-password".to_string(),
            format: Some("ini".to_string()),
        };
        let unnamed = ValueSource::File {
            path: path.clone(),
            dot_path: "http-password".to_string(),
            format: None,
        };
        let bad_name = ValueSource::File {
            path: path.clone(),
            dot_path: "http-password".to_string(),
            format: Some("nonsense".to_string()),
        };
        let read = named.read_secret();
        let without = unnamed.read();
        let bad = bad_name.read();
        std::fs::remove_file(&path).expect("remove test config");

        assert_eq!(read.expect("named format").expose_secret(), "abc123");
        // Without the name, the error says how to supply one rather than only
        // that it could not guess.
        let without = without.expect_err("no extension to detect");
        assert!(without.message().contains("file+FORMAT:"), "{without}");
        let bad = bad.expect_err("unknown format");
        assert!(
            bad.message().contains("not a format this build reads"),
            "{bad}"
        );
    }

    #[test]
    fn a_named_format_without_an_address_is_refused_not_ignored() {
        let source = ValueSource::File {
            path: PathBuf::from("does-not-matter.cfg"),
            dot_path: String::new(),
            format: Some("toml".to_string()),
        };
        let error = source.read().expect_err("a format with nothing to parse");
        assert!(error.message().contains("#DOT_PATH"), "{error}");
    }

    /// The policy difference, in one place: an ordinary value may be a port; a
    /// secret may not.
    #[test]
    fn a_non_string_scalar_is_a_value_but_never_a_secret() {
        let path = temp_config("scalar", "json", r#"{"port":5432,"on":true}"#);
        let port = ValueSource::File {
            path: path.clone(),
            dot_path: "port".to_string(),
            format: None,
        };
        assert_eq!(port.read().as_deref(), Ok("5432"));
        let error = port.read_secret().expect_err("a secret must be a string");
        assert!(error.message().contains("must be a string"), "{error}");

        let on = ValueSource::File {
            path: path.clone(),
            dot_path: "on".to_string(),
            format: None,
        };
        assert_eq!(on.read().as_deref(), Ok("true"));
        std::fs::remove_file(&path).expect("remove test config");
    }

    /// The other policy difference: a plain read explains itself, a secret read
    /// refuses to quote anything it saw.
    #[test]
    fn a_secret_read_never_echoes_what_it_read() {
        let canary = "AFDATA_SOURCE_CANARY";
        // JSON so the assertion holds in every feature combination the gate
        // builds; what is under test is the policy, not the parser.
        let path = temp_config("malformed", "json", &format!(r#"{{"a": [ {canary}"#));
        let source = ValueSource::File {
            path: path.clone(),
            dot_path: "a".to_string(),
            format: None,
        };
        let plain = source.read().expect_err("malformed");
        let secret = source.read_secret().expect_err("malformed");
        std::fs::remove_file(&path).expect("remove test config");
        assert!(
            !secret.message().contains(canary),
            "secret read leaked: {secret}"
        );
        // The plain read is allowed to be helpful; that is the difference.
        assert!(plain.message().contains("cannot read"), "{plain}");
    }

    #[test]
    fn a_collection_is_not_a_value() {
        let path = temp_config("collection", "json", r#"{"a":{"b":1},"c":[1],"d":null}"#);
        for (dot_path, expected) in [("a", "an object"), ("c", "an array"), ("d", "null")] {
            let source = ValueSource::File {
                path: path.clone(),
                dot_path: dot_path.to_string(),
                format: None,
            };
            let error = source.read().expect_err(dot_path);
            assert!(error.message().contains(expected), "{dot_path}: {error}");
        }
        std::fs::remove_file(&path).expect("remove test config");
    }

    /// Parsed by the core, read by nobody: a host scheme is the host's to read.
    #[test]
    fn a_host_scheme_is_not_this_crates_to_read() {
        let error = SourceSet::config()
            .host_scheme("container", "container:NAME")
            .parse("container:x")
            .expect("parses")
            .read()
            .expect_err("this crate cannot read it");
        assert_eq!(error.code(), "value_source_unreadable");
    }

    #[test]
    fn an_unset_environment_source_names_what_it_tried() {
        const ABSENT: &str = "AFDATA_TEST_ABSENT_VALUE_SOURCE";
        let error = ValueSource::Env(ABSENT.to_string())
            .read()
            .expect_err("unset");
        assert_eq!(error.code(), "value_source_unreadable");
        assert!(error.message().contains(ABSENT), "{error}");
    }

    /// Including through `{:?}`, which is how a secret reaches a log nobody
    /// meant to write.
    #[test]
    fn a_secret_string_cannot_be_printed_by_accident() {
        let secret = SecretString::new("s3cret");
        assert_eq!(format!("{secret}"), "***");
        assert_eq!(format!("{secret:?}"), "***");
        assert!(!format!("{secret:?} {secret}").contains("s3cret"));
        assert_eq!(secret.expose_secret(), "s3cret");
        // Held inside something else, it stays redacted there too.
        #[derive(Debug)]
        struct Config {
            #[allow(dead_code)]
            token_secret: SecretString,
        }
        let printed = format!(
            "{:?}",
            Config {
                token_secret: secret
            }
        );
        assert!(!printed.contains("s3cret"), "{printed}");
    }

    #[test]
    fn a_stream_is_read_verbatim_and_capped() {
        assert_eq!(
            read_stream(" v \n".as_bytes(), "test").as_deref(),
            Ok(" v \n")
        );
        let oversized = vec![b'x'; MAX_STREAM_BYTES + 1];
        let error = read_stream(oversized.as_slice(), "test").expect_err("over the cap");
        assert!(error.message().contains("exceeds"), "{error}");
    }

    #[test]
    fn prompt_eof_is_not_an_empty_answer_and_only_one_line_ending_is_removed() {
        assert!(read_prompt_line(std::io::Cursor::new(b"")).is_err());
        for (input, expected) in [
            ("\n", ""),
            ("\r\n", ""),
            ("value\r\r\n", "value\r"),
            ("value\r", "value\r"),
        ] {
            assert_eq!(
                read_prompt_line(std::io::Cursor::new(input)).expect("answer"),
                expected
            );
        }
    }

    #[test]
    fn a_prompt_line_is_bounded_before_allocation_can_grow_without_limit() {
        let exact = format!("{}\r\n", "x".repeat(MAX_STREAM_BYTES));
        assert_eq!(
            read_prompt_line(std::io::Cursor::new(exact))
                .expect("cap-sized line")
                .len(),
            MAX_STREAM_BYTES
        );
        let oversized = format!("{}\n", "x".repeat(MAX_STREAM_BYTES + 1));
        let error = read_prompt_line(std::io::Cursor::new(oversized)).expect_err("over the cap");
        assert!(error.message().contains("exceeds"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn an_fd_source_never_closes_the_callers_descriptor() {
        use std::io::{Read, Seek};
        use std::os::fd::AsRawFd;

        let path = temp_config("fd", "txt", "descriptor value");
        let mut file = std::fs::File::open(&path).expect("open test descriptor");
        let source = ValueSource::Fd(file.as_raw_fd());
        assert_eq!(source.read().as_deref(), Ok("descriptor value"));

        file.rewind()
            .expect("the caller still owns an open descriptor");
        let mut reread = String::new();
        file.read_to_string(&mut reread)
            .expect("read through caller-owned descriptor");
        assert_eq!(reread, "descriptor value");
        std::fs::remove_file(&path).expect("remove test config");
    }
}
