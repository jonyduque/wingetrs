#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Orphan ARP entries (Task 183).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use wingetrs::bridge::elevated::{ElevatedError, ElevatedRunner};
use wingetrs::bridge::worker::{DeleteReason, MachineHive};
use wingetrs::installed::{Hive, InstallSource, InstalledApp};
use wingetrs::winget_install::orphans::{
    ArpRegistry, Orphan, backup_file, check_export, check_export_in, export_args, orphans,
    remove_orphan,
};

const HEADER: &str = "Windows Registry Editor Version 5.00";

fn utf16le(text: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

fn reg_text(root: &str, key: &str) -> String {
    format!(
        "{HEADER}\r\n\r\n[{root}\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{key}]\r\n\"DisplayName\"=\"X\"\r\n\r\n"
    )
}

struct FakeReg {
    /// Bytes written to the export file; `None` writes nothing.
    content: Option<Vec<u8>>,
    still: bool,
    exports: Mutex<Vec<Vec<String>>>,
    deletes: Mutex<Vec<String>>,
}

impl FakeReg {
    fn new(content: Option<Vec<u8>>) -> FakeReg {
        FakeReg {
            content,
            still: true,
            exports: Mutex::new(Vec::new()),
            deletes: Mutex::new(Vec::new()),
        }
    }
}

impl ArpRegistry for FakeReg {
    fn export(&self, args: &[String]) -> std::io::Result<()> {
        self.exports.lock().unwrap().push(args.to_vec());
        if let Some(c) = &self.content {
            std::fs::write(&args[2], c)?;
        }
        Ok(())
    }
    fn delete_user_key(&self, key: &str) -> std::io::Result<()> {
        self.deletes.lock().unwrap().push(key.to_string());
        Ok(())
    }
    fn still_orphan(&self, _hive: Hive, _key: &str) -> bool {
        self.still
    }
}

#[derive(Default)]
struct FakeElevated {
    deletes: Mutex<Vec<(MachineHive, String, DeleteReason)>>,
}

impl ElevatedRunner for FakeElevated {
    fn delete_arp_key(
        &self,
        hive: MachineHive,
        key: &str,
        reason: DeleteReason,
    ) -> Result<(), ElevatedError> {
        self.deletes
            .lock()
            .unwrap()
            .push((hive, key.to_string(), reason));
        Ok(())
    }
}

fn orphan(hive: Hive, key: &str) -> Orphan {
    Orphan {
        hive,
        key: key.to_string(),
        name: format!("{key} name"),
        missing: r"C:\gone\uninst.exe".to_string(),
    }
}

fn app(source: InstallSource, key: &str, stale: Option<&str>) -> InstalledApp {
    InstalledApp {
        source,
        key: key.to_string(),
        name: format!("{key} name"),
        publisher: None,
        version: None,
        product_code: None,
        upgrade_code: None,
        family_name: None,
        scoop_bucket: None,
        stale: stale.map(str::to_string),
    }
}

fn assert_nothing_deleted(reg: &FakeReg, el: &FakeElevated) {
    assert_eq!(reg.deletes.lock().unwrap().len(), 0);
    assert_eq!(el.deletes.lock().unwrap().len(), 0);
}

#[test]
fn empty_export_never_deletes() {
    let dir = tempfile::tempdir().unwrap();
    for hive in [Hive::User, Hive::MachineX64] {
        let reg = FakeReg::new(Some(Vec::new()));
        let el = FakeElevated::default();
        let r = remove_orphan(&reg, &el, dir.path(), "ts1", &orphan(hive, "App"));
        assert!(r.is_err(), "{r:?}");
        assert_eq!(reg.exports.lock().unwrap().len(), 1);
        assert_nothing_deleted(&reg, &el);
    }
}

#[test]
fn export_without_key_name_never_deletes() {
    let dir = tempfile::tempdir().unwrap();
    for (hive, root) in [
        (Hive::User, "HKEY_CURRENT_USER"),
        (Hive::MachineX86, "HKEY_LOCAL_MACHINE"),
    ] {
        let reg = FakeReg::new(Some(utf16le(&reg_text(root, "Outra"))));
        let el = FakeElevated::default();
        let r = remove_orphan(&reg, &el, dir.path(), "ts2", &orphan(hive, "App"));
        assert!(r.is_err(), "{r:?}");
        assert_nothing_deleted(&reg, &el);
    }
    // A subkey of the right key is not the key itself.
    let f = dir.path().join("sub.reg");
    std::fs::write(&f, utf16le(&reg_text("HKEY_CURRENT_USER", r"App\Sub"))).unwrap();
    assert!(check_export(&f, "App").is_err());
    // The same file names the key in other case: accepted.
    std::fs::write(&f, utf16le(&reg_text("HKEY_CURRENT_USER", "aPP"))).unwrap();
    assert_eq!(check_export(&f, "App"), Ok(()));
}

#[test]
fn export_without_header_never_deletes() {
    let dir = tempfile::tempdir().unwrap();
    let no_header = reg_text("HKEY_CURRENT_USER", "App").replacen(HEADER, "REGEDIT4", 1);
    let utf8 = reg_text("HKEY_CURRENT_USER", "App").into_bytes();
    for content in [utf16le(&no_header), utf8] {
        let reg = FakeReg::new(Some(content));
        let el = FakeElevated::default();
        let r = remove_orphan(&reg, &el, dir.path(), "ts3", &orphan(Hive::User, "App"));
        assert!(r.is_err(), "{r:?}");
        assert_nothing_deleted(&reg, &el);
    }
    // The header only counts as the first line.
    let f = dir.path().join("late.reg");
    let late = format!("\r\n{}", reg_text("HKEY_CURRENT_USER", "App"));
    std::fs::write(&f, utf16le(&late)).unwrap();
    assert!(check_export(&f, "App").is_err());
    // A file that was never written.
    assert!(check_export(&dir.path().join("missing.reg"), "App").is_err());
}

#[test]
fn hkcu_deleted_in_process_after_export() {
    let dir = tempfile::tempdir().unwrap();
    let reg = FakeReg::new(Some(utf16le(&reg_text("HKEY_CURRENT_USER", "App"))));
    let el = FakeElevated::default();
    let r = remove_orphan(&reg, &el, dir.path(), "ts4", &orphan(Hive::User, "App"));
    let want = dir.path().join("ts4").join("hkcu-App.reg");
    assert_eq!(r, Ok(want.clone()));
    assert!(want.is_file());
    assert_eq!(*reg.deletes.lock().unwrap(), vec!["App".to_string()]);
    assert_eq!(el.deletes.lock().unwrap().len(), 0);
}

#[test]
fn hklm_goes_to_helper_with_view() {
    let dir = tempfile::tempdir().unwrap();
    let el = FakeElevated::default();
    for (hive, key) in [(Hive::MachineX86, "k1"), (Hive::MachineX64, "k2")] {
        let reg = FakeReg::new(Some(utf16le(&reg_text("HKEY_LOCAL_MACHINE", key))));
        let r = remove_orphan(&reg, &el, dir.path(), "ts5", &orphan(hive, key));
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(reg.deletes.lock().unwrap().len(), 0);
    }
    assert_eq!(
        *el.deletes.lock().unwrap(),
        vec![
            (MachineHive::Hklm32, "k1".to_string(), DeleteReason::Orphan),
            (MachineHive::Hklm64, "k2".to_string(), DeleteReason::Orphan),
        ]
    );
}

#[test]
fn export_args_views() {
    let f = Path::new(r"C:\b\x.reg");
    let x86 = export_args(Hive::MachineX86, "K", f);
    assert!(x86.contains(&"/reg:32".to_string()), "{x86:?}");
    assert!(!x86.contains(&"/reg:64".to_string()), "{x86:?}");
    assert!(
        x86.contains(&r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\K".to_string()),
        "{x86:?}"
    );
    let x64 = export_args(Hive::MachineX64, "K", f);
    assert!(x64.contains(&"/reg:64".to_string()), "{x64:?}");
    assert!(!x64.contains(&"/reg:32".to_string()), "{x64:?}");
    let user = export_args(Hive::User, "K", f);
    assert_eq!(
        user,
        vec![
            "export".to_string(),
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\K".to_string(),
            r"C:\b\x.reg".to_string(),
            "/y".to_string(),
        ]
    );
}

#[test]
fn no_longer_orphan_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    for hive in [Hive::User, Hive::MachineX64] {
        let mut reg = FakeReg::new(Some(utf16le(&reg_text("HKEY_CURRENT_USER", "App"))));
        reg.still = false;
        let el = FakeElevated::default();
        let r = remove_orphan(&reg, &el, dir.path(), "ts7", &orphan(hive, "App"));
        let e = r.unwrap_err();
        assert!(e.contains("deixou de ser sobra"), "{e}");
        assert_eq!(reg.exports.lock().unwrap().len(), 0);
        assert_nothing_deleted(&reg, &el);
    }
}

#[test]
fn invalid_key_is_refused_before_anything() {
    let dir = tempfile::tempdir().unwrap();
    for key in [r"..\Evil", "", "a/b"] {
        let reg = FakeReg::new(Some(utf16le(&reg_text("HKEY_CURRENT_USER", key))));
        let el = FakeElevated::default();
        let r = remove_orphan(&reg, &el, dir.path(), "ts8", &orphan(Hive::User, key));
        assert!(r.is_err(), "{key}: {r:?}");
        assert_eq!(reg.exports.lock().unwrap().len(), 0);
        assert_nothing_deleted(&reg, &el);
    }
}

#[test]
fn existing_backup_is_never_reused() {
    // A stale valid file at the backup path must not stand in for an export that wrote nothing.
    let dir = tempfile::tempdir().unwrap();
    let o = orphan(Hive::User, "App");
    let old = backup_file(dir.path(), "ts9", &o);
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    std::fs::write(&old, utf16le(&reg_text("HKEY_CURRENT_USER", "App"))).unwrap();
    let reg = FakeReg::new(None);
    let el = FakeElevated::default();
    let r = remove_orphan(&reg, &el, dir.path(), "ts9", &o);
    assert!(r.is_err(), "{r:?}");
    assert_nothing_deleted(&reg, &el);
    // A real export next to it lands in a new file and leaves the old one alone.
    let reg = FakeReg::new(Some(utf16le(&reg_text("HKEY_CURRENT_USER", "App"))));
    let got = remove_orphan(&reg, &el, dir.path(), "ts9", &o).unwrap();
    assert_ne!(got, old);
    assert_eq!(got.parent(), old.parent());
    assert_eq!(*reg.deletes.lock().unwrap(), vec!["App".to_string()]);
}

#[test]
fn export_root_must_match_hive() {
    let dir = tempfile::tempdir().unwrap();
    // Root swapped: an HKCU orphan whose export names the HKLM key, and the reverse.
    for (hive, root) in [
        (Hive::User, "HKEY_LOCAL_MACHINE"),
        (Hive::MachineX64, "HKEY_CURRENT_USER"),
        (Hive::MachineX86, "HKEY_CURRENT_USER"),
    ] {
        let reg = FakeReg::new(Some(utf16le(&reg_text(root, "App"))));
        let el = FakeElevated::default();
        let r = remove_orphan(&reg, &el, dir.path(), "ts10", &orphan(hive, "App"));
        assert!(r.is_err(), "{hive:?} {root}: {r:?}");
        assert_nothing_deleted(&reg, &el);
    }
    let f = dir.path().join("root.reg");
    std::fs::write(&f, utf16le(&reg_text("HKEY_LOCAL_MACHINE", "App"))).unwrap();
    assert_eq!(check_export_in(&f, Hive::MachineX86, "App"), Ok(()));
    assert_eq!(check_export_in(&f, Hive::MachineX64, "App"), Ok(()));
    assert!(check_export_in(&f, Hive::User, "App").is_err());
    // A nested `...\Uninstall\X\Uninstall\App` is not the key.
    std::fs::write(
        &f,
        utf16le(&reg_text("HKEY_LOCAL_MACHINE", r"X\Uninstall\App")),
    )
    .unwrap();
    assert!(check_export(&f, "App").is_err());
    assert!(check_export_in(&f, Hive::MachineX64, "App").is_err());
    // Same root in other case: accepted.
    std::fs::write(&f, utf16le(&reg_text("hkey_local_machine", "app"))).unwrap();
    assert_eq!(check_export_in(&f, Hive::MachineX64, "App"), Ok(()));
}

#[test]
fn oversized_export_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("big.reg");
    // A valid export padded to 8 MiB + 1 byte: refused only for its size.
    let mut bytes = utf16le(&reg_text("HKEY_CURRENT_USER", "App"));
    let ok = bytes.clone();
    bytes.resize(8 * 1024 * 1024 + 1, 0);
    std::fs::write(&f, &bytes).unwrap();
    assert_eq!(
        check_export(&f, "App"),
        Err("backup grande demais".to_string())
    );
    let mut at_cap = ok;
    at_cap.resize(8 * 1024 * 1024, 0);
    std::fs::write(&f, &at_cap).unwrap();
    assert_eq!(check_export(&f, "App"), Ok(()));
}

