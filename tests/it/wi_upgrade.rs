#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Native upgrade and engine choice (Task 185).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wingetrs::bridge::elevated::{ElevatedError, ElevatedRunner};
use wingetrs::bridge::worker::{DeleteReason, InstallerRequest, MachineHive};
use wingetrs::bridge::{Applicability, host_arch, winget_applicability};
use wingetrs::cache::{DataDir, UpdateLock, sha256_hex};
use wingetrs::config::{DefaultScope, WingetEngine};
use wingetrs::http::Http;
use wingetrs::install::executor::LaunchGate;
use wingetrs::install::progress::ProgressEvent;
use wingetrs::installed::correlate::{InstallScope, ListRow, UpdateState};
use wingetrs::installed::{Hive, InstallSource, InstalledApp};
use wingetrs::sources::winget::manifest::{self, MergedManifest};
use wingetrs::update::LockWait;
use wingetrs::winget_install::elevated_rules::KnownFolders;
use wingetrs::winget_install::engine::{
    Engine, InstalledFacts, InstalledKind, UpgradeError, UpgradeRoute, choose_engine,
    installed_facts, plan_upgrade, upgrade, upgrade_all_targets, upgrade_route, upgrade_scope,
};
use wingetrs::winget_install::launch::{LaunchSpec, Launcher};
use wingetrs::winget_install::msix::{AddResult, MsixError, MsixOps};
use wingetrs::winget_install::ops::{InstallReport, WingetCtx, WingetOpsError};
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan, plan};
use wingetrs::winget_install::select::{Host, Refusal, Scope, ScopeWanted};
use wingetrs::winget_install::uninstall::{
    Clock, UninstallCtx, UninstallPlan, UserFolders, plan_uninstall,
};
use wingetrs::winget_install::uninstall_cmd::{ArpDetails, Rule};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---- helpers ----

/// `(scope, UpgradeBehavior)` per installer.
type Entries<'a> = Vec<(Option<&'a str>, Option<&'a str>)>;

fn host() -> Host {
    Host {
        arch: host_arch().into(),
        os: [10, 0, 22631, 0],
        locale: "pt-BR".into(),
    }
}

/// One installer per `(scope, behavior)` entry; `root` goes at the top of the manifest.
fn manifest_with(
    kind: &str,
    entries: &[(Option<&str>, Option<&str>)],
    url: &str,
    sha: &str,
    extra: &str,
) -> MergedManifest {
    let mut yaml =
        "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nPackageName: Contoso App\nPublisher: Contoso\nInstallers:\n"
            .to_string();
    for (scope, behavior) in entries {
        yaml += &format!("- Architecture: {}\n  InstallerType: {kind}\n", host_arch());
        if let Some(s) = scope {
            yaml += &format!("  Scope: {s}\n");
        }
        if let Some(b) = behavior {
            yaml += &format!("  UpgradeBehavior: {b}\n");
        }
        yaml += &format!("  InstallerUrl: {url}\n  InstallerSha256: {sha}\n{extra}");
    }
    manifest::parse(yaml.as_bytes()).unwrap()
}

const URL: &str = "https://example.invalid/setup";
const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn planned(
    m: &MergedManifest,
    wanted: ScopeWanted,
    from: Option<&str>,
) -> Result<PlanOrBridge, Refusal> {
    plan(m, &host(), wanted, from.map(str::to_string))
}

fn native(m: &MergedManifest, wanted: ScopeWanted, from: Option<&str>) -> WingetPlan {
    match planned(m, wanted, from).unwrap() {
        PlanOrBridge::Native(p) => p,
        PlanOrBridge::Bridge(why) => panic!("bridge: {why}"),
    }
}

fn one_installer(behavior: Option<&str>) -> manifest::EffectiveInstaller {
    let m = manifest_with("inno", &[(Some("user"), behavior)], URL, SHA, "");
    m.effective().remove(0)
}

// ---- engine choice ----

#[test]
fn engine_setting_winget_always_bridge() {
    let m = manifest_with("inno", &[(Some("user"), None)], URL, SHA, "");
    let ok = planned(&m, ScopeWanted::Prefer(Scope::User), None);
    assert!(matches!(ok, Ok(PlanOrBridge::Native(_))));
    let r = choose_engine(WingetEngine::Winget, ok).unwrap();
    assert!(matches!(r, Engine::Bridge(None)), "{r:?}");
    let r = choose_engine(
        WingetEngine::Winget,
        Err(Refusal("exige Windows 99 ou mais novo".into())),
    )
    .unwrap();
    assert!(matches!(r, Engine::Bridge(None)), "{r:?}");
}

