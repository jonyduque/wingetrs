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
    assert!(installed_from_bucket(&db, "main").is_empty());
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
fn rm_from_catalog_drops_source() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("catalog.db");
    {
        let mut c = wingetrs::catalog::Catalog::open(&p).unwrap();
        c.replace_source(
            "scoop:mine",
            wingetrs::core::SourceKind::Scoop,
            "v",
            1,
            &[],
            &wingetrs::catalog::WingetMaps::default(),
        )
        .unwrap();
    }
    remove_from_catalog(&p, "mine").unwrap();
    let c = wingetrs::catalog::Catalog::open(&p).unwrap();
    assert!(c.sources().unwrap().is_empty());
}
