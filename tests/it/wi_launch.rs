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
        // A user scope never goes to the helper, not even with elevationRequired.
        (Exe, Some("user"), Some("elevationRequired"), Never),
        (Msi, Some("machine"), Some("elevationProhibited"), Never),
        (Msix, Some("machine"), Some("elevationRequired"), Never),
        (Burn, None, None, Helper),
        (Wix, Some("user"), None, Never),
        // Extra rows: appx like msix, case-insensitive requirement, prohibited beats a
        // missing scope on msi.
        (Appx, Some("machine"), None, Never),
        (Exe, Some("USER"), Some("ElevationRequired"), Never),
        (Exe, None, Some("ElevationRequired"), Helper),
        (Inno, Some("x"), Some("elevationRequired"), Helper),
        // An unknown scope counts as absent.
        (Msi, Some("x"), None, Helper),
        (Burn, Some(""), None, Helper),
        (Inno, Some("x"), None, Never),
        (Exe, Some(""), None, Never),
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
    use wingetrs::winget_install::launch::{LaunchSpec, Launcher, Show, Verb};
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
            verb: Verb::Open,
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

    /// Task 196: `run_started` tells `started` once the process exists (before its end, which
    /// the script waits on through a file the callback creates) and never when nothing started.
    #[test]
    fn run_started_signals_once_after_the_process_exists() {
        let t = tempfile::tempdir().unwrap();
        let go = t.path().join("go.txt");
        // Exits 8 once `go.txt` exists (checked about once a second, 40 times), else 9.
        let p = cmd(
            t.path(),
            "@echo off\r\nset n=0\r\n:loop\r\nif exist \"%~dp0go.txt\" exit /b 8\r\nset /a n+=1\r\n\
             if %n% geq 40 exit /b 9\r\nping -n 2 127.0.0.1 >nul\r\ngoto loop",
        );
        let mut seen = 0;
        let code = ShellLauncher
            .run_started(&spec(p, ""), &mut || {
                seen += 1;
                std::fs::write(&go, b"x").unwrap();
            })
            .unwrap();
        assert_eq!(seen, 1);
        assert_eq!(code, 8, "started was not called while the process ran");

        let mut seen = 0;
        let e = ShellLauncher
            .run_started(&spec(t.path().join("nope.cmd"), ""), &mut || seen += 1)
            .unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "{e}");
        assert_eq!(seen, 0);
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