#[test]
fn truncated_utf16_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("odd.reg");
    // A valid export plus one stray byte: refused only for the odd length.
    let mut bytes = utf16le(&reg_text("HKEY_CURRENT_USER", "App"));
    bytes.push(0);
    std::fs::write(&f, &bytes).unwrap();
    assert_eq!(
        check_export(&f, "App"),
        Err("backup UTF-16 truncado".to_string())
    );
    bytes.push(0);
    std::fs::write(&f, &bytes).unwrap();
    assert_eq!(check_export(&f, "App"), Ok(()));
}

#[test]
fn orphans_from_stale() {
    let apps = vec![
        app(
            InstallSource::Arp(Hive::MachineX64),
            "A",
            Some(r"C:\a\u.exe"),
        ),
        app(InstallSource::Arp(Hive::User), "B", Some(r"C:\b\u.exe")),
        app(InstallSource::Arp(Hive::MachineX86), "C", None),
        app(InstallSource::Msix, "D", Some(r"C:\d\u.exe")),
    ];
    let got = orphans(&apps);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!((got[0].hive, got[0].key.as_str()), (Hive::MachineX64, "A"));
    assert_eq!(got[0].missing, r"C:\a\u.exe");
    assert_eq!(got[0].name, "A name");
    assert_eq!((got[1].hive, got[1].key.as_str()), (Hive::User, "B"));
}

