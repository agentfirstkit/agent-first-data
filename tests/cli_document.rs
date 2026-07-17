#![cfg(feature = "cli")]
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::expect_used,
    clippy::bool_assert_comparison
)]
//! CLI integration tests for afdata's document read/edit commands
//! (`show`/`get`/`value`/`set`/`add`/`remove`/`unset`).
//!
//! Every test invoking `CARGO_BIN_EXE_afdata` lives in this file, gated on
//! the `cli` feature (the bin target's `required-features`), so the file
//! still compiles when `cli` is disabled.
//!
//! Coverage includes the doc's required new-command behaviors (stdin
//! default/override, extension inference, `--input-file -` rejection,
//! `value`'s scalar/secret gates, source-preserving mutation, default
//! redaction) plus cases ported from `agent-first-config`'s
//! `tests/e2e.rs`, rewritten to the new command syntax. Two behaviors
//! changed deliberately from `agent-first-config`'s CLI and are called out
//! where ported: `get` now redacts a directly-targeted secret leaf (the old
//! `afconfig` CLI did not), and the raw-scalar read (`get-value` there,
//! `value` here) now gates on the same secret-naming rule unless
//! `--reveal-secret` is passed.

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
    child
        .stdin
        .take()
        .expect("stdin handle")
        .write_all(stdin)
        .expect("failed to write stdin");
    child.wait_with_output().expect("failed to wait for afdata")
}

fn json_stdout(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|err| panic!("stdout is not JSON: {err}: {:?}", output.stdout))
}

// ═══════════════════════════════════════════
// Input resolution: stdin default, --input-format override, extension
// inference + explicit override, --input-file - rejection
// ═══════════════════════════════════════════

#[test]
fn test_stdin_defaults_to_json() {
    let output = run_with_stdin(&["show"], br#"{"a":1}"#);
    assert!(output.status.success(), "{:?}", output);
    let response = json_stdout(&output);
    assert_eq!(response["result"]["format"], "JSON");
    assert_eq!(response["result"]["value"]["a"], 1);
}

#[cfg(feature = "yaml")]
#[test]
fn test_input_format_override_applies_to_stdin() {
    let output = run_with_stdin(&["--input-format", "yaml", "show"], b"a: 1\nb: 2\n");
    assert!(output.status.success(), "{:?}", output);
    let response = json_stdout(&output);
    assert_eq!(response["result"]["format"], "YAML");
    assert_eq!(response["result"]["value"]["a"], 1);
    assert_eq!(response["result"]["value"]["b"], 2);
}

#[test]
fn test_file_extension_inference_and_explicit_override() {
    let temp_dir = TempDir::new().unwrap();
    // No recognizable extension: detection fails unless overridden.
    let config_path = temp_dir.path().join("extensionless.config");
    std::fs::write(&config_path, "{\"name\":\"explicit\"}\n").unwrap();

    let no_override = run(&["show", config_path.to_str().unwrap()]);
    assert!(!no_override.status.success());
    let response = json_stdout(&no_override);
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("cannot detect format"))
    );

    let overridden = run(&[
        "--input-format",
        "json",
        "get",
        "name",
        config_path.to_str().unwrap(),
    ]);
    assert!(overridden.status.success(), "{:?}", overridden);
    let response = json_stdout(&overridden);
    assert_eq!(response["result"]["value"], "explicit");

    // A real extension is still detected without an override.
    let json_path = temp_dir.path().join("config.json");
    std::fs::write(&json_path, "{\"name\":\"by-extension\"}\n").unwrap();
    let detected = run(&["get", "name", json_path.to_str().unwrap()]);
    assert!(detected.status.success());
    assert_eq!(json_stdout(&detected)["result"]["value"], "by-extension");
}

#[test]
fn test_input_file_dash_rejected() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("c.json");
    std::fs::write(&config_path, "{\"a\":1}\n").unwrap();
    let output = run(&["set", "a", "2", "--input-file", "-"]);
    assert!(!output.status.success());
    let response = json_stdout(&output);
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("not a valid path"))
    );
}

// ═══════════════════════════════════════════
// `value`: scalar extraction, non-scalar errors, secret gate
// ═══════════════════════════════════════════

