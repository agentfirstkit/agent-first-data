//! The grammar of a value that names *where it is* instead of carrying itself.
//!
//! An argument value on argv is visible to every process on the machine
//! (`ps`), lands in shell history, and reaches any log that echoes the command.
//! For a credential that is a leak; for a large or awkward value it is merely
//! unpleasant. Both are answered the same way — let the argument name a source:
//!
//! ```text
//! VALUE                    the value itself
//! literal:VALUE            …when it starts with one of the prefixes below
//! env:NAME                 an environment variable
//! file:PATH#DOT_PATH       one address inside a supported document
//! file+FORMAT:PATH#DOT     …when the filename cannot say which (`.conf`, no extension)
//! stdin                    the whole of standard input
//! fd:N                     the whole of an inherited file descriptor
//! prompt                   asked for on the controlling terminal, without echo
//! ```
//!
//! This file is the *grammar* only: which sources an argument accepts, and how
//! one value is classified. That is argv's business, so it lives in the core
//! with the rest of what the registry decides — nothing here opens a file, and
//! nothing here knows what a secret is. Reading a classified source, and the
//! policy that separates a printable value from a credential, is
//! [`crate::value_source`](../value_source/index.html)'s.
//!
//! Acceptance is declared per argument, with
//! [`ArgSpec::sources`](super::ArgSpec::sources), because a source turns an
//! argument into a reader of files and environment variables: right for a
//! credential, wrong for most everything else. The declaration also renders the
//! syntax into help and `--docs`, so no host repeats it in an `about` string.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

// ── errors ──────────────────────────────────────────────────────────────────

/// A source that could not be understood, or could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceError {
    code: &'static str,
    message: String,
}

impl SourceError {
    /// Stable machine-readable code: `value_source_invalid` for a source this
    /// argument cannot accept or cannot parse, `value_source_unreadable` for
    /// one that parsed but could not be read.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// A source this argument cannot accept, or cannot parse.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "value_source_invalid",
            message: message.into(),
        }
    }

    /// A source that parsed, but whose value could not be obtained. Raised by
    /// whoever does the reading — for the schemes this crate implements, that
    /// is [`crate::value_source`]; for a host scheme, the host.
    #[must_use]
    pub fn unreadable(message: impl Into<String>) -> Self {
        Self {
            code: "value_source_unreadable",
            message: message.into(),
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SourceError {}

type Result<T> = std::result::Result<T, SourceError>;

// ── schemes and the set an argument accepts ─────────────────────────────────

/// One way of naming where a value is.
///
/// A literal value is always accepted and is therefore not a scheme; what a
/// [`SourceSet`] lists is the indirection an argument allows *besides* typing
/// the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceScheme {
    Env,
    File,
    Stdin,
    Fd,
    Prompt,
}

impl SourceScheme {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::File => "file",
            Self::Stdin => "stdin",
            Self::Fd => "fd",
            Self::Prompt => "prompt",
        }
    }

    /// How the scheme is spelled in help, value name included.
    #[must_use]
    pub const fn syntax(self) -> &'static str {
        match self {
            Self::Env => "env:NAME",
            Self::File => "file[+FORMAT]:PATH#DOT_PATH",
            Self::Stdin => "stdin",
            Self::Fd => "fd:N",
            Self::Prompt => "prompt",
        }
    }
}

/// A scheme a host defines for itself, declared only so help and validation
/// know about it.
///
/// The reading is the host's own — [`SourceSet::parse`] answers
/// [`ValueSource::Host`] and leaves the rest to it. `afhttp`'s
/// `container:NAME`, which reads a token out of a container it manages, is one:
/// nothing about it belongs in this crate, but everything about *documenting*
/// it does.
///
/// [`CliSpec::build`](super::CliSpec::build) requires a lowercase
/// `name`, rejects built-in/reserved names and duplicates, and requires
/// `syntax` to begin with `name:` plus a value placeholder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostScheme {
    pub name: String,
    pub syntax: String,
}

/// The sources one argument accepts.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSet {
    schemes: Vec<SourceScheme>,
    #[serde(
        default,
        rename = "host_schemes",
        skip_serializing_if = "Vec::is_empty"
    )]
    host: Vec<HostScheme>,
}

