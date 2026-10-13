#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::cache::{BlobStore, DataDir, sha256_hex};
use wingetrs::core::IconRef;
use wingetrs::http::Http;
use wingetrs::icons::{self, IconError, MAX_ICON_BYTES};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn icon_ref(server: &MockServer, sha: &str) -> IconRef {
    IconRef {
        url: format!("{}/i.ico", server.uri()),
        sha256: sha.into(),
    }
}

#[tokio::test]
async fn fetch_caches_by_sha_and_rejects_mismatch() {
    let t = tempfile::tempdir().unwrap();
    let blobs_dir = t.path().join("blobs");
    let blobs = BlobStore::new(blobs_dir.clone());
    let http = Http::new().unwrap();
    let body = b"icon-bytes".to_vec();
    let sha = sha256_hex(&body);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/i.ico"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let r = icon_ref(&server, &sha);
    assert_eq!(icons::fetch(&http, &blobs, &r).await.unwrap(), body);
    assert!(blobs_dir.join(&sha).is_file());
    assert_eq!(icons::fetch(&http, &blobs, &r).await.unwrap(), body);
    server.verify().await;

    // Wrong content for the announced hash.
    let wrong = "a".repeat(64);
    let wrong_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/i.ico"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&wrong_server)
        .await;
    let r = icon_ref(&wrong_server, &wrong);
    let err = icons::fetch(&http, &blobs, &r).await.unwrap_err();
    assert!(matches!(err, IconError::Integrity), "{err:?}");
    assert!(!blobs_dir.join(&wrong).exists());

    // Over the size limit.
    let big_server = MockServer::start().await;
    let big = vec![7u8; MAX_ICON_BYTES + 1];
    Mock::given(method("GET"))
        .and(path("/i.ico"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(big.clone()))
        .mount(&big_server)
        .await;
    let big_sha = sha256_hex(&big);
    let r = icon_ref(&big_server, &big_sha);
    let err = icons::fetch(&http, &blobs, &r).await.unwrap_err();
    assert!(matches!(err, IconError::TooLarge), "{err:?}");
    assert!(!blobs_dir.join(&big_sha).exists());
}

#[test]
fn icon_map_round_trips_and_survives_corruption() {
    let t = tempfile::tempdir().unwrap();
    let data = DataDir::new(t.path().join("w"));
    data.ensure().unwrap();
    let bytes = b"png".to_vec();
    let sha = sha256_hex(&bytes);
    BlobStore::new(data.blobs_dir())
        .put_verified(&sha, &bytes)
        .unwrap();

    icons::remember(&data, "winget", "A.B", &sha);
    let map = icons::load_map(&data);
    assert_eq!(map.get("winget/A.B"), Some(&sha));
    assert_eq!(
        icons::cached_icon(&data, &map, "winget", "A.B"),
        Some(bytes)
    );
    assert_eq!(icons::cached_icon(&data, &map, "winget", "X.Y"), None);

    // Mapped blob missing.
    icons::remember(&data, "winget", "C.D", &"b".repeat(64));
    let map = icons::load_map(&data);
    assert_eq!(icons::cached_icon(&data, &map, "winget", "C.D"), None);

    // Corrupt file: empty map, and the next remember rewrites it validly.
    std::fs::write(data.root().join("icons.json"), b"{not json").unwrap();
    assert!(icons::load_map(&data).is_empty());
    icons::remember(&data, "winget", "A.B", &sha);
    assert_eq!(icons::load_map(&data).get("winget/A.B"), Some(&sha));
}
