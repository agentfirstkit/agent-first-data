#![cfg(feature = "cli")]
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]
//! CLI integration tests for `afdata guard <TYPE> <VALUE>`.
//!
//! These are black-box, subprocess-based (unlike `cli/src/guard.rs`'s own
//! `#[cfg(test)]` unit tests, which inject a fabricated cwd/home/temp-roots
//! and never touch this process's real environment). Here the point is the
//! opposite: prove the *real* `std::env::current_dir()` / `$HOME` /
//! platform-temp-area reading in `guard::guard_from_environment` behaves
//! correctly, and that spoofing `TMPDIR`/`TMP`/`TEMP` on the *child* process
//! (safe — it never touches this test process's own environment) changes
//! nothing about `tmp_path`'s verdict.

use std::path::Path;
use std::process::Command;

fn afdata() -> Command {
    Command::new(env!("CARGO_BIN_EXE_afdata"))
}

fn run(args: &[&str]) -> std::process::Output {
    afdata().args(args).output().expect("failed to run afdata")
}

fn json_stderr(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stderr)
        .unwrap_or_else(|err| panic!("stderr is not JSON: {err}: {:?}", output.stderr))
}

fn error_code(output: &std::process::Output) -> String {
    json_stderr(output)["error"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("no error.code in {:?}", output.stderr))
        .to_string()
}

/// A guarded path always comes back as one absolute path with no trailing
/// newline (design: "raw bytes,无尾随换行").
fn assert_guard_ok(output: &std::process::Output, expected: &Path) {
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout is not UTF-8");
    assert!(!stdout.ends_with('\n'), "{stdout:?}");
    assert_eq!(Path::new(&stdout), plain_form(expected), "{stdout:?}");
}

/// Callers build their expectations with `std::fs::canonicalize`, which on
/// Windows answers in the `\\?\` verbatim form. The command deliberately does
/// not: that spelling is refused by the shell's own item API and reads as a UNC
/// share to .NET, so a guarded operand carrying it is one no downstream verb can
/// use. The contract itself is pinned in the unit test
/// `windows_output_is_the_drive_form_not_the_verbatim_one`; this only keeps the
/// expectations here speaking the same language.
#[cfg(windows)]
fn plain_form(path: &Path) -> std::path::PathBuf {
    use std::ffi::OsString;
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path.to_path_buf();
    };
    let Prefix::VerbatimDisk(letter) = prefix.kind() else {
        return path.to_path_buf();
    };
    let mut plain = OsString::from(format!("{}:", letter as char));
    plain.push(components.as_path().as_os_str());
    std::path::PathBuf::from(plain)
}

#[cfg(not(windows))]
fn plain_form(path: &Path) -> &Path {
    path
}

fn assert_guard_failed(output: &std::process::Output, expected_code: &str) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        output.stdout.is_empty(),
        "guard leaked to stdout on failure: {output:?}"
    );
    assert_eq!(error_code(output), expected_code, "{output:?}");
}

// ═══════════════════════════════════════════
// `path`: base checks and the shared reject set
// ═══════════════════════════════════════════

#[test]
fn path_type_accepts_a_real_directory_outside_the_reject_set() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let output = run(&["guard", "path", elsewhere.to_str().unwrap()]);
    let expected = std::fs::canonicalize(&elsewhere).unwrap();
    assert_guard_ok(&output, &expected);
}

#[test]
fn empty_value_is_rejected_without_touching_the_filesystem() {
    let output = run(&["guard", "path", ""]);
    assert_guard_failed(&output, "guard_empty_value");
}

#[test]
fn value_containing_a_newline_is_rejected() {
    let output = run(&["guard", "path", "line-one\nline-two"]);
    assert_guard_failed(&output, "guard_contains_newline");
}

#[test]
fn an_all_whitespace_value_is_rejected_as_empty() {
    let output = run(&["guard", "path", "   "]);
    assert_guard_failed(&output, "guard_empty_value");
}

