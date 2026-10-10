#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::buckets::*;
use wingetrs::config::Config;
use wingetrs::update::Endpoints;

#[test]
fn parse_repo_forms() {
    for s in [
        "owner/repo",
        "https://github.com/owner/repo",
        "https://github.com/owner/repo.git",
        "https://github.com/owner/repo/",
        "https://github.com/owner/repo.git/",
    ] {
        assert_eq!(parse_repo(s).unwrap(), "owner/repo", "{s}");
    }
}

#[test]
fn parse_repo_rejects_other_hosts() {
    let e = parse_repo("https://gitlab.com/owner/repo").unwrap_err();
    assert!(matches!(e, BucketError::NotGithub(_)), "{e:?}");
    assert!(matches!(
        parse_repo("git@github.com:owner/repo").unwrap_err(),
        BucketError::NotGithub(_)
    ));
    assert!(matches!(
        parse_repo("justname").unwrap_err(),
        BucketError::InvalidRepo(_)
    ));
}

#[test]
fn known_names_resolve() {
    assert_eq!(known_repo("main"), Some("ScoopInstaller/Main"));
    assert_eq!(
        known_repo("nerd-fonts"),
        Some("matthewjberger/scoop-nerd-fonts")
    );
    assert_eq!(known_repo("games"), Some("ScoopInstaller/Games"));
    assert_eq!(known_repo("nope"), None);
}

#[test]
fn add_unknown_needs_confirmation() {
    let mut cfg = Config::default();
    let e = add(&mut cfg, "mine", "me/scoop-mine", false).unwrap_err();
    assert!(matches!(e, BucketError::NeedsConfirmation { .. }), "{e:?}");
    assert!(cfg.buckets.is_empty());
    add(&mut cfg, "mine", "me/scoop-mine", true).unwrap();
    assert_eq!(cfg.buckets.len(), 1);
    add(&mut cfg, "versions", "ScoopInstaller/Versions", false).unwrap();
    assert!(cfg.scoop_buckets.contains(&"versions".to_string()));
    assert!(matches!(
        add(&mut cfg, "mine", "me/other", true).unwrap_err(),
        BucketError::Exists(_)
    ));
}

fn state_db(dir: &std::path::Path, rows: &[(&str, &str)]) -> std::path::PathBuf {
    let p = dir.join("state.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute_batch(
        "CREATE TABLE apps(app TEXT PRIMARY KEY, bucket TEXT, version TEXT, arch TEXT, installed_at INTEGER, manifest_sha256 TEXT);",
    )
    .unwrap();
    for (a, b) in rows {
        c.execute("INSERT INTO apps(app, bucket) VALUES (?1, ?2)", [a, b])
            .unwrap();
    }
    p
}

#[test]
fn rm_refuses_with_installed_apps() {
    let d = tempfile::tempdir().unwrap();
    let db = state_db(
        d.path(),
        &[("git", "main"), ("7zip", "main"), ("x", "extras")],
    );
    let mut cfg = Config::default();
    let e = remove(&mut cfg, "main", &db).unwrap_err();
    assert_eq!(
        e,
        BucketError::InUse {
            bucket: "main".into(),
            apps: vec!["7zip".into(), "git".into()]
        }
    );
    assert!(cfg.scoop_buckets.contains(&"main".to_string()));
    // no apps from "versions": removable
    add(&mut cfg, "versions", "ScoopInstaller/Versions", false).unwrap();
    remove(&mut cfg, "versions", &db).unwrap();
    assert!(!cfg.scoop_buckets.contains(&"versions".to_string()));
}

#[test]
fn rm_without_state_db_and_unknown_name() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("state.db");
    assert!(installed_from_bucket(&db, "main").unwrap().is_empty());
    let mut cfg = Config::default();
    remove(&mut cfg, "main", &db).unwrap();
    assert!(matches!(
        remove(&mut cfg, "main", &db).unwrap_err(),
        BucketError::NotConfigured(_)
    ));
}

#[test]
fn endpoints_include_added_bucket() {
    let mut cfg = Config::default();
    add(
        &mut cfg,
        "mine",
        "https://github.com/me/scoop-mine.git",
        true,
    )
    .unwrap();
    let ep = Endpoints::production(&cfg).unwrap();
    assert!(
        ep.scoop
            .iter()
            .any(|(n, u)| n == "mine"
                && u == "https://codeload.github.com/me/scoop-mine/tar.gz/HEAD")
    );
    assert!(
        ep.scoop.iter().any(|(n, u)| n == "main"
            && u == "https://codeload.github.com/ScoopInstaller/Main/tar.gz/HEAD")
    );
}

