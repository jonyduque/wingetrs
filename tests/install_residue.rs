#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use wingetrs::install::residue::{Confidence, Residue, ResidueInput, ResidueKind, classify, clean};
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{EffectError, Effects, RegHit, RegistryScan};

const REMOVED: &str = r"C:\Users\u\AppData\Local\wingetrs\apps\ripgrep";

fn hit(key: &str, name: Option<&str>, data: &str) -> RegHit {
    RegHit {
        key: key.into(),
        value_name: name.map(str::to_string),
        data: data.into(),
    }
}

struct In {
    app: &'static str,
    homepage: Option<&'static str>,
    hits: Vec<RegHit>,
    shortcuts: Vec<(PathBuf, PathBuf)>,
    folders: Vec<PathBuf>,
    keys: Vec<String>,
    other: bool,
}
impl In {
    fn new() -> In {
        In {
            app: "ripgrep",
            homepage: None,
            hits: vec![],
            shortcuts: vec![],
            folders: vec![],
            keys: vec![],
            other: false,
        }
    }
    fn run(&self) -> Vec<Residue> {
        classify(&ResidueInput {
            app: self.app,
            removed_dir: Path::new(REMOVED),
            homepage: self.homepage,
            reg_hits: &self.hits,
            shortcuts: &self.shortcuts,
            folders: &self.folders,
            software_keys: &self.keys,
            other_installs: self.other,
        })
    }
}

fn find<'a>(rs: &'a [Residue], kind: &ResidueKind) -> Option<&'a Residue> {
    rs.iter().find(|r| &r.kind == kind)
}

// --- classify: high confidence ---

#[test]
fn hit_pointing_into_removed_dir_is_high_and_preselected() {
    let mut i = In::new();
    i.hits = vec![hit(
        r"Software\Classes\rgfile\shell\open\command",
        None,
        r#""C:\Users\u\AppData\Local\wingetrs\apps\ripgrep\current\rg.exe" "%1""#,
    )];
    let rs = i.run();
    assert_eq!(rs.len(), 1);
    assert_eq!(
        rs[0],
        Residue {
            kind: ResidueKind::RegValue {
                key: r"Software\Classes\rgfile\shell\open\command".into(),
                value_name: None,
            },
            confidence: Confidence::High,
            preselected: true,
            note: None,
        }
    );
}

#[test]
fn hit_matching_removed_dir_case_insensitively_and_with_slashes_is_high() {
    let mut i = In::new();
    i.hits = vec![hit(
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        Some("rg"),
        "c:/users/U/appdata/local/WINGETRS/apps/RipGrep/current/rg.exe",
    )];
    let rs = i.run();
    assert_eq!(rs.len(), 1);
    assert_eq!(rs[0].confidence, Confidence::High);
    assert!(rs[0].preselected);
}

#[test]
fn hit_that_only_mentions_app_name_is_not_high() {
    // Data names the app but does not point into the removed folder.
    let mut i = In::new();
    i.hits = vec![
        hit(r"Software\Classes\rgfile", None, r"C:\tools\ripgrep\rg.exe"),
        hit(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            Some("rg"),
            "ripgrep",
        ),
    ];
    let rs = i.run();
    assert!(
        rs.iter().all(|r| r.confidence != Confidence::High),
        "{rs:?}"
    );
    assert!(rs.iter().all(|r| !r.preselected), "{rs:?}");
}

#[test]
fn hit_in_sibling_app_dir_with_same_prefix_is_not_high() {
    // apps\ripgrep-all is a different app than apps\ripgrep.
    let mut i = In::new();
    i.hits = vec![hit(
        r"Software\Classes\x",
        None,
        r"C:\Users\u\AppData\Local\wingetrs\apps\ripgrep-all\current\rga.exe",
    )];
    assert!(i.run().is_empty());
}

#[test]
fn shortcut_whose_target_is_in_removed_dir_is_high_and_preselected() {
    let mut i = In::new();
    let lnk = PathBuf::from(r"C:\Users\u\Desktop\rg.lnk");
    let other = PathBuf::from(r"C:\Users\u\Desktop\notepad.lnk");
    i.shortcuts = vec![
        (
            lnk.clone(),
            PathBuf::from(format!(r"{REMOVED}\current\rg.exe")),
        ),
        (other.clone(), PathBuf::from(r"C:\Windows\notepad.exe")),
    ];
    let rs = i.run();
    assert_eq!(rs.len(), 1, "{rs:?}");
    assert_eq!(
        rs[0],
        Residue {
            kind: ResidueKind::Shortcut { path: lnk },
            confidence: Confidence::High,
            preselected: true,
            note: None,
        }
    );
}

