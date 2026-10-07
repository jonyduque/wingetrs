#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::catalog::{Catalog, MapTable, NewPackage, WingetMaps};
use wingetrs::core::SourceKind;

fn pkg(id: &str, name: &str, version: &str) -> NewPackage {
    NewPackage {
        id: id.into(),
        name: name.into(),
        version: version.into(),
        ..Default::default()
    }
}

#[test]
fn replace_source_only_touches_that_source() {
    let mut c = Catalog::open_in_memory().unwrap();
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v1",
        1,
        &[pkg("A.A", "Alpha", "1")],
        &WingetMaps::default(),
    )
    .unwrap();
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "e1",
        1,
        &[pkg("alpha", "alpha", "2")],
        &WingetMaps::default(),
    )
    .unwrap();
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v2",
        2,
        &[pkg("B.B", "Beta", "1")],
        &WingetMaps::default(),
    )
    .unwrap();
    let mut ids: Vec<String> = c
        .load_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    ids.sort();
    assert_eq!(ids, ["B.B", "alpha"]);
    let metas = c.sources().unwrap();
    assert_eq!(
        metas
            .iter()
            .find(|m| m.source == "winget")
            .unwrap()
            .source_version,
        "v2"
    );
}

#[test]
fn map_keys_are_case_insensitive_for_codes_and_exact_for_norm() {
    let mut c = Catalog::open_in_memory().unwrap();
    let maps = WingetMaps {
        productcodes: vec![(
            "{23170F69-40C1-2701-1900-000001000000}".into(),
            "7zip.7zip".into(),
        )],
        norm_names: vec![("mozillafirefox(X64)".into(), "Mozilla.Firefox".into())],
        ..Default::default()
    };
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v",
        1,
        &[
            pkg("7zip.7zip", "7-Zip", "26.04"),
            pkg("Mozilla.Firefox", "Firefox", "157"),
        ],
        &maps,
    )
    .unwrap();
    assert_eq!(
        c.lookup(
            MapTable::ProductCode,
            "{23170f69-40c1-2701-1900-000001000000}"
        )
        .unwrap(),
        ["7zip.7zip"]
    );
    assert_eq!(
        c.lookup(MapTable::NormName, "mozillafirefox(X64)").unwrap(),
        ["Mozilla.Firefox"]
    );
    assert!(
        c.lookup(MapTable::NormName, "mozillafirefox(x64)")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn dangling_map_rolls_back_whole_import() {
    let mut c = Catalog::open_in_memory().unwrap();
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v1",
        1,
        &[pkg("A.A", "Alpha", "1")],
        &WingetMaps::default(),
    )
    .unwrap();
    let bad = WingetMaps {
        pfns: vec![("x_1".into(), "Missing.Id".into())],
        ..Default::default()
    };
    assert!(
        c.replace_source(
            "winget",
            SourceKind::Winget,
            "v2",
            2,
            &[pkg("B.B", "Beta", "1")],
            &bad
        )
        .is_err()
    );
    let ids: Vec<String> = c
        .load_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(
        ids,
        ["A.A"],
        "failed import must leave previous data intact"
    );
}

#[test]
fn reader_keeps_old_snapshot_while_writer_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut w = Catalog::open(&path).unwrap();
    w.replace_source(
        "winget",
        SourceKind::Winget,
        "v1",
        1,
        &[pkg("A.A", "Alpha", "1")],
        &WingetMaps::default(),
    )
    .unwrap();
    let r = Catalog::open(&path).unwrap();
    assert_eq!(r.load_entries().unwrap().len(), 1);
    let many: Vec<NewPackage> = (0..500)
        .map(|i| pkg(&format!("P.{i}"), &format!("P{i}"), "1"))
        .collect();
    w.replace_source(
        "winget",
        SourceKind::Winget,
        "v2",
        2,
        &many,
        &WingetMaps::default(),
    )
    .unwrap();
    assert_eq!(
        r.load_entries().unwrap().len(),
        500,
        "reader sees the committed state after commit"
    );
}

#[test]
fn tags_and_commands_round_trip() {
    let mut c = Catalog::open_in_memory().unwrap();
    let mut p = pkg("x", "x", "1");
    p.tags = vec!["a".into(), "b-c".into()];
    p.commands = vec!["rg".into()];
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "e",
        1,
        &[p],
        &WingetMaps::default(),
    )
    .unwrap();
    let e = &c.load_entries().unwrap()[0];
    assert_eq!(e.tags, ["a", "b-c"]);
    assert_eq!(e.commands, ["rg"]);
    assert_eq!(e.source_name, "scoop:main");
    assert_eq!(e.source, SourceKind::Scoop);
}
