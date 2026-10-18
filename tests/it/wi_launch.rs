#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Launcher and elevation rule (Task 178).
use wingetrs::installed::Hive;
use wingetrs::sources::winget::manifest::InstallerKind;
use wingetrs::winget_install::launch::{Elevation, install_elevation, uninstall_elevation};

#[test]
fn install_elevation_truth_table() {
    use Elevation::{Helper, Never};
    use InstallerKind::*;
    let table: &[(InstallerKind, Option<&str>, Option<&str>, Elevation)] = &[
        (Msi, None, None, Helper),
        (Msi, Some("user"), None, Never),
        (Inno, None, None, Never),
        (Inno, Some("machine"), None, Helper),
        (Nullsoft, Some("Machine"), Some("elevatesSelf"), Helper),
        (Exe, None, Some("elevatesSelf"), Never),
        (Exe, Some("user"), Some("elevationRequired"), Helper),
        (Msi, Some("machine"), Some("elevationProhibited"), Never),
        (Msix, Some("machine"), Some("elevationRequired"), Never),
        (Burn, None, None, Helper),
        (Wix, Some("user"), None, Never),
        // Extra rows: appx like msix, case-insensitive requirement, prohibited beats a
        // missing scope on msi.
        (Appx, Some("machine"), None, Never),
        (Exe, Some("USER"), Some("ElevationRequired"), Helper),
        (Wix, None, Some("ELEVATIONPROHIBITED"), Never),
        (Exe, None, None, Never),
    ];
    for (kind, scope, req, want) in table {
        assert_eq!(
            install_elevation(kind, *scope, *req),
            *want,
            "{kind:?} {scope:?} {req:?}"
        );
    }
}

#[test]
fn uninstall_elevation_rules() {
    assert_eq!(
        uninstall_elevation(Hive::User, true, true),
        Elevation::Never
    );
    assert_eq!(
        uninstall_elevation(Hive::MachineX86, true, false),
        Elevation::Helper
    );
    assert_eq!(
        uninstall_elevation(Hive::MachineX64, false, true),
        Elevation::Helper
    );
    assert_eq!(
        uninstall_elevation(Hive::MachineX64, false, false),
        Elevation::Never
    );
}

#[cfg(windows)]
mod win {
    use std::path::Path;
    use wingetrs::winget_install::launch::{LaunchSpec, Launcher, Show};
    use wingetrs::winget_install::launch_win::{ShellLauncher, is_elevated, msiexec_path};

    fn cmd(dir: &Path, body: &str) -> std::path::PathBuf {
        let p = dir.join("t.cmd");
        std::fs::write(&p, format!("{body}\r\n")).unwrap();
        p
    }

    fn spec(program: std::path::PathBuf, params: &str) -> LaunchSpec {
        LaunchSpec {
            program,
            params: params.to_string(),
            dir: None,
            show: Show::Hidden,
        }
    }

    #[test]
    fn cmd_exit_code_is_returned() {
        let t = tempfile::tempdir().unwrap();
        let p = cmd(t.path(), "@exit /b 7");
        assert_eq!(ShellLauncher.run(&spec(p, "")).unwrap(), 7);
    }

    #[test]
    fn params_reach_the_program() {
        let t = tempfile::tempdir().unwrap();
        let p = cmd(t.path(), r#"@if "%~1"=="ok" (exit /b 3) else (exit /b 4)"#);
        assert_eq!(ShellLauncher.run(&spec(p.clone(), "ok")).unwrap(), 3);
        assert_eq!(ShellLauncher.run(&spec(p, "no")).unwrap(), 4);
    }

    #[test]
    fn dir_is_the_working_directory() {
        let t = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        std::fs::write(work.path().join("marker.txt"), b"x").unwrap();
        let p = cmd(
            t.path(),
            "@if exist marker.txt (exit /b 5) else (exit /b 6)",
        );
        // Without `dir` the marker is not found: the 5 below comes from `dir`.
        assert_eq!(ShellLauncher.run(&spec(p.clone(), "")).unwrap(), 6);
        let mut s = spec(p, "");
        s.dir = Some(work.path().to_path_buf());
        assert_eq!(ShellLauncher.run(&s).unwrap(), 5);
    }

    #[test]
    fn missing_file_is_not_found() {
        let t = tempfile::tempdir().unwrap();
        let e = ShellLauncher
            .run(&spec(t.path().join("nope.cmd"), ""))
            .unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "{e}");
    }

    #[test]
    fn msiexec_path_is_system32() {
        let p = msiexec_path().unwrap();
        assert!(p.is_absolute(), "{}", p.display());
        let text = p.to_string_lossy().to_ascii_lowercase();
        assert!(text.ends_with(r"system32\msiexec.exe"), "{text}");
        assert!(p.exists(), "{}", p.display());
    }

    #[test]
    fn is_elevated_answers() {
        assert!(is_elevated().is_ok());
    }
}
