#![allow(clippy::print_stdout, clippy::print_stderr)]

use agent_first_data::document::{
    Document, DocumentError, DocumentFile, Format as DocumentFormat, Value as DocumentValue,
    ValueType, get_path, guard_bare_overwrite, join_path, parse_path, value_from_type,
};
use agent_first_data::{
    ErrorBuilder, Event, OutputFormat, OutputOptions, OutputTo, PlainStyle, Redactor,
    build_cli_error, cli_parse_output, is_valid_bcp47, is_valid_rfc3339, is_valid_rfc3339_date,
    is_valid_rfc3339_time, json_error, json_result, normalize_utc_offset, render,
    validate_protocol_event, validate_protocol_stream,
};
#[cfg(feature = "cli-help")]
use clap::CommandFactory;
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Parser)]
#[command(
    name = "afdata",
    version,
    // Bare `about` (no `= "..."`) is clap derive's own documented way to
    // pull the short about from `CARGO_PKG_DESCRIPTION` — the crate
    // `description`, itself synced from README.md's first paragraph by
    // scripts/meta/sync-spore.py — instead of a second hand-written string
    // that only `--help` would ever see and that would drift from it.
    about,
    //
    // Deliberately never spells the word "skill": that command is
    // feature-gated (`#[cfg(feature = "skill")]`) and a minimal build's
    // help must not mention it at all (verified by `tests/cli_e2e.py`).
    long_about = "Commands are grouped into two families: protocol tools \
        that operate on AFDATA protocol-v1 JSON (lint, validate, render), \
        and document tools that read and edit JSON/TOML/YAML/dotenv/INI \
        documents by dot-path (get, value, paths, keys, set, unset, add, \
        remove). Every command's first positional is its input; `-` reads \
        stdin. Mutation commands (set/unset/add/remove) never read stdin.",
    after_help = concat!("AFDATA: ", env!("CARGO_PKG_VERSION")),
    disable_help_subcommand = true
)]
struct Cli {
    /// Output format: json, yaml, or plain (help also accepts markdown)
    #[arg(long, global = true, default_value = "json")]
    output: String,

    /// Where protocol events go: split (default), stdout, or stderr.
    ///
    /// `split` (default, finite one-shot mode) sends `result` to stdout and
    /// `error`/`progress`/`log` to stderr, so a shell capture or pipe never
    /// mistakes a failure for data. `stdout`/`stderr` (event-stream mode)
    /// collapse every event, including `error`, onto that one stream for a
    /// consumer that reads it in order and branches on `kind`. Orthogonal to
    /// `--output` (which selects format, not destination). A file sink is
    /// `--output-to stdout` plus `--stdout-file <PATH>`.
    #[arg(long = "output-to", global = true, default_value = "split")]
    output_to: String,

    /// Redirect stdout to this file
    #[cfg(feature = "stream-redirect")]
    #[arg(long, value_name = "PATH", global = true)]
    stdout_file: Option<std::path::PathBuf>,

    /// Redirect stderr to this file
    #[cfg(feature = "stream-redirect")]
    #[arg(long, value_name = "PATH", global = true)]
    stderr_file: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Lint a JSON/JSONL stream, a JSON Schema, or a document for deterministic AFDATA issues
    ///
    /// JSON/JSONL input (the default when no document format is detected)
    /// keeps its existing dual-mode behavior: a single JSON value, or one
    /// value per line. `--input-format toml|yaml|yml|dotenv|env|ini` (or a
    /// recognized file extension) lints a document as a single value
    /// instead — the AFDATA naming/suffix rules apply equally there.
    /// `toml-frontmatter`/`yaml-frontmatter` address only the `+++`/`---`
    /// metadata block of a Markdown file, leaving its body untouched (never
    /// auto-detected — the format must be named explicitly).
    #[command(display_order = 1)]
    Lint {
        /// Input file, or `-` for stdin
        input: PathBuf,
        /// Document format override; unset means JSON/JSONL unless the file
        /// extension names a document format
        #[arg(long = "input-format", value_name = "FORMAT")]
        input_format: Option<String>,
    },
    /// Validate one protocol event or a finite protocol event stream (JSON only)
    #[command(display_order = 2)]
    Validate {
        /// Input file, or `-` for stdin
        input: PathBuf,
        /// Enforce the recommended strict protocol profile
        #[arg(long)]
        strict: bool,
        /// Validate each input value as an independent event, without stream lifecycle rules
        #[arg(long = "per-event")]
        per_event: bool,
    },
    /// Render JSON or JSONL through AFDATA output formatting and redaction (JSON only)
    #[command(display_order = 3)]
    Render {
        /// Input file, or `-` for stdin
        input: PathBuf,
        /// Extra field name to redact (beyond the `_secret` suffix convention). Repeatable.
        #[arg(long = "secret-name", value_name = "FIELD")]
        secret_names: Vec<String>,
    },
    /// Validate an Agent Skill, or manage the bundled Agent Skill
    #[cfg(feature = "skill")]
    #[command(display_order = 4, subcommand)]
    Skill(SkillCommand),

    /// Read a document as a whole, or the value at a dot-path
    ///
    /// With no KEY, emits `{"code":"document","format":...,"value":...}` —
    /// the whole document. With KEY, adds `"key"` and narrows `"value"` to
    /// that dot-path. `_secret`-suffixed fields (and any `--secret-name`)
    /// are redacted to `"***"` anywhere in the output, including a
    /// directly-targeted secret leaf — use `value --reveal-secret` to read
    /// a secret's real value.
    #[command(display_order = 10)]
    Get {
        /// Document file, or `-` for stdin
        file: PathBuf,
        /// Dot-separated key path (`\.` escapes a literal dot, `\\` a backslash); omit for the whole document
        key: Option<String>,
        /// Document format override; unset means extension detection
        #[arg(long = "input-format", value_name = "FORMAT")]
        input_format: Option<String>,
        /// Extra field name to redact (beyond the `_secret` suffix convention). Repeatable.
        #[arg(long = "secret-name", value_name = "FIELD")]
        secret_names: Vec<String>,
    },
    /// Read the scalar at a dot-path as raw bytes on stdout — no AFDATA envelope
    ///
    /// Only scalars (string/bool/integer/float/null) are supported; arrays
    /// and objects are rejected, as are non-finite floats. A secret-named
    /// leaf is rejected unless `--reveal-secret` is passed. On failure,
    /// stdout is always empty — the error envelope goes to stderr instead
    /// (so `x=$(afdata value f k)` never captures a JSON error as data).
    #[command(display_order = 11, name = "value")]
    ValueGet {
        /// Document file, or `-` for stdin
        file: PathBuf,
        /// Dot-separated key path
        key: String,
        /// Print a secret-named scalar instead of erroring
        #[arg(long = "reveal-secret")]
        reveal_secret: bool,
        /// Print this instead of erroring when KEY's path does not exist or its value is null
        /// (an empty string is a real value and does not trigger the default)
        #[arg(long, value_name = "VALUE")]
        default: Option<String>,
        /// Document format override; unset means extension detection
        #[arg(long = "input-format", value_name = "FORMAT")]
        input_format: Option<String>,
        /// Extra field name to redact (beyond the `_secret` suffix convention). Repeatable.
        #[arg(long = "secret-name", value_name = "FIELD")]
        secret_names: Vec<String>,
    },
    /// List a container's child dot-paths, one per line — feeds back into afdata
    ///
    /// With no KEY, enumerates the document's top-level children. Each line
    /// is a full dot-path from the root (grammar-escaped), so it can be
    /// piped straight back into `get`/`value`/`unset`/… or extended with
    /// `"$p.field"`. A scalar leaf (nothing to enumerate) is an error, the
    /// dual of `value`. On failure, stdout is always empty (same contract
    /// as `value`). Rejects `--output json` — read a container's structured
    /// JSON via `get` instead.
    #[command(display_order = 12)]
    Paths {
        /// Document file, or `-` for stdin
        file: PathBuf,
        /// Dot-separated key path to the container; omit for the top level
        key: Option<String>,
        /// Document format override; unset means extension detection
        #[arg(long = "input-format", value_name = "FORMAT")]
        input_format: Option<String>,
        /// Empty output + exit 0 when KEY's path does not exist (other errors still fail)
        #[arg(long = "missing-ok")]
        missing_ok: bool,
        /// Separate lines with NUL instead of newline (for `xargs -0`/`read -d ''`)
        #[arg(short = '0', long = "null")]
        null: bool,
    },
    /// List a container's child key names or array indices, one per line — for external tools
    ///
    /// The dual of `paths`: raw, unescaped, unprefixed key names/indices —
    /// exactly what a package manager or another tool expects (`lodash.merge`,
    /// not `dependencies.lodash\.merge`). Never feed this back into afdata's
    /// own dot-path arguments; use `paths` for that. Otherwise identical
    /// contract to `paths` (KEY, `--input-format`, `--missing-ok`, `-0`/`--null`,
    /// scalar-leaf error, empty stdout on failure, rejects `--output json`).
    #[command(display_order = 13)]
    Keys {
        /// Document file, or `-` for stdin
        file: PathBuf,
        /// Dot-separated key path to the container; omit for the top level
        key: Option<String>,
        /// Document format override; unset means extension detection
        #[arg(long = "input-format", value_name = "FORMAT")]
        input_format: Option<String>,
        /// Empty output + exit 0 when KEY's path does not exist (other errors still fail)
        #[arg(long = "missing-ok")]
        missing_ok: bool,
        /// Separate lines with NUL instead of newline (for `xargs -0`/`read -d ''`)
        #[arg(short = '0', long = "null")]
        null: bool,
    },
    /// Set a value at a dot-path, preserving the document's source formatting
    ///
    /// A bare VALUE is always a string — zero coercion, so `007` or a
    /// leading-zero-bearing ID is never silently reinterpreted. Overwriting
    /// an *existing* scalar of a different type with a bare VALUE is an
    /// argument error (pass `--value-type` to keep the type, or
    /// `--value-type string` to convert explicitly); a brand-new key never
    /// needs `--value-type`. `--value-type json` is the only entry point
    /// for arrays, objects, and an exact-type scalar. Idempotency: setting
    /// an already-current value is not special-cased — it just writes the
    /// same value again.
    #[command(display_order = 14)]
    Set {
        /// Document file to mutate in place (never reads stdin; rejects `-`)
        file: PathBuf,
        /// Dot-separated key path
        key: String,
        /// Value to write; interpreted per `--value-type` (default: string, zero coercion)
        value: Option<String>,
        /// Exact type for VALUE: string (default), number, bool, null, or json
        #[arg(
            long = "value-type",
            value_name = "TYPE",
            conflicts_with = "secret_from"
        )]
        value_type: Option<String>,
        /// Read a secret string VALUE from stdin, the controlling terminal, an inherited
        /// file descriptor, or an environment variable: stdin|prompt|fd:<N>|env:<VAR>
        #[arg(
            long = "secret-from",
            value_name = "SRC",
            conflicts_with_all = ["value", "value_type"]
        )]
        secret_from: Option<String>,
        /// Document format override; unset means extension detection
        #[arg(long = "input-format", value_name = "FORMAT")]
        input_format: Option<String>,
    },
    /// Remove one entry from a document entirely
    ///
    /// Idempotency: removing an absent KEY is an error
    /// (`document_path_not_found`), not a no-op — script around it with
    /// `afdata unset ... || true` if absence should be silent.
    #[command(display_order = 15)]
    Unset {
        /// Document file to mutate in place (never reads stdin; rejects `-`)
        file: PathBuf,
        /// Dot-path to the entry to remove
        key: String,
        /// Document format override; unset means extension detection
        #[arg(long = "input-format", value_name = "FORMAT")]
        input_format: Option<String>,
    },
    /// Add an element to a keyed list (an array of objects addressed by a slug field)
    ///
    /// Extra `FIELD=VALUE` pairs are always strings (the same zero-coercion
    /// rule as `set`'s bare VALUE — `add` does not invent its own type
    /// syntax; write an exact type afterwards with `set --value-type`).
    /// Idempotency: adding a SLUG that already exists is an error
    /// (`document_slug_exists`), not a no-op or overwrite.
    #[command(display_order = 16)]
    Add {
        /// Document file to mutate in place (never reads stdin; rejects `-`)
        file: PathBuf,
        /// Dot-path to the keyed list
        key: String,
        /// Slug/ID for the new element
        slug: String,
        /// Field name that identifies each element (the slug field)
        #[arg(long = "slug-field")]
        slug_field: String,
        /// Additional `FIELD=VALUE` pairs to set on the new element (always strings)
        #[arg(value_name = "FIELD=VALUE")]
        fields: Vec<String>,
        /// Document format override; unset means extension detection
        #[arg(long = "input-format", value_name = "FORMAT")]
        input_format: Option<String>,
    },
    /// Remove an element from a keyed list by slug
    ///
    /// Idempotency: removing a SLUG that does not exist is an error
    /// (`document_slug_not_found`), not a no-op.
    #[command(display_order = 17)]
    Remove {
        /// Document file to mutate in place (never reads stdin; rejects `-`)
        file: PathBuf,
        /// Dot-path to the keyed list
        key: String,
        /// Slug/ID of the element to remove
        slug: String,
        /// Field name that identifies each element (the slug field)
        #[arg(long = "slug-field")]
        slug_field: String,
        /// Document format override; unset means extension detection
        #[arg(long = "input-format", value_name = "FORMAT")]
        input_format: Option<String>,
    },
}

