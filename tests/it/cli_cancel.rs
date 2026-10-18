//! Ctrl+C during installs from the CLI (Task 140).
//! The "Ctrl+C" is injected (a channel or a test source), never a real console signal.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::DataDir;
use wingetrs::cli::install_cmd::{InstallerGate, Interrupts, until_interrupted};
use wingetrs::http::Http;
use wingetrs::install::executor::{ExecCtx, Msiexec, NoHook};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::msi_queue::Attempt;
use wingetrs::install::plan::InstallPlan;
use wingetrs::install::planner::{PlanInput, plan_install};
use wingetrs::install::progress::{Phase, ProgressEvent};
use wingetrs::install::state::StateDb;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{Effects, EnvStore, EnvValue};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fake_pe(subsystem: u16) -> Vec<u8> {
    let mut b = vec![0u8; 0x80 + 24 + 96];
    b[0] = b'M';
    b[1] = b'Z';
    b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    let off = 0x80 + 24 + 68;
    b[off..off + 2].copy_from_slice(&subsystem.to_le_bytes());
    b
}

fn fixture_zip() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("pkg/tool.exe", o).unwrap();
        z.write_all(&fake_pe(3)).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

/// Starts like an OLE file (what makes an `.msi` eligible for `msiexec /a`); the rest is filler.
fn ole_like() -> Vec<u8> {
    let mut b = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    b.resize(512, 0);
    b
}

fn manifest(url: &str, sha: &str, extract_dir: Option<&str>) -> String {
    let mut m = serde_json::json!({ "version": "1.0", "url": url, "hash": sha });
    if let Some(d) = extract_dir {
        m["extract_dir"] = serde_json::json!(d);
    }
    m.to_string()
}

fn no_progress(_: &ProgressEvent) {}
fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

struct World {
    server: MockServer,
    _t: TempDir,
    layout: Layout,
    data: DataDir,
    env: FakeEnv,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    state_path: PathBuf,
    http: Http,
}

impl World {
    async fn new() -> World {
        let server = MockServer::start().await;
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        let layout = Layout::new(&root);
        std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
        std::fs::write(layout.launcher(), fake_pe(3)).unwrap();
        let env = FakeEnv::new();
        env.set(
            "Path",
            &EnvValue {
                value: r"C:\Existing".into(),
                expand: true,
            },
        )
        .unwrap();
        let shortcuts = FakeShortcuts::new(t.path().join("startmenu"));
        let state_path = t.path().join("state").join("state.db");
        StateDb::open(&state_path).unwrap();
        World {
            server,
            layout,
            data: DataDir::new(&root),
            env,
            shortcuts,
            trash: FakeTrash::new(),
            registry: FakeRegistry::new(),
            state_path,
            http: Http::new().unwrap(),
            _t: t,
        }
    }

    /// Serves `body` at `/<name>` (after `delay`, if any) and returns (url, sha256).
    async fn serve(&self, name: &str, body: Vec<u8>, delay: Option<Duration>) -> (String, String) {
        let sha = hex::encode(Sha256::digest(&body));
        let mut response = ResponseTemplate::new(200).set_body_bytes(body);
        if let Some(d) = delay {
            response = response.set_delay(d);
        }
        Mock::given(method("GET"))
            .and(path(format!("/{name}")))
            .respond_with(response)
            .mount(&self.server)
            .await;
        (format!("{}/{name}", self.server.uri()), sha)
    }

    fn plan(&self, manifest: &str) -> InstallPlan {
        use wingetrs::install::traits::Shortcuts;
        let m = resolve(manifest, Arch::X64).unwrap();
        let root = self.shortcuts.root();
        plan_install(&PlanInput {
            app: "app",
            bucket: "main",
            manifest: &m,
            layout: &self.layout,
            shortcut_root: &root,
            installed: None,
            launcher_present: true,
            keep_old: false,
            manifest_sha256: "",
        })
        .unwrap()
    }

    fn ctx<'a>(
        &'a self,
        state: &'a mut StateDb,
        progress: wingetrs::install::download::Progress<'a>,
    ) -> ExecCtx<'a> {
        ExecCtx {
            layout: &self.layout,
            data: &self.data,
            http: &self.http,
            effects: Effects {
                env: &self.env,
                shortcuts: &self.shortcuts,
                trash: &self.trash,
                registry: &self.registry,
            },
            state,
            hook: &NoHook,
            progress,
            in_use: &nothing_in_use,
            msiexec: None,
        }
    }

    fn apps(&self) -> usize {
        StateDb::open(&self.state_path)
            .unwrap()
            .apps()
            .unwrap()
            .len()
    }
}

/// The "Ctrl+C" of test 1: pressed once the download request reached the server.
struct WhenRequested<'a> {
    server: &'a MockServer,
    fired: bool,
}

