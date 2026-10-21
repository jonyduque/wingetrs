//! Task 208: GitHub releases client.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::github::{self, GhError, GhRepo};
use wingetrs::http::Http;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RIPGREP: &[u8] = include_bytes!("../fixtures/github/ripgrep.json");
const BAT: &[u8] = include_bytes!("../fixtures/github/bat.json");
const JQ: &[u8] = include_bytes!("../fixtures/github/jq.json");
const NPP: &[u8] = include_bytes!("../fixtures/github/notepad-plus-plus.json");
const PWSH: &[u8] = include_bytes!("../fixtures/github/powershell.json");

fn json(body: &[u8]) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("Content-Type", "application/json")
        .set_body_bytes(body.to_vec())
}

fn repo(spec: &str) -> GhRepo {
    GhRepo::parse(spec).unwrap()
}

#[tokio::test]
async fn latest_parses_assets_with_digest() {
    let s = MockServer::start().await;
    let h = Http::new().unwrap();
    let cases: [(&str, &[u8], &str, usize); 5] = [
        ("BurntSushi/ripgrep", RIPGREP, "15.2.0", 28),
        ("sharkdp/bat", BAT, "v0.26.1", 21),
        ("jqlang/jq", JQ, "jq-1.8.2", 29),
        ("notepad-plus-plus/notepad-plus-plus", NPP, "v8.9.8.1", 28),
        ("PowerShell/PowerShell", PWSH, "v7.6.7", 37),
    ];
    for (spec, body, tag, count) in cases {
        Mock::given(method("GET"))
            .and(path(format!("/repos/{spec}/releases/latest")))
            .respond_with(json(body))
            .mount(&s)
            .await;
        let rel = github::latest(&h, &s.uri(), &repo(spec)).await.unwrap();
        assert_eq!(rel.tag, tag, "{spec}");
        assert!(!rel.prerelease, "{spec}");
        assert_eq!(rel.assets.len(), count, "{spec}");
    }
    let rel = github::latest(&h, &s.uri(), &repo("BurntSushi/ripgrep"))
        .await
        .unwrap();
    let a = rel
        .assets
        .iter()
        .find(|a| a.name == "ripgrep-15.2.0-i686-pc-windows-msvc.zip")
        .unwrap();
    assert_eq!(
        hex::encode(a.sha256),
        "9bf73bdb3fda9ad4b0235e1295b02c717031c986afa4d7c05dd0af8b74010a95"
    );
    assert_eq!(a.size, 1642641);
    assert_eq!(
        a.url,
        "https://github.com/BurntSushi/ripgrep/releases/download/15.2.0/ripgrep-15.2.0-i686-pc-windows-msvc.zip"
    );
}

fn release_with(assets: &str) -> Vec<u8> {
    format!(
        r#"{{"tag_name":"v1","prerelease":false,"draft":false,"immutable":true,"assets":[{assets}]}}"#
    )
    .into_bytes()
}

const GOOD_HEX: &str = "9bf73bdb3fda9ad4b0235e1295b02c717031c986afa4d7c05dd0af8b74010a95";

fn asset(name: &str, digest: Option<&str>, url: &str) -> String {
    let digest = digest.map_or("null".to_string(), |d| format!("\"{d}\""));
    format!(r#"{{"name":"{name}","size":10,"digest":{digest},"browser_download_url":"{url}"}}"#)
}

async fn latest_of(body: Vec<u8>) -> github::Release {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/releases/latest"))
        .respond_with(json(&body))
        .mount(&s)
        .await;
    github::latest(&Http::new().unwrap(), &s.uri(), &repo("o/r"))
        .await
        .unwrap()
}

#[tokio::test]
async fn asset_without_digest_dropped() {
    let url = "https://github.com/o/r/releases/download/v1/";
    let digest_ok = format!("sha256:{GOOD_HEX}");
    let short = format!("sha256:{}", &GOOD_HEX[..63]);
    let md5 = format!("md5:{GOOD_HEX}");
    let not_hex = format!("sha256:{}", "z".repeat(64));
    let assets = [
        asset("ok.zip", Some(&digest_ok), &format!("{url}ok.zip")),
        asset("null.zip", None, &format!("{url}null.zip")),
        asset("short.zip", Some(&short), &format!("{url}short.zip")),
        asset("md5.zip", Some(&md5), &format!("{url}md5.zip")),
        asset("nothex.zip", Some(&not_hex), &format!("{url}nothex.zip")),
    ]
    .join(",");
    let rel = latest_of(release_with(&assets)).await;
    let names: Vec<_> = rel.assets.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["ok.zip"]);
    assert!(rel.immutable);
}

