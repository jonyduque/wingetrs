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

#[test]
fn code_lookup_folds_case_in_both_directions() {
    let mut c = Catalog::open_in_memory().unwrap();
    let maps = WingetMaps {
        productcodes: vec![(
            "{23170F69-40C1-2701-1900-000001000000}".into(),
            "7zip.7zip".into(),
        )],
        upgradecodes: vec![(
            "{ABCDEF00-0000-0000-0000-000000000000}".into(),
            "7zip.7zip".into(),
        )],
        pfns: vec![("lower_pfn".into(), "7zip.7zip".into())],
        ..Default::default()
    };
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v",
        1,
        &[pkg("7zip.7zip", "7-Zip", "26.04")],
        &maps,
    )
    .unwrap();
    assert_eq!(
        c.lookup(
            MapTable::ProductCode,
            "{23170F69-40C1-2701-1900-000001000000}"
        )
        .unwrap(),
        ["7zip.7zip"]
    );
    assert_eq!(
        c.lookup(
            MapTable::UpgradeCode,
            "{abcdef00-0000-0000-0000-000000000000}"
        )
        .unwrap(),
        ["7zip.7zip"]
    );
    assert_eq!(c.lookup(MapTable::Pfn, "LOWER_PFN").unwrap(), ["7zip.7zip"]);
}

#[test]
fn open_file_uses_wal_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let _c = Catalog::open(&path).unwrap();
    let other = rusqlite::Connection::open(&path).unwrap();
    let mode: String = other
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn load_entries_does_not_need_raw_column() {
    // A package with a huge raw payload must load with the same entry and without raw.
    let mut c = Catalog::open_in_memory().unwrap();
    let mut p = pkg("big", "Big", "1");
    p.raw = Some("x".repeat(1_000_000));
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "e",
        1,
        &[p],
        &WingetMaps::default(),
    )
    .unwrap();
    let e = c.load_entries().unwrap();
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].id, "big");
    assert_eq!(
        c.get("scoop:main", "big")
            .unwrap()
            .unwrap()
            .raw
            .as_deref()
            .map(str::len),
        Some(1_000_000)
    );
}

#[test]
fn parallel_load_matches_sequential_order_on_sparse_rowids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut c = Catalog::open(&path).unwrap();
    let first: Vec<_> = (0..3000).map(|i| pkg(&format!("a{i}"), "A", "1")).collect();
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "e",
        1,
        &first,
        &WingetMaps::default(),
    )
    .unwrap();
    // Second replace deletes the first rows and inserts at higher, non-contiguous rowids.
    let second: Vec<_> = (0..3000).map(|i| pkg(&format!("b{i}"), "B", "1")).collect();
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "e",
        2,
        &second,
        &WingetMaps::default(),
    )
    .unwrap();
    c.replace_source(
        "scoop:extras",
        SourceKind::Scoop,
        "e",
        2,
        &first[..500],
        &WingetMaps::default(),
    )
    .unwrap();

    let loaded: Vec<String> = c
        .load_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    let other = rusqlite::Connection::open(&path).unwrap();
    let mut st = other
        .prepare("SELECT id FROM packages ORDER BY rowid")
        .unwrap();
    let expected: Vec<String> = st
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(expected.len(), 3500);
    assert_eq!(loaded, expected);
    let unique: std::collections::HashSet<_> = loaded.iter().collect();
    assert_eq!(unique.len(), 3500);
}

#[test]
fn load_entries_on_empty_disk_catalog_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let c = Catalog::open(&dir.path().join("catalog.db")).unwrap();
    assert!(c.load_entries().unwrap().is_empty());
}

#[test]
fn open_read_only_does_not_write_and_sees_data() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("c.db");
    let mut w = Catalog::open(&p).unwrap();
    w.replace_source(
        "winget",
        SourceKind::Winget,
        "v",
        1,
        &[pkg("A.A", "Alpha", "1")],
        &WingetMaps::default(),
    )
    .unwrap();
    drop(w);
    let r = Catalog::open_read_only(&p).unwrap();
    assert!(!r.is_empty().unwrap());
    assert_eq!(r.load_entries().unwrap().len(), 1);
    assert!(
        Catalog::open_read_only(&dir.path().join("missing.db")).is_err(),
        "must not create a file"
    );
    assert!(!dir.path().join("missing.db").exists());
}

