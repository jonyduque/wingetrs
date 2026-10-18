#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Native uninstall flow (Task 184).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use wingetrs::bridge::elevated::{ElevatedError, ElevatedRunner};
use wingetrs::bridge::worker::{DeleteReason, InstallerRequest, MachineHive};
use wingetrs::installed::Hive;
use wingetrs::winget_install::elevated_rules::KnownFolders;
use wingetrs::winget_install::launch::{Elevation, LaunchSpec, Launcher, Show};
use wingetrs::winget_install::msix::{AddResult, MsixError, MsixOps};
use wingetrs::winget_install::orphans::ArpRegistry;
use wingetrs::winget_install::uninstall::{
    Clock, UninstallCtx, UninstallPlan, UninstallStatus, UninstallTarget, UserFolders,
    backup_folders, plan_uninstall, plan_uninstall_among, remove_registry_only, target_of,
    uninstall,
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
    }
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
    plan_uninstall(d, false, false, true, &known(), &user(), &no_folders).unwrap()
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
    fn run_installer(
        &self,
        _req: &InstallerRequest,
        _on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        panic!("not used");
    }
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
    let p = plan_uninstall(&d, true, false, true, &known(), &user(), &no_folders).unwrap();
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
    let p = plan_uninstall(&d, true, true, false, &known(), &user(), &exists).unwrap();
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
        let p = plan_uninstall(&d, true, true, false, &known(), &user(), &all).unwrap();
        assert_eq!(p.shared.unwrap().folders, Vec::<PathBuf>::new());
        assert!(asked.lock().unwrap().is_empty());
    }
}

#[test]
fn no_others_no_shared() {
    let d = kindle(Hive::User, "K", "1");
    let all = |_: &str| true;
    let p = plan_uninstall(&d, true, false, false, &known(), &user(), &all).unwrap();
    assert_eq!(p.shared, None);
}

#[test]
fn no_command_is_an_error() {
    let d = details(Hive::User, "App");
    let e = plan_uninstall(&d, false, false, true, &known(), &user(), &no_folders).unwrap_err();
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

    let main = plan_uninstall(&d, true, true, true, &known(), &user(), &no_folders).unwrap();
    assert!(!main.shared.as_ref().unwrap().registry_only_allowed);
    let reg = FakeReg::new(Some(content.clone()));
    let el = FakeElevated::default();
    assert!(remove_registry_only(&main, &reg, &el, dir.path(), "ts1").is_err());
    assert_eq!(reg.exports.lock().unwrap().len(), 0);
    assert_eq!(el.count(), 0);

    let other = plan_uninstall(&d, true, true, false, &known(), &user(), &no_folders).unwrap();
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
        let p = plan_uninstall(&d, true, true, false, &known(), &user(), &no_folders).unwrap();
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
    let p = plan_uninstall(&d, true, true, false, &known(), &user(), &no_folders).unwrap();
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
    let p = plan_uninstall_among(&old, true, &[new], false, &known(), &user(), &exists).unwrap();
    assert_eq!(p.command.as_ref().unwrap().rule, Rule::Nsis);
    assert_eq!(p.elevation, Elevation::Helper);
    let shared = p.shared.as_ref().unwrap();
    assert!(!shared.registry_only_allowed);
    assert_eq!(shared.folders, vec![PathBuf::from(&roaming_amazon)]);
    let text = p.to_string();
    assert!(text.contains("Outra instalação de Amazon Kindle"), "{text}");
    assert!(
        text.contains("remover só o registro não está disponível"),
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
        let p = plan_uninstall_among(&old, true, &[other], false, &known(), &user(), &no_folders)
            .unwrap();
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
    let p = plan_uninstall_among(
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
    let p =
        plan_uninstall_among(&old, true, &[new], false, &known(), &user(), &no_folders).unwrap();
    assert!(p.shared.unwrap().registry_only_allowed);
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

    let notes = backup_folders(std::slice::from_ref(&src), &dest);

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
    let notes = backup_folders(&[a, gone], &dest);
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
    assert!(text.contains("pede UAC"), "{text}");
    assert!(text.contains("msiexec"), "{text}");
    assert!(!text.contains("abre a janela do desinstalador"), "{text}");

    let mut d = details(Hive::User, "App");
    d.uninstall_string = Some(r#""C:\Users\t\AppData\Local\App\u.exe""#.to_string());
    let p = plan(&d);
    assert!(p.command.as_ref().unwrap().interactive);
    let text = p.to_string();
    assert!(text.contains("abre a janela do desinstalador"), "{text}");
    assert!(!text.contains("pede UAC"), "{text}");
    assert!(!text.contains("Outra instalação"), "{text}");

    let d = kindle(Hive::User, "K", "1");
    let p = plan_uninstall(&d, true, true, false, &known(), &user(), &no_folders).unwrap();
    assert!(p.to_string().contains("Outra instalação"), "{p}");
}