#[test]
fn native_bridge_reason_passes_through() {
    let r = choose_engine(
        WingetEngine::Wingetrs,
        Ok(PlanOrBridge::Bridge(
            "tipo zip: instalado pelo winget".into(),
        )),
    )
    .unwrap();
    match r {
        Engine::Bridge(Some(m)) => assert!(m.contains("tipo zip"), "{m}"),
        other => panic!("{other:?}"),
    }
    let m = manifest_with("inno", &[(Some("user"), None)], URL, SHA, "");
    let ok = planned(&m, ScopeWanted::Prefer(Scope::User), None);
    assert!(matches!(
        choose_engine(WingetEngine::Wingetrs, ok),
        Ok(Engine::Native(_))
    ));
}

#[test]
fn refusal_is_not_bridged_under_wingetrs() {
    let r = choose_engine(
        WingetEngine::Wingetrs,
        Err(Refusal("exige Windows 99 ou mais novo".into())),
    );
    assert_eq!(r.unwrap_err().0, "exige Windows 99 ou mais novo");
}

// ---- route, scope, targets ----

#[test]
fn route_follows_upgrade_behavior() {
    assert_eq!(upgrade_route(&one_installer(None)), UpgradeRoute::InPlace);
    assert_eq!(
        upgrade_route(&one_installer(Some("install"))),
        UpgradeRoute::InPlace
    );
    assert_eq!(
        upgrade_route(&one_installer(Some("uninstallPrevious"))),
        UpgradeRoute::UninstallFirst
    );
    assert_eq!(
        upgrade_route(&one_installer(Some("UNINSTALLPREVIOUS"))),
        UpgradeRoute::UninstallFirst
    );
    assert_eq!(
        upgrade_route(&one_installer(Some("deny"))),
        UpgradeRoute::Denied
    );
    assert_eq!(
        upgrade_route(&one_installer(Some("Deny"))),
        UpgradeRoute::Denied
    );
}

fn row(name: &str, origin: &str, available: bool) -> ListRow {
    ListRow {
        name: name.into(),
        origin: origin.into(),
        installed_version: Some("1.0".into()),
        state: if available {
            UpdateState::Available {
                latest: "2.0".into(),
            }
        } else {
            UpdateState::UpToDate
        },
        ..ListRow::default()
    }
}

#[test]
fn explicit_upgrade_excluded_from_all() {
    let rows = vec![
        row("Explicit", "arp", true),
        row("Normal", "arp", true),
        row("Current", "arp", false),
    ];
    let pins = BTreeSet::new();
    let explicit = |r: &ListRow| r.name == "Explicit";
    let got = upgrade_all_targets(&rows, &pins, &explicit);
    assert_eq!(
        got.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec!["Normal"]
    );
    // Without the explicit filter both come back (what a named target sees).
    let got = upgrade_all_targets(&rows, &pins, &|_| false);
    assert_eq!(got.len(), 2);
}

#[test]
fn pinned_excluded_from_all() {
    let rows = vec![row("Pinned", "arp", true), row("Normal", "arp", true)];
    let mut pins = BTreeSet::new();
    pins.insert(wingetrs::pins::key("arp", "Pinned"));
    let got = upgrade_all_targets(&rows, &pins, &|_| false);
    assert_eq!(
        got.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        vec!["Normal"]
    );
}

#[test]
fn scope_follows_installed() {
    assert_eq!(
        upgrade_scope(Some(InstallScope::Machine), DefaultScope::User),
        ScopeWanted::Only(Scope::Machine)
    );
    assert_eq!(
        upgrade_scope(Some(InstallScope::User), DefaultScope::Machine),
        ScopeWanted::Only(Scope::User)
    );
    assert_eq!(
        upgrade_scope(None, DefaultScope::Machine),
        ScopeWanted::Prefer(Scope::Machine)
    );
    assert_eq!(
        upgrade_scope(None, DefaultScope::User),
        ScopeWanted::Prefer(Scope::User)
    );
}

