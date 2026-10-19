#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Native uninstall flow (Task 184).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use wingetrs::bridge::elevated::{ElevatedError, ElevatedRunner};
use wingetrs::bridge::worker::{DeleteReason, MachineHive};
use wingetrs::installed::Hive;
use wingetrs::winget_install::elevated_rules::KnownFolders;
use wingetrs::winget_install::launch::{Elevation, LaunchSpec, Launcher, Show};
use wingetrs::winget_install::msix::{AddResult, MsixError, MsixOps};
use wingetrs::winget_install::orphans::ArpRegistry;
use wingetrs::winget_install::uninstall::{
    BackupLimits, Clock, FolderSize, MAX_BACKUP_BYTES, MsiQueue, UninstallCtx, UninstallPlan,
    UninstallStatus, UninstallTarget, UserFolders, backup_folders, human_size, measure_folders,
    plan_uninstall, remove_registry_only, target_of, uninstall, uninstall_queued,
};
use wingetrs::winget_install::uninstall_cmd::{ArpDetails, Rule};

const GUID: &str = "{12345678-1234-1234-1234-123456789ABC}";
const LOCAL: &str = r"C:\Users\t\AppData\Local";
const ROAMING: &str = r"C:\Users\t\AppData\Roaming";
const DOCS: &str = r"C:\Users\t\Documents";

fn known() -> KnownFolders {
    KnownFolders {
        program_files: PathBuf::from(r"C:\Program Files"),
        program_files_x86: PathBuf::from(r"C:\Program Files (x86)"),
        system_root: PathBuf::from(r"C:\Windows"),
        program_data: PathBuf::from(r"C:\ProgramData"),
        user_profiles: PathBuf::from(r"C:\Users"),
    }
}

fn user() -> UserFolders {
    UserFolders {
        local_app_data: PathBuf::from(LOCAL),
        app_data: PathBuf::from(ROAMING),
        documents: PathBuf::from(DOCS),
        wingetrs_root: PathBuf::from(format!(r"{LOCAL}\wingetrs")),
    }
}

/// Another record of the app that the helper accepts as newer (HKLM, greater version).
fn newer() -> Vec<ArpDetails> {
    vec![kindle(Hive::MachineX64, "Kindle", "9.9")]
}

fn user_env(name: &str) -> Option<String> {
    match name.to_ascii_uppercase().as_str() {
        "LOCALAPPDATA" => Some(LOCAL.to_string()),
        "APPDATA" => Some(ROAMING.to_string()),
        _ => None,
    }
}

