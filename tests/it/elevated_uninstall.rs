#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Elevated uninstall and key deletion (Task 182).
use std::io::BufReader;
use std::path::{Path, PathBuf};

use wingetrs::bridge::worker::{
    self, ArpDelete, DeleteCheck, DeleteReason, MachineHive, Reply, Request, WorkerOps,
    msiexec_uninstall_args,
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
        user_profiles: PathBuf::from(r"C:\Users"),
    }
}

/// Owner SYSTEM, full access for SYSTEM and Administrators only (`sddl_admin_only` passes).
const ADMIN_SDDL: &str = "O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

/// Every folder is administrator-only (the DACL injection point of `still_orphan`).
fn admin_sddl(_: &Path) -> Option<String> {
    Some(ADMIN_SDDL.to_string())
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
    /// `also_checked` of each launch.
    held: Vec<Vec<PathBuf>>,
    /// 8.3 name -> long name; other paths are their own long name when they exist.
    longs: Vec<(PathBuf, PathBuf)>,
    deleted: Vec<(MachineHive, String)>,
    /// Progress lines the fake uninstaller run reports.
    lines: Vec<String>,
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

    fn long_path(&self, path: &Path) -> std::io::Result<PathBuf> {
        if let Some((_, long)) = self.longs.iter().find(|(short, _)| short == path) {
            return Ok(long.clone());
        }
        if self.files.iter().any(|f| f == path) {
            return Ok(path.to_path_buf());
        }
        Err(std::io::ErrorKind::NotFound.into())
    }

    fn run_uninstaller(
        &mut self,
        program: &Path,
        args: &str,
        also_checked: &[PathBuf],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<i32> {
        self.lines.iter().for_each(|l| on_line(l));
        self.held.push(also_checked.to_vec());
        self.launched
            .push((program.to_path_buf(), args.to_string()));
        Ok(7)
    }

    /// Reads the entry with the crate's own view mapping (`Hive::from`), as the real one does.
    fn delete_arp_key(
        &mut self,
        hive: MachineHive,
        key: &str,
        confirm: DeleteCheck<'_>,
    ) -> std::io::Result<ArpDelete> {
        self.reads += 1;
        let view = Hive::from(hive);
        let Some(d) = self
            .entries
            .iter()
            .find(|e| e.hive == view && e.key == key)
            .cloned()
        else {
            return Ok(ArpDelete::Missing);
        };
        let files = self.files.clone();
        if !confirm(&d, &|p: &Path| files.iter().any(|f| f == p), &admin_sddl) {
            return Ok(ArpDelete::Kept);
        }
        self.deleted.push((hive, key.to_string()));
        Ok(ArpDelete::Deleted)
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
    assert!(still_orphan(&d, &k, |_| false, admin_sddl));
    assert!(!still_orphan(
        &d,
        &k,
        |p| p == Path::new(r"C:\PF\gone\u.exe"),
        admin_sddl
    ));
    let user_var = ArpDetails {
        uninstall_string: Some(r"%LOCALAPPDATA%\gone\u.exe".into()),
        ..entry(Hive::MachineX86, "gone")
    };
    assert!(
        !still_orphan(&user_var, &k, |_| false, admin_sddl),
        "unknown variable"
    );
    assert!(!still_orphan(
        &entry(Hive::MachineX86, "x"),
        &k,
        |_| false,
        admin_sddl
    ));

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

/// Task 190: the helper's progress lines (waiting for leftover processes) reach the client
/// before the exit code.
#[test]
fn run_uninstaller_relays_progress_lines() {
    let mut ops = FakeOps {
        entries: vec![quiet(Hive::MachineX86, "app", r#""C:\PF\A\u.exe" /S"#)],
        files: vec![PathBuf::from(r"C:\PF\A\u.exe")],
        lines: vec!["aguardando o desinstalador terminar".into()],
        ..FakeOps::default()
    };
    let req = uninstall(MachineHive::Hklm32, "app", Rule::Quiet);
    let (r, replies) = serve_lines(&worker::encode(&req).unwrap(), &mut ops);
    r.unwrap();
    assert_eq!(
        replies,
        vec![
            Reply::Line {
                text: "aguardando o desinstalador terminar".into()
            },
            Reply::End { code: 7 }
        ]
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

/// Task 190: a missing uninstaller is an orphan only when the nearest existing folder above it
/// is administrator-only; a folder another SID may write (or an unreadable DACL) is not.
#[test]
fn orphan_in_user_folder_is_not_orphan() {
    let k = kf();
    let d = ArpDetails {
        uninstall_string: Some(r#""%ProgramFiles%\gone\sub\u.exe" /S"#.into()),
        ..entry(Hive::MachineX86, "gone")
    };
    let user_writable = "O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;BU)";
    // `C:\PF` exists, `C:\PF\gone` does not: `C:\PF` is the folder checked.
    let exists = |p: &Path| p == Path::new(r"C:\PF") || p == Path::new(r"C:\");
    let seen = std::cell::RefCell::new(Vec::new());
    let only_pf_admin = |p: &Path| {
        seen.borrow_mut().push(p.to_path_buf());
        let s = if p == Path::new(r"C:\PF") {
            ADMIN_SDDL
        } else {
            user_writable
        };
        Some(s.to_string())
    };
    assert!(still_orphan(&d, &k, exists, only_pf_admin));
    assert_eq!(*seen.borrow(), [PathBuf::from(r"C:\PF")]);
    let pf_user = |p: &Path| {
        let s = if p == Path::new(r"C:\PF") {
            user_writable
        } else {
            ADMIN_SDDL
        };
        Some(s.to_string())
    };
    assert!(
        !still_orphan(&d, &k, exists, pf_user),
        "user-writable folder"
    );
    assert!(!still_orphan(&d, &k, exists, |_| None), "unreadable DACL");
    // The nearest existing folder decides: with `C:\PF\gone` present, its DACL counts.
    let gone_exists = |p: &Path| p == Path::new(r"C:\PF\gone") || p == Path::new(r"C:\PF");
    assert!(!still_orphan(&d, &k, gone_exists, only_pf_admin));
}

/// Real descriptors read on this machine (Task 182 review), plus synthetic edge cases.
mod acl {
    use AclLevel::{Ancestor, File, Parent};
    use wingetrs::winget_install::elevated_rules::{AclLevel, sddl_admin_only};

    const TI: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

    fn real_root() -> String {
        format!(
            "O:{TI}G:{TI}D:(A;;LC;;;AU)(A;OICIIO;SDGXGWGR;;;AU)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)(A;;0x1000a1;;;S-1-15-3-65536-1888954469-739942743-1668119174-2468466756-4239452838-1296943325-355587736-700089176)"
        )
    }

    fn real_program_files() -> String {
        format!(
            "O:{TI}G:{TI}D:PAI(A;OICIIO;GA;;;CO)(A;OICIIO;GA;;;SY)(A;;0x1301bf;;;SY)(A;OICIIO;GA;;;BA)(A;;0x1301bf;;;BA)(A;OICIIO;GXGR;;;BU)(A;;0x1200a9;;;BU)(A;CIIO;GA;;;{TI})(A;;FA;;;{TI})(A;;0x1200a9;;;AC)(A;OICIIO;GXGR;;;AC)(A;;0x1200a9;;;S-1-15-2-2)(A;OICIIO;GXGR;;;S-1-15-2-2)"
        )
    }

    const REAL_UNINSTALL_EXE: &str = "O:BAG:S-1-5-21-375664104-1787174015-2556015593-1001D:AI(A;ID;FA;;;SY)(A;ID;FA;;;BA)(A;ID;0x1200a9;;;BU)(A;ID;0x1200a9;;;AC)(A;ID;0x1200a9;;;S-1-15-2-2)";
    const REAL_CRMLOG: &str = "O:BAG:SYD:PAI(A;OICI;0x1f01df;;;SY)(A;OICI;0x1f01df;;;BA)(A;;0x12008b;;;BU)(A;OIIO;SDGWGR;;;BU)";
    const REAL_PROGRAMDATA: &str = "O:SYG:SYD:PAI(A;OICIIO;GA;;;CO)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)(A;CI;DCLCRPCR;;;BU)";

    #[test]
    fn real_descriptors() {
        assert!(sddl_admin_only(&real_root(), Ancestor));
        assert!(!sddl_admin_only(&real_root(), Parent), "AU may add folders");
        assert!(sddl_admin_only(&real_program_files(), Parent));
        assert!(sddl_admin_only(&real_program_files(), Ancestor));
        assert!(sddl_admin_only(REAL_UNINSTALL_EXE, File));
        assert!(!sddl_admin_only(REAL_CRMLOG, Parent), "BU may write");
        assert!(sddl_admin_only(REAL_PROGRAMDATA, Ancestor));
        assert!(
            !sddl_admin_only(REAL_PROGRAMDATA, Parent),
            "BU may add files"
        );
    }

    #[test]
    fn owner_must_be_trusted() {
        for owner in ["SY", "BA", "S-1-5-18", "S-1-5-32-544", TI] {
            assert!(
                sddl_admin_only(&format!("O:{owner}D:P(A;;FA;;;SY)"), File),
                "{owner}"
            );
        }
        for owner in ["S-1-5-21-1-2-3-1001", "BU", "WD", "AU"] {
            assert!(
                !sddl_admin_only(&format!("O:{owner}D:P(A;;FA;;;SY)"), File),
                "{owner}"
            );
        }
    }

    #[test]
    fn rights_per_level() {
        let one =
            |ace: &str, level| sddl_admin_only(&format!("O:SYG:SYD:P(A;;FA;;;SY){ace}"), level);
        // Write data / append: file and parent, not above.
        for r in ["DC", "LC", "0x2", "0x4", "GW", "FW"] {
            let ace = format!("(A;;{r};;;BU)");
            assert!(!one(&ace, File), "{r}");
            assert!(!one(&ace, Parent), "{r}");
            assert!(one(&ace, Ancestor), "{r}");
        }
        // Delete child: parent and above; meaningless on a file.
        assert!(one("(A;;DT;;;BU)", File));
        assert!(!one("(A;;DT;;;BU)", Parent));
        assert!(!one("(A;;0x40;;;BU)", Ancestor));
        // Delete, re-ACL, take ownership, everything: refused at every level.
        for r in [
            "SD",
            "WD",
            "WO",
            "GA",
            "FA",
            "0x10000",
            "0x40000",
            "0x80000",
            "0x10000000",
        ] {
            let ace = format!("(A;;{r};;;S-1-5-21-1-2-3-1001)");
            for level in [File, Parent, Ancestor] {
                assert!(!one(&ace, level), "{r} {level:?}");
            }
        }
        // Read/execute for anyone is fine.
        for level in [File, Parent, Ancestor] {
            assert!(one("(A;;0x1200a9;;;WD)", level));
            assert!(one("(A;;FA;;;BA)", level));
            assert!(one(&format!("(A;;FA;;;{TI})"), level));
            assert!(one("(A;;FA;;;OW)", level));
        }
    }

    #[test]
    fn ace_forms() {
        let one = |aces: &str| sddl_admin_only(&format!("O:SYD:{aces}"), Parent);
        assert!(
            one("(A;OICIIO;GA;;;BU)"),
            "inherit-only never reaches the object"
        );
        assert!(!one("(A;CIOI;GA;;;BU)"), "CI+OI is not IO");
        assert!(!one("(A;OICI;GA;;;BU)"));
        assert!(one("(D;;FA;;;BU)"), "a deny ACE grants nothing");
        assert!(!one("(XA;;FA;;;BU)"));
        assert!(!one("(A;;KA;;;BU)"), "unknown right code");
        assert!(!one("(AU;SA;FA;;;BU)"), "unknown ACE kind in a DACL");
        assert!(!one("(A;;FA;;;BU"), "unclosed");
        assert!(!one("(A;;FA)"), "too few fields");
        assert!(one("P"), "an empty DACL grants nothing");
        assert!(
            !sddl_admin_only("O:SYD:NO_ACCESS_CONTROL", Parent),
            "null DACL"
        );
        assert!(!sddl_admin_only("O:SYG:SY", Parent), "no DACL");
        assert!(!sddl_admin_only("D:P(A;;FA;;;SY)", Parent), "no owner");
    }
}

#[test]
fn trailing_dot_or_space_component_refused() {
    let k = kf();
    for p in [
        r"C:\W\Temp.\x.exe",
        r"C:\W\Temp \x.exe",
        r"C:\PF\A.\u.exe",
        r"C:\PF\A \u.exe",
        r"C:\PF\A\u.exe.",
    ] {
        assert!(!in_admin_folder(Path::new(p), &k), "{p}");
    }
    assert!(in_admin_folder(Path::new(r"C:\PF\A.B\u.exe"), &k));
}

#[test]
fn arg_paths_found() {
    use wingetrs::winget_install::elevated_rules::arg_paths;
    assert_eq!(
        arg_paths(r#"/S _?=C:\PF\A /log="C:\Users\me\x y.log" \\srv\s\x /p:D:/t x86:\no"#),
        vec![
            r"C:\PF\A".to_string(),
            r"C:\Users\me\x y.log".to_string(),
            r"\\srv\s\x".to_string(),
            "D:/t".to_string(),
        ]
    );
    assert!(arg_paths("/S /quiet --mode=unattended").is_empty());
}

#[test]
fn absolute_paths_in_args_must_be_admin_folders() {
    let run = |args: &str| {
        let mut ops = FakeOps {
            entries: vec![quiet(
                Hive::MachineX64,
                "app",
                &format!(r#""C:\PF\A\u.exe" {args}"#),
            )],
            files: vec![PathBuf::from(r"C:\PF\A\u.exe")],
            ..FakeOps::default()
        };
        let reply = ask(
            &uninstall(MachineHive::Hklm64, "app", Rule::Quiet),
            &mut ops,
        );
        (reply, ops.launched.len())
    };
    assert_eq!(run(r"/S _?=C:\PF\A"), (Reply::End { code: 7 }, 1));
    for bad in [
        r#"/log="C:\Users\me\x.log""#,
        r"/config=C:\Users\me\AppData\Local\Temp\evil.xml",
        r"\\srv\share\x.dll",
        r"/d=C:\PF\..\Users\x",
    ] {
        let (reply, launched) = run(bad);
        assert_eq!(error_text(&reply), worker::ARG_OUTSIDE_ADMIN, "{bad}");
        assert_eq!(launched, 0, "{bad}");
    }
}

#[test]
fn every_interpreter_refused() {
    for name in worker::INTERPRETERS {
        let program = format!(r"C:\PF\Tools\{name}");
        let mut ops = FakeOps {
            entries: vec![quiet(Hive::MachineX64, "app", &format!("{program} /x"))],
            files: vec![PathBuf::from(&program)],
            ..FakeOps::default()
        };
        let reply = ask(
            &uninstall(MachineHive::Hklm64, "app", Rule::Quiet),
            &mut ops,
        );
        assert_eq!(error_text(&reply), worker::INTERPRETER, "{name}");
        assert!(ops.launched.is_empty(), "{name}");
    }
    for name in [
        "msbuild.exe",
        "installutil.exe",
        "regasm.exe",
        "regsvcs.exe",
        "cmstp.exe",
        "odbcconf.exe",
        "forfiles.exe",
        "wmic.exe",
        "msdt.exe",
        "wsl.exe",
        "bash.exe",
        "explorer.exe",
        "mshta.exe",
        "wscript.exe",
        "cscript.exe",
    ] {
        assert!(worker::INTERPRETERS.contains(&name), "{name}");
    }
}

/// Task 182 review: Inno name-list switches of real winget manifests (Task 180 allowlist).
#[test]
fn inno_name_list_switches() {
    use wingetrs::bridge::worker::{ElevatedKind::Inno, exe_params_allowed};
    let roots = [r"C:\Program Files".to_string()];
    for ok in [
        "/VERYSILENT /MERGETASKS=!runcode,addcontextmenufiles",
        "/tasks=desktopicon /COMPONENTS=main,help_files /TYPE=full /LANG=pt-BR",
        "/SILENT /NOICONS /NORESTARTAPPLICATIONS",
        "/RESTARTAPPLICATIONS",
    ] {
        assert!(exe_params_allowed(Inno, ok, &roots), "{ok}");
    }
    for bad in [
        r"/TASKS=..\x",
        "/TASKS=C:x",
        "/TASKS=",
        "/MERGETASKS=a/b",
        r#"/TASKS="a,b""#,
        "/LANG=a.b",
        "/TYPE=a b",
        "/TASKSX=a",
    ] {
        assert!(!exe_params_allowed(Inno, bad, &roots), "{bad}");
    }
}
/// Runs one Quiet entry through the plan with `files` existing and `longs` as the 8.3 map.
fn plan_quiet(cmd: &str, files: &[&str], longs: &[(&str, &str)]) -> (Reply, FakeOps) {
    let mut ops = FakeOps {
        entries: vec![quiet(Hive::MachineX86, "app", cmd)],
        files: files.iter().map(PathBuf::from).collect(),
        longs: longs
            .iter()
            .map(|(a, b)| (PathBuf::from(a), PathBuf::from(b)))
            .collect(),
        ..FakeOps::default()
    };
    let reply = ask(
        &uninstall(MachineHive::Hklm32, "app", Rule::Quiet),
        &mut ops,
    );
    (reply, ops)
}

/// Review 2 of Task 182 (1): outside the known folders the real owner/DACL check decides; only
/// the always user-writable places are refused up front.
#[test]
fn unlisted_location_goes_to_the_acl_check() {
    let (reply, ops) = plan_quiet(
        r#""C:\Ruby34-x64\unins000.exe" /SILENT"#,
        &[r"C:\Ruby34-x64\unins000.exe"],
        &[],
    );
    assert_eq!(reply, Reply::End { code: 7 });
    assert_eq!(
        ops.launched,
        vec![(
            PathBuf::from(r"C:\Ruby34-x64\unins000.exe"),
            "/SILENT".to_string()
        )]
    );
    for refused in [
        r"C:\Users\me\AppData\Local\nvm\unins000.exe",
        r"C:\Users\Public\x\unins000.exe",
        r"C:\W\Temp\x\u.exe",
        r"C:\W\Registration\CRMLog\u.exe",
        r"C:\W\System32\spool\drivers\color\u.exe",
        r"D:\x\..\u.exe",
    ] {
        let (reply, ops) = plan_quiet(&format!(r#""{refused}" /S"#), &[refused], &[]);
        assert_eq!(error_text(&reply), worker::OUTSIDE_ADMIN, "{refused}");
        assert!(ops.launched.is_empty(), "{refused}");
    }
}

#[test]
fn refused_location_rule() {
    use wingetrs::winget_install::elevated_rules::refused_location;
    let k = kf();
    let no = |p: &str| refused_location(Path::new(p), &k);
    assert!(!no(r"C:\Ruby34-x64\unins000.exe"));
    assert!(!no(r"D:\Apps\u.exe"));
    assert!(!no(r"C:\W\System32\spool\DRIVERS\x64\3\E.EXE"));
    assert!(no(r"C:\Users\me\u.exe"));
    assert!(no(r"c:\users\me\u.exe"));
    assert!(no(r"\\srv\share\u.exe"));
    assert!(no(r"\PF\u.exe"));
    assert!(no(r"C:\Apps.\u.exe"));
    assert!(no(r"C:\W\Tasks\u.exe"));
    assert!(no(r"C:\W\Registration\CRMLog\u.exe"));
}

/// (2) 8.3 names are expanded before the rule; a short name that does not exist is refused.
#[test]
fn short_names_are_expanded() {
    let short = r"C:\PROGRA~1\DIFX\B60D12~1\dpinst64.exe";
    let long = r"C:\PF\DIFX\B60D12D\dpinst64.exe";
    let args = r"/u C:\W\System32\DriverStore\FileRepository\a.inf";
    let (reply, ops) = plan_quiet(&format!("{short} {args}"), &[long], &[(short, long)]);
    assert_eq!(reply, Reply::End { code: 7 });
    assert_eq!(ops.launched, vec![(PathBuf::from(long), args.to_string())]);

    let (reply, ops) = plan_quiet(
        r"C:\PROGRA~2\x.exe /S",
        &[r"C:\Users\me\x.exe"],
        &[(r"C:\PROGRA~2\x.exe", r"C:\Users\me\x.exe")],
    );
    assert_eq!(
        error_text(&reply),
        worker::OUTSIDE_ADMIN,
        "expands into a user folder"
    );
    assert!(ops.launched.is_empty());

    let (reply, ops) = plan_quiet(r"C:\PROGRA~3\gone.exe /S", &[], &[]);
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

/// (3) Only `spool\drivers\color` is user-writable under `spool`.
#[test]
fn spool_driver_folder_accepted() {
    let exe = r"C:\W\System32\spool\DRIVERS\x64\3\E_YINSYWE.EXE";
    let (reply, ops) = plan_quiet(
        &format!(r#"{exe} /R /APD /P:"EPSON L3250 Series""#),
        &[exe],
        &[],
    );
    assert_eq!(reply, Reply::End { code: 7 });
    assert_eq!(ops.launched.len(), 1);
    assert!(in_admin_folder(Path::new(exe), &kf()));
}

/// (4) The system `rundll32` only as `"<dll>",<Export> [args]`, the DLL checked like a program.
#[test]
fn rundll32_only_with_a_checked_dll() {
    let rundll = r"C:\W\SysWOW64\RunDll32.EXE";
    let dll = r"C:\PF\NVIDIA\NVI2.DLL";
    let (reply, ops) = plan_quiet(
        &format!(r#"{rundll} "{dll}",UninstallPackage Display.Driver"#),
        &[rundll, dll],
        &[],
    );
    assert_eq!(reply, Reply::End { code: 7 });
    assert_eq!(
        ops.launched,
        vec![(
            PathBuf::from(rundll),
            format!(r#""{dll}",UninstallPackage Display.Driver"#)
        )]
    );
    assert_eq!(ops.held, vec![vec![PathBuf::from(dll)]]);

    let system32 = r"C:\W\System32\rundll32.exe";
    let (reply, _) = plan_quiet(&format!(r#"{system32} "{dll}",Go"#), &[system32, dll], &[]);
    assert_eq!(reply, Reply::End { code: 7 });

    for (args, want) in [
        (format!("{dll},Go"), worker::RUNDLL32_REFUSED),
        (format!(r#""{dll}" Go"#), worker::RUNDLL32_REFUSED),
        (format!(r#""{dll}","#), worker::RUNDLL32_REFUSED),
        (format!(r#""{dll}",a;b"#), worker::RUNDLL32_REFUSED),
        (
            r#""C:\PF\NVIDIA\x.exe",Go"#.to_string(),
            worker::RUNDLL32_REFUSED,
        ),
        (
            r#""C:\Users\me\x.dll",Go"#.to_string(),
            worker::RUNDLL32_REFUSED,
        ),
        (r#""%TEMP%\x.dll",Go"#.to_string(), worker::RUNDLL32_REFUSED),
        (
            format!(r#""{dll}",Go C:\Users\me\y.txt"#),
            worker::ARG_OUTSIDE_ADMIN,
        ),
    ] {
        let (reply, ops) = plan_quiet(
            &format!("{rundll} {args}"),
            &[rundll, dll, r"C:\PF\NVIDIA\x.exe", r"C:\Users\me\x.dll"],
            &[],
        );
        assert_eq!(error_text(&reply), want, "{args}");
        assert!(ops.launched.is_empty(), "{args}");
    }

    // A DLL that does not exist.
    let (reply, ops) = plan_quiet(
        &format!(r#"{rundll} "C:\PF\N\gone.dll",Go"#),
        &[rundll],
        &[],
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

    // Any other rundll32 is an interpreter.
    let other = r"C:\PF\Tools\rundll32.exe";
    let (reply, ops) = plan_quiet(&format!(r#"{other} "{dll}",Go"#), &[other, dll], &[]);
    assert_eq!(error_text(&reply), worker::INTERPRETER);
    assert!(ops.launched.is_empty());
}

#[test]
fn parse_rundll32_forms() {
    use wingetrs::winget_install::elevated_rules::parse_rundll32;
    assert_eq!(
        parse_rundll32(r#" "C:\a\b.dll",Run_1 x y "#),
        Some((
            r"C:\a\b.dll".to_string(),
            "Run_1".to_string(),
            "x y".to_string()
        ))
    );
    assert_eq!(
        parse_rundll32(r#""C:\a\b.DLL",Go"#),
        Some((r"C:\a\b.DLL".to_string(), "Go".to_string(), String::new()))
    );
    for bad in [
        r"C:\a\b.dll,Go",
        r#""C:\a\b.dll Go"#,
        r#""",Go"#,
        r#""C:\a\b.dll",-Go"#,
    ] {
        assert_eq!(parse_rundll32(bad), None, "{bad}");
    }
}
/// Security review of the rundll32 rule: system DLLs export generic launchers, so the DLL must
/// be an application's own, under Program Files; and the rest names no interpreter.
#[test]
fn rundll32_lolbins_refused() {
    let rundll = r"C:\W\System32\rundll32.exe";
    for (dll, rest) in [
        (
            r"C:\W\System32\shell32.dll",
            "ShellExec_RunDLL cmd.exe /c whoami",
        ),
        (r"C:\W\System32\url.dll", r"FileProtocolHandler C:\PF\x.exe"),
        (
            r"C:\W\System32\advpack.dll",
            "LaunchINFSection x.inf,DefaultInstall",
        ),
        (
            r"C:\W\System32\setupapi.dll",
            "InstallHinfSection DefaultInstall 128 x.inf",
        ),
        (r"C:\W\System32\zipfldr.dll", r"RouteTheCall C:\PF\x.exe"),
        (r"C:\W\SysWOW64\ieframe.dll", r"OpenURL C:\PF\x.url"),
        (
            r"C:\W\System32\pcwutl.dll",
            r"LaunchApplication C:\PF\x.exe",
        ),
        (r"%SystemRoot%\System32\shell32.dll", "Control_RunDLL"),
        (r"C:\PD\Package Cache\{g}\x.dll", "Go"),
        (r"D:\Apps\x.dll", "Go"),
    ] {
        let line = format!(r#"{rundll} "{dll}",{rest}"#);
        let expanded = dll.replace("%SystemRoot%", r"C:\W");
        let (reply, ops) = plan_quiet(&line, &[rundll, expanded.as_str()], &[]);
        assert_eq!(error_text(&reply), worker::RUNDLL32_REFUSED, "{line}");
        assert!(ops.launched.is_empty(), "{line}");
    }
    let dll = r"C:\PF\App\x.dll";
    for rest in [
        "Go cmd /c whoami",
        "Go cmd.exe",
        r"Go C:\W\System32\WindowsPowerShell\v1.0\powershell.exe -c x",
        r#"Go "C:\W\System32\mshta""#,
        "Go x,wscript",
        "Go MSBuild.EXE x.proj",
    ] {
        let line = format!(r#"{rundll} "{dll}",{rest}"#);
        let (reply, ops) = plan_quiet(&line, &[rundll, dll], &[]);
        assert_eq!(error_text(&reply), worker::INTERPRETER, "{line}");
        assert!(ops.launched.is_empty(), "{line}");
    }
    // NVIDIA's real line is still accepted.
    let nvidia = r"C:\PF\NVIDIA Corporation\Installer2\InstallerCore\NVI2.DLL";
    let line = format!(r#"C:\W\SysWOW64\RunDll32.EXE "{nvidia}",UninstallPackage Display.Driver"#);
    let (reply, ops) = plan_quiet(&line, &[r"C:\W\SysWOW64\RunDll32.EXE", nvidia], &[]);
    assert_eq!(reply, Reply::End { code: 7 });
    assert_eq!(ops.held, vec![vec![PathBuf::from(nvidia)]]);
}

#[test]
fn names_interpreter_tokens() {
    use wingetrs::winget_install::elevated_rules::names_interpreter;
    let list = worker::INTERPRETERS;
    assert!(names_interpreter("x CMD", &list));
    assert!(names_interpreter(r#""C:\W\cmd.exe" /c"#, &list));
    assert!(names_interpreter("a,bash", &list));
    assert!(!names_interpreter(
        "Display.Driver /quiet cmdline.txt",
        &list
    ));
    assert!(!names_interpreter("", &list));
}

/// Real descriptors that the first measurement refused wrongly.
#[test]
fn acl_trustees_no_user_holds() {
    use wingetrs::winget_install::elevated_rules::{AclLevel, sddl_admin_only};
    // C:\Program Files\DIFX: CREATOR OWNER outside an inherit-only ACE matches nobody.
    let difx = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;CO)(A;OICI;0x1200a9;;;PU)(A;OICI;0x1200a9;;;BU)";
    assert!(sddl_admin_only(difx, AclLevel::Ancestor));
    assert!(sddl_admin_only(difx, AclLevel::Parent));
    // spool driver: full control for the print spooler's restricted-service SID.
    let spool = "O:SYD:AI(A;ID;FA;;;SY)(A;ID;FA;;;BA)(A;ID;0x1200a9;;;WD)(A;ID;0x1200a9;;;AC)(A;ID;FA;;;S-1-5-99-216390572-1995538116-3857911515-2404958512-2623887229)";
    assert!(sddl_admin_only(spool, AclLevel::File));
    assert!(sddl_admin_only(
        "O:SYD:(A;;FA;;;S-1-5-80-1-2-3-4-5)",
        AclLevel::File
    ));
    // But a user, a user group or CO as owner still refuse.
    assert!(!sddl_admin_only(
        "O:SYD:(A;;FA;;;S-1-5-21-1-2-3-1001)",
        AclLevel::File
    ));
    assert!(!sddl_admin_only("O:COD:(A;;FA;;;SY)", AclLevel::File));
    assert!(!sddl_admin_only(
        "O:S-1-5-80-1-2D:(A;;FA;;;SY)",
        AclLevel::File
    ));
    // A callback ACE with a parenthesised condition is parsed whole.
    let read_only = r#"O:SYD:AI(XA;OICIID;0x1200a9;;;BU;(WIN://SYSAPPID Contains "Microsoft.X_8wekyb3d8bbwe"))(A;ID;FA;;;SY)"#;
    assert!(sddl_admin_only(read_only, AclLevel::Parent));
    let full = r#"O:SYD:AI(XA;OICIID;FA;;;BU;(WIN://SYSAPPID Contains "x"))(A;ID;FA;;;SY)"#;
    assert!(!sddl_admin_only(full, AclLevel::Parent));
}

/// Task 190: an uninstaller that starts a copy of itself and exits is waited for through the
/// job (up to a deadline): the execution folder goes only after the copy ended; a copy that
/// outlives the deadline is left running and so is the folder.
#[cfg(windows)]
mod job {
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tempfile::TempDir;
    use wingetrs::bridge::arp_ops_win::{WAITING_UNINSTALLER, run_waiting};
    use wingetrs::bridge::staging_win::SystemEnv;

    /// `RUN_SDDL` plus the owner (the unelevated test user), who may then write inside.
    const TEST_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)";

    /// `uninst.cmd` (exits 3 at once) starting `child.cmd`, which waits for `release` to exist
    /// (at most ~`pings` s), then records whether its `TEMP` still exists in `marker` and exits 5.
    fn scripts(dir: &Path, release: &Path, marker: &Path, pings: u32) -> PathBuf {
        let child = dir.join("child.cmd");
        std::fs::write(
            &child,
            format!(
                "@echo off\r\nfor /l %%i in (1,1,{pings}) do (\r\n\
                 if exist \"{r}\" goto done\r\n\
                 ping -n 2 127.0.0.1 >nul\r\n)\r\n:done\r\n\
                 if exist \"%TEMP%\\\" (echo tmp> \"{m}\") else (echo sem-tmp> \"{m}\")\r\n\
                 exit /b 5\r\n",
                r = release.display(),
                m = marker.display()
            ),
        )
        .unwrap();
        let parent = dir.join("uninst.cmd");
        std::fs::write(
            &parent,
            format!(
                "@echo off\r\nstart \"\" /b cmd /d /c \"{}\"\r\nexit /b 3\r\n",
                child.display()
            ),
        )
        .unwrap();
        parent
    }

    fn run(
        dir: &Path,
        root: &Path,
        parent: &Path,
        linger: Duration,
        lines: &mut Vec<String>,
    ) -> i32 {
        let sys = SystemEnv::query().unwrap();
        let cmd = sys.system32.join("cmd.exe");
        let args = format!("/d /c \"{}\"", parent.display());
        run_waiting(
            &cmd,
            &args,
            dir,
            root,
            TEST_SDDL,
            &sys,
            linger,
            &mut |l: &str| lines.push(l.to_string()),
        )
        .unwrap()
    }

    #[test]
    fn job_waits_for_copied_uninstaller() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let marker = dir.path().join("child-done.txt");
        // Never released: the child sleeps ~1 s (one loop turn) and ends.
        let parent = scripts(dir.path(), &dir.path().join("never"), &marker, 1);
        let mut lines = Vec::new();
        let code = run(
            dir.path(),
            &root,
            &parent,
            Duration::from_secs(600),
            &mut lines,
        );
        let done = std::fs::read_to_string(&marker).unwrap_or_default();
        assert_eq!(done.trim(), "tmp", "the copy ended before the folder went");
        assert_eq!(code, 3, "the code of the program started");
        assert_eq!(lines, [WAITING_UNINSTALLER]);
        assert!(
            std::fs::read_dir(&root).unwrap().next().is_none(),
            "execution folder removed"
        );
    }

    /// Task 190: past the deadline the helper returns with the program's code, leaves the
    /// copy running (not killed) and keeps the execution folder for the 24 h prune.
    #[test]
    fn job_deadline_leaves_the_copy_and_keeps_the_folder() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let marker = dir.path().join("child-done.txt");
        let release = dir.path().join("release");
        // Up to ~60 s, released by the test once the helper returned.
        let parent = scripts(dir.path(), &release, &marker, 60);
        let mut lines = Vec::new();
        let code = run(
            dir.path(),
            &root,
            &parent,
            Duration::from_millis(300),
            &mut lines,
        );
        assert_eq!(code, 3);
        assert!(!marker.exists(), "returned while the copy still ran");
        let kept: Vec<_> = std::fs::read_dir(&root).unwrap().collect();
        assert_eq!(kept.len(), 1, "execution folder kept");
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0], WAITING_UNINSTALLER);
        assert!(lines[1].contains("limpeza de 24 h"), "{lines:?}");
        // Not killed: once released, the copy finishes and finds its TEMP still there.
        std::fs::write(&release, b"").unwrap();
        // Polled until complete (the child may still be writing it).
        let start = std::time::Instant::now();
        loop {
            let done = std::fs::read_to_string(&marker).unwrap_or_default();
            if done.trim() == "tmp" {
                break;
            }
            // Safety cap only (a broken child fails instead of hanging): clock-ok
            assert!(start.elapsed() < Duration::from_secs(120), "{done:?}"); // clock-ok
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
