#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Native install flow (Task 181).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use wingetrs::bridge::elevated::{ElevatedError, ElevatedRunner};
use wingetrs::bridge::worker::{DeleteReason, MachineHive, PackageRequest};
use wingetrs::cache::{DataDir, UpdateLock, sha256_hex};
use wingetrs::http::Http;
use wingetrs::install::executor::LaunchGate;
use wingetrs::install::progress::{Phase, ProgressEvent, phase_status, phase_title};
use wingetrs::installed::{Hive, InstallSource, InstalledApp};
use wingetrs::sources::winget::manifest::{self, MergedManifest};
use wingetrs::tui::theme::{GlyphSet, Glyphs};
use wingetrs::update::LockWait;
use wingetrs::winget_install::codes::{Outcome, UAC_REFUSED};
use wingetrs::winget_install::launch::{LaunchSpec, Launcher, Show, Verb};
use wingetrs::winget_install::msix::{AddResult, MsixError, MsixOps};
use wingetrs::winget_install::ops::{WingetCtx, WingetOpsError, install};
use wingetrs::winget_install::plan::{
    ADMIN_PROMPT, MSIX_SCOPE_STEP, MSIX_STEP, PlanOrBridge, RUN_FOLDER_NOTE, SELF_ADMIN_PROMPT,
    WingetPlan, plan, plan_with_mode,
};
use wingetrs::winget_install::post_check::find_installed;
use wingetrs::winget_install::select::{Host, Scope, ScopeWanted};
use wingetrs::winget_install::uninstall_cmd::Rule;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PRODUCT_CODE: &str = "{11111111-2222-3333-4444-555555555555}";
const UPGRADE_CODE: &str = "{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}";

// ---- fakes ----

#[derive(Default)]
struct FakeLauncher {
    calls: Mutex<Vec<LaunchSpec>>,
    count: Arc<AtomicUsize>,
    codes: Mutex<VecDeque<i32>>,
    /// Gate to try to cancel while "running", and what `try_cancel` answered.
    cancel_during_run: Mutex<Option<(LaunchGate, Option<bool>)>>,
    /// When set, the next launch fails with this OS error (nothing runs).
    os_error: Mutex<Option<i32>>,
    /// While "running": try to rename and to delete the launched file; whether each worked.
    probe_hold: AtomicBool,
    probed: Mutex<Vec<(&'static str, bool)>>,
    /// The names in the launched program's folder, while it "runs".
    seen_in_dir: Mutex<Vec<Vec<String>>>,
    /// While "running": write this line to the installer log named in the parameters.
    log_line: Mutex<Option<String>>,
}

/// The `/log "<path>"` of a filled command line.
fn log_in(params: &str) -> PathBuf {
    let (_, rest) = params.rsplit_once("/log \"").expect("a /log switch");
    PathBuf::from(rest.split('"').next().unwrap())
}

impl Launcher for FakeLauncher {
    fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32> {
        self.calls.lock().unwrap().push(spec.clone());
        self.count.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.os_error.lock().unwrap().take() {
            return Err(std::io::Error::from_raw_os_error(code));
        }
        if let Some((gate, answer)) = self.cancel_during_run.lock().unwrap().as_mut() {
            *answer = Some(gate.try_cancel());
        }
        if self.probe_hold.load(Ordering::SeqCst) {
            let moved = spec.program.with_extension("moved");
            let renamed = std::fs::rename(&spec.program, &moved).is_ok();
            let from = if renamed { &moved } else { &spec.program };
            let deleted = std::fs::remove_file(from).is_ok();
            let mut probed = self.probed.lock().unwrap();
            probed.push(("rename", renamed));
            probed.push(("delete", deleted));
        }
        if let Some(dir) = spec.program.parent()
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            let mut names: Vec<String> = entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            self.seen_in_dir.lock().unwrap().push(names);
        }
        if let Some(text) = self.log_line.lock().unwrap().as_deref() {
            // UTF-16 with a BOM, as msiexec writes it.
            let mut bytes = vec![0xFF, 0xFE];
            for u in format!("{text}\r\n").encode_utf16() {
                bytes.extend(u.to_le_bytes());
            }
            std::fs::write(log_in(&spec.params), bytes).unwrap();
        }
        Ok(self.codes.lock().unwrap().pop_front().unwrap_or(0))
    }
}

/// The session's helper. Uninstalls and key deletions are never asked by an install flow.
/// Installs (Task 202) get the queued answers in order; with none left, the helper cannot exist
/// (wingetrs outside an administrator-only folder) and the flow falls back to the installer's
/// own UAC prompt (Task 196).
#[derive(Default)]
struct FakeElevated {
    calls: Mutex<Vec<String>>,
    installs: Mutex<VecDeque<Result<i32, ElevatedError>>>,
    requests: Mutex<Vec<PackageRequest>>,
    /// Lines sent back on every install request, before its answer.
    lines: Mutex<Vec<String>>,
}

impl FakeElevated {
    fn answer(&self, r: Result<i32, ElevatedError>) {
        self.installs.lock().unwrap().push_back(r);
    }

    fn requests(&self) -> Vec<PackageRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl ElevatedRunner for FakeElevated {
    fn run_uninstaller(
        &self,
        _hive: MachineHive,
        key: &str,
        _rule: Rule,
        _nsis_hint: bool,
    ) -> Result<i32, ElevatedError> {
        self.calls.lock().unwrap().push(format!("uninstall {key}"));
        Err(ElevatedError::Remote("not used".into()))
    }
    fn run_uninstaller_lines(
        &self,
        _hive: MachineHive,
        key: &str,
        _rule: Rule,
        _nsis_hint: bool,
        _on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        self.calls.lock().unwrap().push(format!("uninstall {key}"));
        Err(ElevatedError::Remote("not used".into()))
    }
    fn delete_arp_key(
        &self,
        _hive: MachineHive,
        key: &str,
        _reason: DeleteReason,
    ) -> Result<(), ElevatedError> {
        self.calls.lock().unwrap().push(format!("delete {key}"));
        Err(ElevatedError::Remote("not used".into()))
    }
    fn install_package_lines(
        &self,
        req: &PackageRequest,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        self.requests.lock().unwrap().push(req.clone());
        for l in self.lines.lock().unwrap().iter() {
            on_line(l);
        }
        self.installs
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                Err(ElevatedError::NotAdminFolder(
                    wingetrs::bridge::helper_folder::refusal_text(Path::new(
                        r"C:\Users\u\Downloads",
                    )),
                ))
            })
    }
}
#[derive(Default)]
struct FakeMsix {
    adds: Mutex<Vec<PathBuf>>,
    fail: Mutex<Option<MsixError>>,
}

impl MsixOps for FakeMsix {
    fn add(&self, file: &Path) -> Result<AddResult, MsixError> {
        self.adds.lock().unwrap().push(file.to_path_buf());
        match self.fail.lock().unwrap().take() {
            Some(e) => Err(e),
            None => Ok(AddResult::Installed),
        }
    }
    fn remove(&self, _family: &str) -> Result<usize, MsixError> {
        Ok(0)
    }
}

type PhaseHook = Box<dyn FnMut(&Phase) + Send>;