fn details(hive: Hive, key: &str) -> ArpDetails {
    ArpDetails {
        hive,
        key: key.to_string(),
        display_name: Some("App".to_string()),
        publisher: Some("Pub".to_string()),
        display_version: Some("1.0".to_string()),
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
    let mut d = details(hive, key);
    d.quiet_uninstall_string = Some(cmd.to_string());
    d
}

fn msi(hive: Hive) -> ArpDetails {
    let mut d = details(hive, GUID);
    d.windows_installer = true;
    d
}

fn kindle(hive: Hive, key: &str, version: &str) -> ArpDetails {
    let mut d = details(hive, key);
    d.display_name = Some("Amazon Kindle".to_string());
    d.publisher = Some("Amazon".to_string());
    d.display_version = Some(version.to_string());
    d.uninstall_string =
        Some(r#""C:\Program Files (x86)\Amazon\Kindle\uninstall.exe""#.to_string());
    d
}

fn no_folders(_: &str) -> bool {
    false
}

fn plan(d: &ArpDetails) -> UninstallPlan {
    plan_uninstall(d, false, &[], true, &known(), &user(), &no_folders).unwrap()
}

struct FakeLauncher {
    code: i32,
    specs: Mutex<Vec<LaunchSpec>>,
}

impl FakeLauncher {
    fn new(code: i32) -> FakeLauncher {
        FakeLauncher {
            code,
            specs: Mutex::new(Vec::new()),
        }
    }
    fn count(&self) -> usize {
        self.specs.lock().unwrap().len()
    }
}

impl Launcher for FakeLauncher {
    fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32> {
        self.specs.lock().unwrap().push(spec.clone());
        Ok(self.code)
    }
}

#[derive(Default)]
struct FakeElevated {
    runs: Mutex<Vec<(MachineHive, String, Rule, bool)>>,
    deletes: Mutex<Vec<(MachineHive, String, DeleteReason)>>,
}

impl FakeElevated {
    fn count(&self) -> usize {
        self.runs.lock().unwrap().len() + self.deletes.lock().unwrap().len()
    }
}

impl ElevatedRunner for FakeElevated {
    fn run_uninstaller(
        &self,
        hive: MachineHive,
        key: &str,
        rule: Rule,
        nsis_hint: bool,
    ) -> Result<i32, ElevatedError> {
        self.runs
            .lock()
            .unwrap()
            .push((hive, key.to_string(), rule, nsis_hint));
        Ok(0)
    }
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

#[derive(Default)]
struct FakeMsix {
    removed: Mutex<Vec<String>>,
}

impl MsixOps for FakeMsix {
    fn add(&self, _file: &Path) -> Result<AddResult, MsixError> {
        panic!("not used");
    }
    fn remove(&self, family: &str) -> Result<usize, MsixError> {
        self.removed.lock().unwrap().push(family.to_string());
        Ok(1)
    }
}

/// Only moves time forward; records each sleep.
#[derive(Default)]
struct FakeClock {
    now: Mutex<Duration>,
    sleeps: Mutex<Vec<Duration>>,
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap()
    }
    fn sleep(&self, d: Duration) {
        *self.now.lock().unwrap() += d;
        self.sleeps.lock().unwrap().push(d);
    }
}

/// `key_exists` answers from a programmed sequence; the last answer repeats.
struct Keys {
    seq: Mutex<VecDeque<bool>>,
    last: Mutex<bool>,
    calls: Mutex<Vec<(Hive, String)>>,
}

impl Keys {
    fn new(seq: &[bool]) -> Keys {
        Keys {
            seq: Mutex::new(seq.iter().copied().collect()),
            last: Mutex::new(*seq.last().unwrap_or(&false)),
            calls: Mutex::new(Vec::new()),
        }
    }
    fn exists(&self, hive: Hive, key: &str) -> bool {
        self.calls.lock().unwrap().push((hive, key.to_string()));
        match self.seq.lock().unwrap().pop_front() {
            Some(v) => {
                *self.last.lock().unwrap() = v;
                v
            }
            None => *self.last.lock().unwrap(),
        }
    }
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

struct World {
    launcher: FakeLauncher,
    elevated: FakeElevated,
    msix: FakeMsix,
    clock: FakeClock,
    keys: Keys,
}

impl World {
    fn new(code: i32, keys: &[bool]) -> World {
        World {
            launcher: FakeLauncher::new(code),
            elevated: FakeElevated::default(),
            msix: FakeMsix::default(),
            clock: FakeClock::default(),
            keys: Keys::new(keys),
        }
    }
    fn run(
        &self,
        plan: &UninstallPlan,
    ) -> Result<wingetrs::winget_install::uninstall::UninstallReport, String> {
        let key_exists = |h: Hive, k: &str| self.keys.exists(h, k);
        let ctx = UninstallCtx {
            launcher: &self.launcher,
            elevated: &self.elevated,
            msix: &self.msix,
            key_exists: &key_exists,
            clock: &self.clock,
            user_expand: &user_env,
            msiexec: Path::new(r"C:\Windows\System32\msiexec.exe"),
            helper_line: &|_| {},
            bridge: &no_bridge,
        };
        uninstall(&ctx, plan)
    }
    fn sleeps(&self) -> Vec<Duration> {
        self.clock.sleeps.lock().unwrap().clone()
    }
}

#[test]
fn key_gone_is_removed() {
    let w = World::new(0, &[true, true, false]);
    let p = plan(&quiet(
        Hive::User,
        "App",
        r#""C:\Users\t\AppData\Local\App\u.exe" /S"#,
    ));
    let r = w.run(&p).unwrap();
    assert_eq!(r.status, UninstallStatus::Removed);
    assert_eq!(r.warning, None);
    assert_eq!(w.sleeps(), vec![Duration::from_millis(500); 2]);
    assert_eq!(w.keys.count(), 3);
    assert_eq!(
        w.keys.calls.lock().unwrap()[0],
        (Hive::User, "App".to_string())
    );
}

#[test]
fn key_stays_warns_after_120s() {
    let w = World::new(0, &[true]);
    let p = plan(&quiet(
        Hive::User,
        "App",
        r#""C:\Users\t\AppData\Local\App\u.exe" /S"#,
    ));
    let r = w.run(&p).unwrap();
    assert_eq!(r.status, UninstallStatus::NoEffect);
    assert!(
        r.warning.as_deref().unwrap().contains("registro continua"),
        "{r:?}"
    );
    assert!(w.clock.now() >= Duration::from_secs(120));
    assert_eq!(w.sleeps().len(), 240);
}

#[test]
fn hkcu_never_elevated() {
    let w = World::new(0, &[false]);
    let p = plan(&quiet(
        Hive::User,
        "App",
        r#""C:\Program Files\App\u.exe" /S"#,
    ));
    assert_eq!(p.elevation, Elevation::Never);
    let r = w.run(&p).unwrap();
    assert_eq!(r.status, UninstallStatus::Removed);
    assert_eq!(w.launcher.count(), 1);
    assert_eq!(w.elevated.count(), 0);
    let spec = w.launcher.specs.lock().unwrap()[0].clone();
    assert_eq!(spec.program, PathBuf::from(r"C:\Program Files\App\u.exe"));
    assert_eq!(spec.params, "/S");
    assert_eq!(spec.show, Show::Normal);
}

#[test]
fn hkcu_msi_runs_injected_msiexec_unelevated() {
    let w = World::new(0, &[false]);
    let p = plan(&msi(Hive::User));
    assert_eq!(p.elevation, Elevation::Never);
    w.run(&p).unwrap();
    assert_eq!(w.elevated.count(), 0);
    let spec = w.launcher.specs.lock().unwrap()[0].clone();
    assert_eq!(
        spec.program,
        PathBuf::from(r"C:\Windows\System32\msiexec.exe")
    );
    assert!(spec.params.starts_with(&format!("/x{GUID}")), "{spec:?}");
}

#[test]
fn hklm_msi_goes_helper() {
    let w = World::new(0, &[false]);
    let p = plan(&msi(Hive::MachineX86));
    assert_eq!(p.elevation, Elevation::Helper);
    let r = w.run(&p).unwrap();
    assert_eq!(r.status, UninstallStatus::Removed);
    assert_eq!(
        *w.elevated.runs.lock().unwrap(),
        vec![(MachineHive::Hklm32, GUID.to_string(), Rule::Msi, false)]
    );
    assert_eq!(w.launcher.count(), 0);
    assert_eq!(w.keys.calls.lock().unwrap()[0].0, Hive::MachineX86);
}

#[test]
fn hklm64_helper_and_nsis_hint_pass_through() {
    let w = World::new(0, &[false]);
    let d = kindle(Hive::MachineX64, "Kindle", "2.8.3");
    let p = plan_uninstall(&d, true, &[], true, &known(), &user(), &no_folders).unwrap();
    assert_eq!(p.elevation, Elevation::Helper);
    w.run(&p).unwrap();
    assert_eq!(
        *w.elevated.runs.lock().unwrap(),
        vec![(MachineHive::Hklm64, "Kindle".to_string(), Rule::Nsis, true)]
    );
}

#[test]
fn hklm_user_folder_uninstaller_runs_unelevated() {
    let w = World::new(0, &[false]);
    let p = plan(&quiet(
        Hive::MachineX86,
        "App",
        r"%LOCALAPPDATA%\Temp\u.exe",
    ));
    assert_eq!(p.elevation, Elevation::Never);
    w.run(&p).unwrap();
    assert_eq!(w.launcher.count(), 1);
    assert_eq!(w.elevated.count(), 0);
    let spec = w.launcher.specs.lock().unwrap()[0].clone();
    assert_eq!(spec.program, PathBuf::from(format!(r"{LOCAL}\Temp\u.exe")));
}

#[test]
fn relative_program_is_refused_before_running() {
    let w = World::new(0, &[false]);
    let p = plan(&quiet(Hive::User, "App", "u.exe /S"));
    assert!(w.run(&p).is_err());
    assert_eq!(w.launcher.count(), 0);
    assert_eq!(w.keys.count(), 0);
}

#[test]
fn msix_uses_remove_package() {
    let w = World::new(0, &[true]);
    let p = UninstallPlan {
        target: UninstallTarget::Msix {
            family: "A_b".to_string(),
        },
        name: "A".to_string(),
        version: None,
        command: None,
        elevation: Elevation::Never,
        shared: None,
    };
    let r = w.run(&p).unwrap();
    assert_eq!(r.status, UninstallStatus::Removed);
    assert_eq!(*w.msix.removed.lock().unwrap(), vec!["A_b".to_string()]);
    assert_eq!(w.launcher.count(), 0);
    assert_eq!(w.elevated.count(), 0);
}

#[test]
fn kindle_folders_only_existing() {
    let d = kindle(Hive::MachineX86, "Kindle", "2.8.3");
    let roaming_amazon = format!(r"{ROAMING}\Amazon");
    let asked = Mutex::new(Vec::new());
    let exists = |p: &str| {
        asked.lock().unwrap().push(p.to_string());
        p == roaming_amazon
    };
    let p = plan_uninstall(&d, true, &newer(), false, &known(), &user(), &exists).unwrap();
    let shared = p.shared.unwrap();
    assert_eq!(shared.folders, vec![PathBuf::from(&roaming_amazon)]);
    let asked = asked.lock().unwrap().clone();
    for want in [
        format!(r"{LOCAL}\Amazon"),
        format!(r"{ROAMING}\Amazon"),
        format!(r"{LOCAL}\Amazon Kindle"),
        format!(r"{ROAMING}\Amazon Kindle"),
        format!(r"{DOCS}\Amazon Kindle"),
    ] {
        assert!(asked.contains(&want), "{want} not asked: {asked:?}");
    }
}

#[test]
fn unsafe_names_are_ignored_for_folders() {
    for (publisher, name) in [("..", "a/b"), ("NUL", r"x\y"), ("Pub.", "C:evil")] {
        let mut d = kindle(Hive::User, "K", "1");
        d.publisher = Some(publisher.to_string());
        d.display_name = Some(name.to_string());
        let asked = Mutex::new(Vec::new());
        let all = |p: &str| {
            asked.lock().unwrap().push(p.to_string());
            true
        };
        let p = plan_uninstall(&d, true, &newer(), false, &known(), &user(), &all).unwrap();
        assert_eq!(p.shared.unwrap().folders, Vec::<PathBuf>::new());
        assert!(asked.lock().unwrap().is_empty());
    }
}

#[test]
fn no_others_no_shared() {
    let d = kindle(Hive::User, "K", "1");
    let all = |_: &str| true;
    let p = plan_uninstall(&d, true, &[], false, &known(), &user(), &all).unwrap();
    assert_eq!(p.shared, None);
}

#[test]
fn no_command_is_an_error() {
    let d = details(Hive::User, "App");
    let e = plan_uninstall(&d, false, &[], true, &known(), &user(), &no_folders).unwrap_err();
    assert!(e.contains("sem desinstalador registrado"), "{e}");
}

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
    content: Option<Vec<u8>>,
    exports: Mutex<Vec<Vec<String>>>,
    deletes: Mutex<Vec<String>>,
}

impl FakeReg {
    fn new(content: Option<Vec<u8>>) -> FakeReg {
        FakeReg {
            content,
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
        false
    }
}

#[test]
fn registry_only_offered_only_when_not_main() {
    let dir = tempfile::tempdir().unwrap();
    let d = kindle(Hive::MachineX86, "Kindle", "2.8.3");
    let content = utf16le(&reg_text("HKEY_LOCAL_MACHINE", "Kindle"));

    let main = plan_uninstall(&d, true, &newer(), true, &known(), &user(), &no_folders).unwrap();
    assert!(!main.shared.as_ref().unwrap().registry_only_allowed);
    let reg = FakeReg::new(Some(content.clone()));
    let el = FakeElevated::default();
    assert!(remove_registry_only(&main, &reg, &el, dir.path(), "ts1").is_err());
    assert_eq!(reg.exports.lock().unwrap().len(), 0);
    assert_eq!(el.count(), 0);

    let other = plan_uninstall(&d, true, &newer(), false, &known(), &user(), &no_folders).unwrap();
    assert!(other.shared.as_ref().unwrap().registry_only_allowed);
    let reg = FakeReg::new(Some(content));
    let el = FakeElevated::default();
    let file = remove_registry_only(&other, &reg, &el, dir.path(), "ts2").unwrap();
    assert!(file.is_file());
    assert_eq!(reg.exports.lock().unwrap().len(), 1);
    assert_eq!(
        *el.deletes.lock().unwrap(),
        vec![(
            MachineHive::Hklm32,
            "Kindle".to_string(),
            DeleteReason::Duplicate
        )]
    );
    assert_eq!(reg.deletes.lock().unwrap().len(), 0);
}

#[test]
fn registry_only_checks_export_before_deleting() {
    let dir = tempfile::tempdir().unwrap();
    for (hive, root) in [
        (Hive::MachineX86, "HKEY_CURRENT_USER"),
        (Hive::User, "HKEY_LOCAL_MACHINE"),
    ] {
        let d = kindle(hive, "Kindle", "2.8.3");
        let p = plan_uninstall(&d, true, &newer(), false, &known(), &user(), &no_folders).unwrap();
        // The export names the key under the other root: not this key's backup.
        let reg = FakeReg::new(Some(utf16le(&reg_text(root, "Kindle"))));
        let el = FakeElevated::default();
        assert!(remove_registry_only(&p, &reg, &el, dir.path(), "ts").is_err());
        assert_eq!(reg.exports.lock().unwrap().len(), 1);
        assert_eq!(reg.deletes.lock().unwrap().len(), 0);
        assert_eq!(el.count(), 0);
    }
}

#[test]
fn registry_only_hkcu_deletes_in_process() {
    let dir = tempfile::tempdir().unwrap();
    let d = kindle(Hive::User, "Kindle", "2.9.1");
    let p = plan_uninstall(&d, true, &newer(), false, &known(), &user(), &no_folders).unwrap();
    let reg = FakeReg::new(Some(utf16le(&reg_text("HKEY_CURRENT_USER", "Kindle"))));
    let el = FakeElevated::default();
    remove_registry_only(&p, &reg, &el, dir.path(), "ts").unwrap();
    assert_eq!(*reg.deletes.lock().unwrap(), vec!["Kindle".to_string()]);
    assert_eq!(el.count(), 0);
}

/// Task 182 review: the helper deletes an HKLM key as a duplicate only against another HKLM
/// key with a strictly greater version, so with HKLM 2.8.3 + HKCU 2.9.1 only option A
/// (the old uninstaller, with the warning and the folder backup) is offered.
#[test]
fn kindle_hklm_old_hkcu_new_offers_uninstaller_only() {
    let dir = tempfile::tempdir().unwrap();
    let old = kindle(Hive::MachineX86, "Kindle", "2.8.3");
    let new = kindle(Hive::User, "Kindle", "2.9.1");
    let roaming_amazon = format!(r"{ROAMING}\Amazon");
    let exists = |p: &str| p == roaming_amazon;
    let p = plan_uninstall(&old, true, &[new], false, &known(), &user(), &exists).unwrap();
    assert_eq!(p.command.as_ref().unwrap().rule, Rule::Nsis);
    assert_eq!(p.elevation, Elevation::Helper);
    let shared = p.shared.as_ref().unwrap();
    assert!(!shared.registry_only_allowed);
    assert_eq!(shared.folders, vec![PathBuf::from(&roaming_amazon)]);
    let text = p.to_string();
    assert!(text.contains("Outra instalação de Amazon Kindle"), "{text}");
    assert!(
        text.contains("Remover só o registro não está disponível"),
        "{text}"
    );
    assert!(text.contains("HKCU"), "{text}");

    let reg = FakeReg::new(Some(utf16le(&reg_text("HKEY_LOCAL_MACHINE", "Kindle"))));
    let el = FakeElevated::default();
    assert!(remove_registry_only(&p, &reg, &el, dir.path(), "ts").is_err());
    assert_eq!(el.count(), 0);
    assert_eq!(reg.exports.lock().unwrap().len(), 0);
}

#[test]
fn kindle_newer_hklm_allows_registry_only() {
    let old = kindle(Hive::MachineX86, "Kindle", "2.8.3");
    let cases = [
        (kindle(Hive::MachineX64, "Kindle", "2.9.1"), true),
        (kindle(Hive::MachineX64, "Kindle", "2.8.3.0"), false),
        (kindle(Hive::MachineX64, "Kindle", "2.10"), true),
        (kindle(Hive::MachineX64, "Kindle", "2.7"), false),
    ];
    for (other, allowed) in cases {
        let v = other.display_version.clone();
        let p =
            plan_uninstall(&old, true, &[other], false, &known(), &user(), &no_folders).unwrap();
        assert_eq!(
            p.shared.as_ref().unwrap().registry_only_allowed,
            allowed,
            "{v:?}"
        );
        assert_eq!(
            p.to_string().contains("não está disponível"),
            !allowed,
            "{v:?}"
        );
    }
    // The principal record never offers it, whatever the others say.
    let p = plan_uninstall(
        &old,
        true,
        &[kindle(Hive::MachineX64, "Kindle", "3.0")],
        true,
        &known(),
        &user(),
        &no_folders,
    )
    .unwrap();
    assert!(!p.shared.unwrap().registry_only_allowed);
}

#[test]
fn kindle_hkcu_old_allows_registry_only() {
    let old = kindle(Hive::User, "Kindle", "2.8.3");
    let new = kindle(Hive::MachineX86, "Kindle", "2.9.1");
    let p = plan_uninstall(&old, true, &[new], false, &known(), &user(), &no_folders).unwrap();
    assert!(p.shared.unwrap().registry_only_allowed);
}

fn plenty(_: &Path) -> Option<u64> {
    Some(u64::MAX)
}

fn roomy() -> BackupLimits<'static> {
    BackupLimits::standard(&plenty)
}

/// `Amazon` with 3 files of 10 bytes in a temp dir.
fn three_files(dir: &Path) -> PathBuf {
    let src = dir.join("Amazon");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    for f in ["a.txt", "b.txt", r"sub\c.txt"] {
        std::fs::write(src.join(f), [b'x'; 10]).unwrap();
    }
    src
}

#[test]
fn backup_passes_when_it_fits() {
    let dir = tempfile::tempdir().unwrap();
    let src = three_files(dir.path());
    let dest = dir.path().join("dest");
    assert_eq!(
        measure_folders(std::slice::from_ref(&src)).unwrap(),
        FolderSize {
            bytes: 30,
            entries: 4
        }
    );
    let free = |_: &Path| Some(30);
    let limits = BackupLimits {
        max_bytes: 30,
        free_space: &free,
    };
    let notes = backup_folders(std::slice::from_ref(&src), &dest, &limits).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert!(dest.join("1-Amazon").join("sub").join("c.txt").is_file());
}

#[test]
fn backup_refuses_over_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let src = three_files(dir.path());
    let dest = dir.path().join("dest");
    let limits = BackupLimits {
        max_bytes: 29,
        free_space: &plenty,
    };
    let e = backup_folders(std::slice::from_ref(&src), &dest, &limits).unwrap_err();
    assert!(e.contains("acima do limite"), "{e}");
    assert!(!dest.exists());
    assert_eq!(MAX_BACKUP_BYTES, 2 * 1024 * 1024 * 1024);
}

#[test]
fn backup_refuses_without_free_space() {
    let dir = tempfile::tempdir().unwrap();
    let src = three_files(dir.path());
    let dest = dir.path().join("dest");
    let small = |_: &Path| Some(29);
    let e = backup_folders(
        std::slice::from_ref(&src),
        &dest,
        &BackupLimits::standard(&small),
    )
    .unwrap_err();
    assert!(e.contains("espaço livre insuficiente"), "{e}");
    let unknown = |_: &Path| None;
    let e = backup_folders(
        std::slice::from_ref(&src),
        &dest,
        &BackupLimits::standard(&unknown),
    )
    .unwrap_err();
    assert!(e.contains("espaço livre"), "{e}");
    assert!(!dest.exists());
}

#[test]
fn backup_refuses_a_folder_that_contains_the_destination() {
    let dir = tempfile::tempdir().unwrap();
    let src = three_files(dir.path());
    let dest = PathBuf::from(src.to_string_lossy().to_uppercase()).join("backups");
    let e = backup_folders(std::slice::from_ref(&src), &dest, &roomy()).unwrap_err();
    assert!(e.contains("contém o destino"), "{e}");
    assert!(!dest.exists());
}

#[test]
fn generic_names_are_never_offered() {
    for (publisher, name) in [
        ("Microsoft", "Temp"),
        ("google", "packages"),
        ("NVIDIA", "Programs"),
        ("Intel", "Windows"),
        ("Adobe", "Mozilla"),
        ("Apple", "WINGETRS"),
    ] {
        let mut d = kindle(Hive::User, "K", "1");
        d.publisher = Some(publisher.to_string());
        d.display_name = Some(name.to_string());
        let asked = Mutex::new(Vec::new());
        let all = |p: &str| {
            asked.lock().unwrap().push(p.to_string());
            true
        };
        let p = plan_uninstall(&d, true, &newer(), false, &known(), &user(), &all).unwrap();
        assert_eq!(
            p.shared.unwrap().folders,
            Vec::<PathBuf>::new(),
            "{publisher}/{name}"
        );
        assert!(asked.lock().unwrap().is_empty(), "{publisher}/{name}");
    }
}

#[test]
fn folder_containing_the_wingetrs_root_is_not_offered() {
    let d = kindle(Hive::User, "K", "1");
    let mut u = user();
    u.wingetrs_root = PathBuf::from(format!(r"{LOCAL}\AMAZON\x\wingetrs"));
    let all = |_: &str| true;
    let p = plan_uninstall(&d, true, &newer(), false, &known(), &u, &all).unwrap();
    let folders = p.shared.unwrap().folders;
    assert!(
        !folders.contains(&PathBuf::from(format!(r"{LOCAL}\Amazon"))),
        "{folders:?}"
    );
    assert!(
        folders.contains(&PathBuf::from(format!(r"{ROAMING}\Amazon"))),
        "{folders:?}"
    );
}

#[test]
fn valid_publisher_with_bad_name_offers_only_publisher_folders() {
    for name in ["NUL", "Pub.", "a/b"] {
        let mut d = kindle(Hive::User, "K", "1");
        d.publisher = Some("Amazon".to_string());
        d.display_name = Some(name.to_string());
        let all = |_: &str| true;
        let p = plan_uninstall(&d, true, &newer(), false, &known(), &user(), &all).unwrap();
        assert_eq!(
            p.shared.unwrap().folders,
            vec![
                PathBuf::from(format!(r"{LOCAL}\Amazon")),
                PathBuf::from(format!(r"{ROAMING}\Amazon")),
            ],
            "{name}"
        );
    }
}

#[test]
fn plan_shows_the_backup_size() {
    let d = kindle(Hive::User, "K", "1");
    let mut p = plan_uninstall(&d, true, &newer(), false, &known(), &user(), &no_folders).unwrap();
    assert!(!p.to_string().contains("tamanho"), "{p}");
    p.shared.as_mut().unwrap().size = Some(FolderSize {
        bytes: 3 << 29,
        entries: 7,
    });
    assert!(
        p.to_string()
            .contains("Tamanho da cópia: 1,5 GiB em 7 itens"),
        "{p}"
    );
    assert_eq!(human_size(12), "12 B");
    assert_eq!(human_size(20 << 20), "20,0 MiB");
}

#[cfg(windows)]
#[test]
fn measure_skips_reparse_points() {
    let dir = tempfile::tempdir().unwrap();
    let src = three_files(dir.path());
    let other = dir.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("big.bin"), vec![0u8; 4096]).unwrap();
    junction::create(&other, src.join("link")).unwrap();
    let size = measure_folders(std::slice::from_ref(&src)).unwrap();
    assert_eq!(size.bytes, 30);
}
#[cfg(windows)]
#[test]
fn backup_copies_and_skips_reparse() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("Amazon");
    let other = dir.path().join("other");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(src.join("a.txt"), b"a").unwrap();
    std::fs::write(src.join("sub").join("b.txt"), b"b").unwrap();
    std::fs::write(other.join("secret.txt"), b"s").unwrap();
    junction::create(&other, src.join("link")).unwrap();
    let dest = dir.path().join("backups").join("ts").join("Kindle");

    let notes = backup_folders(std::slice::from_ref(&src), &dest, &roomy()).unwrap();

    let copy = dest.join("1-Amazon");
    assert_eq!(std::fs::read(copy.join("a.txt")).unwrap(), b"a");
    assert_eq!(std::fs::read(copy.join("sub").join("b.txt")).unwrap(), b"b");
    assert!(!copy.join("link").join("secret.txt").exists());
    assert!(!copy.join("link").exists());
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("link"), "{notes:?}");
    assert!(notes[0].contains("pulado"), "{notes:?}");
}