#[test]
fn shortcut_to_sibling_app_dir_is_ignored() {
    let mut i = In::new();
    i.shortcuts = vec![(
        PathBuf::from(r"C:\Users\u\Desktop\rga.lnk"),
        PathBuf::from(r"C:\Users\u\AppData\Local\wingetrs\apps\ripgrep-all\rga.exe"),
    )];
    assert!(i.run().is_empty());
}

// --- classify: low confidence ---

#[test]
fn folder_named_like_app_is_low_and_unselected() {
    let mut i = In::new();
    i.folders = vec![
        PathBuf::from(r"C:\Users\u\AppData\Roaming\ripgrep"),
        PathBuf::from(r"C:\Users\u\AppData\Local\RIPGREP"),
        PathBuf::from(r"C:\Users\u\AppData\Roaming\unrelated"),
    ];
    let rs = i.run();
    assert_eq!(rs.len(), 2, "{rs:?}");
    for r in &rs {
        assert_eq!(r.confidence, Confidence::Low);
        assert!(!r.preselected);
        assert_eq!(r.note, None);
    }
    assert!(
        find(
            &rs,
            &ResidueKind::Folder {
                path: PathBuf::from(r"C:\Users\u\AppData\Roaming\ripgrep")
            }
        )
        .is_some()
    );
}

#[test]
fn github_homepage_owner_names_low_confidence_key_and_folder() {
    let mut i = In::new();
    i.homepage = Some("https://github.com/BurntSushi/ripgrep");
    i.keys = vec![
        r"Software\BurntSushi".into(),
        r"Software\Microsoft".into(),
        r"Software\ripgrep".into(),
    ];
    i.folders = vec![PathBuf::from(r"C:\Users\u\AppData\Roaming\burntsushi")];
    let rs = i.run();
    assert_eq!(rs.len(), 3, "{rs:?}");
    let k = find(
        &rs,
        &ResidueKind::RegKey {
            key: r"Software\BurntSushi".into(),
        },
    )
    .expect("owner key");
    assert_eq!(k.confidence, Confidence::Low);
    assert!(!k.preselected);
    assert!(
        find(
            &rs,
            &ResidueKind::RegKey {
                key: r"Software\ripgrep".into()
            }
        )
        .is_some()
    );
    assert!(
        find(
            &rs,
            &ResidueKind::Folder {
                path: PathBuf::from(r"C:\Users\u\AppData\Roaming\burntsushi")
            }
        )
        .is_some()
    );
}

#[test]
fn non_github_homepage_gives_no_owner_name() {
    let mut i = In::new();
    i.homepage = Some("https://gitlab.com/BurntSushi/ripgrep");
    i.keys = vec![r"Software\BurntSushi".into()];
    assert!(i.run().is_empty());
}

