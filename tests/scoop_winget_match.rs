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

fn stored(id: &str) -> wingetrs::catalog::StoredPackage {
    wingetrs::catalog::StoredPackage {
        entry: wingetrs::core::PackageEntry {
            source: wingetrs::core::SourceKind::Winget,
            source_name: "winget".into(),
            id: id.into(),
            name: id.into(),
            moniker: None,
            version: "1".into(),
            publisher: None,
            description: None,
            tags: Vec::new(),
            commands: Vec::new(),
        },
        arp_min_version: None,
        arp_max_version: None,
        winget_hash: None,
        raw: None,
    }
}

/// Winget details with a homepage and, when `icon` is set, an icon whose sha is the id.
fn details(id: &str, home: Option<&str>, icon: bool) -> wingetrs::core::PackageDetails {
    wingetrs::core::PackageDetails {
        source: wingetrs::core::SourceKind::Winget,
        id: id.into(),
        name: id.into(),
        version: "1".into(),
        publisher: None,
        description: None,
        homepage: home.map(str::to_string),
        license: None,
        tags: Vec::new(),
        installers: Vec::new(),
        available_versions: Vec::new(),
        icon: icon.then(|| wingetrs::core::IconRef {
            url: format!("https://x/{id}.png"),
            sha256: id.into(),
        }),
    }
}

fn twin(pkgs: &[(&str, Option<&str>)], home: &str) -> Option<wingetrs::core::IconRef> {
    let stored: Vec<_> = pkgs.iter().map(|(id, _)| stored(id)).collect();
    let homes: std::collections::HashMap<_, _> = pkgs.iter().copied().collect();
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(wingetrs::sources::scoop::winget_match::twin_from(
            &stored,
            home,
            |p| {
                let d = details(&p.entry.id, homes[p.entry.id.as_str()], true);
                async move { Some(d) }
            },
        ))
}

#[test]
fn twin_icon_is_the_unique_same_site_candidates() {
    let home = "https://www.7-zip.org/";
    let icon = twin(
        &[
            ("A.One", Some("https://7-zip.org/x")),
            ("B.Two", Some("https://o.org")),
        ],
        home,
    );
    assert_eq!(icon.map(|i| i.sha256).as_deref(), Some("A.One"));
    assert_eq!(
        twin(
            &[
                ("A.One", Some("https://7-zip.org")),
                ("B.Two", Some("https://7-zip.org/a"))
            ],
            home
        ),
        None
    );
    let icon = twin(
        &[("A.One", None), ("B.Two", Some("https://7-zip.org"))],
        home,
    );
    assert_eq!(icon.map(|i| i.sha256).as_deref(), Some("B.Two"));
    assert_eq!(twin(&[("A.One", None)], home), None);
}

#[test]
fn details_are_sent_before_the_twin_search() {
    let src = include_str!("../src/tui/run.rs");
    let from = src.find("fn spawn_details").unwrap();
    let body = &src[from..from + src[from..].find("/// Debounced fetch").unwrap()];
    let sent = body.find("ctx.tx.send(msg)").unwrap();
    // `details_icon` is where the twin search (`icon_ref`) starts.
    let twin = body.find("details_icon(").unwrap();
    assert!(sent < twin, "{body}");
    if let Some(direct) = body.find("icon_ref(") {
        assert!(sent < direct, "{body}");
    }
    let icon = src.find("async fn details_icon").unwrap();
    let icon_body = &src[icon..icon + src[icon..].find("async fn load_details").unwrap()];
    assert!(icon_body.contains("icon_ref("), "{icon_body}");
    let load = src.find("async fn load_details").unwrap();
    let load_body = &src[load..load + src[load..].find("async fn winget_details").unwrap()];
    assert!(!load_body.contains("twin_icon"), "{load_body}");
}