#[test]
fn backup_numbers_each_folder_and_reports_missing() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("src1").join("Amazon");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::write(a.join("x.txt"), b"x").unwrap();
    let gone = dir.path().join("src2").join("Gone");
    let dest = dir.path().join("dest");
    let notes = backup_folders(&[a, gone], &dest, &roomy()).unwrap();
    assert!(dest.join("1-Amazon").join("x.txt").is_file());
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("Gone"), "{notes:?}");
}

#[test]
fn nonzero_exit_fails_without_polling() {
    let w = World::new(1603, &[true]);
    let p = plan(&quiet(
        Hive::User,
        "App",
        r#""C:\Users\t\AppData\Local\App\u.exe" /S"#,
    ));
    assert!(w.run(&p).is_err());
    assert_eq!(w.launcher.count(), 1);
    assert_eq!(w.keys.count(), 0);
    assert!(w.sleeps().is_empty());
}

#[test]
fn target_of_parses_app_keys() {
    assert_eq!(
        target_of("arp/hklm32/{G}"),
        Some(UninstallTarget::Arp {
            hive: Hive::MachineX86,
            key: "{G}".to_string()
        })
    );
    assert_eq!(
        target_of("arp/hklm64/K"),
        Some(UninstallTarget::Arp {
            hive: Hive::MachineX64,
            key: "K".to_string()
        })
    );
    assert_eq!(
        target_of("arp/hkcu/App"),
        Some(UninstallTarget::Arp {
            hive: Hive::User,
            key: "App".to_string()
        })
    );
    assert_eq!(
        target_of("msix/A_b"),
        Some(UninstallTarget::Msix {
            family: "A_b".to_string()
        })
    );
    assert_eq!(target_of("scoop/x"), None);
    assert_eq!(target_of("arp/hkcu/"), None);
    assert_eq!(target_of("arp/hkcu/a/b"), None);
    assert_eq!(target_of("arp/hku/App"), None);
    assert_eq!(target_of("msix/"), None);
}

