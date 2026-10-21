#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Task 218: an administrator terminal refuses third-party code and user-scope winget
//! removals in the plan, before any question or effect (CLI and TUI, same text).
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use clap::Parser;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::bridge::BridgeJob;
use wingetrs::bridge::worker::ScopeArg;
use wingetrs::cache::DataDir;
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::cli::Cli;
use wingetrs::cli::install_cmd::{self, Flags, Interrupts, Stop, Ui};
use wingetrs::cli::winget_cmd::{self, WingetFlows};
use wingetrs::config::{Behavior, ConfirmMode};
use wingetrs::core::SourceKind;
use wingetrs::http::Http;
use wingetrs::install::executor::{Runners, ScriptHost, ScriptJob, ScriptOutcome};
use wingetrs::install::ops::{self, OpsCtx, ResidueScan};
use wingetrs::install::script_policy::{AdminBlock, admin_terminal_refusal, elevated_or_unknown};
use wingetrs::install::target::Candidate;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::installed::Hive;
use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
use wingetrs::tui::app::{
    Action, App, Done, InstallFail, Modal, Msg, Planned, Tab, ToastKind, update,
};
use wingetrs::tui::jobs;
use wingetrs::tui::picker::PickerItem;
use wingetrs::tui::theme::GlyphSet;
use wingetrs::update::LockWait;
use wingetrs::winget_install::engine::UpgradeError;
use wingetrs::winget_install::launch::Elevation;
use wingetrs::winget_install::ops::{InstallReport, WingetOpsError};
use wingetrs::winget_install::orphans::Orphan;
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan};
use wingetrs::winget_install::select::{Refusal, ScopeWanted};
use wingetrs::winget_install::uninstall::{UninstallPlan, UninstallTarget};
use wingetrs::winget_install::uninstall_cmd::{ArpDetails, uninstall_command};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SCRIPT_REFUSAL: &str = "script post_install é código de terceiro e o wingetrs não o roda como administrador; abra um terminal sem administrador e repita";
const WINGET_REFUSAL: &str = "pacote instalado só para você: o winget não o remove de um terminal de administrador; abra um terminal sem administrador e repita";

fn elevated() -> std::io::Result<bool> {
    Ok(true)
}
fn not_elevated() -> std::io::Result<bool> {
    Ok(false)
}
fn token_error() -> std::io::Result<bool> {
    Err(std::io::Error::other("token ilegível"))
}

// ---- fakes ----

#[derive(Default)]
struct CountHost {
    runs: AtomicUsize,
}

impl ScriptHost for CountHost {
    fn run(&self, _: &ScriptJob) -> std::io::Result<ScriptOutcome> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(ScriptOutcome {
            code: 0,
            lines: Vec::new(),
        })
    }
}

/// A picker item as shown: its label and note.
type Shown = (String, Option<String>);

/// A terminal that records every question and every stdout line it is handed.
struct TestUi {
    interactive: bool,
    answer: bool,
    asked: AtomicUsize,
    lines: Mutex<Vec<String>>,
    plans: Mutex<Vec<String>>,
    /// Each picker shown: its items (label, note).
    picks: Mutex<Vec<Vec<Shown>>>,
}

impl TestUi {
    fn new(interactive: bool, answer: bool) -> TestUi {
        TestUi {
            interactive,
            answer,
            asked: AtomicUsize::new(0),
            lines: Mutex::new(Vec::new()),
            plans: Mutex::new(Vec::new()),
            picks: Mutex::new(Vec::new()),
        }
    }
    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
    fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
}

impl Ui for TestUi {
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn confirm(&self, _: &str) -> anyhow::Result<bool> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Ok(self.answer)
    }
    fn pick(&self, items: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let shown = items.into_iter().map(|i| (i.label, i.note)).collect();
        self.picks.lock().unwrap().push(shown);
        Ok(None)
    }
    fn pick_many(&self, _: Vec<PickerItem>, _: &[bool]) -> anyhow::Result<Option<Vec<usize>>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        Ok(None)
    }
    fn output(&self, text: &str) {
        self.lines.lock().unwrap().push(text.to_string());
    }
    fn plan(&self, text: &str) {
        self.plans.lock().unwrap().push(text.to_string());
    }
}

/// The winget side with an injected token; anything that would run the removal (the bridge,
/// the uninstaller, the registry) is unreachable.
struct Flows {
    elevated: fn() -> std::io::Result<bool>,
    rows: Vec<ListRow>,
    plan: Option<UninstallPlan>,
}