#[test]
fn github_homepage_without_repo_path_gives_no_owner_name() {
    let mut i = In::new();
    i.homepage = Some("https://github.com/");
    i.keys = vec![r"Software\github.com".into(), r"Software\".into()];
    assert!(i.run().is_empty());
}

// --- classify: other installs ---

#[test]
fn other_installs_notes_low_items_and_never_preselects_them() {
    let mut i = In::new();
    i.other = true;
    i.homepage = Some("https://github.com/BurntSushi/ripgrep");
    i.folders = vec![PathBuf::from(r"C:\Users\u\AppData\Roaming\ripgrep")];
    i.keys = vec![r"Software\BurntSushi".into()];
    i.hits = vec![hit(
        r"Software\Microsoft\Windows\CurrentVersion\App Paths\rg.exe",
        None,
        &format!(r"{REMOVED}\current\rg.exe"),
    )];
    let rs = i.run();
    assert_eq!(rs.len(), 3, "{rs:?}");
    for r in &rs {
        match r.confidence {
            Confidence::Low => {
                assert!(!r.preselected, "{r:?}");
                assert_eq!(r.note.as_deref(), Some("pode ser de outra instalação"));
            }
            Confidence::High => {
                assert!(r.preselected, "{r:?}");
                assert_eq!(r.note, None);
            }
        }
    }
}

#[test]
fn without_other_installs_low_items_have_no_note() {
    let mut i = In::new();
    i.folders = vec![PathBuf::from(r"C:\Users\u\AppData\Roaming\ripgrep")];
    let rs = i.run();
    assert_eq!(rs[0].note, None);
    assert!(!rs[0].preselected);
}

#[test]
fn duplicates_are_reported_once() {
    let mut i = In::new();
    let h = hit(r"Software\Classes\x", None, &format!(r"{REMOVED}\rg.exe"));
    i.hits = vec![h.clone(), h];
    i.folders = vec![
        PathBuf::from(r"C:\A\ripgrep"),
        PathBuf::from(r"C:\A\ripgrep"),
    ];
    assert_eq!(i.run().len(), 2);
}

// --- clean ---

/// Wraps `FakeRegistry`, logging operation order and recording whether the key still
/// existed in the fake at export time; `fail_export` makes exports of keys containing it fail.
struct OrderedRegistry {
    inner: FakeRegistry,
    log: Mutex<Vec<String>>,
    fail_export: Option<&'static str>,
}
impl OrderedRegistry {
    fn new(fail_export: Option<&'static str>) -> OrderedRegistry {
        OrderedRegistry {
            inner: FakeRegistry::new(),
            log: Mutex::new(vec![]),
            fail_export,
        }
    }
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}
impl RegistryScan for OrderedRegistry {
    fn values_containing(&self, roots: &[&str], needle: &str) -> Result<Vec<RegHit>, EffectError> {
        self.inner.values_containing(roots, needle)
    }
    fn subkeys_named(&self, parent: &str, names: &[&str]) -> Result<Vec<String>, EffectError> {
        self.inner.subkeys_named(parent, names)
    }
    fn export(&self, key: &str, file: &Path) -> Result<(), EffectError> {
        let present = self.inner.dump().0.contains_key(key);
        self.log
            .lock()
            .unwrap()
            .push(format!("export {key} present={present}"));
        if self.fail_export.is_some_and(|f| key.contains(f)) {
            return Err(EffectError("acesso negado".into()));
        }
        self.inner.export(key, file)
    }
    fn delete_value(&self, key: &str, value_name: &str) -> Result<(), EffectError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("delete_value {key} {value_name}"));
        self.inner.delete_value(key, value_name)
    }
    fn delete_key(&self, key: &str) -> Result<(), EffectError> {
        self.log.lock().unwrap().push(format!("delete_key {key}"));
        self.inner.delete_key(key)
    }
}

fn sel(kind: ResidueKind) -> Residue {
    Residue {
        kind,
        confidence: Confidence::High,
        preselected: true,
        note: None,
    }
}

#[test]
fn clean_exports_before_deleting_key_and_value() {
    let env = FakeEnv::new();
    let sc = FakeShortcuts::new(r"C:\SM");
    let tr = FakeTrash::new();
    let rg = OrderedRegistry::new(None);
    rg.inner.insert(r"Software\BurntSushi", Some("x"), "1");
    rg.inner.insert(r"Software\BurntSushi\sub", None, "2");
    rg.inner.insert(r"Software\Run", Some("rg"), "data");
    rg.inner.insert(r"Software\Run", Some("keep"), "other");
    let fx = Effects {
        env: &env,
        shortcuts: &sc,
        trash: &tr,
        registry: &rg,
    };
    let backups = tempfile::tempdir().unwrap();
    let res = clean(
        &[
            sel(ResidueKind::RegKey {
                key: r"Software\BurntSushi".into(),
            }),
            sel(ResidueKind::RegValue {
                key: r"Software\Run".into(),
                value_name: Some("rg".into()),
            }),
        ],
        "ripgrep",
        &fx,
        backups.path(),
    );
    assert_eq!(res, vec![Ok(()), Ok(())]);
    assert_eq!(
        rg.log(),
        vec![
            r"export Software\BurntSushi present=true".to_string(),
            r"delete_key Software\BurntSushi".to_string(),
            r"export Software\Run present=true".to_string(),
            r"delete_value Software\Run rg".to_string(),
        ]
    );
    let (state, exports) = rg.inner.dump();
    assert!(!state.contains_key(r"Software\BurntSushi"));
    assert!(!state.contains_key(r"Software\BurntSushi\sub"));
    let run = &state[r"Software\Run"];
    assert!(!run.contains_key(&Some("rg".to_string())));
    assert!(run.contains_key(&Some("keep".to_string())));
    // backups\<YYYYMMDD-HHMMSS>\<app>.reg, with a distinct file per exported key.
    assert_eq!(exports.len(), 2);
    let mut files = vec![];
    for (_, f) in &exports {
        let stamp = f.parent().unwrap();
        assert_eq!(stamp.parent().unwrap(), backups.path());
        let s = stamp.file_name().unwrap().to_str().unwrap();
        assert_eq!(s.len(), 15, "{s}");
        assert_eq!(&s[8..9], "-");
        assert!(
            s[..8]
                .bytes()
                .chain(s[9..].bytes())
                .all(|b| b.is_ascii_digit())
        );
        assert!(stamp.is_dir(), "backup dir must exist before export");
        files.push(f.file_name().unwrap().to_str().unwrap().to_string());
    }
    assert_eq!(files[0], "ripgrep.reg");
    assert_ne!(files[0], files[1]);
    assert!(files[1].starts_with("ripgrep") && files[1].ends_with(".reg"));
}

