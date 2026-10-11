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
    let conn =
        rusqlite::Connection::open_with_flags(&d, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM packages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 28);
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

#[test]
fn list_without_catalog_and_without_network_fails_cleanly_and_fast() {
    let dir = tempfile::tempdir().unwrap();
    let t = std::time::Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_wingetrs"))
        .args(["list"])
        .env("LOCALAPPDATA", dir.path())
        .env("WINGETRS_WINGET_INDEX_URL", "http://127.0.0.1:9/x")
        .env("WINGETRS_SCOOP_BASE_URL", "http://127.0.0.1:9")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("sem catálogo local e sem rede"), "{err}");
    assert!(!err.contains("panicked"), "{err}");
    // Rule: never hangs on the network. Connection refused returns at once; the bound is only
    // a generous hang detector (a real hang would be the 30 s+ connect/lock timeouts).
    assert!(t.elapsed() < std::time::Duration::from_secs(60));
}

fn spawn_update_with_lock_held(
    args: &[&str],
    timeout_ms: &str,
    root: &std::path::Path,
) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_wingetrs"))
        .args(args)
        .env("LOCALAPPDATA", root)
        .env("WINGETRS_LOCK_TIMEOUT_MS", timeout_ms)
        .env("WINGETRS_WINGET_INDEX_URL", "http://127.0.0.1:9/x")
        .env("WINGETRS_SCOOP_BASE_URL", "http://127.0.0.1:9")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
}

fn update_with_lock_held(args: &[&str], timeout_ms: &str) -> (std::process::Output, bool) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("wingetrs");
    std::fs::create_dir_all(&root).unwrap();
    let held = wingetrs::cache::UpdateLock::try_acquire(&root.join("update.lock"))
        .unwrap()
        .unwrap();
    let mut child = spawn_update_with_lock_held(args, timeout_ms, dir.path());
    // Ordering, not speed: did the child exit while we still held the lock?
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut exited_while_held = false;
    while std::time::Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            exited_while_held = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if !exited_while_held {
        child.kill().unwrap();
    }
    drop(held);
    (child.wait_with_output().unwrap(), exited_while_held)
}

#[test]
fn background_update_gives_up_at_once_when_another_update_holds_the_lock() {
    // The lock timeout knob is 10 minutes: a background run that waited would still be blocked
    // when the 60 s hang detector fires, while the lock is held the whole time.
    let (out, exited_while_held) = update_with_lock_held(&["update", "--background"], "600000");
    assert!(
        exited_while_held,
        "background update waited for the lock instead of giving up"
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn foreground_update_still_waits_for_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("wingetrs");
    std::fs::create_dir_all(&root).unwrap();
    let held = wingetrs::cache::UpdateLock::try_acquire(&root.join("update.lock"))
        .unwrap()
        .unwrap();
    let mut child = spawn_update_with_lock_held(&["update"], "600000", dir.path());
    // While we hold the lock the child must stay blocked (a lower bound on time cannot flake).
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(
        child.try_wait().unwrap().is_none(),
        "foreground update did not wait for the held lock"
    );
    drop(held);
    let out = child.wait_with_output().unwrap();
    // After the release it got the lock and ran (source errors are reported, not fatal), not "busy".
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("outra atualização em andamento"), "{err}");
}
