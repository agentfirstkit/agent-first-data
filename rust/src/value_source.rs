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

/// A file named by a source is a config file, not a data set.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// A stream named by a source carries one value, not a document.
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
        ValueSource::Stdin => read_stream(std::io::stdin().lock(), "stdin"),
        ValueSource::Fd(number) => read_fd(*number),
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
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .take((MAX_STREAM_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| SourceError::unreadable(format!("read from {source}: {error}")))?;
    if bytes.len() > MAX_STREAM_BYTES {
        return Err(SourceError::unreadable(format!(
            "{source} exceeds {MAX_STREAM_BYTES} bytes"
        )));
    }
    String::from_utf8(bytes)
        .map_err(|_| SourceError::unreadable(format!("{source} must carry valid UTF-8")))
}

#[cfg(unix)]
fn read_fd(number: i32) -> Result<String> {
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
    read_stream(file, "file descriptor")
}

#[cfg(not(unix))]
fn read_fd(_number: i32) -> Result<String> {
    Err(SourceError::unreadable(
        "the `fd` source is unsupported on this platform",
    ))
}

#[cfg(unix)]
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
    let disabled = set_terminal_echo(&tty, false)
        .map_err(|error| SourceError::unreadable(format!("disable terminal echo: {error}")))?;
    if !disabled {
        return Err(SourceError::unreadable("disabling terminal echo failed"));
    }
    let _echo = EchoGuard { tty: restore_tty };
    write!(tty, "Value: ")
        .map_err(|error| SourceError::unreadable(format!("write the prompt: {error}")))?;
    let reader = std::io::BufReader::new(&mut tty);
    let value = read_prompt_line(reader);
    // The newline the person typed is the terminator, not part of the value.
    let _ = writeln!(tty);
    value
}

#[cfg(not(unix))]
fn read_prompt() -> Result<String> {
    Err(SourceError::unreadable(
        "the `prompt` source is unsupported on this platform",
    ))
}

/// Restores terminal echo however the read ended, including on a panic.
#[cfg(unix)]
fn set_terminal_echo(tty: &std::fs::File, enabled: bool) -> std::io::Result<bool> {
    use std::process::Stdio;

    let input = tty.try_clone()?;
    std::process::Command::new("stty")
        .arg(if enabled { "echo" } else { "-echo" })
        // `stty` operates on stdin. Point it at the controlling terminal that
        // carries the prompt, not the process stdin (which may be a pipe).
        .stdin(Stdio::from(input))
        // A library helper must not let a child write unstructured diagnostics
        // around the caller's own output protocol.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
}

#[cfg(any(unix, test))]
fn read_prompt_line<R: std::io::BufRead>(reader: R) -> Result<String> {
    use std::io::BufRead;

    // Leave room for CRLF beyond the value cap. `Take` bounds `read_line`'s
    // allocation even when the terminal never sends a newline.
    let mut limited = reader.take((MAX_STREAM_BYTES + 2) as u64);
    let mut value = String::new();
    limited
        .read_line(&mut value)
        .map_err(|error| SourceError::unreadable(format!("read from the terminal: {error}")))?;
    let value = value.trim_end_matches(['\r', '\n']);
    if value.len() > MAX_STREAM_BYTES {
        return Err(SourceError::unreadable(format!(
            "prompt exceeds {MAX_STREAM_BYTES} bytes"
        )));
    }
    Ok(value.to_string())
}

#[cfg(unix)]
struct EchoGuard {
    tty: std::fs::File,
}

#[cfg(unix)]
impl Drop for EchoGuard {
    fn drop(&mut self) {
        let _ = set_terminal_echo(&self.tty, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