#[cfg(feature = "skill")]
#[derive(Subcommand)]
#[command(disable_help_subcommand = true)]
enum SkillCommand {
    /// Validate a SKILL.md file or skill directory against the Agent Skills spec
    Validate {
        /// SKILL.md file or skill directory, or `-` for SKILL.md text on stdin
        input: PathBuf,
    },
    /// Report whether the bundled Agent Skill is installed for each target agent
    #[cfg(feature = "skill-admin")]
    Status {
        /// Agent target: all, codex, claude-code, opencode, or hermes
        #[arg(long, default_value = "all")]
        agent: String,
        /// Skill scope: personal or workspace
        #[arg(long, default_value = "personal")]
        scope: String,
        /// Explicit skills directory; requires a single concrete --agent
        #[arg(long)]
        skills_dir: Option<String>,
    },
    /// Install the bundled Agent Skill for each target agent
    #[cfg(feature = "skill-admin")]
    Install {
        /// Agent target: all, codex, claude-code, opencode, or hermes
        #[arg(long, default_value = "all")]
        agent: String,
        /// Skill scope: personal or workspace
        #[arg(long, default_value = "personal")]
        scope: String,
        /// Explicit skills directory; requires a single concrete --agent
        #[arg(long)]
        skills_dir: Option<String>,
        /// Overwrite a skill this tool did not manage
        #[arg(long)]
        force: bool,
    },
    /// Uninstall the bundled Agent Skill for each target agent
    #[cfg(feature = "skill-admin")]
    Uninstall {
        /// Agent target: all, codex, claude-code, opencode, or hermes
        #[arg(long, default_value = "all")]
        agent: String,
        /// Skill scope: personal or workspace
        #[arg(long, default_value = "personal")]
        scope: String,
        /// Explicit skills directory; requires a single concrete --agent
        #[arg(long)]
        skills_dir: Option<String>,
        /// Remove a skill this tool did not manage
        #[arg(long)]
        force: bool,
    },
}

#[derive(Clone, Debug)]
struct Finding {
    rule_id: &'static str,
    severity: &'static str,
    pointer: String,
    message: String,
}

impl Finding {
    fn error(rule_id: &'static str, pointer: String, message: String) -> Self {
        Self {
            rule_id,
            severity: "error",
            pointer,
            message,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "rule_id": self.rule_id,
            "severity": self.severity,
            "pointer": self.pointer,
            "message": self.message,
        })
    }
}

enum ParsedInput {
    Single(Value),
    Lines(Vec<Value>),
}

struct ParseError {
    code: &'static str,
    message: String,
    hint: Option<String>,
    line: Option<usize>,
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().collect();

    // Redirect stdout/stderr before any output, per --stdout-file/--stderr-file.
    #[cfg(feature = "stream-redirect")]
    let _stream_redirect =
        match agent_first_data::stream_redirect::install_from_raw_args(raw.clone()) {
            Ok(installed) => installed,
            Err(err) => {
                let event = build_cli_error(&err.to_string(), None);
                return emit_event(event, OutputFormat::Json, 2);
            }
        };

    // Handle --version through AFDATA so `--version --output json` works too.
    match agent_first_data::cli_handle_version_or_continue(
        &raw,
        "afdata",
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(Some(version)) => return write_text_exit(&version, 0),
        Ok(None) => {}
        Err(err) => return emit_event(err, OutputFormat::Json, 2),
    }

    // Handle --help before clap so `--help --output markdown` works.
    #[cfg(feature = "cli-help")]
    match agent_first_data::cli_handle_help_or_continue(
        &raw,
        &Cli::command(),
        &agent_first_data::HelpConfig::human_cli_default(),
    ) {
        Ok(Some(help)) => return write_text_exit(&help, 0),
        Ok(None) => {}
        Err(err) => return emit_event(err, OutputFormat::Json, 2),
    }

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                return write_text_exit(&err.render().to_string(), 0);
            }
            let event = build_cli_error(&err.to_string(), Some("try: afdata --help"));
            return emit_event(event, OutputFormat::Json, 2);
        }
    };

    // Redirection is installed from raw args above; these fields exist for
    // clap's --help listing only.
    #[cfg(feature = "stream-redirect")]
    let _ = (&cli.stdout_file, &cli.stderr_file);

    let format = match cli_parse_output(&cli.output) {
        Ok(format) => format,
        Err(message) => {
            let event = build_cli_error(&message, Some("valid values: json, yaml, plain"));
            return emit_event(event, OutputFormat::Json, 2);
        }
    };

    // §1: `paths`/`keys` are inherently raw line output (D6 "not applicable
    // is a parameter error") — reject only an *explicit* `--output json`
    // (the default parse of an omitted `--output` is indistinguishable from
    // it at the `OutputFormat` level, so this scans the raw argv instead of
    // `cli.output`). Structured enumeration already exists via `get`.
    if matches!(cli.command, Command::Paths { .. } | Command::Keys { .. })
        && explicit_output_json(&raw)
    {
        let event = build_cli_error(
            "--output json is not supported by paths/keys; they always print raw lines",
            Some("read a container as structured JSON with `get` instead"),
        );
        return emit_event(event, OutputFormat::Json, 2);
    }

    let output_to = match OutputTo::parse(&cli.output_to) {
        Ok(selector) => selector,
        Err(message) => {
            let event = build_cli_error(&message, Some("valid values: split, stdout, stderr"));
            return emit_event(event, OutputFormat::Json, 2);
        }
    };
    // Raw-scalar reader commands (value/paths/keys) are intrinsically split:
    // raw data on stdout, error envelope on stderr. Collapsing their output
    // onto one stream is meaningless (their success is not an envelope), so a
    // non-default --output-to is a usage error that names the envelope path.
    if output_to != OutputTo::Split
        && matches!(
            cli.command,
            Command::ValueGet { .. } | Command::Paths { .. } | Command::Keys { .. }
        )
    {
        let event = build_cli_error(
            "--output-to stdout/stderr is not supported by value/paths/keys; they print a raw scalar, not an event stream",
            Some("read an AFDATA envelope with `get` instead"),
        );
        return emit_event(event, OutputFormat::Json, 2);
    }
    let _ = OUTPUT_TO.set(output_to);

    match cli.command {
        Command::Lint {
            input,
            input_format,
        } => run_lint(&input, input_format.as_deref(), format),
        Command::Validate {
            input,
            strict,
            per_event,
        } => run_validate(&input, format, strict, per_event),
        Command::Render {
            input,
            secret_names,
        } => run_render(&input, &secret_names, format),
        #[cfg(feature = "skill")]
        Command::Skill(action) => run_skill(action, format),
        Command::Get {
            file,
            key,
            input_format,
            secret_names,
        } => run_get(
            &file,
            key.as_deref(),
            &DocumentContext {
                input_format: input_format.as_deref(),
                secret_names: &secret_names,
                format,
            },
        ),
        Command::ValueGet {
            file,
            key,
            reveal_secret,
            default,
            input_format,
            secret_names,
        } => run_value_get(
            &file,
            &key,
            reveal_secret,
            default.as_deref(),
            &DocumentContext {
                input_format: input_format.as_deref(),
                secret_names: &secret_names,
                format,
            },
        ),
        Command::Paths {
            file,
            key,
            input_format,
            missing_ok,
            null,
        } => run_enumerate(
            &file,
            key.as_deref(),
            input_format.as_deref(),
            missing_ok,
            null,
            format,
            EnumerateMode::Paths,
        ),
        Command::Keys {
            file,
            key,
            input_format,
            missing_ok,
            null,
        } => run_enumerate(
            &file,
            key.as_deref(),
            input_format.as_deref(),
            missing_ok,
            null,
            format,
            EnumerateMode::Keys,
        ),
        Command::Set {
            file,
            key,
            value,
            value_type,
            secret_from,
            input_format,
        } => run_set(
            &file,
            &key,
            value,
            value_type.as_deref(),
            secret_from.as_deref(),
            &DocumentContext {
                input_format: input_format.as_deref(),
                secret_names: &[],
                format,
            },
        ),
        Command::Add {
            file,
            key,
            slug,
            slug_field,
            fields,
            input_format,
        } => run_add(
            &file,
            &key,
            &slug,
            &slug_field,
            &fields,
            &DocumentContext {
                input_format: input_format.as_deref(),
                secret_names: &[],
                format,
            },
        ),
        Command::Remove {
            file,
            key,
            slug,
            slug_field,
            input_format,
        } => run_remove(
            &file,
            &key,
            &slug,
            &slug_field,
            &DocumentContext {
                input_format: input_format.as_deref(),
                secret_names: &[],
                format,
            },
        ),
        Command::Unset {
            file,
            key,
            input_format,
        } => run_unset(
            &file,
            &key,
            &DocumentContext {
                input_format: input_format.as_deref(),
                secret_names: &[],
                format,
            },
        ),
    }
}