impl SourceSet {
    /// Exactly the listed schemes, in the order they will be documented.
    pub fn new<I: IntoIterator<Item = SourceScheme>>(schemes: I) -> Self {
        let mut set = Self::default();
        for scheme in schemes {
            if !set.schemes.contains(&scheme) {
                set.schemes.push(scheme);
            }
        }
        set
    }

    /// The two sources a value can come from without a terminal or a pipe: an
    /// environment variable, or an address in a document. This is the set
    /// most arguments want.
    #[must_use]
    pub fn config() -> Self {
        Self::new([SourceScheme::Env, SourceScheme::File])
    }

    /// [`SourceSet::config`] plus the streams — for a value that may be piped
    /// in, handed over on an inherited descriptor, or typed by a person.
    #[must_use]
    pub fn stream() -> Self {
        Self::new([
            SourceScheme::Env,
            SourceScheme::File,
            SourceScheme::Stdin,
            SourceScheme::Fd,
            SourceScheme::Prompt,
        ])
    }

    /// Declare a scheme this crate does not implement. The host parses and
    /// reads it; this only teaches help and validation that it exists.
    #[must_use]
    pub fn host_scheme(mut self, name: impl Into<String>, syntax: impl Into<String>) -> Self {
        self.host.push(HostScheme {
            name: name.into(),
            syntax: syntax.into(),
        });
        self
    }

    #[must_use]
    pub fn schemes(&self) -> &[SourceScheme] {
        &self.schemes
    }

    #[must_use]
    pub fn host_schemes(&self) -> &[HostScheme] {
        &self.host
    }

    #[must_use]
    pub fn accepts(&self, scheme: SourceScheme) -> bool {
        self.schemes.contains(&scheme)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.schemes.is_empty() && self.host.is_empty()
    }

    /// The syntax line help and `--docs` render, so an `about` string never
    /// has to repeat it.
    #[must_use]
    pub fn syntax_summary(&self) -> String {
        let mut parts: Vec<&str> = self.schemes.iter().map(|s| s.syntax()).collect();
        parts.extend(self.host.iter().map(|scheme| scheme.syntax.as_str()));
        format!(
            "the value, or where to read it: {}, literal:VALUE",
            parts.join(", ")
        )
    }

    /// Classify one argument value. Pure: nothing is opened, read, or asked
    /// for, so an argv that names an unacceptable source is rejected as a usage
    /// error before any of it can happen.
    pub fn parse(&self, raw: &str) -> Result<ValueSource> {
        // First, so a literal value that happens to start with a scheme prefix
        // stays expressible.
        if let Some(value) = raw.strip_prefix("literal:") {
            return Ok(ValueSource::Literal(value.to_string()));
        }
        for scheme in &self.host {
            if let Some(rest) = strip_scheme(raw, &scheme.name) {
                if rest.is_empty() {
                    return Err(SourceError::invalid(format!(
                        "`{}` source requires a value: {}",
                        scheme.name, scheme.syntax
                    )));
                }
                return Ok(ValueSource::Host {
                    scheme: scheme.name.clone(),
                    value: rest.to_string(),
                });
            }
        }
        if raw == "stdin" {
            return self
                .require(SourceScheme::Stdin)
                .map(|()| ValueSource::Stdin);
        }
        if raw == "prompt" {
            return self
                .require(SourceScheme::Prompt)
                .map(|()| ValueSource::Prompt);
        }
        if let Some(name) = strip_scheme(raw, "env") {
            self.require(SourceScheme::Env)?;
            if name.is_empty() {
                return Err(SourceError::invalid(
                    "`env` source requires a variable name",
                ));
            }
            return Ok(ValueSource::Env(name.to_string()));
        }
        if let Some(number) = strip_scheme(raw, "fd") {
            self.require(SourceScheme::Fd)?;
            let number: i32 = number.parse().map_err(|_| {
                SourceError::invalid("`fd` source requires a numeric descriptor: fd:N")
            })?;
            // 0, 1, and 2 are this process's own streams; naming one of them is
            // a mistake that would read the wrong thing rather than fail.
            if number < 3 {
                return Err(SourceError::invalid(
                    "`fd` source requires a descriptor >= 3",
                ));
            }
            return Ok(ValueSource::Fd(number));
        }
        if let Some((rest, format)) = strip_file_scheme(raw) {
            self.require(SourceScheme::File)?;
            if format.as_deref().is_some_and(str::is_empty) {
                return Err(SourceError::invalid(
                    "`file` source: `file+` must name a format, as in file+ini:PATH#DOT_PATH",
                ));
            }
            // The last `#` separates the file path from the document address,
            // so a filesystem path may itself contain `#`. A DOT_PATH in this
            // source spelling therefore cannot contain `#`; use another source
            // for that uncommon external key.
            let Some((path, dot_path)) = rest.rsplit_once('#') else {
                return Err(SourceError::invalid(
                    "`file` source must be file:PATH#DOT_PATH",
                ));
            };
            if path.is_empty() || dot_path.is_empty() {
                return Err(SourceError::invalid(
                    "`file` source requires both PATH and DOT_PATH",
                ));
            }
            return Ok(ValueSource::File {
                path: PathBuf::from(path),
                dot_path: dot_path.to_string(),
                format,
            });
        }
        // An unrecognized prefix is not a source; it is a value that contains a
        // colon, which is ordinary in a URL or a `user:pass` pair.
        Ok(ValueSource::Literal(raw.to_string()))
    }

