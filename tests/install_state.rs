use wingetrs::install::layout::Layout;
use wingetrs::install::state::*;
use wingetrs::installed::InstallSource;

fn rec(app: &str, version: &str) -> AppRecord {
    AppRecord {
        app: app.into(),
        bucket: "main".into(),
        version: version.into(),
        arch: Some("64bit".into()),
        installed_at: 1700000000,
        manifest_sha256: "ab".repeat(32),
    }
}
fn art(kind: ArtifactKind, target: &str, previous: Option<&str>) -> Artifact {
    Artifact {
        kind,
        target: target.into(),
        previous: previous.map(str::to_string),
    }
}

#[test]
fn open_creates_schema_in_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("sub").join("state.db");
    let db = StateDb::open(&p).unwrap();
    assert!(p.exists());
    assert!(db.apps().unwrap().is_empty());
}

#[test]
fn record_get_remove_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = StateDb::open(&dir.path().join("state.db")).unwrap();
    let arts = vec![
        art(ArtifactKind::Shim, "shims\\a.exe", None),
        art(ArtifactKind::Env, "FOO", Some("old")),
    ];
    db.record(&rec("a", "1.0"), &arts).unwrap();
    let (r, got) = db.get("a").unwrap().unwrap();
    assert_eq!(r, rec("a", "1.0"));
    assert_eq!(got, arts);
    assert_eq!(db.apps().unwrap().len(), 1);
    db.remove("a").unwrap();
    assert!(db.get("a").unwrap().is_none());
    assert!(db.apps().unwrap().is_empty());
}

#[test]
fn rerecord_replaces_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = StateDb::open(&dir.path().join("state.db")).unwrap();
    db.record(&rec("a", "1.0"), &[art(ArtifactKind::Shim, "x", None)])
        .unwrap();
    db.record(&rec("a", "2.0"), &[art(ArtifactKind::Shim, "y", None)])
        .unwrap();
    let (r, got) = db.get("a").unwrap().unwrap();
    assert_eq!(r.version, "2.0");
    assert_eq!(got, vec![art(ArtifactKind::Shim, "y", None)]);
}

#[test]
fn path_users_excludes_self_and_other_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = StateDb::open(&dir.path().join("state.db")).unwrap();
    db.record(&rec("a", "1"), &[art(ArtifactKind::Path, "C:\\bin", None)])
        .unwrap();
    db.record(&rec("b", "1"), &[art(ArtifactKind::Path, "C:\\bin", None)])
        .unwrap();
    db.record(&rec("c", "1"), &[art(ArtifactKind::Env, "C:\\bin", None)])
        .unwrap();
    assert_eq!(
        db.path_users("a", "C:\\bin").unwrap(),
        vec!["b".to_string()]
    );
    assert!(db.path_users("a", "C:\\other").unwrap().is_empty());
}

#[test]
fn path_users_uses_path_equivalence() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = StateDb::open(&dir.path().join("state.db")).unwrap();
    db.record(&rec("A", "1"), &[art(ArtifactKind::Path, r"C:\Bin\", None)])
        .unwrap();
    assert_eq!(
        db.path_users("B", r"c:\bin").unwrap(),
        vec!["A".to_string()]
    );
}

#[test]
fn open_garbage_file_is_err_and_uses_wal() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.db");
    std::fs::write(&bad, vec![0x41u8; 4096]).unwrap();
    assert!(StateDb::open(&bad).is_err());
    let good = dir.path().join("good.db");
    drop(StateDb::open(&good).unwrap());
    let c = rusqlite::Connection::open(&good).unwrap();
    let m: String = c
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(m, "wal");
}

#[test]
fn install_json_has_scoop_fields() {
    let dir = tempfile::tempdir().unwrap();
    write_install_json(dir.path(), "main", Some("64bit"), "1.2").unwrap();
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("install.json")).unwrap()).unwrap();
    assert_eq!(v["bucket"], "main");
    assert_eq!(v["architecture"], "64bit");
    assert_eq!(v["version"], "1.2");
    assert_eq!(v["wingetrs"], 1);
}

#[cfg(windows)]
#[test]
fn scan_lists_valid_and_broken_apps() {
    let root = tempfile::tempdir().unwrap();
    let layout = Layout::new(root.path());
    let good = layout.version_dir("good", "1.0");
    std::fs::create_dir_all(&good).unwrap();
    write_install_json(&good, "extras", Some("64bit"), "1.0").unwrap();
    junction::create(&good, layout.current("good")).unwrap();

    let gone = layout.version_dir("gone", "1.0");
    std::fs::create_dir_all(&gone).unwrap();
    write_install_json(&gone, "main", None, "1.0").unwrap();
    junction::create(&gone, layout.current("gone")).unwrap();
    std::fs::remove_dir_all(&gone).unwrap();

    std::fs::write(layout.apps().join("stray.txt"), "x").unwrap();
    let apps = scan_wingetrs(&layout);
    assert_eq!(apps.len(), 2);
    assert_eq!(apps[0].key, "gone");
    assert_eq!(apps[0].version, None);
    assert_eq!(apps[0].source, InstallSource::Wingetrs);
    assert_eq!(apps[1].key, "good");
    assert_eq!(apps[1].version.as_deref(), Some("1.0"));
    assert_eq!(apps[1].scoop_bucket.as_deref(), Some("extras"));
}

