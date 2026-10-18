//! Uninstall command from an ARP entry (Task 172).
use wingetrs::installed::Hive;
use wingetrs::installed::arp::{expand_vars, missing_uninstaller, split_command};
use wingetrs::winget_install::uninstall_cmd::{
    ArpDetails, Rule, UninstallCommand, uninstall_command, valid_arp_key,
};

const G: &str = "{11111111-2222-3333-4444-555555555555}";

fn details(key: &str) -> ArpDetails {
    ArpDetails {
        hive: Hive::MachineX64,
        key: key.to_string(),
        display_name: Some("App".into()),
        publisher: None,
        display_version: None,
        windows_installer: false,
        system_component: false,
        quiet_uninstall_string: None,
        uninstall_string: None,
        inno_app_path: None,
        bundle_cache_path: None,
        bundle_upgrade_code: None,
        install_location: None,
    }
}

fn cmd(d: &ArpDetails, hint: bool) -> UninstallCommand {
    uninstall_command(d, hint).unwrap_or_else(|| panic!("no command for {}", d.key))
}

#[test]
fn msi_beats_quiet() {
    let mut d = details(G);
    d.windows_installer = true;
    d.quiet_uninstall_string = Some(r"C:\Q\q.exe /s".into());
    let c = cmd(&d, false);
    assert_eq!(c.rule, Rule::Msi);
    assert_eq!(c.program, "msiexec");
    assert_eq!(
        c.args,
        "/x{11111111-2222-3333-4444-555555555555} /quiet /norestart"
    );
    assert!(!c.interactive);
}

#[test]
fn msi_flag_without_guid_key_falls_through() {
    let mut d = details("Foo");
    d.windows_installer = true;
    d.uninstall_string = Some(r#""C:\X\u.exe""#.into());
    let c = cmd(&d, false);
    assert_eq!(c.rule, Rule::Interactive);
    assert_eq!(c.program, r"C:\X\u.exe");
    assert!(c.interactive);
}

#[test]
fn quiet_rule() {
    let mut d = details("Foo");
    d.quiet_uninstall_string = Some(r#""C:\Program Files\A\unins000.exe" /SILENT"#.into());
    let c = cmd(&d, false);
    assert_eq!(c.rule, Rule::Quiet);
    assert_eq!(c.program, r"C:\Program Files\A\unins000.exe");
    assert_eq!(c.args, "/SILENT");
    assert!(!c.interactive);
}

#[test]
fn inno_rule_needs_is1_and_app_path() {
    let mut d = details("App_is1");
    d.inno_app_path = Some(r"C:\P".into());
    d.uninstall_string = Some(r#""C:\P\unins000.exe""#.into());
    let c = cmd(&d, false);
    assert_eq!(c.rule, Rule::Inno);
    assert_eq!(c.args, "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART");
    let mut other = d.clone();
    other.key = "App".into();
    assert_eq!(cmd(&other, false).rule, Rule::Interactive);
    let mut no_path = d.clone();
    no_path.inno_app_path = None;
    assert_eq!(cmd(&no_path, false).rule, Rule::Interactive);
}

#[test]
fn burn_rule() {
    let mut d = details("{abc}");
    d.bundle_cache_path = Some(r"C:\ProgramData\Package Cache\{g}\setup.exe".into());
    d.uninstall_string = Some(r#""C:\ProgramData\Package Cache\{g}\setup.exe" /uninstall"#.into());
    let c = cmd(&d, false);
    assert_eq!(c.rule, Rule::Burn);
    assert_eq!(c.args, "/uninstall /quiet /norestart");
    d.uninstall_string =
        Some(r#""C:\ProgramData\Package Cache\{g}\setup.exe" /uninstall /Quiet /norestart"#.into());
    assert_eq!(cmd(&d, false).args, "/uninstall /Quiet /norestart");
}

#[test]
fn nsis_only_with_manifest_hint() {
    let mut d = details("Kindle");
    d.uninstall_string = Some(r#""C:\P\K\uninstall.exe""#.into());
    assert_eq!(cmd(&d, false).rule, Rule::Interactive);
    let c = cmd(&d, true);
    assert_eq!(c.rule, Rule::Nsis);
    assert_eq!(c.args, "/S");
    assert!(!c.interactive);
}

#[test]
fn unquoted_with_spaces() {
    let mut d = details("X");
    d.uninstall_string = Some(r"C:\Program Files\X\uninst.exe /x".into());
    let c = cmd(&d, false);
    assert_eq!(c.program, r"C:\Program Files\X\uninst.exe");
    assert_eq!(c.args, "/x");
}

#[test]
fn nothing_is_none() {
    let mut d = details("X");
    assert!(uninstall_command(&d, false).is_none());
    d.windows_installer = true;
    assert!(uninstall_command(&d, false).is_none());
}

#[test]
fn split_command_agrees_with_missing_uninstaller() {
    let env = |v: &str| (v == "ProgramFiles").then(|| r"C:\Program Files".to_string());
    let cmds = [
        r#""C:\Program Files\A\u.exe" /x"#,
        r"C:\Program Files\X\uninst.exe /x",
        r"C:\a\foo.exe.d",
        r"%ProgramFiles%\a.exe /x",
        "msiexec.exe /x{11111111-2222-3333-4444-555555555555}",
        "a.exe",
    ];
    let generic = ["msiexec.exe", "rundll32.exe", "cmd.exe"];
    for c in cmds {
        let expected = split_command(c)
            .and_then(|(p, _)| expand_vars(&p, &env))
            .filter(|p| {
                let b = p.as_bytes();
                let abs =
                    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
                let name = p.rsplit('\\').next().unwrap_or("").to_ascii_lowercase();
                abs && !generic.contains(&name.as_str())
            });
        let got = missing_uninstaller(c, env, |_| false).map(|p| p.to_string_lossy().into_owned());
        assert_eq!(got, expected, "{c}");
    }
    assert!(missing_uninstaller(r"C:\a\foo.exe.d", env, |_| false).is_none());
    assert!(missing_uninstaller(r"%ProgramFiles%\a.exe /x", env, |_| false).is_some());
}

#[test]
fn valid_arp_key_cases() {
    for bad in ["", "a\\b", "a/b", "..", "."] {
        assert!(!valid_arp_key(bad), "{bad}");
    }
    assert!(!valid_arp_key(&"a".repeat(256)));
    assert!(valid_arp_key(&"a".repeat(255)));
    for ok in [G, "KLiteCodecPack_is1", "Steam App 255710"] {
        assert!(valid_arp_key(ok), "{ok}");
    }
}

#[cfg(windows)]
#[test]
#[ignore = "live: SHEvaluateSystemCommandTemplate"]
fn live_split_matches_shell() {
    use wingetrs::winget_install::arp_read_win::shell_split;
    // The shell only splits commands whose program exists: use real copies in a temp dir.
    let dir = std::env::temp_dir().join(format!("wingetrs t172 {}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let sys = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_default())
        .join("System32")
        .join("cmd.exe");
    let exe = dir.join("uninst.exe");
    std::fs::copy(&sys, &exe).expect("copy cmd.exe");
    let p = exe.display().to_string();
    let cmds = [
        format!("\"{p}\" /SILENT"),
        format!("{p} /x"),
        format!("{p} /x{{11111111-2222-3333-4444-555555555555}}"),
    ];
    for c in &cmds {
        let (shell_prog, _) = shell_split(c).expect("shell split");
        let Some((prog, _)) = split_command(c) else {
            panic!("split failed: {c}");
        };
        assert_eq!(
            shell_prog.to_ascii_lowercase(),
            prog.to_ascii_lowercase(),
            "{c}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
