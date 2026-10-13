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
        // Lower bound only: an upper bound would measure the machine, not the rule.
        el >= Duration::from_millis(300),
        "{el:?}"
    );
}

#[test]
fn from_local_app_data_prefers_non_empty_env_value() {
    let dir = wingetrs::cache::DataDir::from_local_app_data(Some("C:/x".into())).unwrap();
    assert_eq!(dir.root(), std::path::Path::new("C:/x").join("wingetrs"));
}

fn age(path: &std::path::Path, days: i64) {
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    let secs = days.unsigned_abs() * 24 * 3600;
    let when = if days >= 0 {
        std::time::SystemTime::now() - Duration::from_secs(secs)
    } else {
        std::time::SystemTime::now() + Duration::from_secs(secs)
    };
    f.set_modified(when).unwrap();
}

fn age_secs(path: &std::path::Path, secs: u64) {
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(std::time::SystemTime::now() - Duration::from_secs(secs))
        .unwrap();
}

fn mtime(path: &std::path::Path) -> std::time::SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

fn put_blob(dir: &std::path::Path, content: &[u8]) -> String {
    let name = sha256_hex(content);
    BlobStore::new(dir.to_path_buf())
        .put_verified(&name, content)
        .unwrap();
    name
}

#[test]
fn get_hit_refreshes_mtime() {
    let t = tempfile::tempdir().unwrap();
    let store = BlobStore::new(t.path().to_path_buf());
    let name = put_blob(t.path(), b"hello");
    age(&t.path().join(&name), 40);
    let before = std::time::SystemTime::now();
    assert_eq!(store.get(&name).as_deref(), Some(&b"hello"[..]));
    assert!(mtime(&t.path().join(&name)) >= before);

    // Tampered content: a miss, and the mtime is left alone.
    std::fs::write(t.path().join(&name), b"other").unwrap();
    age(&t.path().join(&name), 40);
    let old = mtime(&t.path().join(&name));
    assert_eq!(store.get(&name), None);
    assert_eq!(mtime(&t.path().join(&name)), old);
}

#[test]
fn prune_removes_only_old_blobs_and_old_temps() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path();
    let old = put_blob(d, b"old blob");
    let young = put_blob(d, b"young blob");
    let h3 = sha256_hex(b"tmp old");
    let h4 = sha256_hex(b"tmp young");
    std::fs::write(d.join(format!("{h3}.tmp")), b"tmp old").unwrap();
    std::fs::write(d.join(format!("{h4}.tmp")), b"tmp young").unwrap();
    std::fs::write(d.join("index.json"), b"{}").unwrap();
    std::fs::write(d.join("notes.txt"), b"x").unwrap();
    let short = &sha256_hex(b"short")[..63];
    std::fs::write(d.join(short), b"x").unwrap();
    std::fs::create_dir(d.join("subdir")).unwrap();

    age(&d.join(&old), 40);
    age(&d.join(&young), 10);
    age(&d.join(format!("{h3}.tmp")), 2);
    age_secs(&d.join(format!("{h4}.tmp")), 3600);
    for n in ["index.json", "notes.txt", short] {
        age(&d.join(n), 400);
    }

    let expected_bytes = (b"old blob".len() + b"tmp old".len()) as u64;
    let stats = BlobStore::new(d.to_path_buf())
        .prune(
            std::time::SystemTime::now(),
            Duration::from_secs(30 * 86400),
        )
        .unwrap();
    assert_eq!(stats.removed, 2);
    assert_eq!(stats.bytes, expected_bytes);
    assert_eq!(stats.kept, 6);
    let mut names: Vec<String> = std::fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    let mut want = vec![
        young,
        format!("{h4}.tmp"),
        "index.json".to_owned(),
        "notes.txt".to_owned(),
        short.to_owned(),
        "subdir".to_owned(),
    ];
    want.sort();
    assert_eq!(names, want);
}

#[test]
fn prune_keeps_future_mtime_and_missing_dir() {
    let t = tempfile::tempdir().unwrap();
    let name = put_blob(t.path(), b"future");
    age(&t.path().join(&name), -1);
    let max = Duration::from_secs(30 * 86400);
    let stats = BlobStore::new(t.path().to_path_buf())
        .prune(std::time::SystemTime::now(), max)
        .unwrap();
    assert_eq!(stats.removed, 0);
    assert!(t.path().join(&name).exists());

    let missing = t.path().join("nope");
    let stats = BlobStore::new(missing.clone())
        .prune(std::time::SystemTime::now(), max)
        .unwrap();
    assert_eq!(stats, wingetrs::cache::PruneStats::default());
    assert!(!missing.exists());
}