/// Whether `raw` (the full argv) contains an explicit `--output json` or
/// `--output=json`. `--output` is a global flag that clap accepts anywhere
/// in argv (before or after the subcommand), so this scans the whole
/// vector rather than stopping at the first positional — unlike the
/// `--help`/`--version` pre-scanners in the shared crate, which
/// deliberately stop there.
fn explicit_output_json(raw: &[String]) -> bool {
    let mut i = 1; // skip argv[0]
    while i < raw.len() {
        let arg = raw[i].as_str();
        if arg == "--" {
            break;
        }
        if let Some(value) = arg.strip_prefix("--output=") {
            if value == "json" {
                return true;
            }
        } else if arg == "--output" && raw.get(i + 1).map(String::as_str) == Some("json") {
            return true;
        }
        i += 1;
    }
    false
}

// ═══════════════════════════════════════════
// Protocol tools: lint, validate, render, skill
// ═══════════════════════════════════════════

fn run_lint(input: &Path, input_format: Option<&str>, format: OutputFormat) -> ExitCode {
    let resolved = match resolve_input_format(input_format) {
        Ok(resolved) => resolved,
        Err(message) => return emit_usage_error(&message, format),
    };
    // R6: lint reuses the document input layer (extension inference +
    // `--input-format` override) so TOML/YAML/dotenv/INI documents get the
    // same AFDATA naming/suffix checks as JSON. Unlike other document
    // commands, an undetected extension (or `-` with no override) falls
    // back to JSON/JSONL rather than erroring — that dual-mode behavior
    // predates this command's document-format support and stays unchanged.
    let effective = resolved
        .or_else(|| {
            if input == Path::new("-") {
                None
            } else {
                DocumentFormat::detect(input)
            }
        })
        .unwrap_or(DocumentFormat::Json);

    let mut findings = Vec::new();
    if effective == DocumentFormat::Json {
        let text = match read_input_or_stdin(input) {
            Ok(text) => text,
            Err(message) => {
                let event = build_error_event(json_error("read_failed", &message));
                return emit_event(event, format, 1);
            }
        };
        let parsed = match parse_json_or_jsonl(&text) {
            Ok(parsed) => parsed,
            Err(err) => return emit_parse_error(err, format),
        };
        match parsed {
            ParsedInput::Single(value) => lint_value(&value, "", &mut findings),
            ParsedInput::Lines(values) => {
                for (idx, value) in values.iter().enumerate() {
                    lint_value(value, &format!("/{}", idx + 1), &mut findings);
                }
            }
        }
    } else {
        let (value, _doc_format) = match read_document_input(input, Some(effective)) {
            Ok(pair) => pair,
            Err(err) => {
                let event = build_error_event(json_error(err.code(), &err.to_string()));
                return emit_event(event, format, 1);
            }
        };
        let json_value: Value = value.into();
        lint_value(&json_value, "", &mut findings);
    }
    emit_findings("lint_failed", "lint failed", findings, format)
}

fn run_validate(input: &Path, format: OutputFormat, strict: bool, per_event: bool) -> ExitCode {
    let text = match read_input_or_stdin(input) {
        Ok(text) => text,
        Err(message) => {
            let event = build_error_event(json_error("read_failed", &message));
            return emit_event(event, format, 1);
        }
    };
    let parsed = match parse_json_or_jsonl(&text) {
        Ok(parsed) => parsed,
        Err(err) => return emit_parse_error(err, format),
    };
    let mut findings = Vec::new();
    if per_event {
        match parsed {
            ParsedInput::Single(Value::Array(events)) | ParsedInput::Lines(events) => {
                for (idx, event) in events.iter().enumerate() {
                    validate_one_event(event, strict, &format!("/{idx}"), &mut findings);
                }
            }
            ParsedInput::Single(value) => validate_one_event(&value, strict, "", &mut findings),
        }
        return emit_findings("validation_failed", "validation failed", findings, format);
    }
    match parsed {
        ParsedInput::Single(Value::Array(events)) => {
            if let Err(vs) = validate_protocol_stream(&events, strict) {
                for v in vs {
                    findings.push(Finding::error(v.rule, v.pointer, v.message));
                }
            }
        }
        ParsedInput::Single(value) => validate_single_input(value, strict, &mut findings),
        ParsedInput::Lines(values) => {
            if let Err(vs) = validate_protocol_stream(&values, strict) {
                for v in vs {
                    findings.push(Finding::error(v.rule, v.pointer, v.message));
                }
            }
        }
    }
    emit_findings("validation_failed", "validation failed", findings, format)
}

fn validate_single_input(value: Value, strict: bool, findings: &mut Vec<Finding>) {
    let kind = value.get("kind").and_then(Value::as_str);
    if matches!(kind, Some("log" | "progress")) {
        if let Err(vs) = validate_protocol_stream(&[value], strict) {
            for v in vs {
                findings.push(Finding::error(v.rule, v.pointer, v.message));
            }
        }
        return;
    }
    validate_one_event(&value, strict, "", findings);
}

fn validate_one_event(value: &Value, strict: bool, pointer: &str, findings: &mut Vec<Finding>) {
    if let Err(v) = validate_protocol_event(value, strict) {
        findings.push(Finding::error(
            v.rule,
            format!("{pointer}{}", v.pointer),
            v.message,
        ));
    }
}

fn run_render(input: &Path, secret_names: &[String], format: OutputFormat) -> ExitCode {
    let text = match read_input_or_stdin(input) {
        Ok(text) => text,
        Err(message) => {
            let event = build_error_event(json_error("read_failed", &message));
            return emit_event(event, format, 1);
        }
    };
    let parsed = match parse_json_or_jsonl(&text) {
        Ok(parsed) => parsed,
        // R5: parse-error presentation honors the negotiated `--output`
        // (previously hardcoded to JSON, inconsistent with lint/validate).
        Err(err) => return emit_parse_error(err, format),
    };
    let output_options = OutputOptions {
        redaction: Redactor::new().secret_names(secret_names.iter().cloned()),
        style: PlainStyle::default(),
    };
    match parsed {
        ParsedInput::Single(value) => {
            write_text_exit(&format_value(&value, format, false, &output_options), 0)
        }
        ParsedInput::Lines(values) => {
            let mut out = String::new();
            for (idx, value) in values.iter().enumerate() {
                if idx > 0 && is_yaml_format(format) {
                    out.push_str("---\n");
                }
                out.push_str(&format_value(value, format, idx > 0, &output_options));
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            write_text_exit(&out, 0)
        }
    }
}

#[cfg(feature = "skill")]
fn run_skill(action: SkillCommand, format: OutputFormat) -> ExitCode {
    match action {
        SkillCommand::Validate { input } => run_skill_validate(&input, format),
        #[cfg(feature = "skill-admin")]
        SkillCommand::Status {
            agent,
            scope,
            skills_dir,
        } => run_skill_admin_action(
            agent_first_data::skill::SkillAction::Status,
            &agent,
            &scope,
            skills_dir,
            false,
            format,
        ),
        #[cfg(feature = "skill-admin")]
        SkillCommand::Install {
            agent,
            scope,
            skills_dir,
            force,
        } => run_skill_admin_action(
            agent_first_data::skill::SkillAction::Install,
            &agent,
            &scope,
            skills_dir,
            force,
            format,
        ),
        #[cfg(feature = "skill-admin")]
        SkillCommand::Uninstall {
            agent,
            scope,
            skills_dir,
            force,
        } => run_skill_admin_action(
            agent_first_data::skill::SkillAction::Uninstall,
            &agent,
            &scope,
            skills_dir,
            force,
            format,
        ),
    }
}

#[cfg(feature = "skill")]
fn run_skill_validate(input: &Path, format: OutputFormat) -> ExitCode {
    let (text, expected_name, display_path) = match read_skill_input(input) {
        Ok(value) => value,
        Err(message) => {
            let event = build_error_event(json_error("read_failed", &message));
            return emit_event(event, format, 1);
        }
    };
    let validation = match expected_name.as_deref() {
        Some(name) => agent_first_data::skill::validate_skill_named(&text, name),
        None => agent_first_data::skill::validate_skill(&text),
    };
    let metadata = match validation {
        Ok(metadata) => metadata,
        Err(error) => {
            let event = build_error_event(
                json_error("skill_invalid", error.message())
                    .hint("make SKILL.md front matter conform to the Agent Skills specification"),
            );
            return emit_event(event, format, 1);
        }
    };
    let event = json_result(json!({
        "code": "skill_valid",
        "path": display_path,
        "name": metadata.name,
        "description": metadata.description,
        "license": metadata.license,
        "compatibility": metadata.compatibility,
        "metadata": metadata.metadata,
        "allowed_tools": metadata.allowed_tools,
        "disable_model_invocation": metadata.disable_model_invocation,
        "user_invocable": metadata.user_invocable,
    }))
    .build();
    emit_event(event, format, 0)
}

#[cfg(feature = "skill")]
fn read_skill_input(input: &Path) -> Result<(String, Option<String>, String), String> {
    if input == Path::new("-") {
        return read_input_or_stdin(Path::new("-")).map(|text| (text, None, "<stdin>".to_string()));
    }
    let input_metadata = std::fs::symlink_metadata(input)
        .map_err(|error| format!("failed to inspect {}: {error}", input.display()))?;
    if input_metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing to validate symlinked skill input at {}",
            input.display()
        ));
    }

    let (skill_path, expected_name) = if input_metadata.is_dir() {
        let name = path_file_name(input)?;
        (input.join("SKILL.md"), Some(name))
    } else if input_metadata.is_file() {
        let expected_name = if input.file_name().and_then(|name| name.to_str()) == Some("SKILL.md")
        {
            input.parent().map(path_file_name).transpose()?
        } else {
            None
        };
        (input.to_path_buf(), expected_name)
    } else {
        return Err(format!(
            "skill input is not a regular file or directory: {}",
            input.display()
        ));
    };

    let skill_metadata = std::fs::symlink_metadata(&skill_path)
        .map_err(|error| format!("failed to inspect {}: {error}", skill_path.display()))?;
    if skill_metadata.file_type().is_symlink() || !skill_metadata.is_file() {
        return Err(format!(
            "skill document is not a regular file: {}",
            skill_path.display()
        ));
    }
    let text = std::fs::read_to_string(&skill_path)
        .map_err(|error| format!("failed to read {}: {error}", skill_path.display()))?;
    Ok((text, expected_name, skill_path.display().to_string()))
}

