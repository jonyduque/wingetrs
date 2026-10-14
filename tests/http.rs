#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::time::Duration;
use wingetrs::cache::HttpEntry;
use wingetrs::http::{Fetch, Http, HttpError};
use wiremock::matchers::{header, headers, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn first_get_returns_body_and_validators() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/source2.msix"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(b"abc".to_vec())
                .insert_header("ETag", "\"0x1\"")
                .insert_header("Last-Modified", "Wed, 07 Oct 2026 03:02:43 GMT")
                .insert_header("x-ms-meta-SourceVersion", "2026.1007.459.0"),
        )
        .mount(&s)
        .await;
    let h = Http::new().unwrap();
    match h
        .get_conditional(&format!("{}/source2.msix", s.uri()), None)
        .await
        .unwrap()
    {
        Fetch::Modified { body, entry } => {
            assert_eq!(body, b"abc");
            assert_eq!(entry.etag.as_deref(), Some("\"0x1\""));
            assert_eq!(
                entry.last_modified.as_deref(),
                Some("Wed, 07 Oct 2026 03:02:43 GMT")
            );
            assert_eq!(entry.source_version.as_deref(), Some("2026.1007.459.0"));
            assert!(entry.checked_at > 1_700_000_000);
        }
        other => panic!("expected Modified, got {other:?}"),
    }
}

#[tokio::test]
async fn sends_validators_and_maps_304() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/x"))
        .and(header("If-None-Match", "\"0x1\""))
        // wiremock splits request header values on commas, so the HTTP-date arrives as two parts
        .and(headers(
            "If-Modified-Since",
            vec!["Wed", "07 Oct 2026 03:02:43 GMT"],
        ))
        .respond_with(ResponseTemplate::new(304))
        .expect(1)
        .mount(&s)
        .await;
    let prev = HttpEntry {
        etag: Some("\"0x1\"".into()),
        last_modified: Some("Wed, 07 Oct 2026 03:02:43 GMT".into()),
        source_version: None,
        checked_at: 0,
    };
    let r = Http::new()
        .unwrap()
        .get_conditional(&format!("{}/x", s.uri()), Some(&prev))
        .await
        .unwrap();
    assert_eq!(r, Fetch::NotModified);
}

#[tokio::test]
async fn non_success_status_is_error() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&s)
        .await;
    let e = Http::new()
        .unwrap()
        .get(&format!("{}/nope", s.uri()))
        .await
        .unwrap_err();
    assert!(matches!(e, HttpError::Status(404)), "{e:?}");
    assert!(!e.is_offline());
}

#[tokio::test]
async fn slow_server_times_out() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(1500)))
        .mount(&s)
        .await;
    let h = Http::with_timeouts(Duration::from_millis(200), Duration::from_millis(300)).unwrap();
    let e = h.get(&s.uri()).await.unwrap_err();
    assert!(matches!(e, HttpError::Timeout), "{e:?}");
    assert!(e.is_offline());
}

#[tokio::test]
async fn unreachable_host_is_offline() {
    let h = Http::with_timeouts(Duration::from_millis(300), Duration::from_millis(500)).unwrap();
    let e = h.get("http://127.0.0.1:9/").await.unwrap_err();
    assert!(e.is_offline(), "{e:?}");
}

#[test]
fn redirect_allowed_refuses_only_https_to_http() {
    use wingetrs::http::redirect_allowed;
    let u = |s: &str| reqwest::Url::parse(s).unwrap();
    for allow in [false, true] {
        for (prev, next) in [
            ("https://a/", "http://b/"),
            ("https://a/", "https://b/"),
            ("http://a/", "http://b/"),
            ("http://a/", "https://b/"),
        ] {
            let expected = allow || !(prev.starts_with("https") && next.starts_with("http:"));
            assert_eq!(
                redirect_allowed(&u(prev), &u(next), allow),
                expected,
                "{prev} -> {next} allow={allow}"
            );
        }
    }
}

#[tokio::test]
async fn http_redirect_is_still_followed() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/a"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/b"))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/b"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"dest".to_vec()))
        .mount(&s)
        .await;
    let h = Http::new().unwrap();
    assert_eq!(h.get(&format!("{}/a", s.uri())).await.unwrap(), b"dest");
}

#[tokio::test]
async fn redirect_loop_is_still_limited() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/loop"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/loop"))
        .mount(&s)
        .await;
    let h = Http::new().unwrap();
    let r = tokio::time::timeout(Duration::from_secs(20), h.get(&format!("{}/loop", s.uri())))
        .await
        .expect("redirect loop must fail, not hang");
    let e = r.unwrap_err();
    assert!(!matches!(e, HttpError::InsecureRedirect(_)), "{e}");
}

#[test]
fn insecure_redirect_is_not_offline() {
    let e = HttpError::InsecureRedirect("http://x".into());
    assert!(!e.is_offline());
    assert!(e.to_string().contains("permita em Ajustes"));
}

#[tokio::test]
#[ignore = "live: https to http redirect via httpbin"]
async fn live_https_to_http_redirect_is_refused() {
    let url = "https://httpbin.org/redirect-to?url=http://example.com";
    let h = Http::new().unwrap();
    match h.get(url).await {
        Err(HttpError::InsecureRedirect(u)) => assert!(u.starts_with("http://example.com"), "{u}"),
        other => panic!("expected InsecureRedirect, got {other:?}"),
    }
    h.set_allow_http_redirect(true);
    if let Err(e) = h.get(url).await {
        assert!(!matches!(e, HttpError::InsecureRedirect(_)), "{e}");
    }
}