impl Interrupts for WhenRequested<'_> {
    async fn next(&mut self) {
        if self.fired {
            return std::future::pending().await;
        }
        loop {
            let seen = self.server.received_requests().await.unwrap_or_default();
            if !seen.is_empty() {
                self.fired = true;
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

#[tokio::test]
async fn cancel_before_installer_rolls_back() {
    let w = World::new().await;
    // The download never ends within the test: the cancel lands in the middle of it.
    let (url, sha) = w
        .serve("pkg-1.0.zip", fixture_zip(), Some(Duration::from_secs(600)))
        .await;
    let plan = w.plan(&manifest(&url, &sha, Some("pkg")));
    let gate = InstallerGate::default();
    let refused = AtomicBool::new(false);
    let mut state = StateDb::open(&w.state_path).unwrap();
    let progress = |ev: &ProgressEvent| gate.observe(ev);
    let mut ctx = w.ctx(&mut state, &progress);
    let mut ctrl_c = WhenRequested {
        server: &w.server,
        fired: false,
    };
    let r = until_interrupted(
        wingetrs::install::executor::execute_install(&plan, &mut ctx),
        &mut ctrl_c,
        &gate,
        &|| refused.store(true, Ordering::SeqCst),
    )
    .await;
    assert!(r.is_none(), "esperava cancelado: {r:?}");
    assert!(ctrl_c.fired);
    assert!(!refused.load(Ordering::SeqCst));
    assert!(!w.layout.staging_dir("app", "1.0").exists());
    assert!(!w.layout.version_dir("app", "1.0").exists());
    assert!(!w.layout.app_dir("app").exists());
    assert_eq!(w.apps(), 0);
}

static EXTRACTING: AtomicBool = AtomicBool::new(false);

/// Real extraction into the staging folder, then a pause: the cancel arrives meanwhile.
fn slow_extract(
    file: &Path,
    name: &str,
    dir: Option<&str>,
    to: Option<&str>,
    dest: &Path,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    let r = wingetrs::install::extract::extract(file, name, dir, to, dest);
    assert!(dest.join("tool.exe").exists(), "{}", dest.display());
    EXTRACTING.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(300));
    r
}

/// The "Ctrl+C" of the extraction test: pressed once the extraction filled the staging folder.
struct WhenExtracting(bool);

impl Interrupts for WhenExtracting {
    async fn next(&mut self) {
        if self.0 {
            return std::future::pending().await;
        }
        while !EXTRACTING.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.0 = true;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_during_extraction_waits_for_the_worker_then_removes_staging() {
    let w = World::new().await;
    let (url, sha) = w.serve("pkg-1.0.zip", fixture_zip(), None).await;
    let plan = w.plan(&manifest(&url, &sha, Some("pkg")));
    let gate = InstallerGate::default();
    let mut state = StateDb::open(&w.state_path).unwrap();
    let progress = |ev: &ProgressEvent| gate.observe(ev);
    let mut ctx = w.ctx(&mut state, &progress);
    let mut ctrl_c = WhenExtracting(false);
    let r = until_interrupted(
        wingetrs::install::executor::execute_install_with(&plan, &mut ctx, slow_extract),
        &mut ctrl_c,
        &gate,
        &|| panic!("nada recusado antes do Windows Installer"),
    )
    .await;
    assert!(r.is_none(), "esperava cancelado: {r:?}");
    assert!(ctrl_c.0);
    // The worker had written the staging folder; the cancel waited for it and removed it.
    assert!(!w.layout.staging_dir("app", "1.0").exists());
    assert!(!w.layout.version_dir("app", "1.0").exists());
    assert!(!w.layout.app_dir("app").exists());
    assert_eq!(w.apps(), 0);
}

fn unsupported(
    _: &Path,
    _: &str,
    _: Option<&str>,
    _: Option<&str>,
    _: &Path,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    Err(wingetrs::install::extract::ExtractError::Unsupported(
        "formato que só o Windows Installer lê".into(),
    ))
}

fn never_busy() -> bool {
    false
}

/// `(started, released)` of the fake `msiexec`.
static MSIEXEC: (Mutex<(bool, bool)>, Condvar) = (Mutex::new((false, false)), Condvar::new());

fn release_msiexec() {
    let (lock, cvar) = &MSIEXEC;
    lock.lock().unwrap().1 = true;
    cvar.notify_all();
}

/// Says it started, then waits to be released (a hang detector of 10 s ends it with a failure
/// code instead) and leaves the administrative tree.
fn waiting_admin(_msi: &Path, target: &Path, _log: &Path) -> std::io::Result<Attempt> {
    assert_eq!(
        target.parent().unwrap().file_name().unwrap(),
        ".staging-1.0.msiexec"
    );
    let (lock, cvar) = &MSIEXEC;
    let mut s = lock.lock().unwrap();
    s.0 = true;
    let (s, timeout) = cvar
        .wait_timeout_while(s, Duration::from_secs(10), |s| !s.1)
        .unwrap();
    drop(s);
    if timeout.timed_out() {
        return Ok(Attempt::Failed(1603));
    }
    std::fs::create_dir_all(target)?;
    std::fs::write(target.join("tool.txt"), b"ok")?;
    Ok(Attempt::Done)
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_during_installer_waits() {
    let w = World::new().await;
    let (url, sha) = w.serve("pkg-1.0.msi", ole_like(), None).await;
    let plan = w.plan(&manifest(&url, &sha, None));
    let gate = InstallerGate::default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let refusals = std::sync::atomic::AtomicUsize::new(0);
    let mut state = StateDb::open(&w.state_path).unwrap();
    // Ctrl+C pressed right after the Windows Installer started.
    let progress = |ev: &ProgressEvent| {
        gate.observe(ev);
        if matches!(ev, ProgressEvent::Phase(Phase::Installer { .. })) {
            tx.send(()).unwrap();
        }
    };
    let mut ctx = w.ctx(&mut state, &progress);
    ctx.msiexec = Some(Msiexec {
        busy: never_busy,
        launch: waiting_admin,
    });
    let r = until_interrupted(
        wingetrs::install::executor::execute_install_with(&plan, &mut ctx, unsupported),
        &mut rx,
        &gate,
        &|| {
            refusals.fetch_add(1, Ordering::SeqCst);
            release_msiexec();
        },
    )
    .await;
    assert!(MSIEXEC.0.lock().unwrap().0, "o msiexec falso nunca começou");
    assert_eq!(refusals.load(Ordering::SeqCst), 1);
    match r {
        Some(Ok(())) => {}
        other => panic!("esperava a instalação concluída: {other:?}"),
    }
    assert!(gate.running());
    let vdir = w.layout.version_dir("app", "1.0");
    assert_eq!(std::fs::read(vdir.join("tool.txt")).unwrap(), b"ok");
    let app_dir = w.layout.app_dir("app");
    assert!(!app_dir.join(".staging-1.0.msiexec").exists());
    assert!(!w.layout.staging_dir("app", "1.0").exists());
    assert_eq!(w.apps(), 1);
}

#[tokio::test]
async fn stale_msiexec_work_dir_is_removed() {
    let w = World::new().await;
    let app_dir = w.layout.app_dir("app");
    let stale = app_dir.join(".staging-1.0.msiexec").join("SourceDir");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("x"), b"x").unwrap();
    // Left by an extraction of a killed run (another version, another process).
    let temp = app_dir.join(".wingetrs-extract-4242-0");
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::write(temp.join("y"), b"y").unwrap();
    let (url, sha) = w.serve("pkg-1.0.zip", fixture_zip(), None).await;
    let plan = w.plan(&manifest(&url, &sha, Some("pkg")));
    let mut state = StateDb::open(&w.state_path).unwrap();
    let mut ctx = w.ctx(&mut state, &no_progress);
    wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap();
    assert!(!app_dir.join(".staging-1.0.msiexec").exists());
    assert!(!temp.exists());
    assert!(w.layout.version_dir("app", "1.0").join("tool.exe").exists());
    assert_eq!(w.apps(), 1);
}

/// Without a source of requests, the operation just runs to the end.
#[tokio::test]
async fn closed_channel_never_cancels() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    drop(tx);
    let gate = InstallerGate::default();
    let r = until_interrupted(
        async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            7
        },
        &mut rx,
        &gate,
        &|| panic!("nada a recusar"),
    )
    .await;
    assert_eq!(r, Some(7));
}

fn always_busy() -> bool {
    true
}

static QUEUED_LAUNCHED: AtomicBool = AtomicBool::new(false);

fn must_not_launch(_: &Path, _: &Path, _: &Path) -> std::io::Result<Attempt> {
    QUEUED_LAUNCHED.store(true, Ordering::SeqCst);
    Ok(Attempt::Done)
}

/// Ctrl+C while queued behind another installer: the install is cancelled and `msiexec` is
/// never launched afterwards (the dropped install does not wait for the queue to run it).
#[tokio::test(flavor = "multi_thread")]
async fn cancel_while_queued_never_launches_msiexec() {
    let w = World::new().await;
    let (url, sha) = w.serve("pkg-1.0.msi", ole_like(), None).await;
    let plan = w.plan(&manifest(&url, &sha, None));
    let gate = InstallerGate::default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut state = StateDb::open(&w.state_path).unwrap();
    let progress = |ev: &ProgressEvent| {
        gate.observe(ev);
        if matches!(ev, ProgressEvent::Phase(Phase::WaitingInstaller)) {
            let _ = tx.send(());
        }
    };
    let mut ctx = w.ctx(&mut state, &progress);
    ctx.msiexec = Some(Msiexec {
        busy: always_busy,
        launch: must_not_launch,
    });
    let r = until_interrupted(
        wingetrs::install::executor::execute_install_with(&plan, &mut ctx, unsupported),
        &mut rx,
        &gate,
        &|| panic!("nada recusado antes do Windows Installer"),
    )
    .await;
    assert!(r.is_none(), "esperava cancelado: {r:?}");
    assert!(!QUEUED_LAUNCHED.load(Ordering::SeqCst));
    assert!(!gate.running());
    let app_dir = w.layout.app_dir("app");
    assert!(!app_dir.join(".staging-1.0.msiexec").exists());
    assert!(!w.layout.staging_dir("app", "1.0").exists());
    assert!(!app_dir.exists());
    assert_eq!(w.apps(), 0);
}
