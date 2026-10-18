#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 149).
use flate2::{Compression, write::GzEncoder};
use std::io::Write;
use wingetrs::cache::{DataDir, HttpEntry, HttpState};
use wingetrs::config::Config;
use wingetrs::http::Http;
use wingetrs::sources::scoop::bucket::{read_tarball, write_scripts};
use wingetrs::update::{Endpoints, LockWait, run_update};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TGZ: &[u8] = include_bytes!("../fixtures/scoop/Main-mini.tar.gz");

/// (raw path, entry type, content, link name)
type Item<'a> = (&'a str, tar::EntryType, &'a [u8], Option<&'a str>);

/// Builds a tarball writing the raw name into the header (the builder API refuses `..`).
fn tarball(items: &[Item<'_>]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    for (name, kind, data, link) in items {
        let mut h = tar::Header::new_gnu();
        let raw = name.as_bytes();
        h.as_old_mut().name[..raw.len()].copy_from_slice(raw);
        h.set_entry_type(*kind);
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        if let Some(l) = link {
            h.set_link_name(l).unwrap();
        }
        h.set_cksum();
        b.append(&h, *data).unwrap();
    }
    let raw = b.into_inner().unwrap();
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

#[test]
fn scripts_collected_with_relative_paths() {
    let tgz = tarball(&[
        (
            "Main-master/scripts/7-zip/a.reg",
            tar::EntryType::Regular,
            b"REG",
            None,
        ),
        (
            "Main-master/bucket/x.json",
            tar::EntryType::Regular,
            br#"{"version":"1.0","description":"d","homepage":"https://x.io"}"#,
            None,
        ),
    ]);
    let imp = read_tarball(&tgz).unwrap();
    assert_eq!(
        imp.scripts,
        vec![("7-zip/a.reg".to_string(), b"REG".to_vec())]
    );
    assert_eq!(imp.packages.len(), 1);
}

#[test]
fn unsafe_paths_skipped_rest_kept() {
    let reg = tar::EntryType::Regular;
    let tgz = tarball(&[
        ("Main-master/scripts/../x", reg, b"1", None),
        ("Main-master/scripts/C:/x", reg, b"2", None),
        ("Main-master/scripts/aux.reg", reg, b"3", None),
        ("Main-master/scripts/ok.reg", reg, b"4", None),
    ]);
    let imp = read_tarball(&tgz).unwrap();
    assert_eq!(imp.scripts, vec![("ok.reg".to_string(), b"4".to_vec())]);
    for s in ["scripts/../x", "scripts/C:/x", "scripts/aux.reg"] {
        assert!(
            imp.skipped.contains(&s.to_string()),
            "{s}: {:?}",
            imp.skipped
        );
    }
}

#[test]
fn symlink_entry_skipped() {
    let tgz = tarball(&[
        (
            "Main-master/scripts/link.reg",
            tar::EntryType::Symlink,
            b"",
            Some("target"),
        ),
        (
            "Main-master/scripts/hard.reg",
            tar::EntryType::Link,
            b"",
            Some("target"),
        ),
        (
            "Main-master/scripts/ok.reg",
            tar::EntryType::Regular,
            b"4",
            None,
        ),
    ]);
    let imp = read_tarball(&tgz).unwrap();
    assert_eq!(imp.scripts, vec![("ok.reg".to_string(), b"4".to_vec())]);
    assert!(imp.skipped.contains(&"scripts/link.reg".to_string()));
    assert!(imp.skipped.contains(&"scripts/hard.reg".to_string()));
}

#[test]
fn file_limit() {
    let names: Vec<String> = (0..2001)
        .map(|i| format!("Main-master/scripts/f{i}.reg"))
        .collect();
    let items: Vec<Item<'_>> = names
        .iter()
        .map(|n| (n.as_str(), tar::EntryType::Regular, &b"x"[..], None))
        .collect();
    let imp = read_tarball(&tarball(&items)).unwrap();
    assert_eq!(imp.scripts.len(), 2000);
    assert_eq!(
        imp.skipped
            .iter()
            .filter(|s| *s == "scripts: limite excedido")
            .count(),
        1
    );
}

#[test]
fn byte_limit() {
    let big = vec![b'x'; 5 * 1024 * 1024];
    let imp = read_tarball(&tarball(&[
        (
            "Main-master/scripts/a.bin",
            tar::EntryType::Regular,
            &big,
            None,
        ),
        (
            "Main-master/scripts/b.bin",
            tar::EntryType::Regular,
            &big,
            None,
        ),
        (
            "Main-master/scripts/c.reg",
            tar::EntryType::Regular,
            b"1",
            None,
        ),
    ]))
    .unwrap();
    assert_eq!(imp.scripts.len(), 1);
    assert_eq!(imp.scripts[0].0, "a.bin");
    assert_eq!(imp.skipped, vec!["scripts: limite excedido".to_string()]);
}

fn new_files() -> Vec<(String, Vec<u8>)> {
    vec![
        ("n.reg".to_string(), b"N".to_vec()),
        ("sub/m.reg".to_string(), b"M".to_vec()),
    ]
}

fn leftovers(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".scripts-new") || n.starts_with(".scripts-old"))
        .collect()
}

