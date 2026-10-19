#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Default switches and command line (Task 174).

use std::path::Path;
use wingetrs::config::InstallerMode;
use wingetrs::sources::winget::manifest::{InstallerKind, InstallerSwitches};
use wingetrs::winget_install::switches::*;

fn sw() -> InstallerSwitches {
    InstallerSwitches::default()
}

fn args(kind: &InstallerKind, s: &InstallerSwitches, mode: Mode, up: bool) -> CommandLine {
    install_args(kind, s, mode, up).unwrap()
}

#[test]
fn msi_silent_default() {
    let c = args(&InstallerKind::Msi, &sw(), Mode::Silent, false);
    assert_eq!(c.program, Program::Msiexec);
    assert_eq!(c.args, r#"/quiet /norestart /log "<LOGPATH>""#);
}

#[test]
fn wix_and_burn_share_msi_defaults() {
    let w = args(&InstallerKind::Wix, &sw(), Mode::Silent, false);
    assert_eq!(w.program, Program::Msiexec);
    assert_eq!(w.args, r#"/quiet /norestart /log "<LOGPATH>""#);
    let b = args(&InstallerKind::Burn, &sw(), Mode::Silent, false);
    assert_eq!(b.program, Program::Installer);
    assert_eq!(b.args, w.args);
}

#[test]
fn inno_silent_and_progress() {
    let s = args(&InstallerKind::Inno, &sw(), Mode::Silent, false);
    assert_eq!(
        s.args,
        r#"/SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG="<LOGPATH>""#
    );
    let p = args(&InstallerKind::Inno, &sw(), Mode::SilentWithProgress, false);
    assert_eq!(
        p.args,
        r#"/SP- /SILENT /SUPPRESSMSGBOXES /NORESTART /LOG="<LOGPATH>""#
    );
}

#[test]
fn nullsoft_silent() {
    let c = args(&InstallerKind::Nullsoft, &sw(), Mode::Silent, false);
    assert_eq!(c.args, "/S");
    assert_eq!(c.program, Program::Installer);
}

#[test]
fn manifest_silent_replaces_default_custom_appends() {
    let s = InstallerSwitches {
        silent: Some("/VERYSILENT /NOCANCEL".into()),
        custom: Some("/MERGETASKS=!desktopicon".into()),
        ..sw()
    };
    let c = args(&InstallerKind::Inno, &s, Mode::Silent, false);
    assert_eq!(
        c.args,
        r#"/VERYSILENT /NOCANCEL /LOG="<LOGPATH>" /MERGETASKS=!desktopicon"#
    );
}

#[test]
fn upgrade_only_on_upgrade() {
    let s = InstallerSwitches {
        custom: Some("ALLUSERS=1".into()),
        upgrade: Some("REINSTALL=ALL".into()),
        ..sw()
    };
    assert_eq!(
        args(&InstallerKind::Msi, &s, Mode::Silent, false).args,
        r#"/quiet /norestart /log "<LOGPATH>" ALLUSERS=1"#
    );
    assert_eq!(
        args(&InstallerKind::Msi, &s, Mode::Silent, true).args,
        r#"/quiet /norestart /log "<LOGPATH>" ALLUSERS=1 REINSTALL=ALL"#
    );
}

#[test]
fn exe_only_manifest() {
    let s = InstallerSwitches {
        silent: Some("/q".into()),
        ..sw()
    };
    assert_eq!(
        mode_for(&InstallerKind::Exe, &s, InstallerMode::Progress),
        Mode::Silent
    );
    assert_eq!(
        args(&InstallerKind::Exe, &s, Mode::Silent, false).args,
        "/q"
    );

    let none = sw();
    assert_eq!(
        mode_for(&InstallerKind::Exe, &none, InstallerMode::Progress),
        Mode::Interactive
    );
    assert_eq!(
        args(&InstallerKind::Exe, &none, Mode::Interactive, false).args,
        ""
    );

    let p = InstallerSwitches {
        silent_with_progress: Some("/passive".into()),
        ..sw()
    };
    assert_eq!(
        mode_for(&InstallerKind::Exe, &p, InstallerMode::Progress),
        Mode::SilentWithProgress
    );
    assert_eq!(
        args(&InstallerKind::Exe, &p, Mode::SilentWithProgress, false).args,
        "/passive"
    );

    let i = InstallerSwitches {
        interactive: Some("/i".into()),
        ..sw()
    };
    assert_eq!(
        args(&InstallerKind::Exe, &i, Mode::Interactive, false).args,
        "/i"
    );
}

#[test]
fn refuses_bad_args() {
    for bad in [
        "a\nb".to_string(),
        "a\rb".to_string(),
        "a\0b".to_string(),
        "x".repeat(33 * 1024),
    ] {
        let s = InstallerSwitches {
            custom: Some(bad),
            ..sw()
        };
        assert!(matches!(
            install_args(&InstallerKind::Msi, &s, Mode::Silent, false),
            Err(ArgsError::Invalid(_))
        ));
    }
}

#[test]
fn fill_log_quotes_and_refuses_quote_in_path() {
    assert_eq!(
        fill_log(r#"/log "<LOGPATH>""#, Path::new(r"C:\a b\x.log")).unwrap(),
        r#"/log "C:\a b\x.log""#
    );
    assert!(fill_log(r#"/log "<LOGPATH>""#, Path::new("C:\\a\"b\\x.log")).is_err());
    assert!(fill_log(r#"/log "<LOGPATH>""#, Path::new(r"C:\logs\")).is_err());
}

#[test]
fn msiexec_params_form() {
    assert_eq!(
        msiexec_params(Path::new(r"C:\x\a.msi"), "/quiet"),
        r#"/i "C:\x\a.msi" /quiet"#
    );
    assert_eq!(
        msiexec_params(Path::new(r"C:\x\a.msi"), ""),
        r#"/i "C:\x\a.msi""#
    );
}

#[test]
fn no_command_line_types() {
    for k in [
        InstallerKind::Msix,
        InstallerKind::Appx,
        InstallerKind::Zip,
        InstallerKind::Portable,
    ] {
        assert_eq!(
            install_args(&k, &sw(), Mode::Silent, false),
            Err(ArgsError::NoCommandLine)
        );
    }
}

/// Task 190: a bare `<LOGPATH>` gets its own quotes, so a log path with a space stays one
/// argument; an already quoted one is not quoted twice.
#[test]
fn bare_log_placeholder_refused_or_quoted() {
    let log = Path::new(r"C:\Program Files\wingetrs\staging\ab\install.log");
    let cases = [
        (
            "/LOG=<LOGPATH>",
            r#"/LOG="C:\Program Files\wingetrs\staging\ab\install.log""#,
        ),
        (
            "/log <LOGPATH>",
            r#"/log "C:\Program Files\wingetrs\staging\ab\install.log""#,
        ),
        (
            "/out:<LOGPATH> /S",
            r#"/out:"C:\Program Files\wingetrs\staging\ab\install.log" /S"#,
        ),
        (
            r#"/LOG="<LOGPATH>""#,
            r#"/LOG="C:\Program Files\wingetrs\staging\ab\install.log""#,
        ),
        (
            r#"/a "<LOGPATH>" /b <LOGPATH>"#,
            r#"/a "C:\Program Files\wingetrs\staging\ab\install.log" /b "C:\Program Files\wingetrs\staging\ab\install.log""#,
        ),
        ("/S", "/S"),
    ];
    for (args, want) in cases {
        assert_eq!(fill_log(args, log).unwrap(), want, "{args}");
    }
    // A bare placeholder at the end still refuses a trailing backslash (it would escape the
    // closing quote added).
    assert!(fill_log("/LOG=<LOGPATH>", Path::new(r"C:\logs\")).is_err());
}

/// Task 190 addendum: an Inno switch repeated by the manifest (any case) is passed once, the
/// first spelling kept; other kinds are left as they are.
#[test]
fn inno_repeated_switch_passed_once() {
    let s = InstallerSwitches {
        silent: Some("/VERYSILENT /CurrentUser".into()),
        custom: Some("/CURRENTUSER /MERGETASKS=!desktopicon".into()),
        ..sw()
    };
    assert_eq!(
        args(&InstallerKind::Inno, &s, Mode::Silent, false).args,
        r#"/VERYSILENT /CurrentUser /LOG="<LOGPATH>" /MERGETASKS=!desktopicon"#
    );
    let all = InstallerSwitches {
        custom: Some("/ALLUSERS /AllUsers".into()),
        ..sw()
    };
    assert_eq!(
        args(&InstallerKind::Inno, &all, Mode::Silent, false).args,
        r#"/SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG="<LOGPATH>" /ALLUSERS"#
    );
    let exe = InstallerSwitches {
        silent: Some("/S".into()),
        custom: Some("/x /X".into()),
        ..sw()
    };
    assert_eq!(
        args(&InstallerKind::Exe, &exe, Mode::Silent, false).args,
        "/S /x /X"
    );
}

#[test]
fn default_mode_is_silent_with_progress() {
    let m = mode_for(&InstallerKind::Msi, &sw(), InstallerMode::default());
    assert_eq!(m, Mode::SilentWithProgress);
    assert_eq!(
        args(&InstallerKind::Msi, &sw(), m, false).args,
        r#"/passive /norestart /log "<LOGPATH>""#
    );
    let m = mode_for(&InstallerKind::Inno, &sw(), InstallerMode::default());
    assert_eq!(m, Mode::SilentWithProgress);
    assert!(
        args(&InstallerKind::Inno, &sw(), m, false)
            .args
            .contains("/SILENT")
    );
}

#[test]
fn silent_setting_keeps_quiet() {
    let m = mode_for(&InstallerKind::Msi, &sw(), InstallerMode::Silent);
    assert_eq!(m, Mode::Silent);
    assert!(
        args(&InstallerKind::Msi, &sw(), m, false)
            .args
            .starts_with("/quiet")
    );
    // An exe with only a progress switch still uses it, and one with none stays interactive.
    let p = InstallerSwitches {
        silent_with_progress: Some("/p".into()),
        ..sw()
    };
    assert_eq!(
        mode_for(&InstallerKind::Exe, &p, InstallerMode::Silent),
        Mode::SilentWithProgress
    );
    assert_eq!(
        mode_for(&InstallerKind::Exe, &sw(), InstallerMode::Progress),
        Mode::Interactive
    );
}

#[test]
fn progress_setting_uses_manifest_progress_switch() {
    let p = InstallerSwitches {
        silent_with_progress: Some("/passive /x".into()),
        ..sw()
    };
    let m = mode_for(&InstallerKind::Msi, &p, InstallerMode::Progress);
    assert_eq!(
        args(&InstallerKind::Msi, &p, m, false).args,
        r#"/passive /x /log "<LOGPATH>""#
    );
}

#[test]
fn exe_with_both_switches_follows_the_setting() {
    let both = InstallerSwitches {
        silent: Some("/q".into()),
        silent_with_progress: Some("/p".into()),
        ..sw()
    };
    assert_eq!(
        mode_for(&InstallerKind::Exe, &both, InstallerMode::Progress),
        Mode::SilentWithProgress
    );
    assert_eq!(
        mode_for(&InstallerKind::Exe, &both, InstallerMode::Silent),
        Mode::Silent
    );
}