#[test]
fn plan_text_matches_confirmation() {
    let p = plan(&msi(Hive::MachineX86));
    let text = p.to_string();
    assert!(
        text.contains("Pede permissão de administrador (UAC)"),
        "{text}"
    );
    // The command line only in `--dry-run`.
    assert!(!text.contains("msiexec"), "{text}");
    assert!(
        p.dry_run_details().iter().any(|l| l.contains("msiexec")),
        "{p:?}"
    );
    assert!(!text.contains("Abre a janela do desinstalador"), "{text}");

    let mut d = details(Hive::User, "App");
    d.uninstall_string = Some(r#""C:\Users\t\AppData\Local\App\u.exe""#.to_string());
    let p = plan(&d);
    assert!(p.command.as_ref().unwrap().interactive);
    let text = p.to_string();
    assert!(text.contains("Abre a janela do desinstalador"), "{text}");
    assert!(!text.contains("Pede permissão de administrador"), "{text}");
    assert!(!text.contains("Outra instalação"), "{text}");

    let d = kindle(Hive::User, "K", "1");
    let p = plan_uninstall(&d, true, &newer(), false, &known(), &user(), &no_folders).unwrap();
    assert!(p.to_string().contains("Outra instalação"), "{p}");
}

// ---- Task 194: Burn reboot codes and the Windows Installer queue ----

fn burn(hive: Hive) -> ArpDetails {
    let mut d = details(hive, "{B0B0B0B0-1111-2222-3333-444444444444}");
    d.uninstall_string = Some(
        r#""C:\Users\t\AppData\Local\Package Cache\{B0B0}\vc_redist.exe" /uninstall"#.to_string(),
    );
    d.bundle_upgrade_code = Some("{C0C0C0C0-1111-2222-3333-444444444444}".to_string());
    d
}

#[test]
fn burn_uninstall_3010_key_gone_is_success() {
    for (code, initiated) in [(3010, false), (1641, true)] {
        let w = World::new(code, &[true, false]);
        let p = plan(&burn(Hive::User));
        assert_eq!(p.command.as_ref().unwrap().rule, Rule::Burn);
        let r = w.run(&p).unwrap();
        assert_eq!(r.status, UninstallStatus::Removed, "{code}");
        assert_eq!(r.warning, None);
        assert_eq!(r.reboot, Some(initiated));
        assert_eq!(w.keys.count(), 2);
    }
    // QuietUninstallString of a bundle (Burn writes one) follows the same path.
    let mut d = burn(Hive::User);
    d.quiet_uninstall_string = Some(
        r#""C:\Users\t\AppData\Local\Package Cache\{B0B0}\vc_redist.exe" /uninstall /quiet"#
            .to_string(),
    );
    let w = World::new(3010, &[false]);
    let p = plan(&d);
    assert_eq!(p.command.as_ref().unwrap().rule, Rule::Quiet);
    let r = w.run(&p).unwrap();
    assert_eq!(r.status, UninstallStatus::Removed);
    assert_eq!(r.reboot, Some(false));
}

#[test]
fn burn_uninstall_3010_key_present_is_failure() {
    let w = World::new(3010, &[true]);
    let p = plan(&burn(Hive::User));
    let e = w.run(&p).unwrap_err();
    assert!(e.contains("3010"), "{e}");
    assert!(e.contains("registro continua"), "{e}");
    // It waited for the key before declaring the failure.
    assert!(w.clock.now() >= Duration::from_secs(120));
    // Another code is still a failure at once, without waiting for the key.
    let w = World::new(1603, &[false]);
    let e = w.run(&p).unwrap_err();
    assert!(e.contains("1603"), "{e}");
    assert_eq!(w.keys.count(), 0);
}

/// Answers each launch with the next code; records whether `msi.lock` was held meanwhile.
struct SeqLauncher {
    codes: Mutex<VecDeque<i32>>,
    lock: PathBuf,
    held: Mutex<Vec<bool>>,
}

impl Launcher for SeqLauncher {
    fn run(&self, _spec: &LaunchSpec) -> std::io::Result<i32> {
        let other = wingetrs::cache::UpdateLock::try_acquire(&self.lock).unwrap();
        self.held.lock().unwrap().push(other.is_none());
        Ok(self.codes.lock().unwrap().pop_front().unwrap_or(0))
    }
}

fn run_queued_msi(codes: &[i32]) -> (Result<UninstallStatus, String>, Vec<bool>, Vec<Duration>) {
    let tmp = tempfile::tempdir().unwrap();
    let lock = tmp.path().join("msi.lock");
    let launcher = SeqLauncher {
        codes: Mutex::new(codes.iter().copied().collect()),
        lock: lock.clone(),
        held: Mutex::new(Vec::new()),
    };
    let w = World::new(0, &[false]);
    let key_exists = |h: Hive, k: &str| w.keys.exists(h, k);
    let ctx = UninstallCtx {
        launcher: &launcher,
        elevated: &w.elevated,
        msix: &w.msix,
        key_exists: &key_exists,
        clock: &w.clock,
        user_expand: &user_env,
        msiexec: Path::new(r"C:\Windows\System32\msiexec.exe"),
        helper_line: &|_| {},
        bridge: &no_bridge,
    };
    let sleeps: Mutex<Vec<Duration>> = Mutex::new(Vec::new());
    let sleep = |d: Duration| sleeps.lock().unwrap().push(d);
    let queue = MsiQueue {
        lock: lock.clone(),
        busy: &|| false,
        sleep: &sleep,
    };
    let r = uninstall_queued(&ctx, Some(&queue), &plan(&msi(Hive::User))).map(|r| r.status);
    // Released at the end.
    assert!(
        wingetrs::cache::UpdateLock::try_acquire(&lock)
            .unwrap()
            .is_some()
    );
    let held = launcher.held.lock().unwrap().clone();
    let slept = sleeps.lock().unwrap().clone();
    (r, held, slept)
}

#[test]
fn msiexec_uninstall_retries_1618() {
    let (r, held, slept) = run_queued_msi(&[1618, 0]);
    assert_eq!(r, Ok(UninstallStatus::Removed));
    assert_eq!(held, [true, true]);
    assert_eq!(slept, [Duration::from_secs(5)]);

    // The second attempt fails for another reason: that one is reported, not the 1618.
    let (r, held, _) = run_queued_msi(&[1618, 1603]);
    let e = r.unwrap_err();
    assert!(e.contains("1603"), "{e}");
    assert_eq!(held.len(), 2);

    // 1618 every time: the retries run out and the last 1618 is the failure.
    let (r, held, slept) = run_queued_msi(&[1618, 1618, 1618, 1618, 1618]);
    let e = r.unwrap_err();
    assert!(e.contains("outro instalador"), "{e}");
    assert_eq!(held.len(), 4);
    assert_eq!(slept.len(), 3);
}

#[test]
fn non_msi_uninstall_takes_no_msi_queue() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = tmp.path().join("msi.lock");
    let launcher = SeqLauncher {
        codes: Mutex::new([1618].into_iter().collect()),
        lock: lock.clone(),
        held: Mutex::new(Vec::new()),
    };
    let w = World::new(0, &[false]);
    let key_exists = |h: Hive, k: &str| w.keys.exists(h, k);
    let ctx = UninstallCtx {
        launcher: &launcher,
        elevated: &w.elevated,
        msix: &w.msix,
        key_exists: &key_exists,
        clock: &w.clock,
        user_expand: &user_env,
        msiexec: Path::new(r"C:\Windows\System32\msiexec.exe"),
        helper_line: &|_| {},
        bridge: &no_bridge,
    };
    let queue = MsiQueue {
        lock: lock.clone(),
        busy: &|| true,
        sleep: &|_: Duration| panic!("no wait for a non-MSI uninstaller"),
    };
    let p = plan(&quiet(
        Hive::User,
        "App",
        r#""C:\Users\t\AppData\Local\App\u.exe" /S"#,
    ));
    // An exe uninstaller's 1618 is just an unknown code: one launch, no lock, no retry.
    assert!(uninstall_queued(&ctx, Some(&queue), &p).is_err());
    assert_eq!(*launcher.held.lock().unwrap(), [false]);
}

