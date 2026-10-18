//! Installer download and verified file (Task 177).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use wingetrs::cache::{DataDir, sha256_hex};
use wingetrs::http::Http;
use wingetrs::install::download::DownloadError;
use wingetrs::install::progress::ProgressEvent;
use wingetrs::sources::winget::manifest::{self, EffectiveInstaller, InstallerKind};
use wingetrs::winget_install::fetch::{FetchError, extension, fetch_installer};
#[cfg(windows)]
use wingetrs::winget_install::verified_win::VerifiedFile;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn noop(_: &ProgressEvent) {}

fn installer(url: &str, sha: &str, kind: &str) -> EffectiveInstaller {
    let yaml = format!(
        "PackageIdentifier: Pub.App\nPackageVersion: 1.0\nInstallers:\n- Architecture: x64\n  InstallerType: {kind}\n  InstallerUrl: {url}\n  InstallerSha256: {sha}\n"
    );
    manifest::parse(yaml.as_bytes())
        .unwrap()
        .effective()
        .remove(0)
}

async fn serve(body: Vec<u8>, expect: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/setup.exe"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .expect(expect)
        .mount(&server)
        .await;
    server
}

fn body(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

fn part_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".part"))
        .collect()
}

#[tokio::test]
async fn downloads_verifies_and_materializes() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let bytes = body(8 * 1024 * 1024);
    let sha = sha256_hex(&bytes);
    let server = serve(bytes.clone(), 1).await;
    let inst = installer(&format!("{}/setup.exe", server.uri()), &sha, "exe");
    let m = fetch_installer(&Http::new().unwrap(), &data, "Pub.App", "1.0", &inst, &noop)
        .await
        .unwrap();
    assert!(
        m.path()
            .ends_with(r"installers\Pub.App\1.0\Pub.App-1.0.exe"),
        "{}",
        m.path().display()
    );
    assert!(m.path().starts_with(data.root()));
    assert_eq!(m.sha256(), sha);
    assert_eq!(std::fs::read(m.path()).unwrap(), bytes);
    #[cfg(windows)]
    {
        let v = VerifiedFile::open(m.path(), &sha).unwrap();
        assert_eq!(v.path(), m.path());
    }
}

#[tokio::test]
async fn last_byte_wrong_leaves_nothing() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let mut bytes = body(256 * 1024);
    let sha = sha256_hex(&bytes);
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    let server = serve(bytes, 1).await;
    let inst = installer(&format!("{}/setup.exe", server.uri()), &sha, "exe");
    let r = fetch_installer(&Http::new().unwrap(), &data, "Pub.App", "1.0", &inst, &noop).await;
    assert!(
        matches!(
            r,
            Err(FetchError::Download(DownloadError::HashMismatch { .. }))
        ),
        "{:?}",
        r.err()
    );
    let root = data.root();
    assert!(!root.join("installers").join("Pub.App").exists());
    assert!(!root.join("blobs").join(&sha).exists());
    assert!(root.join("tmp").is_dir(), "download never started");
    assert_eq!(part_files(&root.join("tmp")), Vec::<String>::new());
}

#[tokio::test]
async fn http_url_refused_without_request() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let bytes = body(1024);
    let sha = sha256_hex(&bytes);
    let server = serve(bytes, 0).await;
    let port = server.address().port();
    let http = Http::new().unwrap();
    // Loopback-looking but not exactly 127.0.0.1 goes first: a mutant that accepts any http
    // fails here with a local connection error, before any name would be resolved.
    for url in [
        format!("http://127.0.0.2:{port}/setup.exe"),
        format!("http://user@127.0.0.1:{port}/setup.exe"),
        format!("http://user:pw@localhost:{port}/setup.exe"),
        "http://example.com/x.exe".to_string(),
        "http://127.0.0.1.evil.com/".to_string(),
        "http://localhost@evil.com/".to_string(),
        format!("ftp://127.0.0.1:{port}/setup.exe"),
        "not a url".to_string(),
    ] {
        let inst = installer(&url, &sha, "exe");
        let r = fetch_installer(&http, &data, "Pub.App", "1.0", &inst, &noop).await;
        assert!(
            matches!(r, Err(FetchError::NotHttps(_))),
            "{url}: {:?}",
            r.err()
        );
    }
    server.verify().await;
}

