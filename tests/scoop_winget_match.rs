#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::sources::scoop::winget_match::{pick, site};

#[test]
fn site_normalizes_hosts_and_code_forges() {
    assert_eq!(site("https://www.7-zip.org/").as_deref(), Some("7-zip.org"));
    assert_eq!(
        site("https://github.com/BurntSushi/ripgrep").as_deref(),
        Some("github.com/burntsushi")
    );
    assert_eq!(
        site("http://GITLAB.com/Foo/bar/-/x").as_deref(),
        Some("gitlab.com/foo")
    );
    assert_eq!(site("lixo"), None);
}

fn cand(id: &str, home: Option<&str>) -> (String, Option<String>) {
    (id.to_string(), home.map(str::to_string))
}

#[test]
fn pick_requires_exactly_one_same_site() {
    let home = "https://www.7-zip.org/";
    let one = [
        cand("A.One", Some("http://7-zip.org/x")),
        cand("B.Two", Some("https://other.org")),
    ];
    assert_eq!(pick(home, &one).as_deref(), Some("A.One"));
    let two = [
        cand("A.One", Some("https://7-zip.org")),
        cand("B.Two", Some("https://www.7-zip.org/a")),
    ];
    assert_eq!(pick(home, &two), None);
    let none = [cand("B.Two", Some("https://other.org"))];
    assert_eq!(pick(home, &none), None);
    let missing = [
        cand("A.One", None),
        cand("B.Two", Some("https://7-zip.org")),
    ];
    assert_eq!(pick(home, &missing).as_deref(), Some("B.Two"));
    assert_eq!(pick(home, &[cand("A.One", None)]), None);
}

// Run with: cargo test --release --test scoop_winget_match -- --ignored --nocapture live_
#[test]
#[ignore = "live: reads a copy of the real catalog and fetches winget manifests; times winget_candidates"]
fn live_winget_candidates_timing_and_twins() {
    use wingetrs::catalog::Catalog;
    let data = wingetrs::cache::DataDir::default_location().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("catalog.db");
    std::fs::copy(data.catalog_path(), &copy).unwrap();
    let cat = Catalog::open_read_only(&copy).unwrap();
    let blobs = wingetrs::cache::BlobStore::new(dir.path().join("blobs"));
    let http = wingetrs::http::Http::new().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    for app in ["7zip", "ripgrep", "git", "docker", "nodejs"] {
        let t = std::time::Instant::now();
        let ids = cat.winget_candidates(app).unwrap();
        let us = t.elapsed().as_micros();
        let Some(scoop) = cat
            .find_by_id(app)
            .unwrap()
            .into_iter()
            .find(|p| p.raw.is_some())
        else {
            eprintln!("{app}: not in the scoop catalog (candidates {ids:?}, {us} us)");
            continue;
        };
        let home = wingetrs::sources::scoop::manifest::details_from_raw(
            app,
            scoop.raw.as_deref().unwrap(),
        )
        .unwrap()
        .homepage;
        let mut cands = Vec::new();
        let mut icons = std::collections::BTreeMap::new();
        for id in &ids {
            let p = cat.get("winget", id).unwrap().unwrap();
            let d = rt
                .block_on(wingetrs::sources::winget::details::details(
                    &http,
                    &blobs,
                    wingetrs::sources::winget::CDN_BASE,
                    id,
                    p.winget_hash.as_deref().unwrap(),
                    None,
                ))
                .unwrap();
            icons.insert(id.clone(), d.icon.is_some());
            cands.push((id.clone(), d.homepage));
        }
        let twin = home.as_deref().and_then(|h| pick(h, &cands));
        eprintln!(
            "{app}: candidates_us={us} home={home:?} cands={cands:?} twin={twin:?} icons={icons:?}"
        );
    }
}