#[test]
fn swap_replaces_old() {
    let d = tempfile::tempdir().unwrap();
    let scripts = d.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("old.reg"), b"O").unwrap();
    write_scripts(d.path(), &new_files(), "T").unwrap();
    assert!(!scripts.join("old.reg").exists());
    assert_eq!(std::fs::read(scripts.join("n.reg")).unwrap(), b"N");
    assert_eq!(
        std::fs::read(scripts.join("sub").join("m.reg")).unwrap(),
        b"M"
    );
    assert!(leftovers(d.path()).is_empty());
    assert!(d.path().join(".scripts-ok").exists());
}

#[test]
fn empty_set_still_writes_marker() {
    let d = tempfile::tempdir().unwrap();
    let bucket = d.path().join("fresh");
    write_scripts(&bucket, &[], "T").unwrap();
    assert!(bucket.join(".scripts-ok").exists());
    assert!(bucket.join("scripts").is_dir());
}

#[test]
fn failed_write_keeps_old() {
    let d = tempfile::tempdir().unwrap();
    let scripts = d.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("old.reg"), b"O").unwrap();
    std::fs::write(d.path().join(".scripts-new-T"), b"in the way").unwrap();
    assert!(write_scripts(d.path(), &new_files(), "T").is_err());
    assert_eq!(std::fs::read(scripts.join("old.reg")).unwrap(), b"O");
    assert!(!d.path().join(".scripts-ok").exists());
}

#[test]
fn unsafe_name_in_write_fails_and_keeps_old() {
    let d = tempfile::tempdir().unwrap();
    let scripts = d.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("old.reg"), b"O").unwrap();
    let bad = vec![("../evil".to_string(), b"E".to_vec())];
    assert!(write_scripts(d.path(), &bad, "T").is_err());
    assert_eq!(std::fs::read(scripts.join("old.reg")).unwrap(), b"O");
    assert!(leftovers(d.path()).is_empty());
}

async fn requests_for_main(with_marker: bool) -> Vec<wiremock::Request> {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/main.tgz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(TGZ.to_vec()))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/w.msix"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let url = format!("{}/main.tgz", s.uri());
    let mut st = HttpState::default();
    st.set(
        &url,
        HttpEntry {
            etag: Some("\"s1\"".into()),
            checked_at: 1,
            ..Default::default()
        },
    );
    st.save(&data.http_state_path()).unwrap();
    if with_marker {
        let b = data.buckets_dir().join("main");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join(".scripts-ok"), b"").unwrap();
    }
    let ep = Endpoints {
        winget_index: format!("{}/w.msix", s.uri()),
        scoop: vec![("main".into(), url)],
    };
    run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        true,
        LockWait::from_env(),
    )
    .await
    .unwrap();
    s.received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/main.tgz")
        .collect()
}

#[tokio::test]
async fn missing_marker_forces_unconditional_get() {
    let reqs = requests_for_main(false).await;
    assert_eq!(reqs.len(), 1);
    assert!(!reqs[0].headers.contains_key("if-none-match"));
}

#[tokio::test]
async fn present_marker_keeps_conditional_get() {
    let reqs = requests_for_main(true).await;
    assert_eq!(reqs.len(), 1);
    assert!(reqs[0].headers.contains_key("if-none-match"));
}

