#![allow(clippy::unwrap_used, clippy::expect_used)]
use crate::spans;

use wingetrs::cache::{DataDir, HttpEntry, HttpState};
use wingetrs::catalog::Catalog;
use wingetrs::http::Http;
use wingetrs::update::{
    Endpoints, LockWait, Outcome, RETRY_SECS, UpdateError, background_due,
    claim_background_refresh, needs_refresh, run_update,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MSIX: &[u8] = include_bytes!("../fixtures/winget/source2-mini.msix");
const TGZ: &[u8] = include_bytes!("../fixtures/scoop/Main-mini.tar.gz");

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
    // The lock stays held for the whole call, so a run that waited (default 30 s) could only end
    // by the 60 s hang detector; no millisecond threshold.
    let e = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_update(&data, &ep, &Http::new().unwrap(), true, LockWait::NoWait),
    )
    .await
    .expect("NoWait must not wait for the held lock")
    .unwrap_err();
    assert!(matches!(e, UpdateError::Busy), "{e:?}");
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
    assert_eq!(
        closed.parents("update.parse"),
        ["update.fetch", "update.fetch"],
        "parse must run inside its fetch span"
    );
    assert_eq!(closed.count("update.catalog_write"), 2);
    assert_eq!(closed.count("update.state_save"), 1);
    assert_eq!(closed.count("update.search_idx"), 1);
}

#[test]
fn background_refresh_backs_off_after_an_attempt() {
    let stale = HttpState::default();
    let now = 10_000;
    assert!(background_due(&stale, &["u"], None, now), "nunca tentou");
    assert!(
        !background_due(&stale, &["u"], Some(now), now),
        "tentativa agora: espera"
    );
    assert!(!background_due(
        &stale,
        &["u"],
        Some(now - (RETRY_SECS - 1)),
        now
    ));
    assert!(background_due(&stale, &["u"], Some(now - RETRY_SECS), now));
    assert!(
        background_due(&stale, &["u"], Some(now + 60), now),
        "relógio voltou: não trava"
    );
    let mut fresh = HttpState::default();
    fresh.set(
        "u",
        HttpEntry {
            checked_at: now - 10,
            ..Default::default()
        },
    );
    assert!(
        !background_due(&fresh, &["u"], None, now),
        "catálogo novo: nada a fazer"
    );
}

#[test]
fn claim_records_the_attempt_and_refuses_a_second_claim_inside_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let urls = ["http://127.0.0.1:9/x"];
    assert!(claim_background_refresh(&data, &urls, 10_000));
    let read = || std::fs::read_to_string(data.refresh_attempt_path()).unwrap();
    assert_eq!(read().trim(), "10000");
    assert!(
        !claim_background_refresh(&data, &urls, 10_000),
        "mesmo instante"
    );
    assert!(!claim_background_refresh(&data, &urls, 10_060));
    assert_eq!(read().trim(), "10000", "recusa não regrava");
    assert!(claim_background_refresh(&data, &urls, 10_000 + RETRY_SECS));
    assert_eq!(read().trim(), (10_000 + RETRY_SECS).to_string());
}

#[test]
fn unreadable_attempt_file_does_not_block_the_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    std::fs::write(data.refresh_attempt_path(), "lixo").unwrap();
    let urls = ["http://127.0.0.1:9/x"];
    assert!(claim_background_refresh(&data, &urls, 10_000));
    let got = std::fs::read_to_string(data.refresh_attempt_path()).unwrap();
    assert_eq!(got.trim(), "10000");
}

#[tokio::test]
async fn a_failed_source_does_not_respawn_on_every_command() {
    let (_s, ep) = setup().await;
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
    let urls: Vec<&str> = std::iter::once(ep.winget_index.as_str())
        .chain(ep.scoop.iter().map(|(_, u)| u.as_str()))
        .collect();
    let now = wingetrs::cache::now_unix();
    assert!(
        needs_refresh(&HttpState::load(&data.http_state_path()), &urls, now),
        "extras falhou: continua velho"
    );
    assert!(claim_background_refresh(&data, &urls, now));
    for later in [1, 30, RETRY_SECS - 1] {
        assert!(
            !claim_background_refresh(&data, &urls, now + later),
            "+{later}s"
        );
    }
}

fn all_urls(ep: &Endpoints) -> Vec<String> {
    let mut v = vec![ep.winget_index.clone()];
    v.extend(ep.scoop.iter().map(|(_, u)| u.clone()));
    v
}

async fn endpoints_all_answering(s: &MockServer, status: u16, expect0: bool) -> Endpoints {
    for p in ["/source2.msix", "/main.tgz", "/extras.tgz"] {
        let m = Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status));
        if expect0 {
            m.expect(0).mount(s).await;
        } else {
            m.mount(s).await;
        }
    }
    Endpoints {
        winget_index: format!("{}/source2.msix", s.uri()),
        scoop: vec![
            ("main".into(), format!("{}/main.tgz", s.uri())),
            ("extras".into(), format!("{}/extras.tgz", s.uri())),
        ],
    }
}

fn seed_state_and_index(data: &DataDir, ep: &Endpoints, checked_at: i64) {
    data.ensure().unwrap();
    let mut st = HttpState::default();
    for u in all_urls(ep) {
        st.set(
            &u,
            HttpEntry {
                etag: Some("\"e\"".into()),
                checked_at,
                ..HttpEntry::default()
            },
        );
    }
    // A bucket without the scripts marker is refetched even when fresh.
    for (b, _) in &ep.scoop {
        let dir = data.buckets_dir().join(b);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".scripts-ok"), b"").unwrap();
    }
    st.save(&data.http_state_path()).unwrap();
    wingetrs::catalog::index_file::write(
        &Catalog::open_in_memory().unwrap(),
        &data.search_index_path(),
    )
    .unwrap();
}

