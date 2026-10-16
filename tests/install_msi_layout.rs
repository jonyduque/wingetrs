#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::install::msi_layout::{CompRow, DirRow, FileRow, LayoutError, Placed, layout};

fn d(directory: &str, parent: Option<&str>, default_dir: &str) -> DirRow {
    DirRow {
        directory: directory.into(),
        parent: parent.map(Into::into),
        default_dir: default_dir.into(),
    }
}

fn c(component: &str, directory: &str) -> CompRow {
    CompRow {
        component: component.into(),
        directory: directory.into(),
    }
}

fn f(
    file: &str,
    component: &str,
    file_name: &str,
    version: Option<&str>,
    sequence: i32,
) -> FileRow {
    FileRow {
        file: file.into(),
        component: component.into(),
        file_name: file_name.into(),
        version: version.map(Into::into),
        sequence,
    }
}

fn p(file: &str, rel: &[&str]) -> Placed {
    Placed {
        file: file.into(),
        rel: rel.iter().map(|s| s.to_string()).collect(),
    }
}

fn root() -> DirRow {
    d("TARGETDIR", None, "SourceDir")
}

#[test]
fn source_name_after_colon_and_long_form() {
    let dirs = [
        root(),
        d("PF", Some("TARGETDIR"), "PFILES:Program Files"),
        d("APP", Some("PF"), "a~1|App Dir"),
        d("SAME", Some("APP"), "."),
    ];
    let comps = [c("C1", "SAME")];
    let files = [f("F1", "C1", "x.dll", None, 1)];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![p("F1", &["Program Files", "App Dir", "x.dll"])]
    );
}

#[test]
fn two_levels_destination_differs_from_source() {
    let dirs = [
        root(),
        d("L1", Some("TARGETDIR"), "dst1:src1"),
        d("L2", Some("L1"), "DST2~1|dest two:SRC2~1|source two"),
        d("L3", Some("L2"), "dst3:src3"),
    ];
    let comps = [c("C1", "L3")];
    let files = [f("F1", "C1", "a.txt", None, 1)];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![p("F1", &["src1", "source two", "src3", "a.txt"])]
    );
}

#[test]
fn targetdir_root_dropped() {
    let dirs = [root()];
    let comps = [c("C1", "TARGETDIR")];
    let files = [f("F1", "C1", "x.dll", None, 1)];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![p("F1", &["x.dll"])]
    );
    // A root whose parent is itself is also a root.
    let dirs = [d("TARGETDIR", Some("TARGETDIR"), "SourceDir")];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![p("F1", &["x.dll"])]
    );
}

#[test]
fn collision_rules() {
    let dirs = [root()];
    let comps = [c("C1", "TARGETDIR"), c("C2", "TARGETDIR")];
    // (a) versioned beats unversioned, whichever comes first.
    let files = [
        f("U", "C1", "a.dll", None, 1),
        f("V", "C2", "a.dll", Some("1.0"), 2),
    ];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![p("V", &["a.dll"])]
    );
    // (b) numeric version parts: 1.10.0 > 1.9.9.
    let files = [
        f("NEW", "C1", "b.dll", Some("1.10.0"), 5),
        f("OLD", "C2", "b.dll", Some("1.9.9"), 1),
    ];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![p("NEW", &["b.dll"])]
    );
    // (c) same version: lower sequence wins.
    let files = [
        f("S7", "C1", "c.dll", Some("2.0"), 7),
        f("S3", "C2", "c.dll", Some("2.0"), 3),
    ];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![p("S3", &["c.dll"])]
    );
    // (d) names differing only in case collide.
    let files = [
        f("LO", "C1", "readme.txt", None, 2),
        f("UP", "C2", "README.TXT", None, 4),
    ];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![p("LO", &["readme.txt"])]
    );
}

#[test]
fn cycle_missing_parent_depth() {
    let comps = [c("C1", "A")];
    let files = [f("F1", "C1", "x", None, 1)];

    let dirs = [root(), d("A", Some("B"), "a"), d("B", Some("A"), "b")];
    assert!(matches!(
        layout(&dirs, &comps, &files),
        Err(LayoutError::Cycle(_))
    ));

    let dirs = [root(), d("A", Some("NOPE"), "a")];
    assert!(matches!(
        layout(&dirs, &comps, &files),
        Err(LayoutError::MissingParent(_))
    ));

    let mut dirs = vec![root()];
    let mut parent = "TARGETDIR".to_string();
    for i in 0..70 {
        let name = if i == 69 {
            "A".to_string()
        } else {
            format!("D{i}")
        };
        dirs.push(d(&name, Some(&parent), &format!("n{i}")));
        parent = name;
    }
    assert!(matches!(
        layout(&dirs, &comps, &files),
        Err(LayoutError::TooDeep(_))
    ));

    let dirs = [root()];
    assert!(matches!(
        layout(&dirs, &comps, &files),
        Err(LayoutError::MissingDirectory(_))
    ));
}

#[test]
fn unsafe_name_refused() {
    let comps = [c("C1", "BAD")];
    let files = [f("F1", "C1", "x", None, 1)];
    let dirs = [root(), d("BAD", Some("TARGETDIR"), "..:..")];
    assert!(matches!(
        layout(&dirs, &comps, &files),
        Err(LayoutError::Unsafe(_))
    ));

    let comps = [c("C1", "TARGETDIR")];
    let files = [f("F1", "C1", "a|../y", None, 1)];
    assert!(matches!(
        layout(&[root()], &comps, &files),
        Err(LayoutError::Unsafe(_))
    ));
}