#[allow(clippy::unwrap_used)]
fn shared_path_db(dir: &std::path::Path) -> StateDb {
    let mut db = StateDb::open(&dir.join("state.db")).unwrap();
    db.record(
        &rec("a", "1"),
        &[
            art(ArtifactKind::Path, r"C:\R\shims", None),
            art(ArtifactKind::Path, r"C:\R\apps\a\current\bin", None),
            art(ArtifactKind::Shim, "rg", None),
        ],
    )
    .unwrap();
    db.record(
        &rec("b", "1"),
        &[art(ArtifactKind::Path, r"c:\r\shims\", None)],
    )
    .unwrap();
    db.record(
        &rec("c", "1"),
        &[art(ArtifactKind::Env, r"C:\R\shims", None)],
    )
    .unwrap();
    db
}

#[test]
fn uninstall_plan_keeps_a_shared_path_entry_and_undoes_its_own() {
    use wingetrs::install::plan::UninstallPlan;
    let dir = tempfile::tempdir().unwrap();
    let db = shared_path_db(dir.path());
    let plan = UninstallPlan::from_state(&db, "a", false).unwrap().unwrap();
    assert_eq!(
        plan.path_users,
        vec![(r"C:\R\shims".to_string(), vec!["b".to_string()])]
    );
    assert_eq!(
        plan.artifact_lines(),
        vec![
            r"Manter entrada no PATH: C:\R\shims (ainda usada por: b)".to_string(),
            r"Desfazer entrada no PATH: C:\R\apps\a\current\bin".to_string(),
            "Desfazer atalho de linha de comando: rg".to_string(),
        ]
    );
}

#[test]
fn uninstall_plan_lists_every_other_user_of_the_entry() {
    use wingetrs::install::plan::UninstallPlan;
    let dir = tempfile::tempdir().unwrap();
    let mut db = StateDb::open(&dir.path().join("state.db")).unwrap();
    for app in ["a", "b", "c"] {
        db.record(
            &rec(app, "1"),
            &[art(ArtifactKind::Path, r"C:\R\shims", None)],
        )
        .unwrap();
    }
    let plan = UninstallPlan::from_state(&db, "a", false).unwrap().unwrap();
    assert!(
        plan.artifact_lines()[0].ends_with("(ainda usada por: b, c)"),
        "{:?}",
        plan.artifact_lines()
    );
}

#[test]
fn uninstall_plan_text_is_unchanged_without_shared_entries() {
    use wingetrs::install::plan::UninstallPlan;
    let dir = tempfile::tempdir().unwrap();
    let mut db = StateDb::open(&dir.path().join("state.db")).unwrap();
    db.record(
        &rec("a", "1"),
        &[
            art(ArtifactKind::Path, r"C:\R\shims", None),
            art(ArtifactKind::Persist, "conf", None),
        ],
    )
    .unwrap();
    let plan = UninstallPlan::from_state(&db, "a", false).unwrap().unwrap();
    assert_eq!(
        wingetrs::cli::output::uninstall_plan_text(&plan, &Layout::new(r"C:\R")),
        "Desinstalar a 1\n  1. Apagar C:\\R\\apps\\a\n  2. Desfazer entrada no PATH: C:\\R\\shims\n  3. Manter conf (dados persistentes; use --purge)\n"
    );
}

#[test]
fn uninstall_plan_text_says_kept_when_shared() {
    use wingetrs::install::plan::UninstallPlan;
    let dir = tempfile::tempdir().unwrap();
    let db = shared_path_db(dir.path());
    let plan = UninstallPlan::from_state(&db, "a", false).unwrap().unwrap();
    let text = wingetrs::cli::output::uninstall_plan_text(&plan, &Layout::new(r"C:\R"));
    assert!(text.contains(r"Manter entrada no PATH: C:\R\shims (ainda usada por: b)"));
    assert!(!text.contains(r"Desfazer entrada no PATH: C:\R\shims"));
}

#[test]
fn uninstall_plan_only_path_artifacts_consult_other_users() {
    use wingetrs::install::plan::UninstallPlan;
    let dir = tempfile::tempdir().unwrap();
    let mut db = StateDb::open(&dir.path().join("state.db")).unwrap();
    db.record(
        &rec("a", "1"),
        &[art(ArtifactKind::Shim, r"C:\R\shims", None)],
    )
    .unwrap();
    db.record(
        &rec("b", "1"),
        &[art(ArtifactKind::Path, r"C:\R\shims", None)],
    )
    .unwrap();
    let plan = UninstallPlan::from_state(&db, "a", false).unwrap().unwrap();
    assert!(plan.path_users.is_empty(), "{:?}", plan.path_users);
}

#[test]
fn uninstall_plan_purge_line_is_pinned() {
    use wingetrs::install::plan::UninstallPlan;
    let dir = tempfile::tempdir().unwrap();
    let mut db = StateDb::open(&dir.path().join("state.db")).unwrap();
    db.record(&rec("a", "1"), &[art(ArtifactKind::Persist, "conf", None)])
        .unwrap();
    let purge = UninstallPlan::from_state(&db, "a", true).unwrap().unwrap();
    assert_eq!(
        purge.artifact_lines(),
        vec!["Apagar conf (dados persistentes)".to_string()]
    );
    let keep = UninstallPlan::from_state(&db, "a", false).unwrap().unwrap();
    assert_eq!(
        keep.artifact_lines(),
        vec!["Manter conf (dados persistentes; use --purge)".to_string()]
    );
}
