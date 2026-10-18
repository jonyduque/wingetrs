#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Default switches and command line (Task 174).

use std::path::Path;
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
    assert_eq!(mode_for(&InstallerKind::Exe, &s), Mode::Silent);
    assert_eq!(
        args(&InstallerKind::Exe, &s, Mode::Silent, false).args,
        "/q"
    );

    let none = sw();
    assert_eq!(mode_for(&InstallerKind::Exe, &none), Mode::Interactive);
    assert_eq!(
        args(&InstallerKind::Exe, &none, Mode::Interactive, false).args,
        ""
    );

    let p = InstallerSwitches {
        silent_with_progress: Some("/passive".into()),
        ..sw()
    };
    assert_eq!(mode_for(&InstallerKind::Exe, &p), Mode::SilentWithProgress);
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