struct Env {
    tmp: tempfile::TempDir,
    data: DataDir,
    http: Http,
    launcher: FakeLauncher,
    elevated: FakeElevated,
    msix: FakeMsix,
    apps: Mutex<Vec<InstalledApp>>,
    sleeps: Mutex<Vec<Duration>>,
    events: Mutex<Vec<ProgressEvent>>,
    on_phase: Mutex<Option<PhaseHook>>,
    gate: LaunchGate,
    msiexec: PathBuf,
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let data = DataDir::new(tmp.path().join("root"));
        Env {
            msiexec: tmp.path().join("System32").join("msiexec.exe"),
            tmp,
            data,
            http: Http::new().unwrap(),
            launcher: FakeLauncher::default(),
            elevated: FakeElevated::default(),
            msix: FakeMsix::default(),
            apps: Mutex::new(Vec::new()),
            sleeps: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
            on_phase: Mutex::new(None),
            gate: LaunchGate::default(),
        }
    }

    fn root(&self) -> &Path {
        self.data.root()
    }

    fn materialized(&self, ext: &str) -> PathBuf {
        self.root()
            .join("installers")
            .join("Pub.App")
            .join("1.0")
            .join(format!("Pub.App-1.0.{ext}"))
    }

    fn calls(&self) -> (usize, usize, usize) {
        (
            self.launcher.calls.lock().unwrap().len(),
            self.elevated.calls.lock().unwrap().len(),
            self.msix.adds.lock().unwrap().len(),
        )
    }
}

/// The context over `env`'s fakes (temporaries live as long as the `let`).
macro_rules! ctx {
    ($env:expr) => {
        WingetCtx {
            data: &$env.data,
            http: &$env.http,
            wait: LockWait::Block(Duration::from_secs(30)),
            progress: &|ev: &ProgressEvent| {
                $env.events.lock().unwrap().push(ev.clone());
                if let ProgressEvent::Phase(p) = ev
                    && let Some(f) = $env.on_phase.lock().unwrap().as_mut()
                {
                    f(p);
                }
            },
            gate: $env.gate.clone(),
            launcher: &$env.launcher,
            elevated: &$env.elevated,
            msix: &$env.msix,
            scan: &|| $env.apps.lock().unwrap().clone(),
            msiexec: $env.msiexec.clone(),
            busy: &|| false,
            sleep: &|d: Duration| $env.sleeps.lock().unwrap().push(d),
        }
    };
}

// ---- helpers ----

fn host() -> Host {
    Host {
        arch: "x64".into(),
        os: [10, 0, 22631, 0],
        locale: "pt-BR".into(),
    }
}

fn bytes() -> Vec<u8> {
    (0..64 * 1024).map(|i| (i * 31 % 251) as u8).collect()
}

async fn serve(body: Vec<u8>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/setup"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&server)
        .await;
    server
}

/// A manifest with one x64 installer; `extra` goes into the installer, `root` at the top.
fn manifest_of(
    kind: &str,
    scope: Option<&str>,
    url: &str,
    sha: &str,
    extra: &str,
    root: &str,
) -> MergedManifest {
    let scope = scope.map_or(String::new(), |s| format!("  Scope: {s}\n"));
    let yaml = format!(
        "PackageIdentifier: Pub.App\nPackageVersion: '1.0'\nPackageName: Contoso App\nPublisher: Contoso\n{root}Installers:\n- Architecture: x64\n  InstallerType: {kind}\n{scope}  InstallerUrl: {url}\n  InstallerSha256: {sha}\n{extra}"
    );
    manifest::parse(yaml.as_bytes()).unwrap()
}

fn native(m: &MergedManifest, wanted: Scope) -> WingetPlan {
    match plan(m, &host(), ScopeWanted::Prefer(wanted), None).unwrap() {
        PlanOrBridge::Native(p) => p,
        PlanOrBridge::Bridge(why) => panic!("bridge: {why}"),
        PlanOrBridge::Portable(s) => panic!("portable: {s:?}"),
    }
}

fn product_code_line() -> String {
    format!("  ProductCode: '{PRODUCT_CODE}'\n")
}

fn app(key: &str, name: &str, publisher: &str) -> InstalledApp {
    InstalledApp {
        source: InstallSource::Arp(Hive::User),
        key: key.into(),
        name: name.into(),
        publisher: Some(publisher.into()),
        version: None,
        product_code: None,
        upgrade_code: None,
        family_name: None,
        scoop_bucket: None,
        stale: None,
    }
}

/// Plan for a served installer of `kind`/`scope` whose bytes match the SHA.
async fn served_plan(
    kind: &str,
    scope: Option<&str>,
    wanted: Scope,
    extra: &str,
    root: &str,
) -> (MockServer, WingetPlan, String) {
    let body = bytes();
    let sha = sha256_hex(&body);
    let server = serve(body).await;
    let m = manifest_of(
        kind,
        scope,
        &format!("{}/setup", server.uri()),
        &sha,
        extra,
        root,
    );
    (server, native(&m, wanted), sha)
}

/// Holds `msi.lock` like another process; a thread releases it once the flow reports it is
/// waiting, after running `before` (whose result it returns). A flow that never reports
/// waiting fails the thread (when the sender is dropped, or after the 60 s hang detector).
fn hold_msi_lock<T: Send + 'static>(
    env: &Env,
    before: impl FnOnce() -> T + Send + 'static,
) -> std::thread::JoinHandle<T> {
    let held = UpdateLock::try_acquire(&env.data.install_layout().msi_lock())
        .unwrap()
        .expect("msi.lock free at start");
    let (tx, rx) = mpsc::channel::<()>();
    let tx = Mutex::new(Some(tx));
    *env.on_phase.lock().unwrap() = Some(Box::new(move |p: &Phase| {
        if *p == Phase::WaitingInstaller
            && let Some(tx) = tx.lock().unwrap().take()
        {
            tx.send(()).unwrap();
        }
    }));
    std::thread::spawn(move || {
        let signal = rx.recv_timeout(Duration::from_secs(60));
        let out = before();
        drop(held);
        signal.expect("the flow never waited for msi.lock");
        out
    })
}

// ---- tests ----

#[tokio::test]
async fn hash_mismatch_never_launches() {
    // 1) Served bytes differ from the manifest SHA.
    let env = Env::new();
    let body = bytes();
    let wrong = sha256_hex(b"something else");
    let server = serve(body).await;
    let m = manifest_of(
        "inno",
        Some("user"),
        &format!("{}/setup", server.uri()),
        &wrong,
        "",
        "",
    );
    let p = native(&m, Scope::User);
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await;
    assert!(matches!(r, Err(WingetOpsError::Fetch(_))), "{r:?}");
    assert_eq!(env.calls(), (0, 0, 0));

    // 2) The verified file cannot be swapped between the check and the launch: while the
    // flow waits for msi.lock (after verifying), writing to the installer must fail.
    let env = Env::new();
    let (_server, p, _) = served_plan("inno", Some("user"), Scope::User, "", "").await;
    let target = env.materialized("exe");
    let releaser = hold_msi_lock(&env, move || {
        std::fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .and_then(|mut f| std::io::Write::write_all(&mut f, b"MZ evil"))
            .is_ok()
    });
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await;
    *env.on_phase.lock().unwrap() = None;
    let tampered = releaser.join().unwrap();
    assert!(!tampered, "the installer was writable after the hash check");
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(env.calls(), (1, 0, 0));
}

#[tokio::test]
async fn exit_zero_found_is_installed() {
    let env = Env::new();
    let (_s, p, _) = served_plan("inno", Some("user"), Scope::User, &product_code_line(), "").await;
    let mut a = app(
        "{11111111-2222-3333-4444-555555555555}_is1",
        "Other",
        "Other",
    );
    a.product_code = Some(PRODUCT_CODE.into());
    env.apps.lock().unwrap().push(a);
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await.unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    assert!(r.found.is_some());
    assert_eq!(r.warning, None);
}

#[tokio::test]
async fn exit_zero_not_found_warns() {
    let env = Env::new();
    let (_s, p, _) = served_plan("inno", Some("user"), Scope::User, &product_code_line(), "").await;
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await.unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    assert!(r.found.is_none());
    assert!(
        r.warning
            .as_deref()
            .is_some_and(|w| w.contains("não apareceu")),
        "{:?}",
        r.warning
    );
}

