#![allow(clippy::unwrap_used, clippy::expect_used)]
mod spans;

use wingetrs::cache::{DataDir, HttpEntry, HttpState};
use wingetrs::catalog::Catalog;
use wingetrs::http::Http;
use wingetrs::update::{Endpoints, LockWait, Outcome, UpdateError, needs_refresh, run_update};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MSIX: &[u8] = include_bytes!("fixtures/winget/source2-mini.msix");
const TGZ: &[u8] = include_bytes!("fixtures/scoop/Main-mini.tar.gz");

async fn setup() -> (MockServer, Endpoints) {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/source2.msix"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(MSIX.to_vec())
                .insert_header("ETag", "\"w1\"")
                .insert_header("x-ms-meta-SourceVersion", "2026.1007.459.0"),
        )
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/main.tgz"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(TGZ.to_vec())
                .insert_header("ETag", "\"s1\""),
        )
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/extras.tgz"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&s)
        .await;
    let ep = Endpoints {
        winget_index: format!("{}/source2.msix", s.uri()),
        scoop: vec![
            ("main".into(), format!("{}/main.tgz", s.uri())),
            ("extras".into(), format!("{}/extras.tgz", s.uri())),
        ],
    };
    (s, ep)
}

#[tokio::test]
async fn update_imports_sources_and_isolates_failure() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let rep = run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        true,
        LockWait::from_env(),
    )
    .await
    .unwrap();
    let get = |s: &str| rep.iter().find(|r| r.source == s).unwrap().outcome.clone();
    assert_eq!(
        get("winget"),
        Outcome::Updated {
            packages: 20,
            skipped: 0
        }
    );
    assert_eq!(
        get("scoop:main"),
        Outcome::Updated {
            packages: 8,
            skipped: 1
        }
    );
    assert!(matches!(
        get("scoop:extras"),
        Outcome::Failed { offline: false, .. }
    ));
    let cat = Catalog::open(&data.catalog_path()).unwrap();
    assert_eq!(cat.load_entries().unwrap().len(), 28);
    let meta = cat.sources().unwrap();
    assert_eq!(
        meta.iter()
            .find(|m| m.source == "winget")
            .unwrap()
            .source_version,
        "2026.1007.459.0"
    );
    let st = HttpState::load(&data.http_state_path());
    assert_eq!(
        st.get(&ep.winget_index).unwrap().etag.as_deref(),
        Some("\"w1\"")
    );
}

#[tokio::test]
async fn second_update_without_force_is_fresh_and_offline_safe() {
    let (s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        true,
        LockWait::from_env(),
    )
    .await
    .unwrap();
    drop(s); // server gone: any network call would fail
    let rep = run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        false,
        LockWait::from_env(),
    )
    .await
    .unwrap();
    assert_eq!(
        rep.iter().find(|r| r.source == "winget").unwrap().outcome,
        Outcome::Fresh
    );
    assert_eq!(
        Catalog::open(&data.catalog_path())
            .unwrap()
            .load_entries()
            .unwrap()
            .len(),
        28
    );
}

#[tokio::test]
async fn concurrent_update_is_busy() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let _held = wingetrs::cache::UpdateLock::try_acquire(&data.lock_path())
        .unwrap()
        .unwrap();
    // run_update waits up to 30 s; the test must not take that long, so use the
    // test-only knob described in the brief: WINGETRS_LOCK_TIMEOUT_MS
    unsafe { std::env::set_var("WINGETRS_LOCK_TIMEOUT_MS", "200") };
    let e = run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        true,
        LockWait::from_env(),
    )
    .await
    .unwrap_err();
    assert_eq!(e.to_string(), "outra atualização em andamento");
}

#[tokio::test]
async fn no_wait_returns_busy_immediately() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let _held = wingetrs::cache::UpdateLock::try_acquire(&data.lock_path())
        .unwrap()
        .unwrap();
    let t = std::time::Instant::now();
    let e = run_update(&data, &ep, &Http::new().unwrap(), true, LockWait::NoWait)
        .await
        .unwrap_err();
    assert!(matches!(e, UpdateError::Busy), "{e:?}");
    assert!(
        t.elapsed() < std::time::Duration::from_millis(100),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn needs_refresh_rule() {
    let mut st = HttpState::default();
    assert!(needs_refresh(&st, &["u"], 10_000), "never checked");
    st.set(
        "u",
        HttpEntry {
            checked_at: 10_000 - 899,
            ..Default::default()
        },
    );
    assert!(!needs_refresh(&st, &["u"], 10_000));
    st.set(
        "u",
        HttpEntry {
            checked_at: 10_000 - 901,
            ..Default::default()
        },
    );
    assert!(needs_refresh(&st, &["u"], 10_000));
}

async fn single_scoop_outcome(url: String) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let ep = Endpoints {
        // Winget index is fresh-skipped only when not forced; point it at the same bad url.
        winget_index: url.clone(),
        scoop: vec![("main".into(), url)],
    };
    let rep = run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        true,
        LockWait::from_env(),
    )
    .await
    .unwrap();
    rep.into_iter()
        .find(|r| r.source == "scoop:main")
        .unwrap()
        .outcome
}

#[tokio::test]
async fn corrupt_tarball_failure_is_not_offline() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/main.tgz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"not a tarball".to_vec()))
        .mount(&s)
        .await;
    let o = single_scoop_outcome(format!("{}/main.tgz", s.uri())).await;
    assert!(matches!(o, Outcome::Failed { offline: false, .. }), "{o:?}");
}

#[tokio::test]
async fn unreachable_endpoint_failure_is_offline() {
    let o = single_scoop_outcome("http://127.0.0.1:9/main.tgz".into()).await;
    assert!(matches!(o, Outcome::Failed { offline: true, .. }), "{o:?}");
}

#[tokio::test]
async fn update_writes_search_index_and_rebuilds_it_when_missing() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        true,
        wingetrs::update::LockWait::from_env(),
    )
    .await
    .unwrap();
    let bytes = std::fs::read(data.search_index_path()).unwrap();
    assert_eq!(
        wingetrs::catalog::index_file::IndexView::parse(&bytes)
            .unwrap()
            .len(),
        28
    );
    std::fs::remove_file(data.search_index_path()).unwrap();
    // Fresh (no network needed): the missing index alone must trigger a rebuild.
    run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        false,
        wingetrs::update::LockWait::from_env(),
    )
    .await
    .unwrap();
    assert!(data.search_index_path().exists());
}

#[tokio::test]
async fn update_emits_a_fetch_span_per_source_and_parse_only_for_downloaded_ones() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let closed = spans::Closed::default();
    let _g = closed.install();
    run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        true,
        LockWait::from_env(),
    )
    .await
    .unwrap();
    assert_eq!(closed.count("update.lock"), 1, "{:?}", closed.names());
    assert_eq!(closed.count("update.fetch"), 3, "{:?}", closed.names());
    assert_eq!(closed.count("update.parse"), 2, "extras falha antes de ler");
    assert_eq!(closed.count("update.catalog_write"), 2);
    assert_eq!(closed.count("update.state_save"), 1);
    assert_eq!(closed.count("update.search_idx"), 1);
}