impl Flows {
    fn scoop(elevated: fn() -> std::io::Result<bool>) -> Flows {
        Flows {
            elevated,
            rows: Vec::new(),
            plan: None,
        }
    }
}

impl WingetFlows for Flows {
    fn behavior(&self) -> Behavior {
        Behavior::default()
    }
    async fn plan_install(
        &self,
        _: &Candidate,
        _: ScopeWanted,
    ) -> anyhow::Result<Result<PlanOrBridge, Refusal>> {
        unreachable!("winget install")
    }
    fn gate(&self) -> wingetrs::install::executor::LaunchGate {
        unreachable!("removal ran")
    }
    fn progress(&self) -> wingetrs::cli::winget_cmd::Sink {
        unreachable!("removal ran")
    }
    fn install(
        &self,
        _: &WingetPlan,
    ) -> wingetrs::cli::winget_cmd::Op<Result<InstallReport, WingetOpsError>> {
        unreachable!("winget install")
    }
    async fn install_bridge<I: Interrupts>(
        &self,
        _: &dyn Ui,
        _: &Candidate,
        _: Option<ScopeArg>,
        _: Option<&str>,
        _: Flags,
        _: Stop<'_, I>,
    ) -> anyhow::Result<()> {
        unreachable!("bridge")
    }
    fn installed(&self) -> anyhow::Result<Vec<ListRow>> {
        Ok(self.rows.clone())
    }
    async fn plan_uninstall(
        &self,
        _: &ListRow,
        _: &UninstallTarget,
    ) -> Result<UninstallPlan, String> {
        self.plan.clone().ok_or_else(|| "sem plano".to_string())
    }
    fn lock_install(
        &self,
    ) -> wingetrs::cli::winget_cmd::Op<Result<wingetrs::cli::winget_cmd::Held, WingetOpsError>>
    {
        unreachable!("removal ran")
    }
    fn uninstall(&self, _: &UninstallPlan) -> wingetrs::cli::winget_cmd::Uninstaller {
        unreachable!("bridge or uninstaller called")
    }
    fn remove_registry_only(
        &self,
        _: &UninstallPlan,
        _: &str,
    ) -> wingetrs::cli::winget_cmd::RecordRemover {
        unreachable!("registry touched")
    }
    fn free_space(&self) -> fn(&Path) -> Option<u64> {
        unreachable!("removal ran")
    }
    fn pins(&self) -> BTreeSet<String> {
        BTreeSet::new()
    }
    fn elevated(&self) -> fn() -> std::io::Result<bool> {
        self.elevated
    }
    async fn plan_upgrade(
        &self,
        _: &ListRow,
        _: &str,
        _: ScopeWanted,
    ) -> anyhow::Result<(Result<PlanOrBridge, Refusal>, Option<UninstallPlan>)> {
        unreachable!("winget upgrade")
    }
    fn upgrade(
        &self,
        _: &WingetPlan,
        _: Option<&UninstallPlan>,
    ) -> wingetrs::cli::winget_cmd::Op<Result<InstallReport, UpgradeError>> {
        unreachable!("winget upgrade")
    }
    fn upgrade_bridge(&self, _: &BridgeJob, _: &mut dyn FnMut(&str)) -> Result<Done, InstallFail> {
        unreachable!("bridge")
    }
    fn orphans(&self) -> anyhow::Result<Vec<Orphan>> {
        unreachable!("orphans")
    }
    fn backups(&self) -> PathBuf {
        unreachable!("removal ran")
    }
    fn remove_orphan(
        &self,
        _: &Orphan,
        _: &str,
        _: &dyn Fn(&wingetrs::install::progress::ProgressEvent),
    ) -> Result<PathBuf, String> {
        unreachable!("orphans")
    }
}

// ---- scoop world ----

fn fake_pe() -> Vec<u8> {
    let mut b = vec![0u8; 0x80 + 24 + 96];
    b[0] = b'M';
    b[1] = b'Z';
    b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    let off = 0x80 + 24 + 68;
    b[off..off + 2].copy_from_slice(&3u16.to_le_bytes());
    b
}

