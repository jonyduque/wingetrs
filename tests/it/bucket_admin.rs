#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Task 206: bucket add/rm with lock, verification and immediate download.
use flate2::{Compression, write::GzEncoder};
use std::io::Write;
use wingetrs::buckets::{BucketError, add, add_downloaded, remove_and_cleanup_confirmed, rows};
use wingetrs::cache::{DataDir, HttpEntry, HttpState, UpdateLock};
use wingetrs::catalog::{Catalog, WingetMaps, index_file};
use wingetrs::config::Config;
use wingetrs::core::SourceKind;
use wingetrs::http::Http;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Tarball with the given `(path, content)` regular files.
fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    for (name, data) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, name, *data).unwrap();
    }
    let raw = b.into_inner().unwrap();
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

fn bucket_tgz() -> Vec<u8> {
    tarball(&[
        (
            "mine-master/bucket/zebra.json",
            br#"{"version":"1.0","description":"d","homepage":"https://x.io"}"#,
        ),
        (
            "mine-master/bucket/okapi.json",
            br#"{"version":"2.0","description":"d","homepage":"https://x.io"}"#,
        ),
        ("mine-master/scripts/zebra/a.reg", b"REG"),
    ])
}

async fn serve(body: Vec<u8>, status: u16) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/mine.tgz"))
        .respond_with(ResponseTemplate::new(status).set_body_bytes(body))
        .mount(&s)
        .await;
    s
}

fn setup() -> (tempfile::TempDir, DataDir, Config) {
    let d = tempfile::tempdir().unwrap();
    let data = DataDir::new(d.path());
    let mut cfg = Config::default();
    cfg.save(&data.config_path()).unwrap();
    add(&mut cfg, "mine", "me/scoop-mine", true).unwrap();
    (d, data, cfg)
}

/// A catalog and search index that exist before the `add`, so "nothing written" is observable.
fn seed(data: &DataDir) {
    let mut c = Catalog::open(&data.catalog_path()).unwrap();
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "v",
        1,
        &[pkg("seeded")],
        &WingetMaps::default(),
    )
    .unwrap();
    index_file::write(&c, &data.search_index_path()).unwrap();
}

fn pkg(id: &str) -> wingetrs::catalog::NewPackage {
    wingetrs::catalog::NewPackage {
        id: id.into(),
        name: id.into(),
        moniker: None,
        version: "1".into(),
        publisher: None,
        description: None,
        tags: vec![],
        commands: vec![],
        arp_min_version: None,
        arp_max_version: None,
        winget_hash: None,
        raw: None,
    }
}

fn snapshot(data: &DataDir) -> [Vec<u8>; 4] {
    [
        std::fs::read(data.config_path()).unwrap(),
        std::fs::read(data.catalog_path()).unwrap(),
        std::fs::read(data.search_index_path()).unwrap(),
        std::fs::read(data.http_state_path()).unwrap_or_default(),
    ]
}

#[tokio::test]
async fn add_downloads_and_indexes_now() {
    let (_d, data, cfg) = setup();
    let s = serve(bucket_tgz(), 200).await;
    let url = format!("{}/mine.tgz", s.uri());
    let n = add_downloaded(&data, &Http::new().unwrap(), &cfg, "mine", Some(&url))
        .await
        .unwrap();
    assert_eq!(n, 2);
    let c = Catalog::open(&data.catalog_path()).unwrap();
    let srcs: Vec<String> = c.sources().unwrap().into_iter().map(|s| s.source).collect();
    assert_eq!(srcs, vec!["scoop:mine".to_string()]);
    let found = index_file::search(&data.search_index_path(), "zebra", None, 10)
        .unwrap()
        .unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        data.buckets_dir()
            .join("mine")
            .join("scripts")
            .join("zebra")
            .join("a.reg")
            .exists()
    );
    assert!(
        HttpState::load(&data.http_state_path())
            .get(&url)
            .is_some_and(|e| e.checked_at > 0)
    );
    let saved = Config::load(&data.config_path()).unwrap();
    assert_eq!(saved.buckets.len(), 1, "{saved:?}");
}

#[tokio::test]
async fn add_404_writes_nothing() {
    let (_d, data, cfg) = setup();
    seed(&data);
    let before = snapshot(&data);
    let s = serve(Vec::new(), 404).await;
    let url = format!("{}/mine.tgz", s.uri());
    let e = add_downloaded(&data, &Http::new().unwrap(), &cfg, "mine", Some(&url))
        .await
        .unwrap_err();
    assert_eq!(e, BucketError::RepoNotFound("me/scoop-mine".into()));
    assert_eq!(snapshot(&data), before);
    assert!(!data.buckets_dir().join("mine").exists());
}