fn pfn_map(k: &str, id: &str) -> WingetMaps {
    WingetMaps {
        pfns: vec![(k.into(), id.into())],
        ..Default::default()
    }
}

#[test]
fn replace_keeps_map_rows_of_another_source() {
    let pfn = pfn_map;
    let mut c = Catalog::open_in_memory().unwrap();
    c.replace_source(
        "a",
        SourceKind::Winget,
        "1",
        1,
        &[pkg("A.A", "Alpha", "1")],
        &pfn("PFN_A", "A.A"),
    )
    .unwrap();
    c.replace_source(
        "b",
        SourceKind::Winget,
        "1",
        1,
        &[pkg("B.B", "Beta", "1")],
        &pfn("PFN_B", "B.B"),
    )
    .unwrap();
    c.replace_source(
        "a",
        SourceKind::Winget,
        "2",
        2,
        &[pkg("A.A", "Alpha", "2")],
        &pfn("PFN_A2", "A.A"),
    )
    .unwrap();
    assert_eq!(c.lookup(MapTable::Pfn, "pfn_b").unwrap(), ["B.B"]);
    assert!(c.lookup(MapTable::Pfn, "pfn_a").unwrap().is_empty());
    assert_eq!(c.lookup(MapTable::Pfn, "pfn_a2").unwrap(), ["A.A"]);
}

#[test]
fn remove_source_keeps_other_sources_maps() {
    let pfn = pfn_map;
    let mut c = Catalog::open_in_memory().unwrap();
    c.replace_source(
        "a",
        SourceKind::Winget,
        "1",
        1,
        &[pkg("A.A", "Alpha", "1")],
        &pfn("PFN_A", "A.A"),
    )
    .unwrap();
    c.replace_source(
        "b",
        SourceKind::Winget,
        "1",
        1,
        &[pkg("B.B", "Beta", "1")],
        &pfn("PFN_B", "B.B"),
    )
    .unwrap();
    assert_eq!(c.remove_source("a").unwrap(), 1);
    assert_eq!(c.lookup(MapTable::Pfn, "pfn_b").unwrap(), ["B.B"]);
    assert!(c.lookup(MapTable::Pfn, "pfn_a").unwrap().is_empty());
}

#[test]
fn replace_waits_for_a_concurrent_writer_instead_of_failing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut a = Catalog::open(&path).unwrap();
    a.replace_source(
        "a",
        SourceKind::Winget,
        "1",
        1,
        &[pkg("A.A", "Alpha", "1")],
        &pfn_map("PFN_A", "A.A"),
    )
    .unwrap();
    let b = rusqlite::Connection::open(&path).unwrap();
    b.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        a.replace_source(
            "a",
            SourceKind::Winget,
            "2",
            2,
            &[pkg("A.A", "Alpha", "2")],
            &pfn_map("PFN_A2", "A.A"),
        )
    });
    started_rx.recv().unwrap();
    // Let the writer reach its first statement, then change the database under it.
    std::thread::sleep(std::time::Duration::from_millis(300));
    b.execute(
        "INSERT INTO meta (source, kind, source_version, fetched_at) VALUES ('x', 'winget', '1', 1)",
        [],
    )
    .unwrap();
    b.execute_batch("COMMIT").unwrap();
    worker.join().unwrap().unwrap();
    let r = Catalog::open(&path).unwrap();
    assert_eq!(r.lookup(MapTable::Pfn, "pfn_a2").unwrap(), ["A.A"]);
}

fn cached_fixture() -> Catalog {
    let mut c = Catalog::open_in_memory().unwrap();
    replace_cached(&mut c, "A.App");
    c
}

