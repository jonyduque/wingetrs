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

#[tokio::test(flavor = "multi_thread")]
async fn deleted_catalog_with_fresh_http_state_is_rebuilt_by_search() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let s = MockServer::start().await;
    let ok = |p: &'static str, body: &'static [u8]| {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
    };
    ok(
        "/source2.msix",
        include_bytes!("fixtures/winget/source2-mini.msix"),
    )
    .mount(&s)
    .await;
    ok(
        "/main.tgz",
        include_bytes!("fixtures/scoop/Main-mini.tar.gz"),
    )
    .mount(&s)
    .await;
    Mock::given(method("GET"))
        .and(path("/extras.tgz"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let uri = s.uri();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_wingetrs"))
            .args(args)
            .env("LOCALAPPDATA", dir.path())
            .env("WINGETRS_WINGET_INDEX_URL", format!("{uri}/source2.msix"))
            .env("WINGETRS_SCOOP_BASE_URL", &uri)
            .output()
            .unwrap()
    };
    let (a, d) = (
        tokio::task::block_in_place(|| run(&["update", "--force"])),
        dir.path().join("wingetrs").join("catalog.db"),
    );
    assert!(a.status.success());
    std::fs::remove_file(&d).unwrap();
    let out = tokio::task::block_in_place(|| run(&["search", "", "-n", "100", "--json"]));
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 28);
    // The index shortcut must not leave the catalog missing.
    assert!(d.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn existing_but_empty_catalog_is_updated_by_search() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let s = MockServer::start().await;
    let ok = |p: &'static str, body: &'static [u8]| {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
    };
    ok(
        "/source2.msix",
        include_bytes!("fixtures/winget/source2-mini.msix"),
    )
    .mount(&s)
    .await;
    ok(
        "/main.tgz",
        include_bytes!("fixtures/scoop/Main-mini.tar.gz"),
    )
    .mount(&s)
    .await;
    Mock::given(method("GET"))
        .and(path("/extras.tgz"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("wingetrs");
    std::fs::create_dir_all(&data).unwrap();
    // The file exists (schema only, no rows): the early search starts, then the update must run.
    drop(wingetrs::catalog::Catalog::open(&data.join("catalog.db")).unwrap());
    let uri = s.uri();
    let out = tokio::task::block_in_place(|| {
        Command::new(env!("CARGO_BIN_EXE_wingetrs"))
            .args(["search", "", "-n", "100", "--json"])
            .env("LOCALAPPDATA", dir.path())
            .env("WINGETRS_WINGET_INDEX_URL", format!("{uri}/source2.msix"))
            .env("WINGETRS_SCOOP_BASE_URL", &uri)
            .output()
            .unwrap()
    });
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 28);
}