#[test]
fn a_value_starting_with_a_dash_survives_the_argv_boundary_as_an_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    let dashed = dir.path().join("-rf");
    std::fs::create_dir(&dashed).unwrap();

    // Bare, it is argv's problem before it is the guard's: the closed-world
    // parser reads it as an unknown short flag and fails closed.
    let bare = afdata()
        .current_dir(dir.path())
        .args(["guard", "path", "-rf"])
        .output()
        .unwrap();
    assert_eq!(bare.status.code(), Some(2), "{bare:?}");
    assert!(bare.stdout.is_empty(), "{bare:?}");

    // Passed as a value, it comes back absolute — which is the whole point:
    // no output of this command can ever be read as an option by the verb
    // downstream of it.
    let separated = afdata()
        .current_dir(dir.path())
        .args(["guard", "path", "--", "-rf"])
        .output()
        .unwrap();
    assert_guard_ok(&separated, &std::fs::canonicalize(&dashed).unwrap());
}

#[test]
fn a_value_with_spaces_and_a_quote_passes_through_intact() {
    let dir = tempfile::tempdir().unwrap();
    let awkward = dir.path().join("a b'c d");
    std::fs::create_dir(&awkward).unwrap();
    let output = run(&["guard", "path", awkward.to_str().unwrap()]);
    assert_guard_ok(&output, &std::fs::canonicalize(&awkward).unwrap());
}

#[test]
fn a_home_reached_through_a_symlink_is_still_in_the_reject_set() {
    let dir = tempfile::tempdir().unwrap();
    let real_home = dir.path().join("real_home");
    std::fs::create_dir(&real_home).unwrap();
    let home_link = dir.path().join("home_link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_home, &home_link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&real_home, &home_link).unwrap();

    // `$HOME` handed out through a symlink is routine (a CI or sandbox home
    // under a symlinked temp area). The reject set must still recognize the
    // real directory it names.
    let output = afdata()
        .env(
            if cfg!(windows) { "USERPROFILE" } else { "HOME" },
            &home_link,
        )
        .args(["guard", "path", real_home.to_str().unwrap()])
        .output()
        .unwrap();
    assert_guard_failed(&output, "guard_rejected_target");
}

#[test]
fn filesystem_root_is_rejected() {
    #[cfg(unix)]
    let root = "/";
    #[cfg(windows)]
    let root = "C:\\";
    let output = run(&["guard", "path", root]);
    assert_guard_failed(&output, "guard_rejected_target");
}

#[test]
fn home_directory_and_its_parent_are_rejected() {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from);
    let Some(home) = home else {
        // No home directory in this environment (e.g. a stripped container) —
        // there is nothing to assert, and skipping here is not a vacuous pass:
        // the reject-set behavior for a *present* HOME is covered by every
        // other invocation in this file inheriting the test runner's real one.
        return;
    };
    let output = run(&["guard", "path", home.to_str().unwrap()]);
    assert_guard_failed(&output, "guard_rejected_target");

    if let Some(parent) = home.parent() {
        let output = run(&["guard", "path", parent.to_str().unwrap()]);
        assert_guard_failed(&output, "guard_rejected_target");
    }
}