#[tokio::test]
async fn plain_http_to_loopback_is_accepted() {
    // Port 1 has no listener: the scheme check passes and the request fails locally.
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let sha = sha256_hex(b"x");
    let http = Http::new().unwrap();
    for url in [
        "http://127.0.0.1:1/setup.exe",
        "http://[::1]:1/setup.exe",
        "http://LOCALHOST:1/setup.exe",
    ] {
        let inst = installer(url, &sha, "exe");
        let r = fetch_installer(&http, &data, "Pub.App", "1.0", &inst, &noop).await;
        assert!(
            matches!(r, Err(FetchError::Download(_))),
            "{url}: {:?}",
            r.err()
        );
    }
}

#[tokio::test]
async fn https_prefix_is_case_insensitive() {
    // An https URL in any case passes the scheme check (the request itself fails: no server).
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let sha = sha256_hex(b"x");
    let inst = installer("HTTPS://127.0.0.1:1/setup.exe", &sha, "exe");
    let r = fetch_installer(&Http::new().unwrap(), &data, "Pub.App", "1.0", &inst, &noop).await;
    assert!(matches!(r, Err(FetchError::Download(_))), "{:?}", r.err());
}

#[tokio::test]
async fn cached_blob_no_second_request() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let bytes = body(64 * 1024);
    let sha = sha256_hex(&bytes);
    let server = serve(bytes.clone(), 1).await;
    let inst = installer(&format!("{}/setup.exe", server.uri()), &sha, "exe");
    let http = Http::new().unwrap();
    let first = fetch_installer(&http, &data, "Pub.App", "1.0", &inst, &noop).await;
    assert!(first.is_ok(), "{:?}", first.as_ref().err());
    drop(first);
    let second = fetch_installer(&http, &data, "Pub.App", "1.0", &inst, &noop)
        .await
        .unwrap();
    assert_eq!(std::fs::read(second.path()).unwrap(), bytes);
    server.verify().await;
}

#[tokio::test]
async fn bad_sha_and_bad_id_refused() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let bytes = body(1024);
    let sha = sha256_hex(&bytes);
    let server = serve(bytes, 0).await;
    let url = format!("{}/setup.exe", server.uri());
    let http = Http::new().unwrap();

    let bad_sha = installer(&url, "xyz", "exe");
    let r = fetch_installer(&http, &data, "Pub.App", "1.0", &bad_sha, &noop).await;
    assert!(matches!(r, Err(FetchError::BadSha)), "{:?}", r.err());

    // 63 hex digits: right alphabet, wrong length.
    let short = installer(&url, &sha[1..], "exe");
    let r = fetch_installer(&http, &data, "Pub.App", "1.0", &short, &noop).await;
    assert!(matches!(r, Err(FetchError::BadSha)), "{:?}", r.err());

    let good = installer(&url, &sha, "exe");
    let r = fetch_installer(&http, &data, "../x", "1.0", &good, &noop).await;
    assert!(matches!(r, Err(FetchError::BadId(_))), "{:?}", r.err());
    server.verify().await;
}

#[tokio::test]
async fn uppercase_sha_is_accepted() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let bytes = body(4096);
    let sha = sha256_hex(&bytes);
    let server = serve(bytes.clone(), 1).await;
    let inst = installer(
        &format!("{}/setup.exe", server.uri()),
        &sha.to_ascii_uppercase(),
        "exe",
    );
    let m = fetch_installer(&Http::new().unwrap(), &data, "Pub.App", "1.0", &inst, &noop)
        .await
        .unwrap();
    assert_eq!(m.sha256(), sha);
    assert_eq!(std::fs::read(m.path()).unwrap(), bytes);
}

#[tokio::test]
async fn version_is_sanitized() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let bytes = body(4096);
    let sha = sha256_hex(&bytes);
    let server = serve(bytes, 1).await;
    let inst = installer(&format!("{}/setup.exe", server.uri()), &sha, "exe");
    let m = fetch_installer(
        &Http::new().unwrap(),
        &data,
        "Pub.App",
        "1.0/../x",
        &inst,
        &noop,
    )
    .await
    .unwrap();
    let base = data.root().join("installers").join("Pub.App");
    assert!(m.path().starts_with(&base), "{}", m.path().display());
    let rel = m.path().strip_prefix(&base).unwrap();
    for c in rel.components() {
        assert_ne!(c.as_os_str(), "..", "{}", m.path().display());
        assert!(!c.as_os_str().to_string_lossy().contains('/'));
    }
    assert_eq!(rel.components().count(), 2, "{}", rel.display());
    assert!(m.path().is_file());
}