/// Task 196: a machine-scope installer asks for its own UAC: `runas` on the verified file (on
/// the absolute msiexec with `/i "<verified file>"` for an MSI), the allowlisted parameters
/// with the log filled in, "waiting for permission" before the installer's phase; the
/// session's helper is never asked anything.
#[tokio::test]
async fn machine_installer_uses_runas_launcher() {
    let env = Env::new();
    let (_s, p, _) = served_plan("msi", Some("machine"), Scope::Machine, "", "").await;
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await.unwrap();
    let calls = env.launcher.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let spec = &calls[0];
    assert_eq!(spec.verb, Verb::RunAs);
    assert_eq!(spec.program, ctx.msiexec);
    assert!(spec.params.starts_with("/i \""), "{}", spec.params);
    assert!(is_run_copy(&quoted(&spec.params), "msi"), "{}", spec.params);
    assert!(spec.params.contains("/norestart"), "{}", spec.params);
    assert_no_log(&spec.params);
    assert_eq!(spec.show, Show::Normal);
    assert_eq!(r.log, None);
    assert!(env.elevated.calls.lock().unwrap().is_empty());
    let events = env.events.lock().unwrap().clone();
    let at = |p: Phase| {
        events
            .iter()
            .position(|e| *e == ProgressEvent::Phase(p.clone()))
            .unwrap_or_else(|| panic!("{p:?} missing: {events:?}"))
    };
    let running = Phase::RunningInstaller {
        name: "Contoso App".into(),
    };
    assert!(at(Phase::WaitingElevation) < at(running), "{events:?}");

    // An exe installer: `runas` on the verified file itself.
    let env = Env::new();
    let (_s, p, _) = served_plan("inno", Some("machine"), Scope::Machine, "", "").await;
    let ctx = ctx!(env);
    install(&ctx, &p).await.unwrap();
    let calls = env.launcher.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].verb, Verb::RunAs);
    assert!(
        is_run_copy(&calls[0].program, "exe"),
        "{}",
        calls[0].program.display()
    );
    assert_no_log(&calls[0].params);
    assert!(env.elevated.calls.lock().unwrap().is_empty());

    // A user-scope installer is not elevated.
    let env = Env::new();
    let (_s, p, _) = served_plan("inno", Some("user"), Scope::User, "", "").await;
    let ctx = ctx!(env);
    install(&ctx, &p).await.unwrap();
    let calls = env.launcher.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].verb, Verb::Open);
    assert!(
        !env.events
            .lock()
            .unwrap()
            .contains(&ProgressEvent::Phase(Phase::WaitingElevation))
    );
}

/// The first double-quoted run of `params` (msiexec's `/i "<file>"`).
fn quoted(params: &str) -> PathBuf {
    PathBuf::from(params.split('"').nth(1).unwrap_or(""))
}

/// Whether `p` is the copy of `Pub.App-1.0.<ext>` in a `installers\run-<32 hex>` folder.
fn is_run_copy(p: &Path, ext: &str) -> bool {
    let named = p.file_name().and_then(|n| n.to_str()) == Some(&format!("Pub.App-1.0.{ext}"));
    let run = p.parent();
    let in_run = run
        .and_then(|r| r.file_name())
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("run-"))
        .is_some_and(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_hexdigit()));
    let under = run
        .and_then(|r| r.parent())
        .and_then(|i| i.file_name())
        .is_some_and(|n| n == "installers");
    named && in_run && under
}

/// No log switch of any form (msiexec `/log`, `/l*v`, Inno `/LOG=`, the placeholder).
fn assert_no_log(params: &str) {
    let low = params.to_ascii_lowercase();
    for bad in ["/log", "/l*", "-log", "<logpath>", r"log\installers"] {
        assert!(!low.contains(bad), "{bad} in {params}");
    }
}

/// Task 196: an installer launched through `runas` gets no log argument at all (an elevated
/// write to a path the user controls is a junction away from anywhere): msi, inno and burn,
/// whose defaults all carry one; the log folder is not even created.
#[tokio::test]
async fn runas_command_line_has_no_log() {
    for (kind, ext) in [("msi", "msi"), ("inno", "exe"), ("burn", "exe")] {
        let env = Env::new();
        let (_s, p, _) = served_plan(kind, Some("machine"), Scope::Machine, "", "").await;
        assert!(
            !p.command.as_ref().unwrap().args.contains("<LOGPATH>"),
            "{kind}: {:?}",
            p.command
        );
        let ctx = ctx!(env);
        let r = install(&ctx, &p).await.unwrap();
        let calls = env.launcher.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "{kind}");
        assert_eq!(calls[0].verb, Verb::RunAs, "{kind}");
        assert!(
            is_run_copy(&calls[0].program, ext) || is_run_copy(&quoted(&calls[0].params), ext),
            "{kind}"
        );
        assert_no_log(&calls[0].params);
        assert_eq!(r.log, None, "{kind}");
        assert!(
            !env.root().join("log").join("installers").exists(),
            "{kind}"
        );
    }
    // Control: the same installers unelevated keep their log.
    for kind in ["msi", "inno", "burn"] {
        let env = Env::new();
        let (_s, p, _) = served_plan(kind, Some("user"), Scope::User, "", "").await;
        let ctx = ctx!(env);
        install(&ctx, &p).await.unwrap();
        let calls = env.launcher.calls.lock().unwrap().clone();
        assert_eq!(calls[0].verb, Verb::Open, "{kind}");
        assert!(
            calls[0].params.to_ascii_lowercase().contains("/log"),
            "{kind}: {}",
            calls[0].params
        );
    }
}

/// Task 196: the verified file cannot be renamed nor deleted while the elevated installer
/// runs (the launcher has not returned), and is released (then removed) once it ended.
#[tokio::test]
async fn verified_handle_held_until_exit() {
    let env = Env::new();
    env.launcher.probe_hold.store(true, Ordering::SeqCst);
    let (_s, p, _) = served_plan("inno", Some("machine"), Scope::Machine, "", "").await;
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await;
    assert!(r.is_ok(), "{r:?}");
    let calls = env.launcher.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].verb, Verb::RunAs);
    assert_eq!(
        *env.launcher.probed.lock().unwrap(),
        [("rename", false), ("delete", false)],
        "the verified file changed while the installer ran"
    );
    let file = env.materialized("exe");
    assert!(!file.exists(), "released and removed after the run");
    assert!(!file.with_extension("moved").exists());
}
#[tokio::test]
async fn user_inno_runs_unelevated_with_filled_log() {
    let env = Env::new();
    let (_s, p, _) = served_plan("inno", Some("user"), Scope::User, "", "").await;
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await.unwrap();
    let calls = env.launcher.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let spec = &calls[0];
    assert_eq!(spec.program, env.materialized("exe"));
    assert!(spec.params.contains("/LOG=\""), "{}", spec.params);
    let log_dir = env.root().join("log").join("installers");
    assert!(
        spec.params.contains(&format!("{}\\", log_dir.display())),
        "{}",
        spec.params
    );
    assert!(!spec.params.contains("<LOGPATH>"), "{}", spec.params);
    assert_eq!(spec.show, Show::Normal);
    assert_eq!(env.elevated.calls.lock().unwrap().len(), 0);
    assert!(r.log.is_some_and(|l| l.starts_with(&log_dir)));
    assert!(
        env.events
            .lock()
            .unwrap()
            .contains(&ProgressEvent::Phase(Phase::RunningInstaller {
                name: "Contoso App".into()
            }))
    );
}

#[tokio::test]
async fn unelevated_msi_uses_absolute_msiexec() {
    let env = Env::new();
    let (_s, p, _) = served_plan("msi", Some("user"), Scope::User, "", "").await;
    let ctx = ctx!(env);
    install(&ctx, &p).await.unwrap();
    let calls = env.launcher.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, ctx.msiexec);
    assert!(calls[0].params.starts_with("/i \""), "{}", calls[0].params);
    let file = env.materialized("msi");
    assert!(
        calls[0].params.contains(&file.display().to_string()),
        "{}",
        calls[0].params
    );
    assert_eq!(env.elevated.calls.lock().unwrap().len(), 0);
}