/// Task 190: when the elevated helper says it waits for what the uninstaller left running,
/// the removal shows "aguardando o desinstalador terminar" instead of keeping the UAC text.
#[test]
fn helper_wait_line_shows_the_waiting_phase() {
    use wingetrs::install::progress::{Phase, ProgressEvent};
    use wingetrs::winget_install::removal::{run_uninstaller, waiting_uninstaller_phase};

    /// `FakeElevated` whose helper reports the wait before the exit code.
    struct Waiting(FakeElevated);
    impl ElevatedRunner for Waiting {
        fn run_uninstaller_lines(
            &self,
            hive: MachineHive,
            key: &str,
            rule: Rule,
            nsis_hint: bool,
            on_line: &mut dyn FnMut(&str),
        ) -> Result<i32, ElevatedError> {
            on_line(wingetrs::bridge::worker::WAITING_UNINSTALLER);
            self.0.run_uninstaller(hive, key, rule, nsis_hint)
        }
    }

    let d = kindle(Hive::MachineX64, "Kindle", "2.8.3");
    let p = plan_uninstall(&d, true, &[], true, &known(), &user(), &no_folders).unwrap();
    assert_eq!(p.elevation, Elevation::Helper);
    let elevated = Waiting(FakeElevated::default());
    let launcher = FakeLauncher::new(0);
    let msix = FakeMsix::default();
    let clock = FakeClock::default();
    let gone = |_: Hive, _: &str| false;
    let seen = Mutex::new(Vec::new());
    let ctx = UninstallCtx {
        launcher: &launcher,
        elevated: &elevated,
        msix: &msix,
        key_exists: &gone,
        clock: &clock,
        user_expand: &user_env,
        msiexec: Path::new(r"C:\Windows\System32\msiexec.exe"),
        helper_line: &|l: &str| seen.lock().unwrap().push(l.to_string()),
        bridge: &no_bridge,
    };
    let dir = tempfile::TempDir::new().unwrap();
    let queue = MsiQueue {
        lock: dir.path().join("msi.lock"),
        busy: &|| false,
        sleep: &|_| {},
    };
    let events = Mutex::new(Vec::new());
    let progress = |ev: &ProgressEvent| events.lock().unwrap().push(ev.clone());
    let r = run_uninstaller(&ctx, &queue, &p, &progress).unwrap();
    assert_eq!(r.status, UninstallStatus::Removed);
    let waiting = waiting_uninstaller_phase(&p.name);
    assert_eq!(
        waiting,
        Phase::Removing {
            app: format!("{}: aguardando o desinstalador terminar", p.name)
        }
    );
    assert_eq!(
        events.lock().unwrap().first(),
        Some(&ProgressEvent::Phase(waiting))
    );
    // The caller's own hook still sees the line.
    assert_eq!(
        *seen.lock().unwrap(),
        [wingetrs::bridge::worker::WAITING_UNINSTALLER]
    );
}