#[cfg(feature = "skill")]
fn path_file_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("path has no UTF-8 directory name: {}", path.display()))
}

#[cfg(feature = "skill-admin")]
fn run_skill_admin_action(
    action: agent_first_data::skill::SkillAction,
    agent: &str,
    scope: &str,
    skills_dir: Option<String>,
    force: bool,
    format: OutputFormat,
) -> ExitCode {
    use agent_first_data::skill::{SkillOptions, SkillSpec, run_skill_admin};

    let agent = match parse_skill_agent(agent) {
        Ok(agent) => agent,
        Err(message) => {
            let event = build_cli_error(
                &message,
                Some("valid agents: all, codex, claude-code, opencode, hermes"),
            );
            return emit_event(event, format, 2);
        }
    };
    let scope = match parse_skill_scope(scope) {
        Ok(scope) => scope,
        Err(message) => {
            let event = build_cli_error(&message, Some("valid scopes: personal, workspace"));
            return emit_event(event, format, 2);
        }
    };

    const SKILL_SOURCE: &str = include_str!("../../skills/agent-first-data/SKILL.md");
    let spec = SkillSpec {
        name: "agent-first-data",
        source: SKILL_SOURCE,
        title: "Agent-First Data",
        marker_slug: "afdata",
    };
    let options = SkillOptions {
        agent,
        scope,
        skills_dir,
        force,
    };
    match run_skill_admin(&spec, action, &options) {
        Ok(report) => match serde_json::to_value(report) {
            Ok(value) => {
                let event = json_result(value).build();
                emit_event(event, format, 0)
            }
            Err(err) => {
                let event = build_error_event(json_error(
                    "serialization_failed",
                    &format!("failed to serialize skill report: {err}"),
                ));
                emit_event(event, format, 1)
            }
        },
        Err(err) => {
            let mut builder = json_error("cli_error", &err.message);
            if let Some(hint) = err.hint.as_deref() {
                builder = builder.hint(hint);
            }
            if let Some(report) = err.partial_report.as_ref()
                && let Ok(partial_report) = serde_json::to_value(report)
            {
                builder = builder.field("partial_report", partial_report);
            }
            let event = build_error_event(builder);
            emit_event(event, format, 2)
        }
    }
}

#[cfg(feature = "skill-admin")]
fn parse_skill_agent(value: &str) -> Result<agent_first_data::skill::SkillAgentSelection, String> {
    match value {
        "all" => Ok(agent_first_data::skill::SkillAgentSelection::All),
        "codex" => Ok(agent_first_data::skill::SkillAgentSelection::Codex),
        "claude-code" => Ok(agent_first_data::skill::SkillAgentSelection::ClaudeCode),
        "opencode" => Ok(agent_first_data::skill::SkillAgentSelection::Opencode),
        "hermes" => Ok(agent_first_data::skill::SkillAgentSelection::Hermes),
        other => Err(format!("invalid --agent '{other}'")),
    }
}

#[cfg(feature = "skill-admin")]
fn parse_skill_scope(value: &str) -> Result<agent_first_data::skill::SkillScope, String> {
    match value {
        "personal" => Ok(agent_first_data::skill::SkillScope::Personal),
        "workspace" => Ok(agent_first_data::skill::SkillScope::Workspace),
        other => Err(format!("invalid --scope '{other}'")),
    }
}

fn format_value(
    value: &Value,
    format: OutputFormat,
    suppress_yaml_boundary: bool,
    output_options: &OutputOptions,
) -> String {
    let mut out = render(value, format, output_options);
    if suppress_yaml_boundary
        && is_yaml_format(format)
        && let Some(stripped) = out.strip_prefix("---\n")
    {
        out = stripped.to_string();
    }
    out
}

/// Whether `format` is [`OutputFormat::Yaml`].
fn is_yaml_format(format: OutputFormat) -> bool {
    format == OutputFormat::Yaml
}

/// Read `input` fully: `-` reads stdin to EOF, otherwise reads the file at
/// that path. D1: no implicit stdin fallback and no TTY detection — every
/// caller passes an explicit path or `-`, so there is no "maybe hangs"
/// shape left to guard against; reading `-` on an interactive terminal is
/// the user's explicit request, same as any other Unix tool.
fn read_input_or_stdin(input: &Path) -> Result<String, String> {
    if input == Path::new("-") {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|err| format!("failed to read stdin: {err}"))?;
        return Ok(text);
    }
    std::fs::read_to_string(input)
        .map_err(|err| format!("failed to read {}: {err}", input.display()))
}

fn parse_json_or_jsonl(text: &str) -> Result<ParsedInput, ParseError> {
    if text.trim().is_empty() {
        return Err(ParseError {
            code: "json_parse_failed",
            message: "input is empty".to_string(),
            hint: Some("provide a JSON value or JSONL stream".to_string()),
            line: None,
        });
    }
    match serde_json::from_str::<Value>(text) {
        Ok(value) => Ok(ParsedInput::Single(value)),
        Err(whole_error) => {
            let mut values = Vec::new();
            for (idx, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(line) {
                    Ok(value) => values.push(value),
                    Err(line_error) => {
                        return Err(ParseError {
                            code: "jsonl_parse_failed",
                            message: format!("line {} is not valid JSON: {line_error}", idx + 1),
                            hint: Some(format!("complete JSON parse failed first: {whole_error}")),
                            line: Some(idx + 1),
                        });
                    }
                }
            }
            if values.is_empty() {
                Err(ParseError {
                    code: "json_parse_failed",
                    message: whole_error.to_string(),
                    hint: Some("provide a JSON value or JSONL stream".to_string()),
                    line: None,
                })
            } else {
                Ok(ParsedInput::Lines(values))
            }
        }
    }
}

/// Build an error event without an `expect`. The error builders here always use
/// non-empty literal codes/messages and non-reserved fields, so `build()` cannot
/// actually fail; on the impossible error we fall back to `build_cli_error` so the
/// function stays total and panic-free.
fn build_error_event(builder: ErrorBuilder) -> Event {
    match builder.build() {
        Ok(event) => event,
        Err(err) => build_cli_error(&err.to_string(), None),
    }
}

fn emit_parse_error(err: ParseError, format: OutputFormat) -> ExitCode {
    let mut fields = serde_json::Map::new();
    if let Some(line) = err.line {
        fields.insert("line".to_string(), json!(line));
    }
    let mut builder = json_error(err.code, &err.message);
    if let Some(hint) = err.hint.as_deref() {
        builder = builder.hint(hint);
    }
    builder = builder.fields(Value::Object(fields));
    let event = build_error_event(builder);
    emit_event(event, format, 1)
}

fn emit_findings(
    error_code: &'static str,
    error_message: &'static str,
    findings: Vec<Finding>,
    format: OutputFormat,
) -> ExitCode {
    let findings_json = Value::Array(findings.iter().map(Finding::to_json).collect());
    if findings.is_empty() {
        let event = json_result(json!({"ok": true, "findings": findings_json})).build();
        emit_event(event, format, 0)
    } else {
        let event = build_error_event(
            json_error(error_code, error_message).fields(json!({"findings": findings_json})),
        );
        emit_event(event, format, 1)
    }
}

/// A usage-class error (R2): the CLI invocation's own shape was wrong
/// (an invalid `--input-format`/`--value-type` name, a malformed
/// `FIELD=VALUE`, a mutation command given `-`, …) — distinct from a
/// runtime document error, and always exit 2. Routed by [`emit_event`] as a
/// `kind:"error"` envelope, so it lands on stderr under the default split (and
/// on the chosen stream under `--output-to stdout|stderr`).
fn emit_usage_error(message: &str, format: OutputFormat) -> ExitCode {
    let event = build_error_event(json_error("document_usage_error", message));
    emit_event(event, format, 2)
}

/// The resolved `--output-to` selector. Read through [`output_to`], which
/// falls back to `Split` before the flag is parsed so any pre-dispatch
/// usage/parse error still routes its `error` envelope to stderr by default.
static OUTPUT_TO: std::sync::OnceLock<OutputTo> = std::sync::OnceLock::new();

fn output_to() -> OutputTo {
    OUTPUT_TO.get().copied().unwrap_or(OutputTo::Split)
}

/// Stream a `kind:"result"` payload lands on under `selector`.
fn result_stream(selector: OutputTo) -> Stream {
    match selector {
        OutputTo::Split | OutputTo::Stdout => Stream::Stdout,
        OutputTo::Stderr => Stream::Stderr,
    }
}

