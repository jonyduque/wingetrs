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

#[test]
fn concurrent_remember_keeps_every_entry() {
    const THREADS: usize = 16;
    const CALLS: usize = 4;
    let sha_of = |i: usize, c: usize| format!("{:032x}{:032x}", i, c);
    for _round in 0..2 {
        let t = tempfile::tempdir().unwrap();
        let data = DataDir::new(t.path().join("w"));
        data.ensure().unwrap();
        let barrier = std::sync::Barrier::new(THREADS);
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..THREADS)
                .map(|i| {
                    let (data, barrier) = (&data, &barrier);
                    s.spawn(move || {
                        barrier.wait();
                        for c in 0..CALLS {
                            icons::remember(data, "winget", &format!("P.{i}"), &sha_of(i, c));
                        }
                    })
                })
                .collect();
            for h in handles {
                if let Err(p) = h.join() {
                    std::panic::resume_unwind(p);
                }
            }
        });
        let map = icons::load_map(&data);
        assert_eq!(map.len(), THREADS);
        for i in 0..THREADS {
            assert_eq!(
                map.get(&format!("winget/P.{i}")),
                Some(&sha_of(i, CALLS - 1)),
                "entry {i}"
            );
        }
    }
}

/// Raw HTTP server that sends `body_len` bytes with no Content-Length (close-delimited).
async fn serve_unsized(body_len: usize) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = listener.accept().await {
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .await;
            let _ = s.write_all(&vec![7u8; body_len]).await;
            let _ = s.shutdown().await;
        }
    });
    format!("http://{addr}/i.ico")
}

#[tokio::test]
async fn get_limited_stops_a_stream_without_content_length() {
    let http = Http::new().unwrap();
    let url = serve_unsized(MAX_ICON_BYTES + 1).await;
    let err = http.get_limited(&url, MAX_ICON_BYTES).await.unwrap_err();
    assert!(
        matches!(err, wingetrs::http::HttpError::TooLarge(_)),
        "{err:?}"
    );
    let url = serve_unsized(MAX_ICON_BYTES).await;
    assert_eq!(
        http.get_limited(&url, MAX_ICON_BYTES).await.unwrap().len(),
        MAX_ICON_BYTES
    );
}

#[tokio::test]
async fn get_limited_refuses_on_content_length_before_reading() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((mut s, _)) = listener.accept().await {
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\nshort",
                MAX_ICON_BYTES + 1
            );
            let _ = s.write_all(head.as_bytes()).await;
            let _ = s.shutdown().await;
        }
    });
    let http = Http::new().unwrap();
    let err = http
        .get_limited(&format!("http://{addr}/i.ico"), MAX_ICON_BYTES)
        .await
        .unwrap_err();
    assert!(
        matches!(err, wingetrs::http::HttpError::TooLarge(_)),
        "{err:?}"
    );
}
