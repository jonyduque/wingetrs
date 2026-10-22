#![allow(clippy::unwrap_used, clippy::expect_used)]
//! MSI portability analysis (Task 229). Synthetic MSIs built with the `msi` crate; the
//! cabinet is `tests/fixtures/msi/three.cab` (entries f_a, f_b, f_c).
use std::io::{Cursor, Write};
use std::path::Path;

use msi::{Column, Insert, Package, PackageType, Value};
use wingetrs::winget_install::portable_msi::{Blocker, Loss, MsiFacts, analyze};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/msi");

pub(crate) struct Extra {
    pub(crate) name: &'static str,
    pub(crate) columns: Vec<Column>,
    pub(crate) rows: Vec<Vec<Value>>,
}

pub(crate) struct AppMsi {
    /// Directory of `readme.txt` (f_c); `foo.exe` (f_a) is in INSTALLDIR, `lib.dll` (f_b) in BIN.
    pub(crate) f_c_dir: &'static str,
    pub(crate) word_count: i32,
    pub(crate) properties: Vec<(&'static str, &'static str)>,
    pub(crate) extra: Vec<Extra>,
}

pub(crate) fn app() -> AppMsi {
    AppMsi {
        f_c_dir: "INSTALLDIR",
        word_count: 2,
        properties: vec![("ProductName", "Foo")],
        extra: Vec::new(),
    }
}

pub(crate) fn build(s: AppMsi) -> Vec<u8> {
    let mut p = Package::create(PackageType::Installer, Cursor::new(Vec::new())).unwrap();
    p.summary_info_mut().set_word_count(s.word_count);
    p.create_table(
        "Directory",
        vec![
            Column::build("Directory").primary_key().id_string(72),
            Column::build("Directory_Parent").nullable().id_string(72),
            Column::build("DefaultDir").text_string(255),
        ],
    )
    .unwrap();
    p.create_table(
        "Component",
        vec![
            Column::build("Component").primary_key().id_string(72),
            Column::build("Directory_").id_string(72),
            Column::build("KeyPath").nullable().id_string(72),
        ],
    )
    .unwrap();
    p.create_table(
        "File",
        vec![
            Column::build("File").primary_key().id_string(72),
            Column::build("Component_").id_string(72),
            Column::build("FileName").text_string(255),
            Column::build("FileSize").int32(),
            Column::build("Version").nullable().string(72),
            Column::build("Attributes").nullable().int16(),
            Column::build("Sequence").int32(),
        ],
    )
    .unwrap();
    p.create_table(
        "Media",
        vec![
            Column::build("DiskId").primary_key().int16(),
            Column::build("LastSequence").int32(),
            Column::build("Cabinet").nullable().string(255),
        ],
    )
    .unwrap();
    p.create_table(
        "Property",
        vec![
            Column::build("Property").primary_key().id_string(72),
            Column::build("Value").text_string(0),
        ],
    )
    .unwrap();
    let dirs: [(&str, Option<&str>, &str); 5] = [
        ("TARGETDIR", None, "SourceDir"),
        ("ProgramFiles64Folder", Some("TARGETDIR"), "PFiles"),
        (
            "INSTALLDIR",
            Some("ProgramFiles64Folder"),
            "FOOT~1|Foo Target:FOOS~1|Foo Source",
        ),
        ("BIN", Some("INSTALLDIR"), "bin"),
        ("System64Folder", Some("TARGETDIR"), "System64"),
    ];
    for (d, parent, default) in dirs {
        let parent = parent.map_or(Value::Null, Value::from);
        p.insert_rows(Insert::into("Directory").row(vec![d.into(), parent, default.into()]))
            .unwrap();
    }
    let comps = [
        ("C_INSTALLDIR", "INSTALLDIR", "f_a"),
        ("C_BIN", "BIN", "f_b"),
        ("C_OTHER", s.f_c_dir, "f_c"),
    ];
    for (c, d, key) in comps {
        p.insert_rows(Insert::into("Component").row(vec![c.into(), d.into(), key.into()]))
            .unwrap();
    }
    let files = [
        ("f_a", "C_INSTALLDIR", "FOO~1.EXE|foo.exe", 1),
        ("f_b", "C_BIN", "lib.dll", 2),
        ("f_c", "C_OTHER", "readme.txt", 3),
    ];
    for (key, comp, name, seq) in files {
        p.insert_rows(Insert::into("File").row(vec![
            key.into(),
            comp.into(),
            name.into(),
            0.into(),
            Value::Null,
            Value::Int(0),
            seq.into(),
        ]))
        .unwrap();
    }
    p.insert_rows(Insert::into("Media").row(vec![Value::Int(1), 100.into(), "#cab1".into()]))
        .unwrap();
    for (k, v) in &s.properties {
        p.insert_rows(Insert::into("Property").row(vec![(*k).into(), (*v).into()]))
            .unwrap();
    }
    for e in s.extra {
        p.create_table(e.name, e.columns).unwrap();
        for r in e.rows {
            p.insert_rows(Insert::into(e.name).row(r)).unwrap();
        }
    }
    let cab = std::fs::read(Path::new(FIXTURES).join("three.cab")).unwrap();
    p.write_stream("cab1").unwrap().write_all(&cab).unwrap();
    p.into_inner().unwrap().into_inner()
}