fn replace_cached(c: &mut Catalog, pc_a_target: &str) {
    let maps = WingetMaps {
        productcodes: vec![
            ("{PC-A}".into(), pc_a_target.into()),
            ("{PC-B}".into(), "B.App".into()),
        ],
        norm_names: vec![("aapp".into(), "A.App".into())],
        ..Default::default()
    };
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v",
        1,
        &[pkg("A.App", "A", "1"), pkg("B.App", "B", "1")],
        &maps,
    )
    .unwrap();
}

fn run_cached_queries(c: &Catalog) {
    assert_eq!(
        c.lookup(MapTable::ProductCode, "{pc-a}").unwrap(),
        ["A.App"]
    );
    assert_eq!(
        c.lookup(MapTable::ProductCode, "{PC-B}").unwrap(),
        ["B.App"]
    );
    assert_eq!(c.get("winget", "B.App").unwrap().unwrap().entry.id, "B.App");
    assert_eq!(c.lookup(MapTable::NormName, "aapp").unwrap(), ["A.App"]);
    assert_eq!(
        c.lookup(MapTable::ProductCode, "{PC-A}").unwrap(),
        ["A.App"]
    );
    let found = c.find_by_id("a.app").unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].entry.id, "A.App");
    assert_eq!(c.get("winget", "A.App").unwrap().unwrap().entry.id, "A.App");
}

#[test]
fn cached_statements_return_fresh_rows_for_each_key() {
    run_cached_queries(&cached_fixture());
}

#[test]
fn cached_statements_see_a_replaced_source() {
    let mut c = cached_fixture();
    run_cached_queries(&c);
    replace_cached(&mut c, "B.App");
    assert_eq!(
        c.lookup(MapTable::ProductCode, "{PC-A}").unwrap(),
        ["B.App"]
    );
    assert!(c.get("winget", "A.App").unwrap().is_some());
}

fn winget_with_productcode(c: &mut Catalog) {
    let maps = WingetMaps {
        productcodes: vec![("{PC-A}".into(), "A.App".into())],
        ..Default::default()
    };
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v",
        1,
        &[pkg("A.App", "App", "1")],
        &maps,
    )
    .unwrap();
}

#[test]
fn scoop_source_with_maps_is_refused() {
    let mut c = Catalog::open_in_memory().unwrap();
    let maps = WingetMaps {
        productcodes: vec![("{X}".into(), "x".into())],
        ..Default::default()
    };
    let e = c
        .replace_source(
            "scoop:main",
            SourceKind::Scoop,
            "v",
            1,
            &[pkg("x", "x", "1")],
            &maps,
        )
        .unwrap_err();
    assert!(
        matches!(e, wingetrs::catalog::CatalogError::MapsForScoop(_)),
        "{e:?}"
    );
    assert!(
        c.sources()
            .unwrap()
            .iter()
            .all(|s| s.source != "scoop:main")
    );
}

#[test]
fn removing_a_scoop_source_keeps_winget_maps() {
    let mut c = Catalog::open_in_memory().unwrap();
    winget_with_productcode(&mut c);
    let two = [pkg("s1", "s1", "1"), pkg("s2", "s2", "1")];
    let no = WingetMaps::default();
    c.replace_source("scoop:main", SourceKind::Scoop, "v", 1, &two, &no)
        .unwrap();
    assert_eq!(c.remove_source("scoop:main").unwrap(), 2);
    assert_eq!(
        c.lookup(MapTable::ProductCode, "{PC-A}").unwrap(),
        ["A.App"]
    );
    c.replace_source("scoop:main", SourceKind::Scoop, "v", 1, &two, &no)
        .unwrap();
    assert_eq!(
        c.lookup(MapTable::ProductCode, "{PC-A}").unwrap(),
        ["A.App"]
    );
}

