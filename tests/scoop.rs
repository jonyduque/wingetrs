#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::sources::scoop::bucket::{ScoopFetch, bucket_url, fetch_bucket, read_tarball};
use wingetrs::sources::scoop::manifest::{details_from_raw, parse};

const TGZ: &[u8] = include_bytes!("fixtures/scoop/Main-mini.tar.gz");

#[test]
fn tarball_imports_valid_and_skips_broken() {
    let imp = read_tarball(TGZ).unwrap();
    let mut ids: Vec<&str> = imp.packages.iter().map(|p| p.id.as_str()).collect();
    ids.sort();
    assert_eq!(
        ids,
        [
            "7zip",
            "aria2",
            "ffmpeg",
            "gh",
            "git",
            "nodejs-lts",
            "python",
            "ripgrep"
        ]
    );
    assert_eq!(imp.skipped, ["broken.json"]);
}

#[test]
fn commands_from_bin_variants() {
    let imp = read_tarball(TGZ).unwrap();
    let get = |id: &str| imp.packages.iter().find(|p| p.id == id).unwrap().clone();
    assert_eq!(get("7zip").commands, ["7z", "7zg", "7zfm"]);
    assert_eq!(get("ripgrep").commands, ["rg"]);
    assert_eq!(get("python").commands[0], "python3");
    assert_eq!(get("7zip").version, "26.04");
}

#[test]
fn bom_is_tolerated() {
    let mut j = vec![0xEF, 0xBB, 0xBF];
    j.extend_from_slice(
        br#"{"version":"1.0","description":"d","url":"https://x/a.zip","hash":"aa","bin":"a.exe"}"#,
    );
    let p = parse("a", &j).unwrap();
    assert_eq!(p.version, "1.0");
    assert!(!p.raw.as_ref().unwrap().starts_with('\u{feff}'));
}

#[test]
fn details_map_architectures_and_license_object() {
    let imp = read_tarball(TGZ).unwrap();
    let raw = imp
        .packages
        .iter()
        .find(|p| p.id == "7zip")
        .unwrap()
        .raw
        .clone()
        .unwrap();
    let d = details_from_raw("7zip", &raw).unwrap();
    assert!(d.license.as_deref().unwrap().starts_with("BSD-2-Clause"));
    let archs: Vec<&str> = d
        .installers
        .iter()
        .map(|i| i.architecture.as_str())
        .collect();
    assert_eq!(archs, ["x64", "x86", "arm64"]);
    assert!(
        d.installers
            .iter()
            .all(|i| i.installer_type == "scoop" && i.scope.as_deref() == Some("user"))
    );
    assert_eq!(d.homepage.as_deref(), Some("https://www.7-zip.org"));
}

#[test]
fn neutral_installer_without_architecture() {
    let raw = r#"{"version":"2","url":["https://x/one.zip","https://x/two.zip"],"hash":["a","b"]}"#;
    let d = details_from_raw("n", raw).unwrap();
    assert_eq!(d.installers.len(), 1);
    assert_eq!(d.installers[0].architecture, "neutral");
    assert_eq!(d.installers[0].url, "https://x/one.zip");
}

#[test]
fn bucket_urls() {
    assert_eq!(
        bucket_url("main").unwrap(),
        "https://codeload.github.com/ScoopInstaller/Main/tar.gz/refs/heads/master"
    );
    assert!(bucket_url("choco").is_err());
}

#[tokio::test]
async fn fetch_bucket_conditional() {
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/b.tgz"))
        .and(header_exists("if-none-match"))
        .respond_with(ResponseTemplate::new(304))
        .with_priority(1)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/b.tgz"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"e1\"")
                .set_body_bytes(TGZ.to_vec()),
        )
        .with_priority(5)
        .mount(&s)
        .await;
    let http = wingetrs::http::Http::new().unwrap();
    let url = format!("{}/b.tgz", s.uri());
    let first = fetch_bucket(&http, &url, None).await.unwrap();
    let ScoopFetch::Modified { import, entry } = first else {
        panic!("expected Modified")
    };
    assert_eq!(import.packages.len(), 8);
    assert_eq!(entry.etag.as_deref(), Some("\"e1\""));
    let second = fetch_bucket(&http, &url, Some(&entry)).await.unwrap();
    assert!(matches!(second, ScoopFetch::NotModified));
}