#[test]
fn config_roundtrip_with_buckets() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("config.toml");
    let mut cfg = Config::default();
    add(&mut cfg, "mine", "me/scoop-mine", true).unwrap();
    cfg.save(&p).unwrap();
    assert_eq!(Config::load(&p).unwrap(), cfg);
}

#[test]
fn unreadable_state_db_fails_closed() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("state.db");
    std::fs::write(
        &db,
        b"this is not a sqlite database at all, just text padding",
    )
    .unwrap();
    assert!(installed_from_bucket(&db, "main").is_err());
    let mut cfg = Config::default();
    assert!(remove(&mut cfg, "main", &db).is_err());
    assert!(cfg.scoop_buckets.contains(&"main".to_string()));
}

#[test]
fn parse_repo_rejects_dots_scheme_and_extra_path() {
    for s in [
        "./repo",
        "owner/..",
        "../repo",
        "http://github.com/o/r",
        "ftp://github.com/o/r",
        "o/r/extra",
        "https://github.com/o/r/tree/main",
        "a/b/c.git",
    ] {
        assert!(parse_repo(s).is_err(), "{s}");
    }
}

fn pkg(id: &str) -> wingetrs::catalog::NewPackage {
    wingetrs::catalog::NewPackage {
        id: id.into(),
        name: id.into(),
        moniker: None,
        version: "1".into(),
        publisher: None,
        description: None,
        tags: vec![],
        commands: vec![],
        arp_min_version: None,
        arp_max_version: None,
        winget_hash: None,
        raw: None,
    }
}

#[test]
fn remove_source_clears_map_rows() {
    let mut c = wingetrs::catalog::Catalog::open_in_memory().unwrap();
    let maps = wingetrs::catalog::WingetMaps {
        pfns: vec![("PFN_1".into(), "zed".into())],
        ..Default::default()
    };
    c.replace_source(
        "winget",
        wingetrs::core::SourceKind::Winget,
        "v",
        1,
        &[pkg("zed")],
        &maps,
    )
    .unwrap();
    assert_eq!(
        c.lookup(wingetrs::catalog::MapTable::Pfn, "pfn_1")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(c.remove_source("winget").unwrap(), 1);
    assert!(
        c.lookup(wingetrs::catalog::MapTable::Pfn, "pfn_1")
            .unwrap()
            .is_empty()
    );
    assert!(c.sources().unwrap().is_empty());
}

#[test]
fn remove_and_cleanup_purges_catalog_index_and_http_state() {
    use wingetrs::cache::{DataDir, HttpEntry, HttpState};
    let d = tempfile::tempdir().unwrap();
    let data = DataDir::new(d.path());
    let mut cfg = Config::default();
    add(&mut cfg, "x", "me/scoop-x", true).unwrap();
    {
        let mut c = wingetrs::catalog::Catalog::open(&data.catalog_path()).unwrap();
        let k = wingetrs::core::SourceKind::Scoop;
        let no = wingetrs::catalog::WingetMaps::default();
        c.replace_source("scoop:x", k, "v", 1, &[pkg("zebrax")], &no)
            .unwrap();
        c.replace_source("scoop:main", k, "v", 1, &[pkg("zebramain")], &no)
            .unwrap();
        wingetrs::catalog::index_file::write(&c, &data.search_index_path()).unwrap();
    }
    let url_x = "https://codeload.github.com/me/scoop-x/tar.gz/HEAD".to_string();
    let url_main = "https://codeload.github.com/ScoopInstaller/Main/tar.gz/HEAD".to_string();
    let mut st = HttpState::default();
    st.set(
        &url_x,
        HttpEntry {
            checked_at: 5,
            ..Default::default()
        },
    );
    st.set(
        &url_main,
        HttpEntry {
            checked_at: 5,
            ..Default::default()
        },
    );
    st.save(&data.http_state_path()).unwrap();

    let db = d.path().join("state.db");
    remove_and_cleanup(&data, &mut cfg, "x", &db).unwrap();

    let found = wingetrs::catalog::index_file::search(&data.search_index_path(), "zebra", None, 10)
        .unwrap()
        .unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    let c = wingetrs::catalog::Catalog::open(&data.catalog_path()).unwrap();
    let srcs: Vec<String> = c.sources().unwrap().into_iter().map(|s| s.source).collect();
    assert_eq!(srcs, vec!["scoop:main".to_string()]);
    let st = HttpState::load(&data.http_state_path());
    assert!(st.get(&url_x).is_none());
    assert!(st.get(&url_main).is_some());
    assert!(cfg.buckets.is_empty());
}