/// Task 201: the helper's UAC notice line shows the notice phase; once the helper runs, the
/// uninstaller's phase replaces it (the notice never stays on screen during the uninstall).
#[test]
fn helper_uac_lines_show_notice_then_uninstaller() {
    use wingetrs::install::progress::{ELEVATION_GRANTED, ELEVATION_NOTICE, Phase, ProgressEvent};
    use wingetrs::winget_install::removal::run_uninstaller;

    /// `FakeElevated` whose helper opens its UAC prompt first.
    struct Prompting(FakeElevated);
    impl ElevatedRunner for Prompting {
        fn run_uninstaller_lines(
            &self,
            hive: MachineHive,
            key: &str,
            rule: Rule,
            nsis_hint: bool,
            on_line: &mut dyn FnMut(&str),
        ) -> Result<i32, ElevatedError> {
            on_line(ELEVATION_NOTICE);
            on_line(ELEVATION_GRANTED);
            self.0.run_uninstaller(hive, key, rule, nsis_hint)
        }
    }

    let d = kindle(Hive::MachineX64, "Kindle", "2.8.3");
    let p = plan_uninstall(&d, true, &[], true, &known(), &user(), &no_folders).unwrap();
    assert_eq!(p.elevation, Elevation::Helper);
    let elevated = Prompting(FakeElevated::default());
    let launcher = FakeLauncher::new(0);
    let msix = FakeMsix::default();
    let clock = FakeClock::default();
    let gone = |_: Hive, _: &str| false;
    let ctx = UninstallCtx {
        launcher: &launcher,
        elevated: &elevated,
        msix: &msix,
        key_exists: &gone,
        clock: &clock,
        user_expand: &user_env,
        msiexec: Path::new(r"C:\Windows\System32\msiexec.exe"),
        helper_line: &|_| {},
        bridge: &no_bridge,
    };
    let dir = tempfile::TempDir::new().unwrap();
    let queue = MsiQueue {
        lock: dir.path().join("msi.lock"),
        busy: &|| false,
        sleep: &|_| {},
    };
    let events = Mutex::new(Vec::new());
    let progress = |ev: &ProgressEvent| events.lock().unwrap().push(ev.clone());
    run_uninstaller(&ctx, &queue, &p, &progress).unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        [
            ProgressEvent::Phase(Phase::WaitingElevation),
            ProgressEvent::Phase(Phase::Uninstalling {
                name: p.name.clone()
            }),
            ProgressEvent::Phase(Phase::CheckingRemoval {
                name: p.name.clone()
            }),
        ]
    );
}