#[test]
fn backup_name_sanitized() {
    let b = Path::new(r"C:\data\backups");
    assert_eq!(
        backup_file(b, "ts", &orphan(Hive::MachineX64, "Steam App 255710")),
        PathBuf::from(r"C:\data\backups\ts\hklm64-Steam App 255710.reg")
    );
    assert_eq!(
        backup_file(b, "ts", &orphan(Hive::MachineX64, "a:b*c")),
        PathBuf::from(r"C:\data\backups\ts\hklm64-a_b_c.reg")
    );
    assert_eq!(
        backup_file(b, "ts", &orphan(Hive::MachineX86, "{AB-12}_x+y.z")),
        PathBuf::from(r"C:\data\backups\ts\hklm32-{AB-12}_x+y.z.reg")
    );
    assert_eq!(
        backup_file(b, "ts", &orphan(Hive::User, "Ünï")),
        PathBuf::from(r"C:\data\backups\ts\hkcu-_n_.reg")
    );
}

#[cfg(windows)]
#[test]
#[ignore = "live: lê o registro real"]
fn live_export_hklm_without_admin() {
    use wingetrs::winget_install::orphans_win::SystemArp;
    use winreg::RegKey;
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
    let base = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
            KEY_READ | KEY_WOW64_64KEY,
        )
        .unwrap();
    let key = base.enum_keys().next().unwrap().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("live.reg");
    let args = export_args(Hive::MachineX86, &key, &file);
    SystemArp.export(&args).unwrap();
    let r = check_export(&file, &key);
    std::fs::remove_file(&file).unwrap();
    assert_eq!(r, Ok(()), "key {key}");
    assert!(!file.exists());
}

