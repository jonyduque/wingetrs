#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::cache::DataDir;
use wingetrs::installed::overrides::{Choice, load, set};

#[test]
fn overrides_round_trip_and_survive_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    assert!(load(&data).is_empty());

    let pkg = Choice::Package {
        source: "winget".into(),
        package_id: "A.A".into(),
    };
    set(&data, "arp/{G}", Some(pkg.clone())).unwrap();
    set(&data, "scoop/x", Some(Choice::NotInCatalog)).unwrap();
    let map = load(&data);
    assert_eq!(map.get("arp/{G}"), Some(&pkg));
    assert_eq!(map.get("scoop/x"), Some(&Choice::NotInCatalog));

    set(&data, "scoop/x", None).unwrap();
    assert_eq!(load(&data).len(), 1);

    std::fs::write(dir.path().join("correlations.json"), b"{ not json").unwrap();
    assert!(load(&data).is_empty());
    // A write after corruption starts over from an empty map.
    set(&data, "arp/{G}", Some(pkg.clone())).unwrap();
    assert_eq!(load(&data).get("arp/{G}"), Some(&pkg));
}

#[test]
fn legacy_override_key_still_applies() {
    use wingetrs::installed::overrides::lookup;
    use wingetrs::installed::{Hive, InstallSource, InstalledApp};
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    let pkg = |id: &str| Choice::Package {
        source: "winget".into(),
        package_id: id.into(),
    };
    let app = InstalledApp {
        source: InstallSource::Arp(Hive::User),
        key: "X".into(),
        name: "X".into(),
        publisher: None,
        version: None,
        product_code: None,
        upgrade_code: None,
        family_name: None,
        scoop_bucket: None,
        stale: None,
    };
    std::fs::write(
        dir.path().join("correlations.json"),
        br#"{"arp/X":{"kind":"Package","source":"winget","package_id":"Old.Id"}}"#,
    )
    .unwrap();
    assert_eq!(lookup(&load(&data), &app), Some(&pkg("Old.Id")));
    set(&data, "arp/hkcu/X", Some(pkg("New.Id"))).unwrap();
    assert_eq!(lookup(&load(&data), &app), Some(&pkg("New.Id")));
}

#[test]
fn undo_removes_legacy_choice() {
    use wingetrs::installed::overrides::lookup;
    use wingetrs::installed::{Hive, InstallSource, InstalledApp};
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    let app = InstalledApp {
        source: InstallSource::Arp(Hive::User),
        key: "X".into(),
        name: "X".into(),
        publisher: None,
        version: None,
        product_code: None,
        upgrade_code: None,
        family_name: None,
        scoop_bucket: None,
        stale: None,
    };
    std::fs::write(
        dir.path().join("correlations.json"),
        br#"{"arp/X":{"kind":"Package","source":"winget","package_id":"Old.Id"}}"#,
    )
    .unwrap();
    set(&data, "arp/hkcu/X", None).unwrap();
    assert_eq!(lookup(&load(&data), &app), None);
}