/// Task 201: the registry-only removal of an HKLM duplicate goes through the helper, whose UAC
/// prompt shows the notice phase until it runs, then `Removendo`.
#[test]
fn registry_only_hklm_shows_uac_notice_phase() {
    use wingetrs::install::progress::{ELEVATION_GRANTED, ELEVATION_NOTICE, Phase, ProgressEvent};
    use wingetrs::winget_install::uninstall::remove_registry_only_with;

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
    let d = kindle(Hive::MachineX86, "Kindle", "2.8.3");
    let p = plan_uninstall(&d, true, &newer(), false, &known(), &user(), &no_folders).unwrap();
    let reg = FakeReg::new(Some(utf16le(&reg_text("HKEY_LOCAL_MACHINE", "Kindle"))));
    let el = Prompting(FakeElevated::default());
    let events = Mutex::new(Vec::new());
    let progress = |ev: &ProgressEvent| events.lock().unwrap().push(ev.clone());
    remove_registry_only_with(&p, &reg, &el, dir.path(), "ts", &progress).unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        [
            ProgressEvent::Phase(Phase::WaitingElevation),
            ProgressEvent::Phase(Phase::Removing {
                app: p.name.clone()
            }),
        ]
    );
    assert_eq!(el.0.count(), 1);
}

/// Task 203 items 5 and 6: the UAC step comes before the uninstaller (as in the install
/// plan), and the shared-data warning is a block of its own (⚠), apart from the numbered
/// steps, in the CLI and after the steps in the TUI text.
#[test]
fn removal_plan_uac_first_and_warnings_apart() {
    use wingetrs::cli::{output::winget_uninstall_text, style::with_enabled};
    let old = kindle(Hive::MachineX86, "Kindle", "2.8.3");
    let new = kindle(Hive::User, "Kindle", "2.9.1");
    let roaming_amazon = format!(r"{ROAMING}\Amazon");
    let exists = |p: &str| p == roaming_amazon;
    let mut p = plan_uninstall(&old, true, &[new], false, &known(), &user(), &exists).unwrap();
    p.shared.as_mut().unwrap().size = Some(FolderSize {
        bytes: 3 << 20,
        entries: 7,
    });
    assert_eq!(
        p.steps(),
        vec![
            "Pede permissão de administrador (UAC)".to_string(),
            "Roda o desinstalador registrado do app".to_string()
        ]
    );
    let warnings = p.warnings();
    assert_eq!(warnings.len(), 4, "{warnings:?}");
    assert_eq!(
        warnings[0],
        "Outra instalação de Amazon Kindle usa as mesmas pastas: copie antes de remover."
    );
    assert_eq!(
        warnings[1],
        format!("Pasta compartilhada: {roaming_amazon}")
    );
    assert_eq!(warnings[2], "Tamanho da cópia: 3,0 MiB em 7 itens");
    assert!(
        warnings[3].starts_with("Remover só o registro não está disponível"),
        "{warnings:?}"
    );
    let cli = with_enabled(false, || winget_uninstall_text(&p, true));
    let lines: Vec<&str> = cli.lines().collect();
    assert_eq!(lines[0], "Desinstalar Amazon Kindle 2.8.3", "{cli}");
    assert_eq!(
        lines[1], "  1. ⚠ Pede permissão de administrador (UAC)",
        "{cli}"
    );
    assert_eq!(
        lines[2], "  2. Roda o desinstalador registrado do app",
        "{cli}"
    );
    for (i, w) in warnings.iter().enumerate() {
        assert_eq!(lines[3 + i], format!("  ⚠ {w}"), "{cli}");
    }
    assert!(lines[7].starts_with("  Comando: "), "{cli}");
    assert_eq!(lines.len(), 8, "{cli}");
    // The TUI text: the steps, then the warnings.
    let tui: Vec<String> = p.to_string().lines().map(str::to_string).collect();
    let mut want = vec![p.title()];
    want.extend(p.steps());
    want.extend(warnings);
    assert_eq!(tui, want);

    // A removal without UAC and without another installation: no warning block.
    let mut d = details(Hive::User, "App");
    d.uninstall_string = Some(r#""C:\Users\t\AppData\Local\App\u.exe""#.to_string());
    let p = plan(&d);
    assert!(p.warnings().is_empty());
    assert_eq!(
        p.steps(),
        vec!["Abre a janela do desinstalador".to_string()]
    );
    let cli = with_enabled(false, || winget_uninstall_text(&p, false));
    assert!(!cli.contains('⚠'), "{cli}");
}

/// The bridge runner of an `UninstallCtx` whose uninstall never goes through winget.
fn no_bridge(
    _: &str,
    _: &[String],
    _: &mut dyn FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    panic!("winget bridge not expected")
}

/// Task 203: the same ARP row (jq: `winget uninstall --product-code <id>`, unquoted) gives the
/// plan "Remove pelo winget" and runs through the bridge's runner, once, with that id; never
/// the native launcher nor the helper, never "desinstalador sem caminho absoluto". Through
/// `removal::remove`, what the CLI and the TUI both run. A machine record also says, in the
/// plan (CLI and TUI text) and in the phase, that winget may open a UAC prompt itself.
#[tokio::test]
async fn winget_record_plan_and_run_take_the_bridge() {
    use wingetrs::cli::{output::winget_uninstall_text, style::with_enabled};
    use wingetrs::install::executor::LaunchGate;
    use wingetrs::install::progress::{
        ELEVATION_NOTICE, Phase, ProgressEvent, SELF_ELEVATION_LATER, SELF_ELEVATION_WAIT,
        phase_status, phase_text,
    };
    use wingetrs::winget_install::removal::{
        RemovalChoice, Removed, Steps, remove, run_uninstaller,
    };
    use wingetrs::winget_install::uninstall::WINGET_ADMIN_PROMPT;
    let id = "jqlang.jq_Microsoft.Winget.Source_8wekyb3d8bbwe";
    let remove_step = "Remove pelo winget (pode mostrar o progresso dele)".to_string();
    for hive in [Hive::User, Hive::MachineX64] {
        let machine = hive != Hive::User;
        let mut d = details(hive, id);
        d.display_name = Some("jq".to_string());
        d.uninstall_string = Some(format!("winget uninstall --product-code {id}"));
        let p = plan_uninstall(&d, false, &[], true, &known(), &user(), &no_folders).unwrap();
        let want = if machine {
            vec![WINGET_ADMIN_PROMPT.to_string(), remove_step.clone()]
        } else {
            vec![remove_step.clone()]
        };
        assert_eq!(p.steps(), want, "{hive:?}");
        let cli = with_enabled(false, || winget_uninstall_text(&p, false));
        let tui = p.to_string();
        let notice = "O winget pode pedir permissão de administrador";
        assert_eq!(cli.contains(&format!("1. ⚠ {notice}")), machine, "{cli}");
        assert_eq!(tui.contains(notice), machine, "{tui}");

        let w = World::new(0, &[false]);
        let key_exists = |h: Hive, k: &str| w.keys.exists(h, k);
        let calls: Mutex<Vec<(String, Vec<String>)>> = Mutex::new(Vec::new());
        let bridge = |program: &str,
                      args: &[String],
                      _: &mut dyn FnMut(&str)|
         -> std::io::Result<(i32, Vec<String>)> {
            calls
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            Ok((0, Vec::new()))
        };
        let ctx = UninstallCtx {
            launcher: &w.launcher,
            elevated: &w.elevated,
            msix: &w.msix,
            key_exists: &key_exists,
            clock: &w.clock,
            user_expand: &user_env,
            msiexec: Path::new(r"C:\Windows\System32\msiexec.exe"),
            helper_line: &|_| {},
            bridge: &bridge,
        };
        let tmp = tempfile::tempdir().unwrap();
        let queue = MsiQueue {
            lock: tmp.path().join("msi.lock"),
            busy: &|| false,
            sleep: &|_| {},
        };
        let phases: Mutex<Vec<Phase>> = Mutex::new(Vec::new());
        let progress = |ev: &ProgressEvent| {
            if let ProgressEvent::Phase(ph) = ev {
                phases.lock().unwrap().push(ph.clone());
            }
        };
        let gate = LaunchGate::default();
        let steps = Steps {
            gate: &gate,
            progress: &progress,
            backups: tmp.path(),
            stamp: "1",
            free_space: &|_| None,
        };
        let removed = remove(
            &steps,
            async { Ok::<(), wingetrs::winget_install::ops::WingetOpsError>(()) },
            &p,
            RemovalChoice::Remove,
            |pr| run_uninstaller(&ctx, &queue, &p, pr),
            |_| panic!("registry-only not chosen"),
        )
        .await
        .unwrap();
        let Removed::App { report, .. } = removed else {
            panic!("not an app removal")
        };
        assert_eq!(report.status, UninstallStatus::Removed, "{hive:?}");
        let calls = calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "{hive:?}");
        assert_eq!(calls[0].0, "winget");
        assert_eq!(
            calls[0].1[..3],
            [
                "uninstall".to_string(),
                "--product-code".to_string(),
                id.to_string()
            ]
        );
        assert_eq!(w.launcher.count(), 0, "{hive:?}");
        assert_eq!(w.elevated.count(), 0, "{hive:?}");
        // The phase while winget runs: the UAC notice for a machine record (the CLI line and
        // the TUI status bar both render it with `phase_text`/`phase_status`).
        let phases = phases.lock().unwrap().clone();
        let self_elevating = Phase::SelfElevating {
            name: "jq".to_string(),
        };
        assert_eq!(phases.contains(&self_elevating), machine, "{phases:?}");
        assert_eq!(
            phases.contains(&Phase::Uninstalling {
                name: "jq".to_string()
            }),
            !machine,
            "{phases:?}"
        );
        if machine {
            let early = phase_status(&self_elevating, std::time::Duration::ZERO);
            assert!(early.contains(ELEVATION_NOTICE), "{early}");
            let late = phase_text(&self_elevating, SELF_ELEVATION_WAIT);
            assert!(late.contains(SELF_ELEVATION_LATER), "{late}");
        }
    }
}

