#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::{Path, PathBuf};
use wingetrs::install::layout::Layout;
use wingetrs::install::path_edit::{path_add, path_contains, path_remove};
use wingetrs::install::snapshot::snapshot;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{EnvStore, EnvValue, RegistryScan, Shortcuts, Trash};

const ENTRY: &str = r"C:\Users\u\AppData\Local\wingetrs\shims";

#[test]
fn path_add_appends_once_and_keeps_everything_else() {
    let user = r"%USERPROFILE%\bin;;C:\Tools\;%JAVA_HOME%\bin";
    let out = path_add(user, ENTRY);
    assert_eq!(out, format!(r"{user};{ENTRY}"));
    assert_eq!(path_add(&out, ENTRY), out, "idempotent");
    assert_eq!(
        path_add(&out, &ENTRY.to_uppercase()),
        out,
        "case-insensitive"
    );
    assert_eq!(
        path_add(&out, &format!(r"{ENTRY}\")),
        out,
        "trailing backslash"
    );
    assert_eq!(path_add("", ENTRY), ENTRY);
    assert_eq!(
        path_add("A;", ENTRY),
        format!("A;{ENTRY}"),
        "no doubled separator"
    );
}

#[test]
fn path_remove_only_removes_equivalent_entries() {
    let user = format!(
        r"%USERPROFILE%\bin;;{ENTRY};C:\Tools\;{}\;%JAVA_HOME%\bin",
        ENTRY.to_lowercase()
    );
    let out = path_remove(&user, ENTRY);
    assert_eq!(out, r"%USERPROFILE%\bin;;C:\Tools\;%JAVA_HOME%\bin");
    assert!(!path_contains(&out, ENTRY));
    assert_eq!(path_remove(&out, ENTRY), out);
    assert_eq!(path_remove(ENTRY, ENTRY), "");
}

#[test]
fn snapshot_detects_content_and_structure_changes() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("a/b")).unwrap();
    std::fs::write(d.path().join("a/b/f.txt"), b"1").unwrap();
    let s1 = snapshot(d.path()).unwrap();
    assert_eq!(s1, snapshot(d.path()).unwrap());
    std::fs::write(d.path().join("a/b/f.txt"), b"2").unwrap();
    assert_ne!(s1, snapshot(d.path()).unwrap());
    std::fs::write(d.path().join("a/b/f.txt"), b"1").unwrap();
    std::fs::create_dir(d.path().join("empty")).unwrap();
    assert_ne!(s1, snapshot(d.path()).unwrap(), "empty dirs count");
}

#[test]
fn layout_builds_contract_paths() {
    let l = Layout::new("R");
    let r = Path::new("R");
    assert_eq!(l.root(), r);
    assert_eq!(l.apps(), r.join("apps"));
    assert_eq!(l.app_dir("a"), r.join("apps").join("a"));
    assert_eq!(l.version_dir("a", "1"), r.join("apps").join("a").join("1"));
    assert_eq!(
        l.staging_dir("a", "1"),
        r.join("apps").join("a").join(".staging-1")
    );
    assert_eq!(l.current("a"), r.join("apps").join("a").join("current"));
    assert_eq!(l.persist_dir("a"), r.join("persist").join("a"));
    assert_eq!(l.shims(), r.join("shims"));
    assert_eq!(
        l.launcher(),
        r.join("shims").join(".launcher").join("shim.exe")
    );
    assert_eq!(l.backups(), r.join("backups"));
    assert_eq!(l.state_db(), r.join("state.db"));
    assert_eq!(l.install_lock(), r.join("install.lock"));
}

#[test]
fn fakes_dump_changes_and_reverts() {
    let env = FakeEnv::new();
    let d0 = env.dump();
    let v = EnvValue {
        value: "x".into(),
        expand: true,
    };
    env.set("A", &v).unwrap();
    assert_ne!(env.dump(), d0);
    assert_eq!(env.get("A").unwrap(), Some(v));
    env.remove("A").unwrap();
    assert_eq!(env.dump(), d0);
    env.broadcast();
    assert_ne!(env.dump(), d0);

    let sc = FakeShortcuts::new("S");
    let d0 = sc.dump();
    let link = PathBuf::from("S/a.lnk");
    sc.create(&link, Path::new("t.exe"), Some("-x"), None)
        .unwrap();
    assert_ne!(sc.dump(), d0);
    sc.remove(&link).unwrap();
    assert_eq!(sc.dump(), d0);
    assert_eq!(sc.root(), PathBuf::from("S"));

    let tr = FakeTrash::new();
    let d0 = tr.dump();
    tr.recycle(Path::new("x")).unwrap();
    assert_ne!(tr.dump(), d0);

    let rg = FakeRegistry::new();
    let d0 = rg.dump();
    rg.insert(r"Software\Foo\Bar", Some("v"), r"C:\Wingetrs\apps\a");
    assert_ne!(rg.dump(), d0);
    assert_eq!(
        rg.values_containing(&[r"Software\Foo"], "wingetrs")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        rg.subkeys_named(r"Software\Foo", &["bar"]).unwrap(),
        vec![r"Software\Foo\Bar"]
    );
    rg.delete_value(r"Software\Foo\Bar", "v").unwrap();
    rg.delete_key(r"Software\Foo\Bar").unwrap();
    assert_eq!(rg.dump(), d0);
}

#[cfg(windows)]
#[test]
fn snapshot_treats_junction_as_a_link_not_a_directory() {
    let ext = tempfile::tempdir().unwrap();
    std::fs::write(ext.path().join("f.txt"), b"1").unwrap();
    let ext2 = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let j = root.path().join("j");
    junction::create(ext.path(), &j).unwrap();
    let s1 = snapshot(root.path()).unwrap();
    assert!(s1.get("j").unwrap().starts_with("link:"), "{s1:?}");
    assert!(s1.keys().all(|k| !k.starts_with("j/")), "{s1:?}");
    std::fs::write(ext.path().join("f.txt"), b"changed").unwrap();
    assert_eq!(
        s1,
        snapshot(root.path()).unwrap(),
        "target content is not followed"
    );
    junction::delete(&j).unwrap();
    std::fs::remove_dir(&j).unwrap();
    junction::create(ext2.path(), &j).unwrap();
    assert_ne!(
        s1,
        snapshot(root.path()).unwrap(),
        "retargeting changes the snapshot"
    );
}
