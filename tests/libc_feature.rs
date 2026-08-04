#[cfg(all(feature = "stream-redirect", not(feature = "libc")))]
compile_error!("the `stream-redirect` feature must enable the `libc` feature");

#[cfg(all(unix, not(feature = "libc")))]
#[test]
fn capped_read_without_libc_keeps_follow_and_refuses_no_follow() {
    use agent_first_data::document::{DocumentFile, SymlinkPolicy};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, r#"{"k": "v"}"#).unwrap();

    assert!(
        DocumentFile::open_capped_with_policy(&path, None, 1024, SymlinkPolicy::Follow).is_ok()
    );
    let error = DocumentFile::open_capped_with_policy(&path, None, 1024, SymlinkPolicy::NoFollow)
        .unwrap_err();
    assert_eq!(error.code(), "document_unsupported_operation");
    assert!(error.to_string().contains("Cargo feature `libc`"));
}

/// Without this, every test in the file is `cfg(unix)` and the Windows CI leg
/// compiles a target with zero tests that passes while proving nothing. The
/// non-unix `NoFollow` arm is only reachable here, so it is only ever executed
/// here.
#[cfg(not(unix))]
#[test]
fn capped_read_on_non_unix_refuses_no_follow_and_allows_follow() {
    use agent_first_data::document::{DocumentFile, SymlinkPolicy};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, r#"{"k": "v"}"#).unwrap();

    assert!(
        DocumentFile::open_capped_with_policy(&path, None, 1024, SymlinkPolicy::Follow).is_ok()
    );
    let error = DocumentFile::open_capped_with_policy(&path, None, 1024, SymlinkPolicy::NoFollow)
        .unwrap_err();
    assert_eq!(error.code(), "document_unsupported_operation");
}

#[cfg(all(unix, feature = "libc"))]
#[test]
fn capped_read_with_libc_atomically_refuses_a_symlink() {
    use agent_first_data::document::{DocumentFile, SymlinkPolicy};

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.json");
    let link = dir.path().join("link.json");
    std::fs::write(&target, r#"{"k": "v"}"#).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    assert!(
        DocumentFile::open_capped_with_policy(&link, None, 1024, SymlinkPolicy::Follow).is_ok()
    );
    let error = DocumentFile::open_capped_with_policy(&link, None, 1024, SymlinkPolicy::NoFollow)
        .unwrap_err();
    assert_eq!(error.code(), "document_io_failed");
}