/// Task 196 (from Task 197): the helper did not start because wingetrs is not in an
/// administrator-only folder. The job ends with that refusal text alone, never as a refused
/// UAC nor with a "falha ao apagar" prefix in front of it.
#[test]
fn folder_refusal_is_its_own_error() {
    struct FolderRefused(String);
    impl ElevatedRunner for FolderRefused {
        fn delete_arp_key(
            &self,
            _hive: MachineHive,
            _key: &str,
            _reason: DeleteReason,
        ) -> Result<(), ElevatedError> {
            Err(ElevatedError::NotAdminFolder(self.0.clone()))
        }
    }
    let text = wingetrs::bridge::helper_folder::refusal_text(Path::new(r"C:\Users\u\Downloads"));
    let dir = tempfile::tempdir().unwrap();
    let reg = FakeReg::new(Some(utf16le(&reg_text("HKEY_LOCAL_MACHINE", "k1"))));
    let el = FolderRefused(text.clone());
    let r = remove_orphan(
        &reg,
        &el,
        dir.path(),
        "ts9",
        &orphan(Hive::MachineX64, "k1"),
    );
    assert_eq!(r, Err(text.clone()));
    let shown = r.unwrap_err();
    assert!(
        !shown.contains(wingetrs::winget_install::codes::UAC_REFUSED),
        "{shown}"
    );
    assert!(!shown.contains("sem permissão"), "{shown}");
}