/// Task 196: the user says no to the installer's own UAC prompt (`ERROR_CANCELLED`): the
/// install is cancelled with the UAC message, nothing else is tried.
#[tokio::test]
async fn runas_refused_is_cancelled() {
    let env = Env::new();
    *env.launcher.os_error.lock().unwrap() = Some(1223);
    let (_s, p, _) = served_plan("msi", Some("machine"), Scope::Machine, "", "").await;
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(matches!(e, WingetOpsError::UacRefused), "{e:?}");
    assert_eq!(e.to_string(), UAC_REFUSED);
    let calls = env.launcher.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1, "no second prompt");
    assert_eq!(calls[0].verb, Verb::RunAs);
    assert!(env.elevated.calls.lock().unwrap().is_empty());
    assert!(!env.root().join("state.db").exists());
}
#[tokio::test]
async fn msi_1618_retries_then_succeeds() {
    let env = Env::new();
    env.launcher.codes.lock().unwrap().extend([1618, 0]);
    let (_s, p, _) = served_plan("msi", Some("user"), Scope::User, "", "").await;
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await;
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(*env.sleeps.lock().unwrap(), vec![Duration::from_secs(5)]);
    assert_eq!(env.launcher.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn msi_retry_then_other_failure_reports_the_second() {
    // The 2nd attempt fails for another reason than the 1st (per-attempt state reset).
    let env = Env::new();
    env.launcher.codes.lock().unwrap().extend([1618, 1603]);
    let (_s, p, _) = served_plan("msi", Some("user"), Scope::User, "", "").await;
    let ctx = ctx!(env);
    match install(&ctx, &p).await {
        Err(WingetOpsError::Failed { code, .. }) => assert_eq!(code, 1603),
        other => panic!("{other:?}"),
    }
    assert_eq!(env.launcher.calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn self_elevating_uac_refused_is_cancelled() {
    // An unelevated exe that asks for UAC by itself: the user says no (ERROR_CANCELLED).
    let env = Env::new();
    *env.launcher.os_error.lock().unwrap() = Some(1223);
    let (_s, p, _) = served_plan("inno", Some("user"), Scope::User, "", "").await;
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(matches!(e, WingetOpsError::UacRefused), "{e:?}");
    assert_eq!(e.to_string(), UAC_REFUSED);

    // Any other launch failure stays a launch failure.
    let env = Env::new();
    *env.launcher.os_error.lock().unwrap() = Some(2);
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(matches!(e, WingetOpsError::Launch(_)), "{e:?}");
}

#[tokio::test]
async fn failed_exit_code_is_an_error() {
    let env = Env::new();
    env.launcher.codes.lock().unwrap().push_back(2);
    let (_s, p, _) = served_plan("inno", Some("user"), Scope::User, "", "").await;
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(e.to_string().contains("cancelada pelo usuário"), "{e}");
    assert!(!env.root().join("state.db").exists());
}

#[test]
fn plan_text_is_the_confirmation() {
    let sha = "a".repeat(64);
    let eula = "Agreements:\n- AgreementLabel: EULA\n  AgreementUrl: https://x/eula\n";
    let msi = manifest_of(
        "msi",
        Some("machine"),
        "https://h/setup.msi",
        &sha,
        "",
        eula,
    );
    let text = native(&msi, Scope::Machine).to_string();
    // Task 202: the same line for msi and exe (the session's helper runs both).
    assert!(text.contains(ADMIN_PROMPT), "{text}");
    assert!(text.contains(RUN_FOLDER_NOTE), "{text}");
    let inno_machine = manifest_of("inno", Some("machine"), "https://h/setup.exe", &sha, "", "");
    let machine_text = native(&inno_machine, Scope::Machine).to_string();
    assert!(machine_text.contains(ADMIN_PROMPT), "{machine_text}");
    assert!(text.contains("Licença: EULA — https://x/eula"), "{text}");
    assert!(text.contains("Contoso App 1.0 (msi, máquina)"), "{text}");
    // URL, command line and hash only in `--dry-run`.
    assert!(!text.contains("https://h/setup.msi"), "{text}");
    let details = native(&msi, Scope::Machine).dry_run_details();
    assert!(
        details.contains(&"Instalador: https://h/setup.msi".to_string()),
        "{details:?}"
    );
    assert!(
        details.contains(&format!("SHA-256: {}", sha.to_uppercase())),
        "{details:?}"
    );
    assert!(!text.contains("<LOGPATH>"), "elevated: no log: {text}");

    let inno = manifest_of("inno", Some("user"), "https://h/setup.exe", &sha, "", "");
    let text = native(&inno, Scope::User).to_string();
    assert!(!text.contains(ADMIN_PROMPT), "{text}");
    assert!(!text.contains(RUN_FOLDER_NOTE), "{text}");
    assert!(!text.contains("Abre a janela"), "{text}");

    let exe = manifest_of("exe", None, "https://h/setup.exe", &sha, "", "");
    let text = native(&exe, Scope::User).to_string();
    assert!(text.contains("Abre a janela do instalador"), "{text}");
    assert!(text.contains("escopo do instalador"), "{text}");
}

#[test]
fn find_installed_order() {
    let sha = "a".repeat(64);
    let entries = format!(
        "  AppsAndFeaturesEntries:\n  - DisplayName: Contoso App\n    UpgradeCode: '{UPGRADE_CODE}'\n"
    );
    let m = manifest_of("msi", Some("user"), "https://h/x.msi", &sha, &entries, "");
    let p = native(&m, Scope::User);
    let by_name = app("by-name", "Contoso App", "Contoso");
    let mut by_upgrade = app("by-upgrade", "Something", "Else");
    // No braces, lowercase: still the same code.
    by_upgrade.upgrade_code = Some(UPGRADE_CODE.trim_matches(['{', '}']).to_ascii_lowercase());
    let apps = vec![by_name.clone(), by_upgrade];
    assert_eq!(find_installed(&p, &apps).unwrap().key, "by-upgrade");
    // Without a code match the name and publisher still find it.
    assert_eq!(find_installed(&p, &apps[..1]).unwrap().key, "by-name");

    let m = manifest_of(
        "msi",
        Some("user"),
        "https://h/x.msi",
        &sha,
        &product_code_line(),
        "",
    );
    let p = native(&m, Scope::User);
    let mut by_code = app("by-code", "Something", "Else");
    by_code.product_code = Some(PRODUCT_CODE.into());
    let apps = vec![by_name, by_code];
    assert_eq!(find_installed(&p, &apps).unwrap().key, "by-code");
}

#[tokio::test]
async fn msix_goes_to_package_manager() {
    let env = Env::new();
    let (_s, p, _) = served_plan(
        "msix",
        None,
        Scope::User,
        "  PackageFamilyName: Pub.App_8wekyb3d8bbwe\n",
        "",
    )
    .await;
    let mut other = app("by-name", "Contoso App", "Contoso");
    other.source = InstallSource::Arp(Hive::User);
    let mut pkg = app("Pub.App_8wekyb3d8bbwe", "x", "y");
    pkg.source = InstallSource::Msix;
    pkg.family_name = Some("pub.app_8wekyb3d8bbwe".into());
    env.apps.lock().unwrap().extend([other, pkg]);
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await.unwrap();
    assert_eq!(
        *env.msix.adds.lock().unwrap(),
        vec![env.materialized("msix")]
    );
    assert_eq!(env.launcher.calls.lock().unwrap().len(), 0);
    assert_eq!(r.found.unwrap().key, "Pub.App_8wekyb3d8bbwe");
}

#[tokio::test]
async fn cancel_before_launch() {
    let env = Env::new();
    let (_s, p, _) = served_plan("inno", Some("user"), Scope::User, "", "").await;
    assert!(env.gate.try_cancel());
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await;
    assert!(matches!(r, Err(WingetOpsError::Cancelled)), "{r:?}");
    assert_eq!(env.calls(), (0, 0, 0));
}

#[tokio::test]
async fn cancel_while_running_is_refused() {
    let env = Env::new();
    *env.launcher.cancel_during_run.lock().unwrap() = Some((env.gate.clone(), None));
    let (_s, p, _) = served_plan("inno", Some("user"), Scope::User, "", "").await;
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await;
    assert!(r.is_ok(), "{r:?}");
    let answer = env
        .launcher
        .cancel_during_run
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .1;
    assert_eq!(answer, Some(false));
    assert_eq!(env.calls(), (1, 0, 0));
}

#[tokio::test]
async fn winget_ops_row_written() {
    let env = Env::new();
    let (_s, p, sha) =
        served_plan("inno", Some("user"), Scope::User, &product_code_line(), "").await;
    let mut a = app("ContosoApp_is1", "Other", "Other");
    a.product_code = Some(PRODUCT_CODE.into());
    env.apps.lock().unwrap().push(a);
    let ctx = ctx!(env);
    install(&ctx, &p).await.unwrap();
    let conn = rusqlite::Connection::open(env.root().join("state.db")).unwrap();
    let rows: Vec<(String, String, String, String, Option<String>)> = conn
        .prepare("SELECT id, version, installer_type, sha256, arp_key FROM winget_ops")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![(
            "Pub.App".to_string(),
            "1.0".to_string(),
            "inno".to_string(),
            sha,
            Some("ContosoApp_is1".to_string())
        )]
    );
}

#[test]
fn consumers_show_new_phases() {
    use wingetrs::cli::install_cmd::LineProgress;
    use wingetrs::cli::output::{PhaseLog, Phases};
    use wingetrs::tui::app::{App, Msg, Running, update};
    use wingetrs::tui::jobs::{ProgressThrottle, progress_msg};

    let running = Phase::RunningInstaller { name: "X".into() };
    assert!(phase_status(&running, Duration::ZERO).contains("não pode ser cancelado"));
    assert!(!phase_status(&Phase::Verifying, Duration::ZERO).contains("não pode"));
    let preparing = Phase::HelperPreparing { name: "X".into() };
    let phases = [
        Phase::WaitingElevation,
        preparing.clone(),
        running.clone(),
        Phase::Verifying,
    ];

    let glyphs = Glyphs::for_set(GlyphSet::Ascii);
    let throttle = ProgressThrottle::new(Instant::now());
    let line = LineProgress::new(false, glyphs, Default::default());
    let mut plain = PhaseLog::new();
    for p in &phases {
        let ev = ProgressEvent::Phase(p.clone());
        // TUI: every new phase reaches the status bar.
        assert!(
            matches!(progress_msg(&throttle, Instant::now(), &ev), Some(Msg::InstallPhase(q)) if q == *p)
        );
        // CLI off a terminal: `LineProgress::update` writes `PhaseLog::line` to stderr (not
        // capturable in-process), so the text is checked on `PhaseLog` and `update` only
        // has to run.
        line.update(&ev);
        let text = plain.line(&ev).unwrap();
        assert!(text.contains(&phase_title(p)), "{text}");
        // CLI on a terminal.
        let mut tty = Phases::new();
        tty.update_at(&ev, Instant::now());
        let shown = tty.line_at(0, glyphs, Instant::now()).unwrap();
        assert!(shown.contains(&phase_title(p)), "{shown}");
        assert_eq!(
            shown.contains("não pode ser cancelado"),
            *p != Phase::Verifying,
            "{shown}"
        );
    }
    line.finish();
    assert!(
        phase_title(&Phase::WaitingElevation)
            .starts_with(wingetrs::install::progress::ELEVATION_NOTICE)
    );
    assert!(
        phase_status(&Phase::WaitingElevation, Duration::ZERO).contains("não pode ser cancelado")
    );
    assert_eq!(
        phase_title(&running),
        "Instalador de X rodando (pode abrir janelas)"
    );
    assert_eq!(phase_title(&Phase::Verifying), "Conferindo a instalação");
    assert_eq!(
        phase_title(&preparing),
        "Baixando e conferindo o instalador de X como administrador"
    );

    // TUI status bar: the running installer cannot be cancelled.
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.running = Some(Running {
        id: 1,
        label: "Instalar X".into(),
        progress: None,
        action: None,
    });
    update(&mut a, Msg::InstallPhase(Phase::Verifying));
    assert!(!a.installer_running);
    // Task 202: the helper's catalog and download checks are already past the gate.
    update(&mut a, Msg::InstallPhase(preparing));
    assert!(a.installer_running);
    a.installer_running = false;
    // Sent after the launch gate: the elevated installer may already run.
    update(&mut a, Msg::InstallPhase(Phase::WaitingElevation));
    assert!(a.installer_running);
    update(&mut a, Msg::InstallPhase(running));
    assert!(a.installer_running);
    let text = wingetrs::tui::view::running_text(&a, glyphs).unwrap();
    assert!(text.contains("Instalador de X rodando"), "{text}");
    assert!(text.contains("não pode ser cancelado"), "{text}");
}

#[tokio::test]
async fn hold_msi_lock_while_running() {
    let env = Env::new();
    let (_s, p, _) = served_plan("inno", Some("user"), Scope::User, "", "").await;
    let count = Arc::clone(&env.launcher.count);
    let releaser = hold_msi_lock(&env, move || count.load(Ordering::SeqCst));
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await;
    *env.on_phase.lock().unwrap() = None;
    let before_release = releaser.join().unwrap();
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(before_release, 0, "launched while msi.lock was held");
    assert_eq!(env.launcher.count.load(Ordering::SeqCst), 1);
    let _ = &env.tmp;
}

#[tokio::test]
async fn msi_failure_carries_the_log_hint() {
    // Unelevated: the log is there to read (an elevated run has none, Task 196).
    let env = Env::new();
    env.launcher.codes.lock().unwrap().push_back(1603);
    *env.launcher.log_line.lock().unwrap() = Some("Error 1304. Error writing to file".into());
    let (_s, p, _) = served_plan("msi", Some("user"), Scope::User, "", "").await;
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(
        matches!(e, WingetOpsError::Failed { code: 1603, .. }),
        "{e:?}"
    );
    assert!(
        e.to_string()
            .contains(wingetrs::install::msiexec_log::MAX_PATH_HINT),
        "{e}"
    );
    assert_eq!(env.launcher.calls.lock().unwrap()[0].verb, Verb::Open);

    // Elevated: the same exit code is classified from the code alone.
    let env = Env::new();
    env.launcher.codes.lock().unwrap().push_back(1603);
    let (_s, p, _) = served_plan("msi", Some("machine"), Scope::Machine, "", "").await;
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(
        matches!(e, WingetOpsError::Failed { code: 1603, .. }),
        "{e:?}"
    );
    assert_eq!(env.launcher.calls.lock().unwrap()[0].verb, Verb::RunAs);
}
#[tokio::test]
async fn msix_failure_is_an_error() {
    let env = Env::new();
    *env.msix.fail.lock().unwrap() = Some(MsixError {
        hresult: (0x8007_0000u32 | 15622) as i32,
        text: "downgrade".into(),
    });
    let (_s, p, _) = served_plan("msix", None, Scope::User, "", "").await;
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(e.to_string().contains("versão mais nova"), "{e}");
}

#[test]
fn helper_params_outside_allowlist_go_to_bridge() {
    let sha = "a".repeat(64);
    let custom = |c: &str| {
        format!(
            "  InstallerSwitches:
    Custom: '{c}'
"
        )
    };
    let refused = manifest_of(
        "inno",
        Some("machine"),
        "https://h/s.exe",
        &sha,
        &custom("/INVENTED"),
        "",
    );
    match plan(&refused, &host(), ScopeWanted::Prefer(Scope::Machine), None).unwrap() {
        PlanOrBridge::Bridge(why) => assert_eq!(
            why,
            "parâmetros do instalador fora da lista permitida para elevação: /INVENTED"
        ),
        PlanOrBridge::Native(p) => panic!("native: {p}"),
        PlanOrBridge::Portable(s) => panic!("portable: {s:?}"),
    }
    let accepted = manifest_of(
        "inno",
        Some("machine"),
        "https://h/s.exe",
        &sha,
        &custom("/NOCANCEL"),
        "",
    );
    let p = native(&accepted, Scope::Machine);
    assert!(p.command.unwrap().args.contains("/NOCANCEL"));
    // The same custom switch does not matter unelevated.
    let user = manifest_of(
        "inno",
        Some("user"),
        "https://h/s.exe",
        &sha,
        &custom("/INVENTED"),
        "",
    );
    native(&user, Scope::User);
}

#[tokio::test]
async fn exit_3010_is_ok_reboot() {
    let env = Env::new();
    env.launcher.codes.lock().unwrap().push_back(3010);
    let (_s, p, _) = served_plan("msi", Some("user"), Scope::User, "", "").await;
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await.unwrap();
    assert_eq!(r.outcome, Outcome::OkReboot { initiated: false });
    assert_eq!(env.launcher.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn msix_cancel_before_launch() {
    let env = Env::new();
    let (_s, p, _) = served_plan("msix", None, Scope::User, "", "").await;
    assert!(env.gate.try_cancel());
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await;
    assert!(matches!(r, Err(WingetOpsError::Cancelled)), "{r:?}");
    assert_eq!(env.calls(), (0, 0, 0));
}

/// Task 196: right before the UAC prompt the flow checks the elevation allowlist again; a plan
/// whose parameters left it (built or changed elsewhere) launches nothing.
#[tokio::test]
async fn runas_refuses_params_outside_the_allowlist() {
    let env = Env::new();
    let (_s, mut p, _) = served_plan("msi", Some("machine"), Scope::Machine, "", "").await;
    p.command
        .as_mut()
        .unwrap()
        .args
        .push_str(r" TRANSFORMS=C:\x.mst");
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(matches!(e, WingetOpsError::Elevated(_)), "{e:?}");
    assert!(e.to_string().contains("elevação"), "{e}");
    assert_eq!(env.launcher.calls.lock().unwrap().len(), 0);

    // A log placeholder (the allowlist alone would take `/log "<LOGPATH>"`): no elevated run
    // writes a log.
    let env = Env::new();
    let (_s, mut p, _) = served_plan("msi", Some("machine"), Scope::Machine, "", "").await;
    p.command
        .as_mut()
        .unwrap()
        .args
        .push_str(r#" /log "<LOGPATH>""#);
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(matches!(e, WingetOpsError::Elevated(_)), "{e:?}");
    assert_eq!(env.launcher.calls.lock().unwrap().len(), 0);
}

/// `mklink /J link target`.
fn junction(link: &Path, target: &Path) {
    let ok = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "mklink /J");
}

/// Task 196 review: a junction anywhere from the data root down to the installer (it could be
/// re-pointed after the hash check) is refused before anything launches.
#[tokio::test]
async fn junction_in_the_installer_chain_is_refused() {
    for depth in [&["installers"][..], &["installers", "Pub.App"][..]] {
        let env = Env::new();
        let elsewhere = env.tmp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let mut parent = env.root().to_path_buf();
        for d in &depth[..depth.len() - 1] {
            parent.push(d);
        }
        std::fs::create_dir_all(&parent).unwrap();
        junction(&parent.join(depth[depth.len() - 1]), &elsewhere);
        let (_s, p, _) = served_plan("inno", Some("machine"), Scope::Machine, "", "").await;
        let ctx = ctx!(env);
        let r = install(&ctx, &p).await;
        assert!(
            matches!(&r, Err(WingetOpsError::Fetch(e)) if e.to_string().contains("reanálise")),
            "{depth:?}: {r:?}"
        );
        assert_eq!(env.launcher.calls.lock().unwrap().len(), 0, "{depth:?}");
    }
}

/// Task 196 review: the elevated launch gets the held file's final path (here the data root
/// is written with a `..`, which the final path no longer has) and System32 as its folder.
#[tokio::test]
async fn runas_gets_the_final_path_and_system32() {
    let mut env = Env::new();
    env.data = DataDir::new(env.tmp.path().join("x").join("..").join("root"));
    let (_s, p, _) = served_plan("inno", Some("machine"), Scope::Machine, "", "").await;
    let ctx = ctx!(env);
    install(&ctx, &p).await.unwrap();
    let calls = env.launcher.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].verb, Verb::RunAs);
    let tmp = std::fs::canonicalize(env.tmp.path()).unwrap();
    let tmp = tmp.to_str().unwrap();
    let tmp = tmp.strip_prefix(r"\\?\").unwrap_or(tmp).to_string();
    let installers = Path::new(&tmp).join("root").join("installers");
    let program = &calls[0].program;
    assert_eq!(program.file_name().unwrap(), "Pub.App-1.0.exe");
    let run = program.parent().unwrap();
    assert_eq!(run.parent().unwrap(), installers, "{}", program.display());
    assert_ne!(*program, env.materialized("exe"));
    assert_eq!(calls[0].dir.as_deref(), env.msiexec.parent());
}

/// Task 196 (user decision on DLL planting): the elevated installer runs from a fresh
/// `installers\run-<32 hex>\` folder that holds only its copy, gone once it ended; the
/// downloaded file's own folder is not where it runs.
#[tokio::test]
async fn runas_runs_from_a_fresh_run_folder() {
    for kind in ["inno", "msi"] {
        let env = Env::new();
        let (_s, p, _) = served_plan(kind, Some("machine"), Scope::Machine, "", "").await;
        let ctx = ctx!(env);
        install(&ctx, &p).await.unwrap();
        let calls = env.launcher.calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 1, "{kind}");
        let ext = if kind == "msi" { "msi" } else { "exe" };
        let copy = match kind {
            // msiexec /i "<copy>" ...
            "msi" => PathBuf::from(calls[0].params.split('"').nth(1).unwrap()),
            _ => calls[0].program.clone(),
        };
        assert_eq!(
            copy.file_name().unwrap().to_str().unwrap(),
            format!("Pub.App-1.0.{ext}")
        );
        let run = copy.parent().unwrap();
        let name = run.file_name().unwrap().to_str().unwrap();
        let hex = name
            .strip_prefix("run-")
            .unwrap_or_else(|| panic!("{name}"));
        assert!(
            hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
            "{name}"
        );
        assert_eq!(
            run.parent().unwrap().file_name().unwrap(),
            "installers",
            "{}",
            run.display()
        );
        assert!(!run.exists(), "{kind}: run folder removed after the run");
        if kind == "inno" {
            assert_eq!(
                *env.launcher.seen_in_dir.lock().unwrap(),
                [vec![format!("Pub.App-1.0.{ext}")]],
                "only the installer beside it"
            );
        }
    }
}

/// Task 196: a run folder name that already exists (planted, or left behind) is refused and
/// left alone; a fresh one holds the re-hashed copy and goes when dropped.
#[test]
fn run_folder_that_exists_is_refused() {
    use wingetrs::winget_install::verified_win::RunCopy;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let installers = root.join("installers");
    std::fs::create_dir_all(&installers).unwrap();
    let source = installers.join("setup.exe");
    std::fs::write(&source, b"MZ t196").unwrap();
    let sha = sha256_hex(b"MZ t196");
    let name = format!("run-{}", "a".repeat(32));
    let planted = installers.join(&name);
    std::fs::create_dir(&planted).unwrap();
    std::fs::write(planted.join("version.dll"), b"x").unwrap();
    let r = RunCopy::create_named(&root, &name, &source, &sha);
    assert!(r.is_err(), "{r:?}");
    assert!(planted.join("version.dll").is_file(), "left alone");

    let fresh = format!("run-{}", "b".repeat(32));
    let run = RunCopy::create_named(&root, &fresh, &source, &sha).unwrap();
    let dir = run.dir().to_path_buf();
    assert_eq!(dir, installers.join(&fresh));
    let names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["setup.exe"]);
    assert!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("setup.exe"))
            .is_err(),
        "the copy is held denying writes"
    );
    drop(run);
    assert!(!dir.exists());

    // A wrong hash: refused, and the folder does not stay.
    let other = format!("run-{}", "c".repeat(32));
    let r = RunCopy::create_named(&root, &other, &source, &"0".repeat(64));
    assert!(r.is_err());
    assert!(!installers.join(&other).exists());
}

