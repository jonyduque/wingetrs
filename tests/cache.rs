#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::time::{Duration, Instant};
use wingetrs::cache::{
    BlobStore, CacheError, DataDir, HttpEntry, HttpState, UpdateLock, sha256_hex,
};

fn entry(tag: &str) -> HttpEntry {
    HttpEntry {
        etag: Some(tag.into()),
        last_modified: Some("Wed, 07 Oct 2026 03:02:43 GMT".into()),
        source_version: Some("1.0".into()),
        checked_at: 1_800_000_000,
    }
}

#[test]
fn data_dir_paths_and_ensure() {
    let t = tempfile::tempdir().unwrap();
    let d = DataDir::new(t.path().join("w"));
    d.ensure().unwrap();
    assert!(d.blobs_dir().is_dir() && d.log_dir().is_dir() && d.tmp_dir().is_dir());
    assert_eq!(d.catalog_path(), d.root().join("catalog.db"));
    assert_eq!(d.http_state_path(), d.root().join("http.json"));
    assert_eq!(d.lock_path(), d.root().join("update.lock"));
    assert_eq!(d.config_path(), d.root().join("config.toml"));
}

#[test]
fn http_state_roundtrip() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("http.json");
    let mut s = HttpState::default();
    s.set("http://a/x", entry("\"1\""));
    s.set("http://b/y", entry("\"2\""));
    s.save(&p).unwrap();
    let back = HttpState::load(&p);
    assert_eq!(back, s);
    assert_eq!(back.get("http://a/x"), Some(&entry("\"1\"")));
    assert_eq!(back.get("nope"), None);
}

#[test]
fn http_state_missing_or_corrupt_is_empty() {
    let t = tempfile::tempdir().unwrap();
    assert_eq!(
        HttpState::load(&t.path().join("none.json")),
        HttpState::default()
    );
    let p = t.path().join("http.json");
    std::fs::write(&p, "{lixo").unwrap();
    assert_eq!(HttpState::load(&p), HttpState::default());
}

#[test]
fn save_leaves_no_tmp_file() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("http.json");
    let mut s = HttpState::default();
    s.set("u", entry("\"1\""));
    s.save(&p).unwrap();
    s.save(&p).unwrap();
    assert!(p.exists());
    assert!(!t.path().join("http.json.tmp").exists());
}

#[test]
fn blob_put_verified_rejects_wrong_hash() {
    let t = tempfile::tempdir().unwrap();
    let b = BlobStore::new(t.path().join("blobs"));
    let wrong = sha256_hex(b"other");
    let e = b.put_verified(&wrong, b"data").unwrap_err();
    assert!(matches!(e, CacheError::HashMismatch { .. }), "{e:?}");
    assert!(b.get(&wrong).is_none());
    let n = std::fs::read_dir(t.path().join("blobs"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(n, 0);
}

#[test]
fn blob_roundtrip_and_tamper_detection() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("blobs");
    let b = BlobStore::new(dir.clone());
    let h = sha256_hex(b"data");
    assert_eq!(h.len(), 64);
    b.put_verified(&h, b"data").unwrap();
    assert!(dir.join(&h).is_file());
    assert_eq!(b.get(&h).as_deref(), Some(&b"data"[..]));
    std::fs::write(dir.join(&h), b"tampered").unwrap();
    assert!(b.get(&h).is_none());
}

#[test]
fn lock_is_exclusive_until_drop() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("update.lock");
    let first = UpdateLock::try_acquire(&p).unwrap();
    assert!(first.is_some());
    assert!(UpdateLock::try_acquire(&p).unwrap().is_none());
    drop(first);
    assert!(UpdateLock::try_acquire(&p).unwrap().is_some());
}

#[test]
fn acquire_timeout_waits_then_gives_up() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("update.lock");
    let _held = UpdateLock::try_acquire(&p).unwrap().unwrap();
    let start = Instant::now();
    let r = UpdateLock::acquire_timeout(&p, Duration::from_millis(300)).unwrap();
    let el = start.elapsed();
    assert!(r.is_none());
    assert!(
        el >= Duration::from_millis(300) && el < Duration::from_secs(2),
        "{el:?}"
    );
}

#[test]
fn from_local_app_data_prefers_non_empty_env_value() {
    let dir = wingetrs::cache::DataDir::from_local_app_data(Some("C:/x".into())).unwrap();
    assert_eq!(dir.root(), std::path::Path::new("C:/x").join("wingetrs"));
}