#[test]
fn bucket_remove_deletes_scripts_dir() {
    let d = tempfile::tempdir().unwrap();
    let data = DataDir::new(d.path());
    let mut cfg = Config::default();
    wingetrs::buckets::add(&mut cfg, "x", "me/scoop-x", true).unwrap();
    write_scripts(&data.buckets_dir().join("x"), &new_files(), "T").unwrap();
    write_scripts(&data.buckets_dir().join("main"), &new_files(), "T").unwrap();
    let db = d.path().join("state.db");
    wingetrs::buckets::remove_and_cleanup(&data, &mut cfg, "x", &db).unwrap();
    assert!(!data.buckets_dir().join("x").exists());
    assert!(data.buckets_dir().join("main").join("scripts").is_dir());
}

#[test]
fn marker_is_removed_before_the_swap_starts() {
    let d = tempfile::tempdir().unwrap();
    let scripts = d.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("old.reg"), b"O").unwrap();
    std::fs::write(d.path().join(".scripts-ok"), b"").unwrap();
    std::fs::write(d.path().join(".scripts-new-T"), b"in the way").unwrap();
    assert!(write_scripts(d.path(), &new_files(), "T").is_err());
    assert!(!d.path().join(".scripts-ok").exists());
    assert_eq!(std::fs::read(scripts.join("old.reg")).unwrap(), b"O");
}

#[test]
fn stale_staging_of_other_tags_is_removed() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".scripts-old-111")).unwrap();
    std::fs::create_dir_all(d.path().join(".scripts-new-222").join("x")).unwrap();
    write_scripts(d.path(), &new_files(), "T").unwrap();
    assert!(leftovers(d.path()).is_empty());
}

#[test]
fn duplicates_and_file_dir_conflicts_are_skipped_rest_kept() {
    let reg = tar::EntryType::Regular;
    let tgz = tarball(&[
        ("Main-master/scripts/A.reg", reg, b"1", None),
        ("Main-master/scripts/a.reg", reg, b"2", None),
        ("Main-master/scripts/A.reg", reg, b"3", None),
        ("Main-master/scripts/d", reg, b"4", None),
        ("Main-master/scripts/d/f", reg, b"5", None),
        ("Main-master/scripts/e/f", reg, b"6", None),
        ("Main-master/scripts/e", reg, b"7", None),
        ("Main-master/scripts/ok.reg", reg, b"8", None),
    ]);
    let imp = read_tarball(&tgz).unwrap();
    let got: Vec<&str> = imp.scripts.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(got, vec!["A.reg", "d", "e/f", "ok.reg"]);
    assert_eq!(imp.skipped.len(), 4, "{:?}", imp.skipped);
}

#[test]
fn skipped_notes_are_capped() {
    let names: Vec<String> = (0..2005)
        .map(|i| format!("Main-master/scripts/aux{i}/../x"))
        .collect();
    let items: Vec<Item<'_>> = names
        .iter()
        .map(|n| (n.as_str(), tar::EntryType::Regular, &b"x"[..], None))
        .collect();
    let imp = read_tarball(&tarball(&items)).unwrap();
    assert_eq!(imp.skipped.len(), 2001);
    assert_eq!(imp.skipped.last().unwrap(), "scripts: mais 5 ignorados");
}

#[tokio::test]
async fn missing_marker_downloads_even_when_the_bucket_is_fresh() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/main.tgz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(TGZ.to_vec()))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let url = format!("{}/main.tgz", s.uri());
    let mut st = HttpState::default();
    st.set(
        &url,
        HttpEntry {
            etag: Some("\"s1\"".into()),
            checked_at: wingetrs::cache::now_unix(),
            ..Default::default()
        },
    );
    st.save(&data.http_state_path()).unwrap();
    let ep = Endpoints {
        winget_index: format!("{}/w.msix", s.uri()),
        scoop: vec![("main".into(), url)],
    };
    run_update(
        &data,
        &ep,
        &Http::new().unwrap(),
        false,
        LockWait::from_env(),
    )
    .await
    .unwrap();
    let n = s
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/main.tgz")
        .count();
    assert_eq!(n, 1);
    assert!(data.buckets_dir().join("main").join(".scripts-ok").exists());
}