fn native_in(
    m: &MergedManifest,
    wanted: Scope,
    mode: wingetrs::config::InstallerMode,
) -> WingetPlan {
    match plan_with_mode(m, &host(), ScopeWanted::Prefer(wanted), None, mode).unwrap() {
        PlanOrBridge::Native(p) => p,
        PlanOrBridge::Bridge(why) => panic!("bridge: {why}"),
        PlanOrBridge::Portable(s) => panic!("portable: {s:?}"),
    }
}

/// Task 203 item 2: the mode step comes from the switch that runs. NSIS takes `/S` in both
/// modes: "Sem janela" whatever the setting; a manifest whose two switches are the same too.
#[test]
fn mode_step_comes_from_the_switch_that_runs() {
    use wingetrs::config::InstallerMode::{Progress, Silent};
    let sha = "a".repeat(64);
    let url = "https://h/setup.exe";
    let nsis = manifest_of("nullsoft", Some("user"), url, &sha, "", "");
    for mode in [Progress, Silent] {
        let p = native_in(&nsis, Scope::User, mode);
        assert!(p.steps().contains(&"Sem janela".to_string()), "{p}");
        assert!(!p.to_string().contains("Mostra o progresso"), "{p}");
        assert_eq!(p.command.as_ref().unwrap().args, "/S");
    }
    let same = "  InstallerSwitches:\n    Silent: /quiet /x\n    SilentWithProgress: /QUIET  /x\n";
    let exe = manifest_of("exe", Some("user"), url, &sha, same, "");
    let p = native_in(&exe, Scope::User, Progress);
    assert!(p.steps().contains(&"Sem janela".to_string()), "{p}");
    // Inno and an exe with two different switches keep the progress window.
    let inno = manifest_of("inno", Some("user"), url, &sha, "", "");
    let p = native_in(&inno, Scope::User, Progress);
    assert!(
        p.steps()
            .contains(&"Mostra o progresso do instalador".to_string()),
        "{p}"
    );
    assert!(
        p.command.as_ref().unwrap().args.contains("/SILENT"),
        "{p:?}"
    );
    let two = "  InstallerSwitches:\n    Silent: /q\n    SilentWithProgress: /p\n";
    let exe = manifest_of("exe", Some("user"), url, &sha, two, "");
    let p = native_in(&exe, Scope::User, Progress);
    assert!(
        p.steps()
            .contains(&"Mostra o progresso do instalador".to_string()),
        "{p}"
    );
    assert_eq!(p.command.as_ref().unwrap().args, "/p");
    let p = native_in(&exe, Scope::User, Silent);
    assert!(p.steps().contains(&"Sem janela".to_string()), "{p}");
    // Only a progress switch: progress (nothing quieter to run).
    let only = "  InstallerSwitches:\n    SilentWithProgress: /p\n";
    let exe = manifest_of("exe", Some("user"), url, &sha, only, "");
    let p = native_in(&exe, Scope::User, Progress);
    assert!(
        p.steps()
            .contains(&"Mostra o progresso do instalador".to_string()),
        "{p}"
    );
}

