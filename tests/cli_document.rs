#![cfg(feature = "cli")]
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    clippy::bool_assert_comparison
)]
//! CLI integration tests for afdata's document read/edit commands
//! (`get`/`value`/`paths`/`keys`/`set`/`add`/`remove`/`unset`) and the
//! post-redesign protocol-tool surface (`lint`/`validate`/`render`/`skill`).
//!
//! Every test invoking `CARGO_BIN_EXE_afdata` lives in this file, gated on
//! the `cli` feature (the bin target's `required-features`), so the file
//! still compiles when `cli` is disabled.
//!
//! Coverage is grouped by the document CLI's design invariants (D1–D7 and
//! R1–R7), including paths/keys, defaults, exact value types, and number
//! literal fidelity.

use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::TempDir;

#[cfg(feature = "yaml")]
use agent_first_data::document::Format;

fn afdata() -> Command {
    Command::new(env!("CARGO_BIN_EXE_afdata"))
}

fn run(args: &[&str]) -> std::process::Output {
    afdata().args(args).output().expect("failed to run afdata")
}

fn run_with_stdin(args: &[&str], stdin: &[u8]) -> std::process::Output {
    let mut child = afdata()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn afdata");
    {
        let mut handle = child.stdin.take().expect("stdin handle");
        // The child may reject its arguments and exit before reading stdin —
        // e.g. a raw-scalar command rejecting a non-default `--output-to` — which
        // closes the read end of the pipe first. A BrokenPipe on this write is
        // that expected early exit, not a test failure; the assertions below run
        // against the child's actual exit + output.
        match handle.write_all(stdin) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(err) => panic!("failed to write stdin: {err}"),
        }
    }
    child.wait_with_output().expect("failed to wait for afdata")
}

fn json_stdout(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|err| panic!("stdout is not JSON: {err}: {:?}", output.stdout))
}

fn json_stderr(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stderr)
        .unwrap_or_else(|err| panic!("stderr is not JSON: {err}: {:?}", output.stderr))
}

fn write_temp(dir: &TempDir, name: &str, contents: &str) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    path.to_str().unwrap().to_string()
}

const OVERSIZED_DOCUMENT_CANARY: &str = "oversized_document_secret_canary";

fn write_oversized_document(dir: &TempDir) -> String {
    use agent_first_data::value_source::MAX_FILE_BYTES;

    let path = dir.path().join("oversized.json");
    let mut file = std::fs::File::create(&path).unwrap();
    write!(
        file,
        "{{\"api_key_secret\":\"{OVERSIZED_DOCUMENT_CANARY}\",\"plain_value\":1,\"items\":[{{\"id\":\"existing\"}}]}}"
    )
    .unwrap();
    // A sparse file exercises the metadata cap without allocating/writing a
    // large fixture. The stable size error must precede document parsing.
    file.set_len(MAX_FILE_BYTES + 1).unwrap();
    assert_eq!(MAX_FILE_BYTES, 16 * 1024 * 1024);
    path.to_str().unwrap().to_string()
}

fn directory_entries(dir: &TempDir) -> Vec<std::ffi::OsString> {
    let mut entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    entries.sort();
    entries
}

fn assert_document_too_large(output: &std::process::Output, command: &str) {
    assert_eq!(output.status.code(), Some(1), "{command}: {output:?}");
    assert!(output.stdout.is_empty(), "{command}: {output:?}");
    let response = json_stderr(output);
    assert_eq!(response["kind"], "error", "{command}: {response}");
    assert_eq!(
        response["error"]["code"], "document_too_large",
        "{command}: {response}"
    );
    // Mutations refuse `-`, so the hint offers only the flag.
    assert_eq!(
        response["error"]["hint"], "raise the limit with --max-file-bytes BYTES",
        "{command}: {response}"
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains(OVERSIZED_DOCUMENT_CANARY),
        "{command} echoed document contents"
    );
}

#[test]
fn oversized_document_reads_are_rejected_without_echoing_contents() {
    let dir = TempDir::new().unwrap();
    let config_path = write_oversized_document(&dir);
    let original = std::fs::read(&config_path).unwrap();
    let entries = directory_entries(&dir);
    for arguments in [
        vec!["get", config_path.as_str()],
        vec!["value", config_path.as_str(), "plain_value"],
        vec!["values", config_path.as_str(), "plain_value"],
        vec!["paths", config_path.as_str()],
        vec!["keys", config_path.as_str()],
    ] {
        let command = arguments[0];
        let output = run(&arguments);
        assert_document_too_large(&output, command);
        assert!(
            std::fs::read(&config_path).unwrap() == original,
            "{command} changed the document"
        );
        assert_eq!(
            directory_entries(&dir),
            entries,
            "{command} changed the directory"
        );
    }
}

#[test]
fn oversized_document_mutations_leave_the_file_and_directory_unchanged() {
    let dir = TempDir::new().unwrap();
    let config_path = write_oversized_document(&dir);
    let original = std::fs::read(&config_path).unwrap();
    let entries = directory_entries(&dir);
    for arguments in [
        vec![
            "set",
            config_path.as_str(),
            "plain_value",
            "2",
            "--value-type",
            "number",
        ],
        vec![
            "add",
            config_path.as_str(),
            "items",
            "new",
            "--slug-field",
            "id",
        ],
        vec![
            "remove",
            config_path.as_str(),
            "items",
            "existing",
            "--slug-field",
            "id",
        ],
        vec!["unset", config_path.as_str(), "api_key_secret"],
    ] {
        let command = arguments[0];
        let output = run(&arguments);
        assert_document_too_large(&output, command);
        assert!(
            std::fs::read(&config_path).unwrap() == original,
            "{command} changed the document"
        );
        assert_eq!(
            directory_entries(&dir),
            entries,
            "{command} left a temporary file"
        );
    }
}