pub(crate) fn facts(bytes: &[u8]) -> MsiFacts {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("x.msi");
    std::fs::write(&f, bytes).unwrap();
    analyze(&f).unwrap()
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_string()).collect()
}

/// `(Action, Type, in InstallExecuteSequence)`.
fn custom_actions(rows: &[(&'static str, i32, bool)]) -> Vec<Extra> {
    vec![
        Extra {
            name: "CustomAction",
            columns: vec![
                Column::build("Action").primary_key().id_string(72),
                Column::build("Type").int16(),
                Column::build("Source").nullable().id_string(72),
                Column::build("Target").nullable().string(255),
            ],
            rows: rows
                .iter()
                .map(|(a, t, _)| vec![(*a).into(), Value::Int(*t), Value::Null, Value::Null])
                .collect(),
        },
        Extra {
            name: "InstallExecuteSequence",
            columns: vec![
                Column::build("Action").primary_key().id_string(72),
                Column::build("Condition").nullable().string(255),
                Column::build("Sequence").nullable().int16(),
            ],
            rows: rows
                .iter()
                .filter(|(_, _, seq)| *seq)
                .enumerate()
                .map(|(n, (a, _, _))| vec![(*a).into(), Value::Null, Value::Int(4000 + n as i32)])
                .collect(),
        },
    ]
}

fn registry(rows: &[(&'static str, i32, &'static str)]) -> Extra {
    Extra {
        name: "Registry",
        columns: vec![
            Column::build("Registry").primary_key().id_string(72),
            Column::build("Root").int16(),
            Column::build("Key").text_string(255),
            Column::build("Name").nullable().text_string(255),
            Column::build("Value").nullable().text_string(0),
            Column::build("Component_").id_string(72),
        ],
        rows: rows
            .iter()
            .map(|(r, root, key)| {
                vec![
                    (*r).into(),
                    Value::Int(*root),
                    (*key).into(),
                    "x".into(),
                    "1".into(),
                    "C_INSTALLDIR".into(),
                ]
            })
            .collect(),
    }
}

#[test]
fn plain_app_is_portable() {
    let f = facts(&build(app()));
    assert_eq!(f.blockers, Vec::<Blocker>::new());
    assert_eq!(f.main_dir, Some(s(&["PFiles", "Foo Source"])));
    assert_eq!(
        f.files,
        vec![s(&["bin", "lib.dll"]), s(&["foo.exe"]), s(&["readme.txt"])]
    );
    assert!(!f.per_user_capable);
}

#[test]
fn service_blocks() {
    let extra = vec![Extra {
        name: "ServiceInstall",
        columns: vec![
            Column::build("ServiceInstall").primary_key().id_string(72),
            Column::build("Name").text_string(255),
            Column::build("Component_").id_string(72),
        ],
        rows: vec![vec!["svc".into(), "FooSvc".into(), "C_INSTALLDIR".into()]],
    }];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(f.blockers, vec![Blocker::Service("FooSvc".into())]);
}

#[test]
fn driver_blocks() {
    let extra = vec![Extra {
        name: "MsiDriverPackages",
        columns: vec![
            Column::build("Component").primary_key().id_string(72),
            Column::build("Flags").int32(),
        ],
        rows: vec![vec!["C_BIN".into(), 0.into()]],
    }];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(f.blockers, vec![Blocker::Driver("C_BIN".into())]);
}

#[test]
fn deferred_code_action_blocks_immediate_is_a_loss() {
    let extra = custom_actions(&[
        ("Deferred", 1 | 0x400, true),
        ("Immediate", 34, true),
        ("SetDir", 35 | 0x400, true),
        ("SetProp", 51, true),
        ("UiOnly", 1 | 0x400, false),
    ]);
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(f.blockers, vec![Blocker::DeferredAction("Deferred".into())]);
    assert_eq!(f.losses, vec![Loss::ImmediateAction("Immediate".into())]);
}

#[test]
fn com_class_blocks() {
    let extra = vec![Extra {
        name: "Class",
        columns: vec![
            Column::build("CLSID").primary_key().string(38),
            Column::build("Context").primary_key().id_string(32),
            Column::build("Component_").primary_key().id_string(72),
        ],
        rows: vec![vec![
            "{00000000-0000-0000-0000-000000000001}".into(),
            "InprocServer32".into(),
            "C_BIN".into(),
        ]],
    }];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(
        f.blockers,
        vec![Blocker::Com(
            "{00000000-0000-0000-0000-000000000001}".into()
        )]
    );
}

#[test]
fn com_registry_key_blocks() {
    let extra = vec![registry(&[(
        "r1",
        0,
        "CLSID\\{00000000-0000-0000-0000-000000000002}",
    )])];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(
        f.blockers,
        vec![Blocker::Com(
            "CLSID\\{00000000-0000-0000-0000-000000000002}".into()
        )]
    );
    assert_eq!(f.losses, Vec::<Loss>::new());
}

#[test]
fn system_registry_blocks_other_rows_are_a_loss() {
    let f = facts(&build(AppMsi {
        extra: vec![registry(&[
            ("r1", 2, "Software\\Foo"),
            ("r2", 1, "Software\\Foo"),
        ])],
        ..app()
    }));
    assert_eq!(f.blockers, Vec::<Blocker>::new());
    assert_eq!(f.losses, vec![Loss::Registry(2)]);
    let f = facts(&build(AppMsi {
        extra: vec![registry(&[(
            "r3",
            2,
            "SYSTEM\\CurrentControlSet\\Services\\Foo",
        )])],
        ..app()
    }));
    assert_eq!(
        f.blockers,
        vec![Blocker::SystemRegistry(
            "SYSTEM\\CurrentControlSet\\Services\\Foo".into()
        )]
    );
}

#[test]
fn system_files_block() {
    let f = facts(&build(AppMsi {
        f_c_dir: "System64Folder",
        ..app()
    }));
    assert_eq!(f.blockers, vec![Blocker::SystemFiles(1)]);
    assert_eq!(f.main_dir, Some(s(&["PFiles", "Foo Source"])));
}

#[test]
fn scattered_files_block() {
    let f = facts(&build(AppMsi {
        f_c_dir: "TARGETDIR",
        ..app()
    }));
    assert_eq!(f.blockers, vec![Blocker::Scattered]);
    assert_eq!(f.main_dir, None);
}

#[test]
fn associations_and_environment_are_losses() {
    let extra = vec![
        Extra {
            name: "Extension",
            columns: vec![
                Column::build("Extension").primary_key().text_string(255),
                Column::build("Component_").primary_key().id_string(72),
            ],
            rows: vec![vec!["foo".into(), "C_INSTALLDIR".into()]],
        },
        Extra {
            name: "Environment",
            columns: vec![
                Column::build("Environment").primary_key().id_string(72),
                Column::build("Name").text_string(255),
                Column::build("Value").nullable().text_string(0),
                Column::build("Component_").id_string(72),
            ],
            rows: vec![vec![
                "e1".into(),
                "=-*FOO_HOME".into(),
                "[INSTALLDIR]".into(),
                "C_INSTALLDIR".into(),
            ]],
        },
    ];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(f.blockers, Vec::<Blocker>::new());
    assert_eq!(
        f.losses,
        vec![
            Loss::Associations(s(&[".foo"])),
            Loss::Environment(s(&["FOO_HOME"]))
        ]
    );
}

#[test]
fn shortcut_targets_are_relative_to_the_main_dir() {
    let extra = vec![Extra {
        name: "Shortcut",
        columns: vec![
            Column::build("Shortcut").primary_key().id_string(72),
            Column::build("Directory_").id_string(72),
            Column::build("Name").text_string(128),
            Column::build("Component_").id_string(72),
            Column::build("Target").string(72),
        ],
        rows: vec![
            vec![
                "s1".into(),
                "ProgramMenuFolder".into(),
                "FOO|Foo App".into(),
                "C_INSTALLDIR".into(),
                "[#f_a]".into(),
            ],
            vec![
                "s2".into(),
                "DesktopFolder".into(),
                "Lib".into(),
                "C_BIN".into(),
                "DefaultFeature".into(),
            ],
        ],
    }];
    let f = facts(&build(AppMsi { extra, ..app() }));
    assert_eq!(
        f.shortcuts,
        vec![
            ("Foo App".to_string(), s(&["foo.exe"])),
            ("Lib".to_string(), s(&["bin", "lib.dll"]))
        ]
    );
}

#[test]
fn per_user_capable_needs_the_flag_and_the_property() {
    let with = |wc: i32, props: Vec<(&'static str, &'static str)>| {
        facts(&build(AppMsi {
            word_count: wc,
            properties: props,
            ..app()
        }))
        .per_user_capable
    };
    assert!(with(2 | 8, vec![("ProductName", "Foo"), ("ALLUSERS", "2")]));
    assert!(with(
        2 | 8,
        vec![("ProductName", "Foo"), ("MSIINSTALLPERUSER", "1")]
    ));
    assert!(!with(2, vec![("ProductName", "Foo"), ("ALLUSERS", "2")]));
    assert!(!with(
        2 | 8,
        vec![("ProductName", "Foo"), ("ALLUSERS", "1")]
    ));
}

#[test]
fn own_extraction_lands_where_the_analysis_says() {
    let bytes = build(app());
    let f = facts(&bytes);
    let d = tempfile::tempdir().unwrap();
    let msi = d.path().join("x.msi");
    std::fs::write(&msi, &bytes).unwrap();
    let dest = d.path().join("out");
    let dir = f.main_dir.clone().unwrap().join("\\");
    wingetrs::install::extract::extract(&msi, "x.msi", Some(&dir), None, &dest).unwrap();
    for rel in &f.files {
        let p = rel.iter().fold(dest.clone(), |p, c| p.join(c));
        assert!(p.is_file(), "{}", p.display());
    }
}