/// Task 203 item 3: an MSIX plan has its own steps (the package manager, no window, the
/// user's scope), numbered in the CLI.
#[test]
fn msix_plan_has_steps() {
    use wingetrs::cli::{output::winget_plan_text, style::with_enabled};
    let sha = "a".repeat(64);
    let m = manifest_of("msix", None, "https://h/a.msixbundle", &sha, "", "");
    let p = native(&m, Scope::User);
    assert_eq!(
        p.steps(),
        vec![
            "Instala pelo Windows (pacote MSIX), sem janela".to_string(),
            "Só para este usuário".to_string()
        ]
    );
    assert_eq!(
        p.steps(),
        vec![MSIX_STEP.to_string(), MSIX_SCOPE_STEP.to_string()]
    );
    let cli = with_enabled(false, || winget_plan_text(&p, true));
    assert!(
        cli.contains(
            "  1. Instala pelo Windows (pacote MSIX), sem janela\n  2. Só para este usuário\n"
        ),
        "{cli}"
    );
    // An installer with a command line never gets the MSIX steps.
    let exe = manifest_of("inno", Some("user"), "https://h/setup.exe", &sha, "", "");
    let steps = native(&exe, Scope::User).steps();
    assert!(!steps.contains(&MSIX_STEP.to_string()), "{steps:?}");
    assert!(!steps.contains(&MSIX_SCOPE_STEP.to_string()), "{steps:?}");
}

