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
            .is_some_and(|m| m.contains("cannot detect format"))
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

    // A key may contain a newline, which is exactly why the line-separated
    // default is unsafe for machine consumption.
    let newline_key = run_with_stdin(&["paths", "-", "--null"], br#"{"a":1,"b\nc":2}"#);
    assert!(newline_key.status.success(), "{:?}", newline_key);
    assert_eq!(newline_key.stdout, b"a\0b\nc\0");

    // `-0` is not an alias: AFDATA spells its flags out in full.
    let short_form = run_with_stdin(&["paths", "-", "-0"], br#"{"a":1}"#);
    assert_eq!(short_form.status.code(), Some(2));
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
    // Read-side keyed-list addressing is out of scope (R8); index by
    // position instead.
    let response = json_stdout(&run(&["get", &config_path, "items.0.count"]));
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