#[tokio::test]
async fn fresh_update_touches_neither_http_json_nor_the_catalog() {
    let s = MockServer::start().await;
    let ep = endpoints_all_answering(&s, 500, true).await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    seed_state_and_index(&data, &ep, wingetrs::cache::now_unix());
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    std::fs::File::options()
        .write(true)
        .open(data.http_state_path())
        .unwrap()
        .set_modified(old)
        .unwrap();
    let rep = run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        false,
        LockWait::from_env(),
    )
    .await
    .unwrap();
    assert_eq!(rep.len(), 3);
    assert!(rep.iter().all(|r| r.outcome == Outcome::Fresh), "{rep:?}");
    assert_eq!(
        std::fs::metadata(data.http_state_path())
            .unwrap()
            .modified()
            .unwrap(),
        old
    );
    assert!(!data.catalog_path().exists());
}

#[tokio::test]
async fn not_modified_update_still_refreshes_checked_at() {
    let s = MockServer::start().await;
    let ep = endpoints_all_answering(&s, 304, false).await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    seed_state_and_index(&data, &ep, 1);
    let rep = run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        false,
        LockWait::from_env(),
    )
    .await
    .unwrap();
    assert!(
        rep.iter().all(|r| r.outcome == Outcome::Unchanged),
        "{rep:?}"
    );
    let back = HttpState::load(&data.http_state_path());
    for u in all_urls(&ep) {
        assert!(back.get(&u).unwrap().checked_at > 1, "{u}");
    }
}

#[tokio::test]
async fn not_modified_update_rebuilds_a_missing_search_index() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    {
        let (_s, ep) = setup().await;
        run_update(
            data_ensured(&data),
            &ep,
            &Http::new().unwrap(),
            true,
            LockWait::from_env(),
        )
        .await
        .unwrap();
    }
    let s = MockServer::start().await;
    let ep = endpoints_all_answering(&s, 304, false).await;
    seed_state_and_index(&data, &ep, 1);
    std::fs::remove_file(data.search_index_path()).unwrap();
    let rep = run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        false,
        LockWait::from_env(),
    )
    .await
    .unwrap();
    assert!(
        rep.iter().all(|r| r.outcome == Outcome::Unchanged),
        "{rep:?}"
    );
    assert!(data.catalog_path().exists());
    let bytes = std::fs::read(data.search_index_path()).unwrap();
    assert_eq!(
        wingetrs::catalog::index_file::IndexView::parse(&bytes)
            .unwrap()
            .len(),
        28
    );
}

fn data_ensured(data: &DataDir) -> &DataDir {
    data.ensure().unwrap();
    data
}

fn blob_with_age(data: &DataDir, content: &[u8], days: u64) -> std::path::PathBuf {
    let name = wingetrs::cache::sha256_hex(content);
    let p = data.blobs_dir().join(name);
    std::fs::create_dir_all(data.blobs_dir()).unwrap();
    std::fs::write(&p, content).unwrap();
    let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86400))
        .unwrap();
    p
}

async fn outcomes_of_prune_run(data: &DataDir, ep: &Endpoints) -> Vec<(String, Outcome)> {
    let rep = run_update(data, ep, &Http::new().unwrap(), true, LockWait::from_env())
        .await
        .unwrap();
    let mut v: Vec<_> = rep.into_iter().map(|r| (r.source, r.outcome)).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v.into_iter()
        .map(|(s, o)| match o {
            Outcome::Failed { offline, .. } => (
                s,
                Outcome::Failed {
                    message: String::new(),
                    offline,
                },
            ),
            o => (s, o),
        })
        .collect()
}

fn expected_prune_outcomes() -> Vec<(String, Outcome)> {
    vec![
        (
            "scoop:extras".to_owned(),
            Outcome::Failed {
                message: String::new(),
                offline: false,
            },
        ),
        (
            "scoop:main".to_owned(),
            Outcome::Updated {
                packages: 8,
                skipped: 1,
            },
        ),
        (
            "winget".to_owned(),
            Outcome::Updated {
                packages: 20,
                skipped: 0,
            },
        ),
    ]
}

#[tokio::test]
async fn update_prunes_old_blobs() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let old = blob_with_age(&data, b"old", 40);
    let young = blob_with_age(&data, b"young", 1);
    let out = outcomes_of_prune_run(&data, &ep).await;
    assert_eq!(out, expected_prune_outcomes());
    assert!(!old.exists());
    assert!(young.exists());
}

#[tokio::test]
async fn update_skips_prune_while_install_lock_is_held() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let old = blob_with_age(&data, b"old", 40);
    let young = blob_with_age(&data, b"young", 1);
    let held = wingetrs::cache::UpdateLock::try_acquire(&data.install_layout().install_lock())
        .unwrap()
        .unwrap();
    let out = outcomes_of_prune_run(&data, &ep).await;
    drop(held);
    assert_eq!(out, expected_prune_outcomes());
    assert!(old.exists());
    assert!(young.exists());
}

#[tokio::test]
async fn prune_failure_never_fails_update() {
    let (_s, ep) = setup().await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    std::fs::remove_dir(data.blobs_dir()).unwrap();
    // `blobs` is a file, not a directory.
    std::fs::write(data.blobs_dir(), b"not a dir").unwrap();
    let out = outcomes_of_prune_run(&data, &ep).await;
    assert_eq!(out, expected_prune_outcomes());
}