#[tokio::test]
async fn unusable_version_is_refused_without_request() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let bytes = body(1024);
    let sha = sha256_hex(&bytes);
    let server = serve(bytes, 0).await;
    let inst = installer(&format!("{}/setup.exe", server.uri()), &sha, "exe");
    let http = Http::new().unwrap();
    for ver in ["", "..", "1.0."] {
        let r = fetch_installer(&http, &data, "Pub.App", ver, &inst, &noop).await;
        assert!(
            matches!(r, Err(FetchError::BadId(_))),
            "{ver:?}: {:?}",
            r.err()
        );
    }
    server.verify().await;
}

#[test]
fn extension_by_kind() {
    assert_eq!(extension(&InstallerKind::Msi), "msi");
    assert_eq!(extension(&InstallerKind::Wix), "msi");
    assert_eq!(extension(&InstallerKind::Msix), "msix");
    assert_eq!(extension(&InstallerKind::Appx), "appx");
    for k in [
        InstallerKind::Inno,
        InstallerKind::Nullsoft,
        InstallerKind::Burn,
        InstallerKind::Exe,
    ] {
        assert_eq!(extension(&k), "exe", "{k:?}");
    }
}

#[cfg(windows)]
#[tokio::test]
async fn verified_open_detects_change() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let bytes = body(4096);
    let sha = sha256_hex(&bytes);
    let server = serve(bytes.clone(), 1).await;
    let inst = installer(&format!("{}/setup.exe", server.uri()), &sha, "exe");
    let m = fetch_installer(&Http::new().unwrap(), &data, "Pub.App", "1.0", &inst, &noop)
        .await
        .unwrap();
    assert!(VerifiedFile::open(m.path(), &sha).is_ok());
    let other: Vec<u8> = bytes.iter().map(|b| b ^ 0x5A).collect();
    std::fs::write(m.path(), &other).unwrap();
    let r = VerifiedFile::open(m.path(), &sha);
    assert!(matches!(r, Err(FetchError::Integrity)), "{:?}", r.err());
}

#[cfg(windows)]
#[tokio::test]
async fn verified_handle_blocks_writers_and_rename() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let bytes = body(4096);
    let sha = sha256_hex(&bytes);
    let server = serve(bytes, 1).await;
    let inst = installer(&format!("{}/setup.exe", server.uri()), &sha, "exe");
    let m = fetch_installer(&Http::new().unwrap(), &data, "Pub.App", "1.0", &inst, &noop)
        .await
        .unwrap();
    let p = m.path().to_path_buf();
    let moved = p.with_extension("moved");
    let v = VerifiedFile::open(&p, &sha).unwrap();

    let w = std::fs::OpenOptions::new().write(true).open(&p);
    assert_eq!(w.err().and_then(|e| e.raw_os_error()), Some(32));
    assert!(std::fs::remove_file(&p).is_err());
    assert!(std::fs::rename(&p, &moved).is_err());
    assert!(p.is_file());

    drop(v);
    drop(std::fs::OpenOptions::new().write(true).open(&p).unwrap());
    std::fs::rename(&p, &moved).unwrap();
    std::fs::remove_file(&moved).unwrap();
    assert!(!p.exists() && !moved.exists());
}

#[tokio::test]
async fn materialized_drop_removes_link_keeps_blob() {
    let data_tmp = tempfile::tempdir().unwrap();
    let data = DataDir::new(data_tmp.path());
    let bytes = body(4096);
    let sha = sha256_hex(&bytes);
    let server = serve(bytes.clone(), 1).await;
    let inst = installer(&format!("{}/setup.exe", server.uri()), &sha, "exe");
    let m = fetch_installer(&Http::new().unwrap(), &data, "Pub.App", "1.0", &inst, &noop)
        .await
        .unwrap();
    let p = m.path().to_path_buf();
    let ver_dir = p.parent().unwrap().to_path_buf();
    assert!(p.is_file());
    drop(m);
    assert!(!p.exists());
    assert!(!ver_dir.exists(), "empty version folder left behind");
    let blob = data.root().join("blobs").join(&sha);
    assert_eq!(std::fs::read(blob).unwrap(), bytes);
}
