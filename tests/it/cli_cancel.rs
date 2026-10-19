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
use wingetrs::cli::install_cmd::{Cancelled, Interrupts, exit_code, until_interrupted};
use wingetrs::http::Http;
use wingetrs::install::executor::{ExecCtx, LaunchGate, Msiexec, NoHook};
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
        self.plan_of("app", manifest)
    }

    fn plan_of(&self, app: &str, manifest: &str) -> InstallPlan {
        use wingetrs::install::traits::Shortcuts;
        let m = resolve(manifest, Arch::X64).unwrap();
        let root = self.shortcuts.root();
        plan_install(&PlanInput {
            app,
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
            launch: Default::default(),
            runners: wingetrs::install::executor::Runners::disabled(),
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
    let refused = AtomicBool::new(false);
    let mut state = StateDb::open(&w.state_path).unwrap();
    let mut ctx = w.ctx(&mut state, &no_progress);
    let gate = ctx.launch.clone();
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
    assert!(gate.cancelled());
    assert!(!w.layout.staging_dir("app", "1.0").exists());
    assert!(!w.layout.version_dir("app", "1.0").exists());
    assert!(!w.layout.app_dir("app").exists());
    assert_eq!(w.apps(), 0);
}

/// Real extraction into the staging folder, then raises `flag` and pauses: the cancel
/// arrives meanwhile.
fn extract_then_pause(
    flag: &AtomicBool,
    file: &Path,
    name: &str,
    dir: Option<&str>,
    to: Option<&str>,
    dest: &Path,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    let r = wingetrs::install::extract::extract(file, name, dir, to, dest);
    assert!(dest.join("tool.exe").exists(), "{}", dest.display());
    flag.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(300));
    r
}

static EXTRACTING: AtomicBool = AtomicBool::new(false);

fn slow_extract(
    file: &Path,
    name: &str,
    dir: Option<&str>,
    to: Option<&str>,
    dest: &Path,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    extract_then_pause(&EXTRACTING, file, name, dir, to, dest)
}

/// The "Ctrl+C" pressed once `flag` is up.
struct When(&'static AtomicBool, bool);

impl Interrupts for When {
    async fn next(&mut self) {
        if self.1 {
            return std::future::pending().await;
        }
        while !self.0.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.1 = true;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_during_extraction_waits_for_the_worker_then_removes_staging() {
    let w = World::new().await;
    let (url, sha) = w.serve("pkg-1.0.zip", fixture_zip(), None).await;
    let plan = w.plan(&manifest(&url, &sha, Some("pkg")));
    let mut state = StateDb::open(&w.state_path).unwrap();
    let mut ctx = w.ctx(&mut state, &no_progress);
    let gate = ctx.launch.clone();
    let mut ctrl_c = When(&EXTRACTING, false);
    let r = until_interrupted(
        wingetrs::install::executor::execute_install_with(&plan, &mut ctx, slow_extract),
        &mut ctrl_c,
        &gate,
        &|| panic!("nada recusado antes do Windows Installer"),
    )
    .await;
    assert!(r.is_none(), "esperava cancelado: {r:?}");
    assert!(ctrl_c.1);
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

fn msiexec_started() -> bool {
    MSIEXEC.0.lock().unwrap().0
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

/// Ctrl+C pressed twice once the fake `msiexec` runs; asked a third time (both presses were
/// handled) it lets `msiexec` finish and presses no more.
struct TwiceWhileMsiexecRuns(usize);

impl Interrupts for TwiceWhileMsiexecRuns {
    async fn next(&mut self) {
        self.0 += 1;
        if self.0 > 2 {
            release_msiexec();
            return std::future::pending().await;
        }
        while !msiexec_started() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_during_installer_waits() {
    let w = World::new().await;
    let (url, sha) = w.serve("pkg-1.0.msi", ole_like(), None).await;
    let plan = w.plan(&manifest(&url, &sha, None));
    let refusals = std::sync::atomic::AtomicUsize::new(0);
    let mut state = StateDb::open(&w.state_path).unwrap();
    let mut ctx = w.ctx(&mut state, &no_progress);
    ctx.msiexec = Some(Msiexec {
        busy: never_busy,
        launch: waiting_admin,
    });
    let gate = ctx.launch.clone();
    let mut ctrl_c = TwiceWhileMsiexecRuns(0);
    let r = until_interrupted(
        wingetrs::install::executor::execute_install_with(&plan, &mut ctx, unsupported),
        &mut ctrl_c,
        &gate,
        &|| {
            refusals.fetch_add(1, Ordering::SeqCst);
        },
    )
    .await;
    assert!(msiexec_started(), "o msiexec falso nunca começou");
    match r {
        Some(Ok(())) => {}
        other => panic!("esperava a instalação concluída: {other:?}"),
    }
    // Two presses handled, one message.
    assert_eq!(ctrl_c.0, 3);
    assert_eq!(refusals.load(Ordering::SeqCst), 1);
    assert!(!gate.cancelled());
    let vdir = w.layout.version_dir("app", "1.0");
    assert_eq!(std::fs::read(vdir.join("tool.txt")).unwrap(), b"ok");
    let app_dir = w.layout.app_dir("app");
    assert!(!app_dir.join(".staging-1.0.msiexec").exists());
    assert!(!w.layout.staging_dir("app", "1.0").exists());
    assert_eq!(w.apps(), 1);
}

#[test]
fn launch_gate_first_come_wins_in_both_orders() {
    // Launch first: Ctrl+C refused, a 1618 relaunch still allowed.
    let g = LaunchGate::default();
    assert!(g.try_launch());
    assert!(!g.try_cancel());
    assert!(g.try_launch());
    assert!(!g.cancelled());
    // Cancel first: the launch is refused, the cancel stays.
    let g = LaunchGate::default();
    assert!(g.try_cancel());
    assert!(!g.try_launch());
    assert!(g.try_cancel());
    assert!(g.cancelled());
    // A new plan is cancellable again, and clones share the decision.
    let g = LaunchGate::default();
    let worker = g.clone();
    assert!(worker.try_launch());
    g.next_plan();
    assert!(g.try_cancel());
    assert!(!worker.try_launch());
}

fn quick_admin(_msi: &Path, target: &Path, _log: &Path) -> std::io::Result<Attempt> {
    std::fs::create_dir_all(target)?;
    std::fs::write(target.join("tool.txt"), b"ok")?;
    Ok(Attempt::Done)
}

static EXTRACTING_NEXT: AtomicBool = AtomicBool::new(false);

fn slow_extract_next(
    file: &Path,
    name: &str,
    dir: Option<&str>,
    to: Option<&str>,
    dest: &Path,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    extract_then_pause(&EXTRACTING_NEXT, file, name, dir, to, dest)
}

/// A batch (dependency, then target): `msiexec` ran in the first plan; Ctrl+C during the
/// second plan cancels it again.
#[tokio::test(flavor = "multi_thread")]
async fn ctrl_c_cancels_the_next_plan_after_msiexec_ran() {
    let w = World::new().await;
    let (url, sha) = w.serve("pkg-1.0.msi", ole_like(), None).await;
    let first = w.plan(&manifest(&url, &sha, None));
    let (url, sha) = w.serve("other-1.0.zip", fixture_zip(), None).await;
    let second = w.plan_of("other", &manifest(&url, &sha, Some("pkg")));
    let mut state = StateDb::open(&w.state_path).unwrap();
    let mut ctx = w.ctx(&mut state, &no_progress);
    ctx.msiexec = Some(Msiexec {
        busy: never_busy,
        launch: quick_admin,
    });
    let gate = ctx.launch.clone();
    let mut ctrl_c = When(&EXTRACTING_NEXT, false);
    let batch = async {
        wingetrs::install::executor::execute_install_with(&first, &mut ctx, unsupported).await?;
        wingetrs::install::executor::execute_install_with(&second, &mut ctx, slow_extract_next)
            .await
    };
    let r = until_interrupted(batch, &mut ctrl_c, &gate, &|| {
        panic!("o msiexec do plano anterior não trava o seguinte")
    })
    .await;
    assert!(r.is_none(), "esperava cancelado: {r:?}");
    assert!(ctrl_c.1);
    assert!(w.layout.version_dir("app", "1.0").join("tool.txt").exists());
    assert!(!w.layout.app_dir("other").exists());
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
    let gate = LaunchGate::default();
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

/// When `always_busy` was first asked.
static FIRST_BUSY: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
/// Set when the queue kept asking after the cancel (it should stop at once).
static WAITED_ON: AtomicBool = AtomicBool::new(false);

/// Always busy, but a hang detector: asked again 5 s after the first time (2.5x the 2 s
/// poll), it records that and frees the queue instead of letting it wait 600 s. A timeout
/// around the test could not fire: the dropped install waits inside the poll, synchronously.
fn always_busy() -> bool {
    let first = *FIRST_BUSY.get_or_init(std::time::Instant::now);
    // clock-ok: hang detector, 2.5x the poll
    if first.elapsed() >= Duration::from_secs(5) {
        WAITED_ON.store(true, Ordering::SeqCst);
        return false;
    }
    true
}

static QUEUED_LAUNCHED: AtomicBool = AtomicBool::new(false);

fn must_not_launch(_: &Path, _: &Path, _: &Path) -> std::io::Result<Attempt> {
    QUEUED_LAUNCHED.store(true, Ordering::SeqCst);
    Ok(Attempt::Done)
}

/// Ctrl+C while queued behind another installer: the install is cancelled at once and
/// `msiexec` is never launched afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn cancel_while_queued_never_launches_msiexec() {
    let w = World::new().await;
    let (url, sha) = w.serve("pkg-1.0.msi", ole_like(), None).await;
    let plan = w.plan(&manifest(&url, &sha, None));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut state = StateDb::open(&w.state_path).unwrap();
    let progress = |ev: &ProgressEvent| {
        if matches!(ev, ProgressEvent::Phase(Phase::WaitingInstaller)) {
            let _ = tx.send(());
        }
    };
    let mut ctx = w.ctx(&mut state, &progress);
    ctx.msiexec = Some(Msiexec {
        busy: always_busy,
        launch: must_not_launch,
    });
    let gate = ctx.launch.clone();
    let r = until_interrupted(
        wingetrs::install::executor::execute_install_with(&plan, &mut ctx, unsupported),
        &mut rx,
        &gate,
        &|| panic!("nada recusado antes do Windows Installer"),
    )
    .await;
    assert!(r.is_none(), "esperava cancelado: {r:?}");
    assert!(
        !WAITED_ON.load(Ordering::SeqCst),
        "a fila seguiu esperando depois do Ctrl+C"
    );
    assert!(!QUEUED_LAUNCHED.load(Ordering::SeqCst));
    assert!(gate.cancelled());
    let app_dir = w.layout.app_dir("app");
    assert!(!app_dir.join(".staging-1.0.msiexec").exists());
    assert!(!w.layout.staging_dir("app", "1.0").exists());
    assert!(!app_dir.exists());
    assert_eq!(w.apps(), 0);
}

#[test]
fn exit_code_is_130_only_for_ctrl_c() {
    assert_eq!(exit_code(&anyhow::Error::from(Cancelled)), 130);
    assert_eq!(
        exit_code(&anyhow::Error::from(Cancelled).context("instalar")),
        130
    );
    assert_eq!(exit_code(&anyhow::anyhow!("cancelado pelo usuário")), 1);
    assert_eq!(
        exit_code(&anyhow::anyhow!("falhou").context("cancelado")),
        1
    );
}
// ---- the inline selector (Task 200): keys and terminal injected, never the real one ----

mod selector {
    use crossterm::event::KeyCode::{self, Char, Down, Enter, Esc, Up};
    use crossterm::event::{Event, KeyEvent, KeyModifiers};
    use wingetrs::cli::picker::fake::{self, ScriptedTerm, ctrl_c, key};
    use wingetrs::cli::picker::{self, Letters, LineKind};
    use wingetrs::tui::picker::{Picker, PickerItem};
    use wingetrs::tui::theme::{ColorDepth, GlyphSet, Glyphs, Theme};

    fn items(labels: &[&str]) -> Vec<PickerItem> {
        labels
            .iter()
            .map(|l| PickerItem {
                label: l.to_string(),
                detail: String::new(),
                selectable: true,
                note: None,
            })
            .collect()
    }

    fn theme() -> Theme {
        Theme::new(true, ColorDepth::NoColor)
    }

    fn ascii() -> &'static Glyphs {
        Glyphs::for_set(GlyphSet::Ascii)
    }

    /// One choice among `labels` answered with `keys`: (result, plain text, raw bytes).
    fn one(keys: Vec<Event>, labels: &[&str]) -> (Option<usize>, String, String) {
        let mut t = ScriptedTerm::new(keys);
        let screen = t.screen();
        let got = picker::select_one(
            &mut t,
            None,
            items(labels),
            None,
            Letters::Pick,
            &theme(),
            ascii(),
        )
        .unwrap();
        let s = screen.lock().unwrap();
        assert!(!s.raw, "raw mode left on");
        assert_eq!(s.raw_enabled, 1);
        let raw = String::from_utf8_lossy(&s.out).into_owned();
        (got, fake::plain(&s.out), raw)
    }

    const HINT_ONE: &str = "setas movem · letra ou Enter escolhe · Esc cancela";
    const HINT_MANY: &str = "setas movem · Espaço marca · Enter confirma · Esc cancela";

    #[test]
    fn selector_arrows_and_enter_pick() {
        let (got, text, raw) = one(
            vec![key(Down), key(Down), key(Up), key(Enter)],
            &["Alfa", "Beta", "Gama"],
        );
        assert_eq!(got, Some(1));
        let first = format!("> a) Alfa\r\n  b) Beta\r\n  g) Gama\r\n{HINT_ONE}\r\n");
        assert!(text.starts_with(&first), "{text:?}");
        // Inline: never the alternate screen; the 4 lines are erased before the summary.
        assert!(!raw.contains("\u{1b}[?1049h"), "{raw:?}");
        assert!(raw.contains("\u{1b}[4A\u{1b}[1G\u{1b}[J"), "{raw:?}");
        assert!(
            text.ends_with(&format!("{HINT_ONE}\r\n> Beta\r\n")),
            "{text:?}"
        );
    }

    #[test]
    fn selector_esc_q_and_ctrl_c_cancel() {
        for k in [key(Esc), key(Char('q')), ctrl_c()] {
            let (got, text, _) = one(vec![key(Down), k.clone()], &["Alfa", "Beta"]);
            assert_eq!(got, None, "{k:?}");
            // Nothing is left on screen after a cancel: no summary line.
            assert!(
                text.ends_with(&format!("{HINT_ONE}\r\n")),
                "{k:?}: {text:?}"
            );
        }
    }

    #[test]
    fn selector_letter_picks_its_row() {
        let (got, text, _) = one(vec![key(Char('G'))], &["Alfa", "Beta", "Gama"]);
        assert_eq!(got, Some(2));
        assert!(text.ends_with("> Gama\r\n"), "{text:?}");
        let mut list = items(&[
            "Copiar e remover",
            "Cancelar",
            "Remover",
            "quieto",
            "9 itens",
        ]);
        list.push(PickerItem {
            selectable: false,
            ..items(&["Zeta"])[0].clone()
        });
        assert_eq!(
            picker::shortcuts(&list),
            vec![None, None, Some('r'), None, Some('9'), None]
        );
    }

    #[test]
    fn selector_yes_no_defaults_to_no() {
        let ask = |keys: Vec<Event>| {
            let mut t = ScriptedTerm::new(keys);
            let screen = t.screen();
            let yes = picker::yes_no(&mut t, "Continuar?", &theme(), ascii()).unwrap();
            let text = fake::plain(&screen.lock().unwrap().out);
            (yes, text)
        };
        let (yes, text) = ask(vec![key(Enter)]);
        assert!(!yes);
        assert!(
            text.starts_with("Continuar?\r\n  s) Sim\r\n> n) Não\r\n"),
            "{text:?}"
        );
        assert!(text.ends_with("Continuar? Não\r\n"), "{text:?}");
        let (yes, text) = ask(vec![key(Char('s'))]);
        assert!(yes);
        assert!(text.ends_with("Continuar? Sim\r\n"), "{text:?}");
        assert!(ask(vec![key(Up), key(Enter)]).0);
        assert!(!ask(vec![key(Char('n'))]).0);
        assert!(!ask(vec![key(Esc)]).0);
    }

    #[test]
    fn selector_space_marks_several() {
        let many = |keys: Vec<Event>| {
            let mut t = ScriptedTerm::new(keys);
            let screen = t.screen();
            let got = picker::select_many(
                &mut t,
                None,
                items(&["Alfa", "Beta", "Gama"]),
                &[true, false, false],
                &theme(),
                ascii(),
            )
            .unwrap();
            (got, fake::plain(&screen.lock().unwrap().out))
        };
        let (got, text) = many(vec![key(Down), key(Char(' ')), key(Enter)]);
        assert_eq!(got, Some(vec![0, 1]));
        let first = format!("> [x] Alfa\r\n  [ ] Beta\r\n  [ ] Gama\r\n{HINT_MANY}\r\n");
        assert!(text.starts_with(&first), "{text:?}");
        assert!(text.ends_with("> Alfa, Beta\r\n"), "{text:?}");
        // A letter moves the cursor (it does not confirm); Space then marks that row.
        let (got, _) = many(vec![key(Char('g')), key(Char(' ')), key(Enter)]);
        assert_eq!(got, Some(vec![0, 2]));
        let (got, text) = many(vec![key(Char(' ')), key(Enter)]);
        assert_eq!(got, Some(vec![]));
        assert!(text.ends_with("> nenhum\r\n"), "{text:?}");
        assert_eq!(many(vec![key(Esc)]).0, None);
    }

    #[test]
    fn selector_frame_text() {
        let mut list = items(&["a", "b", "c", "d", "e"]);
        list[1].detail = "1.0".into();
        list[1].note = Some("nota".into());
        list[2].selectable = false;
        let mut p = Picker::multi(list, &[false, true]);
        for _ in 0..3 {
            p.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        // Cursor on `e` (the disabled `c` is skipped); a 3-row window follows it.
        let lines = picker::frame(&p, 3, Some("Quais?"), Letters::Move, ascii());
        let text: Vec<(&str, LineKind)> = lines.iter().map(|l| (l.text.as_str(), l.kind)).collect();
        assert_eq!(
            text,
            vec![
                ("Quais?", LineKind::Prompt),
                ("  [ ] c", LineKind::Disabled),
                ("  [ ] d", LineKind::Normal),
                ("> [ ] e", LineKind::Current),
                (HINT_MANY, LineKind::Hint),
            ]
        );
        let p = Picker::multi(items(&["a", "b"]), &[false, true]);
        let lines = picker::frame(&p, 5, None, Letters::Move, ascii());
        assert_eq!(lines[1].text, "  [x] b");
        let mut list = items(&["a", "b"]);
        list[1].detail = "1.0".into();
        list[1].note = Some("nota".into());
        let lines = picker::frame(&Picker::single(list), 5, None, Letters::Pick, ascii());
        assert_eq!(lines[1].text, "  b) b  1.0  (nota)");
        // Rows that share an initial get no letter; the column stays aligned.
        let lines = picker::frame(
            &Picker::single(items(&["Remover", "Registro", "Cancelar"])),
            5,
            None,
            Letters::Pick,
            ascii(),
        );
        assert_eq!(lines[0].text, ">    Remover");
        assert_eq!(lines[2].text, "  c) Cancelar");
        // No row has a letter: no letter column at all.
        let lines = picker::frame(
            &Picker::single(items(&["a1", "a2"])),
            5,
            None,
            Letters::Move,
            ascii(),
        );
        assert_eq!(lines[0].text, "> a1");
        assert_eq!(
            lines[2].text,
            "setas ou letra movem · Enter escolhe · Esc cancela"
        );
    }

    #[test]
    fn selector_restores_terminal_on_panic() {
        let mut t = ScriptedTerm::new(vec![key(Down)]);
        t.panic_when_empty = true;
        let screen = t.screen();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            picker::select_one(
                &mut t,
                None,
                items(&["Alfa", "Beta"]),
                None,
                Letters::Pick,
                &theme(),
                ascii(),
            )
        }));
        assert!(r.is_err(), "the scripted read should have panicked");
        let s = screen.lock().unwrap();
        assert_eq!(s.raw_enabled, 1);
        assert!(!s.raw, "raw mode left on after a panic");
        let raw = String::from_utf8_lossy(&s.out);
        // The drawing is erased and the cursor shown again.
        assert!(
            raw.ends_with("\u{1b}[3A\u{1b}[1G\u{1b}[J\u{1b}[?25h\u{1b}[0m"),
            "{raw:?}"
        );
    }

    #[test]
    fn selector_without_terminal_refuses() {
        let mut t = ScriptedTerm::new(vec![key(Enter)]);
        t.interactive = false;
        let screen = t.screen();
        let e = picker::select_one(
            &mut t,
            None,
            items(&["Alfa"]),
            None,
            Letters::Pick,
            &theme(),
            ascii(),
        )
        .unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::Unsupported);
        let s = screen.lock().unwrap();
        assert_eq!((s.raw_enabled, s.reads, s.out.len()), (0, 0, 0));
    }

    /// Every `ESC [ n A` (move up n lines) in `raw`: the height of each erased frame.
    fn move_ups(raw: &str) -> Vec<usize> {
        raw.split("\u{1b}[")
            .skip(1)
            .filter_map(|p| {
                let n: String = p.chars().take_while(char::is_ascii_digit).collect();
                (p[n.len()..].starts_with('A') && !n.is_empty()).then(|| n.parse().unwrap())
            })
            .collect()
    }

    fn numbered(n: usize) -> Vec<&'static str> {
        (1..=n)
            .map(|i| &*Box::leak(format!("item {i}").into_boxed_str()))
            .collect()
    }

    /// Runs one choice of `labels` on a `cols` x `rows` terminal: (result, plain, raw).
    fn sized(
        keys: Vec<Event>,
        labels: &[&str],
        size: (u16, u16),
    ) -> (Option<usize>, String, String) {
        let mut t = ScriptedTerm::new(keys);
        t.size = size;
        let screen = t.screen();
        let got = picker::select_one(
            &mut t,
            None,
            items(labels),
            None,
            Letters::Pick,
            &theme(),
            ascii(),
        )
        .unwrap();
        let s = screen.lock().unwrap();
        let raw = String::from_utf8_lossy(&s.out).into_owned();
        (got, fake::plain(&s.out), raw)
    }

    #[test]
    fn selector_cuts_lines_to_the_terminal_width() {
        let long = "Um rótulo comprido demais 漢字漢字 com 🙂 no fim";
        let (got, text, _) = sized(vec![key(Down), key(Esc)], &[long, "Beta"], (20, 24));
        assert_eq!(got, None);
        let lines: Vec<&str> = text.split("\r\n").filter(|l| !l.is_empty()).collect();
        assert!(lines.len() >= 6, "{lines:?}");
        for l in &lines {
            let w = ratatui::text::Span::raw(*l).width();
            assert!(w <= 19, "{w} columns: {l:?}");
        }
        // Wide characters never split a column: a CJK pair either fits or is dropped.
        assert_eq!(picker::fit("ab漢字", 3), "ab");
        assert_eq!(picker::fit("ab漢字", 4), "ab漢");
    }

    #[test]
    fn selector_window_follows_the_cursor_in_a_short_terminal() {
        let labels = numbered(30);
        let mut keys = vec![key(Down); 29];
        keys.push(key(Enter));
        let (got, text, raw) = sized(keys, &labels, (80, 8));
        assert_eq!(got, Some(29));
        let ups = move_ups(&raw);
        assert_eq!(ups.len(), 30, "29 redraws and the final erase");
        assert!(ups.iter().all(|&n| n <= 7), "{ups:?}");
        // The last frame shows the last item under the cursor.
        assert!(text.contains("> item 30\r\nsetas"), "{text:?}");
        // A tall terminal: still at most 15 rows plus the hint.
        let (_, _, raw) = sized(vec![key(Down), key(Enter)], &labels, (80, 40));
        assert_eq!(move_ups(&raw), vec![16, 16]);
    }

    #[test]
    fn selector_redraws_after_a_resize() {
        let labels = numbered(30);
        let keys = vec![
            Event::Resize(80, 8),
            key(Down),
            Event::Resize(30, 40),
            key(Enter),
        ];
        let (got, text, raw) = sized(keys, &labels, (80, 40));
        assert_eq!(got, Some(1));
        // 16 lines, then 7 after shrinking, 7 again, 16 after growing; the final erase
        // covers exactly the last frame.
        assert_eq!(move_ups(&raw), vec![16, 7, 7, 16]);
        assert!(text.ends_with("> item 2\r\n"), "{text:?}");
    }
}