/// Task 203 item 7: an installer that asks for UAC by itself (`elevatesSelf`, or
/// `elevationRequired` with a user scope, which wingetrs does not elevate) warns in the plan.
#[test]
fn self_elevating_installer_warns() {
    use wingetrs::cli::{output::winget_plan_text, style::with_enabled};
    let sha = "a".repeat(64);
    let url = "https://h/setup.exe";
    for req in ["elevatesSelf", "elevationRequired", "ELEVATESSELF"] {
        let extra = format!("  ElevationRequirement: {req}\n");
        let m = manifest_of("inno", Some("user"), url, &sha, &extra, "");
        let p = native(&m, Scope::User);
        assert_eq!(p.steps()[0], SELF_ADMIN_PROMPT, "{req}");
        assert_eq!(
            SELF_ADMIN_PROMPT,
            "O instalador pode pedir permissão de administrador"
        );
        let cli = with_enabled(false, || winget_plan_text(&p, false));
        assert!(
            cli.contains("  1. ⚠ O instalador pode pedir permissão de administrador\n"),
            "{cli}"
        );
    }
    for extra in ["", "  ElevationRequirement: elevationProhibited\n"] {
        let m = manifest_of("inno", Some("user"), url, &sha, extra, "");
        let steps = native(&m, Scope::User).steps();
        assert!(!steps.contains(&SELF_ADMIN_PROMPT.to_string()), "{steps:?}");
    }
    // Elevated by wingetrs: its own UAC line, not this one.
    let extra = "  ElevationRequirement: elevationRequired\n";
    let m = manifest_of("inno", Some("machine"), url, &sha, extra, "");
    let steps = native(&m, Scope::Machine).steps();
    assert_eq!(steps[0], ADMIN_PROMPT, "{steps:?}");
    assert!(!steps.contains(&SELF_ADMIN_PROMPT.to_string()), "{steps:?}");
    // MSIX never: the package manager has no UAC of its own.
    let extra = "  ElevationRequirement: elevatesSelf\n";
    let m = manifest_of("msix", None, "https://h/a.msix", &sha, extra, "");
    let steps = native(&m, Scope::User).steps();
    assert!(!steps.contains(&SELF_ADMIN_PROMPT.to_string()), "{steps:?}");
}

/// Task 203 item 10: the SHA-256 of the dry run is always in capitals (GPXSee's manifest has
/// it in lowercase).
#[test]
fn dry_run_hash_is_upper_case() {
    let lower = "da5b3122a4375117fca2607b2a1fc32d165960d36ddd467842f96ced3676ea8c";
    let m = manifest_of(
        "nullsoft",
        Some("user"),
        "https://h/setup.exe",
        lower,
        "",
        "",
    );
    let details = native(&m, Scope::User).dry_run_details();
    assert!(
        details.contains(
            &"SHA-256: DA5B3122A4375117FCA2607B2A1FC32D165960D36DDD467842F96CED3676EA8C"
                .to_string()
        ),
        "{details:?}"
    );
}
// ---- Task 202: installers that run elevated go through the session's helper ----

