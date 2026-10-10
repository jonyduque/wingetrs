#![allow(clippy::unwrap_used, clippy::expect_used)]
// TEMP: trocar por wingetrs::install::manifest quando a Task 24 unificar os tipos.
use wingetrs::install::download::{DownloadError, HashSpec, Source, fetch_all, verify};

use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn verify_all_algorithms_case_insensitive() {
    let b = b"abc";
    for (s, ok) in [
        (
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            true,
        ),
        (
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
            true,
        ),
        ("sha1:a9993e364706816aba3e25717850c26c9cd0d89d", true),
        ("md5:900150983cd24fb0d6963f7d28e17f72", true),
        (
            "sha512:ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            true,
        ),
        ("sha1:a9993e364706816aba3e25717850c26c9cd0d89e", false),
    ] {
        assert_eq!(verify(&HashSpec::parse(s).unwrap(), b), ok, "{s}");
    }
}

fn src(server: &MockServer, p: &str, hash: &str) -> Source {
    Source {
        url: format!("{}{p}", server.uri()),
        file_name: p.trim_start_matches('/').to_owned(),
        hash: HashSpec::parse(hash).unwrap(),
        extract_dir: None,
        extract_to: None,
    }
}

const SHA_A: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"; // "abc"
const MD5_B: &str = "md5:900150983cd24fb0d6963f7d28e17f72"; // "abc"

async fn serve(server: &MockServer, p: &str, body: &[u8]) {
    Mock::given(method("GET"))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec()))
        .mount(server)
        .await;
}

fn count_files(dir: std::path::PathBuf) -> usize {
    std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

#[tokio::test]
async fn fetches_two_sources_then_serves_from_cache() {
    let server = MockServer::start().await;
    serve(&server, "/a", b"abc").await;
    serve(&server, "/b", b"abc").await;
    let td = tempfile::tempdir().unwrap();
    let data = DataDir::new(td.path());
    let http = Http::new().unwrap();
    let sources = [src(&server, "/a", SHA_A), src(&server, "/b", MD5_B)];
    let progress = |_: &str, _: u64, _: Option<u64>| {};

    let paths = fetch_all(&http, &data, &sources, &progress).await.unwrap();
    assert_eq!(paths.len(), 2);
    for p in &paths {
        assert_eq!(std::fs::read(p).unwrap(), b"abc");
        assert!(p.starts_with(data.blobs_dir()));
    }

    server.reset().await; // mocks gone: only the cache can answer
    let again = fetch_all(&http, &data, &sources, &progress).await.unwrap();
    assert_eq!(again, paths);
}

#[tokio::test]
async fn wrong_bytes_give_hash_mismatch_and_leave_no_files() {
    let server = MockServer::start().await;
    serve(&server, "/a", b"not abc").await;
    let td = tempfile::tempdir().unwrap();
    let data = DataDir::new(td.path());
    let http = Http::new().unwrap();
    let progress = |_: &str, _: u64, _: Option<u64>| {};
    let err = fetch_all(&http, &data, &[src(&server, "/a", SHA_A)], &progress)
        .await
        .unwrap_err();
    assert!(matches!(err, DownloadError::HashMismatch { .. }), "{err:?}");
    assert_eq!(count_files(data.blobs_dir()), 0);
    assert_eq!(count_files(data.tmp_dir()), 0);
}

#[tokio::test]
async fn tampered_cached_blob_is_redownloaded() {
    let server = MockServer::start().await;
    serve(&server, "/a", b"abc").await;
    let td = tempfile::tempdir().unwrap();
    let data = DataDir::new(td.path());
    let http = Http::new().unwrap();
    let progress = |_: &str, _: u64, _: Option<u64>| {};
    let sources = [src(&server, "/a", SHA_A)];
    let paths = fetch_all(&http, &data, &sources, &progress).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 1);

    std::fs::write(&paths[0], b"evil").unwrap();
    let again = fetch_all(&http, &data, &sources, &progress).await.unwrap();
    assert_eq!(std::fs::read(&again[0]).unwrap(), b"abc");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[test]
fn verify_md5_rejects_wrong_digest() {
    let spec = HashSpec::parse("md5:900150983cd24fb0d6963f7d28e17f73").unwrap();
    assert!(!verify(&spec, b"abc"));
}

#[tokio::test]
async fn index_pointing_at_wrong_blob_is_not_trusted() {
    let server = MockServer::start().await;
    serve(&server, "/b", b"abc").await;
    let td = tempfile::tempdir().unwrap();
    let data = DataDir::new(td.path());
    let http = Http::new().unwrap();
    let progress = |_: &str, _: u64, _: Option<u64>| {};
    // A perfectly valid blob (content matches its sha256 name) but of other bytes.
    let other = b"other";
    let other_sha = "d9298a10d1b0735837dc4bd85dac641b0f3cef27a47e5d53a54f2f3f5b2fcffa"; // sha256("other")
    std::fs::create_dir_all(data.blobs_dir()).unwrap();
    std::fs::write(data.blobs_dir().join(other_sha), other).unwrap();
    std::fs::write(
        data.blobs_dir().join("index.json"),
        format!(r#"{{"{MD5_B}":"{other_sha}"}}"#),
    )
    .unwrap();
    let paths = fetch_all(&http, &data, &[src(&server, "/b", MD5_B)], &progress)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&paths[0]).unwrap(), b"abc");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