/// Stream a `kind:"error"` (or `progress`/`log` diagnostic) lands on under
/// `selector`.
fn error_stream(selector: OutputTo) -> Stream {
    match selector {
        OutputTo::Split | OutputTo::Stderr => Stream::Stderr,
        OutputTo::Stdout => Stream::Stdout,
    }
}

/// Route an already-built event by its `kind`: `result` follows
/// [`result_stream`], every other kind follows [`error_stream`].
fn stream_for(event: &Value, selector: OutputTo) -> Stream {
    if event.get("kind").and_then(Value::as_str) == Some("result") {
        result_stream(selector)
    } else {
        error_stream(selector)
    }
}

fn emit_event(event: impl Into<Value>, format: OutputFormat, code: u8) -> ExitCode {
    let event: Value = event.into();
    let stream = stream_for(&event, output_to());
    emit_event_to(event, format, &OutputOptions::default(), code, stream)
}

enum Stream {
    Stdout,
    Stderr,
}

/// As [`emit_event`], but rendering through `output_options` (redaction and
/// style) and writing to an explicit `stream`. Used by the document commands
/// so `--secret-name` reaches the final render and so callers that already
/// know the routing (via [`result_stream`]/[`error_stream`] under the resolved
/// `--output-to`) can name the stream directly.
fn emit_event_to(
    event: impl Into<Value>,
    format: OutputFormat,
    output_options: &OutputOptions,
    code: u8,
    stream: Stream,
) -> ExitCode {
    let mut event: Value = event.into();
    if event.get("trace").is_none()
        && let Some(object) = event.as_object_mut()
    {
        object.insert("trace".to_string(), json!({}));
    }
    if let Err(violation) = validate_protocol_event(&event, true) {
        let fallback = build_error_event(
            json_error(
                "internal_protocol_error",
                "afdata attempted to emit an invalid protocol event",
            )
            .field("validation_message", json!(violation.to_string())),
        );
        let mut text = render(
            fallback.as_value(),
            OutputFormat::Json,
            &OutputOptions::default(),
        );
        if !text.ends_with('\n') {
            text.push('\n');
        }
        return write_text_exit_to(&text, 1, stream);
    }
    let mut text = render(&event, format, output_options);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    write_text_exit_to(&text, code, stream)
}

fn write_text_exit(text: &str, code: u8) -> ExitCode {
    write_text_exit_to(text, code, Stream::Stdout)
}

// The sanctioned emitter sink. Per the spec's Channel policy, a finite
// command splits by kind (`result` → stdout, `error`/diagnostics → stderr) and
// `--output-to stdout|stderr` collapses everything onto one stream; both routes
// resolve to a `Stream` here. This is the one place allowed to touch
// `std::io::stderr` directly (clippy.toml's `disallowed-methods` guards against
// stray stderr writes elsewhere that would bypass this routing).
#[allow(clippy::disallowed_methods)]
fn write_text_exit_to(text: &str, code: u8, stream: Stream) -> ExitCode {
    let result = match stream {
        Stream::Stdout => std::io::stdout().lock().write_all(text.as_bytes()),
        Stream::Stderr => std::io::stderr().lock().write_all(text.as_bytes()),
    };
    if let Err(err) = result {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            return ExitCode::from(0);
        }
        return ExitCode::from(1);
    }
    ExitCode::from(code)
}

// ═══════════════════════════════════════════
// Document read/edit commands
// ═══════════════════════════════════════════

/// Shared per-invocation context threaded through the document read/edit
/// commands: the `--input-format` override, the `--secret-name` redaction
/// list, and the negotiated `--output` rendering format.
struct DocumentContext<'a> {
    input_format: Option<&'a str>,
    secret_names: &'a [String],
    format: OutputFormat,
}

/// A document command failure, classified per R2: `Usage` is a CLI-shape
/// mistake (bad `--input-format`/`--value-type` name, malformed
/// `FIELD=VALUE`, a mutation command given `-`, VALUE absent with no
/// `--value-type`/`--secret-from`, …) and always exit 2; `Document` wraps a
/// library [`DocumentError`], mapped to a stable code by
/// [`DocumentError::code`] and always exit 1; `Runtime` covers the handful
/// of CLI-originated *runtime* facts about the document/secret source that
/// are not `DocumentError` variants (a secret leaf without
/// `--reveal-secret`, a non-scalar `value` target, a scalar `paths`/`keys`
/// target, a failed `--secret-from` read) — also exit 1, with an explicit
/// stable code.
enum CliDocError {
    Usage(String),
    Document(DocumentError),
    Runtime { code: &'static str, message: String },
}

impl CliDocError {
    fn runtime(code: &'static str, message: impl Into<String>) -> Self {
        Self::Runtime {
            code,
            message: message.into(),
        }
    }
}

impl From<DocumentError> for CliDocError {
    fn from(err: DocumentError) -> Self {
        Self::Document(err)
    }
}

/// Whether `err` is in the `document_path_not_found` bucket — the "the
/// address doesn't exist" family that `value --default` and
/// `paths`/`keys --missing-ok` are allowed to swallow. Reuses
/// [`DocumentError::code`] directly so the exemption is always exactly the
/// bucket agents see in `error.code`, never a separately-maintained variant list.
fn is_path_not_found(err: &CliDocError) -> bool {
    matches!(err, CliDocError::Document(doc_err) if doc_err.code() == "document_path_not_found")
}

/// Render `err` into `(event, exit_code)`.
fn document_error_event(err: &CliDocError) -> (Event, u8) {
    match err {
        CliDocError::Usage(message) => (
            build_error_event(json_error("document_usage_error", message)),
            2,
        ),
        CliDocError::Document(doc_err) => (
            build_error_event(json_error(doc_err.code(), &doc_err.to_string())),
            1,
        ),
        CliDocError::Runtime { code, message } => (build_error_event(json_error(code, message)), 1),
    }
}

/// Finish a `get`/`set`/`unset`/`add`/`remove` invocation. Under the default
/// `--output-to split` the success `result` envelope goes to stdout and the
/// `error` envelope to stderr (so `x=$(afdata get f k)` never captures an error
/// as data and `afdata set f k v >/dev/null` never swallows the diagnostic);
/// `--output-to stdout|stderr` collapses both onto the one chosen stream.
fn finish_document(result: Result<Value, CliDocError>, ctx: &DocumentContext<'_>) -> ExitCode {
    let selector = output_to();
    match result {
        Ok(payload) => emit_document_event(
            json_result(payload).build(),
            ctx,
            0,
            result_stream(selector),
        ),
        Err(err) => {
            let (event, code) = document_error_event(&err);
            emit_document_event(event, ctx, code, error_stream(selector))
        }
    }
}

/// Finish a `value`/`paths`/`keys` invocation: success writes raw bytes
/// straight to stdout with no envelope; failure writes the envelope to stderr,
/// leaving stdout empty — so `x=$(afdata value f k)` never captures a JSON
/// error as data. These commands are always split (they reject a non-default
/// `--output-to`), so `error_stream` here always resolves to stderr.
fn finish_raw(result: Result<Vec<u8>, CliDocError>, ctx: &DocumentContext<'_>) -> ExitCode {
    match result {
        Ok(bytes) => write_raw_exit(&bytes),
        Err(err) => {
            let (event, code) = document_error_event(&err);
            emit_document_event(event, ctx, code, error_stream(output_to()))
        }
    }
}

/// Emit a document command's event to `stream`, redacting through
/// `ctx.secret_names` in addition to the crate's default `_secret`-suffix
/// convention.
fn emit_document_event(
    event: impl Into<Value>,
    ctx: &DocumentContext<'_>,
    code: u8,
    stream: Stream,
) -> ExitCode {
    let output_options = document_output_options(ctx);
    emit_event_to(event, ctx.format, &output_options, code, stream)
}

fn document_output_options(ctx: &DocumentContext<'_>) -> OutputOptions {
    OutputOptions {
        redaction: Redactor::new().secret_names(ctx.secret_names.iter().cloned()),
        style: PlainStyle::default(),
    }
}

/// Parse an explicit `--input-format` value into a [`DocumentFormat`].
fn parse_document_format(name: &str) -> Result<DocumentFormat, String> {
    match name.to_ascii_lowercase().as_str() {
        "json" => Ok(DocumentFormat::Json),
        "toml" => Ok(DocumentFormat::Toml),
        "yaml" | "yml" => Ok(DocumentFormat::Yaml),
        "dotenv" | "env" => Ok(DocumentFormat::Dotenv),
        "ini" => Ok(DocumentFormat::Ini),
        "toml-frontmatter" => Ok(DocumentFormat::TomlFrontmatter),
        "yaml-frontmatter" => Ok(DocumentFormat::YamlFrontmatter),
        other => Err(format!(
            "unsupported --input-format `{other}`; expected json, toml, yaml, yml, dotenv, env, ini, \
             toml-frontmatter, or yaml-frontmatter"
        )),
    }
}

/// Resolve an optional `--input-format` string into an optional
/// [`DocumentFormat`], surfacing a parse error as `Err`.
fn resolve_input_format(input_format: Option<&str>) -> Result<Option<DocumentFormat>, String> {
    input_format.map(parse_document_format).transpose()
}

/// Resolve `(value, format)` for a document read command from `FILE`
/// (`-` reads stdin). `-` defaults to JSON unless `input_format` overrides
/// it; a real path's format is detected from its extension unless
/// `input_format` overrides it.
fn read_document_input(
    file: &Path,
    input_format: Option<DocumentFormat>,
) -> Result<(DocumentValue, DocumentFormat), DocumentError> {
    if file == Path::new("-") {
        let format = input_format.unwrap_or(DocumentFormat::Json);
        let doc = Document::from_reader(std::io::stdin().lock(), format)?;
        return Ok((doc.value().clone(), doc.format()));
    }
    let doc = DocumentFile::open(file, input_format)?;
    Ok((doc.value().clone(), doc.format()))
}

/// Extract `key`'s scalar as raw bytes for `value`: a string's bytes are
/// copied verbatim; other scalars render their display form; `Null` renders
/// `"null"`; a non-finite float, array, or object is rejected.
///
/// §4 digit fidelity: [`DocumentValue::Number`] is emitted byte for byte
/// from its stored literal — never routed through `f64::to_string()`, which
/// is exactly the corruption path this variant exists to avoid. `value` is
/// otherwise type-lossy (every scalar becomes plain text) but this makes it
/// digit-faithful: the exact source digits, always.
fn document_scalar_bytes(value: &DocumentValue, key: &str) -> Result<Vec<u8>, String> {
    let text = match value {
        DocumentValue::String(value) => return Ok(value.as_bytes().to_vec()),
        DocumentValue::Bool(value) => value.to_string(),
        DocumentValue::Integer(value) => value.to_string(),
        DocumentValue::Unsigned(value) => value.to_string(),
        DocumentValue::Float(value) => {
            if !value.is_finite() {
                return Err(format!("non-finite scalar at `{key}`"));
            }
            value.to_string()
        }
        DocumentValue::Number(text) => return Ok(text.clone().into_bytes()),
        DocumentValue::Null => "null".to_string(),
        DocumentValue::Array(_) | DocumentValue::Object(_) => {
            return Err(format!("path `{key}` is not a scalar"));
        }
    };
    Ok(text.into_bytes())
}

/// Whether `key`'s leaf (final) path segment would be redacted by afdata's
/// secret-naming convention: an exact `_secret`/`_SECRET` suffix, or an exact
/// match against `secret_names` (the `--secret-name` list).
fn document_leaf_is_secret(key: &str, secret_names: &[String]) -> Result<bool, DocumentError> {
    let segments = parse_path(key)?;
    let Some(leaf) = segments.last() else {
        return Err(DocumentError::EmptyPath);
    };
    let redactor = Redactor::new().secret_names(secret_names.iter().cloned());
    Ok(redactor.is_secret_name(leaf))
}

/// Reject a literal `-` FILE for a mutation command (D1): unlike read
/// commands (where FILE `-` means stdin), mutation commands never read
/// stdin, so `-` has no meaning and is a usage error rather than a silent
/// attempt to open a file literally named `-`.
fn reject_mutation_dash(file: &Path) -> Result<(), CliDocError> {
    if file == Path::new("-") {
        return Err(CliDocError::Usage(
            "`-` is not a valid FILE for a mutation command; mutation commands never read stdin"
                .to_string(),
        ));
    }
    Ok(())
}

/// Write `bytes` directly to stdout with no AFDATA envelope and no forced
/// trailing newline (used by `value`'s raw scalar output and `paths`/`keys`'
/// line-oriented output).
fn write_raw_exit(bytes: &[u8]) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    if let Err(err) = stdout.write_all(bytes) {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            return ExitCode::from(0);
        }
        return ExitCode::from(1);
    }
    if stdout.flush().is_err() {
        return ExitCode::from(1);
    }
    ExitCode::from(0)
}