/// A plan for an installer the server must never serve: the helper downloads it itself.
async fn unserved_plan(kind: &str, scope: Option<&str>, extra: &str) -> (MockServer, WingetPlan) {
    let body = bytes();
    let sha = sha256_hex(&body);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/setup"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .expect(0)
        .mount(&server)
        .await;
    let m = manifest_of(
        kind,
        scope,
        &format!("{}/setup", server.uri()),
        &sha,
        extra,
        "",
    );
    (server, native(&m, Scope::Machine))
}

fn helper_lines() -> Vec<String> {
    use wingetrs::bridge::signed_install::{
        CHECKING_CATALOG, DOWNLOADING_INSTALLER, INSTALLER_RUNNING,
    };
    use wingetrs::install::progress::{ELEVATION_GRANTED, ELEVATION_NOTICE};
    [
        ELEVATION_NOTICE,
        ELEVATION_GRANTED,
        CHECKING_CATALOG,
        DOWNLOADING_INSTALLER,
        INSTALLER_RUNNING,
    ]
    .map(String::from)
    .to_vec()
}

/// A machine-scope installer: one request to the helper with the package, the installer's
/// position and the mode (nothing about the file); nothing downloaded nor launched here; the
/// helper's lines move the phases (its UAC, its catalog and download checks, the installer);
/// the operation is recorded with the manifest's SHA-256.
#[tokio::test]
async fn machine_installer_goes_through_the_helper() {
    use wingetrs::bridge::worker::{ModeArg, ScopeArg};
    let env = Env::new();
    let (server, p) = unserved_plan("inno", Some("machine"), "").await;
    env.elevated.answer(Ok(0));
    *env.elevated.lines.lock().unwrap() = helper_lines();
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await.unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    assert_eq!(r.log, None);
    assert_eq!(
        env.elevated.requests(),
        [PackageRequest {
            id: "Pub.App".into(),
            version: "1.0".into(),
            installer: 0,
            scope: ScopeArg::Machine,
            mode: ModeArg::SilentWithProgress,
            upgrade: false,
        }]
    );
    assert_eq!(env.calls(), (0, 0, 0));
    assert!(
        !env.root().join("installers").exists(),
        "nothing downloaded here"
    );
    let events = env.events.lock().unwrap().clone();
    let at = |p: Phase| {
        events
            .iter()
            .position(|e| *e == ProgressEvent::Phase(p.clone()))
            .unwrap_or_else(|| panic!("{p:?} missing: {events:?}"))
    };
    let preparing = Phase::HelperPreparing {
        name: "Contoso App".into(),
    };
    let running = Phase::RunningInstaller {
        name: "Contoso App".into(),
    };
    assert!(
        at(Phase::WaitingElevation) < at(preparing.clone()),
        "{events:?}"
    );
    assert!(at(preparing) < at(running.clone()), "{events:?}");
    assert!(at(running) < at(Phase::Verifying), "{events:?}");
    let conn = rusqlite::Connection::open(env.root().join("state.db")).unwrap();
    let sha: String = conn
        .query_row("SELECT sha256 FROM winget_ops", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sha, sha256_hex(&bytes()));
    drop(server);

    // An MSI: the same request, its own mode.
    let env = Env::new();
    let (_server, p) = unserved_plan("msi", Some("machine"), "").await;
    env.elevated.answer(Ok(0));
    let ctx = ctx!(env);
    install(&ctx, &p).await.unwrap();
    let reqs = env.elevated.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].mode, ModeArg::SilentWithProgress);
    assert_eq!(env.calls(), (0, 0, 0));
}

/// What the helper answers decides the result: a refused UAC, a refusal or a lost helper are
/// errors with nothing launched here; an exit code is classified like any installer's.
#[tokio::test]
async fn helper_answers_are_the_result() {
    let cases: [(Result<i32, ElevatedError>, &str); 4] = [
        (Err(ElevatedError::Refused), UAC_REFUSED),
        (
            Err(ElevatedError::Remote("catálogo recusado".into())),
            "catálogo recusado",
        ),
        (Err(ElevatedError::Lost("canal".into())), "canal"),
        (Ok(1603), "1603"),
    ];
    for (answer, text) in cases {
        let env = Env::new();
        let (_server, p) = unserved_plan("msi", Some("machine"), "").await;
        env.elevated.answer(answer);
        let ctx = ctx!(env);
        let e = install(&ctx, &p).await.unwrap_err();
        assert!(e.to_string().contains(text), "{e}");
        assert_eq!(env.elevated.requests().len(), 1);
        assert_eq!(env.launcher.calls.lock().unwrap().len(), 0, "{e}");
        assert!(!env.root().join("state.db").exists());
    }
    let env = Env::new();
    let (_server, p) = unserved_plan("msi", Some("machine"), "").await;
    env.elevated.answer(Err(ElevatedError::Refused));
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(matches!(e, WingetOpsError::UacRefused), "{e:?}");
}

/// Without a helper this session (wingetrs outside an administrator-only folder), the
/// installer asks for its own UAC prompt (Task 196) after one request; the launch gate is
/// free again for the fallback's download.
#[tokio::test]
async fn no_helper_falls_back_to_runas() {
    let env = Env::new();
    let (_s, p, _) = served_plan("inno", Some("machine"), Scope::Machine, "", "").await;
    let gate = env.gate.clone();
    *env.on_phase.lock().unwrap() = Some(Box::new(move |p: &Phase| {
        // The fallback's download starts with `Preparing`, after the request came back.
        if *p == Phase::Preparing {
            assert!(gate.running().is_none(), "gate still taken: {p:?}");
        }
    }));
    let ctx = ctx!(env);
    install(&ctx, &p).await.unwrap();
    *env.on_phase.lock().unwrap() = None;
    assert_eq!(env.elevated.requests().len(), 1);
    let calls = env.launcher.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].verb, Verb::RunAs);
}

/// A cancel before the launch gate sends nothing to the helper.
#[tokio::test]
async fn cancel_before_the_helper_sends_nothing() {
    let env = Env::new();
    let (_s, p) = unserved_plan("inno", Some("machine"), "").await;
    env.elevated.answer(Ok(0));
    assert!(env.gate.try_cancel());
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await;
    assert!(matches!(r, Err(WingetOpsError::Cancelled)), "{r:?}");
    assert!(env.elevated.requests().is_empty());
}

/// Windows Installer types: a 1618 from the helper is retried with a new request.
#[tokio::test]
async fn helper_1618_is_retried() {
    let env = Env::new();
    let (_s, p) = unserved_plan("msi", Some("machine"), "").await;
    env.elevated.answer(Ok(1618));
    env.elevated.answer(Ok(0));
    let ctx = ctx!(env);
    let r = install(&ctx, &p).await.unwrap();
    assert_eq!(r.outcome, Outcome::Ok);
    assert_eq!(env.elevated.requests().len(), 2);
    assert!(!env.sleeps.lock().unwrap().is_empty());
}

/// A plan whose command line the helper would not build itself is refused before any request.
#[tokio::test]
async fn changed_command_line_never_reaches_the_helper() {
    let env = Env::new();
    let (_s, mut p) = unserved_plan("inno", Some("machine"), "").await;
    p.command.as_mut().unwrap().args.push_str(" /NOICONS");
    env.elevated.answer(Ok(0));
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert!(matches!(e, WingetOpsError::Elevated(_)), "{e:?}");
    assert!(env.elevated.requests().is_empty());
    assert_eq!(env.calls(), (0, 0, 0));
}