/// Task 201: an HKLM orphan goes through the session's helper, whose UAC prompt shows the
/// notice phase until the helper runs; then back to `Removendo`.
#[test]
fn hklm_orphan_shows_uac_notice_phase() {
    use wingetrs::install::progress::{ELEVATION_GRANTED, ELEVATION_NOTICE, Phase, ProgressEvent};
    use wingetrs::winget_install::orphans::remove_orphan_with;

    /// A helper that opens its UAC prompt before deleting.
    struct Prompting(FakeElevated);
    impl ElevatedRunner for Prompting {
        fn delete_arp_key_lines(
            &self,
            hive: MachineHive,
            key: &str,
            reason: DeleteReason,
            on_line: &mut dyn FnMut(&str),
        ) -> Result<(), ElevatedError> {
            on_line(ELEVATION_NOTICE);
            on_line(ELEVATION_GRANTED);
            self.0.delete_arp_key(hive, key, reason)
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let el = Prompting(FakeElevated::default());
    let reg = FakeReg::new(Some(utf16le(&reg_text("HKEY_LOCAL_MACHINE", "k1"))));
    let events = Mutex::new(Vec::new());
    let progress = |ev: &ProgressEvent| events.lock().unwrap().push(ev.clone());
    let o = orphan(Hive::MachineX64, "k1");
    remove_orphan_with(&reg, &el, dir.path(), "ts11", &o, &progress).unwrap();
    let removing = ProgressEvent::Phase(Phase::Removing {
        app: o.name.clone(),
    });
    assert_eq!(
        *events.lock().unwrap(),
        [
            removing.clone(),
            ProgressEvent::Phase(Phase::WaitingElevation),
            removing,
        ]
    );
    assert_eq!(el.0.deletes.lock().unwrap().len(), 1);
}