#[test]
fn native_agrees_with_winget_applicability() {
    let user = Some(InstallScope::User);
    let cases: Vec<(&str, Entries)> = vec![
        ("deny", vec![(Some("user"), Some("deny"))]),
        ("only machine", vec![(Some("machine"), None)]),
        (
            "user and machine",
            vec![(Some("machine"), None), (Some("user"), None)],
        ),
        ("no scope", vec![(None, None)]),
        (
            "deny one of two",
            vec![(Some("user"), Some("deny")), (Some("user"), None)],
        ),
    ];
    for (label, entries) in cases {
        let m = manifest_with("inno", &entries, URL, SHA, "");
        let fit = winget_applicability(&m, user, host_arch()).applicability;
        let native_ok = match plan_upgrade(
            &m,
            &host(),
            upgrade_scope(user, DefaultScope::User),
            "0.9",
            &InstalledFacts::default(),
        ) {
            Ok(PlanOrBridge::Native(p)) => upgrade_route(&p.installer) != UpgradeRoute::Denied,
            Ok(PlanOrBridge::Bridge(_)) => panic!("{label}: bridge"),
            Err(_) => false,
        };
        match fit {
            Applicability::Ok => assert!(native_ok, "{label}: winget accepts, native refuses"),
            Applicability::Denied | Applicability::ScopeMismatch { .. } => {
                assert!(!native_ok, "{label}: winget refuses, native accepts")
            }
            Applicability::Unknown => panic!("{label}: unknown"),
        }
    }

    // System architecture listed in UnsupportedOSArchitectures: both sides refuse
    // (installer-level and root-level lists).
    let extra = format!(
        "  UnsupportedOSArchitectures:
  - {}
",
        host_arch()
    );
    let m = manifest_with("inno", &[(None, None)], URL, SHA, &extra);
    let native = plan_upgrade(
        &m,
        &host(),
        upgrade_scope(user, DefaultScope::User),
        "0.9",
        &InstalledFacts::default(),
    );
    assert!(
        native.is_err(),
        "native accepts an unsupported architecture"
    );
    let fit = winget_applicability(&m, user, host_arch()).applicability;
    assert!(
        !matches!(fit, Applicability::Ok),
        "bridge accepts an unsupported architecture: {fit:?}"
    );
    let root = format!(
        "UnsupportedOSArchitectures:
- {}
",
        host_arch()
    );
    let m = manifest_with("inno", &[(None, None)], URL, SHA, "");
    let yaml_root = manifest::parse(
        format!(
            "{root}PackageIdentifier: Pub.App
PackageVersion: '1.0'
Installers:
- Architecture: {}
  InstallerType: inno
  InstallerUrl: {URL}
  InstallerSha256: {SHA}
",
            host_arch()
        )
        .as_bytes(),
    )
    .unwrap();
    assert!(!matches!(
        winget_applicability(&yaml_root, user, host_arch()).applicability,
        Applicability::Ok
    ));
    assert!(matches!(
        winget_applicability(&m, user, host_arch()).applicability,
        Applicability::Ok
    ));

    // The only arch-compatible installer is unsupported here and a second one (other
    // architecture) is denied: native and bridge must give the same answer (not Ok).
    let other = "neutral";
    let a = host_arch();
    let yaml = [
        "PackageIdentifier: Pub.App
PackageVersion: '1.0'
Installers:
"
        .to_string(),
        format!(
            "- Architecture: {a}
  InstallerType: inno
"
        ),
        format!(
            "  InstallerUrl: {URL}
  InstallerSha256: {SHA}
"
        ),
        format!(
            "  UnsupportedOSArchitectures:
  - {a}
"
        ),
        format!(
            "- Architecture: {other}
  InstallerType: inno
"
        ),
        format!(
            "  UpgradeBehavior: deny
  InstallerUrl: {URL}
  InstallerSha256: {SHA}
"
        ),
    ]
    .concat();
    let m = manifest::parse(yaml.as_bytes()).unwrap();
    let denied_only = manifest_with("inno", &[(None, Some("deny"))], URL, SHA, "");
    let refusal = |m: &MergedManifest| match plan_upgrade(
        m,
        &host(),
        upgrade_scope(user, DefaultScope::User),
        "0.9",
        &InstalledFacts::default(),
    ) {
        Err(e) => e.0,
        Ok(_) => panic!("native accepts"),
    };
    // the unsupported installer is dropped before the deny rule: the answer is "denied"
    assert_eq!(refusal(&m), refusal(&denied_only));
    let fit = winget_applicability(&m, user, host_arch()).applicability;
    assert!(matches!(fit, Applicability::Denied), "bridge: {fit:?}");
}

// ---- upgrade flow ----

type Events = Arc<Mutex<Vec<String>>>;

/// Records `label` for each launch; `params` of the launches are kept too.
struct FakeLauncher {
    label: &'static str,
    events: Events,
    params: Mutex<Vec<String>>,
    /// Exit code of the launched program.
    code: i32,
    /// `install.lock` path: each launch records whether another taker would be turned away.
    lock_path: PathBuf,
    lock_held: Mutex<Vec<bool>>,
    /// When set, records `launch_gate` before its label if the gate was already taken.
    gate: Option<LaunchGate>,
    /// When set, the launch fails with this OS error instead of running.
    os_error: Option<i32>,
}

impl Launcher for FakeLauncher {
    fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32> {
        if let Some(code) = self.os_error {
            return Err(std::io::Error::from_raw_os_error(code));
        }
        if let Some(g) = &self.gate
            && g.running().is_some()
        {
            self.events.lock().unwrap().push("launch_gate".to_string());
        }
        self.events.lock().unwrap().push(self.label.to_string());
        self.params.lock().unwrap().push(spec.params.clone());
        let other = UpdateLock::try_acquire(&self.lock_path).unwrap();
        self.lock_held.lock().unwrap().push(other.is_none());
        Ok(self.code)
    }
}