/// Task 203: winget's failure is the removal's failure (its code), and a record whose line
/// is not exactly `winget uninstall --product-code <plain id>` never reaches winget.
#[test]
fn winget_record_failure_and_other_lines() {
    let id = "jqlang.jq_Microsoft.Winget.Source_8wekyb3d8bbwe";
    let mut d = details(Hive::User, id);
    d.uninstall_string = Some(format!("winget uninstall --product-code {id}"));
    let p = plan(&d);
    let w = World::new(0, &[true]);
    let key_exists = |h: Hive, k: &str| w.keys.exists(h, k);
    let failing =
        |_: &str, _: &[String], _: &mut dyn FnMut(&str)| -> std::io::Result<(i32, Vec<String>)> {
            Ok((
                0x8A150014u32 as i32,
                vec!["Nenhum pacote encontrado".to_string()],
            ))
        };
    let ctx = UninstallCtx {
        launcher: &w.launcher,
        elevated: &w.elevated,
        msix: &w.msix,
        key_exists: &key_exists,
        clock: &w.clock,
        user_expand: &user_env,
        msiexec: Path::new(r"C:\Windows\System32\msiexec.exe"),
        helper_line: &|_| {},
        bridge: &failing,
    };
    let e = uninstall(&ctx, &p).unwrap_err();
    assert!(e.contains("0x8A150014"), "{e}");
    assert!(e.contains("Nenhum pacote encontrado"), "{e}");
    assert_eq!(w.launcher.count(), 0);
    for line in [
        format!("winget uninstall --product-code {id} --force"),
        format!("winget uninstall --id {id}"),
        "winget uninstall --product-code a&b".to_string(),
        format!("wingetx uninstall --product-code {id}"),
        // Separate arguments: the program itself must be winget.
        r#""C:\x\evil.exe" uninstall --product-code x"#.to_string(),
        // An id winget would read as one of its options.
        "winget uninstall --product-code -x".to_string(),
    ] {
        let mut d = details(Hive::User, id);
        d.uninstall_string = Some(line.clone());
        let p = plan(&d);
        assert!(
            !p.steps().iter().any(|s| s.contains("winget")),
            "{line}: {:?}",
            p.steps()
        );
        let w = World::new(0, &[false]);
        // `no_bridge` panics: the line goes to the native checks.
        let _ = w.run(&p);
    }
}