#[tokio::test]
async fn add_tarball_without_manifests_refused() {
    let (_d, data, cfg) = setup();
    seed(&data);
    let before = snapshot(&data);
    let tgz = tarball(&[("mine-master/README.md", b"hello")]);
    let s = serve(tgz, 200).await;
    let url = format!("{}/mine.tgz", s.uri());
    let e = add_downloaded(&data, &Http::new().unwrap(), &cfg, "mine", Some(&url))
        .await
        .unwrap_err();
    assert_eq!(e, BucketError::NotABucket);
    assert_eq!(snapshot(&data), before);
    assert!(!data.buckets_dir().join("mine").exists());
}

#[tokio::test]
async fn add_and_rm_refuse_while_update_lock_held() {
    let (d, data, mut cfg) = setup();
    let s = serve(bucket_tgz(), 200).await;
    let url = format!("{}/mine.tgz", s.uri());
    let _held = UpdateLock::try_acquire(&data.lock_path()).unwrap().unwrap();
    let e = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        add_downloaded(&data, &Http::new().unwrap(), &cfg, "mine", Some(&url)),
    )
    .await
    .expect("add must refuse at once, not wait")
    .unwrap_err();
    assert_eq!(e, BucketError::Busy);
    let db = d.path().join("state.db");
    let e = remove_and_cleanup_confirmed(&data, &mut cfg, "mine", &db, true).unwrap_err();
    assert_eq!(e, BucketError::Busy);
    assert!(!data.catalog_path().exists());
    assert_eq!(cfg.buckets.len(), 1, "rm must not touch the config");
}

#[test]
fn list_shows_app_count_and_date() {
    let (_d, data, cfg) = setup();
    {
        let mut c = Catalog::open(&data.catalog_path()).unwrap();
        c.replace_source(
            "scoop:mine",
            SourceKind::Scoop,
            "v",
            1_700_000_000,
            &[pkg("a"), pkg("b"), pkg("c")],
            &WingetMaps::default(),
        )
        .unwrap();
    }
    let list = rows(&data, &cfg).unwrap();
    let mine = list.iter().find(|r| r.name == "mine").unwrap();
    assert_eq!(mine.apps, Some(3));
    assert_eq!(mine.updated, Some(1_700_000_000));
    let text = wingetrs::cli::output::buckets_text(&list);
    assert!(text.contains("2023-11-14"), "{text}");
    assert!(text.contains("me/scoop-mine"), "{text}");
    assert!(text.contains("não baixado"), "{text}");
}

#[test]
fn rm_default_bucket_needs_confirmation() {
    let (d, data, mut cfg) = setup();
    add(&mut cfg, "extras", "ScoopInstaller/Extras", false).unwrap();
    let db = d.path().join("state.db");
    let e = remove_and_cleanup_confirmed(&data, &mut cfg, "extras", &db, false).unwrap_err();
    assert_eq!(e, BucketError::DefaultNeedsConfirmation("extras".into()));
    assert!(cfg.scoop_buckets.iter().any(|n| n == "extras"));
    remove_and_cleanup_confirmed(&data, &mut cfg, "extras", &db, true).unwrap();
    assert!(!cfg.scoop_buckets.iter().any(|n| n == "extras"));
}

#[test]
fn rm_drops_legacy_master_url() {
    let (d, data, mut cfg) = setup();
    let current = "https://codeload.github.com/me/scoop-mine/tar.gz/HEAD";
    let legacy = "https://codeload.github.com/me/scoop-mine/tar.gz/refs/heads/master";
    let other = "https://codeload.github.com/ScoopInstaller/Main/tar.gz/refs/heads/master";
    let mut st = HttpState::default();
    for u in [current, legacy, other] {
        st.set(
            u,
            HttpEntry {
                checked_at: 5,
                ..Default::default()
            },
        );
    }
    st.save(&data.http_state_path()).unwrap();
    let db = d.path().join("state.db");
    remove_and_cleanup_confirmed(&data, &mut cfg, "mine", &db, false).unwrap();
    let st = HttpState::load(&data.http_state_path());
    assert!(st.get(current).is_none());
    assert!(st.get(legacy).is_none());
    assert!(st.get(other).is_some());
}