struct NoElevated;

impl ElevatedRunner for NoElevated {
    fn run_installer(
        &self,
        _req: &InstallerRequest,
        _on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        panic!("not used");
    }
    fn run_uninstaller(
        &self,
        _hive: MachineHive,
        _key: &str,
        _rule: Rule,
        _nsis_hint: bool,
    ) -> Result<i32, ElevatedError> {
        panic!("not used");
    }
    fn delete_arp_key(
        &self,
        _hive: MachineHive,
        _key: &str,
        _reason: DeleteReason,
    ) -> Result<(), ElevatedError> {
        panic!("not used");
    }
}

struct NoMsix;

impl MsixOps for NoMsix {
    fn add(&self, _file: &Path) -> Result<AddResult, MsixError> {
        panic!("not used");
    }
    fn remove(&self, _family: &str) -> Result<usize, MsixError> {
        panic!("not used");
    }
}

#[derive(Default)]
struct FakeClock {
    now: Mutex<Duration>,
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap()
    }
    fn sleep(&self, d: Duration) {
        *self.now.lock().unwrap() += d;
    }
}

fn known() -> KnownFolders {
    KnownFolders {
        program_files: PathBuf::from(r"C:\Program Files"),
        program_files_x86: PathBuf::from(r"C:\Program Files (x86)"),
        system_root: PathBuf::from(r"C:\Windows"),
        program_data: PathBuf::from(r"C:\ProgramData"),
        user_profiles: PathBuf::from(r"C:\Users"),
    }
}

fn user_folders() -> UserFolders {
    UserFolders {
        local_app_data: PathBuf::from(r"C:\Users\t\AppData\Local"),
        app_data: PathBuf::from(r"C:\Users\t\AppData\Roaming"),
        documents: PathBuf::from(r"C:\Users\t\Documents"),
        wingetrs_root: PathBuf::from(r"C:\Users\t\AppData\Local\wingetrs"),
    }
}