#[test]
fn clean_default_value_uses_empty_value_name() {
    let env = FakeEnv::new();
    let sc = FakeShortcuts::new(r"C:\SM");
    let tr = FakeTrash::new();
    let rg = OrderedRegistry::new(None);
    rg.inner.insert(r"Software\Classes\rgfile", None, "x");
    let fx = Effects {
        env: &env,
        shortcuts: &sc,
        trash: &tr,
        registry: &rg,
    };
    let backups = tempfile::tempdir().unwrap();
    let res = clean(
        &[sel(ResidueKind::RegValue {
            key: r"Software\Classes\rgfile".into(),
            value_name: None,
        })],
        "ripgrep",
        &fx,
        backups.path(),
    );
    assert_eq!(res, vec![Ok(())]);
    assert_eq!(
        rg.log()[1],
        r"delete_value Software\Classes\rgfile ".to_string()
    );
}

#[test]
fn clean_failed_export_leaves_key_intact() {
    let env = FakeEnv::new();
    let sc = FakeShortcuts::new(r"C:\SM");
    let tr = FakeTrash::new();
    let rg = OrderedRegistry::new(Some("Locked"));
    rg.inner.insert(r"Software\Locked", Some("x"), "1");
    rg.inner.insert(r"Software\Run", Some("rg"), "1");
    rg.inner.insert(r"Software\Free", Some("y"), "2");
    let fx = Effects {
        env: &env,
        shortcuts: &sc,
        trash: &tr,
        registry: &rg,
    };
    let backups = tempfile::tempdir().unwrap();
    let res = clean(
        &[
            sel(ResidueKind::RegKey {
                key: r"Software\Locked".into(),
            }),
            sel(ResidueKind::RegValue {
                key: r"Software\Locked".into(),
                value_name: Some("x".into()),
            }),
            sel(ResidueKind::RegKey {
                key: r"Software\Free".into(),
            }),
        ],
        "ripgrep",
        &fx,
        backups.path(),
    );
    assert_eq!(res.len(), 3);
    assert!(res[0].is_err(), "{res:?}");
    assert!(res[1].is_err(), "{res:?}");
    assert_eq!(res[2], Ok(()));
    let (state, _) = rg.inner.dump();
    assert_eq!(state[r"Software\Locked"].len(), 1, "locked key untouched");
    assert!(!state.contains_key(r"Software\Free"));
    assert!(
        rg.log()
            .iter()
            .all(|l| !l.starts_with(r"delete_key Software\Locked")
                && !l.starts_with(r"delete_value Software\Locked")),
        "{:?}",
        rg.log()
    );
}

#[test]
fn clean_sends_folders_and_shortcuts_to_trash() {
    let env = FakeEnv::new();
    let sc = FakeShortcuts::new(r"C:\SM");
    let tr = FakeTrash::new();
    let rg = OrderedRegistry::new(None);
    let fx = Effects {
        env: &env,
        shortcuts: &sc,
        trash: &tr,
        registry: &rg,
    };
    let backups = tempfile::tempdir().unwrap();
    let res = clean(
        &[
            sel(ResidueKind::Folder {
                path: PathBuf::from(r"C:\Users\u\AppData\Roaming\ripgrep"),
            }),
            sel(ResidueKind::Shortcut {
                path: PathBuf::from(r"C:\Users\u\Desktop\rg.lnk"),
            }),
        ],
        "ripgrep",
        &fx,
        backups.path(),
    );
    assert_eq!(res, vec![Ok(()), Ok(())]);
    assert_eq!(
        tr.dump(),
        vec![
            PathBuf::from(r"C:\Users\u\AppData\Roaming\ripgrep"),
            PathBuf::from(r"C:\Users\u\Desktop\rg.lnk"),
        ]
    );
    assert!(rg.log().is_empty());
    // No registry item ⇒ no backup directory created.
    assert_eq!(std::fs::read_dir(backups.path()).unwrap().count(), 0);
}