fn zip_of(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        z.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(bytes).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn no_progress(_: &wingetrs::install::progress::ProgressEvent) {}
fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

fn post_install() -> serde_json::Value {
    serde_json::json!({ "post_install": ["Write-Host 'ola'"] })
}

fn none() -> serde_json::Value {
    serde_json::json!({})
}

struct World {
    server: MockServer,
    _t: TempDir,
    data: DataDir,
    http: Http,
    catalog: Catalog,
    env: FakeEnv,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    host: Arc<CountHost>,
}

impl World {
    async fn new() -> World {
        let t = tempfile::tempdir().unwrap();
        let data = DataDir::new(t.path().join("root"));
        let layout = data.install_layout();
        std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
        std::fs::write(layout.launcher(), fake_pe()).unwrap();
        World {
            server: MockServer::start().await,
            data,
            http: Http::new().unwrap(),
            catalog: Catalog::open_in_memory().unwrap(),
            env: FakeEnv::new(),
            shortcuts: FakeShortcuts::new(t.path().join("startmenu")),
            trash: FakeTrash::new(),
            registry: FakeRegistry::new(),
            host: Arc::new(CountHost::default()),
            _t: t,
        }
    }

    /// Replaces `scoop:main` with `apps` (id, version, manifest extras), serving their zips.
    async fn publish(&mut self, tag: &str, apps: &[(&str, &str, serde_json::Value)]) {
        let mut pkgs = Vec::new();
        for (id, version, extra) in apps {
            let zip = zip_of(&format!("pkg/{id}.exe"), &fake_pe());
            let sha = hex::encode(Sha256::digest(&zip));
            let file = format!("{id}-{version}.zip");
            Mock::given(method("GET"))
                .and(path(format!("/{file}")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
                .mount(&self.server)
                .await;
            let mut m = serde_json::json!({
                "version": version,
                "url": format!("{}/{file}", self.server.uri()),
                "hash": sha,
                "extract_dir": "pkg",
                "bin": format!("{id}.exe"),
            });
            for (k, v) in extra.as_object().unwrap() {
                m[k] = v.clone();
            }
            pkgs.push(NewPackage {
                id: (*id).into(),
                name: (*id).into(),
                version: (*version).into(),
                raw: Some(m.to_string()),
                ..Default::default()
            });
        }
        self.catalog
            .replace_source(
                "scoop:main",
                SourceKind::Scoop,
                tag,
                1,
                &pkgs,
                &WingetMaps::default(),
            )
            .unwrap();
    }

    fn ctx(&self) -> OpsCtx<'_> {
        let scripts: Arc<dyn ScriptHost> = self.host.clone();
        OpsCtx {
            data: &self.data,
            http: &self.http,
            catalog: &self.catalog,
            effects: Effects {
                env: &self.env,
                shortcuts: &self.shortcuts,
                trash: &self.trash,
                registry: &self.registry,
            },
            wait: LockWait::NoWait,
            progress: &no_progress,
            in_use: &nothing_in_use,
            residue: ResidueScan::none(),
            msiexec: wingetrs::install::executor::Msiexec::DISABLED,
            launch: Default::default(),
            runners: Runners {
                scripts: Some(scripts),
                ..Runners::disabled()
            },
        }
    }

    /// `wingetrs <args>` in process, with `ui` and the token `probe`.
    async fn run(
        &self,
        ui: &dyn Ui,
        probe: fn() -> std::io::Result<bool>,
        args: &[&str],
    ) -> anyhow::Result<()> {
        let cli =
            Cli::try_parse_from(std::iter::once("wingetrs").chain(args.iter().copied())).unwrap();
        let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let flows = Flows::scoop(probe);
        install_cmd::execute(&self.ctx(), ui, cli.command.unwrap(), &mut rx, &flows).await
    }

    fn runs(&self) -> usize {
        self.host.runs.load(Ordering::SeqCst)
    }

    async fn requests(&self) -> usize {
        self.server.received_requests().await.unwrap().len()
    }

    fn installed(&self, app: &str) -> Option<String> {
        let state = ops::read_state(&self.data.install_layout()).unwrap();
        state.get(app).unwrap().map(|(rec, _)| rec.version)
    }

    fn extracted(&self, app: &str) -> bool {
        self.data.install_layout().app_dir(app).exists()
    }
}

// ---- winget world ----

const JQ_KEY: &str = "jqlang.jq_Microsoft.Winget.Source_8wekyb3d8bbwe";

fn jq_plan(hive: Hive) -> UninstallPlan {
    let d = ArpDetails {
        hive,
        key: JQ_KEY.into(),
        display_name: Some("jq".into()),
        publisher: None,
        display_version: Some("1.8.2".into()),
        windows_installer: false,
        system_component: false,
        quiet_uninstall_string: None,
        uninstall_string: Some(format!("winget uninstall --product-code {JQ_KEY}")),
        inno_app_path: None,
        bundle_cache_path: None,
        bundle_upgrade_code: None,
        install_location: None,
    };
    UninstallPlan {
        target: UninstallTarget::Arp {
            hive,
            key: JQ_KEY.into(),
        },
        name: "jq".into(),
        version: Some("1.8.2".into()),
        command: uninstall_command(&d, false),
        elevation: Elevation::Never,
        shared: None,
    }
}

fn jq_flows(hive: Hive, probe: fn() -> std::io::Result<bool>) -> Flows {
    let tag = match hive {
        Hive::User => "hkcu",
        Hive::MachineX64 => "hklm64",
        Hive::MachineX86 => "hklm32",
    };
    Flows {
        elevated: probe,
        rows: vec![ListRow {
            name: "jq".into(),
            app_key: format!("arp/{tag}/{JQ_KEY}"),
            origin: "arp".into(),
            installed_version: Some("1.8.2".into()),
            ..Default::default()
        }],
        plan: Some(jq_plan(hive)),
    }
}

/// `wingetrs uninstall jq <extra>` through the winget flow; the stdout lines and the result.
async fn uninstall_jq(flows: &Flows, ui: &TestUi, f: Flags) -> (Vec<String>, anyhow::Result<()>) {
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let gate = Default::default();
    let stop = Stop::new(&mut rx, &gate);
    let mut said = Vec::new();
    let mut say = |l: &str| said.push(l.to_string());
    let r = winget_cmd::uninstall(flows, ui, "jq", false, f, stop, &mut say).await;
    (said, r)
}

fn yes() -> Flags {
    Flags {
        yes: true,
        ..Flags::default()
    }
}

// ---- TUI ----

fn tui(probe: fn() -> std::io::Result<bool>) -> App {
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.size = (80, 24);
    a.settings.behavior.confirm = ConfirmMode::Never;
    a.elevated = probe;
    a
}

fn errors(a: &App) -> Vec<String> {
    a.toasts
        .iter()
        .filter(|t| t.kind == ToastKind::Error)
        .map(|t| t.text.clone())
        .collect()
}

fn wsrc() -> jobs::WingetSrc {
    jobs::WingetSrc {
        handle: tokio::runtime::Handle::current(),
        cdn_base: String::new(),
    }
}

// ---- tests ----

#[test]
fn refusal_texts_fixed() {
    let code = || AdminBlock::ThirdPartyCode {
        what: "script post_install".into(),
    };
    assert_eq!(
        admin_terminal_refusal(true, code()).as_deref(),
        Some(SCRIPT_REFUSAL)
    );
    assert_eq!(
        admin_terminal_refusal(true, AdminBlock::WingetUserScope).as_deref(),
        Some(WINGET_REFUSAL)
    );
    assert_eq!(admin_terminal_refusal(false, code()), None);
    assert_eq!(
        admin_terminal_refusal(false, AdminBlock::WingetUserScope),
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn scoop_script_refused_before_confirm() {
    let mut w = World::new().await;
    w.publish("v1", &[("x", "1.0", post_install())]).await;
    for args in [
        &["install", "x", "--allow-scripts"][..],
        &["install", "x", "--yes", "--allow-scripts"],
        &["install", "x"],
    ] {
        let ui = TestUi::new(true, true);
        let e = w.run(&ui, elevated, args).await.unwrap_err();
        println!("{args:?} => {e}");
        assert_eq!(e.to_string(), SCRIPT_REFUSAL, "{args:?}");
        assert_eq!(
            ui.asked(),
            0,
            "{args:?}: no question read from the terminal"
        );
    }
    assert_eq!(w.requests().await, 0, "nothing downloaded");
    assert!(!w.extracted("x"), "nothing extracted");
    assert_eq!(w.runs(), 0);
    assert_eq!(w.installed("x"), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn scoop_removal_refused_before_confirm() {
    let mut w = World::new().await;
    let pre = serde_json::json!({ "pre_uninstall": ["Write-Host 'tchau'"] });
    w.publish("v1", &[("x", "1.0", pre)]).await;
    let ui = TestUi::new(false, false);
    w.run(
        &ui,
        not_elevated,
        &["install", "x", "--yes", "--allow-scripts"],
    )
    .await
    .unwrap();
    assert_eq!(w.installed("x").as_deref(), Some("1.0"));
    let ui = TestUi::new(true, true);
    let e = w
        .run(
            &ui,
            elevated,
            &["uninstall", "x", "--yes", "--allow-scripts"],
        )
        .await
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "script pre_uninstall é código de terceiro e o wingetrs não o roda como administrador; abra um terminal sem administrador e repita"
    );
    assert_eq!(ui.asked(), 0);
    assert_eq!(w.installed("x").as_deref(), Some("1.0"), "still installed");
    assert_eq!(w.runs(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn scoop_script_allowed_when_not_elevated() {
    let mut w = World::new().await;
    w.publish("v1", &[("x", "1.0", post_install())]).await;
    // The consent block and "Continuar?" come as today: the user says no.
    let ui = TestUi::new(true, false);
    w.run(&ui, not_elevated, &["install", "x", "--allow-scripts"])
        .await
        .unwrap();
    assert_eq!(ui.asked(), 1, "Continuar? was asked");
    assert_eq!(w.installed("x"), None);
    // And says yes: the script runs.
    let ui = TestUi::new(true, true);
    w.run(&ui, not_elevated, &["install", "x", "--allow-scripts"])
        .await
        .unwrap();
    assert_eq!(w.runs(), 1);
    assert_eq!(w.installed("x").as_deref(), Some("1.0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_shows_plan_and_warning() {
    let mut w = World::new().await;
    w.publish("v1", &[("x", "1.0", post_install())]).await;
    let ui = TestUi::new(true, true);
    w.run(&ui, elevated, &["install", "x", "--dry-run"])
        .await
        .unwrap();
    let lines = ui.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].ends_with(&format!("aviso: {SCRIPT_REFUSAL}")),
        "{lines:?}"
    );
    assert_eq!(ui.asked(), 0);
    assert_eq!(w.requests().await, 0);
    assert_eq!(w.runs(), 0);
    // Not elevated: no warning.
    let ui = TestUi::new(true, true);
    w.run(&ui, not_elevated, &["install", "x", "--dry-run"])
        .await
        .unwrap();
    assert!(ui.lines().is_empty());
    // The winget removal's dry run: the plan, then the warning, exit 0.
    let flows = jq_flows(Hive::User, elevated);
    let ui = TestUi::new(true, true);
    let f = Flags {
        dry_run: true,
        ..Flags::default()
    };
    let (said, r) = uninstall_jq(&flows, &ui, f).await;
    r.unwrap();
    let n = said.len();
    assert!(n >= 3, "{said:?}");
    assert!(said[0].contains("jq"), "plan first: {said:?}");
    assert!(
        said[n - 2].ends_with(&format!("aviso: {WINGET_REFUSAL}")),
        "{said:?}"
    );
    assert_eq!(said[n - 1], wingetrs::cli::output::DRY_RUN_DONE);
}

#[tokio::test(flavor = "multi_thread")]
async fn batch_skips_only_the_blocked_item() {
    // CLI: `upgrade --all` where only y's new version has a script.
    let mut w = World::new().await;
    w.publish("v1", &[("x", "1.0", none()), ("y", "1.0", none())])
        .await;
    let ui = TestUi::new(false, false);
    for app in ["x", "y"] {
        w.run(&ui, not_elevated, &["install", app, "--yes"])
            .await
            .unwrap();
    }
    w.publish("v2", &[("x", "2.0", none()), ("y", "2.0", post_install())])
        .await;
    let ui = TestUi::new(true, true);
    w.run(
        &ui,
        elevated,
        &["upgrade", "--all", "--yes", "--allow-scripts"],
    )
    .await
    .unwrap();
    assert_eq!(
        w.installed("x").as_deref(),
        Some("2.0"),
        "the other one goes on"
    );
    assert_eq!(w.installed("y").as_deref(), Some("1.0"), "skipped");
    assert_eq!(w.runs(), 0);
    assert_eq!(ui.asked(), 0);

    // TUI: a batch of two removals, only alpha runs code.
    let mut a = tui(elevated);
    a.installed = vec![irow("alpha"), irow("beta")];
    a.tab = Tab::Installed;
    for i in 0..2 {
        a.selected = i;
        update(&mut a, key(KeyCode::Char(' ')));
    }
    update(&mut a, key(KeyCode::Char('u')));
    let results = vec![
        Ok(Planned::ReadyScripts {
            text: String::new(),
            action: Action::Remove(scoop_removal("alpha")),
            manifest_sha256: "sha".into(),
            code: vec![wingetrs::cli::output::ThirdParty::Script {
                hook: "post_install".into(),
                script: "Write-Host 1".into(),
                report: wingetrs::install::script_scan::ScriptReport {
                    commands: Vec::new(),
                    variables: BTreeSet::new(),
                    flags: Vec::new(),
                    verdict: wingetrs::install::script_scan::Verdict::Confirm,
                },
            }],
        }),
        Ok(Planned::Ready {
            text: String::new(),
            action: Action::Remove(scoop_removal("beta")),
        }),
    ];
    update(&mut a, Msg::BatchPlanned(results));
    let warned: Vec<&str> = a
        .toasts
        .iter()
        .filter(|t| t.kind == ToastKind::Warning)
        .map(|t| t.text.as_str())
        .collect();
    assert!(
        warned.contains(&format!("alpha pulado: {SCRIPT_REFUSAL}").as_str()),
        "{warned:?}"
    );
    let started = a.running.as_ref().and_then(|r| r.action.clone());
    let queued: Vec<Action> = a.queue.iter().map(|q| q.action.clone()).collect();
    let all: Vec<Action> = started.into_iter().chain(queued).collect();
    assert!(
        matches!(all.as_slice(), [Action::Remove(p)] if p.app == "beta"),
        "{all:?}"
    );
    assert!(a.modal.is_none());
}

fn key(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

fn irow(name: &str) -> ListRow {
    ListRow {
        name: name.into(),
        origin: "wingetrs".into(),
        installed_version: Some("1".into()),
        correlation: Correlation::Matched {
            source: "scoop:main".into(),
            package_id: name.into(),
            rule: Rule::ScoopManifest,
        },
        latest: Some("1".into()),
        state: UpdateState::UpToDate,
        ..Default::default()
    }
}

fn scoop_removal(name: &str) -> wingetrs::install::plan::UninstallPlan {
    wingetrs::install::plan::UninstallPlan {
        app: name.into(),
        version: "1.0".into(),
        purge: false,
        artifacts: Vec::new(),
        path_users: Vec::new(),
        remove_path_entry: false,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn winget_user_scope_refused_in_plan() {
    let flows = jq_flows(Hive::User, elevated);
    for f in [yes(), Flags::default()] {
        let ui = TestUi::new(true, true);
        let (said, r) = uninstall_jq(&flows, &ui, f).await;
        assert_eq!(r.unwrap_err().to_string(), WINGET_REFUSAL);
        assert_eq!(ui.asked(), 0, "no question");
        assert!(said.iter().any(|l| l.contains("jq")), "the plan is shown");
    }
    // Not elevated: the removal goes on to its confirmation.
    let flows = jq_flows(Hive::User, not_elevated);
    let ui = TestUi::new(true, false);
    let (_, r) = uninstall_jq(&flows, &ui, Flags::default()).await;
    r.unwrap();
    assert_eq!(ui.asked(), 1);
    // A user record removed by its own uninstaller is not refused.
    let mut plan = jq_plan(Hive::User);
    plan.command.as_mut().unwrap().program = r"C:\jq\uninstall.exe".into();
    plan.command.as_mut().unwrap().args = String::new();
    assert_eq!(winget_cmd::winget_admin_refusal(&plan, true), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn winget_machine_scope_unchanged() {
    for hive in [Hive::MachineX64, Hive::MachineX86] {
        assert_eq!(winget_cmd::winget_admin_refusal(&jq_plan(hive), true), None);
        let flows = jq_flows(hive, elevated);
        let ui = TestUi::new(true, false);
        let (said, r) = uninstall_jq(&flows, &ui, Flags::default()).await;
        r.unwrap();
        assert_eq!(ui.asked(), 1, "{hive:?}: Continuar? as today");
        assert!(said.iter().all(|l| !l.contains(WINGET_REFUSAL)));
        // TUI: the confirmation opens as today.
        let mut a = tui(elevated);
        a.settings.behavior.confirm = ConfirmMode::Always;
        update(&mut a, Msg::WingetUninstallPlanned(Ok(jq_plan(hive))));
        assert!(errors(&a).is_empty(), "{:?}", errors(&a));
        assert!(matches!(a.modal, Some(Modal::Confirm { .. })), "{hive:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_and_tui_same_refusal_text() {
    // Scoop: the CLI error and the TUI toast for the same plan.
    let mut w = World::new().await;
    w.publish("v1", &[("x", "1.0", post_install())]).await;
    let ui = TestUi::new(true, true);
    let cli = w
        .run(&ui, elevated, &["install", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap_err()
        .to_string();
    let planned = jobs::plan_install(&w.ctx(), &wsrc(), "scoop:main", "x", false).unwrap();
    let mut a = tui(elevated);
    update(&mut a, Msg::Planned(Ok(planned)));
    assert_eq!(errors(&a), vec![cli.clone()]);
    assert_eq!(cli, SCRIPT_REFUSAL);
    assert!(a.modal.is_none() && a.running.is_none() && a.queue.is_empty());
    // Not elevated, the TUI opens the confirmation (third-party code always asks).
    let planned = jobs::plan_install(&w.ctx(), &wsrc(), "scoop:main", "x", false).unwrap();
    let mut b = tui(not_elevated);
    update(&mut b, Msg::Planned(Ok(planned)));
    assert!(matches!(b.modal, Some(Modal::Confirm { .. })));
    assert!(errors(&b).is_empty());

    // Winget user scope.
    let flows = jq_flows(Hive::User, elevated);
    let ui = TestUi::new(true, true);
    let (_, r) = uninstall_jq(&flows, &ui, yes()).await;
    let cli = r.unwrap_err().to_string();
    let mut a = tui(elevated);
    update(&mut a, Msg::WingetUninstallPlanned(Ok(jq_plan(Hive::User))));
    assert_eq!(errors(&a), vec![cli.clone()]);
    assert_eq!(cli, WINGET_REFUSAL);
    assert!(a.modal.is_none() && a.running.is_none() && a.queue.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn token_error_counts_as_elevated() {
    assert!(elevated_or_unknown(token_error));
    assert!(elevated_or_unknown(elevated));
    assert!(!elevated_or_unknown(not_elevated));
    let mut w = World::new().await;
    w.publish("v1", &[("x", "1.0", post_install())]).await;
    let ui = TestUi::new(true, true);
    let e = w
        .run(
            &ui,
            token_error,
            &["install", "x", "--yes", "--allow-scripts"],
        )
        .await
        .unwrap_err();
    assert_eq!(e.to_string(), SCRIPT_REFUSAL);
    assert_eq!(w.runs(), 0);
    let flows = jq_flows(Hive::User, token_error);
    let ui = TestUi::new(true, true);
    let (_, r) = uninstall_jq(&flows, &ui, yes()).await;
    assert_eq!(r.unwrap_err().to_string(), WINGET_REFUSAL);
    let mut a = tui(token_error);
    update(&mut a, Msg::WingetUninstallPlanned(Ok(jq_plan(Hive::User))));
    assert_eq!(errors(&a), vec![WINGET_REFUSAL.to_string()]);
}

// ---- review of Task 218 ----

#[tokio::test(flavor = "multi_thread")]
async fn innounp_refused_in_plan() {
    let mut w = World::new().await;
    let inno = serde_json::json!({
        "innosetup": true,
        "url": format!("{}/x-1.0.exe", w.server.uri()),
    });
    // The planner adds innounp (from the catalog) as a dependency.
    w.publish("v1", &[("innounp", "1.0", none()), ("x", "1.0", inno)])
        .await;
    let refusal = "innounp é código de terceiro e o wingetrs não o roda como administrador; abra um terminal sem administrador e repita";
    let ui = TestUi::new(true, true);
    let e = w
        .run(&ui, elevated, &["install", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap_err();
    assert_eq!(e.to_string(), refusal);
    assert_eq!(ui.asked(), 0);
    assert_eq!(w.requests().await, 0);
    assert!(!w.extracted("x") && !w.extracted("innounp"));
    assert_eq!(w.installed("x"), None);
    // TUI: the same plan, the same text.
    let planned = jobs::plan_install(&w.ctx(), &wsrc(), "scoop:main", "x", false).unwrap();
    let mut a = tui(elevated);
    update(&mut a, Msg::Planned(Ok(planned)));
    assert_eq!(errors(&a), vec![refusal.to_string()]);
    assert!(a.modal.is_none() && a.running.is_none() && a.queue.is_empty());
}

fn shared_record_only() -> wingetrs::winget_install::uninstall::SharedData {
    wingetrs::winget_install::uninstall::SharedData {
        folders: Vec::new(),
        size: None,
        registry_only_allowed: true,
        registry_only_refused: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn winget_user_scope_offers_only_the_record() {
    use wingetrs::winget_install::removal::{CANCEL_LABEL, REGISTRY_ONLY_LABEL};
    let mut flows = jq_flows(Hive::User, elevated);
    flows.plan.as_mut().unwrap().shared = Some(shared_record_only());
    // CLI: only "remove the record" and "cancel", the refusal as the note; cancelled here.
    let ui = TestUi::new(true, true);
    let (said, r) = uninstall_jq(&flows, &ui, Flags::default()).await;
    r.unwrap();
    let picks = ui.picks.lock().unwrap().clone();
    assert_eq!(
        picks,
        vec![vec![
            (
                REGISTRY_ONLY_LABEL.to_string(),
                Some(WINGET_REFUSAL.to_string())
            ),
            (CANCEL_LABEL.to_string(), None),
        ]]
    );
    assert!(
        said.last().is_some_and(|l| l.ends_with("Cancelado.")),
        "{said:?}"
    );
    // `--yes` never chooses: the refusal.
    let ui = TestUi::new(true, true);
    let (_, r) = uninstall_jq(&flows, &ui, yes()).await;
    assert_eq!(r.unwrap_err().to_string(), WINGET_REFUSAL);
    assert_eq!(ui.asked(), 0);
    // TUI: the confirmation holds only the record-only removal, with the refusal as a note.
    let mut a = tui(elevated);
    let plan = flows.plan.clone().unwrap();
    update(&mut a, Msg::WingetUninstallPlanned(Ok(plan.clone())));
    assert!(errors(&a).is_empty(), "{:?}", errors(&a));
    let Some(Modal::Confirm { actions, notes, .. }) = &a.modal else {
        panic!("no confirmation");
    };
    assert_eq!(actions, &vec![Action::WingetRegistryOnly(plan)]);
    assert!(notes.contains(&WINGET_REFUSAL.to_string()), "{notes:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn install_ignores_code_that_runs_only_at_removal() {
    let mut w = World::new().await;
    let pre = serde_json::json!({ "pre_uninstall": ["Write-Host 'tchau'"] });
    w.publish("v1", &[("x", "1.0", pre)]).await;
    let ui = TestUi::new(true, true);
    w.run(&ui, elevated, &["install", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap();
    assert_eq!(w.installed("x").as_deref(), Some("1.0"));
    assert_eq!(w.runs(), 0);
    // Its removal runs that code now: refused.
    let e = w
        .run(
            &ui,
            elevated,
            &["uninstall", "x", "--yes", "--allow-scripts"],
        )
        .await
        .unwrap_err();
    assert!(
        e.to_string()
            .starts_with("script pre_uninstall é código de terceiro"),
        "{e}"
    );
    assert_eq!(w.installed("x").as_deref(), Some("1.0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_shows_plan_before_refusal() {
    let mut w = World::new().await;
    w.publish("v1", &[("x", "1.0", none())]).await;
    let ui = TestUi::new(false, false);
    w.run(&ui, not_elevated, &["install", "x", "--yes"])
        .await
        .unwrap();
    w.publish("v2", &[("x", "2.0", post_install())]).await;
    let ui = TestUi::new(true, true);
    let e = w
        .run(&ui, elevated, &["upgrade", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap_err();
    assert_eq!(e.to_string(), SCRIPT_REFUSAL);
    let plans = ui.plans.lock().unwrap().clone();
    assert_eq!(plans.len(), 1, "{plans:?}");
    assert!(plans[0].contains("Atualizar x 1.0 -> 2.0"), "{plans:?}");
    assert_eq!(ui.asked(), 0);
    assert_eq!(w.installed("x").as_deref(), Some("1.0"));
    assert_eq!(w.runs(), 0);
}

/// An upgrade runs the installed version's removal code now (`UpgradeUninstallHook`): refused,
/// while a fresh install of a manifest with that code is not.
#[tokio::test(flavor = "multi_thread")]
async fn upgrade_counts_the_old_versions_removal_code() {
    let mut w = World::new().await;
    let pre = serde_json::json!({ "pre_uninstall": ["Write-Host 'tchau'"] });
    w.publish("v1", &[("x", "1.0", pre)]).await;
    let ui = TestUi::new(true, true);
    w.run(
        &ui,
        not_elevated,
        &["install", "x", "--yes", "--allow-scripts"],
    )
    .await
    .unwrap();
    w.publish("v2", &[("x", "2.0", none())]).await;
    let e = w
        .run(&ui, elevated, &["upgrade", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "script pre_uninstall da versão instalada é código de terceiro e o wingetrs não o roda como administrador; abra um terminal sem administrador e repita"
    );
    assert_eq!(w.installed("x").as_deref(), Some("1.0"));
    assert_eq!(w.runs(), 0);
}