/// `get`: read a document whole, or the value at dot-path `key` (D2 — `show`
/// no longer exists; omitting `key` is the whole-document form).
fn run_get(file: &Path, key: Option<&str>, ctx: &DocumentContext<'_>) -> ExitCode {
    finish_document(compute_get(file, key, ctx), ctx)
}

fn compute_get(
    file: &Path,
    key: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> Result<Value, CliDocError> {
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let (root, doc_format) = read_document_input(file, input_format)?;
    let mut payload = serde_json::Map::new();
    payload.insert("code".to_string(), json!("document"));
    payload.insert("format".to_string(), json!(doc_format.name()));
    let json_value: Value = match key {
        None => root.into(),
        Some(key) => {
            let target = get_path(&root, key, &[])?;
            let is_secret = document_leaf_is_secret(key, ctx.secret_names)?;
            payload.insert("key".to_string(), json!(key));
            // A generic whole-document redact walk (applied via the
            // `--secret-name` output options below) only rewrites object
            // fields it finds by name; the value here sits under the
            // generic `"value"` wrapper key instead of its own field name,
            // so a directly-targeted secret leaf needs this explicit check.
            if is_secret {
                json!("***")
            } else {
                target.into()
            }
        }
    };
    payload.insert("value".to_string(), json_value);
    Ok(Value::Object(payload))
}

/// `value`: like `get` with a required KEY, but writes only the scalar's raw
/// bytes to stdout — no AFDATA envelope, no forced trailing newline. Arrays,
/// objects, and non-finite floats are rejected. A secret-named leaf is
/// rejected unless `--reveal-secret` is passed (never bypassed by default,
/// not even by `--default`). `--default VAL` prints `VAL` instead of erroring
/// when KEY's path does not exist or its value is `null`; an empty string is
/// a real value and does not trigger it (§2).
fn run_value_get(
    file: &Path,
    key: &str,
    reveal_secret: bool,
    default: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    finish_raw(compute_value(file, key, reveal_secret, default, ctx), ctx)
}

fn compute_value(
    file: &Path,
    key: &str,
    reveal_secret: bool,
    default: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> Result<Vec<u8>, CliDocError> {
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let (root, _doc_format) = read_document_input(file, input_format)?;
    if !reveal_secret && document_leaf_is_secret(key, ctx.secret_names)? {
        return Err(CliDocError::runtime(
            "document_secret_redacted",
            format!("path `{key}` names a secret; pass --reveal-secret"),
        ));
    }
    let target = match get_path(&root, key, &[]) {
        Ok(target) => target,
        Err(err) => {
            let err = CliDocError::Document(err);
            if default.is_some() && is_path_not_found(&err) {
                return Ok(default.unwrap_or_default().as_bytes().to_vec());
            }
            return Err(err);
        }
    };
    if target.is_null()
        && let Some(default) = default
    {
        return Ok(default.as_bytes().to_vec());
    }
    document_scalar_bytes(&target, key)
        .map_err(|message| CliDocError::runtime("document_not_scalar", message))
}

/// `paths`/`keys` share everything except how a child name becomes an output
/// line: `Paths` emits the full grammar-escaped dot-path from the root,
/// `Keys` emits the raw child key name / array index.
enum EnumerateMode {
    Paths,
    Keys,
}

/// §1: enumerate the immediate children of the container at `key` (the
/// document's top level when `key` is omitted). Rejects a scalar target (the
/// dual of `value` rejecting a container target). `--missing-ok` swallows
/// only a `document_path_not_found`-coded failure into empty output + exit
/// 0; every other error still fails, matching `value`'s stdout-stays-empty
/// contract (R1).
fn run_enumerate(
    file: &Path,
    key: Option<&str>,
    input_format: Option<&str>,
    missing_ok: bool,
    null_separated: bool,
    format: OutputFormat,
    mode: EnumerateMode,
) -> ExitCode {
    let ctx = DocumentContext {
        input_format,
        secret_names: &[],
        format,
    };
    match compute_enumerate(file, key, &ctx, mode) {
        Ok(lines) => {
            let separator: u8 = if null_separated { 0 } else { b'\n' };
            let mut bytes = Vec::new();
            for line in &lines {
                bytes.extend_from_slice(line.as_bytes());
                bytes.push(separator);
            }
            write_raw_exit(&bytes)
        }
        Err(err) if missing_ok && is_path_not_found(&err) => write_raw_exit(&[]),
        Err(err) => {
            let (event, code) = document_error_event(&err);
            emit_document_event(event, &ctx, code, error_stream(output_to()))
        }
    }
}

fn compute_enumerate(
    file: &Path,
    key: Option<&str>,
    ctx: &DocumentContext<'_>,
    mode: EnumerateMode,
) -> Result<Vec<String>, CliDocError> {
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let (root, _doc_format) = read_document_input(file, input_format)?;
    let base_segments: Vec<String> = match key {
        Some(key) => parse_path(key)?,
        None => Vec::new(),
    };
    let target = match key {
        Some(key) => get_path(&root, key, &[])?,
        None => root,
    };
    let names: Vec<String> = match &target {
        DocumentValue::Object(map) => map.keys().cloned().collect(),
        DocumentValue::Array(items) => (0..items.len()).map(|index| index.to_string()).collect(),
        _ => {
            return Err(CliDocError::runtime(
                "document_not_container",
                format!(
                    "path `{}` is a scalar; nothing to enumerate",
                    key.unwrap_or("<root>")
                ),
            ));
        }
    };
    Ok(names
        .into_iter()
        .map(|name| match mode {
            EnumerateMode::Keys => name,
            EnumerateMode::Paths => {
                let mut segments = base_segments.clone();
                segments.push(name);
                join_path(&segments)
            }
        })
        .collect())
}

/// `set`: write a value at dot-path `key` into `file`, preserving the rest
/// of the document's source formatting. See the `Set` variant's doc comment
/// for the bare-VALUE/`--value-type`/heterogeneous-overwrite-guard contract
/// (§3) and `--secret-from` (D4).
fn run_set(
    file: &Path,
    key: &str,
    value: Option<String>,
    value_type: Option<&str>,
    secret_from: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    finish_document(
        compute_set(file, key, value, value_type, secret_from, ctx),
        ctx,
    )
}

fn compute_set(
    file: &Path,
    key: &str,
    value: Option<String>,
    value_type: Option<&str>,
    secret_from: Option<&str>,
    ctx: &DocumentContext<'_>,
) -> Result<Value, CliDocError> {
    reject_mutation_dash(file)?;
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let mut doc = DocumentFile::open(file, input_format)?;
    // Guard the target before consuming a `--secret-from stdin`/`prompt`/`fd:N`
    // source: an unsafe target (symlink, or on unix a hardlink) must be
    // rejected before the secret is read, not after.
    doc.ensure_mutable("set")?;

    let new_value = if let Some(secret_from) = secret_from {
        let source = parse_secret_from(secret_from).map_err(CliDocError::Usage)?;
        DocumentValue::String(read_secret_from(source)?)
    } else if let Some(type_name) = value_type {
        let parsed_type = ValueType::parse(type_name).ok_or_else(|| {
            CliDocError::Usage(format!(
                "invalid --value-type `{type_name}`; expected string, number, bool, null, or json"
            ))
        })?;
        value_from_type(parsed_type, value.as_deref())?
    } else {
        let raw = value.ok_or_else(|| {
            CliDocError::Usage(
                "set requires VALUE, --value-type null, or --secret-from".to_string(),
            )
        })?;
        // §3 异型覆盖守卫: a bare VALUE is always a string, so overwriting an
        // *existing* scalar of a different kind would silently change its
        // type — that is an argument error, not a coercion decision. A
        // brand-new key, an existing string, or a container are unguarded.
        let existing = get_path(doc.value(), key, &[]).ok();
        if let Err(kind) = guard_bare_overwrite(existing.as_ref()) {
            return Err(CliDocError::Usage(format!(
                "bare VALUE would silently change `{key}` from {kind} to string; pass \
                 --value-type {kind} to keep its type, or --value-type string to convert it \
                 explicitly",
                kind = kind.value_type_name()
            )));
        }
        DocumentValue::String(raw)
    };

    doc.set(key, new_value)?;
    doc.save()?;
    Ok(json!({
        "code": "document_set",
        "format": doc.format().name(),
        "key": key,
        "path": doc.path().display().to_string(),
    }))
}

/// `add`: append an element to the keyed list at dot-path `key` in `file`,
/// preserving the rest of the document's source formatting. Idempotency:
/// adding a `slug` that already exists is an error (`document_slug_exists`).
fn run_add(
    file: &Path,
    key: &str,
    slug: &str,
    slug_field: &str,
    fields: &[String],
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    finish_document(compute_add(file, key, slug, slug_field, fields, ctx), ctx)
}

fn compute_add(
    file: &Path,
    key: &str,
    slug: &str,
    slug_field: &str,
    fields: &[String],
    ctx: &DocumentContext<'_>,
) -> Result<Value, CliDocError> {
    reject_mutation_dash(file)?;
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let mut field_pairs: Vec<(String, DocumentValue)> = Vec::with_capacity(fields.len());
    for field in fields {
        let Some((name, value)) = field.split_once('=') else {
            return Err(CliDocError::Usage(format!(
                "field `{field}` must use FIELD=VALUE"
            )));
        };
        if name.is_empty() {
            return Err(CliDocError::Usage(
                "field name must not be empty".to_string(),
            ));
        }
        // §3 "堵侧门": add's FIELD=VALUE is always a string, the same
        // zero-coercion rule as set's bare VALUE — no separate type syntax.
        field_pairs.push((name.to_string(), DocumentValue::String(value.to_string())));
    }
    let mut doc = DocumentFile::open(file, input_format)?;
    doc.add(key, slug, slug_field, &field_pairs)?;
    doc.save()?;
    Ok(json!({
        "code": "document_added",
        "format": doc.format().name(),
        "key": key,
        "slug": slug,
        "path": doc.path().display().to_string(),
    }))
}

/// `remove`: delete the element identified by `slug`/`slug_field` from the
/// keyed list at dot-path `key` in `file`, preserving the rest of the
/// document's source formatting. Idempotency: removing a `slug` that does
/// not exist is an error (`document_slug_not_found`).
fn run_remove(
    file: &Path,
    key: &str,
    slug: &str,
    slug_field: &str,
    ctx: &DocumentContext<'_>,
) -> ExitCode {
    finish_document(compute_remove(file, key, slug, slug_field, ctx), ctx)
}

fn compute_remove(
    file: &Path,
    key: &str,
    slug: &str,
    slug_field: &str,
    ctx: &DocumentContext<'_>,
) -> Result<Value, CliDocError> {
    reject_mutation_dash(file)?;
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let mut doc = DocumentFile::open(file, input_format)?;
    doc.remove(key, slug, slug_field)?;
    doc.save()?;
    Ok(json!({
        "code": "document_removed",
        "format": doc.format().name(),
        "key": key,
        "slug": slug,
        "path": doc.path().display().to_string(),
    }))
}

/// `unset`: remove the entry at dot-path `key` entirely from `file`,
/// preserving the rest of the document's source formatting. Idempotency:
/// unsetting an absent `key` is an error (`document_path_not_found`).
fn run_unset(file: &Path, key: &str, ctx: &DocumentContext<'_>) -> ExitCode {
    finish_document(compute_unset(file, key, ctx), ctx)
}

fn compute_unset(file: &Path, key: &str, ctx: &DocumentContext<'_>) -> Result<Value, CliDocError> {
    reject_mutation_dash(file)?;
    let input_format = resolve_input_format(ctx.input_format).map_err(CliDocError::Usage)?;
    let mut doc = DocumentFile::open(file, input_format)?;
    // `Document::unset` is idempotent, but the `afdata unset` CLI keeps its
    // strict contract: unsetting an absent key is a caught error, so scripts
    // can tell an actual removal from a no-op.
    if !doc.unset(key)? {
        return Err(DocumentError::PathNotFound {
            path: key.to_string(),
        }
        .into());
    }
    doc.save()?;
    Ok(json!({
        "code": "document_unset",
        "format": doc.format().name(),
        "key": key,
        "path": doc.path().display().to_string(),
    }))
}

// ═══════════════════════════════════════════
// D4: --secret-from stdin|prompt|fd:<N>|env:<VAR>
// ═══════════════════════════════════════════

enum SecretSource {
    Stdin,
    Prompt,
    Fd(i32),
    Env(String),
}

/// Parse a `--secret-from` flag value. Descriptor/name shape problems
/// (non-numeric `fd:`, `fd:` below 3, empty `env:` name, an unrecognized
/// source keyword) are usage errors (R2) caught here, before anything is
/// read; a failure actually *reading* the source is a separate runtime
/// concern (see [`read_secret_from`]).
fn parse_secret_from(spec: &str) -> Result<SecretSource, String> {
    match spec {
        "stdin" => Ok(SecretSource::Stdin),
        "prompt" => Ok(SecretSource::Prompt),
        other => {
            if let Some(fd) = other.strip_prefix("fd:") {
                let number = fd.parse::<i32>().map_err(|_| {
                    format!("invalid --secret-from `{spec}`; fd: requires a numeric descriptor")
                })?;
                if number < 3 {
                    return Err(format!(
                        "invalid --secret-from `{spec}`; fd: requires a descriptor >= 3"
                    ));
                }
                Ok(SecretSource::Fd(number))
            } else if let Some(var) = other.strip_prefix("env:") {
                if var.is_empty() {
                    return Err(format!(
                        "invalid --secret-from `{spec}`; env: requires a variable name"
                    ));
                }
                Ok(SecretSource::Env(var.to_string()))
            } else {
                Err(format!(
                    "invalid --secret-from `{spec}`; expected stdin, prompt, fd:<N>, or env:<VAR>"
                ))
            }
        }
    }
}

const MAX_VALUE_SECRET_BYTES: usize = 1024 * 1024;

fn read_secret_from(source: SecretSource) -> Result<String, CliDocError> {
    match source {
        SecretSource::Stdin => read_secret_reader(std::io::stdin().lock(), "stdin"),
        SecretSource::Prompt => {
            #[cfg(unix)]
            {
                read_secret_prompt()
            }
            #[cfg(not(unix))]
            {
                Err(CliDocError::runtime(
                    "document_secret_source_failed",
                    "prompt secret input is unsupported on this platform",
                ))
            }
        }
        SecretSource::Fd(number) => {
            #[cfg(unix)]
            {
                use std::os::unix::io::FromRawFd;
                // SAFETY: ownership is transferred exactly once and the descriptor is closed on drop.
                let file = unsafe { std::fs::File::from_raw_fd(number) };
                read_secret_reader(file, "file descriptor")
            }
            #[cfg(not(unix))]
            {
                let _ = number;
                Err(CliDocError::runtime(
                    "document_secret_source_failed",
                    "raw file descriptors are unsupported on this platform",
                ))
            }
        }
        SecretSource::Env(name) => std::env::var(&name).map_err(|_| {
            CliDocError::runtime(
                "document_secret_source_failed",
                format!("environment variable `{name}` is not set"),
            )
        }),
    }
}

fn read_secret_reader<R: std::io::Read>(reader: R, source: &str) -> Result<String, CliDocError> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_VALUE_SECRET_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            CliDocError::runtime(
                "document_secret_source_failed",
                format!("read secret from {source}: {error}"),
            )
        })?;
    if bytes.len() > MAX_VALUE_SECRET_BYTES {
        return Err(CliDocError::runtime(
            "document_secret_source_failed",
            format!("secret exceeds {MAX_VALUE_SECRET_BYTES} bytes"),
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        CliDocError::runtime(
            "document_secret_source_failed",
            "secret input must be valid UTF-8",
        )
    })
}