#[tokio::test]
async fn foreign_download_host_dropped() {
    let digest = format!("sha256:{GOOD_HEX}");
    let assets = [
        asset(
            "gh.zip",
            Some(&digest),
            "https://github.com/o/r/releases/download/v1/gh.zip",
        ),
        asset(
            "cdn.zip",
            Some(&digest),
            "https://objects.githubusercontent.com/github-production-release-asset/1/cdn.zip",
        ),
        asset(
            "evil.zip",
            Some(&digest),
            "https://evil.example/o/r/releases/download/v1/evil.zip",
        ),
        asset(
            "http.zip",
            Some(&digest),
            "http://github.com/o/r/releases/download/v1/http.zip",
        ),
        asset(
            "other.zip",
            Some(&digest),
            "https://github.com/x/y/releases/download/v1/other.zip",
        ),
        asset(
            "lookalike.zip",
            Some(&digest),
            "https://objects.githubusercontent.com.evil.example/lookalike.zip",
        ),
        asset(
            "userinfo.zip",
            Some(&digest),
            "https://objects.githubusercontent.com@evil.example/userinfo.zip",
        ),
        asset(
            "ghlook.zip",
            Some(&digest),
            "https://github.com.evil.example/o/r/releases/download/v1/ghlook.zip",
        ),
        asset(
            "ghuser.zip",
            Some(&digest),
            "https://github.com@evil.example/o/r/releases/download/v1/ghuser.zip",
        ),
    ]
    .join(",");
    let rel = latest_of(release_with(&assets)).await;
    let names: Vec<_> = rel.assets.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["gh.zip", "cdn.zip"]);
}

#[tokio::test]
async fn rate_limited_reports_reset() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/releases/latest"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("X-RateLimit-Remaining", "0")
                .insert_header("X-RateLimit-Reset", "1760900000"),
        )
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/o/forbidden/releases/latest"))
        .respond_with(ResponseTemplate::new(403).insert_header("X-RateLimit-Remaining", "12"))
        .mount(&s)
        .await;
    let h = Http::new().unwrap();
    match github::latest(&h, &s.uri(), &repo("o/r")).await {
        Err(GhError::RateLimited { reset }) => assert_eq!(reset, Some(1760900000)),
        other => panic!("expected RateLimited, got {other:?}"),
    }
    match github::latest(&h, &s.uri(), &repo("o/forbidden")).await {
        Err(GhError::Status(403)) => {}
        other => panic!("expected Status(403), got {other:?}"),
    }
}

#[tokio::test]
async fn not_found_is_typed() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&s)
        .await;
    let h = Http::new().unwrap();
    assert!(matches!(
        github::latest(&h, &s.uri(), &repo("o/r")).await,
        Err(GhError::NotFound)
    ));
    assert!(matches!(
        github::by_tag(&h, &s.uri(), &repo("o/r"), "v9").await,
        Err(GhError::NotFound)
    ));
    assert!(matches!(
        github::latest_tag(&h, &s.uri(), &repo("o/r")).await,
        Err(GhError::NotFound)
    ));
}

#[tokio::test]
async fn draft_or_garbage_is_typed() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/o/draft/releases/latest"))
        .respond_with(json(br#"{"tag_name":"v1","draft":true,"assets":[]}"#))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/o/bad/releases/latest"))
        .respond_with(json(b"<html>"))
        .mount(&s)
        .await;
    let h = Http::new().unwrap();
    assert!(matches!(
        github::latest(&h, &s.uri(), &repo("o/draft")).await,
        Err(GhError::NoRelease)
    ));
    assert!(matches!(
        github::latest(&h, &s.uri(), &repo("o/bad")).await,
        Err(GhError::BadJson)
    ));
}

#[tokio::test]
async fn latest_tag_from_redirect() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/o/r/releases/latest"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", "/o/r/releases/tag/v1.2%2Brc%201"),
        )
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/o/r/releases/tag/v1.2%2Brc%201"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/o/plain/releases/latest"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
        .mount(&s)
        .await;
    let h = Http::new().unwrap();
    assert_eq!(
        github::latest_tag(&h, &s.uri(), &repo("o/r"))
            .await
            .unwrap(),
        "v1.2+rc 1"
    );
    // A 200 that is not a redirect to a tag page has no tag to read.
    assert!(matches!(
        github::latest_tag(&h, &s.uri(), &repo("o/plain")).await,
        Err(GhError::NoRelease)
    ));
}

#[tokio::test]
async fn by_tag_uses_tags_path() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/releases/tags/v1.0"))
        .respond_with(json(&release_with("")))
        .expect(1)
        .mount(&s)
        .await;
    let rel = github::by_tag(&Http::new().unwrap(), &s.uri(), &repo("o/r"), "v1.0")
        .await
        .unwrap();
    assert_eq!(rel.tag, "v1");
    assert!(rel.assets.is_empty());
}

#[test]
fn repo_names_follow_bucket_rules() {
    assert!(GhRepo::parse("BurntSushi/ripgrep").is_ok());
    assert!(GhRepo::parse("a.b/c_d-e").is_ok());
    for bad in [
        "", "a", "a/", "/b", "../b", "a/..", "a/b/c", "a b/c", "a/b?x", "a..b/c",
    ] {
        assert!(GhRepo::parse(bad).is_err(), "{bad}");
    }
}