#[test]
fn test_value_scalar_and_errors() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.json");
    std::fs::write(
        &config_path,
        "{\"name\":\"hello\",\"empty\":\"\",\"enabled\":true,\"items\":[1]}\n",
    )
    .unwrap();

    let output = run(&["value", "name", config_path.to_str().unwrap()]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hello");
    assert!(output.stderr.is_empty());

    let output = run(&["value", "enabled", config_path.to_str().unwrap()]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"true");

    let output = run(&["value", "empty", config_path.to_str().unwrap()]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"");

    let output = run(&["value", "items", config_path.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let response = json_stdout(&output);
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("not a scalar"))
    );
}

#[cfg(feature = "yaml")]
#[test]
fn test_value_non_finite_float_errors() {
    // JSON has no literal for infinity/NaN, so this guard is only reachable
    // through a format whose scalars can represent one — YAML's `.inf`.
    let output = run_with_stdin(&["--input-format", "yaml", "value", "f"], b"f: .inf\n");
    assert_eq!(output.status.code(), Some(1));
    let response = json_stdout(&output);
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("non-finite"))
    );
}

#[test]
fn test_value_secret_gate_requires_reveal_flag() {
    let output = run_with_stdin(&["value", "a.b_secret"], br#"{"a":{"b_secret":"x"}}"#);
    assert_eq!(output.status.code(), Some(1));
    let response = json_stdout(&output);
    assert_eq!(response["error"]["code"], "document_error");
    assert!(
        response["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("--reveal-secret"))
    );

    let output = run_with_stdin(
        &["value", "a.b_secret", "--reveal-secret"],
        br#"{"a":{"b_secret":"x"}}"#,
    );
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(output.stdout, b"x");
}

#[test]
fn test_value_secret_name_gate_and_reveal() {
    // `--secret-name` extends the gate beyond the `_secret` suffix
    // convention, matching `get`/`show` redaction.
    let output = run_with_stdin(
        &["--secret-name", "PASSWORD", "value", "PASSWORD"],
        br#"{"PASSWORD":"hunter2"}"#,
    );
    assert_eq!(output.status.code(), Some(1));

    let output = run_with_stdin(
        &[
            "--secret-name",
            "PASSWORD",
            "value",
            "PASSWORD",
            "--reveal-secret",
        ],
        br#"{"PASSWORD":"hunter2"}"#,
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hunter2");
}

// ═══════════════════════════════════════════
// `get`/`show`: default `_secret` redaction, and the new targeted-get
// redaction behavior (a deliberate change from `agent-first-config`)
// ═══════════════════════════════════════════

#[test]
fn test_get_and_show_redact_secret_by_default() {
    let output = run_with_stdin(
        &["show"],
        br#"{"password_secret":"old","nested":{"API_KEY":"key"}}"#,
    );
    assert!(output.status.success());
    let response = json_stdout(&output);
    assert_eq!(response["result"]["value"]["password_secret"], "***");
    assert_eq!(response["result"]["value"]["nested"]["API_KEY"], "key");

    // Targeted get on a `_secret`-suffixed leaf is ALSO redacted by default —
    // unlike `agent-first-config`'s old `afconfig get KEY` (which returned
    // the raw targeted value regardless of secret naming).
    let output = run_with_stdin(&["get", "password_secret"], br#"{"password_secret":"old"}"#);
    assert!(output.status.success());
    assert_eq!(json_stdout(&output)["result"]["value"], "***");

    // A non-secret leaf is returned as-is.
    let output = run_with_stdin(
        &["get", "nested.API_KEY"],
        br#"{"nested":{"API_KEY":"key"}}"#,
    );
    assert!(output.status.success());
    assert_eq!(json_stdout(&output)["result"]["value"], "key");
}

#[test]
fn test_secret_name_redacts_show_and_targeted_get() {
    let stdin = br#"{"PASSWORD":"hunter2"}"#;

    let show = run_with_stdin(&["--secret-name", "PASSWORD", "show"], stdin);
    assert!(show.status.success());
    assert_eq!(json_stdout(&show)["result"]["value"]["PASSWORD"], "***");

    // New behavior: a targeted `get` on a `--secret-name` field is also
    // redacted (the old afconfig CLI left targeted `get` un-redacted).
    let get = run_with_stdin(&["--secret-name", "PASSWORD", "get", "PASSWORD"], stdin);
    assert!(get.status.success());
    assert_eq!(json_stdout(&get)["result"]["value"], "***");
}

// ═══════════════════════════════════════════
// Mutation: TOML source preservation, JSON/YAML keyed lists, nested
// dotted prefixes, missing-key insertion
// ═══════════════════════════════════════════

#[cfg(feature = "toml")]
#[test]
fn test_set_preserves_toml_formatting() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().join("config.toml");
    std::fs::write(
        &path,
        "# leading comment\nhost = \"example.com\"\nport = 993 # inline comment\n",
    )
    .unwrap();

    let output = run(&[
        "set",
        "port",
        "1024",
        "--input-file",
        path.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{:?}", output);
    let response = json_stdout(&output);
    assert_eq!(response["result"]["code"], "document_set");
    assert_eq!(response["result"]["format"], "TOML");
    assert_eq!(response["result"]["write_count"], 1);

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("# leading comment"));
    assert!(after.contains("port = 1024 # inline comment"));
}

#[cfg(feature = "toml")]
#[test]
fn test_set_missing_key_toml_and_yaml_preserve_comments() {
    let temp_dir = TempDir::new().unwrap();

    let toml_path = temp_dir.path().join("c.toml");
    std::fs::write(&toml_path, "a = 1\n\n[srv]\nhost = \"x\"  # keep\n").unwrap();
    let out = run(&[
        "set",
        "srv.port",
        "8080",
        "--input-file",
        toml_path.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{:?}", out);
    let after = std::fs::read_to_string(&toml_path).unwrap();
    assert!(after.contains("# keep"), "comment preserved: {after}");
    assert!(after.contains("port = 8080"), "new key inserted: {after}");

    #[cfg(feature = "yaml")]
    {
        let yaml_path = temp_dir.path().join("c.yaml");
        std::fs::write(&yaml_path, "a: 1\nsrv:\n  host: x\n").unwrap();
        let out = run(&[
            "set",
            "srv.port",
            "8080",
            "--input-file",
            yaml_path.to_str().unwrap(),
        ]);
        assert!(out.status.success(), "{:?}", out);
        let after = std::fs::read_to_string(&yaml_path).unwrap();
        assert_eq!(after, "a: 1\nsrv:\n  host: x\n  port: 8080\n");
    }
}

#[test]
fn test_json_keyed_collection_edits_preserve_document() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("config.json");
    let source =
        "{\n  \"items\": [\n    {\"id\": \"a\", \"name\": \"A\"}\n  ],\n  \"keep\": 1e+3\n}\n";
    std::fs::write(&config_path, source).unwrap();

    let added = run(&[
        "add",
        "items",
        "b",
        "--slug-field",
        "id",
        "name=B",
        "--input-file",
        config_path.to_str().unwrap(),
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

    let removed = run(&[
        "remove",
        "items",
        "b",
        "--slug-field",
        "id",
        "--input-file",
        config_path.to_str().unwrap(),
    ]);
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
    let config_path = temp_dir.path().join("config.json");
    let source = "{\n  \"cfg\": {\n    \"users\": [\n      {\"uid\": \"a\", \"role\": \"admin\"}\n    ]\n  }\n}\n";
    std::fs::write(&config_path, source).unwrap();

    let added = run(&[
        "add",
        "cfg.users",
        "bob",
        "--slug-field",
        "uid",
        "role=dev",
        "--input-file",
        config_path.to_str().unwrap(),
    ]);
    assert!(added.status.success(), "{:?}", added);
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(parsed["cfg"]["users"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["cfg"]["users"][1]["uid"], "bob");
    assert_eq!(parsed["cfg"]["users"][1]["role"], "dev");

    let removed = run(&[
        "remove",
        "cfg.users",
        "a",
        "--slug-field",
        "uid",
        "--input-file",
        config_path.to_str().unwrap(),
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
    let path = temp_dir.path().join("config.toml");
    let original = "value = 1\nkeep = 2\n";
    std::fs::write(&path, original).unwrap();
    let output = run(&[
        "set",
        "value",
        "j:null",
        "--input-file",
        path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
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
        let config_path = temp_dir.path().join(".env");
        let original = "KEY=value\n";
        std::fs::write(&config_path, original).unwrap();

        let mut args: Vec<&str> = (*arguments).to_vec();
        args.push("--input-file");
        let path_str = config_path.to_str().unwrap();
        args.push(path_str);
        let output = run(&args);
        assert_eq!(output.status.code(), Some(1));
        let response = json_stdout(&output);
        assert_eq!(response["error"]["code"], "document_error");
        // `add` reaches the format-capability check directly; `remove` first
        // resolves the (nonexistent, in this fixture) keyed-list path and
        // fails with "not found" before ever reaching that check.
        assert!(
            response["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("does not support") || m.contains("not found"))
        );
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), original);
    }

    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join(".env");
    let original = "# keep\nexport KEY=value # comment\nOTHER=unchanged\n";
    std::fs::write(&config_path, original).unwrap();
    let output = run(&[
        "set",
        "KEY",
        "changed",
        "--input-file",
        config_path.to_str().unwrap(),
    ]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        "# keep\nexport KEY=changed # comment\nOTHER=unchanged\n"
    );
}

#[cfg(feature = "dotenv")]
#[test]
fn test_dotenv_get_and_show() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join(".env");
    std::fs::write(&config_path, "KEY=value\nEMPTY=\n").unwrap();

    let get = run(&["get", "KEY", config_path.to_str().unwrap()]);
    assert!(get.status.success());
    assert_eq!(json_stdout(&get)["result"]["value"], "value");

    let show = run(&["show", config_path.to_str().unwrap()]);
    assert!(show.status.success());
    assert_eq!(json_stdout(&show)["result"]["value"]["EMPTY"], "");

    // A `${VAR}`-shaped value is a dotenv literal, never expanded from the
    // afdata process's own environment.
    std::fs::write(&config_path, "REFERENCE=${AFDATA_TEST_PROCESS_VALUE}\n").unwrap();
    let literal = Command::new(env!("CARGO_BIN_EXE_afdata"))
        .env("AFDATA_TEST_PROCESS_VALUE", "must-not-be-read")
        .args(["get", "REFERENCE", config_path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(literal.status.success());
    assert_eq!(
        json_stdout(&literal)["result"]["value"],
        "${AFDATA_TEST_PROCESS_VALUE}"
    );
}

// ═══════════════════════════════════════════
// Secret sources: --value-secret[-stdin|-prompt|-fd], exact round-trip,
// oversized/invalid-utf8 rejection, preflight-before-read ordering
// ═══════════════════════════════════════════

#[test]
fn test_secret_sources_and_exact_redaction() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("secrets.json");
    std::fs::write(
        &config_path,
        "{\"password_secret\":\"old\",\"nested\":{\"API_KEY\":\"key\"}}\n",
    )
    .unwrap();
    let set = run(&[
        "set",
        "password_secret",
        "--value-secret",
        "new-secret",
        "--input-file",
        config_path.to_str().unwrap(),
    ]);
    assert!(set.status.success());
    assert!(!String::from_utf8_lossy(&set.stdout).contains("new-secret"));

    let show = run(&[
        "--secret-name",
        "API_KEY",
        "show",
        config_path.to_str().unwrap(),
    ]);
    assert!(show.status.success());
    let response = json_stdout(&show);
    assert_eq!(response["result"]["value"]["password_secret"], "***");
    assert_eq!(response["result"]["value"]["nested"]["API_KEY"], "***");
}

#[test]
fn test_secret_round_trips_to_exact_value() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("secrets.json");
    std::fs::write(&config_path, "{\"api_key_secret\":\"old\"}\n").unwrap();
    let argv_secret = "s3kr3t-Ünïcode-#=";
    let set = run(&[
        "set",
        "api_key_secret",
        "--value-secret",
        argv_secret,
        "--input-file",
        config_path.to_str().unwrap(),
    ]);
    assert!(set.status.success());
    assert!(!String::from_utf8_lossy(&set.stdout).contains("s3kr3t"));

    let got = run(&[
        "value",
        "api_key_secret",
        "--reveal-secret",
        config_path.to_str().unwrap(),
    ]);
    assert!(got.status.success());
    assert_eq!(String::from_utf8_lossy(&got.stdout), argv_secret);
    let on_disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(on_disk["api_key_secret"], argv_secret);

    // stdin reads to EOF and preserves the trailing newline exactly.
    let output = run_with_stdin(
        &[
            "set",
            "api_key_secret",
            "--value-secret-stdin",
            "--input-file",
            config_path.to_str().unwrap(),
        ],
        b"piped\n",
    );
    assert!(output.status.success(), "{:?}", output);
    let got = run(&[
        "value",
        "api_key_secret",
        "--reveal-secret",
        config_path.to_str().unwrap(),
    ]);
    assert_eq!(got.stdout, b"piped\n");
}

#[test]
fn test_secret_stdin_oversized_and_invalid_utf8() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("secrets.json");
    std::fs::write(&config_path, "{\"password_secret\":\"old\"}\n").unwrap();

    let oversized = vec![b'x'; 1024 * 1024 + 1];
    let output = run_with_stdin(
        &[
            "set",
            "password_secret",
            "--value-secret-stdin",
            "--input-file",
            config_path.to_str().unwrap(),
        ],
        &oversized,
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("exceeds"));

    let output = run_with_stdin(
        &[
            "set",
            "password_secret",
            "--value-secret-stdin",
            "--input-file",
            config_path.to_str().unwrap(),
        ],
        &[0xff, b'\n'],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("UTF-8"));
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
            "token_secret",
            "--value-secret-stdin",
            "--input-file",
            linked.to_str().unwrap(),
        ],
        b"must-not-be-read\n",
    );
    assert_eq!(output.status.code(), Some(1));
    let response = json_stdout(&output);
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

// A full `--value-secret-fd` round trip (dup2'ing an open file onto a chosen
// descriptor via `pre_exec`) needs the `libc` crate as a dev-dependency,
// which this crate does not add just for a test; it was verified manually
// instead (see the task report's smoke commands). The descriptor-validation
// errors below need no real fd juggling, so they are covered here.

#[cfg(unix)]
#[test]
fn test_secret_fd_rejects_low_and_non_numeric_descriptors() {
    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("secrets.json");
    std::fs::write(&config_path, "{\"password_secret\":\"old\"}\n").unwrap();

    let error = run(&[
        "set",
        "password_secret",
        "--value-secret-fd",
        "2",
        "--input-file",
        config_path.to_str().unwrap(),
    ]);
    assert_eq!(error.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&error.stdout).contains("descriptor >= 3"));

    let error = run(&[
        "set",
        "password_secret",
        "--value-secret-fd",
        "nope",
        "--input-file",
        config_path.to_str().unwrap(),
    ]);
    assert_eq!(error.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&error.stdout).contains("numeric descriptor"));
}

// ═══════════════════════════════════════════
// File mode preservation, output formats, and argument conflicts
// ═══════════════════════════════════════════

#[cfg(unix)]
#[test]
fn test_set_preserves_file_mode() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().unwrap();
    let config_path = temp_dir.path().join("m.json");
    std::fs::write(&config_path, "{\"a\":1}\n").unwrap();
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o640)).unwrap();

    let out = run(&[
        "set",
        "a",
        "2",
        "--input-file",
        config_path.to_str().unwrap(),
    ]);
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
    let config_path = temp_dir.path().join("config.json");
    std::fs::write(&config_path, "{\"name\":\"demo\"}\n").unwrap();

    for output_format in ["yaml", "plain"] {
        let output = run(&[
            "--output",
            output_format,
            "get",
            "name",
            config_path.to_str().unwrap(),
        ]);
        assert!(output.status.success());
        assert!(!output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }

    let error = run(&[
        "set",
        "name",
        "--value-secret",
        "x",
        "ordinary",
        "--input-file",
        config_path.to_str().unwrap(),
    ]);
    assert_eq!(error.status.code(), Some(2));
    let response = json_stdout(&error);
    assert!(response["error"].is_object());
    assert!(error.stderr.is_empty());
}
