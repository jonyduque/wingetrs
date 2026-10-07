#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::process::Command;

#[test]
fn search_without_catalog_and_without_network_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_wingetrs"))
        .args(["search", "foo"])
        .env("LOCALAPPDATA", dir.path())
        .env("WINGETRS_WINGET_INDEX_URL", "http://127.0.0.1:9/x")
        .env("WINGETRS_SCOOP_BASE_URL", "http://127.0.0.1:9")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("sem catálogo local e sem rede"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
}