#[test]
fn cwd_itself_and_its_parent_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("nested");
    std::fs::create_dir(&nested).unwrap();

    let output = afdata()
        .current_dir(&nested)
        .args(["guard", "cwd_path", "."])
        .output()
        .unwrap();
    assert_guard_failed(&output, "guard_rejected_target");

    let output = afdata()
        .current_dir(&nested)
        .args(["guard", "path", nested.to_str().unwrap()])
        .output()
        .unwrap();
    assert_guard_failed(&output, "guard_rejected_target");

    let output = afdata()
        .current_dir(&nested)
        .args(["guard", "path", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_guard_failed(&output, "guard_rejected_target");
}

// ═══════════════════════════════════════════
// `cwd_path`
// ═══════════════════════════════════════════

#[test]
fn cwd_path_accepts_children_and_rejects_outsiders() {
    let cwd_dir = tempfile::tempdir().unwrap();
    let child = cwd_dir.path().join("child");
    std::fs::create_dir(&child).unwrap();

    let inside = afdata()
        .current_dir(cwd_dir.path())
        .args(["guard", "cwd_path", "child"])
        .output()
        .unwrap();
    let expected = std::fs::canonicalize(&child).unwrap();
    assert_guard_ok(&inside, &expected);

    let outside_dir = tempfile::tempdir().unwrap();
    let outside = afdata()
        .current_dir(cwd_dir.path())
        .args(["guard", "cwd_path", outside_dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_guard_failed(&outside, "guard_outside_containment");
}

#[test]
fn cwd_path_follows_a_mid_script_cd() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first");
    let second = root.path().join("second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let first_child = first.join("only-under-first");
    std::fs::create_dir(&first_child).unwrap();

    // A value that is a real child of `first` must be rejected once the
    // process's cwd has moved to `second` — `cwd_path` tracks the *current*
    // directory, not whichever one the script started in (design: "不做
    // 项目根推断").
    let output = afdata()
        .current_dir(&second)
        .args(["guard", "cwd_path", first_child.to_str().unwrap()])
        .output()
        .unwrap();
    assert_guard_failed(&output, "guard_outside_containment");
}

// ═══════════════════════════════════════════
// `tmp_path` and environment independence
// ═══════════════════════════════════════════

/// A manually created (not `tempfile`-tracked) directory under one of this
/// platform's *fixed* system temp roots, removed on drop. Distinct from
/// `tempfile::tempdir()`, which resolves through `std::env::temp_dir()` —
/// itself `TMPDIR`/`TMP`/`TEMP`-sensitive — and so cannot be trusted to land
/// under a recognized root once a test starts spoofing those variables.
struct RealSystemTempChild {
    path: std::path::PathBuf,
}

impl RealSystemTempChild {
    fn new(name: &str) -> Self {
        #[cfg(target_os = "macos")]
        let base = Path::new("/private/tmp");
        #[cfg(all(unix, not(target_os = "macos")))]
        let base = Path::new("/tmp");
        #[cfg(windows)]
        let base = Path::new(r"C:\Windows\Temp");
        let path = base.join(format!("afdata-guard-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap_or_else(|err| {
            panic!("failed to create fixture under the real system temp area {base:?}: {err}")
        });
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RealSystemTempChild {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn tmp_path_recognizes_the_real_temp_area_regardless_of_spoofed_tmpdir_env() {
    let real = RealSystemTempChild::new("real-root");
    let child = real.path().join("work");
    std::fs::create_dir(&child).unwrap();
    let expected = std::fs::canonicalize(&child).unwrap();

    let spoofed_target = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from);

    for tmpdir_override in [
        spoofed_target
            .as_deref()
            .map(|p| p.to_str().unwrap().to_string()),
        Some(String::new()),
    ]
    .into_iter()
    .flatten()
    {
        let output = afdata()
            .env("TMPDIR", &tmpdir_override)
            .env("TMP", &tmpdir_override)
            .env("TEMP", &tmpdir_override)
            .args(["guard", "tmp_path", child.to_str().unwrap()])
            .output()
            .unwrap();
        assert_guard_ok(&output, &expected);
    }

    let output = afdata()
        .env_remove("TMPDIR")
        .env_remove("TMP")
        .env_remove("TEMP")
        .args(["guard", "tmp_path", child.to_str().unwrap()])
        .output()
        .unwrap();
    assert_guard_ok(&output, &expected);
}

#[test]
fn tmp_path_rejects_a_mktemp_result_from_a_tmpdir_pointed_at_home() {
    let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(std::path::PathBuf::from)
    else {
        return;
    };
    // Simulate `TMPDIR=$HOME mktemp -d`: a real directory, physically outside
    // every recognized system temp root, that a spoofed `TMPDIR` would make a
    // naive (env-trusting) implementation accept.
    let fake_mktemp_result = home.join(format!("afdata-guard-e2e-fake-tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&fake_mktemp_result);
    std::fs::create_dir(&fake_mktemp_result).unwrap();

    let output = afdata()
        .env("TMPDIR", &home)
        .env("TMP", &home)
        .env("TEMP", &home)
        .args(["guard", "tmp_path", fake_mktemp_result.to_str().unwrap()])
        .output()
        .unwrap();

    std::fs::remove_dir_all(&fake_mktemp_result).unwrap();
    assert_guard_failed(&output, "guard_outside_containment");
}

#[test]
fn tmp_path_rejects_a_real_directory_outside_every_root() {
    // A real, existing subdirectory of the checkout: never a system temp
    // root, and (unlike `CARGO_MANIFEST_DIR` itself, which is the test
    // process's inherited cwd) not in the shared reject set either, so this
    // isolates the `tmp_path` containment check specifically.
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("cli");
    let output = run(&["guard", "tmp_path", target.to_str().unwrap()]);
    assert_guard_failed(&output, "guard_outside_containment");
}

#[cfg(target_os = "macos")]
#[test]
fn tmp_path_recognizes_var_tmp_through_its_symlink_to_private_var_tmp() {
    // No fixture is created: containment only needs the existing prefix
    // (`/var/tmp`, itself a symlink to `/private/var/tmp` on every real
    // macOS system) to resolve; the nonexistent leaf passes lexically.
    let output = run(&[
        "guard",
        "tmp_path",
        "/var/tmp/afdata-guard-e2e-nonexistent-probe",
    ]);
    let expected = Path::new("/private/var/tmp/afdata-guard-e2e-nonexistent-probe");
    assert_guard_ok(&output, expected);
}

// ═══════════════════════════════════════════
// Symlinks: final segment is guarded as itself
// ═══════════════════════════════════════════

#[cfg(unix)]
#[test]
fn final_symlink_segment_is_guarded_not_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let target_dir = dir.path().join("target_dir");
    std::fs::create_dir(&target_dir).unwrap();
    let link = dir.path().join("link_name");
    std::os::unix::fs::symlink(&target_dir, &link).unwrap();

    let output = run(&["guard", "path", link.to_str().unwrap()]);
    let canonical_dir = std::fs::canonicalize(dir.path()).unwrap();
    assert_guard_ok(&output, &canonical_dir.join("link_name"));
}

#[cfg(unix)]
#[test]
fn a_symlink_inside_the_temp_area_pointing_out_of_it_is_rejected() {
    let real = RealSystemTempChild::new("escape-link");
    // An existing directory of the checkout, not `tempfile::tempdir()`: the
    // latter resolves through `std::env::temp_dir()` and so would land under
    // a recognized temp root, which is not an escape at all.
    let escape_target = Path::new(env!("CARGO_MANIFEST_DIR")).join("cli");
    let link = real.path().join("escape_link");
    std::os::unix::fs::symlink(&escape_target, &link).unwrap();

    // The link itself is inside the temp area, so containment holds for `rm`.
    // But `>`/`chmod`/`cp` follow it, and would then act on a directory the
    // `tmp_path` type promised was out of reach.
    let output = run(&["guard", "tmp_path", link.to_str().unwrap()]);
    assert_guard_failed(&output, "guard_symlink_escapes_containment");
    assert!(escape_target.exists());
}

// ═══════════════════════════════════════════
// `--under`: a containment root the caller names
// ═══════════════════════════════════════════

#[test]
fn under_root_accepts_a_child_and_rejects_an_outsider() {
    let anchor = tempfile::tempdir().unwrap();
    let child = anchor.path().join("build");
    std::fs::create_dir(&child).unwrap();

    let inside = run(&[
        "guard",
        "path",
        child.to_str().unwrap(),
        "--under",
        anchor.path().to_str().unwrap(),
    ]);
    assert_guard_ok(&inside, &std::fs::canonicalize(&child).unwrap());

    let outside = tempfile::tempdir().unwrap();
    let rejected = run(&[
        "guard",
        "path",
        outside.path().to_str().unwrap(),
        "--under",
        anchor.path().to_str().unwrap(),
    ]);
    assert_guard_failed(&rejected, "guard_outside_containment");
}

#[test]
fn a_blank_under_root_is_rejected_rather_than_silently_meaning_cwd() {
    let anchor = tempfile::tempdir().unwrap();
    let child = anchor.path().join("build");
    std::fs::create_dir(&child).unwrap();

    // The accident this exists for: `--under "$UNSET_VAR"`.
    for blank in ["", "   "] {
        let output = afdata()
            .current_dir(anchor.path())
            .args(["guard", "path", child.to_str().unwrap(), "--under", blank])
            .output()
            .unwrap();
        assert_guard_failed(&output, "guard_invalid_root");
    }
}

/// The accident that motivated refusing `..` outright, taken from a real
/// dogfood site: a caller removing `"$dir/$name"` anchors `--under` at a root
/// *above* `$dir`, and a `$name` of `..` then walks out of `$dir` while
/// staying inside the root. Containment reports no violation because none
/// occurred — the operand really is under the root. It is simply not the
/// directory the caller was addressing.
#[test]
fn a_traversal_segment_is_refused_before_normalization_can_launder_it() {
    let anchor = tempfile::tempdir().unwrap();
    let inner = anchor.path().join("public");
    std::fs::create_dir(&inner).unwrap();

    let escaped = inner.join("..");
    let output = run(&[
        "guard",
        "path",
        escaped.to_str().unwrap(),
        "--under",
        anchor.path().to_str().unwrap(),
    ]);
    assert_guard_failed(&output, "guard_traversal_segment");

    // Naming the same directory directly is not what is being refused, so the
    // rule cannot be satisfied by accident: this one is refused for its own
    // reason, being the containment root itself.
    let root_itself = run(&[
        "guard",
        "path",
        anchor.path().to_str().unwrap(),
        "--under",
        anchor.path().to_str().unwrap(),
    ]);
    assert_guard_failed(&root_itself, "guard_outside_containment");
}

#[test]
fn a_traversal_segment_is_refused_for_every_type_and_position() {
    let anchor = tempfile::tempdir().unwrap();
    let inner = anchor.path().join("nested");
    std::fs::create_dir(&inner).unwrap();

    for value in [
        inner.join("..").to_str().unwrap().to_string(),
        inner.join("../sibling").to_str().unwrap().to_string(),
        "../relative".to_string(),
    ] {
        for guard_type in ["path", "tmp_path", "cwd_path"] {
            let output = afdata()
                .current_dir(anchor.path())
                .args(["guard", guard_type, &value])
                .output()
                .unwrap();
            assert_guard_failed(&output, "guard_traversal_segment");
        }
    }
}

/// ROOT is a position to compare against, not an operand a verb acts on, and
/// it is resolved in full — so traversal inside it changes nothing about what
/// is protected and must not be refused. Pinned so the VALUE rule is not
/// widened to ROOT by symmetry.
#[test]
fn a_traversal_segment_inside_the_under_root_is_accepted() {
    let anchor = tempfile::tempdir().unwrap();
    let inner = anchor.path().join("public");
    let child = inner.join("blog");
    std::fs::create_dir_all(&child).unwrap();

    let root_via_traversal = inner.join("..").join("public");
    let output = run(&[
        "guard",
        "path",
        child.to_str().unwrap(),
        "--under",
        root_via_traversal.to_str().unwrap(),
    ]);
    assert_guard_ok(&output, &std::fs::canonicalize(&child).unwrap());
}

// ═══════════════════════════════════════════
// CLI-shape contract: strict single value, closed TYPE vocabulary
// ═══════════════════════════════════════════

#[test]
fn extra_positional_arguments_are_a_usage_error() {
    let output = run(&["guard", "path", "one", "two"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        error_code(&output),
        "cli_unexpected_positional",
        "{output:?}"
    );
}

#[test]
fn missing_value_argument_is_a_usage_error() {
    let output = run(&["guard", "path"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
}

#[test]
fn unregistered_type_is_a_usage_error() {
    let output = run(&["guard", "bogus", "/tmp/x"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_eq!(
        error_code(&output),
        "cli_invalid_argument_value",
        "{output:?}"
    );
}

#[test]
fn guard_rejects_a_non_default_output_argument() {
    let output = run(&["guard", "path", "/tmp", "--output", "json"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        error_code(&output),
        "cli_unregistered_combination",
        "{output:?}"
    );
}

#[test]
fn guard_rejects_a_non_default_output_to_argument() {
    let output = run(&["guard", "path", "/tmp", "--output-to", "stdout"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(
        error_code(&output),
        "cli_unregistered_combination",
        "{output:?}"
    );
}

// ═══════════════════════════════════════════
// Regression probe: a rejected target is provably untouched
// ═══════════════════════════════════════════

#[test]
fn a_rejected_target_is_not_named_in_stdout_for_a_downstream_verb_to_consume() {
    let dir = tempfile::tempdir().unwrap();
    let sentinel = dir.path().join("sentinel.txt");
    std::fs::write(&sentinel, b"do not delete me").unwrap();

    // `dir` is not itself rejected (it is a real directory outside the
    // reject set), so use its ancestor chain by pointing cwd *at* the
    // directory whose deletion we want to prove `guard` would never enable:
    // cwd itself is always in the reject set.
    let output = afdata()
        .current_dir(dir.path())
        .args(["guard", "cwd_path", "."])
        .output()
        .unwrap();
    assert_guard_failed(&output, "guard_rejected_target");
    assert!(
        sentinel.exists(),
        "fixture must survive a rejected guard call"
    );
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"do not delete me");
}
