#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::cache::BlobStore;
use wingetrs::http::Http;
use wingetrs::sources::winget::details::details;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const VD: &[u8] = include_bytes!("../fixtures/winget/powertoys.versionData.mszyml");
const MANIFEST: &[u8] = include_bytes!("../fixtures/winget/powertoys-0.101.2362.0.merged.yaml");
const HASH_HEX: &str = "d193b67ac71ce5d456c10f6731447abb6df68ae12ef8691c92af8ed82659e720";

async fn server(manifest: &[u8]) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/packages/Microsoft.PowerToys/d193b67a/versionData.mszyml",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(VD.to_vec()))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifests/m/Microsoft/PowerToys/0.101.2362.0/509b"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(manifest.to_vec()))
        .mount(&s)
        .await;
    s
}

#[tokio::test]
async fn latest_details_with_all_versions() {
    let s = server(MANIFEST).await;
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::new(dir.path().to_path_buf());
    let d = details(
        &Http::new().unwrap(),
        &blobs,
        &format!("{}/", s.uri()),
        "Microsoft.PowerToys",
        &hex::decode(HASH_HEX).unwrap(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(d.version, "0.101.2362.0");
    assert_eq!(d.available_versions.len(), 115);
    assert_eq!(d.available_versions[0], "0.101.2362.0");
    assert_eq!(d.installers.len(), 4);
    assert!(
        blobs
            .get("345056f4d70d74ef350a0e00a7c5a710657271a882be4fa0bc263af6bb088245")
            .is_some(),
        "manifest cached by sha256"
    );
}

#[tokio::test]
async fn tampered_manifest_is_rejected() {
    let mut bad = MANIFEST.to_vec();
    bad[10] ^= 1;
    let s = server(&bad).await;
    let dir = tempfile::tempdir().unwrap();
    let r = details(
        &Http::new().unwrap(),
        &BlobStore::new(dir.path().to_path_buf()),
        &format!("{}/", s.uri()),
        "Microsoft.PowerToys",
        &hex::decode(HASH_HEX).unwrap(),
        None,
    )
    .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn unknown_version_is_reported() {
    let s = server(MANIFEST).await;
    let dir = tempfile::tempdir().unwrap();
    let r = details(
        &Http::new().unwrap(),
        &BlobStore::new(dir.path().to_path_buf()),
        &format!("{}/", s.uri()),
        "Microsoft.PowerToys",
        &hex::decode(HASH_HEX).unwrap(),
        Some("9.9.9"),
    )
    .await;
    assert!(r.unwrap_err().to_string().contains("9.9.9"));
}

async fn server_expecting(vd_calls: u64) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/packages/Microsoft.PowerToys/d193b67a/versionData.mszyml",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(VD.to_vec()))
        .expect(vd_calls)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifests/m/Microsoft/PowerToys/0.101.2362.0/509b"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(MANIFEST.to_vec()))
        .mount(&s)
        .await;
    s
}

#[tokio::test]
async fn version_data_is_cached_by_catalog_hash() {
    let s = server_expecting(1).await;
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::new(dir.path().to_path_buf());
    let http = Http::new().unwrap();
    let base = format!("{}/", s.uri());
    let hash = hex::decode(HASH_HEX).unwrap();
    let first = details(&http, &blobs, &base, "Microsoft.PowerToys", &hash, None)
        .await
        .unwrap();
    let second = details(&http, &blobs, &base, "Microsoft.PowerToys", &hash, None)
        .await
        .unwrap();
    assert_eq!(first, second);
    assert!(blobs.get(HASH_HEX).is_some());
}

#[tokio::test]
async fn tampered_cached_version_data_is_refetched() {
    let s = server_expecting(1).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(HASH_HEX), b"garbage").unwrap();
    let blobs = BlobStore::new(dir.path().to_path_buf());
    let d = details(
        &Http::new().unwrap(),
        &blobs,
        &format!("{}/", s.uri()),
        "Microsoft.PowerToys",
        &hex::decode(HASH_HEX).unwrap(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(d.version, "0.101.2362.0");
    assert_eq!(d.available_versions.len(), 115);
    assert!(blobs.get(HASH_HEX).is_some());
}

#[tokio::test]
async fn version_data_with_another_hash_is_shown_but_not_cached() {
    let s = server_expecting(2).await;
    let dir = tempfile::tempdir().unwrap();
    let blobs = BlobStore::new(dir.path().to_path_buf());
    let http = Http::new().unwrap();
    let base = format!("{}/", s.uri());
    let mut hash = hex::decode(HASH_HEX).unwrap();
    let last = hash.len() - 1;
    hash[last] ^= 0xff;
    for _ in 0..2 {
        let d = details(&http, &blobs, &base, "Microsoft.PowerToys", &hash, None)
            .await
            .unwrap();
        assert_eq!(d.version, "0.101.2362.0");
    }
    assert!(!dir.path().join(hex::encode(&hash)).exists());
}

#[tokio::test]
async fn version_data_cache_write_failure_does_not_fail_show() {
    let s = server_expecting(2).await;
    let dir = tempfile::tempdir().unwrap();
    let ok_blobs = BlobStore::new(dir.path().join("ok"));
    // A non-empty directory where the blob should land makes only the cache rename fail.
    let broken_dir = dir.path().join("broken");
    std::fs::create_dir_all(broken_dir.join(HASH_HEX)).unwrap();
    std::fs::write(broken_dir.join(HASH_HEX).join("keep"), b"x").unwrap();
    let broken_blobs = BlobStore::new(broken_dir.clone());
    let http = Http::new().unwrap();
    let base = format!("{}/", s.uri());
    let hash = hex::decode(HASH_HEX).unwrap();
    let expected = details(&http, &ok_blobs, &base, "Microsoft.PowerToys", &hash, None)
        .await
        .unwrap();
    let got = details(
        &http,
        &broken_blobs,
        &base,
        "Microsoft.PowerToys",
        &hash,
        None,
    )
    .await;
    assert_eq!(got.unwrap(), expected);
    assert!(!broken_dir.join(HASH_HEX).is_file(), "write really failed");
}