/// `--max-file-bytes` moves the budget in both directions, and the
/// too-large error names it as the way out. One small document under a cap
/// a few bytes either side of its size checks every command that reads a
/// document FILE, so a command that forgets to pass the flag through fails here.
#[test]
fn max_file_bytes_sets_the_document_read_budget() {
    let dir = TempDir::new().unwrap();
    let config_path = write_temp(
        &dir,
        "budget.json",
        r#"{"plain_value":1,"items":[{"id":"existing"}],"drop":true}"#,
    );
    let commands: Vec<Vec<&str>> = vec![
        vec!["lint", config_path.as_str()],
        vec!["get", config_path.as_str()],
        vec!["value", config_path.as_str(), "plain_value"],
        vec!["values", config_path.as_str(), "plain_value"],
        vec!["paths", config_path.as_str()],
        vec!["keys", config_path.as_str()],
        vec![
            "set",
            config_path.as_str(),
            "plain_value",
            "2",
            "--value-type",
            "number",
        ],
        vec![
            "add",
            config_path.as_str(),
            "items",
            "new",
            "--slug-field",
            "id",
        ],
        vec![
            "remove",
            config_path.as_str(),
            "items",
            "new",
            "--slug-field",
            "id",
        ],
        vec!["unset", config_path.as_str(), "drop"],
    ];
    for arguments in &commands {
        let command = arguments[0];
        let original = std::fs::read(&config_path).unwrap();
        // Edits grow and shrink the file, so each command gets a budget cut
        // to the size it is about to read.
        let size = original.len() as u64;
        let short = (size - 1).to_string();
        let exact = size.to_string();
        let mut too_small = arguments.clone();
        too_small.extend(["--max-file-bytes", short.as_str()]);
        let output = run(&too_small);
        assert_eq!(output.status.code(), Some(1), "{command}: {output:?}");
        let response = json_stderr(&output);
        assert_eq!(
            response["error"]["code"], "document_too_large",
            "{command}: {response}"
        );
        assert!(
            response["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("--max-file-bytes"),
            "{command}: {response}"
        );
        assert_eq!(
            std::fs::read(&config_path).unwrap(),
            original,
            "{command} wrote over budget"
        );

        let mut enough = arguments.clone();
        enough.extend(["--max-file-bytes", exact.as_str()]);
        let output = run(&enough);
        assert_eq!(output.status.code(), Some(0), "{command}: {output:?}");
    }
}

#[test]
fn max_file_bytes_must_be_a_positive_byte_count() {
    let dir = TempDir::new().unwrap();
    let config_path = write_temp(&dir, "budget.json", r#"{"plain_value":1}"#);
    for bad in ["0", "-1"] {
        for arguments in [
            vec!["get", config_path.as_str(), "--max-file-bytes", bad],
            vec![
                "unset",
                config_path.as_str(),
                "plain_value",
                "--max-file-bytes",
                bad,
            ],
            vec!["lint", config_path.as_str(), "--max-file-bytes", bad],
        ] {
            let output = run(&arguments);
            assert_eq!(output.status.code(), Some(2), "{arguments:?}: {output:?}");
            let response = json_stderr(&output);
            assert_eq!(
                response["error"]["code"], "document_usage_error",
                "{response}"
            );
        }
    }
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        r#"{"plain_value":1}"#
    );
}

/// `-` is the append position `set` writes to. Any other verb that reaches it
/// gets an address error saying so, not a hint to register a naming rule.
#[test]
fn append_position_is_refused_outside_set() {
    let dir = TempDir::new().unwrap();
    for (name, contents) in [
        ("list.json", "{\"a\":[1,2]}\n"),
        ("list.toml", "a = [1, 2]\n"),
    ] {
        let path = write_temp(&dir, name, contents);
        for arguments in [
            vec!["unset", path.as_str(), "a.-"],
            vec!["get", path.as_str(), "a.-"],
        ] {
            let output = run(&arguments);
            assert_eq!(output.status.code(), Some(1), "{arguments:?}: {output:?}");
            let response = json_stderr(&output);
            assert_eq!(
                response["error"]["code"], "document_invalid_path",
                "{response}"
            );
            let message = response["error"]["message"].as_str().unwrap();
            assert!(
                message.contains("only valid as the last segment of a `set` path"),
                "{message}"
            );
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
    }
}

/// A final `-` is the append position in every verb, even when an element's
/// slug is literally `-`; were it a name for `get` and `unset`, `get a.-`
/// would read an element that `set a.-` then appends beside.
#[test]
fn append_position_is_reserved_even_against_a_slug_named_dash() {
    let dir = TempDir::new().unwrap();
    let contents = "{\"a\":[{\"id\":\"-\",\"v\":1},{\"id\":\"z\",\"v\":2}]}\n";
    let path = write_temp(&dir, "list.json", contents);
    for arguments in [
        vec!["get", path.as_str(), "a.-", "--slug-field", "id"],
        vec!["value", path.as_str(), "a.-", "--slug-field", "id"],
        vec!["unset", path.as_str(), "a.-", "--slug-field", "id"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}: {output:?}");
        let response = json_stderr(&output);
        assert_eq!(
            response["error"]["code"], "document_invalid_path",
            "{arguments:?}: {response}"
        );
        let message = response["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("`-` names the append position")
                && message.contains("only valid as the last segment of a `set` path"),
            "{arguments:?}: {message}"
        );
    }
    // Inside a path it is not an element name either.
    for arguments in [
        vec!["get", path.as_str(), "a.-.v", "--slug-field", "id"],
        vec!["set", path.as_str(), "a.-.v", "3", "--slug-field", "id"],
        vec!["unset", path.as_str(), "a.-.v", "--slug-field", "id"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}: {output:?}");
        let response = json_stderr(&output);
        assert_eq!(
            response["error"]["code"], "document_invalid_path",
            "{arguments:?}: {response}"
        );
        let message = response["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("can only be the last segment of a path"),
            "{arguments:?}: {message}"
        );
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
}

/// `set` is the verb that writes `-`, so when `-` sits mid-path the refusal
/// says where `-` may go, not which verb may use it.
#[test]
fn append_position_inside_a_set_path_names_the_segment_rule() {
    let dir = TempDir::new().unwrap();
    for (name, contents) in [
        ("list.json", "{\"a\":[{\"b\":1}]}\n"),
        ("list.toml", "a = [{ b = 1 }]\n"),
        ("list.yaml", "a:\n  - b: 1\n"),
    ] {
        let path = write_temp(&dir, name, contents);
        let output = run(&["set", path.as_str(), "a.-.b", "2"]);
        assert_eq!(output.status.code(), Some(1), "{name}: {output:?}");
        let response = json_stderr(&output);
        assert_eq!(
            response["error"]["code"], "document_invalid_path",
            "{response}"
        );
        let message = response["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("can only be the last segment of a path")
                && !message.contains("only `set`"),
            "{name}: {message}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
    }
}

/// Removing the first element of a commented TOML array keeps every comment
/// that is not the removed element's own, and grows a table inside an inline
/// table with ordinary spacing.
#[cfg(feature = "toml")]
#[test]
fn toml_edits_keep_comments_and_inline_spacing() {
    let dir = TempDir::new().unwrap();
    for (contents, expected) in [
        (
            "a = [\n  1, # one\n  # about two\n  2,\n]\n",
            "a = [\n  # about two\n  2,\n]\n",
        ),
        (
            "a = [ # header\n  # about one\n  1,\n  2,\n]\n",
            "a = [ # header\n  2,\n]\n",
        ),
        ("a = [ 1, 2 ]\n", "a = [ 2 ]\n"),
    ] {
        let path = write_temp(&dir, "array.toml", contents);
        let output = run(&["unset", path.as_str(), "a.0"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    }

    let path = write_temp(&dir, "inline.toml", "a = { b = 1 }\n");
    let output = run(&["set", path.as_str(), "a.c.d", "2", "--value-type", "number"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "a = { b = 1, c = { d = 2 } }\n"
    );
}

/// Appending to an array of tables is refused with the array-of-tables reason,
/// not as a key lookup inside a non-table value.
#[cfg(feature = "toml")]
#[test]
fn toml_append_to_an_array_of_tables_explains_the_refusal() {
    let dir = TempDir::new().unwrap();
    let contents = "[[x]]\nk = 1\n";
    let path = write_temp(&dir, "tables.toml", contents);
    let output = run(&["set", path.as_str(), "x.-", "{}", "--value-type", "json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let response = json_stderr(&output);
    assert_eq!(
        response["error"]["code"], "document_unsupported_operation",
        "{response}"
    );
    let message = response["error"]["message"].as_str().unwrap();
    assert!(message.contains("array of tables"), "{message}");
    assert!(!message.contains("non-table"), "{message}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
}

/// An empty YAML flow sequence takes any string, including one with a
/// character YAML only allows escaped, and reads it back unchanged.
#[cfg(feature = "yaml")]
#[test]
fn yaml_append_to_an_empty_flow_sequence_escapes_what_yaml_forbids_raw() {
    let dir = TempDir::new().unwrap();
    for (contents, append, element) in [
        ("[]\n", "-", "0"),
        ("- []\n", "0.-", "0.0"),
        ("items: []\n", "items.-", "items.0"),
    ] {
        for text in ["a\u{7f}b", "c1\u{85}\u{9f}", "plain"] {
            let path = write_temp(&dir, "list.yaml", contents);
            let output = run(&["set", path.as_str(), append, text]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{contents:?} {text:?}: {output:?}"
            );
            let read = run(&["value", path.as_str(), element]);
            assert_eq!(read.status.code(), Some(0), "{read:?}");
            assert_eq!(String::from_utf8(read.stdout).unwrap(), text);
        }
    }
}

/// `{ }` is as empty as `{}`, so parents grow inside either.
#[cfg(feature = "yaml")]
#[test]
fn yaml_creates_parents_inside_an_empty_flow_mapping_with_inner_space() {
    let dir = TempDir::new().unwrap();
    for contents in ["α: {}\n", "α: { }\n"] {
        let path = write_temp(&dir, "map.yaml", contents);
        let output = run(&["set", path.as_str(), "α.γ.δ", "x"]);
        assert_eq!(output.status.code(), Some(0), "{contents:?}: {output:?}");
        let read = run(&["value", path.as_str(), "α.γ.δ"]);
        assert_eq!(read.stdout, b"x", "{contents:?}: {read:?}");
    }
}

/// An edit that would land behind a YAML alias names the alias and the rule,
/// and leaks nothing of the YAML library's own vocabulary.
#[cfg(feature = "yaml")]
#[test]
fn yaml_edits_through_an_alias_name_the_alias() {
    let dir = TempDir::new().unwrap();
    for (contents, arguments) in [
        ("a: &x {k: 1}\nb: *x\n", vec!["set", "b.n", "v"]),
        (
            "a: &x {k: 1}\nb: *x\n",
            vec!["set", "b.k", "2", "--value-type", "number"],
        ),
        ("a: &x {k: 1}\nb: *x\n", vec!["unset", "b.k"]),
        (
            "a: &x {k: 1}\nb: *x\n",
            vec!["set", "b", "[]", "--value-type", "json"],
        ),
        ("- &x {k: 1}\n- *x\n", vec!["set", "1.n", "v"]),
        ("l:\n  - &x [1]\n  - *x\n", vec!["set", "l.1.-", "2"]),
    ] {
        let path = write_temp(&dir, "alias.yaml", contents);
        let mut full = vec![arguments[0], path.as_str()];
        full.extend(&arguments[1..]);
        let output = run(&full);
        assert_eq!(output.status.code(), Some(1), "{full:?}: {output:?}");
        let response = json_stderr(&output);
        assert_eq!(
            response["error"]["code"], "document_unsupported_operation",
            "{full:?}: {response}"
        );
        let message = response["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("reached through the alias `*x`")
                && message.contains("does not edit through YAML aliases"),
            "{full:?}: {message}"
        );
        assert!(
            !message.contains("materialise") && !message.contains("parse error"),
            "{full:?}: {message}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
    }
}

/// Stdin is a stream the caller chose to send, so no file budget applies to it.
#[test]
fn max_file_bytes_does_not_cap_stdin() {
    let output = run_with_stdin(
        &["value", "-", "plain_value", "--max-file-bytes", "1"],
        br#"{"plain_value":"from-stdin"}"#,
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"from-stdin");
}

#[test]
fn bare_relative_document_mutation_commits_and_reports_success() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.json"), r#"{"port":993}"#).unwrap();

    let output = afdata()
        .current_dir(dir.path())
        .args([
            "set",
            "config.json",
            "port",
            "1024",
            "--value-type",
            "number",
        ])
        .output()
        .expect("failed to run afdata");

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.json")).unwrap(),
        r#"{"port":1024}"#
    );
}

// ═══════════════════════════════════════════
// Shell authoring kit and scalar event emission
// ═══════════════════════════════════════════

#[test]
fn test_shell_bash_outputs_embedded_source() {
    let output = run(&["shell", "bash"]);
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, include_bytes!("../bash/afdata.sh"));
}

#[test]
fn test_shell_bash_rejects_output_to_override() {
    let output = run(&["shell", "bash", "--output-to", "stdout"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let response = json_stderr(&output);
    assert_eq!(response["error"]["code"], "cli_unregistered_combination");
}

#[test]
fn test_emit_log_uses_diagnostic_stream() {
    let output = run(&["emit", "log", "info", "building project"]);
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stdout.is_empty());
    let response = json_stderr(&output);
    assert_eq!(response["kind"], "log");
    assert_eq!(response["log"]["level"], "info");
    assert_eq!(response["log"]["message"], "building project");
    assert_eq!(response["trace"], serde_json::json!({}));
}

#[test]
fn test_emit_result_uses_stdout() {
    let output = run(&["emit", "result", "build complete"]);
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    let response = json_stdout(&output);
    assert_eq!(response["kind"], "result");
    assert_eq!(response["result"]["message"], "build complete");
}

#[test]
fn test_emit_error_uses_stderr_and_failure_status() {
    let output = run(&[
        "emit",
        "error",
        "build_failed",
        "build failed",
        "--hint",
        "inspect the child output",
        "--retryable",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let response = json_stderr(&output);
    assert_eq!(response["kind"], "error");
    assert_eq!(response["error"]["code"], "build_failed");
    assert_eq!(response["error"]["message"], "build failed");
    assert_eq!(response["error"]["hint"], "inspect the child output");
    assert_eq!(response["error"]["retryable"], true);
}

#[test]
fn test_emit_rejects_unknown_log_level_as_usage_error() {
    let output = run(&["emit", "log", "notice", "building project"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let response = json_stderr(&output);
    assert_eq!(response["error"]["code"], "cli_invalid_argument_value");
}

#[test]
fn test_emit_honors_unified_destination_and_output_format() {
    let output = run(&[
        "emit",
        "log",
        "warn",
        "build delayed",
        "--output-to",
        "stdout",
        "--output",
        "yaml",
    ]);
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).expect("YAML output must be UTF-8");
    assert!(text.contains("kind: \"log\""), "{text}");
    assert!(text.contains("level: \"warn\""), "{text}");
    assert!(text.contains("message: \"build delayed\""), "{text}");
}

#[test]
fn test_cli_parse_error_does_not_trust_unresolved_destination() {
    let output = run(&[
        "emit",
        "log",
        "notice",
        "building project",
        "--output-to",
        "stdout",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        json_stderr(&output)["error"]["code"],
        "cli_invalid_argument_value"
    );
}

// ═══════════════════════════════════════════
// D1: FILE|- is always the first positional; mutation rejects `-`
// ═══════════════════════════════════════════

#[test]
fn test_stdin_dash_defaults_to_json() {
    let output = run_with_stdin(&["get", "-"], br#"{"a":1}"#);
    assert!(output.status.success(), "{:?}", output);
    let response = json_stdout(&output);
    assert_eq!(response["result"]["format"], "json");
    assert_eq!(response["result"]["value"]["a"], 1);
}

#[test]
fn test_omitted_input_is_a_cli_parse_error_not_implicit_stdin() {
    // D1 killed the old implicit-stdin-when-omitted fallback: FILE is a
    // required positional now, so omitting it is a parse error (exit 2),
    // not a silent attempt to read stdin.
    let output = run(&["get"]);
    assert_eq!(output.status.code(), Some(2));
}

#[cfg(feature = "yaml")]
#[test]
fn test_input_format_override_applies_to_stdin() {
    let output = run_with_stdin(&["get", "--input-format", "yaml", "-"], b"a: 1\nb: 2\n");
    assert!(output.status.success(), "{:?}", output);
    let response = json_stdout(&output);
    assert_eq!(response["result"]["format"], "yaml");
    assert_eq!(response["result"]["value"]["a"], 1);
    assert_eq!(response["result"]["value"]["b"], 2);
}

#[test]
fn test_file_extension_inference_and_explicit_override() {
    let temp_dir = TempDir::new().unwrap();
    // No recognizable extension: detection fails unless overridden.
    let config_path = write_temp(
        &temp_dir,
        "extensionless.config",
        "{\"name\":\"explicit\"}\n",
    );

    let no_override = run(&["get", &config_path]);
    assert!(!no_override.status.success());
    assert!(no_override.stdout.is_empty());
    let response = json_stderr(&no_override);
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("cannot detect the format"))
    );

    let overridden = run(&["get", "--input-format", "json", &config_path, "name"]);
    assert!(overridden.status.success(), "{:?}", overridden);
    let response = json_stdout(&overridden);
    assert_eq!(response["result"]["value"], "explicit");

    // A real extension is still detected without an override.
    let json_path = write_temp(&temp_dir, "config.json", "{\"name\":\"by-extension\"}\n");
    let detected = run(&["get", &json_path, "name"]);
    assert!(detected.status.success());
    assert_eq!(json_stdout(&detected)["result"]["value"], "by-extension");
}

#[test]
fn test_mutation_rejects_dash_as_usage_error() {
    let output = run(&["set", "-", "a", "2"]);
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let response = json_stderr(&output);
    assert_eq!(response["error"]["code"], "document_usage_error");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("never read stdin"))
    );
}

// ═══════════════════════════════════════════
// D2: `show` is gone; `get` KEY is optional (unified `code:"document"`)
// ═══════════════════════════════════════════

#[test]
fn test_get_whole_document_and_targeted_share_document_code() {
    let whole = run_with_stdin(&["get", "-"], br#"{"a":1}"#);
    assert!(whole.status.success());
    let response = json_stdout(&whole);
    assert_eq!(response["result"]["code"], "document");
    assert!(response["result"].get("key").is_none());
    assert_eq!(response["result"]["value"]["a"], 1);

    let targeted = run_with_stdin(&["get", "-", "a"], br#"{"a":1}"#);
    assert!(targeted.status.success());
    let response = json_stdout(&targeted);
    assert_eq!(response["result"]["code"], "document");
    assert_eq!(response["result"]["key"], "a");
    assert_eq!(response["result"]["value"], 1);
}

// ═══════════════════════════════════════════
// R1: `value` failure -> stdout empty, error envelope on stderr
// ═══════════════════════════════════════════

#[test]
fn test_value_failure_stdout_is_always_empty() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(
        &temp_dir,
        "config.json",
        "{\"name\":\"hello\",\"a_secret\":\"x\",\"items\":[1]}\n",
    );

    // Path not found.
    let missing = run(&["value", &config_path, "nope"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty(), "{:?}", missing.stdout);
    assert_eq!(
        json_stderr(&missing)["error"]["code"],
        "document_path_not_found"
    );

    // Secret gate.
    let secret = run(&["value", &config_path, "a_secret"]);
    assert_eq!(secret.status.code(), Some(1));
    assert!(secret.stdout.is_empty());
    assert_eq!(
        json_stderr(&secret)["error"]["code"],
        "document_secret_redacted"
    );

    // Non-scalar.
    let non_scalar = run(&["value", &config_path, "items"]);
    assert_eq!(non_scalar.status.code(), Some(1));
    assert!(non_scalar.stdout.is_empty());
    assert_eq!(
        json_stderr(&non_scalar)["error"]["code"],
        "document_not_scalar"
    );
}

#[test]
fn test_value_scalar_bytes_on_stdout_no_envelope() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(
        &temp_dir,
        "config.json",
        "{\"name\":\"hello\",\"empty\":\"\",\"enabled\":true}\n",
    );

    let output = run(&["value", &config_path, "name"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hello");
    assert!(output.stderr.is_empty());

    let output = run(&["value", &config_path, "enabled"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"true");

    let output = run(&["value", &config_path, "empty"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"");
}

#[test]
fn test_paths_never_emits_an_address_it_cannot_read() {
    let temp_dir = TempDir::new().unwrap();

    // npm keys a package-lock's root package by the empty string. Nested, that
    // is addressable — `outer.` — and must round-trip.
    let nested = write_temp(
        &temp_dir,
        "nested.json",
        "{\"outer\":{\"\":{\"version\":\"1.2.3\"}}}\n",
    );
    let output = run(&["paths", &nested, "outer"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"outer.\n");
    // The emitted address reads back.
    let output = run(&["value", &nested, "outer..version"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1.2.3");

    // At the root there is no spelling: the address would be the empty string,
    // which names no path. Refuse rather than print a blank line the caller
    // will feed back and be refused on.
    let rooted = write_temp(&temp_dir, "rooted.json", "{\"\":1,\"a\":2}\n");
    let output = run(&["paths", &rooted]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("document_unaddressable_key"), "{stderr}");

    // `keys` makes no addressing claim, so it still lists the key.
    let output = run(&["keys", &rooted]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"\na\n");
}

#[test]
fn test_values_reads_many_paths_from_one_parse() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(
        &temp_dir,
        "config.json",
        "{\"name\":\"hello\",\"port\":8080,\"enabled\":true}\n",
    );

    // One line per requested path, in the order asked for — so a caller can
    // pair its own list against the output positionally.
    let output = run(&["values", &config_path, "name", "port", "enabled"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hello\n8080\ntrue\n");
    assert!(output.stderr.is_empty());

    // A single path is still a line, not `value`'s bare scalar: the framing is
    // the point of the command.
    let output = run(&["values", &config_path, "name"]);
    assert_eq!(output.stdout, b"hello\n");
}

#[test]
fn test_values_expands_a_wildcard_across_a_collection() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(
        &temp_dir,
        "config.json",
        "{\"pkgs\":[{\"name\":\"a\"},{\"name\":\"b\"},{\"name\":\"c\"}]}\n",
    );

    // Reading one field across a collection used to be: enumerate the children,
    // append the field to each address with `sed`, then read them back.
    let output = run(&["values", &config_path, "pkgs.*.name"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\nb\nc\n");

    // Objects fan out by key, in document order.
    let object_path = write_temp(
        &temp_dir,
        "object.json",
        "{\"svc\":{\"one\":{\"port\":1},\"two\":{\"port\":2}}}\n",
    );
    let output = run(&["values", &object_path, "svc.*.port"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n2\n");

    // A wildcard needs a container under it.
    let output = run(&["values", &config_path, "pkgs.0.name.*"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("document_not_container"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_a_star_key_is_still_reachable_beside_the_wildcard() {
    let temp_dir = TempDir::new().unwrap();
    // `*` is a legal key; reserving the bare form must not strand it.
    let config_path = write_temp(
        &temp_dir,
        "config.json",
        "{\"a\":{\"*\":1},\"b\":{\"*\":2}}\n",
    );

    // Escaped, it is a key — even alongside a wildcard in the same path.
    let output = run(&["values", &config_path, r"*.\*"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1\n2\n");

    // `paths` emits the escaped spelling, and that spelling reads back.
    let output = run(&["paths", &config_path, "a"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"a.\\*\n");
    let output = run(&["value", &config_path, r"a.\*"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"1");

    // A bare `*` where nothing can expand it is refused, not read as a key.
    let output = run(&["value", &config_path, "a.*"]);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn test_values_refuses_a_value_it_cannot_frame() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(
        &temp_dir,
        "config.json",
        "{\"one\":\"a\\nb\",\"two\":\"plain\"}\n",
    );

    // Emitting this would put two lines where the caller expects one, and
    // every later value would be paired with the wrong path. Refuse instead.
    let output = run(&["values", &config_path, "one", "two"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("document_multiline_value"), "{stderr}");

    // The same value is readable one at a time, where there is no framing.
    let output = run(&["value", &config_path, "one"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\nb");
}

#[test]
fn test_values_keeps_the_guarantees_value_makes() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(
        &temp_dir,
        "config.json",
        "{\"api_key_secret\":\"sk-live\",\"name\":\"hello\"}\n",
    );

    // A secret-named leaf is gated exactly as `value` gates it.
    let output = run(&["values", &config_path, "api_key_secret", "name"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("sk-live"));

    let output = run(&[
        "values",
        &config_path,
        "api_key_secret",
        "name",
        "--reveal-secret",
    ]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"sk-live\nhello\n");

    // `--default` covers a missing path, per path.
    let output = run(&["values", &config_path, "name", "absent", "--default", "-"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hello\n-\n");
}

#[cfg(feature = "yaml")]
#[test]
fn test_value_non_finite_float_errors() {
    let output = run_with_stdin(&["value", "--input-format", "yaml", "-", "f"], b"f: .inf\n");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        json_stderr(&output)["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("non-finite"))
    );
}

#[test]
fn test_value_secret_gate_requires_reveal_flag() {
    let output = run_with_stdin(&["value", "-", "a.b_secret"], br#"{"a":{"b_secret":"x"}}"#);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let response = json_stderr(&output);
    assert_eq!(response["error"]["code"], "document_secret_redacted");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("--reveal-secret"))
    );

    let output = run_with_stdin(
        &["value", "-", "a.b_secret", "--reveal-secret"],
        br#"{"a":{"b_secret":"x"}}"#,
    );
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(output.stdout, b"x");
}

#[test]
fn test_secret_marks_the_whole_subtree_not_just_the_leaf() {
    // A `_secret` name marks everything beneath it. Judging the leaf alone let
    // a caller step past the marked node and read the subtree in the clear —
    // every test here had only ever put `_secret` on a leaf.
    const DOC: &[u8] = br#"{"credentials_secret":{"password":"hunter2"}}"#;

    let output = run_with_stdin(&["get", "-", "credentials_secret.password"], DOC);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(json_stdout(&output)["result"]["value"], "***");

    let output = run_with_stdin(&["value", "-", "credentials_secret.password"], DOC);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        json_stderr(&output)["error"]["code"],
        "document_secret_redacted"
    );

    // Explicit consent still reaches it.
    let output = run_with_stdin(
        &[
            "value",
            "-",
            "credentials_secret.password",
            "--reveal-secret",
        ],
        DOC,
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"hunter2");
}

#[test]
fn test_document_errors_never_echo_document_content() {
    // An error event is what an agent routinely writes to a log, so a message
    // that quotes the document is a leak on the most-travelled path.
    let output = run_with_stdin(
        &["get", "-", "next"],
        b"api_key_secret = \"sk-live-SUPERSECRET\nnext = 1\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let message = json_stderr(&output)["error"]["message"]
        .as_str()
        .expect("message")
        .to_string();
    assert!(!message.contains("sk-live-SUPERSECRET"), "{message}");
    // The location survives — it is what makes the error actionable.
    assert!(message.contains("line 1"), "{message}");

    let output = run_with_stdin(
        &["get", "-", "token_secret.inner"],
        br#"{"token_secret":"sk-live-LEAKME"}"#,
    );
    assert_eq!(output.status.code(), Some(1));
    let response = json_stderr(&output);
    assert_eq!(response["error"]["code"], "document_type_mismatch");
    let message = response["error"]["message"].as_str().expect("message");
    assert!(!message.contains("sk-live-LEAKME"), "{message}");
    assert!(message.contains("string"), "{message}");
}

#[test]
fn test_value_secret_name_gate_and_reveal() {
    let output = run_with_stdin(
        &["value", "-", "PASSWORD", "--secret-name", "PASSWORD"],
        br#"{"PASSWORD":"hunter2"}"#,
    );
    assert_eq!(output.status.code(), Some(1));

    let output = run_with_stdin(
        &[
            "value",
            "-",
            "PASSWORD",
            "--secret-name",
            "PASSWORD",
            "--reveal-secret",
        ],
        br#"{"PASSWORD":"hunter2"}"#,
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hunter2");
}

// ═══════════════════════════════════════════
// §2: `value --default`
// ═══════════════════════════════════════════

#[test]
fn test_value_default_covers_path_absent_and_null_not_empty_string() {
    let output = run_with_stdin(
        &["value", "-", "missing", "--default", "fallback"],
        br#"{"present":null,"empty":""}"#,
    );
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(output.stdout, b"fallback");

    let output = run_with_stdin(
        &["value", "-", "present", "--default", "fallback"],
        br#"{"present":null,"empty":""}"#,
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"fallback");

    // Empty string is a real value; it does not trigger the default.
    let output = run_with_stdin(
        &["value", "-", "empty", "--default", "fallback"],
        br#"{"present":null,"empty":""}"#,
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"");

    // A real parse error still errors even with --default.
    let output = run_with_stdin(&["value", "-", "k", "--default", "fallback"], b"not-json");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

// ═══════════════════════════════════════════
// §1: `paths`/`keys`
// ═══════════════════════════════════════════

#[test]
fn test_paths_and_keys_object_array_and_top_level() {
    let stdin = br#"{"extra":{"tools":[{"slug":"a"},{"slug":"b"}]},"deps":{"foo.bar":"1"}}"#;

    let paths = run_with_stdin(&["paths", "-", "extra.tools"], stdin);
    assert!(paths.status.success(), "{:?}", paths);
    assert_eq!(
        String::from_utf8_lossy(&paths.stdout),
        "extra.tools.0\nextra.tools.1\n"
    );

    let keys = run_with_stdin(&["keys", "-", "extra.tools"], stdin);
    assert!(keys.status.success());
    assert_eq!(String::from_utf8_lossy(&keys.stdout), "0\n1\n");

    // Escaped dotted key: `paths` re-escapes, `keys` emits the raw name.
    let paths_dotted = run_with_stdin(&["paths", "-", "deps"], stdin);
    assert!(paths_dotted.status.success());
    assert_eq!(
        String::from_utf8_lossy(&paths_dotted.stdout),
        "deps.foo\\.bar\n"
    );
    let keys_dotted = run_with_stdin(&["keys", "-", "deps"], stdin);
    assert!(keys_dotted.status.success());
    assert_eq!(String::from_utf8_lossy(&keys_dotted.stdout), "foo.bar\n");

    // Top-level (KEY omitted).
    let top = run_with_stdin(&["keys", "-"], stdin);
    assert!(top.status.success());
    assert_eq!(String::from_utf8_lossy(&top.stdout), "deps\nextra\n");

    // A key holding a space needs no escaping — only `.` and `\` do — and each
    // line stays one whole address, so `while IFS= read -r` reads it back
    // unsplit. Asserted because the line format is what shell callers parse.
    let spaced = br#"{"deps":{"two words":1,"a.b":2}}"#;
    let paths_spaced = run_with_stdin(&["paths", "-", "deps"], spaced);
    assert!(paths_spaced.status.success());
    assert_eq!(
        String::from_utf8_lossy(&paths_spaced.stdout),
        "deps.a\\.b\ndeps.two words\n"
    );
    let keys_spaced = run_with_stdin(&["keys", "-", "deps"], spaced);
    assert!(keys_spaced.status.success());
    assert_eq!(
        String::from_utf8_lossy(&keys_spaced.stdout),
        "a.b\ntwo words\n"
    );

    // And the address `paths` printed round-trips back through a read.
    let read_back = run_with_stdin(&["value", "-", "deps.two words"], spaced);
    assert!(read_back.status.success(), "{read_back:?}");
    assert_eq!(String::from_utf8_lossy(&read_back.stdout), "1");
}

#[test]
fn test_paths_and_keys_empty_container_and_scalar_error() {
    let empty = run_with_stdin(&["paths", "-", "empty"], br#"{"empty":{}}"#);
    assert!(empty.status.success());
    assert!(empty.stdout.is_empty());

    let scalar = run_with_stdin(&["paths", "-", "name"], br#"{"name":"x"}"#);
    assert_eq!(scalar.status.code(), Some(1));
    assert!(scalar.stdout.is_empty());
    assert_eq!(
        json_stderr(&scalar)["error"]["code"],
        "document_not_container"
    );
}

#[test]
fn test_paths_and_keys_missing_ok_and_null_separator() {
    let stdin = br#"{"a":1}"#;

    let missing = run_with_stdin(&["keys", "-", "nope"], stdin);
    assert_eq!(missing.status.code(), Some(1));

    let missing_ok = run_with_stdin(&["keys", "-", "nope", "--missing-ok"], stdin);
    assert!(missing_ok.status.success());
    assert!(missing_ok.stdout.is_empty());

    // --missing-ok does not swallow a real parse error.
    let parse_error = run_with_stdin(&["keys", "-", "nope", "--missing-ok"], b"not-json");
    assert!(!parse_error.status.success());

    let null_sep = run_with_stdin(&["paths", "-", "--null"], br#"{"a":1,"b":2}"#);
    assert!(null_sep.status.success(), "{:?}", null_sep);
    assert_eq!(null_sep.stdout, b"a\0b\0");

    // A key containing a newline needs NUL framing; the default refuses it.
    let newline_key = run_with_stdin(&["paths", "-", "--null"], br#"{"a":1,"b\nc":2}"#);
    assert!(newline_key.status.success(), "{:?}", newline_key);
    assert_eq!(newline_key.stdout, b"a\0b\nc\0");

    // `-0` is not an alias: AFDATA spells its flags out in full, so a
    // number-shaped token is a value — here the key `-0`, which is absent.
    let short_form = run_with_stdin(&["paths", "-", "-0"], br#"{"a":1}"#);
    assert_eq!(short_form.status.code(), Some(1));
    assert!(short_form.stdout.is_empty());
    assert_eq!(
        json_stderr(&short_form)["error"]["code"],
        "document_path_not_found"
    );
}

#[test]
fn test_paths_and_keys_reject_explicit_output_json() {
    let output = run_with_stdin(&["paths", "-", "--output", "json"], br#"{"a":1}"#);
    assert_eq!(output.status.code(), Some(2));
    // The implicit default (no --output at all) is fine.
    let default_output = run_with_stdin(&["paths", "-"], br#"{"a":1}"#);
    assert!(default_output.status.success());
}

// ═══════════════════════════════════════════
// §3: bare VALUE is always string; --value-type; heterogeneous guard
// ═══════════════════════════════════════════

#[test]
fn test_bare_value_is_zero_coercion_string() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", "{}\n");

    let output = run(&["set", &config_path, "code", "007"]);
    assert!(output.status.success(), "{:?}", output);
    let value = run(&["value", &config_path, "code"]);
    assert_eq!(value.stdout, b"007");
}

#[test]
fn test_bare_value_overwriting_existing_scalar_of_different_type_is_guarded() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", r#"{"port":8080}"#);

    let guarded = run(&["set", &config_path, "port", "9090"]);
    assert_eq!(guarded.status.code(), Some(2));
    assert!(guarded.stdout.is_empty());
    let response = json_stderr(&guarded);
    assert_eq!(response["error"]["code"], "document_usage_error");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("--value-type number") && m.contains("--value-type string"))
    );
    // The file is untouched.
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        r#"{"port":8080}"#
    );

    // Escape hatch 1: keep the type.
    let kept = run(&[
        "set",
        &config_path,
        "port",
        "9090",
        "--value-type",
        "number",
    ]);
    assert!(kept.status.success(), "{:?}", kept);
    let value = run(&["value", &config_path, "port"]);
    assert_eq!(value.stdout, b"9090");

    // Escape hatch 2: explicit string conversion.
    let converted = run(&[
        "set",
        &config_path,
        "port",
        "9090",
        "--value-type",
        "string",
    ]);
    assert!(converted.status.success());

    // A brand-new key never needs --value-type.
    let new_key = run(&["set", &config_path, "brand_new", "hello"]);
    assert!(new_key.status.success(), "{:?}", new_key);
}

#[test]
fn test_value_type_null_bool_number_and_json() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", "{}\n");

    let null = run(&["set", &config_path, "a", "--value-type", "null"]);
    assert!(null.status.success(), "{:?}", null);
    let value = run(&["value", &config_path, "a"]);
    assert_eq!(value.status.code(), Some(1));
    assert!(value.stdout.is_empty());
    assert_eq!(json_stderr(&value)["error"]["code"], "document_null_value");
    let defaulted = run(&["value", &config_path, "a", "--default", "fallback"]);
    assert!(defaulted.status.success(), "{:?}", defaulted);
    assert_eq!(defaulted.stdout, b"fallback");

    // A bare string over null gets an actionable null-specific hint.
    let guarded = run(&["set", &config_path, "a", "replacement"]);
    assert_eq!(guarded.status.code(), Some(2));
    assert!(
        json_stderr(&guarded)["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("--value-type null without VALUE"))
    );

    // --value-type null rejects an accompanying VALUE.
    let extra = run(&["set", &config_path, "a", "x", "--value-type", "null"]);
    assert!(!extra.status.success());

    let boolean = run(&["set", &config_path, "b", "yes", "--value-type", "bool"]);
    assert!(boolean.status.success());
    assert_eq!(run(&["value", &config_path, "b"]).stdout, b"true");

    let array = run(&["set", &config_path, "c", "[1,2,3]", "--value-type", "json"]);
    assert!(array.status.success(), "{:?}", array);
    let response = json_stdout(&run(&["get", &config_path, "c"]));
    assert_eq!(response["result"]["value"], serde_json::json!([1, 2, 3]));

    // --value-type json is the only entry point for an exact-type scalar:
    // the string "8080", not the number.
    let exact_string = run(&["set", &config_path, "d", "\"8080\"", "--value-type", "json"]);
    assert!(exact_string.status.success());
    assert_eq!(run(&["value", &config_path, "d"]).stdout, b"8080");
    let response = json_stdout(&run(&["get", &config_path, "d"]));
    assert!(response["result"]["value"].is_string());
}

#[test]
fn test_add_field_value_is_always_string() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", "{\"items\":[]}\n");
    let added = run(&[
        "add",
        &config_path,
        "items",
        "a",
        "--slug-field",
        "id",
        "count=007",
    ]);
    assert!(added.status.success(), "{:?}", added);
    let response = json_stdout(&run(&[
        "get",
        &config_path,
        "items.a.count",
        "--slug-field",
        "id",
    ]));
    assert_eq!(response["result"]["value"], "007");
}

#[test]
fn test_malformed_field_value_is_usage_error() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", "{\"items\":[]}\n");
    let output = run(&[
        "add",
        &config_path,
        "items",
        "a",
        "--slug-field",
        "id",
        "not-a-pair",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        json_stderr(&output)["error"]["code"],
        "document_usage_error"
    );
}

/// An empty slug or slug field would write an element keyed `""` that no read
/// can address; both are usage errors and the file is left untouched.
#[test]
fn add_rejects_an_empty_slug_or_slug_field() {
    let temp_dir = TempDir::new().unwrap();
    let source = "{\"items\":[]}\n";
    let config_path = write_temp(&temp_dir, "config.json", source);
    for (slug, slug_field, message) in [
        ("b", "", "--slug-field must not be empty"),
        ("", "id", "SLUG must not be empty"),
    ] {
        let output = run(&[
            "add",
            &config_path,
            "items",
            slug,
            "--slug-field",
            slug_field,
        ]);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stdout.is_empty());
        let error = json_stderr(&output);
        assert_eq!(error["error"]["code"], "document_usage_error");
        assert_eq!(error["error"]["message"], message);
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), source);
    }
}

// ═══════════════════════════════════════════
// §4: number literal fidelity (get/value are literal- and digit-faithful)
// ═══════════════════════════════════════════

#[test]
fn test_number_fidelity_oversized_integer_and_high_precision_float() {
    let temp_dir = TempDir::new().unwrap();
    let huge = "123456789012345678901234567890";
    let precise = "0.1000000000000000055511151231257827";
    let config_path = write_temp(
        &temp_dir,
        "config.json",
        &format!("{{\"huge\":{huge},\"precise\":{precise},\"max_u64\":18446744073709551615}}\n"),
    );

    let value_huge = run(&["value", &config_path, "huge"]);
    assert!(value_huge.status.success(), "{:?}", value_huge);
    assert_eq!(value_huge.stdout, huge.as_bytes());

    let value_precise = run(&["value", &config_path, "precise"]);
    assert!(value_precise.status.success());
    assert_eq!(value_precise.stdout, precise.as_bytes());

    // Regression: u64::MAX was already faithful before this fix.
    let value_max = run(&["value", &config_path, "max_u64"]);
    assert_eq!(value_max.stdout, b"18446744073709551615");

    let get_huge = json_stdout(&run(&["get", &config_path, "huge"]));
    assert_eq!(get_huge["result"]["value"].to_string(), huge);
}

#[test]
fn test_number_fidelity_set_value_type_number_preserves_literal() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", "{}\n");
    let huge = "123456789012345678901234567890";
    let set = run(&["set", &config_path, "n", huge, "--value-type", "number"]);
    assert!(set.status.success(), "{:?}", set);
    let value = run(&["value", &config_path, "n"]);
    assert_eq!(value.stdout, huge.as_bytes());
    // On-disk bytes preserve the literal too.
    let on_disk = std::fs::read_to_string(&config_path).unwrap();
    assert!(on_disk.contains(huge), "{on_disk}");
}

// ═══════════════════════════════════════════
// R3: mutation results carry `path`
// ═══════════════════════════════════════════

#[test]
fn test_mutation_results_carry_path() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", "{\"a\":1,\"items\":[]}\n");

    // A brand-new key needs no --value-type (the heterogeneous-overwrite
    // guard only applies to an *existing* differently-kinded scalar).
    let set = json_stdout(&run(&["set", &config_path, "brand_new", "2"]));
    assert_eq!(set["result"]["path"], config_path);

    let added = json_stdout(&run(&[
        "add",
        &config_path,
        "items",
        "x",
        "--slug-field",
        "id",
    ]));
    assert_eq!(added["result"]["path"], config_path);

    let removed = json_stdout(&run(&[
        "remove",
        &config_path,
        "items",
        "x",
        "--slug-field",
        "id",
    ]));
    assert_eq!(removed["result"]["path"], config_path);

    let unset = json_stdout(&run(&["unset", &config_path, "a"]));
    assert_eq!(unset["result"]["path"], config_path);
}

// ═══════════════════════════════════════════
// R4: mutation idempotency stays error-by-default
// ═══════════════════════════════════════════

#[test]
fn test_idempotency_add_existing_remove_absent_unset_absent_are_errors() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", "{\"items\":[{\"id\":\"a\"}]}\n");

    let duplicate = run(&["add", &config_path, "items", "a", "--slug-field", "id"]);
    assert_eq!(duplicate.status.code(), Some(1));
    assert!(duplicate.stdout.is_empty());
    assert_eq!(
        json_stderr(&duplicate)["error"]["code"],
        "document_slug_exists"
    );

    let missing_remove = run(&[
        "remove",
        &config_path,
        "items",
        "nope",
        "--slug-field",
        "id",
    ]);
    assert_eq!(missing_remove.status.code(), Some(1));
    assert_eq!(
        json_stderr(&missing_remove)["error"]["code"],
        "document_slug_not_found"
    );

    let missing_unset = run(&["unset", &config_path, "nope"]);
    assert_eq!(missing_unset.status.code(), Some(1));
    assert_eq!(
        json_stderr(&missing_unset)["error"]["code"],
        "document_path_not_found"
    );
}

// ═══════════════════════════════════════════
// R2: error code taxonomy and exit codes
// ═══════════════════════════════════════════

#[test]
fn test_bad_input_format_is_usage_error_exit_2() {
    let output = run_with_stdin(&["get", "--input-format", "xml", "-"], br#"{}"#);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        json_stderr(&output)["error"]["code"],
        "cli_invalid_argument_value"
    );
}

#[test]
fn test_type_mismatch_and_path_not_found_codes() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", "{\"a\":{\"b\":1}}\n");

    let not_found = run(&["get", &config_path, "a.nope"]);
    assert_eq!(not_found.status.code(), Some(1));
    assert!(not_found.stdout.is_empty());
    assert_eq!(
        json_stderr(&not_found)["error"]["code"],
        "document_path_not_found"
    );

    let type_mismatch = run(&["get", &config_path, "a.b.c"]);
    assert_eq!(type_mismatch.status.code(), Some(1));
    assert!(type_mismatch.stdout.is_empty());
    assert_eq!(
        json_stderr(&type_mismatch)["error"]["code"],
        "document_type_mismatch"
    );
}

// ═══════════════════════════════════════════
// --output-to <split|stdout|stderr>: event-stream routing contract
// ═══════════════════════════════════════════

#[test]
fn test_split_default_sends_error_to_stderr_stdout_empty() {
    // The default (split) routes `kind:"result"` to stdout and `kind:"error"`
    // to stderr, so a failed `get` writes nothing to stdout.
    let output = run_with_stdin(&["get", "-", "nope"], br#"{"a":1}"#);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert_eq!(
        json_stderr(&output)["error"]["code"],
        "document_path_not_found"
    );
}

#[test]
fn test_output_to_stdout_unifies_error_onto_stdout() {
    // Unified stdout mode: even the error envelope lands on stdout, and stderr
    // stays empty.
    let output = run_with_stdin(
        &["get", "-", "nope", "--output-to", "stdout"],
        br#"{"a":1}"#,
    );
    assert!(!output.status.success());
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    assert_eq!(
        json_stdout(&output)["error"]["code"],
        "document_path_not_found"
    );

    // Same for a mutation usage error.
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", r#"{"port":8080}"#);
    let set = run(&["set", &config_path, "port", "9090", "--output-to", "stdout"]);
    assert_eq!(set.status.code(), Some(2));
    assert!(set.stderr.is_empty(), "{:?}", set.stderr);
    assert_eq!(json_stdout(&set)["error"]["code"], "document_usage_error");
}

#[test]
fn test_output_to_stderr_unifies_result_onto_stderr() {
    // Unified stderr mode: a successful result envelope goes to stderr; stdout
    // stays empty.
    let output = run_with_stdin(&["get", "-", "a", "--output-to", "stderr"], br#"{"a":1}"#);
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    let response = json_stderr(&output);
    assert_eq!(response["result"]["code"], "document");
    assert_eq!(response["result"]["value"], 1);
}

#[test]
fn test_set_error_survives_stdout_redirected_to_null() {
    // The "`set >/dev/null` no longer swallows errors" guarantee: with stdout
    // discarded, the split default still surfaces the error envelope on stderr.
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", r#"{"port":8080}"#);
    let output = afdata()
        .args(["set", &config_path, "port", "9090"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to run afdata");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        json_stderr(&output)["error"]["code"],
        "document_usage_error"
    );
    // The file is untouched despite the discarded stdout.
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        r#"{"port":8080}"#
    );
}

#[test]
fn test_raw_scalar_commands_reject_non_default_output_to() {
    // value/paths/keys print a raw scalar, not an event stream, so a non-default
    // --output-to is a usage error (exit 2) whose message names the trio.
    for command in ["value", "paths", "keys"] {
        for mode in ["stdout", "stderr"] {
            let output = run_with_stdin(&[command, "--output-to", mode, "-", "a"], br#"{"a":1}"#);
            assert_eq!(
                output.status.code(),
                Some(2),
                "{command} --output-to {mode}"
            );
            assert!(output.stdout.is_empty());
            let response = json_stderr(&output);
            assert!(
                response["error"]["code"]
                    .as_str()
                    .is_some_and(|code| code.starts_with("cli_")),
                "{command} --output-to {mode}: {response}"
            );
        }
    }
}

#[test]
fn test_output_to_unknown_value_is_usage_error() {
    let output = run_with_stdin(&["get", "-", "a", "--output-to", "bogus"], br#"{"a":1}"#);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        json_stderr(&output)["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("expected one of split, stdout, stderr")),
        "{:?}",
        output.stderr
    );
}

// ═══════════════════════════════════════════
// R5: parse-error presentation honors --output
// ═══════════════════════════════════════════

#[test]
fn test_render_parse_error_honors_output_format() {
    let output = run_with_stdin(&["render", "-", "--output", "yaml"], b"not-json");
    assert!(!output.status.success());
    // Under the default split routing, the error envelope goes to stderr and
    // stdout stays empty.
    assert!(output.stdout.is_empty());
    let text = String::from_utf8_lossy(&output.stderr);
    // YAML rendering, not a hardcoded JSON blob.
    assert!(text.contains("kind:"), "{text}");
    assert!(!text.trim_start().starts_with('{'), "{text}");
}

// ═══════════════════════════════════════════
// D6: `render` accepts `--secret-name`, matching `get`'s redaction surface
// ═══════════════════════════════════════════

#[test]
fn test_render_secret_name_matches_get_redaction() {
    let output = run_with_stdin(
        &[
            "render",
            "-",
            "--secret-name",
            "PASSWORD",
            "--output",
            "json",
        ],
        br#"{"PASSWORD":"hunter2","api_key_secret":"sk-live","ok":true}"#,
    );
    assert!(output.status.success(), "{:?}", output);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    // Default `_secret` suffix redaction still applies...
    assert_eq!(value["api_key_secret"], "***");
    // ...and --secret-name extends it to a field with no suffix.
    assert_eq!(value["PASSWORD"], "***");
    assert_eq!(value["ok"], true);
}

// ═══════════════════════════════════════════
// R6: `lint` accepts document formats
// ═══════════════════════════════════════════

#[cfg(feature = "toml")]
#[test]
fn test_lint_accepts_toml_document_via_extension_and_override() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.toml", "language_bcp47 = \"zh_CN\"\n");
    let by_extension = run(&["lint", &config_path]);
    assert_eq!(by_extension.status.code(), Some(1));
    assert!(by_extension.stdout.is_empty());
    let findings = json_stderr(&by_extension)["error"]["findings"].clone();
    assert_eq!(findings[0]["rule_id"], "suffix_type_mismatch");

    let by_override = run_with_stdin(
        &["lint", "--input-format", "toml", "-"],
        b"language_bcp47 = \"zh_CN\"\n",
    );
    assert_eq!(by_override.status.code(), Some(1));

    // JSON/JSONL default behavior is unchanged.
    let jsonl = run_with_stdin(&["lint", "-"], b"{\"a\":1}\n{\"b\":2}\n");
    assert!(jsonl.status.success(), "{:?}", jsonl);
}

// ═══════════════════════════════════════════
// D7: `validate --per-event` (renamed from `--event`)
// ═══════════════════════════════════════════

#[test]
fn test_validate_per_event_flag() {
    let valid = br#"{"kind":"log","log":{"event":"startup"},"trace":{}}"#;
    let output = run_with_stdin(&["validate", "-", "--strict", "--per-event"], valid);
    assert!(output.status.success(), "{:?}", output);
}

// ═══════════════════════════════════════════
// D6: per-command flags — an inapplicable combination is a parse error
// ═══════════════════════════════════════════

#[test]
fn test_flags_are_per_command_not_global() {
    // `lint` has no --secret-name.
    let output = run_with_stdin(&["lint", "-", "--secret-name", "X"], b"{}");
    assert_eq!(output.status.code(), Some(2));

    // `validate`/`render` have no --input-format.
    let output = run_with_stdin(&["validate", "-", "--input-format", "toml"], b"{}");
    assert_eq!(output.status.code(), Some(2));
}

// ═══════════════════════════════════════════
// Mutation: TOML source preservation, JSON/YAML keyed lists, nested
// dotted prefixes, missing-key insertion
// ═══════════════════════════════════════════

#[cfg(feature = "toml")]
#[test]
fn test_set_preserves_toml_formatting() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(
        &temp_dir,
        "config.toml",
        "# leading comment\nhost = \"example.com\"\nport = 993 # inline comment\n",
    );

    // `port` already exists as a number; keep its type explicitly (§3
    // heterogeneous-overwrite guard).
    let output = run(&[
        "set",
        &config_path,
        "port",
        "1024",
        "--value-type",
        "number",
    ]);
    assert!(output.status.success(), "{:?}", output);
    let response = json_stdout(&output);
    assert_eq!(response["result"]["code"], "document_set");
    assert_eq!(response["result"]["format"], "toml");

    let after = std::fs::read_to_string(&config_path).unwrap();
    assert!(after.contains("# leading comment"));
    assert!(after.contains("port = 1024 # inline comment"));
}

#[cfg(feature = "toml")]
#[test]
fn test_set_missing_key_toml_and_yaml_preserve_comments() {
    let temp_dir = TempDir::new().unwrap();

    let toml_path = write_temp(
        &temp_dir,
        "c.toml",
        "a = 1\n\n[srv]\nhost = \"x\"  # keep\n",
    );
    // `srv.port` is a brand-new key, so no guard fires; --value-type number
    // is passed anyway to keep this test's original intent (a numeric
    // value), not because it is required.
    let out = run(&[
        "set",
        &toml_path,
        "srv.port",
        "8080",
        "--value-type",
        "number",
    ]);
    assert!(out.status.success(), "{:?}", out);
    let after = std::fs::read_to_string(&toml_path).unwrap();
    assert!(after.contains("# keep"), "comment preserved: {after}");
    assert!(after.contains("port = 8080"), "new key inserted: {after}");

    #[cfg(feature = "yaml")]
    {
        let yaml_path = write_temp(&temp_dir, "c.yaml", "a: 1\nsrv:\n  host: x\n");
        let out = run(&[
            "set",
            &yaml_path,
            "srv.port",
            "8080",
            "--value-type",
            "number",
        ]);
        assert!(out.status.success(), "{:?}", out);
        let after = std::fs::read_to_string(&yaml_path).unwrap();
        assert_eq!(after, "a: 1\nsrv:\n  host: x\n  port: 8080\n");
    }
}

#[test]
fn test_json_keyed_collection_edits_preserve_document() {
    let temp_dir = TempDir::new().unwrap();
    let source =
        "{\n  \"items\": [\n    {\"id\": \"a\", \"name\": \"A\"}\n  ],\n  \"keep\": 1e+3\n}\n";
    let config_path = write_temp(&temp_dir, "config.json", source);

    let added = run(&[
        "add",
        &config_path,
        "items",
        "b",
        "--slug-field",
        "id",
        "name=B",
    ]);
    assert!(added.status.success(), "{:?}", added);
    let after_add = std::fs::read_to_string(&config_path).unwrap();
    assert!(after_add.contains("1e+3"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&after_add).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let removed = run(&["remove", &config_path, "items", "b", "--slug-field", "id"]);
    assert!(removed.status.success(), "{:?}", removed);
    let after_remove = std::fs::read_to_string(&config_path).unwrap();
    assert!(after_remove.contains("1e+3"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&after_remove).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn test_json_root_keyed_collection_edits() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "root.json", "[]\n");

    let added = run(&["add", &config_path, "", "b", "--slug-field", "id", "name=B"]);
    assert!(added.status.success(), "{added:?}");
    let after_add: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(after_add.as_array().unwrap().len(), 1);
    assert_eq!(after_add[0]["id"], "b");
    assert_eq!(after_add[0]["name"], "B");

    let removed = run(&["remove", &config_path, "", "b", "--slug-field", "id"]);
    assert!(removed.status.success(), "{removed:?}");
    assert_eq!(std::fs::read_to_string(config_path).unwrap(), "[]\n");
}

#[cfg(feature = "yaml")]
#[test]
fn test_yaml_root_keyed_collection_edits() {
    let temp_dir = TempDir::new().unwrap();
    let source = "# keep\n- id: a\n  name: 'A'\n";
    let config_path = write_temp(&temp_dir, "root.yaml", source);

    let added = run(&["add", &config_path, "", "b", "--slug-field", "id", "name=B"]);
    assert!(added.status.success(), "{added:?}");
    let after_add = std::fs::read_to_string(&config_path).unwrap();
    let parsed = Format::Yaml.load(&after_add).unwrap();
    assert_eq!(parsed.as_array().unwrap().len(), 2);
    assert!(after_add.contains("# keep"));
    assert!(after_add.contains("name: 'A'"));

    let removed = run(&["remove", &config_path, "", "b", "--slug-field", "id"]);
    assert!(removed.status.success(), "{removed:?}");
    assert_eq!(std::fs::read_to_string(config_path).unwrap(), source);
}

#[test]
fn test_keyed_slug_with_a_dot_uses_the_normal_path_escape() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "dotted-slug.json", "{\"items\":[]}\n");

    let added = run(&[
        "add",
        &config_path,
        "items",
        "case.add",
        "--slug-field",
        "id",
        "name=value",
    ]);
    assert!(added.status.success(), "{added:?}");

    let read = run(&[
        "value",
        &config_path,
        r"items.case\.add.name",
        "--slug-field",
        "id",
    ]);
    assert!(read.status.success(), "{read:?}");
    assert_eq!(read.stdout, b"value");

    let removed = run(&[
        "remove",
        &config_path,
        "items",
        "case.add",
        "--slug-field",
        "id",
    ]);
    assert!(removed.status.success(), "{removed:?}");
    assert_eq!(
        std::fs::read_to_string(config_path).unwrap(),
        "{\"items\":[]}\n"
    );
}

#[test]
fn test_remove_refuses_duplicate_slug_without_changing_source() {
    let temp_dir = TempDir::new().unwrap();
    let cases = [
        (
            "duplicates.json",
            "{\n  \"items\": [\n    {\"id\": \"same\", \"secret\": \"first\"},\n    {\"id\": \"same\", \"secret\": \"second\"}\n  ]\n}\n",
        ),
        #[cfg(feature = "yaml")]
        (
            "duplicates.yaml",
            "items:\n  - id: same\n    secret: first\n  - id: same\n    secret: second\n",
        ),
    ];

    for (name, source) in cases {
        let config_path = write_temp(&temp_dir, name, source);
        let removed = run(&[
            "remove",
            &config_path,
            "items",
            "same",
            "--slug-field",
            "id",
        ]);

        assert_eq!(removed.status.code(), Some(1), "{name}: {removed:?}");
        let event = json_stderr(&removed);
        assert_eq!(event["error"]["code"], "document_ambiguous_match");
        assert_eq!(
            event["error"]["message"],
            "segment `same` matches 2 elements of `items` at indices 0, 1"
        );
        assert!(!String::from_utf8_lossy(&removed.stderr).contains("first"));
        assert!(!String::from_utf8_lossy(&removed.stderr).contains("second"));
        assert_eq!(std::fs::read_to_string(config_path).unwrap(), source);
    }
}

#[test]
fn test_keyed_remove_reports_a_non_array_as_a_type_error() {
    let temp_dir = TempDir::new().unwrap();
    let source = "{\"items\":\"not an array\"}\n";
    let config_path = write_temp(&temp_dir, "scalar-items.json", source);

    let removed = run(&[
        "remove",
        &config_path,
        "items",
        "same",
        "--slug-field",
        "id",
    ]);

    assert_eq!(removed.status.code(), Some(1), "{removed:?}");
    assert_eq!(
        json_stderr(&removed)["error"]["code"],
        "document_type_mismatch"
    );
    assert_eq!(std::fs::read_to_string(config_path).unwrap(), source);
}

#[test]
fn test_keyed_edits_on_nested_dotted_prefix() {
    let temp_dir = TempDir::new().unwrap();
    let source = "{\n  \"cfg\": {\n    \"users\": [\n      {\"uid\": \"a\", \"role\": \"admin\"}\n    ]\n  }\n}\n";
    let config_path = write_temp(&temp_dir, "config.json", source);

    let added = run(&[
        "add",
        &config_path,
        "cfg.users",
        "bob",
        "--slug-field",
        "uid",
        "role=dev",
    ]);
    assert!(added.status.success(), "{:?}", added);
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(parsed["cfg"]["users"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["cfg"]["users"][1]["uid"], "bob");
    assert_eq!(parsed["cfg"]["users"][1]["role"], "dev");

    let removed = run(&[
        "remove",
        &config_path,
        "cfg.users",
        "a",
        "--slug-field",
        "uid",
    ]);
    assert!(removed.status.success(), "{:?}", removed);
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(parsed["cfg"]["users"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["cfg"]["users"][0]["uid"], "bob");
}

#[cfg(feature = "toml")]
#[test]
fn test_atomic_failure_keeps_original_file() {
    let temp_dir = TempDir::new().unwrap();
    let original = "value = 1\nkeep = 2\n";
    let config_path = write_temp(&temp_dir, "config.toml", original);
    let output = run(&["set", &config_path, "value", "--value-type", "null"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), original);
}

#[cfg(feature = "dotenv")]
#[test]
fn test_dotenv_mutations_preserve_source_and_reject_structural_ops() {
    let cases: &[&[&str]] = &[
        &["add", "items", "new", "--slug-field", "id", "name=value"],
        &["remove", "items", "old", "--slug-field", "id"],
    ];
    for arguments in cases {
        let temp_dir = TempDir::new().unwrap();
        let original = "KEY=value\n";
        let config_path = write_temp(&temp_dir, ".env", original);

        let mut full: Vec<&str> = vec![arguments[0], &config_path];
        full.extend_from_slice(&arguments[1..]);
        let output = run(&full);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let response = json_stderr(&output);
        assert!(
            response["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("does not support") || m.contains("not found"))
        );
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), original);
    }

    let temp_dir = TempDir::new().unwrap();
    let original = "# keep\nexport KEY=value # comment\nOTHER=unchanged\n";
    let config_path = write_temp(&temp_dir, ".env", original);
    let output = run(&["set", &config_path, "KEY", "changed"]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        "# keep\nexport KEY=changed # comment\nOTHER=unchanged\n"
    );
}

#[cfg(feature = "dotenv")]
#[test]
fn test_dotenv_get_and_whole_document() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, ".env", "KEY=value\nEMPTY=\n");

    let get = run(&["get", &config_path, "KEY"]);
    assert!(get.status.success());
    assert_eq!(json_stdout(&get)["result"]["value"], "value");

    let whole = run(&["get", &config_path]);
    assert!(whole.status.success());
    assert_eq!(json_stdout(&whole)["result"]["value"]["EMPTY"], "");

    // A `${VAR}`-shaped value is a dotenv literal, never expanded from the
    // afdata process's own environment.
    std::fs::write(&config_path, "REFERENCE=${AFDATA_TEST_PROCESS_VALUE}\n").unwrap();
    let literal = Command::new(env!("CARGO_BIN_EXE_afdata"))
        .env("AFDATA_TEST_PROCESS_VALUE", "must-not-be-read")
        .args(["get", &config_path, "REFERENCE"])
        .output()
        .unwrap();
    assert!(literal.status.success());
    assert_eq!(
        json_stdout(&literal)["result"]["value"],
        "${AFDATA_TEST_PROCESS_VALUE}"
    );
}

// ═══════════════════════════════════════════
// D4: --secret-from stdin|fd:<N>|env:<VAR>, exact round-trip,
// oversized/invalid-utf8 rejection, preflight-before-read ordering
// ═══════════════════════════════════════════

#[test]
fn test_secret_from_stdin_and_env_round_trip_exactly() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(
        &temp_dir,
        "secrets.json",
        "{\"password_secret\":\"old\",\"nested\":{\"API_KEY\":\"key\"}}\n",
    );
    let output = run_with_stdin(
        &[
            "set",
            &config_path,
            "password_secret",
            "--secret-from",
            "stdin",
        ],
        b"piped\n",
    );
    assert!(output.status.success(), "{:?}", output);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("piped"));
    let value = run(&["value", &config_path, "password_secret", "--reveal-secret"]);
    assert_eq!(value.stdout, b"piped\n");

    let show = run(&["get", &config_path, "--secret-name", "API_KEY"]);
    assert!(show.status.success());
    let response = json_stdout(&show);
    assert_eq!(response["result"]["value"]["password_secret"], "***");
    assert_eq!(response["result"]["value"]["nested"]["API_KEY"], "***");

    // env: source.
    let config_path2 = write_temp(&temp_dir, "secrets2.json", "{\"api_key_secret\":\"old\"}\n");
    let set = Command::new(env!("CARGO_BIN_EXE_afdata"))
        .env("AFDATA_TEST_SECRET", "s3kr3t-Ünïcode-#=")
        .args([
            "set",
            &config_path2,
            "api_key_secret",
            "--secret-from",
            "env:AFDATA_TEST_SECRET",
        ])
        .output()
        .unwrap();
    assert!(set.status.success(), "{:?}", set);
    let got = run(&["value", &config_path2, "api_key_secret", "--reveal-secret"]);
    assert_eq!(String::from_utf8_lossy(&got.stdout), "s3kr3t-Ünïcode-#=");
}

#[test]
fn test_secret_from_env_unset_is_runtime_not_usage_error() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "secrets.json", "{\"password_secret\":\"old\"}\n");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_afdata"));
    cmd.env_remove("AFDATA_TEST_SECRET_UNSET");
    let output = cmd
        .args([
            "set",
            &config_path,
            "password_secret",
            "--secret-from",
            "env:AFDATA_TEST_SECRET_UNSET",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        json_stderr(&output)["error"]["code"],
        "document_secret_source_failed"
    );
}

#[test]
fn test_secret_from_env_preserves_an_explicit_empty_value() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "secrets.json", "{\"password_secret\":\"old\"}\n");
    let output = Command::new(env!("CARGO_BIN_EXE_afdata"))
        .env("AFDATA_TEST_EMPTY_SECRET", "")
        .args([
            "set",
            &config_path,
            "password_secret",
            "--secret-from",
            "env:AFDATA_TEST_EMPTY_SECRET",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(document["password_secret"], "");
}

#[test]
fn test_secret_from_stdin_oversized_and_invalid_utf8() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "secrets.json", "{\"password_secret\":\"old\"}\n");

    let oversized = vec![b'x'; 1024 * 1024 + 1];
    let output = run_with_stdin(
        &[
            "set",
            &config_path,
            "password_secret",
            "--secret-from",
            "stdin",
        ],
        &oversized,
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exceeds"));

    let output = run_with_stdin(
        &[
            "set",
            &config_path,
            "password_secret",
            "--secret-from",
            "stdin",
        ],
        &[0xff, b'\n'],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("UTF-8"));
}

#[cfg(unix)]
#[test]
fn test_secret_preflight_rejects_hardlink_before_reading_stdin() {
    let temp_dir = TempDir::new().unwrap();
    let original = temp_dir.path().join("original.json");
    let linked = temp_dir.path().join("linked.json");
    std::fs::write(&original, "{\"token_secret\":\"old\"}\n").unwrap();
    std::fs::hard_link(&original, &linked).unwrap();
    let output = run_with_stdin(
        &[
            "set",
            linked.to_str().unwrap(),
            "token_secret",
            "--secret-from",
            "stdin",
        ],
        b"must-not-be-read\n",
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let response = json_stderr(&output);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("hardlinked")
    );
    assert_eq!(
        std::fs::read_to_string(&original).unwrap(),
        "{\"token_secret\":\"old\"}\n"
    );
}

#[cfg(unix)]
#[test]
fn test_secret_from_fd_rejects_low_and_non_numeric_descriptors() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "secrets.json", "{\"password_secret\":\"old\"}\n");

    let error = run(&[
        "set",
        &config_path,
        "password_secret",
        "--secret-from",
        "fd:2",
    ]);
    assert_eq!(error.status.code(), Some(2));
    assert!(error.stdout.is_empty());
    assert!(String::from_utf8_lossy(&error.stderr).contains("descriptor >= 3"));

    let error = run(&[
        "set",
        &config_path,
        "password_secret",
        "--secret-from",
        "fd:nope",
    ]);
    assert_eq!(error.status.code(), Some(2));
    assert!(error.stdout.is_empty());
    assert!(String::from_utf8_lossy(&error.stderr).contains("numeric descriptor"));
}

/// The flag exists so a secret never sits on argv; its error must not point
/// at `literal:`, the one form it refuses.
#[test]
fn test_secret_from_errors_never_suggest_a_literal() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "secrets.json", "{}\n");
    for source in ["file:/etc/hosts", "bogus", "literal:x"] {
        let error = run(&[
            "set",
            &config_path,
            "password_secret",
            "--secret-from",
            source,
        ]);
        assert_eq!(error.status.code(), Some(2), "{source}");
        let message = json_stderr(&error)["error"]["message"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(message.contains("env:<VAR>"), "{source}: {message}");
        assert!(!message.contains("literal"), "{source}: {message}");
    }
}

#[test]
fn test_invalid_secret_source_is_rejected_before_target_io_without_echoing_it() {
    let temp_dir = TempDir::new().unwrap();
    let missing = temp_dir.path().join("missing.json");
    let canary = "AFDATA_SECRET_SOURCE_CANARY";
    let output = run(&[
        "set",
        missing.to_str().unwrap(),
        "password_secret",
        "--secret-from",
        canary,
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains(canary), "{stderr}");
    assert_eq!(
        json_stderr(&output)["error"]["code"],
        "document_usage_error"
    );

    let invalid_fd = run(&[
        "set",
        missing.to_str().unwrap(),
        "password_secret",
        "--secret-from",
        "fd:nope",
    ]);
    assert_eq!(invalid_fd.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&invalid_fd.stderr).contains("numeric descriptor"),
        "{invalid_fd:?}"
    );
}

// ═══════════════════════════════════════════
// File mode preservation, output formats, and argument conflicts
// ═══════════════════════════════════════════

#[cfg(unix)]
#[test]
fn test_set_preserves_file_mode() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "m.json", "{\"a\":1}\n");
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o640)).unwrap();

    let out = run(&["set", &config_path, "a", "2", "--value-type", "number"]);
    assert!(out.status.success(), "{:?}", out);
    let mode = std::fs::metadata(&config_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o640,
        "atomic replace must preserve the original mode"
    );
}

#[test]
fn test_output_formats_and_conflicting_secret_source() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(&temp_dir, "config.json", "{\"name\":\"demo\"}\n");

    for output_format in ["yaml", "plain"] {
        let output = run(&["get", &config_path, "name", "--output", output_format]);
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }

    let error = run(&[
        "set",
        &config_path,
        "name",
        "ordinary",
        "--secret-from",
        "stdin",
    ]);
    assert_eq!(error.status.code(), Some(2));
    let response = json_stderr(&error);
    assert!(response["error"].is_object());
    assert!(error.stdout.is_empty());
}

// ═══════════════════════════════════════════
// §8: frontmatter mode — address the `+++`/`---` block of a Markdown file,
// body bytes frozen, format named explicitly (never sniffed).
// ═══════════════════════════════════════════

#[cfg(feature = "toml")]
#[test]
fn test_frontmatter_toml_read_and_body_frozen_on_set() {
    let temp_dir = TempDir::new().unwrap();
    let body = "# Heading\n\nProse with a stray `+++` inline and a fake\n+++closing lookalike.\n";
    let page = write_temp(
        &temp_dir,
        "_index.md",
        &format!("+++\ntitle = \"Old\"\ndescription = \"d\"\n+++\n{body}"),
    );

    // read a frontmatter field
    let got = run(&[
        "value",
        &page,
        "title",
        "--input-format",
        "toml-frontmatter",
    ]);
    assert!(got.status.success(), "{:?}", got);
    assert_eq!(String::from_utf8_lossy(&got.stdout), "Old");

    // edit a frontmatter field
    let set = run(&[
        "set",
        &page,
        "title",
        "New",
        "--input-format",
        "toml-frontmatter",
    ]);
    assert!(set.status.success(), "{:?}", set);
    assert_eq!(json_stdout(&set)["result"]["format"], "toml-frontmatter");

    let after = std::fs::read_to_string(&page).unwrap();
    assert_eq!(
        after,
        format!("+++\ntitle = \"New\"\ndescription = \"d\"\n+++\n{body}"),
        "frontmatter edited; every body byte (including the lookalike fences) unchanged"
    );
}

#[cfg(feature = "toml")]
#[test]
fn test_frontmatter_toml_insert_and_unset_keep_body() {
    let temp_dir = TempDir::new().unwrap();
    let body = "Body line one.\nBody line two.\n";
    let page = write_temp(
        &temp_dir,
        "post.md",
        &format!("+++\ntitle = \"t\"\n[extra]\ntagline = \"x\"\n+++\n{body}"),
    );

    // insert a brand-new key into an existing table
    let ins = run(&[
        "set",
        &page,
        "extra.ask_prompt",
        "Ask me",
        "--input-format",
        "toml-frontmatter",
    ]);
    assert!(ins.status.success(), "{:?}", ins);
    let after_insert = std::fs::read_to_string(&page).unwrap();
    assert!(after_insert.contains("ask_prompt = \"Ask me\""));
    assert!(
        after_insert.ends_with(&format!("+++\n{body}")),
        "body frozen"
    );

    // remove it again
    let del = run(&[
        "unset",
        &page,
        "extra.ask_prompt",
        "--input-format",
        "toml-frontmatter",
    ]);
    assert!(del.status.success(), "{:?}", del);
    let after_unset = std::fs::read_to_string(&page).unwrap();
    assert!(!after_unset.contains("ask_prompt"));
    assert!(
        after_unset.ends_with(&format!("+++\n{body}")),
        "body frozen"
    );
}

#[cfg(feature = "yaml")]
#[test]
fn test_frontmatter_yaml_read_and_edit() {
    let temp_dir = TempDir::new().unwrap();
    let body = "Content below the fence.\n";
    let page = write_temp(
        &temp_dir,
        "note.md",
        &format!("---\ntitle: Old\ndraft: false\n---\n{body}"),
    );

    let got = run(&[
        "value",
        &page,
        "title",
        "--input-format",
        "yaml-frontmatter",
    ]);
    assert!(got.status.success(), "{:?}", got);
    assert_eq!(String::from_utf8_lossy(&got.stdout), "Old");

    let set = run(&[
        "set",
        &page,
        "title",
        "New",
        "--input-format",
        "yaml-frontmatter",
    ]);
    assert!(set.status.success(), "{:?}", set);
    assert_eq!(json_stdout(&set)["result"]["format"], "yaml-frontmatter");

    let after = std::fs::read_to_string(&page).unwrap();
    assert!(after.contains("title: New"));
    assert!(after.ends_with(&format!("---\n{body}")), "body frozen");
}

#[cfg(feature = "toml")]
#[test]
fn test_frontmatter_missing_block_is_a_hard_error() {
    let temp_dir = TempDir::new().unwrap();
    // A plain Markdown file with no frontmatter must error, not be treated as
    // an all-body document.
    let page = write_temp(&temp_dir, "plain.md", "# Just a heading\n\nprose\n");
    let got = run(&[
        "value",
        "title",
        &page,
        "--input-format",
        "toml-frontmatter",
    ]);
    assert!(!got.status.success());
    assert!(got.stdout.is_empty(), "value writes nothing on failure");
    assert!(json_stderr(&got)["error"].is_object());
}

#[cfg(feature = "toml")]
#[test]
fn test_frontmatter_secret_field_still_redacts_on_get() {
    let temp_dir = TempDir::new().unwrap();
    let page = write_temp(
        &temp_dir,
        "creds.md",
        "+++\napi_key_secret = \"sk-live-xyz\"\n+++\nbody\n",
    );
    // A `_secret` leaf stays starred even on a targeted `get` — the frontmatter
    // backend feeds the same AFDATA record path as every other format.
    let got = run(&[
        "get",
        &page,
        "api_key_secret",
        "--input-format",
        "toml-frontmatter",
    ]);
    assert!(got.status.success(), "{:?}", got);
    assert_eq!(json_stdout(&got)["result"]["value"], "***");
}

#[cfg(feature = "markdown")]
#[test]
fn test_markdown_frontmatter_block_never_copies_metadata_text() {
    let output = run_with_stdin(
        &[
            "get",
            "-",
            "preamble.blocks.0",
            "--input-format",
            "markdown",
        ],
        b"---\ntoken_secret: sk-live-TOPSECRET\n---\n\n# T\n",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        json_stdout(&output)["result"]["value"]["type"],
        "frontmatter"
    );
    assert_eq!(json_stdout(&output)["result"]["value"]["format"], "yaml");
    assert_eq!(json_stdout(&output)["result"]["value"]["text"], "");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("sk-live-TOPSECRET"));
}

#[cfg(feature = "markdown")]
#[test]
fn test_markdown_reads_a_readme_as_sections() {
    let temp_dir = TempDir::new().unwrap();
    // The shape a real README opens with, including the badge line that makes
    // "first block wins" publish the wrong name.
    let readme = write_temp(
        &temp_dir,
        "README.md",
        "[![CI](a.svg)](b)\n\n# Agent First Data\n\nA naming **convention** for\nagents.\n\n\
         > **Ask your agent:** \"Lint this config.\"\n\n## A quick look\n\nInside.\n",
    );

    // The badge is preamble, so it cannot shift the title.
    let preamble = run(&[
        "get",
        &readme,
        "preamble.blocks.0",
        "--input-format",
        "markdown",
    ]);
    assert!(preamble.status.success(), "{preamble:?}");
    assert_eq!(json_stdout(&preamble)["result"]["value"]["text"], "CI");

    let title = run(&["value", &readme, "h1.0.text", "--input-format", "markdown"]);
    assert!(title.status.success(), "{title:?}");
    assert_eq!(String::from_utf8_lossy(&title.stdout), "Agent First Data");

    // Wrapped lines join and inline emphasis is gone — this exact string is
    // what reaches a package registry's description field.
    let lead = run(&[
        "value",
        &readme,
        "h1.0.paragraph.0.text",
        "--input-format",
        "markdown",
    ]);
    assert_eq!(
        String::from_utf8_lossy(&lead.stdout),
        "A naming convention for agents."
    );

    // Addressed by content, because this blockquote sits at no fixed index.
    let prompt = run(&[
        "value",
        &readme,
        "h1.0.blockquote.Ask your agent.text",
        "--input-format",
        "markdown",
    ]);
    assert!(prompt.status.success(), "{prompt:?}");
    assert_eq!(
        String::from_utf8_lossy(&prompt.stdout),
        "Ask your agent: \"Lint this config.\""
    );

    // A child section, by a word of its heading.
    let section = run(&[
        "value",
        &readme,
        "h1.0.h2.quick.paragraph.0.text",
        "--input-format",
        "markdown",
    ]);
    assert_eq!(String::from_utf8_lossy(&section.stdout), "Inside.");

    let got = run(&["get", &readme, "h1.0.h2.0", "--input-format", "markdown"]);
    assert_eq!(json_stdout(&got)["result"]["format"], "markdown");
    assert_eq!(json_stdout(&got)["result"]["value"]["text"], "A quick look");
    assert_eq!(
        json_stdout(&got)["result"]["value"]["source_start_line"],
        10
    );
    assert_eq!(json_stdout(&got)["result"]["value"]["heading_end_line"], 10);

    let lead_end = run(&[
        "value",
        &readme,
        "h1.0.paragraph.0.source_end_line",
        "--input-format",
        "markdown",
    ]);
    assert_eq!(String::from_utf8_lossy(&lead_end.stdout), "6");
}

#[cfg(feature = "markdown")]
#[test]
fn test_markdown_reports_inline_structure_and_nested_code() {
    let temp_dir = TempDir::new().unwrap();
    let readme = write_temp(
        &temp_dir,
        "README.md",
        "# Run `serve`\n\nRead [the docs](https://u:inline_canary@h/docs).\n\n\
         - step\n\n  ```bash\n  afdata nested\n  ```\n",
    );
    let code = run(&[
        "value",
        &readme,
        "h1.0.inlines.1.type",
        "--input-format",
        "markdown",
    ]);
    assert_eq!(String::from_utf8_lossy(&code.stdout), "code", "{code:?}");

    // A link's destination is a `_url` field, so its credentials are redacted
    // like any other.
    let link = run(&[
        "get",
        &readme,
        "h1.0.paragraph.0.inlines.docs",
        "--input-format",
        "markdown",
    ]);
    assert!(link.status.success(), "{link:?}");
    let value = &json_stdout(&link)["result"]["value"];
    assert_eq!(value["type"], "link");
    assert_eq!(value["destination_url"], "https://u:***@h/docs");
    assert!(!String::from_utf8_lossy(&link.stdout).contains("inline_canary"));

    // A fence inside a list item stays a `code` block.
    let nested = run(&[
        "get",
        &readme,
        "h1.0.blocks.1.blocks.0.blocks.1",
        "--input-format",
        "markdown",
    ]);
    assert!(nested.status.success(), "{nested:?}");
    let value = &json_stdout(&nested)["result"]["value"];
    assert_eq!(value["type"], "code");
    assert_eq!(value["language"], "bash");
    assert_eq!(value["text"], "afdata nested");
}

#[cfg(feature = "markdown")]
#[test]
fn test_markdown_content_addressing_refuses_to_guess() {
    let temp_dir = TempDir::new().unwrap();
    let readme = write_temp(
        &temp_dir,
        "README.md",
        "# T\n\n## Quick look\n\na.\n\n## Another look\n\nb.\n",
    );

    let ambiguous = run(&[
        "value",
        &readme,
        "h1.0.h2.look.text",
        "--input-format",
        "markdown",
    ]);
    assert!(!ambiguous.status.success(), "{ambiguous:?}");
    assert_eq!(
        json_stderr(&ambiguous)["error"]["code"],
        "document_ambiguous_match"
    );
    let error_event = json_stderr(&ambiguous);
    let error_message = error_event["error"]["message"].as_str().unwrap_or_default();
    assert!(error_message.contains("indices 0, 1"), "{error_message}");
    assert!(!error_message.contains("Quick look"), "{error_message}");
    assert!(!error_message.contains("Another look"), "{error_message}");
    assert!(
        ambiguous.stdout.is_empty(),
        "value writes nothing on failure"
    );

    let missing = run(&[
        "value",
        &readme,
        "h1.0.h2.nowhere.text",
        "--input-format",
        "markdown",
    ]);
    assert_eq!(
        json_stderr(&missing)["error"]["code"],
        "document_slug_not_found"
    );

    // A word that separates them resolves.
    let resolved = run(&[
        "value",
        &readme,
        "h1.0.h2.Another.text",
        "--input-format",
        "markdown",
    ]);
    assert_eq!(String::from_utf8_lossy(&resolved.stdout), "Another look");
}

#[test]
fn test_content_addressing_is_markdown_only() {
    // JSON hands back whatever the file said; nothing in it tells afdata which
    // field of a `deps` element `serde` is meant to match, so the segment stays
    // an error rather than becoming a scan.
    let output = run_with_stdin(
        &["value", "-", "deps.serde", "--input-format", "json"],
        br#"{"deps":[{"name":"serde"}]}"#,
    );
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(
        json_stderr(&output)["error"]["code"],
        "document_path_not_found"
    );
}

#[cfg(feature = "markdown")]
#[test]
fn test_markdown_is_read_only_and_never_sniffed() {
    let temp_dir = TempDir::new().unwrap();
    let readme = write_temp(&temp_dir, "README.md", "# Title\n\nThe lead.\n");

    // Never auto-detected: `.md` has valid readings as both blocks and
    // frontmatter, and the caller picks one.
    let sniffed = run(&["value", &readme, "h1.0.text"]);
    assert!(!sniffed.status.success(), "{sniffed:?}");
    assert_eq!(
        json_stderr(&sniffed)["error"]["code"],
        "document_format_unknown"
    );

    // Every write verb is refused, and the file is untouched.
    for args in [
        vec!["set", &readme, "h1.0.text", "New"],
        vec!["unset", &readme, "h1.0"],
        vec![
            "add",
            &readme,
            "preamble.blocks",
            "x",
            "--slug-field",
            "type",
        ],
        vec![
            "remove",
            &readme,
            "preamble.blocks",
            "x",
            "--slug-field",
            "type",
        ],
    ] {
        let mut argv = args.clone();
        argv.extend(["--input-format", "markdown"]);
        let refused = run(&argv);
        assert!(!refused.status.success(), "{args:?} must fail: {refused:?}");
        // Refused while parsing argv, not after opening the file: a mutating
        // verb does not accept `markdown` as an `--input-format` value at all,
        // so `--help` and docs/cli.md cannot advertise a combination that only
        // ever fails. Exit 2 (usage), not 1 (runtime).
        assert_eq!(
            json_stderr(&refused)["error"]["code"],
            "cli_invalid_argument_value",
            "{args:?}"
        );
        assert_eq!(refused.status.code(), Some(2), "{args:?}");
    }

    // The refusal lands before any external secret source is read. This missing
    // environment variable would produce a different error if afdata reached
    // for it first.
    let secret_refused = run(&[
        "set",
        &readme,
        "h1.0.text",
        "--secret-from",
        "env:AFDATA_TEST_MARKDOWN_READ_ONLY_MISSING_7F32A5",
        "--input-format",
        "markdown",
    ]);
    assert!(!secret_refused.status.success(), "{secret_refused:?}");
    assert_eq!(
        json_stderr(&secret_refused)["error"]["code"],
        "cli_invalid_argument_value"
    );

    // `lint` judges field names against the naming convention, and Markdown's
    // are afdata's own (`type`, `text`, `level`), so it is refused for the same
    // reason rather than always answering "no findings".
    let linted = run(&["lint", &readme, "--input-format", "markdown"]);
    assert!(!linted.status.success(), "{linted:?}");
    assert_eq!(
        json_stderr(&linted)["error"]["code"],
        "cli_invalid_argument_value"
    );

    assert_eq!(
        std::fs::read_to_string(&readme).unwrap(),
        "# Title\n\nThe lead.\n"
    );
}

/// Every address `paths` emits must be one the write verbs accept.
///
/// `paths` gained content-addressed output before `set`/`unset` could consume
/// it, so listing an element's paths and then editing one — the obvious use for
/// the feature — failed on afdata's own output. Reading and writing must agree
/// on what an address means, and only a round trip proves they do; each verb
/// tested alone passed throughout.
#[test]
fn test_content_addressed_paths_round_trip_through_the_write_verbs() {
    let temp_dir = TempDir::new().unwrap();
    let file = write_temp(
        &temp_dir,
        "identities.json",
        r#"{"identities":[{"identity":"me","email":"a@b.c"},{"identity":"you","email":"d@e.f"}]}"#,
    );

    let listed = run(&["paths", &file, "identities.me", "--slug-field", "identity"]);
    assert!(listed.status.success(), "{listed:?}");
    let emitted: Vec<String> = String::from_utf8(listed.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(
        emitted.contains(&"identities.me.email".to_string()),
        "{emitted:?}"
    );

    // Each emitted address, fed straight back exactly as printed. A fresh copy
    // per address because one of them is the slug field itself: writing it
    // renames the element, which would move every sibling address mid-loop and
    // test the wrong thing.
    for (index, path) in emitted.iter().enumerate() {
        let scratch = write_temp(
            &temp_dir,
            &format!("copy{index}.json"),
            &std::fs::read_to_string(&file).unwrap(),
        );
        let written = run(&["set", &scratch, path, "z", "--slug-field", "identity"]);
        assert!(written.status.success(), "set {path}: {written:?}");
        let read_back = run(&["value", &scratch, path, "--slug-field", "identity"]);
        // Reading `identities.me.identity` back after setting it to `z` is
        // expected to miss — the address named the old slug. Only the addresses
        // the write did not move must still resolve.
        if !path.ends_with(".identity") {
            assert_eq!(read_back.stdout, b"z", "{path}");
        }
    }

    let written = run(&[
        "set",
        &file,
        "identities.me.email",
        "z",
        "--slug-field",
        "identity",
    ]);
    assert!(written.status.success(), "{written:?}");
    // The sibling is untouched: a content address resolves to one element.
    assert_eq!(
        run(&[
            "value",
            &file,
            "identities.you.email",
            "--slug-field",
            "identity"
        ])
        .stdout,
        b"d@e.f"
    );

    let removed = run(&[
        "unset",
        &file,
        "identities.me.email",
        "--slug-field",
        "identity",
    ]);
    assert!(removed.status.success(), "{removed:?}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        r#"{"identities":[{"identity":"me"},{"identity":"you","email":"d@e.f"}]}"#
    );
}

/// A content address that names several elements, or none, is refused by the
/// write verbs on the same terms as the read verbs — never resolved to the
/// first hit, which would edit a different element than the caller named.
#[test]
fn test_write_verbs_refuse_ambiguous_and_missing_content_addresses() {
    let temp_dir = TempDir::new().unwrap();
    let file = write_temp(
        &temp_dir,
        "dupes.json",
        r#"{"xs":[{"id":"me","v":1},{"id":"me","v":2}]}"#,
    );
    let before = std::fs::read_to_string(&file).unwrap();

    for verb in [
        vec!["set", &file, "xs.me.v", "9", "--slug-field", "id"],
        vec!["unset", &file, "xs.me.v", "--slug-field", "id"],
    ] {
        let refused = run(&verb);
        assert!(!refused.status.success(), "{verb:?}: {refused:?}");
        assert_eq!(
            json_stderr(&refused)["error"]["code"],
            "document_ambiguous_match",
            "{verb:?}"
        );
    }

    let missing = run(&["unset", &file, "xs.nobody.v", "--slug-field", "id"]);
    assert!(!missing.status.success(), "{missing:?}");
    assert_eq!(
        json_stderr(&missing)["error"]["code"],
        "document_slug_not_found"
    );

    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
}

/// The same round trip through a format whose writer preserves source, to prove
/// the address is resolved before the backend sees it rather than by it.
#[test]
fn test_content_addressed_write_preserves_yaml_source() {
    let temp_dir = TempDir::new().unwrap();
    let file = write_temp(
        &temp_dir,
        "hosts.yaml",
        "# fleet\nhosts:\n  - name: alpha   # first\n    port: 1\n  - name: beta\n    port: 2\n",
    );

    let written = run(&[
        "set",
        &file,
        "hosts.beta.port",
        "9",
        "--value-type",
        "number",
        "--slug-field",
        "name",
    ]);
    assert!(written.status.success(), "{written:?}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "# fleet\nhosts:\n  - name: alpha   # first\n    port: 1\n  - name: beta\n    port: 9\n"
    );
}

#[cfg(feature = "markdown")]
#[test]
fn test_markdown_reads_stdin() {
    let output = run_with_stdin(
        &["value", "-", "h1.0.text", "--input-format", "markdown"],
        b"Setext title\n============\n",
    );
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Setext title");
}

#[test]
fn test_lint_reports_unlabelled_fields_without_failing() {
    // The four questions the README opens with. Before this, `lint` answered
    // every one of them with `{"findings":[],"ok":true}`.
    let output = run_with_stdin(
        &["lint", "-"],
        br#"{"timeout":5000,"price":1200,"created":1738886400,"api_key":"sk-live-abc123"}"#,
    );
    assert!(output.status.success(), "{output:?}");
    let result = &json_stdout(&output)["result"];
    assert_eq!(result["ok"], false);
    let findings = result["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 4);
    assert!(
        findings
            .iter()
            .all(|finding| finding["severity"] == "warning"
                && finding["rule_id"] == "missing_suffix")
    );

    // Heuristics are opt-out: `error` leaves only what the tool is sure of.
    let output = run_with_stdin(
        &["lint", "-", "--min-severity", "error"],
        br#"{"timeout":5000,"price":1200,"created":1738886400,"api_key":"sk-live-abc123"}"#,
    );
    assert!(output.status.success(), "{output:?}");
    let result = &json_stdout(&output)["result"];
    assert_eq!(result["ok"], true);
    assert!(result["findings"].as_array().expect("findings").is_empty());
}

#[test]
fn test_bare_value_overwriting_a_container_is_guarded() {
    // The guard used to cover scalar→scalar only, so it caught `8080 → "hello"`
    // (one value changes type) while letting `["a","b","c"] → "hello"` through
    // at exit 0 — the larger loss, with nothing left to notice it by.
    let temp_dir = TempDir::new().unwrap();
    let config_path = write_temp(
        &temp_dir,
        "config.json",
        r#"{"deps":["a","b","c"],"cfg":{"x":1}}"#,
    );

    for (key, kind) in [("deps", "array"), ("cfg", "object")] {
        let guarded = run(&["set", &config_path, key, "hello"]);
        assert_eq!(guarded.status.code(), Some(2), "{guarded:?}");
        assert!(guarded.stdout.is_empty());
        let response = json_stderr(&guarded);
        assert_eq!(response["error"]["code"], "document_usage_error");
        let message = response["error"]["message"].as_str().expect("message");
        assert!(message.contains(kind), "{message}");
        // Keeping a container means writing it back as JSON, not as a scalar type.
        assert!(message.contains("--value-type json"), "{message}");
    }

    // Untouched.
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        r#"{"deps":["a","b","c"],"cfg":{"x":1}}"#
    );

    // Escape hatch: replace it deliberately.
    let replaced = run(&[
        "set",
        &config_path,
        "deps",
        r#"["x"]"#,
        "--value-type",
        "json",
    ]);
    assert!(replaced.status.success(), "{replaced:?}");
}

#[test]
fn test_read_commands_can_address_a_keyed_list_by_slug() {
    // Registering a keyed list used to reach only `add`/`remove`: the read
    // commands passed no registration at all, so `identities.me.email` was
    // unreachable however the caller asked for it. `--slug-field` is how a
    // read states what a non-numeric segment means.
    let document =
        br#"{"identities":[{"identity":"me","email":"a@x"},{"identity":"you","email":"b@x"}]}"#;

    let refused = run_with_stdin(&["value", "-", "identities.me.email"], document);
    assert!(!refused.status.success(), "{refused:?}");
    assert_eq!(
        json_stderr(&refused)["error"]["code"],
        "document_path_not_found"
    );

    let resolved = run_with_stdin(
        &[
            "value",
            "-",
            "identities.me.email",
            "--slug-field",
            "identity",
        ],
        document,
    );
    assert!(resolved.status.success(), "{resolved:?}");
    assert_eq!(String::from_utf8_lossy(&resolved.stdout), "a@x");

    // Same declaration works for the other read commands.
    let keys = run_with_stdin(
        &["keys", "-", "identities.you", "--slug-field", "identity"],
        document,
    );
    assert!(keys.status.success(), "{keys:?}");
    assert_eq!(String::from_utf8_lossy(&keys.stdout), "email\nidentity\n");

    let got = run_with_stdin(
        &[
            "get",
            "-",
            "identities.you.email",
            "--slug-field",
            "identity",
        ],
        document,
    );
    assert_eq!(json_stdout(&got)["result"]["value"], "b@x");

    // An unknown slug is still a miss, not a scan for something close.
    let missing = run_with_stdin(
        &[
            "value",
            "-",
            "identities.nobody.email",
            "--slug-field",
            "identity",
        ],
        document,
    );
    assert_eq!(
        json_stderr(&missing)["error"]["code"],
        "document_slug_not_found"
    );

    // External documents are not guaranteed to obey keyed-edit uniqueness.
    // A duplicate exact identity is ambiguous, never "take the first".
    let duplicates =
        br#"{"identities":[{"identity":"me","email":"first"},{"identity":"me","email":"second"}]}"#;
    let ambiguous = run_with_stdin(
        &[
            "value",
            "-",
            "identities.me.email",
            "--slug-field",
            "identity",
        ],
        duplicates,
    );
    assert!(!ambiguous.status.success(), "{ambiguous:?}");
    let event = json_stderr(&ambiguous);
    assert_eq!(event["error"]["code"], "document_ambiguous_match");
    assert_eq!(
        event["error"]["message"],
        "segment `me` matches 2 elements of `identities` at indices 0, 1"
    );
    assert!(!String::from_utf8_lossy(&ambiguous.stderr).contains("first"));
    assert!(!String::from_utf8_lossy(&ambiguous.stderr).contains("second"));
}

#[test]
fn test_root_reads_reject_address_only_flags() {
    let document = br#"{"items":[{"id":"one"}]}"#;
    let cases: &[&[&str]] = &[
        &["get", "-", "--slug-field", "id"],
        &["paths", "-", "--slug-field", "id"],
        &["keys", "-", "--missing-ok"],
    ];

    for args in cases {
        let output = run_with_stdin(args, document);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        assert_eq!(
            json_stderr(&output)["error"]["code"],
            "cli_unregistered_combination",
            "{args:?}"
        );
    }
}

#[cfg(feature = "markdown")]
#[test]
fn test_explicit_slug_field_overrides_the_format_rule() {
    let temp_dir = TempDir::new().unwrap();
    let readme = write_temp(&temp_dir, "README.md", "# T\n\n## A Quick Look\n\nx.\n");

    // Markdown's own rule is substring: a word finds the section.
    let builtin = run(&[
        "value",
        &readme,
        "h1.0.h2.look.text",
        "--input-format",
        "markdown",
    ]);
    assert_eq!(String::from_utf8_lossy(&builtin.stdout), "A Quick Look");

    // Naming a field explicitly is the more specific statement, and it means
    // exact match — so the same substring no longer resolves.
    let exact = run(&[
        "value",
        &readme,
        "h1.0.h2.look.text",
        "--input-format",
        "markdown",
        "--slug-field",
        "text",
    ]);
    assert!(!exact.status.success(), "{exact:?}");
    assert_eq!(
        json_stderr(&exact)["error"]["code"],
        "document_slug_not_found"
    );

    let full = run(&[
        "value",
        &readme,
        "h1.0.h2.A Quick Look.text",
        "--input-format",
        "markdown",
        "--slug-field",
        "text",
    ]);
    assert_eq!(String::from_utf8_lossy(&full.stdout), "A Quick Look");
}

#[cfg(feature = "markdown")]
#[test]
fn test_default_absorbs_a_content_miss_but_not_an_ambiguity() {
    let temp_dir = TempDir::new().unwrap();

    // "Nothing matched" is a miss like any other, so `--default` covers it.
    // Without this a script reading an optional section by name — a README
    // that simply has no such heading — dies instead of taking its fallback.
    let plain = write_temp(&temp_dir, "plain.md", "# T\n\n> a quote\n");
    let missed = run(&[
        "value",
        &plain,
        "h1.0.blocks.nothing like this.text",
        "--input-format",
        "markdown",
        "--default",
        "(none)",
    ]);
    assert!(missed.status.success(), "{missed:?}");
    assert_eq!(String::from_utf8_lossy(&missed.stdout), "(none)");

    // Several matched, so the document is not missing anything — the address
    // is. Falling back would answer a question the caller never asked.
    let twice = write_temp(&temp_dir, "twice.md", "# T\n\n> look one\n\n> look two\n");
    let ambiguous = run(&[
        "value",
        &twice,
        "h1.0.blocks.look.text",
        "--input-format",
        "markdown",
        "--default",
        "(none)",
    ]);
    assert!(!ambiguous.status.success(), "{ambiguous:?}");
    assert!(ambiguous.stdout.is_empty());
    assert_eq!(
        json_stderr(&ambiguous)["error"]["code"],
        "document_ambiguous_match"
    );

    // `keys --missing-ok` shares the exemption, and the same split.
    let keys = run(&[
        "keys",
        &plain,
        "h1.0.blocks.nothing like this",
        "--input-format",
        "markdown",
        "--missing-ok",
    ]);
    assert!(keys.status.success(), "{keys:?}");
    assert!(keys.stdout.is_empty());
}

#[cfg(feature = "dotenv")]
#[test]
fn dotenv_set_preserves_crlf_keys_and_new_lines() {
    let temp = TempDir::new().unwrap();
    let file = write_temp(&temp, "crlf.env", "KEY=old\r\nexport OTHER=x\r\n");
    for (key, value) in [("KEY", "new"), ("OTHER", "y"), ("NEW", "z")] {
        let output = run(&["set", &file, key, value]);
        assert!(output.status.success(), "{output:?}");
    }
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"KEY=new\r\nexport OTHER=y\r\nNEW=z\r\n"
    );
    assert_eq!(run(&["value", &file, "KEY"]).stdout, b"new");
}

#[cfg(feature = "dotenv")]
#[test]
fn dotenv_set_preserves_crlf_around_comments_indentation_and_a_missing_final_newline() {
    let temp = TempDir::new().unwrap();

    // An inline comment sits between the value and the line ending; both stay.
    let commented = write_temp(&temp, "commented.env", "KEY=old # c\r\nOTHER=x\r\n");
    let output = run(&["set", &commented, "KEY", "new"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read(&commented).unwrap(),
        b"KEY=new # c\r\nOTHER=x\r\n"
    );
    assert_eq!(run(&["value", &commented, "KEY"]).stdout, b"new");

    // Leading indentation, with and without `export`, is not the editor's to drop.
    let indented = write_temp(
        &temp,
        "indented.env",
        "  KEY=old\r\n\texport OTHER=x # c\r\n",
    );
    for (key, value) in [("KEY", "new"), ("OTHER", "y")] {
        let output = run(&["set", &indented, key, value]);
        assert!(output.status.success(), "{output:?}");
    }
    assert_eq!(
        std::fs::read(&indented).unwrap(),
        b"  KEY=new\r\n\texport OTHER=y # c\r\n"
    );
    assert_eq!(run(&["value", &indented, "OTHER"]).stdout, b"y");

    // A final line with no terminator gets one in the file's own style before
    // the appended key; editing that final line in place adds nothing.
    let unterminated = write_temp(&temp, "unterminated.env", "A=1\r\nB=2");
    let output = run(&["set", &unterminated, "B", "3"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(&unterminated).unwrap(), b"A=1\r\nB=3");
    let output = run(&["set", &unterminated, "NEW", "z"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read(&unterminated).unwrap(),
        b"A=1\r\nB=3\r\nNEW=z\r\n"
    );
    assert_eq!(run(&["value", &unterminated, "NEW"]).stdout, b"z");
}

#[cfg(feature = "toml")]
#[test]
fn toml_mutations_preserve_crlf() {
    let temp = TempDir::new().unwrap();
    let file = write_temp(
        &temp,
        "crlf.toml",
        "a = 1\r\n# note\r\n[s]\r\nk = \"v\"\r\n",
    );
    let output = run(&["set", &file, "s.k", "w"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"a = 1\r\n# note\r\n[s]\r\nk = \"w\"\r\n"
    );
    let output = run(&["unset", &file, "s.k"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(&file).unwrap(), b"a = 1\r\n# note\r\n[s]\r\n");
}

#[cfg(feature = "toml")]
#[test]
fn toml_crlf_file_stores_a_multiline_value_byte_for_byte() {
    let temp = TempDir::new().unwrap();
    let file = write_temp(&temp, "crlf.toml", "a = 1\r\nk = \"v\"\r\n");

    // Replacing an existing key: the value's own `\n` is not a line ending of
    // the document, so CRLF preservation must leave it alone.
    let output = run(&["set", &file, "k", "x\ny"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(&file).unwrap(), b"a = 1\r\nk = \"x\\ny\"\r\n");
    assert_eq!(run(&["value", &file, "k"]).stdout, b"x\ny");

    // Appending a new key takes the same route.
    let output = run(&["set", &file, "n", "p\n\"q\"\\\tr\n"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"a = 1\r\nk = \"x\\ny\"\r\nn = \"p\\n\\\"q\\\"\\\\\\tr\\n\"\r\n"
    );
    assert_eq!(run(&["value", &file, "n"]).stdout, b"p\n\"q\"\\\tr\n");
}

#[cfg(feature = "toml")]
#[test]
fn toml_crlf_file_keeps_an_existing_multiline_string_when_another_key_changes() {
    let temp = TempDir::new().unwrap();
    let file = write_temp(
        &temp,
        "crlf.toml",
        "a = 1\r\nm = \"\"\"\r\nl1\r\nl2\"\"\"\r\nk = \"v\"\r\n",
    );
    let before = run(&["value", &file, "m"]);
    assert!(before.status.success(), "{before:?}");
    assert_eq!(before.stdout, b"l1\r\nl2");

    let output = run(&["set", &file, "k", "w"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read(&file).unwrap(),
        b"a = 1\r\nm = \"\"\"\r\nl1\r\nl2\"\"\"\r\nk = \"w\"\r\n"
    );
    assert_eq!(run(&["value", &file, "m"]).stdout, before.stdout);
}

#[cfg(unix)]
#[test]
fn value_succeeds_when_stdout_reader_has_closed() {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    let temp = TempDir::new().unwrap();
    let file = write_temp(&temp, "value.json", r#"{"a":"hello"}"#);
    let (reader, writer) = UnixStream::pair().unwrap();
    drop(reader);
    let output = afdata()
        .args(["value", &file, "a"])
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

/// The raw-output commands follow the same exit contract as every other
/// command: a write that fails for any reason but a closed reader is exit 4
/// with `output_write_failed` on stderr, never a bare exit 1.
#[cfg(unix)]
#[test]
fn raw_output_that_cannot_be_written_exits_4_with_a_diagnostic() {
    let dir = TempDir::new().unwrap();
    let file = write_temp(&dir, "doc.json", r#"{"a":[1,2],"s":"text"}"#);
    for args in [
        vec!["paths", file.as_str()],
        vec!["keys", file.as_str()],
        vec!["value", file.as_str(), "s"],
    ] {
        // A zero file-size limit makes the first stdout write fail with EFBIG.
        let output = Command::new("sh")
            .arg("-c")
            .arg(r#"trap '' XFSZ; ulimit -f 0; exec "$0" "$@" > "$OUT""#)
            .arg(env!("CARGO_BIN_EXE_afdata"))
            .args(&args)
            .env("OUT", dir.path().join("out.txt"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(4), "{args:?}");
        assert_eq!(
            json_stderr(&output)["error"]["code"],
            "output_write_failed",
            "{args:?}"
        );
    }
}

/// The protocol-event commands keep the same promise as the raw ones: an
/// unwritable stdout is exit 4 with `output_write_failed` on stderr, not a
/// silent exit 4 that leaves the caller guessing what failed.
#[cfg(unix)]
#[test]
fn event_output_that_cannot_be_written_exits_4_with_a_diagnostic() {
    let dir = TempDir::new().unwrap();
    let file = write_temp(&dir, "doc.json", r#"{"a":[1,2],"s":"text"}"#);
    for args in [
        vec!["get", file.as_str()],
        vec!["get", file.as_str(), "--output", "yaml"],
        vec!["emit", "result", "--json", "{}"],
        vec!["--version"],
    ] {
        let output = Command::new("sh")
            .arg("-c")
            .arg(r#"trap '' XFSZ; ulimit -f 0; exec "$0" "$@" > "$OUT""#)
            .arg(env!("CARGO_BIN_EXE_afdata"))
            .args(&args)
            .env("OUT", dir.path().join("out.txt"))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(4), "{args:?}: {output:?}");
        assert_eq!(
            json_stderr(&output)["error"]["code"],
            "output_write_failed",
            "{args:?}"
        );
    }
}

/// A closed reader stays success with nothing on stderr for event output too.
#[cfg(unix)]
#[test]
fn event_output_to_a_closed_reader_is_silent_success() {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    let dir = TempDir::new().unwrap();
    let file = write_temp(&dir, "doc.json", r#"{"a":1}"#);
    for args in [
        vec!["get", file.as_str()],
        vec!["emit", "result", "--json", "{}"],
    ] {
        let (reader, writer) = UnixStream::pair().unwrap();
        drop(reader);
        let output = afdata()
            .args(&args)
            .stdout(Stdio::from(OwnedFd::from(writer)))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
    }
}

/// An output sink that cannot be opened names the flag and the path, not just
/// the OS error, so a caller passing both sinks knows which one to fix.
#[cfg(all(unix, feature = "stream-redirect"))]
#[test]
fn an_unopenable_output_file_names_its_flag_and_path() {
    let temp = TempDir::new().unwrap();
    let missing_dir = temp.path().join("no_such_dir");
    for flag in ["--stdout-file", "--stderr-file"] {
        let sink = missing_dir.join("x").to_str().unwrap().to_string();
        let output = run(&["emit", "result", "--json", "{}", flag, &sink]);
        assert_eq!(output.status.code(), Some(1), "{flag}: {output:?}");
        assert!(output.stdout.is_empty(), "{flag}: {output:?}");
        let error = json_stderr(&output);
        assert_eq!(error["error"]["code"], "output_setup_failed", "{flag}");
        let message = error["error"]["message"].as_str().unwrap();
        assert!(
            message.starts_with(&format!("{flag} {sink}: ")),
            "{flag}: {message}"
        );
    }
}

#[cfg(feature = "stream-redirect")]
#[test]
fn invalid_emit_json_is_rejected_before_output_files_are_opened() {
    let temp = TempDir::new().unwrap();
    let stdout_file = write_temp(&temp, "stdout.txt", "keep stdout\n");
    let stderr_file = write_temp(&temp, "stderr.txt", "keep stderr\n");
    let output = run(&[
        "emit",
        "result",
        "--json",
        "{\"token_secret\":\"invalid_json_canary\",",
        "--output",
        "yaml",
        "--stdout-file",
        &stdout_file,
        "--stderr-file",
        &stderr_file,
    ]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let error = json_stderr(&output);
    assert_eq!(error["error"]["code"], "cli_invalid_argument_value");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("invalid_json_canary"));
    assert_eq!(
        std::fs::read_to_string(stdout_file).unwrap(),
        "keep stdout\n"
    );
    assert_eq!(
        std::fs::read_to_string(stderr_file).unwrap(),
        "keep stderr\n"
    );
}

#[test]
fn targeted_get_retains_url_redaction_context_and_absent_secrets() {
    let source = br#"{"connection_url":"https://u:targeted_url_canary@h/?token_secret=targeted_query_canary","pool_url":[{"endpoint":"https://u:nested_url_canary@h/"}],"token_secret":null}"#;
    let whole = run_with_stdin(&["get", "-"], source);
    let whole = json_stdout(&whole);
    for (key, expected) in [
        (
            "connection_url",
            whole["result"]["value"]["connection_url"].clone(),
        ),
        (
            "pool_url.0.endpoint",
            whole["result"]["value"]["pool_url"][0]["endpoint"].clone(),
        ),
        ("token_secret", serde_json::Value::Null),
    ] {
        let output = run_with_stdin(&["get", "-", key], source);
        assert!(output.status.success(), "{key}: {output:?}");
        assert_eq!(json_stdout(&output)["result"]["value"], expected, "{key}");
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("canary"),
            "{key}: {output:?}"
        );
    }
}

#[test]
fn raw_url_credentials_require_explicit_reveal() {
    let source = br#"{"connection_url":"https://u:raw_url_canary@h/?token_secret=raw_query_canary","public_url":"https://h/","pool_url":["https://u:nested_url_canary@h/"]}"#;
    for command in ["value", "values"] {
        for key in ["connection_url", "pool_url.0"] {
            let output = run_with_stdin(&[command, "-", key], source);
            assert_eq!(output.status.code(), Some(1), "{command} {key}: {output:?}");
            assert!(output.stdout.is_empty());
            assert_eq!(
                json_stderr(&output)["error"]["code"],
                "document_secret_redacted"
            );
            assert!(!String::from_utf8_lossy(&output.stderr).contains("canary"));
        }
        let public = run_with_stdin(&[command, "-", "public_url"], source);
        assert!(public.status.success(), "{public:?}");
        let fallback = run_with_stdin(
            &[
                command,
                "-",
                "missing_url",
                "--default",
                "https://u:fallback_url_canary@h/",
            ],
            source,
        );
        assert_eq!(fallback.status.code(), Some(1), "{fallback:?}");
        assert!(fallback.stdout.is_empty());
        assert_eq!(
            json_stderr(&fallback)["error"]["code"],
            "document_secret_redacted"
        );
        let message = json_stderr(&fallback)["error"]["message"].to_string();
        assert!(message.contains("--default for `missing_url`"), "{message}");
        assert!(!message.contains("canary"), "{message}");
        let revealed = run_with_stdin(&[command, "-", "connection_url", "--reveal-secret"], source);
        assert!(revealed.status.success(), "{revealed:?}");
        assert!(String::from_utf8_lossy(&revealed.stdout).contains("raw_url_canary"));
    }
}

/// `get` redacts a URL value it cannot read cleanly in full, but `value` asked
/// for that one string refuses only when a credential is actually in it.
#[test]
fn raw_url_without_credentials_is_returned_even_when_get_redacts_it() {
    let source = br#"{"contact_url":"mailto:x@y.z","page_url":"https://h/a b","dsn_url":"user:dsn_canary@host/db","spaced_url":"https://u:spaced_canary@h/a b","query_url":"https://h/a b?token_secret=query_canary","redirect_url":"https://h/a b?next=https://u:redirect_canary@i/","email_url":"sip:alice:sip_canary@host"}"#;

    let whole = json_stdout(&run_with_stdin(&["get", "-"], source));
    for key in ["contact_url", "page_url"] {
        assert_eq!(
            whole["result"]["value"][key], "***",
            "{key}: get is unchanged"
        );
    }

    for command in ["value", "values"] {
        for (key, expected) in [
            ("contact_url", "mailto:x@y.z"),
            ("page_url", "https://h/a b"),
        ] {
            let output = run_with_stdin(&[command, "-", key], source);
            assert!(output.status.success(), "{command} {key}: {output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim_end(),
                expected,
                "{command} {key}"
            );
        }
        for key in [
            "dsn_url",
            "spaced_url",
            "query_url",
            "redirect_url",
            "email_url",
        ] {
            let output = run_with_stdin(&[command, "-", key], source);
            assert_eq!(output.status.code(), Some(1), "{command} {key}: {output:?}");
            assert!(output.stdout.is_empty(), "{command} {key}");
            assert_eq!(
                json_stderr(&output)["error"]["code"],
                "document_secret_redacted",
                "{command} {key}"
            );
            assert!(!String::from_utf8_lossy(&output.stderr).contains("canary"));
        }
    }
}

#[cfg(feature = "toml")]
#[test]
fn rejected_numeric_edits_do_not_echo_the_value_in_diagnostics() {
    let temp = TempDir::new().unwrap();
    let source = "amount_secret = 1\n";
    let file = write_temp(&temp, "config.toml", source);
    let canary = "987654321098765432109876543210";
    let output = run(&[
        "set",
        &file,
        "amount_secret",
        canary,
        "--value-type",
        "number",
    ]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        json_stderr(&output)["error"]["code"],
        "document_unsupported_operation"
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains(canary),
        "{output:?}"
    );
    assert_eq!(std::fs::read_to_string(file).unwrap(), source);
}

#[test]
fn enumeration_rejects_names_that_would_split_or_truncate_output_records() {
    for command in ["keys", "paths"] {
        for (source, options) in [
            (br#"{"a":1,"b\nc":2}"#.as_slice(), vec![]),
            (br#"{"a":1,"b\u0000c":2}"#.as_slice(), vec!["--null"]),
            (br#"{"a":1,"b\u0000c":2}"#.as_slice(), vec![]),
        ] {
            let mut args = vec![command, "-"];
            args.extend(options);
            let output = run_with_stdin(&args, source);
            assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
            assert!(
                output.stdout.is_empty(),
                "no partially framed records: {output:?}"
            );
            assert_eq!(
                json_stderr(&output)["error"]["code"],
                "document_unframeable_name"
            );
        }
    }
}

#[test]
fn enumeration_cannot_expose_keys_inside_a_secret_subtree() {
    let source = br#"{"vault_secret":{"credential_key_canary":{"nested_key_canary":1}}}"#;
    for command in ["keys", "paths"] {
        for key in ["vault_secret", "vault_secret.credential_key_canary"] {
            let output = run_with_stdin(&[command, "-", key, "--missing-ok"], source);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(output.stdout.is_empty());
            assert_eq!(
                json_stderr(&output)["error"]["code"],
                "document_secret_redacted"
            );
            assert!(!String::from_utf8_lossy(&output.stderr).contains("canary"));
        }
    }
}

#[test]
fn a_flag_left_without_its_value_is_not_told_to_use_the_equals_spelling() {
    let flag_next = run(&["emit", "result", "--json", "--output", "json"]);
    assert_eq!(flag_next.status.code(), Some(2), "{flag_next:?}");
    let error = &json_stderr(&flag_next)["error"];
    assert_eq!(error["code"], "cli_missing_argument_value");
    assert_eq!(
        error["hint"],
        "pass a value right after the argument; `afdata emit result --help` shows its value name"
    );

    let dash_value = run(&["emit", "result", "--json", "-x"]);
    assert_eq!(dash_value.status.code(), Some(2), "{dash_value:?}");
    assert_eq!(
        json_stderr(&dash_value)["error"]["hint"],
        "write `--json=VALUE` for a value that starts with `-`"
    );
}

#[test]
fn get_keeps_the_sign_of_a_json_negative_zero() {
    let output = run_with_stdin(&["get", "-", "a"], br#"{"a":-0}"#);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("-0"),
        "get lost the sign of -0: {output:?}"
    );
}

#[test]
fn traversal_errors_name_the_scalar_that_blocked_them() {
    let dir = TempDir::new().unwrap();
    let path = write_temp(&dir, "k.json", r#"{"x":{"n":1},"s":1}"#);
    for arguments in [
        vec!["get", path.as_str(), "x.n.z.w"],
        vec!["value", path.as_str(), "x.n.z.w"],
        vec!["keys", path.as_str(), "x.n.z.w"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}: {output:?}");
        let response = json_stderr(&output);
        assert_eq!(
            response["error"]["message"], "path `x.n` is integer, cannot traverse further",
            "{arguments:?}: {response}"
        );
    }
    let output = run(&["add", path.as_str(), "s.y", "z", "--slug-field", "id"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let response = json_stderr(&output);
    assert_eq!(
        response["error"]["message"], "path `s` is integer, cannot traverse further",
        "{response}"
    );
}

#[test]
fn add_rejects_a_repeated_field() {
    let dir = TempDir::new().unwrap();
    let original = "{\"items\":[]}";
    let path = write_temp(&dir, "k.json", original);
    let output = run(&[
        "add",
        path.as_str(),
        "items",
        "a",
        "email=1",
        "email=2",
        "--slug-field",
        "id",
    ]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let response = json_stderr(&output);
    assert_eq!(
        response["error"]["code"], "document_usage_error",
        "{response}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn an_unknown_format_names_the_flag_to_pass() {
    let dir = TempDir::new().unwrap();
    let path = write_temp(&dir, "x.conf", "a=1\n");
    let output = run(&["get", path.as_str()]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let response = json_stderr(&output);
    assert_eq!(
        response["error"]["code"], "document_format_unknown",
        "{response}"
    );
    assert_eq!(
        response["error"]["hint"], "pass --input-format FORMAT",
        "{response}"
    );
}

#[cfg(feature = "yaml")]
#[test]
fn a_secret_never_replaces_a_collection_or_a_non_string() {
    let dir = TempDir::new().unwrap();
    for (name, original) in [("s.yaml", "db:\n  host: h\n"), ("n.yaml", "db: 1\n")] {
        let path = write_temp(&dir, name, original);
        let output = afdata()
            .args([
                "set",
                path.as_str(),
                "db",
                "--secret-from",
                "env:AFDATA_TEST_SECRET",
            ])
            .env("AFDATA_TEST_SECRET", "x")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{name}: {output:?}");
        let response = json_stderr(&output);
        assert_eq!(
            response["error"]["code"], "document_usage_error",
            "{response}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original, "{name}");
    }
}