#[cfg(unix)]
fn read_secret_prompt() -> Result<String, CliDocError> {
    use std::io::BufRead;
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|error| {
            CliDocError::runtime(
                "document_secret_source_failed",
                format!("open controlling terminal: {error}"),
            )
        })?;
    let status = std::process::Command::new("stty")
        .args(["-echo"])
        .status()
        .map_err(|error| {
            CliDocError::runtime(
                "document_secret_source_failed",
                format!("disable terminal echo: {error}"),
            )
        })?;
    if !status.success() {
        return Err(CliDocError::runtime(
            "document_secret_source_failed",
            "disable terminal echo failed",
        ));
    }
    let _echo_guard = TerminalEchoGuard;
    write!(tty, "Secret: ").map_err(|error| {
        CliDocError::runtime(
            "document_secret_source_failed",
            format!("write prompt: {error}"),
        )
    })?;
    let mut value = String::new();
    let result = {
        let mut reader = std::io::BufReader::new(&mut tty);
        reader.read_line(&mut value)
    }
    .map_err(|error| {
        CliDocError::runtime(
            "document_secret_source_failed",
            format!("read secret from prompt: {error}"),
        )
    });
    let _ = writeln!(tty);
    result?;
    let value = value.trim_end_matches(['\n', '\r']);
    if value.len() > MAX_VALUE_SECRET_BYTES {
        return Err(CliDocError::runtime(
            "document_secret_source_failed",
            format!("secret exceeds {MAX_VALUE_SECRET_BYTES} bytes"),
        ));
    }
    Ok(value.to_string())
}

#[cfg(unix)]
struct TerminalEchoGuard;

#[cfg(unix)]
impl Drop for TerminalEchoGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("stty").arg("echo").status();
    }
}

// ═══════════════════════════════════════════
// Lint rules (deterministic AFDATA naming/suffix checks)
// ═══════════════════════════════════════════