fn previous_arp() -> ArpDetails {
    ArpDetails {
        hive: Hive::User,
        key: "App".to_string(),
        display_name: Some("App".to_string()),
        publisher: Some("Pub".to_string()),
        display_version: Some("0.9".to_string()),
        windows_installer: false,
        system_component: false,
        quiet_uninstall_string: Some(r#""C:\Users\t\AppData\Local\App\u.exe" /S"#.to_string()),
        uninstall_string: None,
        inno_app_path: None,
        bundle_cache_path: None,
        bundle_upgrade_code: None,
        install_location: None,
    }
}

fn previous_plan() -> UninstallPlan {
    let d = previous_arp();
    plan_uninstall(&d, false, &[], true, &known(), &user_folders(), &|_| false).unwrap()
}

fn body() -> Vec<u8> {
    (0..64 * 1024).map(|i| (i * 31 % 251) as u8).collect()
}

struct World {
    _tmp: tempfile::TempDir,
    _server: MockServer,
    data: DataDir,
    http: Http,
    events: Events,
    install: FakeLauncher,
    uninstall: FakeLauncher,
    clock: FakeClock,
    /// Whether the ARP key is still there after the uninstaller ran.
    key_stays: bool,
    manifest: MergedManifest,
    gate: LaunchGate,
    /// Version of the app the post-install scan finds (`None`: the scan finds nothing).
    found_version: Option<String>,
}

/// Serves the installer and records `fetch` for each request.
struct Served {
    events: Events,
    bytes: Vec<u8>,
}

impl wiremock::Respond for Served {
    fn respond(&self, _req: &wiremock::Request) -> ResponseTemplate {
        self.events.lock().unwrap().push("fetch".to_string());
        ResponseTemplate::new(200).set_body_bytes(self.bytes.clone())
    }
}

impl World {
    async fn new(kind: &str, behavior: Option<&str>, extra: &str, key_stays: bool) -> World {
        let tmp = tempfile::tempdir().unwrap();
        let data = DataDir::new(tmp.path().join("root"));
        let bytes = body();
        let sha = sha256_hex(&bytes);
        let server = MockServer::start().await;
        let events: Events = Arc::default();
        Mock::given(method("GET"))
            .and(path("/setup"))
            .respond_with(Served {
                events: events.clone(),
                bytes,
            })
            .mount(&server)
            .await;
        let manifest = manifest_with(
            kind,
            &[(Some("user"), behavior)],
            &format!("{}/setup", server.uri()),
            &sha,
            extra,
        );
        let gate = LaunchGate::default();
        let launcher = |label, gate| FakeLauncher {
            label,
            events: events.clone(),
            params: Mutex::new(Vec::new()),
            code: 0,
            lock_path: data.install_layout().install_lock(),
            lock_held: Mutex::new(Vec::new()),
            gate,
            os_error: None,
        };
        World {
            install: launcher("install", None),
            uninstall: launcher("uninstall", Some(gate.clone())),
            events: events.clone(),
            _tmp: tmp,
            _server: server,
            data,
            http: Http::new().unwrap(),
            clock: FakeClock::default(),
            key_stays,
            manifest,
            gate,
            found_version: None,
        }
    }

    fn plan(&self, from: Option<&str>) -> WingetPlan {
        native(&self.manifest, ScopeWanted::Only(Scope::User), from)
    }

    async fn run(&self, p: &WingetPlan) -> Result<(), UpgradeError> {
        self.run_with(p, &previous_plan()).await
    }

    async fn run_with(&self, p: &WingetPlan, previous: &UninstallPlan) -> Result<(), UpgradeError> {
        self.run_report(p, previous).await.map(|_| ())
    }

    async fn run_report(
        &self,
        p: &WingetPlan,
        previous: &UninstallPlan,
    ) -> Result<InstallReport, UpgradeError> {
        let found = self.found_version.clone();
        let scan = move || match &found {
            Some(v) => vec![InstalledApp {
                source: InstallSource::Arp(Hive::User),
                key: "App".to_string(),
                name: "Contoso App".to_string(),
                publisher: Some("Contoso".to_string()),
                version: Some(v.clone()),
                product_code: None,
                upgrade_code: None,
                family_name: None,
                scoop_bucket: None,
                stale: None,
            }],
            None => Vec::new(),
        };
        let msiexec = self._tmp.path().join("msiexec.exe");
        let wctx = WingetCtx {
            data: &self.data,
            http: &self.http,
            wait: LockWait::Block(Duration::from_secs(30)),
            progress: &|_: &ProgressEvent| {},
            gate: self.gate.clone(),
            launcher: &self.install,
            elevated: &NoElevated,
            msix: &NoMsix,
            scan: &scan,
            msiexec: msiexec.clone(),
            busy: &|| false,
            sleep: &|_: Duration| {},
        };
        let key_exists = |_: Hive, _: &str| {
            let mut ev = self.events.lock().unwrap();
            if ev.last().map(String::as_str) != Some("wait_key") {
                ev.push("wait_key".to_string());
            }
            self.key_stays
        };
        let uctx = UninstallCtx {
            launcher: &self.uninstall,
            elevated: &NoElevated,
            msix: &NoMsix,
            key_exists: &key_exists,
            clock: &self.clock,
            user_expand: &|_: &str| None,
            msiexec: Path::new(r"C:\Windows\System32\msiexec.exe"),
        };
        upgrade(&wctx, &uctx, p, Some(previous)).await
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_previous_order() {
    let w = World::new("inno", Some("uninstallPrevious"), "", false).await;
    let p = w.plan(Some("0.9"));
    w.run(&p).await.unwrap();
    assert_eq!(
        w.events(),
        ["fetch", "launch_gate", "uninstall", "wait_key", "install"]
    );

    // The key stays: the uninstaller said success without effect; no install.
    let w = World::new("inno", Some("uninstallPrevious"), "", true).await;
    let p = w.plan(Some("0.9"));
    let err = w.run(&p).await.unwrap_err();
    assert!(matches!(err, UpgradeError::NotRemoved(_)), "{err}");
    assert_eq!(
        w.events(),
        ["fetch", "launch_gate", "uninstall", "wait_key"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deny_refuses() {
    let w = World::new("inno", Some("deny"), "", false).await;
    let p = w.plan(Some("0.9"));
    let err = w.run(&p).await.unwrap_err();
    assert!(err.to_string().contains("não permite atualizar"), "{err}");
    assert!(w.events().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn in_place_uses_upgrade_switch() {
    let extra = "  InstallerSwitches:\n    Upgrade: REINSTALL=ALL\n";
    let w = World::new("msi", None, extra, false).await;
    let p = w.plan(Some("0.9"));
    w.run(&p).await.unwrap();
    assert_eq!(w.events(), ["fetch", "install"]);
    let params = w.install.params.lock().unwrap().clone();
    assert!(params[0].ends_with("REINSTALL=ALL"), "{params:?}");

    // A fresh install (no `upgrade_from`) never carries it.
    let w = World::new("msi", None, extra, false).await;
    let p = w.plan(None);
    let ctx_events = w.events();
    assert!(ctx_events.is_empty());
    let cmd = p.command.clone().unwrap();
    assert!(!cmd.args.contains("REINSTALL"), "{}", cmd.args);
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstall_first_install_has_no_upgrade_switch() {
    let extra = "  InstallerSwitches:\n    Upgrade: REINSTALL=ALL\n";
    let w = World::new("msi", Some("uninstallPrevious"), extra, false).await;
    let p = w.plan(Some("0.9"));
    assert!(p.command.as_ref().unwrap().args.contains("REINSTALL=ALL"));
    w.run(&p).await.unwrap();
    assert_eq!(
        w.events(),
        ["fetch", "launch_gate", "uninstall", "wait_key", "install"]
    );
    let params = w.install.params.lock().unwrap().clone();
    assert!(!params[0].contains("REINSTALL"), "{params:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_needs_a_greater_version() {
    // The manifest is 1.0.
    for (installed, ok) in [
        ("0.9", true),
        ("0.9.9.9", true),
        ("1.1", false),
        ("1.0", false),
        ("1.0.0", false),
        ("1.0.0.0", false),
    ] {
        let w = World::new("inno", None, "", false).await;
        let p = w.plan(Some(installed));
        let r = w.run(&p).await;
        assert_eq!(r.is_ok(), ok, "installed {installed}: {r:?}");
        if !ok {
            assert!(matches!(r, Err(UpgradeError::NotNewer { .. })), "{r:?}");
            assert!(w.events().is_empty());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_previous_version_is_an_error() {
    let w = World::new("inno", None, "", false).await;
    let p = w.plan(None);
    let err = w.run(&p).await.unwrap_err();
    assert!(matches!(err, UpgradeError::NoPrevious), "{err}");
    assert!(w.events().is_empty());
}
#[tokio::test(flavor = "multi_thread")]
async fn install_failing_after_removal_says_so() {
    let mut w = World::new("inno", Some("uninstallPrevious"), "", false).await;
    w.install.code = 1;
    let p = w.plan(Some("0.9"));
    let err = w.run(&p).await.unwrap_err();
    assert!(matches!(err, UpgradeError::InstallAfterRemoval(_)), "{err}");
    assert!(
        err.to_string()
            .starts_with("a versão anterior foi removida e a nova não instalou: "),
        "{err}"
    );
    assert_eq!(
        w.events(),
        ["fetch", "launch_gate", "uninstall", "wait_key", "install"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn removal_and_install_run_under_one_lock() {
    let w = World::new("inno", Some("uninstallPrevious"), "", false).await;
    let p = w.plan(Some("0.9"));
    w.run(&p).await.unwrap();
    // Another taker is turned away while the uninstaller runs and while the installer runs.
    assert_eq!(*w.uninstall.lock_held.lock().unwrap(), [true]);
    assert_eq!(*w.install.lock_held.lock().unwrap(), [true]);
    // Released at the end.
    assert!(
        UpdateLock::try_acquire(&w.data.install_layout().install_lock())
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstall_error_stops_before_install() {
    let w = World::new("inno", Some("uninstallPrevious"), "", false).await;
    let p = w.plan(Some("0.9"));
    let mut prev = previous_plan();
    prev.command = None;
    let err = w.run_with(&p, &prev).await.unwrap_err();
    assert!(matches!(err, UpgradeError::Uninstall(_)), "{err}");
    assert_eq!(w.events(), ["fetch"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_previous_download_failure_keeps_old() {
    let w = World::new("inno", Some("uninstallPrevious"), "", false).await;
    w._server.reset().await;
    Mock::given(method("GET"))
        .and(path("/setup"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&w._server)
        .await;
    let p = w.plan(Some("0.9"));
    let err = w.run(&p).await.unwrap_err();
    assert!(
        matches!(err, UpgradeError::Install(WingetOpsError::Fetch(_))),
        "{err}"
    );
    // The old version is untouched: no uninstaller, no wait for its key, no install.
    assert!(w.events().is_empty(), "{:?}", w.events());
    assert!(w.uninstall.params.lock().unwrap().is_empty());

    // A download that does not match the manifest's hash removes nothing either.
    let w = World::new("inno", Some("uninstallPrevious"), "", false).await;
    w._server.reset().await;
    Mock::given(method("GET"))
        .and(path("/setup"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"other bytes".to_vec()))
        .mount(&w._server)
        .await;
    let err = w.run(&p).await.unwrap_err();
    assert!(
        matches!(err, UpgradeError::Install(WingetOpsError::Fetch(_))),
        "{err}"
    );
    assert!(w.events().is_empty(), "{:?}", w.events());
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_previous_cancel_before_removal_keeps_old() {
    let w = World::new("inno", Some("uninstallPrevious"), "", false).await;
    // Ctrl+C came first: the download may finish, the removal never starts.
    assert!(w.gate.try_cancel());
    let p = w.plan(Some("0.9"));
    let err = w.run(&p).await.unwrap_err();
    assert!(
        matches!(err, UpgradeError::Install(WingetOpsError::Cancelled)),
        "{err}"
    );
    assert_eq!(w.events(), ["fetch"]);
}

#[test]
fn upgrade_previous_confirmation_mentions_removal() {
    const LINE: &str = "remove a versão instalada antes (o desinstalador pode abrir janela ou pedir permissão de administrador)";
    let m = manifest_with(
        "inno",
        &[(Some("user"), Some("uninstallPrevious"))],
        URL,
        SHA,
        "",
    );
    let text = native(&m, ScopeWanted::Only(Scope::User), Some("0.9")).to_string();
    assert!(text.contains(LINE), "{text}");
    // A fresh install of the same package removes nothing.
    let text = native(&m, ScopeWanted::Only(Scope::User), None).to_string();
    assert!(!text.contains("remove a versão instalada"), "{text}");
    // An in-place upgrade neither.
    let m = manifest_with("inno", &[(Some("user"), Some("install"))], URL, SHA, "");
    let text = native(&m, ScopeWanted::Only(Scope::User), Some("0.9")).to_string();
    assert!(!text.contains("remove a versão instalada"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_same_version_after_warns() {
    // (version offered, version found after the installer, warns)
    for (offered, found, warns) in [
        ("1.10", "1.9", true),
        ("1.0", "1.0.0", false),
        ("1.0", "1.0", false),
        ("1.0", "0.9", true),
        // Newer than the catalog (the installer brought a later build): updated, no warning.
        ("1.0", "1.1", false),
    ] {
        for behavior in [None, Some("uninstallPrevious")] {
            let mut w = World::new("inno", behavior, "", false).await;
            w.found_version = Some(found.to_string());
            let mut p = w.plan(Some("0.9"));
            p.version = offered.to_string();
            let r = w.run_report(&p, &previous_plan()).await.unwrap();
            let want = format!("o instalador terminou, mas a versão instalada continua {found}");
            if warns {
                assert_eq!(
                    r.warning.as_deref(),
                    Some(want.as_str()),
                    "{offered} {found}"
                );
            } else {
                assert_eq!(r.warning, None, "{offered} {found} {behavior:?}");
            }
        }
    }
}

fn arp(hive: Hive, key: &str, msi: bool) -> ArpDetails {
    let mut d = previous_arp();
    d.hive = hive;
    d.key = key.to_string();
    d.windows_installer = msi;
    d
}

#[test]
fn installed_facts_from_the_registry() {
    let f = installed_facts(&arp(Hive::MachineX86, "{GUID}", true));
    assert_eq!((f.arch, f.kind), (Some("x86"), Some(InstalledKind::Msi)));
    let f = installed_facts(&arp(Hive::MachineX64, "App_is1", false));
    assert_eq!((f.arch, f.kind), (Some("x64"), Some(InstalledKind::Exe)));
    let f = installed_facts(&arp(Hive::User, "Other", false));
    assert_eq!((f.arch, f.kind), (None, None));
}

const ZERO_SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One installer line per `(arch, type, behavior)`; the URL ends with `/<arch>-<type>`.
fn multi(entries: &[(&str, &str, Option<&str>)]) -> MergedManifest {
    let mut yaml = "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nInstallers:\n".to_string();
    for (arch, kind, behavior) in entries {
        yaml += &format!("- Architecture: {arch}\n  InstallerType: {kind}\n");
        if let Some(b) = behavior {
            yaml += &format!("  UpgradeBehavior: {b}\n");
        }
        yaml += &format!(
            "  InstallerUrl: https://example.invalid/{arch}-{kind}\n  InstallerSha256: {ZERO_SHA}\n"
        );
    }
    manifest::parse(yaml.as_bytes()).unwrap()
}

fn x64_host() -> Host {
    Host {
        arch: "x64".into(),
        ..host()
    }
}

fn upgrade_url(m: &MergedManifest, facts: &InstalledFacts) -> Result<String, Refusal> {
    let wanted = ScopeWanted::Prefer(Scope::User);
    match plan_upgrade(m, &x64_host(), wanted, "0.9", facts)? {
        PlanOrBridge::Native(p) => Ok(p.installer.url),
        PlanOrBridge::Bridge(w) => panic!("{w}"),
    }
}

#[test]
fn installed_architecture_is_kept() {
    let m = multi(&[("x64", "inno", None), ("x86", "inno", None)]);
    // Without facts the native architecture wins; an x86 install stays x86.
    assert!(
        upgrade_url(&m, &InstalledFacts::default())
            .unwrap()
            .ends_with("/x64-inno")
    );
    let x86 = installed_facts(&arp(Hive::MachineX86, "App_is1", false));
    assert!(upgrade_url(&m, &x86).unwrap().ends_with("/x86-inno"));
    // An installed architecture the manifest does not offer changes nothing.
    let arm = InstalledFacts {
        arch: Some("arm64"),
        kind: None,
    };
    assert!(upgrade_url(&m, &arm).unwrap().ends_with("/x64-inno"));
}

#[test]
fn installed_type_filters_installers() {
    let m = multi(&[("x64", "msi", None), ("x64", "inno", None)]);
    let facts = |kind| InstalledFacts {
        arch: None,
        kind: Some(kind),
    };
    assert!(
        upgrade_url(&m, &facts(InstalledKind::Exe))
            .unwrap()
            .ends_with("/x64-inno")
    );
    assert!(
        upgrade_url(&m, &facts(InstalledKind::Msi))
            .unwrap()
            .ends_with("/x64-msi")
    );
    assert!(upgrade_url(&m, &facts(InstalledKind::Msix)).is_err());
}

#[test]
fn deny_is_judged_among_installers_that_run_here() {
    // The only x64 installer denies; the arm64 one does not run on x64: denied.
    let m = multi(&[("x64", "inno", Some("deny")), ("arm64", "inno", None)]);
    let r = upgrade_url(&m, &InstalledFacts::default());
    assert!(r.unwrap_err().0.contains("não permite atualizar"));
    let fit = winget_applicability(&m, None, "x64").applicability;
    assert_eq!(fit, Applicability::Denied);
}

#[test]
fn msix_plan_never_says_installer_window() {
    // NanaZip, WinDbg: MSIX has no silent switch, but the package manager deploys it without
    // an installer window.
    for kind in ["msix", "appx"] {
        let m = manifest_with(kind, &[(Some("user"), None)], URL, SHA, "");
        let p = native(&m, ScopeWanted::Only(Scope::User), None);
        assert!(!p.interactive, "{kind}");
        assert!(!p.to_string().contains("abre a janela"), "{p}");
    }
    // An exe with no silent mode still says it.
    let m = manifest_with("exe", &[(Some("user"), None)], URL, SHA, "");
    let p = native(&m, ScopeWanted::Only(Scope::User), None);
    assert!(p.to_string().contains("abre a janela do instalador"), "{p}");
}

/// The ARP record of a .NET runtime bundle (Burn): always in the 32-bit view.
fn dotnet_runtime(major: &str, arch: &str) -> ArpDetails {
    let mut d = arp(
        Hive::MachineX86,
        "{0DDB00D8-AAAA-BBBB-CCCC-DDDDDDDDDDDD}",
        false,
    );
    d.display_name = Some(format!(
        "Microsoft Windows Desktop Runtime - {major}.0.11 ({arch})"
    ));
    d.quiet_uninstall_string = None;
    d.uninstall_string = Some(format!(
        r#""C:\ProgramData\Package Cache\{{0DDB00D8}}\windowsdesktop-runtime-{major}.0.11-win-{arch}.exe"  /uninstall"#
    ));
    d.bundle_cache_path = Some(format!(
        r"C:\ProgramData\Package Cache\{{0DDB00D8}}\windowsdesktop-runtime-{major}.0.11-win-{arch}.exe"
    ));
    d.bundle_upgrade_code = Some("{C0C0C0C0-1111-2222-3333-444444444444}".to_string());
    d
}

#[test]
fn burn_bundle_arch_is_not_the_registry_view() {
    let m = multi(&[
        ("x86", "burn", None),
        ("x64", "burn", None),
        ("arm64", "burn", None),
    ]);
    for major in ["8", "10"] {
        let x64 = installed_facts(&dotnet_runtime(major, "x64"));
        assert_eq!(x64.arch, Some("x64"), "{major}");
        assert!(
            upgrade_url(&m, &x64).unwrap().ends_with("/x64-burn"),
            "{major}"
        );
        // A real x86 runtime stays x86.
        let x86 = installed_facts(&dotnet_runtime(major, "x86"));
        assert_eq!(x86.arch, Some("x86"));
        assert!(upgrade_url(&m, &x86).unwrap().ends_with("/x86-burn"));
    }
    // No hint in the name or the cache: unknown, the system architecture wins.
    let mut d = dotnet_runtime("8", "x64");
    d.display_name = Some("Microsoft Windows Desktop Runtime - 8.0.11".to_string());
    d.bundle_cache_path = Some(r"C:\ProgramData\Package Cache\{X}\setup.exe".to_string());
    let f = installed_facts(&d);
    assert_eq!(f.arch, None);
    assert!(upgrade_url(&m, &f).unwrap().ends_with("/x64-burn"));
    // A bundle known only by its upgrade code is still a bundle.
    d.bundle_cache_path = None;
    assert_eq!(installed_facts(&d).arch, None);
    // Not a bundle: the 32-bit view still means x86.
    assert_eq!(
        installed_facts(&arp(Hive::MachineX86, "App_is1", false)).arch,
        Some("x86")
    );
}