// Rows dumped with the `msi` crate (0.10.0) from target/probe-2b/corpus-msi/; expected paths
// are the files `msiexec /a` wrote under target/probe-2b/msi-admin/<id>/SourceDir.
#[test]
fn three_real_vectors() {
    // ncftpclient: Setup NcFTP 3.2.6.msi, sha256 b034c6c1...
    let dirs = [
        d("TARGETDIR", None, "SourceDir"),
        d(
            "_FEFA166629EE4B32B8A3D619BB8C8687",
            Some("_EE90B94DB8C444809B0336CA91C2EC5B"),
            "HTML|html",
        ),
        d(
            "_EE90B94DB8C444809B0336CA91C2EC5B",
            Some("TARGETDIR"),
            "DOC|doc",
        ),
        d(
            "WindowsFolder",
            Some("TARGETDIR"),
            ".:WINDOW~1|Windows Folder",
        ),
        d(
            "ProgramMenuFolder",
            Some("TARGETDIR"),
            ".:USER'S~1|User's Programs Menu",
        ),
        d(
            "_5D1BB256F6884871B6F84173336C20EB",
            Some("ProgramMenuFolder"),
            "NCFTP|NcFTP",
        ),
        d(
            "DesktopFolder",
            Some("TARGETDIR"),
            ".:USER'S~2|User's Desktop",
        ),
    ];
    let comps = [
        c(
            "C__121DA1EEE02E49A085D49ED9D2314908",
            "_FEFA166629EE4B32B8A3D619BB8C8687",
        ),
        c("C__57890D660A3E428ABF662D1F2BFCA9B9", "WindowsFolder"),
        c("C__89B7AC90F7F842FC81A394FB6731D439", "TARGETDIR"),
    ];
    let files = [
        f(
            "_121DA1EEE02E49A085D49ED9D2314908",
            "C__121DA1EEE02E49A085D49ED9D2314908",
            "INDEX~1.HTM|index.html",
            None,
            2,
        ),
        f(
            "_57890D660A3E428ABF662D1F2BFCA9B9",
            "C__57890D660A3E428ABF662D1F2BFCA9B9",
            "NCFTP.EXE|ncftp.exe",
            Some("3.2.6.20"),
            8,
        ),
        f(
            "_89B7AC90F7F842FC81A394FB6731D439",
            "C__89B7AC90F7F842FC81A394FB6731D439",
            "LOCATI~1.TXT|Location_of_Programs_is_C_Windows.txt",
            None,
            13,
        ),
    ];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![
            p(
                "_121DA1EEE02E49A085D49ED9D2314908",
                &["doc", "html", "index.html"]
            ),
            p(
                "_57890D660A3E428ABF662D1F2BFCA9B9",
                &["Windows Folder", "ncftp.exe"]
            ),
            p(
                "_89B7AC90F7F842FC81A394FB6731D439",
                &["Location_of_Programs_is_C_Windows.txt"]
            ),
        ]
    );

    // isocreator: IsoCreator.msi, sha256 f01a0a82...
    let dirs = [
        d("TARGETDIR", None, "SourceDir"),
        d("SHORTCUTDIR", Some("TARGETDIR"), "SHORTC~1|SHORTCUTDIR"),
        d("DesktopFolder", Some("TARGETDIR"), "Deskto~1|DesktopFolder"),
        d("APPDIR", Some("TARGETDIR"), "APPDIR:."),
    ];
    let comps = [
        c("Export.dll", "APPDIR"),
        c("Export.pdb", "APPDIR"),
        c("IsoCreator.exe", "APPDIR"),
    ];
    let files = [
        f("Export.dll", "Export.dll", "Export.dll", Some("1.0.0.0"), 1),
        f(
            "IsoCreator.exe",
            "IsoCreator.exe",
            "IsoCre~1.exe|IsoCreator.exe",
            Some("1.0.0.0"),
            3,
        ),
        f(
            "IsoCreator.pdb",
            "Export.pdb",
            "IsoCre~1.pdb|IsoCreator.pdb",
            None,
            4,
        ),
    ];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![
            p("Export.dll", &["Export.dll"]),
            p("IsoCreator.exe", &["IsoCreator.exe"]),
            p("IsoCreator.pdb", &["IsoCreator.pdb"]),
        ]
    );

    // pkgconf: pkgconf-x64-3.0.8.msi, sha256 a236479b...
    let dirs = [
        d(
            "INSTALLFOLDER",
            Some("ProgramFiles6432Folder"),
            "5lvxyizs|pkgconf-3.0.8",
        ),
        d("ProgramFiles6432Folder", Some("ProgramFiles64Folder"), "."),
        d("ProgramFiles64Folder", Some("TARGETDIR"), "PFiles64"),
        d("TARGETDIR", None, "SourceDir"),
    ];
    let comps = [
        c("PkgconfExe", "INSTALLFOLDER"),
        c("PkgconfigExe", "INSTALLFOLDER"),
        c("BomtoolExe", "INSTALLFOLDER"),
    ];
    let files = [
        f("PkgconfExeFile", "PkgconfExe", "pkgconf.exe", None, 4),
        f(
            "PkgconfigExeFile",
            "PkgconfigExe",
            "fwixba2l.exe|pkg-config.exe",
            None,
            3,
        ),
        f("BomtoolExeFile", "BomtoolExe", "bomtool.exe", None, 1),
    ];
    assert_eq!(
        layout(&dirs, &comps, &files).unwrap(),
        vec![
            p(
                "BomtoolExeFile",
                &["PFiles64", "pkgconf-3.0.8", "bomtool.exe"]
            ),
            p(
                "PkgconfigExeFile",
                &["PFiles64", "pkgconf-3.0.8", "pkg-config.exe"]
            ),
            p(
                "PkgconfExeFile",
                &["PFiles64", "pkgconf-3.0.8", "pkgconf.exe"]
            ),
        ]
    );
}