    fn require(&self, scheme: SourceScheme) -> Result<()> {
        if self.accepts(scheme) {
            return Ok(());
        }
        Err(SourceError::invalid(format!(
            "`{}` is not a source this argument accepts; {}",
            scheme.name(),
            self.syntax_summary()
        )))
    }
}

/// `file:rest` or `file+FORMAT:rest`, and the format the caller named.
///
/// The `+FORMAT` sits before the colon so a Windows drive letter in the path
/// cannot be mistaken for it.
fn strip_file_scheme(raw: &str) -> Option<(&str, Option<String>)> {
    let rest = raw.strip_prefix("file")?;
    if let Some(rest) = rest.strip_prefix(':') {
        return Some((rest, None));
    }
    let rest = rest.strip_prefix('+')?;
    let (format, rest) = rest.split_once(':')?;
    Some((rest, Some(format.to_string())))
}

/// `scheme:rest`, or nothing. Split on the first colon only, so a path or a URL
/// after the prefix keeps its own colons.
fn strip_scheme<'a>(raw: &'a str, scheme: &str) -> Option<&'a str> {
    raw.strip_prefix(scheme)?.strip_prefix(':')
}

// ── the parsed source ───────────────────────────────────────────────────────

/// Where one value is, decided at parse time and read on demand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueSource {
    Literal(String),
    Env(String),
    File {
        path: PathBuf,
        dot_path: String,
        /// The format the caller named with `file+FORMAT:`, if any.
        ///
        /// Carried as the caller wrote it: this module decides argv grammar and
        /// deliberately knows nothing about document formats, so the name is
        /// resolved — and rejected if unknown — where the read happens.
        format: Option<String>,
    },
    Stdin,
    Fd(i32),
    Prompt,
    /// A scheme the host declared and reads itself.
    Host {
        scheme: String,
        value: String,
    },
}

impl ValueSource {
    /// How this source is named where a result, a log, or an error may be read
    /// by someone else. Never the value it resolves to.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Literal(_) => "direct".to_string(),
            Self::Env(name) => format!("env:{name}"),
            Self::File {
                path,
                dot_path,
                format,
            } => match format {
                Some(format) => format!("file+{format}:{}#{dot_path}", path.display()),
                None => format!("file:{}#{dot_path}", path.display()),
            },
            Self::Stdin => "stdin".to_string(),
            Self::Fd(number) => format!("fd:{number}"),
            Self::Prompt => "prompt".to_string(),
            Self::Host { scheme, value } => format!("{scheme}:{value}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_value_is_the_value_and_a_prefix_names_a_source() {
        let set = SourceSet::stream();
        assert_eq!(
            set.parse("plain").expect("bare"),
            ValueSource::Literal("plain".to_string())
        );
        assert_eq!(
            set.parse("literal:env:NAME").expect("escape hatch"),
            ValueSource::Literal("env:NAME".to_string())
        );
        assert_eq!(
            set.parse("env:NAME").expect("env"),
            ValueSource::Env("NAME".to_string())
        );
        assert_eq!(set.parse("stdin").expect("stdin"), ValueSource::Stdin);
        assert_eq!(set.parse("fd:3").expect("fd"), ValueSource::Fd(3));
        assert_eq!(set.parse("prompt").expect("prompt"), ValueSource::Prompt);
        assert_eq!(
            set.parse("file:/etc/app.json#a.b").expect("file"),
            ValueSource::File {
                path: PathBuf::from("/etc/app.json"),
                dot_path: "a.b".to_string(),
                format: None,
            }
        );
        // A colon that is not a scheme is just part of the value.
        assert_eq!(
            set.parse("postgres://u:p@h/db").expect("url"),
            ValueSource::Literal("postgres://u:p@h/db".to_string())
        );
    }

