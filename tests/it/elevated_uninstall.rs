#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Elevated uninstall and key deletion (Task 182).
use std::io::BufReader;
use std::path::{Path, PathBuf};

use wingetrs::bridge::worker::{
    self, DeleteReason, MachineHive, Reply, Request, WorkerOps, msiexec_uninstall_args,
};
use wingetrs::installed::Hive;
use wingetrs::winget_install::elevated_rules::{
    KnownFolders, elevated_duplicate, expand_known, in_admin_folder, is_duplicate, still_orphan,
};
use wingetrs::winget_install::uninstall_cmd::{ArpDetails, Rule};

const GUID: &str = "{12345678-1234-1234-1234-123456789ABC}";
const MSIEXEC: &str = r"C:\W\System32\msiexec.exe";

fn kf() -> KnownFolders {
    KnownFolders {
        program_files: PathBuf::from(r"C:\PF"),
        program_files_x86: PathBuf::from(r"C:\PF86"),
        system_root: PathBuf::from(r"C:\W"),
        program_data: PathBuf::from(r"C:\PD"),
    }
}

fn entry(hive: Hive, key: &str) -> ArpDetails {
    ArpDetails {
        hive,
        key: key.to_string(),
        display_name: None,
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

fn quiet(hive: Hive, key: &str, cmd: &str) -> ArpDetails {
    ArpDetails {
        quiet_uninstall_string: Some(cmd.to_string()),
        ..entry(hive, key)
    }
}

fn named(hive: Hive, key: &str, name: &str) -> ArpDetails {
    ArpDetails {
        display_name: Some(name.to_string()),
        ..entry(hive, key)
    }
}

/// Operations over synthetic ARP entries and files; records what would have run.
#[derive(Default)]
struct FakeOps {
    entries: Vec<ArpDetails>,
    files: Vec<PathBuf>,
    reads: usize,
    launched: Vec<(PathBuf, String)>,
    deleted: Vec<(MachineHive, String)>,
}

impl WorkerOps for FakeOps {
    fn run(
        &mut self,
        program: &str,
        _args: &[String],
        _on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<(i32, Vec<String>)> {
        panic!("unexpected run of {program}")
    }

    fn run_installer(
        &mut self,
        _req: &worker::InstallerRequest,
        _on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<i32> {
        panic!("unexpected installer")
    }

    fn known_folders(&self) -> std::io::Result<KnownFolders> {
        Ok(kf())
    }

    fn read_arp(&mut self, hive: Hive, key: &str) -> std::io::Result<Option<ArpDetails>> {
        self.reads += 1;
        Ok(self
            .entries
            .iter()
            .find(|e| e.hive == hive && e.key == key)
            .cloned())
    }

    fn arp_entries(&mut self) -> std::io::Result<Vec<ArpDetails>> {
        Ok(self.entries.clone())
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.iter().any(|f| f == path)
    }

    fn msiexec(&self) -> std::io::Result<PathBuf> {
        Ok(PathBuf::from(MSIEXEC))
    }

    fn run_uninstaller(&mut self, program: &Path, args: &str) -> std::io::Result<i32> {
        self.launched
            .push((program.to_path_buf(), args.to_string()));
        Ok(7)
    }

    fn delete_arp_key(&mut self, hive: MachineHive, key: &str) -> std::io::Result<()> {
        self.deleted.push((hive, key.to_string()));
        Ok(())
    }
}

fn serve_lines(input: &str, ops: &mut FakeOps) -> (std::io::Result<()>, Vec<Reply>) {
    let mut out = Vec::new();
    let r = worker::serve(&mut BufReader::new(input.as_bytes()), &mut out, ops);
    let replies = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| worker::decode::<Reply>(l).unwrap())
        .collect();
    (r, replies)
}

fn ask(req: &Request, ops: &mut FakeOps) -> Reply {
    let (r, replies) = serve_lines(&worker::encode(req).unwrap(), ops);
    r.unwrap();
    assert_eq!(replies.len(), 1, "{replies:?}");
    replies.into_iter().next().unwrap()
}

fn uninstall(hive: MachineHive, key: &str, rule: Rule) -> Request {
    Request::RunUninstaller {
        hive,
        key: key.to_string(),
        rule,
        nsis_hint: false,
    }
}

fn delete(hive: MachineHive, key: &str, reason: DeleteReason) -> Request {
    Request::DeleteArpKey {
        hive,
        key: key.to_string(),
        reason,
    }
}

fn error_text(reply: &Reply) -> &str {
    match reply {
        Reply::Error { message, .. } => message,
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn expand_known_only() {
    let k = kf();
    assert_eq!(
        expand_known(r"%ProgramFiles%\A\u.exe", &k).as_deref(),
        Some(r"C:\PF\A\u.exe")
    );
    assert_eq!(
        expand_known(r"%programfiles(x86)%\B\u.exe", &k).as_deref(),
        Some(r"C:\PF86\B\u.exe")
    );
    for raw in [
        r"%LOCALAPPDATA%\x.exe",
        r"%TEMP%\x.exe",
        r"%USERPROFILE%\x.exe",
    ] {
        assert_eq!(expand_known(raw, &k), None, "{raw}");
    }
    assert_eq!(expand_known(r"%ProgramFiles\x", &k), None);
}

#[test]
fn expand_known_every_listed_variable() {
    let k = kf();
    for (raw, want) in [
        (r"%ProgramW6432%\a", r"C:\PF\a"),
        (r"%CommonProgramFiles%\a", r"C:\PF\Common Files\a"),
        (r"%CommonProgramFiles(x86)%\a", r"C:\PF86\Common Files\a"),
        (r"%SystemRoot%\a", r"C:\W\a"),
        (r"%WINDIR%\a", r"C:\W\a"),
        (r"%ProgramData%\a", r"C:\PD\a"),
        (r"C:\plain\a", r"C:\plain\a"),
    ] {
        assert_eq!(expand_known(raw, &k).as_deref(), Some(want), "{raw}");
    }
}

#[test]
fn admin_folder_rule() {
    let k = kf();
    let yes = |p: &str| in_admin_folder(Path::new(p), &k);
    assert!(yes(r"C:\PF\A\u.exe"));
    assert!(!yes(r"C:\PFX\u.exe"));
    assert!(!yes(r"C:\PF\..\Users\x.exe"));
    assert!(yes(r"C:\PD\Package Cache\{g}\s.exe"));
    assert!(!yes(r"C:\PD\Other\s.exe"));
    assert!(!yes(r"C:\Users\me\AppData\Local\Temp\uninst.exe"));
    assert!(yes(r"c:\pf\a\U.EXE"));
}

#[test]
fn admin_folder_other_roots_and_user_writable_windows_folders() {
    let k = kf();
    let yes = |p: &str| in_admin_folder(Path::new(p), &k);
    assert!(yes(r"C:\PF86\B\u.exe"));
    assert!(yes(r"C:\W\System32\x.exe"));
    assert!(!yes(r"C:\PF"), "the root itself is not inside it");
    assert!(!yes(r"PF\A\u.exe"), "relative");
    assert!(!yes(r"\PF\A\u.exe"), "no drive");
    assert!(!yes(r"D:\PF\A\u.exe"), "other drive");
    for p in [
        r"C:\W\Temp\u.exe",
        r"C:\W\Tasks\u.exe",
        r"C:\W\tracing\u.exe",
        r"C:\W\System32\Tasks\u.exe",
        r"C:\W\System32\spool\drivers\color\u.exe",
        r"C:\W\SysWOW64\Tasks\u.exe",
        r"c:\w\temp\sub\u.exe",
    ] {
        assert!(!yes(p), "{p}");
    }
}

#[test]
fn still_orphan_and_duplicate_rules() {
    let k = kf();
    let d = ArpDetails {
        uninstall_string: Some(r#""%ProgramFiles%\gone\u.exe" /S"#.into()),
        ..entry(Hive::MachineX86, "gone")
    };
    assert!(still_orphan(&d, &k, |_| false));
    assert!(!still_orphan(&d, &k, |p| p == Path::new(r"C:\PF\gone\u.exe")));
    let user_var = ArpDetails {
        uninstall_string: Some(r"%LOCALAPPDATA%\gone\u.exe".into()),
        ..entry(Hive::MachineX86, "gone")
    };
    assert!(!still_orphan(&user_var, &k, |_| false), "unknown variable");
    assert!(!still_orphan(&entry(Hive::MachineX86, "x"), &k, |_| false));

    let me = named(Hive::MachineX86, "Kindle", "Amazon Kindle");
    assert!(!is_duplicate(&me, std::slice::from_ref(&me)), "itself");
    let same_key_other_hive = named(Hive::MachineX64, "Kindle", "AMAZON KINDLE");
    assert!(is_duplicate(&me, &[me.clone(), same_key_other_hive]));
    let other_key = named(Hive::MachineX86, "Kindle2", "amazon kindle");
    assert!(is_duplicate(&me, &[me.clone(), other_key]));
    let other_name = named(Hive::User, "K", "Amazon Kindle Beta");
    assert!(!is_duplicate(&me, &[me.clone(), other_name]));
    let unnamed = entry(Hive::MachineX86, "a");
    assert!(!is_duplicate(&unnamed, &[entry(Hive::User, "b")]));
}

#[test]
fn run_uninstaller_recomputes_rule() {
    let mut ops = FakeOps {
        entries: vec![quiet(Hive::MachineX86, "app", r#""C:\PF\A\u.exe" /S"#)],
        files: vec![PathBuf::from(r"C:\PF\A\u.exe")],
        ..FakeOps::default()
    };
    let reply = ask(&uninstall(MachineHive::Hklm32, "app", Rule::Msi), &mut ops);
    assert!(error_text(&reply).contains("a chave mudou"), "{reply:?}");
    assert!(ops.launched.is_empty());
}

#[test]
fn run_uninstaller_runs_checked_admin_program() {
    let mut ops = FakeOps {
        entries: vec![quiet(Hive::MachineX86, "app", r#""C:\PF\A\u.exe" /S"#)],
        files: vec![PathBuf::from(r"C:\PF\A\u.exe")],
        ..FakeOps::default()
    };
    let reply = ask(
        &uninstall(MachineHive::Hklm32, "app", Rule::Quiet),
        &mut ops,
    );
    assert_eq!(reply, Reply::End { code: 7 });
    assert_eq!(
        ops.launched,
        vec![(PathBuf::from(r"C:\PF\A\u.exe"), "/S".to_string())]
    );
}

#[test]
fn run_uninstaller_uses_the_view_of_the_hive() {
    // The same key name in the other view must not be what runs.
    let mut ops = FakeOps {
        entries: vec![quiet(Hive::MachineX64, "app", r#""C:\PF\A\u.exe" /S"#)],
        files: vec![PathBuf::from(r"C:\PF\A\u.exe")],
        ..FakeOps::default()
    };
    let reply = ask(
        &uninstall(MachineHive::Hklm32, "app", Rule::Quiet),
        &mut ops,
    );
    assert_eq!(error_text(&reply), worker::KEY_GONE);
    assert!(ops.launched.is_empty());
    let reply = ask(
        &uninstall(MachineHive::Hklm64, "app", Rule::Quiet),
        &mut ops,
    );
    assert_eq!(reply, Reply::End { code: 7 });
}

#[test]
fn run_uninstaller_nsis_hint_changes_the_rule() {
    let d = ArpDetails {
        uninstall_string: Some(r#""C:\PF\N\uninst.exe""#.into()),
        ..entry(Hive::MachineX64, "nsis")
    };
    let mut ops = FakeOps {
        entries: vec![d],
        files: vec![PathBuf::from(r"C:\PF\N\uninst.exe")],
        ..FakeOps::default()
    };
    let req = Request::RunUninstaller {
        hive: MachineHive::Hklm64,
        key: "nsis".into(),
        rule: Rule::Nsis,
        nsis_hint: true,
    };
    assert_eq!(ask(&req, &mut ops), Reply::End { code: 7 });
    assert_eq!(ops.launched[0].1, "/S");
    let reply = ask(
        &uninstall(MachineHive::Hklm64, "nsis", Rule::Nsis),
        &mut ops,
    );
    assert_eq!(error_text(&reply), worker::KEY_CHANGED);
    assert_eq!(ops.launched.len(), 1);
}

#[test]
fn msi_rule_runs_system_msiexec() {
    let d = ArpDetails {
        windows_installer: true,
        uninstall_string: Some(format!(r"C:\Users\me\evil\msiexec.exe /x{GUID}")),
        ..entry(Hive::MachineX64, GUID)
    };
    let mut ops = FakeOps {
        entries: vec![d],
        ..FakeOps::default()
    };
    let reply = ask(&uninstall(MachineHive::Hklm64, GUID, Rule::Msi), &mut ops);
    assert_eq!(reply, Reply::End { code: 7 });
    assert_eq!(
        ops.launched,
        vec![(
            PathBuf::from(MSIEXEC),
            format!("/x{GUID} /quiet /norestart")
        )]
    );
}

#[test]
fn quiet_msiexec_line_runs_system_msiexec_only_with_x_guid() {
    let ok = quiet(
        Hive::MachineX86,
        "q",
        &format!(r"C:\Users\me\MsiExec.exe /X{GUID} /qn REBOOT=ReallySuppress"),
    );
    let mut ops = FakeOps {
        entries: vec![ok],
        ..FakeOps::default()
    };
    let reply = ask(&uninstall(MachineHive::Hklm32, "q", Rule::Quiet), &mut ops);
    assert_eq!(reply, Reply::End { code: 7 });
    assert_eq!(
        ops.launched,
        vec![(
            PathBuf::from(MSIEXEC),
            format!("/x{GUID} /qn REBOOT=ReallySuppress")
        )]
    );
    for bad in [
        format!("MsiExec.exe /I{GUID}"),
        format!("MsiExec.exe /f{GUID}"),
        format!("MsiExec.exe /p {GUID}"),
        format!(r"MsiExec.exe /x{GUID} /p C:\Users\me\patch.msp"),
        format!(r"MsiExec.exe /x{GUID} TRANSFORMS=C:\Users\me\t.mst"),
        "MsiExec.exe /x{not-a-guid}".to_string(),
        "MsiExec.exe /x".to_string(),
        "MsiExec.exe".to_string(),
    ] {
        let mut ops = FakeOps {
            entries: vec![quiet(Hive::MachineX86, "q", &bad)],
            ..FakeOps::default()
        };
        let reply = ask(&uninstall(MachineHive::Hklm32, "q", Rule::Quiet), &mut ops);
        assert_eq!(error_text(&reply), worker::MSIEXEC_REFUSED, "{bad}");
        assert!(ops.launched.is_empty(), "{bad}");
    }
}

#[test]
fn msiexec_args_forms() {
    assert_eq!(
        msiexec_uninstall_args(&format!("/x {GUID} /quiet")).as_deref(),
        Some(format!("/x{GUID} /quiet").as_str())
    );
    assert_eq!(
        msiexec_uninstall_args(&format!("-X{GUID}")).as_deref(),
        Some(format!("/x{GUID}").as_str())
    );
    assert_eq!(
        msiexec_uninstall_args(&format!("/x{GUID} /l*v x.log")),
        None
    );
}

#[test]
fn protocol_has_no_user_hive() {
    for line in [
        r#"{"op":"run_uninstaller","hive":"hkcu","key":"app","rule":"quiet","nsis_hint":false}"#,
        r#"{"op":"delete_arp_key","hive":"hkcu","key":"app","reason":"orphan"}"#,
        r#"{"op":"run_uninstaller","hive":"hklm32","key":"app","rule":"quiet","nsis_hint":false,"program":"C:\\x.exe"}"#,
        r#"{"op":"delete_arp_key","hive":"hklm32","key":"app","reason":"orphan","force":true}"#,
        r#"{"op":"delete_arp_key","hive":"hklm32","key":"app","reason":"anything"}"#,
    ] {
        let mut ops = FakeOps {
            entries: vec![
                quiet(Hive::User, "app", r#""C:\PF\A\u.exe" /S"#),
                quiet(Hive::MachineX86, "app", r#""C:\PF\A\u.exe" /S"#),
            ],
            files: vec![PathBuf::from(r"C:\PF\A\u.exe")],
            ..FakeOps::default()
        };
        let input = format!("{line}\n");
        let (r, replies) = serve_lines(&input, &mut ops);
        assert!(r.is_err(), "{line}");
        assert!(replies.is_empty(), "{line}");
        assert_eq!(ops.reads, 0, "{line}");
        assert!(ops.launched.is_empty() && ops.deleted.is_empty(), "{line}");
    }
    // The same request with an HKLM view is accepted (the lines above fail only on the hive
    // or the extra field).
    let mut ops = FakeOps {
        entries: vec![quiet(Hive::MachineX86, "app", r#""C:\PF\A\u.exe" /S"#)],
        files: vec![PathBuf::from(r"C:\PF\A\u.exe")],
        ..FakeOps::default()
    };
    let line =
        r#"{"op":"run_uninstaller","hive":"hklm32","key":"app","rule":"quiet","nsis_hint":false}"#;
    let (r, replies) = serve_lines(&format!("{line}\n"), &mut ops);
    r.unwrap();
    assert_eq!(replies, vec![Reply::End { code: 7 }]);
}

#[test]
fn delete_orphan_revalidates() {
    for (hive, view) in [
        (MachineHive::Hklm32, Hive::MachineX86),
        (MachineHive::Hklm64, Hive::MachineX64),
    ] {
        let d = ArpDetails {
            uninstall_string: Some(r#""C:\PF\gone\u.exe" /S"#.into()),
            ..entry(view, "gone")
        };
        let mut ops = FakeOps {
            entries: vec![d.clone()],
            files: vec![PathBuf::from(r"C:\PF\gone\u.exe")],
            ..FakeOps::default()
        };
        let reply = ask(&delete(hive, "gone", DeleteReason::Orphan), &mut ops);
        assert_eq!(error_text(&reply), "a chave deixou de ser sobra");
        assert!(ops.deleted.is_empty());

        let mut ops = FakeOps {
            entries: vec![d],
            ..FakeOps::default()
        };
        let reply = ask(&delete(hive, "gone", DeleteReason::Orphan), &mut ops);
        assert_eq!(reply, Reply::End { code: 0 });
        assert_eq!(ops.deleted, vec![(hive, "gone".to_string())]);
    }
}

#[test]
fn delete_of_a_missing_key_is_refused() {
    let mut ops = FakeOps::default();
    let reply = ask(
        &delete(MachineHive::Hklm32, "gone", DeleteReason::Orphan),
        &mut ops,
    );
    assert_eq!(error_text(&reply), worker::KEY_GONE);
    assert!(ops.deleted.is_empty());
}

fn versioned(hive: Hive, key: &str, name: &str, version: &str) -> ArpDetails {
    ArpDetails {
        display_version: Some(version.to_string()),
        ..named(hive, key, name)
    }
}

fn ask_duplicate(entries: Vec<ArpDetails>, hive: MachineHive, key: &str) -> (Reply, FakeOps) {
    let mut ops = FakeOps {
        entries,
        ..FakeOps::default()
    };
    let reply = ask(&delete(hive, key, DeleteReason::Duplicate), &mut ops);
    (reply, ops)
}

/// Task 182 review (security): only another HKLM record may make an HKLM key a duplicate;
/// an HKCU record (written by the user) never does, and only the non-principal record (the
/// lower `DisplayVersion`) may go.
#[test]
fn delete_duplicate_needs_other_record() {
    let me = versioned(Hive::MachineX86, "Kindle", "Amazon Kindle", "2.8.3");
    // No other record with the same name.
    let (reply, ops) = ask_duplicate(
        vec![me.clone(), versioned(Hive::User, "Other", "Kindle", "9.0")],
        MachineHive::Hklm32,
        "Kindle",
    );
    assert_eq!(error_text(&reply), "a chave deixou de ser duplicada");
    assert!(ops.deleted.is_empty());

    // An identical, newer record in HKCU does not authorize deleting the HKLM one.
    let (reply, ops) = ask_duplicate(
        vec![
            me.clone(),
            versioned(Hive::User, "Amazon Kindle", " amazon kindle ", "2.9.1"),
        ],
        MachineHive::Hklm32,
        "Kindle",
    );
    assert_eq!(error_text(&reply), "a chave deixou de ser duplicada");
    assert!(ops.deleted.is_empty());

    // A newer record in the other HKLM view does.
    let (reply, ops) = ask_duplicate(
        vec![
            me.clone(),
            versioned(
                Hive::MachineX64,
                "Amazon Kindle",
                " amazon kindle ",
                "2.9.1",
            ),
        ],
        MachineHive::Hklm32,
        "Kindle",
    );
    assert_eq!(reply, Reply::End { code: 0 });
    assert_eq!(
        ops.deleted,
        vec![(MachineHive::Hklm32, "Kindle".to_string())]
    );
}

#[test]
fn elevated_duplicate_never_for_a_user_record() {
    let mine = versioned(Hive::User, "K", "Amazon Kindle", "1.0");
    let newer = versioned(Hive::MachineX64, "K", "Amazon Kindle", "2.0");
    assert!(!elevated_duplicate(&mine, &[mine.clone(), newer.clone()]));
    let hklm = versioned(Hive::MachineX86, "K", "Amazon Kindle", "1.0");
    assert!(elevated_duplicate(&hklm, &[hklm.clone(), newer]));
}

#[test]
fn delete_duplicate_only_the_non_principal_hklm_record() {
    let old = versioned(Hive::MachineX86, "K1", "Amazon Kindle", "2.8.3");
    let new = versioned(Hive::MachineX86, "K2", "amazon kindle", "2.10");
    let both = vec![old.clone(), new.clone()];
    // The newer one (2.10 > 2.8.3 as versions, not as text) is the principal: refused.
    let (reply, ops) = ask_duplicate(both.clone(), MachineHive::Hklm32, "K2");
    assert_eq!(error_text(&reply), "a chave deixou de ser duplicada");
    assert!(ops.deleted.is_empty());
    let (reply, ops) = ask_duplicate(both, MachineHive::Hklm32, "K1");
    assert_eq!(reply, Reply::End { code: 0 });
    assert_eq!(ops.deleted, vec![(MachineHive::Hklm32, "K1".to_string())]);

    // Same version, or no version: which one is the principal is unknown, so neither goes.
    for (a, b) in [("2.8.3", "2.8.3.0"), ("", "2.9")] {
        let x = versioned(Hive::MachineX86, "K1", "Amazon Kindle", a);
        let y = versioned(Hive::MachineX64, "K1", "Amazon Kindle", b);
        let (reply, ops) = ask_duplicate(vec![x, y], MachineHive::Hklm32, "K1");
        assert_eq!(
            error_text(&reply),
            "a chave deixou de ser duplicada",
            "{a} {b}"
        );
        assert!(ops.deleted.is_empty());
    }
}

#[test]
fn bad_key_refused() {
    let long = "a".repeat(256);
    for key in ["", r"a\b", long.as_str()] {
        for req in [
            delete(MachineHive::Hklm32, key, DeleteReason::Orphan),
            uninstall(MachineHive::Hklm32, key, Rule::Quiet),
        ] {
            let mut ops = FakeOps {
                entries: vec![quiet(Hive::MachineX86, key, r#""C:\PF\A\u.exe" /S"#)],
                ..FakeOps::default()
            };
            let reply = ask(&req, &mut ops);
            assert_eq!(error_text(&reply), worker::BAD_KEY, "{key:?}");
            assert_eq!(ops.reads, 0, "{key:?}");
            assert!(ops.deleted.is_empty() && ops.launched.is_empty());
        }
    }
}

#[test]
fn user_folder_uninstaller_refused() {
    let mut ops = FakeOps {
        entries: vec![quiet(
            Hive::MachineX86,
            "usb",
            r"%LOCALAPPDATA%\Temp\u.exe /S",
        )],
        files: vec![PathBuf::from(r"C:\Users\me\AppData\Local\Temp\u.exe")],
        ..FakeOps::default()
    };
    let reply = ask(
        &uninstall(MachineHive::Hklm32, "usb", Rule::Quiet),
        &mut ops,
    );
    assert!(
        error_text(&reply).contains("fora de pasta de administrador"),
        "{reply:?}"
    );
    assert!(ops.launched.is_empty());
}

/// Each refusal of the program, with an entry that passes every other check.
#[test]
fn program_refusals_each_alone() {
    let pf = |p: &str| PathBuf::from(p);
    let cases: [(&str, &str, Vec<PathBuf>); 6] = [
        // Absolute path in the user's folder, existing.
        (
            r#""C:\Users\me\AppData\Local\Temp\uninst.exe" /S"#,
            worker::OUTSIDE_ADMIN,
            vec![pf(r"C:\Users\me\AppData\Local\Temp\uninst.exe")],
        ),
        // Relative (would be looked up on PATH).
        (
            "uninst.exe /S",
            worker::OUTSIDE_ADMIN,
            vec![pf("uninst.exe")],
        ),
        // Open quote.
        (
            r#""C:\PF\A\u.exe"#,
            worker::OUTSIDE_ADMIN,
            vec![pf(r"C:\PF\A\u.exe")],
        ),
        // Not an .exe, in an admin folder and existing.
        (
            r"C:\PF\A\u.bat",
            worker::OUTSIDE_ADMIN,
            vec![pf(r"C:\PF\A\u.bat")],
        ),
        // A shell in an admin folder.
        (
            r"C:\W\System32\cmd.exe /c C:\Users\me\x.bat",
            worker::INTERPRETER,
            vec![pf(r"C:\W\System32\cmd.exe")],
        ),
        (
            r#""%ProgramFiles%\PowerShell\7\pwsh.exe" -File x.ps1"#,
            worker::INTERPRETER,
            vec![pf(r"C:\PF\PowerShell\7\pwsh.exe")],
        ),
    ];
    for (cmd, want, files) in cases {
        let mut ops = FakeOps {
            entries: vec![quiet(Hive::MachineX64, "app", cmd)],
            files,
            ..FakeOps::default()
        };
        let reply = ask(
            &uninstall(MachineHive::Hklm64, "app", Rule::Quiet),
            &mut ops,
        );
        assert_eq!(error_text(&reply), want, "{cmd}");
        assert!(ops.launched.is_empty(), "{cmd}");
    }
}

#[test]
fn missing_uninstaller_is_not_found() {
    let mut ops = FakeOps {
        entries: vec![quiet(Hive::MachineX64, "app", r#""C:\PF\A\u.exe" /S"#)],
        ..FakeOps::default()
    };
    let reply = ask(
        &uninstall(MachineHive::Hklm64, "app", Rule::Quiet),
        &mut ops,
    );
    assert!(
        matches!(
            reply,
            Reply::Error {
                not_found: true,
                ..
            }
        ),
        "{reply:?}"
    );
    assert!(ops.launched.is_empty());
}

/// Live: the real helper (one UAC prompt) deletes a test key the user created beforehand in an
/// elevated terminal (see the Task 182 report).
#[cfg(windows)]
#[test]
#[ignore = "live: pede UAC; cria e apaga uma chave de teste no HKLM"]
fn live_delete_test_key() {
    use wingetrs::bridge::elevated::{ElevatedBridge, ElevatedRunner, SystemTransport};
    use wingetrs::winget_install::arp_read_win::read_details;
    let key = "wingetrs-test-orphan";
    assert!(
        read_details(Hive::MachineX86, key).unwrap().is_some(),
        "crie a chave de teste antes (roteiro no relatório da Task 182)"
    );
    let transport = SystemTransport::new(PathBuf::from(env!("CARGO_BIN_EXE_wingetrs")));
    let bridge = ElevatedBridge::new(Box::new(transport));
    let r = bridge.delete_arp_key(MachineHive::Hklm32, key, DeleteReason::Orphan);
    assert!(bridge.shutdown(std::time::Duration::from_secs(5)));
    assert_eq!(r, Ok(()));
    assert!(read_details(Hive::MachineX86, key).unwrap().is_none());
}
