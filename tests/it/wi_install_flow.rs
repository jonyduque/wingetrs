#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Native install flow (Task 181).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use wingetrs::bridge::elevated::{ElevatedError, ElevatedRunner};
use wingetrs::bridge::worker::{ElevatedKind, InstallerRequest};
use wingetrs::cache::{DataDir, UpdateLock, sha256_hex};
use wingetrs::http::Http;
use wingetrs::install::executor::LaunchGate;
use wingetrs::install::progress::{Phase, ProgressEvent, phase_status, phase_title};
use wingetrs::installed::{Hive, InstallSource, InstalledApp};
use wingetrs::sources::winget::manifest::{self, MergedManifest};
use wingetrs::tui::theme::{GlyphSet, Glyphs};
use wingetrs::update::LockWait;
use wingetrs::winget_install::codes::{Outcome, UAC_REFUSED};
use wingetrs::winget_install::launch::{LaunchSpec, Launcher, Show};
use wingetrs::winget_install::msix::{AddResult, MsixError, MsixOps};
use wingetrs::winget_install::ops::{WingetCtx, WingetOpsError, install};
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan, plan};
use wingetrs::winget_install::post_check::find_installed;
use wingetrs::winget_install::select::{Host, Scope, ScopeWanted};
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
}

impl Launcher for FakeLauncher {
    fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32> {
        self.calls.lock().unwrap().push(spec.clone());
        self.count.fetch_add(1, Ordering::SeqCst);
        if let Some((gate, answer)) = self.cancel_during_run.lock().unwrap().as_mut() {
            *answer = Some(gate.try_cancel());
        }
        Ok(self.codes.lock().unwrap().pop_front().unwrap_or(0))
    }
}

#[derive(Default)]
struct FakeElevated {
    calls: Mutex<Vec<InstallerRequest>>,
    lines: Mutex<Vec<String>>,
    results: Mutex<VecDeque<Result<i32, ElevatedError>>>,
}

impl ElevatedRunner for FakeElevated {
    fn run_installer(
        &self,
        req: &InstallerRequest,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        self.calls.lock().unwrap().push(req.clone());
        on_line("MSI (s) fake log line");
        for l in self.lines.lock().unwrap().iter() {
            on_line(l);
        }
        self.results.lock().unwrap().pop_front().unwrap_or(Ok(0))
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

#[tokio::test]
async fn machine_msi_goes_through_helper() {
    let env = Env::new();
    let (_s, p, sha) = served_plan("msi", Some("machine"), Scope::Machine, "", "").await;
    let ctx = ctx!(env);
    install(&ctx, &p).await.unwrap();
    let calls = env.elevated.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let req = &calls[0];
    assert_eq!(req.kind, ElevatedKind::Msi);
    assert!(req.file_name.ends_with(".msi"), "{}", req.file_name);
    assert!(req.params.contains("/quiet /norestart"), "{}", req.params);
    assert!(req.params.contains("<LOGPATH>"), "{}", req.params);
    assert_eq!(req.sha256, sha);
    // Task 180 contract: the source is the download cache's blob.
    let blob = env.root().join("blobs").join(&sha);
    assert_eq!(Path::new(&req.source), blob);
    assert_eq!(env.launcher.calls.lock().unwrap().len(), 0);
    assert!(
        env.events
            .lock()
            .unwrap()
            .contains(&ProgressEvent::Phase(Phase::WaitingElevation))
    );
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

#[tokio::test]
async fn refused_elevation_fails_with_message() {
    let env = Env::new();
    env.elevated
        .results
        .lock()
        .unwrap()
        .push_back(Err(ElevatedError::Refused));
    let (_s, p, _) = served_plan("msi", Some("machine"), Scope::Machine, "", "").await;
    let ctx = ctx!(env);
    let e = install(&ctx, &p).await.unwrap_err();
    assert_eq!(e.to_string(), UAC_REFUSED);
    assert_eq!(env.launcher.calls.lock().unwrap().len(), 0);
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
    assert!(text.contains("pede UAC"), "{text}");
    assert!(text.contains("Licença: EULA — https://x/eula"), "{text}");
    assert!(text.contains("Contoso App 1.0 (msi, machine)"), "{text}");
    assert!(text.contains("Instalador: https://h/setup.msi"), "{text}");
    assert!(text.contains("<LOGPATH>"), "{text}");

    let inno = manifest_of("inno", Some("user"), "https://h/setup.exe", &sha, "", "");
    let text = native(&inno, Scope::User).to_string();
    assert!(!text.contains("pede UAC"), "{text}");
    assert!(!text.contains("abre a janela"), "{text}");

    let exe = manifest_of("exe", None, "https://h/setup.exe", &sha, "", "");
    let text = native(&exe, Scope::User).to_string();
    assert!(text.contains("abre a janela do instalador"), "{text}");
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
    let phases = [Phase::WaitingElevation, running.clone(), Phase::Verifying];

    let glyphs = Glyphs::for_set(GlyphSet::Ascii);
    let throttle = ProgressThrottle::new(Instant::now());
    let line = LineProgress::new(false, glyphs);
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
            .starts_with("Aguardando o UAC ou o instalador elevado")
    );
    assert!(
        phase_status(&Phase::WaitingElevation, Duration::ZERO).contains("não pode ser cancelado")
    );
    assert_eq!(
        phase_title(&running),
        "Instalador de X rodando (pode abrir janelas)"
    );
    assert_eq!(phase_title(&Phase::Verifying), "Conferindo a instalação");

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
async fn elevated_msi_failure_carries_the_log_hint() {
    let env = Env::new();
    env.elevated.results.lock().unwrap().push_back(Ok(1603));
    env.elevated
        .lines
        .lock()
        .unwrap()
        .push("Error 1304. Error writing to file".into());
    let (_s, p, _) = served_plan("msi", Some("machine"), Scope::Machine, "", "").await;
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