    #[test]
    fn a_file_source_may_name_its_format() {
        let set = SourceSet::config();
        assert_eq!(
            set.parse("file+ini:/etc/phoenix.conf#http-password")
                .expect("named format"),
            ValueSource::File {
                path: PathBuf::from("/etc/phoenix.conf"),
                dot_path: "http-password".to_string(),
                format: Some("ini".to_string()),
            }
        );
        // The `+FORMAT` sits before the colon, so a Windows drive letter in the
        // path is never mistaken for one.
        assert_eq!(
            set.parse(r"file:C:\creds\app.json#a.b")
                .expect("drive letter"),
            ValueSource::File {
                path: PathBuf::from(r"C:\creds\app.json"),
                dot_path: "a.b".to_string(),
                format: None,
            }
        );
        assert!(set.parse("file+:/etc/x#a").is_err());
        // The name itself is checked where the read happens, not here: this
        // module is not allowed to know what a document format is.
        assert!(set.parse("file+nonsense:/etc/x#a").is_ok());
    }

    /// A source an argument does not accept is refused by name, and the refusal
    /// says what it would have accepted.
    #[test]
    fn a_scheme_outside_the_set_is_refused() {
        let set = SourceSet::config();
        let error = set.parse("prompt").expect_err("prompt is not in config()");
        assert_eq!(error.code(), "value_source_invalid");
        assert!(error.message().contains("env:NAME"), "{error}");
        assert!(set.parse("stdin").is_err());
        assert!(set.parse("fd:3").is_err());
        assert!(set.parse("env:NAME").is_ok());
    }

    #[test]
    fn a_malformed_source_is_refused_before_anything_is_read() {
        let set = SourceSet::stream();
        for raw in [
            "env:",
            "fd:x",
            "fd:2",
            "file:",
            "file:/etc/app.json",
            "file:#a.b",
            "file:/etc/app.json#",
        ] {
            let error = set.parse(raw).expect_err(raw);
            assert_eq!(error.code(), "value_source_invalid", "{raw}");
        }
    }

    #[test]
    fn a_host_scheme_parses_here_and_is_read_elsewhere() {
        let set = SourceSet::config().host_scheme("container", "container:NAME");
        assert_eq!(
            set.parse("container:afhttp-host").expect("host scheme"),
            ValueSource::Host {
                scheme: "container".to_string(),
                value: "afhttp-host".to_string(),
            }
        );
        assert!(set.parse("container:").is_err());
        // The escape hatch still outranks a host scheme.
        assert_eq!(
            set.parse("literal:container:x").expect("escape hatch"),
            ValueSource::Literal("container:x".to_string())
        );
    }

    #[test]
    fn a_source_describes_itself_without_its_value() {
        assert_eq!(ValueSource::Literal("v".into()).describe(), "direct");
        assert_eq!(ValueSource::Env("NAME".into()).describe(), "env:NAME");
        assert_eq!(ValueSource::Fd(3).describe(), "fd:3");
        assert_eq!(
            ValueSource::File {
                path: PathBuf::from("/etc/app.json"),
                dot_path: "a.b".into(),
                format: None,
            }
            .describe(),
            "file:/etc/app.json#a.b"
        );
    }

    #[test]
    fn the_syntax_summary_is_what_help_renders() {
        let summary = SourceSet::config()
            .host_scheme("container", "container:NAME")
            .syntax_summary();
        assert_eq!(
            summary,
            "the value, or where to read it: env:NAME, file[+FORMAT]:PATH#DOT_PATH, container:NAME, \
             literal:VALUE"
        );
    }
}