fn lint_value(value: &Value, pointer: &str, findings: &mut Vec<Finding>) {
    lint_unsafe_integer(value, pointer, findings);
    match value {
        Value::Object(map) => {
            if let Some(Value::Object(properties)) = map.get("properties") {
                for (name, schema) in properties {
                    lint_secret_schema_property(
                        name,
                        schema,
                        &join_pointer(pointer, "properties"),
                        findings,
                    );
                }
            }
            for (key, child) in map {
                lint_suffix_type(key, child, &join_pointer(pointer, key), findings);
                lint_value(child, &join_pointer(pointer, key), findings);
            }
        }
        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                lint_value(item, &join_pointer(pointer, &idx.to_string()), findings);
            }
        }
        _ => {}
    }
}

fn lint_secret_schema_property(
    name: &str,
    schema: &Value,
    properties_pointer: &str,
    findings: &mut Vec<Finding>,
) {
    if !name.ends_with("_secret") {
        return;
    }
    let Some(obj) = schema.as_object() else {
        return;
    };
    let property_pointer = join_pointer(properties_pointer, name);
    for field in ["default", "example"] {
        if let Some(value) = obj.get(field)
            && !is_redacted_secret_literal(value)
        {
            findings.push(Finding::error(
                "secret_schema_value_exposed",
                join_pointer(&property_pointer, field),
                format!("schema property {name:?} exposes secret {field}"),
            ));
        }
    }
    if let Some(Value::Array(examples)) = obj.get("examples") {
        for (idx, value) in examples.iter().enumerate() {
            if !is_redacted_secret_literal(value) {
                findings.push(Finding::error(
                    "secret_schema_value_exposed",
                    join_pointer(
                        &join_pointer(&property_pointer, "examples"),
                        &idx.to_string(),
                    ),
                    format!("schema property {name:?} exposes secret example"),
                ));
            }
        }
    }
}

fn is_redacted_secret_literal(value: &Value) -> bool {
    matches!(value, Value::Null) || matches!(value, Value::String(s) if s == "***")
}

fn lint_suffix_type(key: &str, value: &Value, pointer: &str, findings: &mut Vec<Finding>) {
    // `null` means the field is absent/unset, not present-with-the-wrong-type:
    // the suffix type constraint below applies only to present, non-null
    // values. Absence may be expressed by omitting the key entirely or by an
    // explicit `null`; both are valid. This mirrors `is_redacted_secret_literal`,
    // which already treats `Value::Null` as a valid absent literal for
    // `_secret` schema properties.
    if value.is_null() {
        return;
    }
    let message = if key.ends_with("_bytes") {
        if is_non_negative_integer(value) {
            None
        } else {
            Some(format!("{key:?} must be a non-negative integer byte count"))
        }
    } else if key.ends_with("_epoch_s") || key.ends_with("_epoch_ms") {
        if is_integer(value) {
            None
        } else {
            Some(format!("{key:?} must be an integer epoch timestamp"))
        }
    } else if key.ends_with("_epoch_ns") {
        if is_decimal_integer_string(value) {
            None
        } else {
            Some(format!("{key:?} must be a decimal integer string"))
        }
    } else if key.ends_with("_sats") || key.ends_with("_msats") {
        if is_integer(value) || is_decimal_integer_string(value) {
            None
        } else {
            Some(format!(
                "{key:?} must be an integer or decimal integer string"
            ))
        }
    } else if key.ends_with("_percent") {
        if value.is_number() {
            None
        } else {
            Some(format!("{key:?} must be numeric"))
        }
    } else if is_duration_suffix(key) {
        if value.is_number() {
            None
        } else {
            Some(format!("{key:?} must be a numeric duration"))
        }
    } else if is_currency_minor_unit_suffix(key) {
        if is_integer(value) {
            None
        } else {
            Some(format!("{key:?} must be an integer currency amount"))
        }
    } else if key.ends_with("_rfc3339") {
        if value.as_str().is_some_and(is_valid_rfc3339) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 date-time with a mandatory offset (e.g. 2026-02-14T10:30:00Z)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if key.ends_with("_url") {
        if value.as_str().is_some_and(is_wellformed_url_field) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be a single URL (no internal whitespace or bare credentials)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if key.ends_with("_bcp47") {
        if value.as_str().is_some_and(is_valid_bcp47) {
            None
        } else if value.is_string() {
            Some(format!("{key:?} must be a well-formed BCP 47 language tag"))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if key.ends_with("_rfc3339_date") {
        if value.as_str().is_some_and(is_valid_rfc3339_date) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 full-date (YYYY-MM-DD)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if key.ends_with("_rfc3339_time") {
        if value.as_str().is_some_and(is_valid_rfc3339_time) {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be an RFC 3339 partial-time (HH:MM:SS[.fraction], no Z or offset)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else if key.ends_with("_utc_offset") {
        if value.as_str().and_then(normalize_utc_offset).is_some() {
            None
        } else if value.is_string() {
            Some(format!(
                "{key:?} must be a fixed UTC offset (\"UTC\" or ±HH:MM)"
            ))
        } else {
            Some(format!("{key:?} must be a string"))
        }
    } else {
        None
    };
    if let Some(message) = message {
        findings.push(Finding::error(
            "suffix_type_mismatch",
            pointer.to_string(),
            message,
        ));
    }
}

fn lint_unsafe_integer(value: &Value, pointer: &str, findings: &mut Vec<Finding>) {
    let Value::Number(number) = value else {
        return;
    };
    if number.is_i64() {
        let Some(value) = number.as_i64() else {
            return;
        };
        if value.unsigned_abs() > MAX_SAFE_INTEGER {
            findings.push(unsafe_integer_finding(pointer));
        }
    } else if number.is_u64() {
        let Some(value) = number.as_u64() else {
            return;
        };
        if value > MAX_SAFE_INTEGER {
            findings.push(unsafe_integer_finding(pointer));
        }
    }
}

fn unsafe_integer_finding(pointer: &str) -> Finding {
    Finding::error(
        "unsafe_integer",
        pointer.to_string(),
        "integer exceeds JavaScript safe integer range ±(2^53-1)".to_string(),
    )
}

fn is_integer(value: &Value) -> bool {
    matches!(value, Value::Number(number) if number.is_i64() || number.is_u64())
}

fn is_non_negative_integer(value: &Value) -> bool {
    matches!(value, Value::Number(number) if number.as_u64().is_some())
}

fn is_decimal_integer_string(value: &Value) -> bool {
    let Value::String(text) = value else {
        return false;
    };
    let digits = text.strip_prefix('-').unwrap_or(text);
    !digits.is_empty() && digits.chars().all(|ch| ch.is_ascii_digit())
}

/// A numeric duration suffix (`timeout_s`, `retry_after_ms`, `ttl_minutes`, …).
/// The epoch suffixes (`_epoch_s`/`_epoch_ms`/`_epoch_ns`) are matched earlier in
/// the chain, so they never reach here.
fn is_duration_suffix(key: &str) -> bool {
    key.ends_with("_ns")
        || key.ends_with("_us")
        || key.ends_with("_ms")
        || key.ends_with("_s")
        || key.ends_with("_minutes")
        || key.ends_with("_hours")
        || key.ends_with("_days")
}

/// An integer minor-unit currency suffix (`price_usd_cents`, `fee_jpy`,
/// `budget_btc_micro`, …). `_sats`/`_msats` allow a decimal-string form and are
/// matched earlier in the chain.
fn is_currency_minor_unit_suffix(key: &str) -> bool {
    key.ends_with("_cents") || key.ends_with("_micro") || key.ends_with("_jpy")
}

/// True when a `_url` field value is a single URL: a scheme-prefixed absolute URL,
/// or a schemeless relative reference with no internal whitespace and no bare `@`
/// credential sigil. This mirrors the redaction gate in the library's
/// `redaction.rs`: a value this rejects is exactly one redaction would blanket-
/// redact (internal whitespace, or a schemeless `user:pass@host` connection
/// string) rather than surgically clean.
fn is_wellformed_url_field(s: &str) -> bool {
    if is_scheme_prefixed_url(s) || is_scheme_prefixed_url(s.trim()) {
        return true;
    }
    !s.chars().any(char::is_whitespace) && !s.contains('@')
}

/// True when `s` begins with a URL scheme (`ALPHA *(ALPHA / DIGIT / "+" / "-" /
/// ".") "://"`) and contains no ASCII whitespace — a single bare absolute URL.
fn is_scheme_prefixed_url(s: &str) -> bool {
    if s.bytes().any(|b| b.is_ascii_whitespace()) {
        return false;
    }
    let bytes = s.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.') {
            i += 1;
        } else {
            break;
        }
    }
    s[i..].starts_with("://")
}

fn join_pointer(base: &str, token: &str) -> String {
    let escaped = token.replace('~', "~0").replace('/', "~1");
    if base.is_empty() {
        format!("/{escaped}")
    } else {
        format!("{base}/{escaped}")
    }
}

#[cfg(all(test, feature = "skill"))]
mod skill_tests {
    use super::*;

    #[test]
    fn parses_and_validates_skill_directory() {
        let parsed =
            Cli::try_parse_from(["afdata", "skill", "validate", "skills/agent-first-data"]);
        assert!(matches!(
            parsed,
            Ok(Cli {
                command: Command::Skill(SkillCommand::Validate { input }),
                ..
            }) if input == Path::new("skills/agent-first-data")
        ));

        let loaded = read_skill_input(Path::new("skills/agent-first-data"));
        assert!(matches!(
            loaded,
            Ok((text, Some(expected_name), _))
                if expected_name == "agent-first-data"
                    && agent_first_data::skill::validate_skill_named(&text, &expected_name).is_ok()
        ));
    }

    #[cfg(feature = "skill-admin")]
    #[test]
    fn keeps_existing_skill_admin_command_shape() {
        let parsed = Cli::try_parse_from([
            "afdata",
            "skill",
            "status",
            "--agent",
            "opencode",
            "--scope",
            "workspace",
        ]);
        assert!(matches!(
            parsed,
            Ok(Cli {
                command: Command::Skill(SkillCommand::Status { agent, scope, .. }),
                ..
            }) if agent == "opencode" && scope == "workspace"
        ));
    }

    #[cfg(feature = "skill-admin")]
    #[test]
    fn admin_subcommands_reject_a_validate_only_input_path() {
        // Nested clap subcommands (D5) mean `status`/`install`/`uninstall`
        // simply have no positional INPUT argument at all — an extra
        // positional is a clap parse error, not a runtime branch.
        let parsed = Cli::try_parse_from(["afdata", "skill", "status", "some/path"]);
        assert!(parsed.is_err());
    }
}