#[test]
fn removing_a_winget_source_still_drops_its_maps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut c = Catalog::open(&path).unwrap();
    winget_with_productcode(&mut c);
    let two = [pkg("s1", "s1", "1"), pkg("s2", "s2", "1")];
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "v",
        1,
        &two,
        &WingetMaps::default(),
    )
    .unwrap();
    assert_eq!(c.remove_source("winget").unwrap(), 1);
    assert!(
        c.lookup(MapTable::ProductCode, "{PC-A}")
            .unwrap()
            .is_empty()
    );
    let raw = rusqlite::Connection::open(&path).unwrap();
    let n: i64 = raw
        .query_row("SELECT count(*) FROM map_productcode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

fn all_maps() -> WingetMaps {
    let pair = |k: &str| vec![(k.to_string(), "A.A".to_string())];
    WingetMaps {
        productcodes: pair("{PC-1}"),
        upgradecodes: pair("{UC-1}"),
        pfns: pair("PFN_A"),
        norm_names: pair("alpha"),
        norm_publishers: pair("acme"),
    }
}

fn schema_of(conn: &rusqlite::Connection) -> Vec<(String, String, String, Option<String>)> {
    let mut st = conn
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn index_names(conn: &rusqlite::Connection) -> Vec<String> {
    let mut st = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name LIKE 'ix_%' ORDER BY name")
        .unwrap();
    st.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn replace_with_maps_keeps_the_schema_of_a_fresh_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("c.db");
    let mut w = Catalog::open(&p).unwrap();
    let before = schema_of(&rusqlite::Connection::open(&p).unwrap());
    w.replace_source(
        "winget",
        SourceKind::Winget,
        "v",
        1,
        &[pkg("A.A", "Alpha", "1")],
        &all_maps(),
    )
    .unwrap();
    let raw = rusqlite::Connection::open(&p).unwrap();
    assert_eq!(before, schema_of(&raw));
    let mut st = raw
        .prepare("EXPLAIN QUERY PLAN SELECT package FROM map_productcode WHERE key = 'x'")
        .unwrap();
    let details: Vec<String> = st
        .query_map([], |r| r.get::<_, String>("detail"))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(details.iter().any(|d| d.contains("ix_pc")), "{details:?}");
}

#[test]
fn dangling_map_rollback_keeps_the_map_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("c.db");
    let mut w = Catalog::open(&p).unwrap();
    w.replace_source(
        "winget",
        SourceKind::Winget,
        "v1",
        1,
        &[pkg("A.A", "Alpha", "1")],
        &all_maps(),
    )
    .unwrap();
    let bad = WingetMaps {
        pfns: vec![("x".into(), "Missing.Id".into())],
        ..Default::default()
    };
    let err = w
        .replace_source(
            "winget",
            SourceKind::Winget,
            "v2",
            2,
            &[pkg("B.B", "Beta", "1")],
            &bad,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        wingetrs::catalog::CatalogError::DanglingMap(_)
    ));
    let raw = rusqlite::Connection::open(&p).unwrap();
    assert_eq!(
        index_names(&raw),
        ["ix_nn", "ix_np", "ix_pc", "ix_pfn", "ix_uc"]
    );
    assert_eq!(w.lookup(MapTable::Pfn, "pfn_a").unwrap(), ["A.A"]);
}

#[test]
fn reader_sees_new_rows_after_an_index_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("c.db");
    let mut w = Catalog::open(&p).unwrap();
    w.replace_source(
        "winget",
        SourceKind::Winget,
        "v1",
        1,
        &[pkg("A.A", "Alpha", "1")],
        &pfn_map("PFN_A", "A.A"),
    )
    .unwrap();
    let r = Catalog::open_read_only(&p).unwrap();
    assert_eq!(r.lookup(MapTable::Pfn, "pfn_a").unwrap(), ["A.A"]);
    w.replace_source(
        "winget",
        SourceKind::Winget,
        "v2",
        2,
        &[pkg("A.A", "Alpha", "2")],
        &pfn_map("PFN_B", "A.A"),
    )
    .unwrap();
    assert_eq!(r.lookup(MapTable::Pfn, "pfn_b").unwrap(), ["A.A"]);
    assert!(r.lookup(MapTable::Pfn, "pfn_a").unwrap().is_empty());
}
