#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(windows)]
//! Phase 2c tests (Task 161).
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use clap::{CommandFactory, Parser};
use crossterm::event::KeyCode::{Char, Enter, Esc};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::bridge::BridgeJob;
use wingetrs::bridge::worker::ScopeArg;
use wingetrs::cache::DataDir;
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::cli::Cli;
use wingetrs::cli::install_cmd::{self, Flags, Interrupts, Stop, Ui};
use wingetrs::cli::output::ThirdParty;
use wingetrs::cli::picker::fake::key;
use wingetrs::cli::winget_cmd::WingetFlows;
use wingetrs::config::Behavior;
use wingetrs::core::SourceKind;
use wingetrs::http::Http;
use wingetrs::install::executor::{ExecError, Runners, ScriptHost, ScriptJob, ScriptOutcome};
use wingetrs::install::ops::{self, Allow, OpsCtx, OpsError, ResidueScan};
use wingetrs::install::script_scan::CallRoots;
use wingetrs::install::target::Candidate;
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::installed::correlate::ListRow;
use wingetrs::tui::app::{Done, InstallFail};
use wingetrs::tui::picker::PickerItem;
use wingetrs::update::LockWait;
use wingetrs::winget_install::engine::UpgradeError;
use wingetrs::winget_install::ops::{InstallReport, WingetOpsError};
use wingetrs::winget_install::orphans::Orphan;
use wingetrs::winget_install::plan::{PlanOrBridge, WingetPlan};
use wingetrs::winget_install::select::{Refusal, ScopeWanted};
use wingetrs::winget_install::uninstall::{UninstallPlan, UninstallTarget};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const HEADER: &str = "Script de terceiro (post_install), roda com os seus privilégios e pode alterar qualquer arquivo seu";
const DOWNLOAD_FLAG: &str = "baixa arquivos da internet por conta própria (linha 2)";

// ---- fakes ----

/// Counts the scripts it is asked to run; runs nothing.
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

struct TestUi {
    interactive: bool,
    confirm: bool,
}

impl Ui for TestUi {
    fn interactive(&self) -> bool {
        self.interactive
    }
    fn confirm(&self, _: &str) -> anyhow::Result<bool> {
        Ok(self.confirm)
    }
    fn pick(&self, _: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
        Ok(None)
    }
    fn pick_many(&self, _: Vec<PickerItem>, _: &[bool]) -> anyhow::Result<Option<Vec<usize>>> {
        Ok(None)
    }
}

const NO_TTY: TestUi = TestUi {
    interactive: false,
    confirm: false,
};

/// Every app of these tests is a Scoop app of wingetrs: winget is never asked.
struct NoWinget;

impl WingetFlows for NoWinget {
    fn behavior(&self) -> Behavior {
        Behavior::default()
    }
    async fn plan_install(
        &self,
        _: &Candidate,
        _: ScopeWanted,
    ) -> anyhow::Result<Result<PlanOrBridge, Refusal>> {
        unreachable!("winget")
    }
    fn gate(&self) -> wingetrs::install::executor::LaunchGate {
        unreachable!("winget")
    }
    fn progress(&self) -> wingetrs::cli::winget_cmd::Sink {
        unreachable!("winget")
    }
    fn install(
        &self,
        _: &WingetPlan,
    ) -> wingetrs::cli::winget_cmd::Op<Result<InstallReport, WingetOpsError>> {
        unreachable!("winget")
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
        unreachable!("winget")
    }
    fn installed(&self) -> anyhow::Result<Vec<ListRow>> {
        unreachable!("winget")
    }
    async fn plan_uninstall(
        &self,
        _: &ListRow,
        _: &UninstallTarget,
    ) -> Result<UninstallPlan, String> {
        unreachable!("winget")
    }
    fn lock_install(
        &self,
    ) -> wingetrs::cli::winget_cmd::Op<Result<wingetrs::cli::winget_cmd::Held, WingetOpsError>>
    {
        unreachable!("winget")
    }
    fn uninstall(&self, _: &UninstallPlan) -> wingetrs::cli::winget_cmd::Uninstaller {
        unreachable!("winget")
    }
    fn remove_registry_only(
        &self,
        _: &UninstallPlan,
        _: &str,
    ) -> wingetrs::cli::winget_cmd::RecordRemover {
        unreachable!("winget")
    }
    fn free_space(&self) -> fn(&Path) -> Option<u64> {
        unreachable!("winget")
    }
    fn pins(&self) -> BTreeSet<String> {
        BTreeSet::new()
    }
    async fn plan_upgrade(
        &self,
        _: &ListRow,
        _: &str,
        _: ScopeWanted,
    ) -> anyhow::Result<(Result<PlanOrBridge, Refusal>, Option<UninstallPlan>)> {
        unreachable!("winget")
    }
    fn upgrade(
        &self,
        _: &WingetPlan,
        _: Option<&UninstallPlan>,
    ) -> wingetrs::cli::winget_cmd::Op<Result<InstallReport, UpgradeError>> {
        unreachable!("winget")
    }
    fn upgrade_bridge(&self, _: &BridgeJob, _: &mut dyn FnMut(&str)) -> Result<Done, InstallFail> {
        unreachable!("winget")
    }
    fn orphans(&self) -> anyhow::Result<Vec<Orphan>> {
        unreachable!("winget")
    }
    fn backups(&self) -> PathBuf {
        unreachable!("winget")
    }
    fn remove_orphan(
        &self,
        _: &Orphan,
        _: &str,
        _: &dyn Fn(&wingetrs::install::progress::ProgressEvent),
    ) -> Result<PathBuf, String> {
        unreachable!("winget")
    }
}

// ---- world ----

/// Installer files the fake installer runner was asked to run (only one test installs one).
static INSTALLER_RUNS: AtomicUsize = AtomicUsize::new(0);

fn count_installer(_: &Path, _: &[String], _: &Path) -> std::io::Result<(i32, Vec<String>)> {
    INSTALLER_RUNS.fetch_add(1, Ordering::SeqCst);
    Ok((0, Vec::new()))
}

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

/// A post_install with a gate flag on line 2.
fn post_install() -> serde_json::Value {
    serde_json::json!({
        "post_install": ["Write-Host 'ola'", "Invoke-WebRequest https://example.invalid/a -OutFile a"]
    })
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

    /// Puts `x <version>` in `scoop:main` (the manifest plus `extra`) and serves its zip.
    async fn publish(&mut self, version: &str, extra: serde_json::Value) {
        self.publish_in("scoop:main", version, extra).await;
    }

    /// Puts `x <version>` in `source` (the manifest plus `extra`) and serves its zip.
    async fn publish_in(&mut self, source: &str, version: &str, extra: serde_json::Value) {
        let zip = zip_of("pkg/x.exe", &fake_pe());
        let sha = hex::encode(Sha256::digest(&zip));
        let file = format!("x-{version}.zip");
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
            "bin": "x.exe",
        });
        for (k, v) in extra.as_object().unwrap() {
            m[k] = v.clone();
        }
        let pkg = NewPackage {
            id: "x".into(),
            name: "x".into(),
            version: version.into(),
            raw: Some(m.to_string()),
            ..Default::default()
        };
        self.catalog
            .replace_source(
                source,
                SourceKind::Scoop,
                version,
                1,
                &[pkg],
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
                installer: Some(count_installer),
                ..Runners::disabled()
            },
        }
    }

    /// `wingetrs <args>` in process, with `ui`.
    async fn run(&self, ui: &dyn Ui, args: &[&str]) -> anyhow::Result<()> {
        let cli =
            Cli::try_parse_from(std::iter::once("wingetrs").chain(args.iter().copied())).unwrap();
        let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        install_cmd::execute(&self.ctx(), ui, cli.command.unwrap(), &mut rx, &NoWinget).await
    }

    fn runs(&self) -> usize {
        self.host.runs.load(Ordering::SeqCst)
    }

    async fn requests(&self) -> usize {
        self.server.received_requests().await.unwrap().len()
    }

    fn installed(&self) -> Option<String> {
        let state = ops::read_state(&self.data.install_layout()).unwrap();
        state.get("x").unwrap().map(|(rec, _)| rec.version)
    }
}

const YES: TestUi = NO_TTY;

// ---- tests ----

#[tokio::test(flavor = "multi_thread")]
async fn yes_without_allow_scripts_errors() {
    let mut w = World::new().await;
    w.publish("1.0", post_install()).await;
    let e = w.run(&YES, &["install", "x", "--yes"]).await.unwrap_err();
    assert_eq!(
        e.to_string(),
        "x roda código de terceiros (script post_install); revise com --dry-run e rode de novo com --allow-scripts"
    );
    assert_eq!(w.requests().await, 0);
    assert_eq!(w.runs(), 0);
    assert_eq!(w.installed(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn allow_scripts_yes_runs() {
    let mut w = World::new().await;
    w.publish("1.0", post_install()).await;
    w.run(&YES, &["install", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap();
    assert_eq!(w.runs(), 1);
    assert_eq!(w.installed().as_deref(), Some("1.0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_tty_without_yes_errors() {
    let mut w = World::new().await;
    w.publish("1.0", post_install()).await;
    let e = w
        .run(&NO_TTY, &["install", "x", "--allow-scripts"])
        .await
        .unwrap_err();
    assert_eq!(
        e.to_string(),
        "a confirmação exige um terminal interativo; use --yes"
    );
    assert_eq!(w.runs(), 0);
    // A terminal that refuses: nothing runs either.
    let refuse = TestUi {
        interactive: true,
        confirm: false,
    };
    w.run(&refuse, &["install", "x", "--allow-scripts"])
        .await
        .unwrap();
    assert_eq!(w.runs(), 0);
    assert_eq!(w.installed(), None);
}

/// A real run of the binary (its stdout is the text the user reads), under the test
/// registry key: third-party code is never launched there.
async fn binary_lab(
    server: &MockServer,
    tag: &str,
    x: serde_json::Value,
) -> crate::cli_install::Lab {
    let lab = crate::cli_install::Lab::new(server, tag).await;
    let mut m = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/solo-1.0.zip", server.uri()),
        "hash": "0".repeat(64),
        "extract_dir": "pkg",
        "bin": "solo.exe",
    });
    for (k, v) in x.as_object().unwrap() {
        m[k] = v.clone();
    }
    let mut catalog = Catalog::open(&lab.root.join("catalog.db")).unwrap();
    let pkg = NewPackage {
        id: "s".into(),
        name: "s".into(),
        version: "1.0".into(),
        raw: Some(m.to_string()),
        ..Default::default()
    };
    catalog
        .replace_source(
            "scoop:main",
            SourceKind::Scoop,
            "v2",
            1,
            &[pkg],
            &WingetMaps::default(),
        )
        .unwrap();
    lab
}

fn stdout(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn confirmation_block_has_header_flags_numbered_script() {
    let server = MockServer::start().await;
    let lab = binary_lab(&server, "t161-block", post_install()).await;
    let o = lab.run(&["install", "s", "--allow-scripts"]);
    let out = stdout(&o);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success());
    assert!(
        err.contains("a confirmação exige um terminal interativo"),
        "{err}"
    );
    assert!(out.contains(HEADER), "{out}");
    assert!(out.contains(&format!("  ! {DOWNLOAD_FLAG}")), "{out}");
    assert!(out.contains("  1 | Write-Host 'ola'"), "{out}");
    assert!(out.contains("  2 | Invoke-WebRequest"), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_shows_block_without_running() {
    let server = MockServer::start().await;
    let lab = binary_lab(&server, "t161-dry", post_install()).await;
    let o = lab.run(&["install", "s", "--dry-run"]);
    let out = stdout(&o);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(out.contains(HEADER), "{out}");
    assert!(out.contains("  1 | Write-Host 'ola'"), "{out}");
    assert!(out.contains("Simulação: nada foi alterado."), "{out}");
    // In process: the dry run asks nothing, downloads nothing, runs nothing.
    let mut w = World::new().await;
    w.publish("1.0", post_install()).await;
    w.run(&NO_TTY, &["install", "x", "--dry-run"])
        .await
        .unwrap();
    assert_eq!(w.requests().await, 0);
    assert_eq!(w.runs(), 0);
    assert_eq!(w.installed(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn uninstall_with_scripts_needs_allow_scripts() {
    let mut w = World::new().await;
    w.publish(
        "1.0",
        serde_json::json!({ "pre_uninstall": "Write-Host 'tchau'" }),
    )
    .await;
    // Its own uninstall scripts already need the consent at install time.
    let e = w.run(&YES, &["install", "x", "--yes"]).await.unwrap_err();
    assert!(
        e.to_string().contains("(script pre_uninstall, na remoção)"),
        "{e}"
    );
    w.run(&YES, &["install", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap();
    assert_eq!(w.runs(), 0);
    let e = w.run(&YES, &["uninstall", "x", "--yes"]).await.unwrap_err();
    assert_eq!(
        e.to_string(),
        "x roda código de terceiros (script pre_uninstall); revise com --dry-run e rode de novo com --allow-scripts"
    );
    assert_eq!(w.installed().as_deref(), Some("1.0"));
    assert_eq!(w.runs(), 0);
    w.run(&NO_TTY, &["uninstall", "x", "--dry-run"])
        .await
        .unwrap();
    assert_eq!(w.installed().as_deref(), Some("1.0"));
    assert_eq!(w.runs(), 0);
    w.run(&YES, &["uninstall", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap();
    assert_eq!(w.runs(), 1);
    assert_eq!(w.installed(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn upgrade_needs_allow_scripts_when_old_or_new_has_scripts() {
    let mut w = World::new().await;
    w.publish(
        "1.0",
        serde_json::json!({ "pre_uninstall": "Write-Host 'tchau'" }),
    )
    .await;
    w.run(&YES, &["install", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap();
    // Only the OLD manifest has a script: it runs during the upgrade.
    w.publish("2.0", serde_json::json!({})).await;
    let e = w.run(&YES, &["upgrade", "x", "--yes"]).await.unwrap_err();
    assert_eq!(
        e.to_string(),
        "x roda código de terceiros (script pre_uninstall da versão instalada); revise com --dry-run e rode de novo com --allow-scripts"
    );
    assert_eq!(w.installed().as_deref(), Some("1.0"));
    assert_eq!(w.runs(), 0);
    w.run(&YES, &["upgrade", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap();
    assert_eq!(w.runs(), 1);
    assert_eq!(w.installed().as_deref(), Some("2.0"));
    // Only the NEW manifest has a script.
    w.publish("3.0", post_install()).await;
    let e = w.run(&YES, &["upgrade", "x", "--yes"]).await.unwrap_err();
    assert!(e.to_string().contains("(script post_install)"), "{e}");
    assert_eq!(w.installed().as_deref(), Some("2.0"));
}

/// An `uninstaller.file` (not PowerShell) runs at removal: installing it already needs consent.
#[tokio::test(flavor = "multi_thread")]
async fn uninstaller_file_needs_allow_scripts() {
    let mut w = World::new().await;
    w.publish(
        "1.0",
        serde_json::json!({ "uninstaller": { "file": "un.exe", "args": ["/S"] } }),
    )
    .await;
    let e = w.run(&YES, &["install", "x", "--yes"]).await.unwrap_err();
    assert_eq!(
        e.to_string(),
        "x roda código de terceiros (desinstalador un.exe); revise com --dry-run e rode de novo com --allow-scripts"
    );
    assert_eq!(w.requests().await, 0);
    assert_eq!(w.installed(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn plan_without_scripts_needs_no_flag() {
    let mut w = World::new().await;
    w.publish("1.0", serde_json::json!({})).await;
    w.run(&YES, &["install", "x", "--yes"]).await.unwrap();
    assert_eq!(w.installed().as_deref(), Some("1.0"));
    w.publish("2.0", serde_json::json!({})).await;
    w.run(&YES, &["upgrade", "x", "--yes"]).await.unwrap();
    assert_eq!(w.installed().as_deref(), Some("2.0"));
    w.run(&YES, &["uninstall", "x", "--yes"]).await.unwrap();
    assert_eq!(w.installed(), None);
    assert_eq!(w.runs(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn text_never_says_seguro() {
    let server = MockServer::start().await;
    let installer = serde_json::json!({
        "pre_install": "Write-Host 'a'",
        "post_install": ["Write-Host 'ola'", "Invoke-WebRequest https://example.invalid/a -OutFile a"],
        "installer": { "file": "setup.exe", "args": ["/S"] },
        "uninstaller": { "file": "un.ps1" },
        "pre_uninstall": "Write-Host 'b'",
    });
    let lab = binary_lab(&server, "t161-seguro", installer).await;
    let o = lab.run(&["install", "s", "--dry-run"]);
    let out = stdout(&o);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        out.contains("Instalador de terceiro: setup.exe /S"),
        "{out}"
    );
    assert!(out.contains("Desinstalador de terceiro: un.ps1"), "{out}");
    assert!(
        out.contains("instalador .ps1: conteúdo não analisado"),
        "{out}"
    );
    assert!(!out.to_lowercase().contains("segur"), "{out}");
    let o = lab.run(&["install", "s", "--yes"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("--allow-scripts"), "{err}");
    assert!(!err.to_lowercase().contains("segur"), "{err}");
}

#[test]
fn help_lists_allow_scripts() {
    for sub in ["install", "upgrade", "uninstall"] {
        let mut cmd = Cli::command();
        let help = cmd
            .find_subcommand_mut(sub)
            .unwrap()
            .render_help()
            .to_string();
        assert!(help.contains("--allow-scripts"), "{sub}: {help}");
        assert!(
            help.contains(
                "Permite rodar scripts e instaladores de terceiros do manifesto (mostrados antes)"
            ),
            "{sub}: {help}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn ops_masks_runners_without_consent() {
    let mut w = World::new().await;
    w.publish("1.0", post_install()).await;
    let ctx = w.ctx();
    let cand = match ops::resolve_target(&ctx, "x", None).unwrap() {
        ops::Resolution::One(c) => c,
        other => panic!("{other:?}"),
    };
    let plans = ops::plan_for(&ctx, &cand, false).unwrap();
    let r = ops::apply_with(&ctx, &plans, &Allow::default()).await;
    assert!(
        matches!(
            r,
            Err(OpsError::Exec(ExecError::NeedsScriptConsent { ref app, .. })) if app == "x"
        ),
        "{r:?}"
    );
    assert_eq!(w.runs(), 0);
    assert_eq!(w.installed(), None);
}

/// Task 151 review: the notes of a skipped uninstaller reach the user.
#[tokio::test(flavor = "multi_thread")]
async fn uninstall_shows_skipped_uninstaller_note() {
    let server = MockServer::start().await;
    let lab = crate::cli_install::Lab::new(&server, "t161-note").await;
    let o = lab.run(&["install", "solo", "--yes"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let manifest = wingetrs::install::layout::Layout::new(&lab.root)
        .version_dir("solo", "1.0")
        .join("manifest.json");
    std::fs::write(&manifest, b"{\"version\":\"1.0\",\"altered\":true}").unwrap();
    let o = lab.run(&["uninstall", "solo", "--yes"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "{err}");
    assert!(
        stdout(&o).contains("Desinstalado: solo 1.0"),
        "{}",
        stdout(&o)
    );
    assert!(
        err.contains("aviso: o desinstalador do app não rodou: manifesto gravado foi alterado"),
        "{err}"
    );
}

/// Review of Task 161: the installer runner is masked too (the only guard on the TUI path).
#[tokio::test(flavor = "multi_thread")]
async fn ops_masks_installer_runner_without_consent() {
    let mut w = World::new().await;
    w.publish(
        "1.0",
        serde_json::json!({ "installer": { "file": "x.exe", "args": ["/S"] } }),
    )
    .await;
    let ctx = w.ctx();
    let cand = match ops::resolve_target(&ctx, "x", None).unwrap() {
        ops::Resolution::One(c) => c,
        other => panic!("{other:?}"),
    };
    let plans = ops::plan_for(&ctx, &cand, false).unwrap();
    assert!(
        plans[0]
            .steps
            .iter()
            .any(|s| matches!(s, wingetrs::install::plan::Step::RunInstaller { .. }))
    );
    let r = ops::apply_with(&ctx, &plans, &Allow::default()).await;
    assert!(
        matches!(
            r,
            Err(OpsError::Exec(ExecError::NeedsScriptConsent { ref app, .. })) if app == "x"
        ),
        "{r:?}"
    );
    assert_eq!(INSTALLER_RUNS.load(Ordering::SeqCst), 0);
    assert_eq!(w.installed(), None);
}

/// Review of Task 161: escape sequences and hidden bidi/zero-width characters ("Trojan
/// Source") are shown, never interpreted.
#[test]
fn printable_shows_control_and_hidden_format_characters() {
    use wingetrs::install::script_scan::printable;
    assert_eq!(
        printable("a\u{202E}b\u{1b}[2Jc\td\u{200B}e\u{2066}f\u{FEFF}"),
        "a\\u{202e}b\\u{001b}[2Jc\td\\u{200b}e\\u{2066}f\\u{feff}"
    );
    assert_eq!(printable("Write-Host 'olá'"), "Write-Host 'olá'");
    let script = "Write-Host 'a'\u{202E} # }";
    let block = wingetrs::cli::output::third_party_block(&[ThirdParty::Script {
        hook: "post_install".into(),
        script: script.into(),
        report: wingetrs::install::script_scan::scan(script, &[]),
    }]);
    assert!(
        block.contains("  1 | Write-Host 'a'\\u{202e} # }"),
        "{block}"
    );
    assert!(!block.contains('\u{202E}'), "{block}");
}

/// Review of Task 161: installers are told apart by the step kind, not the file extension.
#[tokio::test(flavor = "multi_thread")]
async fn installer_kind_follows_the_step() {
    use wingetrs::install::plan::Step;
    let mut w = World::new().await;
    w.publish("1.0", serde_json::json!({})).await;
    let ctx = w.ctx();
    let cand = match ops::resolve_target(&ctx, "x", None).unwrap() {
        ops::Resolution::One(c) => c,
        other => panic!("{other:?}"),
    };
    let mut plan = ops::plan_for(&ctx, &cand, false).unwrap().remove(0);
    plan.steps.push(Step::RunInstaller {
        file: "a.ps1".into(),
        args: Vec::new(),
        keep: false,
    });
    plan.steps.push(Step::RunPs1Installer {
        file: "b.exe".into(),
        args: Vec::new(),
    });
    let items = wingetrs::cli::output::plan_third_party(&plan);
    assert!(
        matches!(
            items.as_slice(),
            [ThirdParty::Program { file: a, .. }, ThirdParty::Ps1 { file: b, .. }]
                if a == "a.ps1" && b == "b.exe"
        ),
        "{items:?}"
    );
}
// ---- Task 191 ----

fn resolved(extra: serde_json::Value) -> wingetrs::install::manifest::Resolved {
    let mut m = serde_json::json!({
        "version": "1.0",
        "url": "https://example.invalid/x-1.0.zip",
        "hash": "0".repeat(64),
    });
    for (k, v) in extra.as_object().unwrap() {
        m[k] = v.clone();
    }
    wingetrs::install::manifest::resolve(&m.to_string(), wingetrs::install::manifest::Arch::X64)
        .unwrap()
}

/// An `uninstaller` with only `args` runs the first download (as the executor does): the
/// consent lists it.
#[test]
fn removal_consent_lists_args_only_uninstaller() {
    let m = resolved(serde_json::json!({"uninstaller": {"args": "/S"}}));
    let vdir = Path::new(r"C:\w\apps\x\1.0");
    let items = wingetrs::cli::output::removal_third_party(&m, vdir, "", &CallRoots::default());
    assert_eq!(
        items,
        [ThirdParty::Program {
            uninstaller: true,
            file: "x-1.0.zip".into(),
            args: vec!["/S".into()],
        }]
    );
    assert_eq!(items[0].label(), "desinstalador x-1.0.zip");
    // `$dir` in the arguments is the version folder, as the executor passes it.
    let m = resolved(serde_json::json!({"uninstaller": {"file": "un.exe", "args": ["/D=$dir"]}}));
    let items = wingetrs::cli::output::removal_third_party(&m, vdir, "", &CallRoots::default());
    assert_eq!(
        items,
        [ThirdParty::Program {
            uninstaller: true,
            file: "un.exe".into(),
            args: vec![r"/D=C:\w\apps\x\1.0".into()],
        }]
    );
    // No uninstaller file and no args: nothing runs, nothing listed.
    let m = resolved(serde_json::json!({"uninstaller": {"script": "info 'x'"}}));
    let items = wingetrs::cli::output::removal_third_party(&m, vdir, "", &CallRoots::default());
    assert_eq!(
        items.iter().map(ThirdParty::label).collect::<Vec<_>>(),
        ["script uninstaller.script"]
    );
}

/// The executor runs `pre_uninstall`, the uninstaller file, `uninstaller.script`, then
/// `post_uninstall`: the consent lists them in that order.
#[test]
fn removal_consent_order_matches_executor() {
    let m = resolved(serde_json::json!({
        "pre_uninstall": "info 'pre'",
        "uninstaller": {"file": "un.exe", "script": "info 'unins'"},
        "post_uninstall": "info 'post'",
    }));
    let items = wingetrs::cli::output::removal_third_party(
        &m,
        Path::new(r"C:\v"),
        ", na remoção",
        &CallRoots::default(),
    );
    assert_eq!(
        items.iter().map(ThirdParty::label).collect::<Vec<_>>(),
        [
            "script pre_uninstall, na remoção",
            "desinstalador un.exe",
            "script uninstaller.script, na remoção",
            "script post_uninstall, na remoção",
        ]
    );
}

/// The consent covers the manifest the user read: a different hash under `install.lock`
/// runs nothing and removes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn uninstall_with_sha_mismatch_runs_nothing() {
    let mut w = World::new().await;
    w.publish(
        "1.0",
        serde_json::json!({ "pre_uninstall": "Write-Host 'tchau'" }),
    )
    .await;
    w.run(&YES, &["install", "x", "--yes", "--allow-scripts"])
        .await
        .unwrap();
    let ctx = w.ctx();
    let plan = ops::plan_uninstall(&ctx, "x", false).unwrap();
    let (manifest, shown) = ops::uninstall_manifest(&ctx, &plan).unwrap();
    assert!(manifest.is_some());
    assert_eq!(shown.len(), 64, "{shown}");
    let e = ops::uninstall_with(&ctx, &plan, Some(&"0".repeat(64))).unwrap_err();
    assert!(
        matches!(e, OpsError::ManifestChanged(ref a) if a == "x"),
        "{e}"
    );
    assert_eq!(
        e.to_string(),
        "x: o manifesto mudou desde que você revisou os scripts; nada foi removido, peça a remoção de novo"
    );
    assert_eq!(w.runs(), 0);
    assert_eq!(w.installed().as_deref(), Some("1.0"));
    // The hash shown: removed, its script ran once.
    ops::uninstall_with(&ctx, &plan, Some(&shown)).unwrap();
    assert_eq!(w.runs(), 1);
    assert_eq!(w.installed(), None);
}

/// `--dry-run` of an MSI app (like `main/rust`) says the `msiexec /a` fallback needs
/// `--allow-msiexec`, and the manifest step is capitalized like the others.
#[tokio::test(flavor = "multi_thread")]
async fn dry_run_msi_shows_allow_msiexec_hint_and_capitalized_steps() {
    let server = MockServer::start().await;
    let url = format!("{}/solo-1.0.msi", server.uri());
    let x = serde_json::json!({"url": url, "extract_dir": "SourceDir"});
    let lab = binary_lab(&server, "t191-msi", x).await;
    let o = lab.run(&["install", "s", "--dry-run"]);
    let out = stdout(&o);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        out.contains("Se o extrator não suportar este MSI, pede --allow-msiexec"),
        "{out}"
    );
    assert!(out.contains(". Gravar "), "{out}");
    assert!(!out.contains(". gravar "), "{out}");
    let o = lab.run(&["install", "s", "--dry-run", "--allow-msiexec"]);
    let out = stdout(&o);
    assert!(
        out.contains("usa o Windows Installer (msiexec /a)"),
        "{out}"
    );
    assert!(!out.contains("pede --allow-msiexec"), "{out}");
}
// ---- Task 192 ----

/// A root whose `extras` bucket stores `scripts\ok.ps1` with the Task 149 marker.
fn root_with_ok_script() -> TempDir {
    let t = tempfile::tempdir().unwrap();
    let bucket = t.path().join("buckets").join("extras");
    std::fs::create_dir_all(bucket.join("scripts")).unwrap();
    std::fs::write(
        bucket.join("scripts").join("ok.ps1"),
        "Write-Host 'chamado'\r\n",
    )
    .unwrap();
    std::fs::write(bucket.join(".scripts-ok"), b"").unwrap();
    t
}

/// The consent (CLI and TUI build it with `plan_third_party`) shows the file a hook runs.
#[tokio::test(flavor = "multi_thread")]
async fn consent_shows_called_bucket_script() {
    use wingetrs::install::manifest::HookKind;
    use wingetrs::install::plan::Step;
    let mut w = World::new().await;
    w.publish("1.0", serde_json::json!({})).await;
    let ctx = w.ctx();
    let cand = match ops::resolve_target(&ctx, "x", None).unwrap() {
        ops::Resolution::One(c) => c,
        other => panic!("{other:?}"),
    };
    let mut plan = ops::plan_for(&ctx, &cand, false).unwrap().remove(0);
    let t = root_with_ok_script();
    plan.root = t.path().to_path_buf();
    plan.steps.push(Step::Hook {
        kind: HookKind::PostInstall,
        script: ". \"$bucketsdir\\extras\\scripts\\ok.ps1\"".into(),
    });
    let items = wingetrs::cli::output::plan_third_party(&plan);
    let called = items.iter().find_map(|i| match i {
        ThirdParty::Script { hook, script, .. }
            if hook == "arquivo chamado por post_install: $bucketsdir\\extras\\scripts\\ok.ps1" =>
        {
            Some(script.clone())
        }
        _ => None,
    });
    assert_eq!(
        called.as_deref(),
        Some("Write-Host 'chamado'\r\n"),
        "{items:?}"
    );
    let block = wingetrs::cli::output::third_party_block(&items);
    assert!(block.contains("Write-Host 'chamado'"), "{block}");
    assert!(block.contains("analisado e mostrado abaixo"), "{block}");
}

/// The removal consent reads the installed `$dir`.
#[test]
fn removal_consent_shows_called_dir_script() {
    let t = tempfile::tempdir().unwrap();
    let vdir = t.path().join("apps").join("x").join("1.0");
    std::fs::create_dir_all(&vdir).unwrap();
    std::fs::write(vdir.join("un.ps1"), "Write-Host 'tchau'\r\n").unwrap();
    let m = resolved(serde_json::json!({"pre_uninstall": "& \"$dir\\un.ps1\""}));
    let roots = CallRoots::new(t.path(), Some(&vdir));
    let items = wingetrs::cli::output::removal_third_party(&m, &vdir, "", &roots);
    let hooks: Vec<&str> = items
        .iter()
        .filter_map(|i| match i {
            ThirdParty::Script { hook, .. } => Some(hook.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        hooks,
        [
            "pre_uninstall",
            "arquivo chamado por pre_uninstall: $dir\\un.ps1"
        ]
    );
}
// ---- every question is the inline selector (Task 200) ----

/// The real CLI `Ui` on a scripted terminal: `keys` answer, the drawing is kept.
fn scripted_ui(
    keys: Vec<crossterm::event::Event>,
    interactive: bool,
) -> (
    install_cmd::TtyUi,
    Arc<std::sync::Mutex<wingetrs::cli::picker::fake::Screen>>,
) {
    let mut term = wingetrs::cli::picker::fake::ScriptedTerm::new(keys);
    term.interactive = interactive;
    let screen = term.screen();
    let cfg = wingetrs::config::Config {
        glyphs: "ascii".into(),
        ..Default::default()
    };
    (install_cmd::TtyUi::with_term(&cfg, Box::new(term)), screen)
}

#[tokio::test(flavor = "multi_thread")]
async fn no_tty_never_prompts() {
    let mut w = World::new().await;
    w.publish("1.0", serde_json::json!({})).await;
    let (ui, screen) = scripted_ui(vec![key(Char('s'))], false);
    let e = w.run(&ui, &["install", "x"]).await.unwrap_err();
    assert_eq!(
        e.to_string(),
        "a confirmação exige um terminal interativo; use --yes"
    );
    {
        let s = screen.lock().unwrap();
        assert_eq!((s.raw_enabled, s.reads, s.out.len()), (0, 0, 0));
    }
    assert_eq!(w.installed(), None);
    // `--yes` answers the confirmation without drawing anything.
    w.run(&ui, &["install", "x", "--yes"]).await.unwrap();
    assert_eq!(w.installed().as_deref(), Some("1.0"));
    let s = screen.lock().unwrap();
    assert_eq!((s.raw_enabled, s.reads, s.out.len()), (0, 0, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_question_uses_selector() {
    let mut w = World::new().await;
    w.publish("1.0", serde_json::json!({})).await;
    // Enter on the default answer ("Não"): nothing happens.
    let (ui, screen) = scripted_ui(vec![key(Enter)], true);
    w.run(&ui, &["install", "x"]).await.unwrap();
    assert_eq!(w.installed(), None);
    let text = wingetrs::cli::picker::fake::plain(&screen.lock().unwrap().out);
    assert!(
        text.starts_with("Continuar?  [ Sim ]  [ Não ]\r\n"),
        "{text:?}"
    );
    assert!(text.ends_with("Continuar? Não\r\n"), "{text:?}");
    let (ui, screen) = scripted_ui(vec![key(Char('s'))], true);
    w.run(&ui, &["install", "x"]).await.unwrap();
    assert_eq!(w.installed().as_deref(), Some("1.0"));
    assert!(!screen.lock().unwrap().raw);
}

#[tokio::test(flavor = "multi_thread")]
async fn ambiguous_target_uses_selector() {
    let mut w = World::new().await;
    w.publish_in("scoop:main", "1.0", serde_json::json!({}))
        .await;
    w.publish_in("scoop:extras", "2.0", serde_json::json!({}))
        .await;
    // `m` moves to `main/x`, Enter confirms it, then `s` answers "Continuar?".
    let (ui, screen) = scripted_ui(vec![key(Char('m')), key(Enter), key(Char('s'))], true);
    w.run(&ui, &["install", "x"]).await.unwrap();
    assert_eq!(w.installed().as_deref(), Some("1.0"));
    let text = wingetrs::cli::picker::fake::plain(&screen.lock().unwrap().out);
    assert!(text.contains("extras/x  2.0\r\n"), "{text:?}");
    assert!(text.contains("> main/x\r\n"), "{text:?}");
    // Esc cancels the pick: nothing more is asked.
    let mut w = World::new().await;
    w.publish_in("scoop:main", "1.0", serde_json::json!({}))
        .await;
    w.publish_in("scoop:extras", "2.0", serde_json::json!({}))
        .await;
    let (ui, screen) = scripted_ui(vec![key(Esc)], true);
    w.run(&ui, &["install", "x"]).await.unwrap();
    assert_eq!(w.installed(), None);
    assert_eq!(screen.lock().unwrap().reads, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn target_letter_only_moves_the_cursor() {
    let mut w = World::new().await;
    w.publish_in("scoop:main", "1.0", serde_json::json!({}))
        .await;
    w.publish_in("scoop:extras", "2.0", serde_json::json!({}))
        .await;
    // `m` moves to `main/x` but does not confirm; Esc then cancels.
    let (ui, screen) = scripted_ui(vec![key(Char('m')), key(Esc)], true);
    w.run(&ui, &["install", "x"]).await.unwrap();
    assert_eq!(w.installed(), None);
    let s = screen.lock().unwrap();
    assert_eq!(s.reads, 2);
    let text = wingetrs::cli::picker::fake::plain(&s.out);
    assert!(text.contains("> m) main/x  1.0\r\n"), "{text:?}");
    assert!(text.contains("setas ou letra movem"), "{text:?}");
}

#[test]
fn bucket_add_question_uses_selector() {
    let has = |cfg: &wingetrs::config::Config| {
        wingetrs::buckets::resolved(cfg)
            .unwrap()
            .iter()
            .any(|(n, _)| n == "meu")
    };
    let add = |ui: &install_cmd::TtyUi, cfg: &mut wingetrs::config::Config, yes: bool| {
        let mut said = Vec::new();
        let r = wingetrs::cli::bucket_add(
            ui,
            cfg,
            "meu",
            Some("dono/repo".into()),
            yes,
            &mut |l: &str| said.push(l.to_string()),
        );
        (r, said)
    };
    // No terminal and no --yes: refused, naming the flag, nothing drawn.
    let mut cfg = wingetrs::config::Config::default();
    let (ui, screen) = scripted_ui(vec![key(Char('s'))], false);
    let e = add(&ui, &mut cfg, false).0.unwrap_err().to_string();
    assert!(e.contains("--yes"), "{e}");
    assert!(!has(&cfg));
    {
        let s = screen.lock().unwrap();
        assert_eq!((s.raw_enabled, s.reads, s.out.len()), (0, 0, 0));
    }
    // Enter keeps the default "Não".
    let (ui, screen) = scripted_ui(vec![key(Enter)], true);
    let (r, said) = add(&ui, &mut cfg, false);
    assert!(!r.unwrap());
    assert_eq!(said, vec!["⚠ Cancelado.".to_string()]);
    assert!(!has(&cfg));
    let text = wingetrs::cli::picker::fake::plain(&screen.lock().unwrap().out);
    assert!(
        text.starts_with(
            "O bucket meu (dono/repo) não é conhecido. Adicionar?  [ Sim ]  [ Não ]\r\n"
        ),
        "{text:?}"
    );
    // `s` adds it.
    let (ui, _) = scripted_ui(vec![key(Char('s'))], true);
    assert!(add(&ui, &mut cfg, false).0.unwrap());
    assert!(has(&cfg));
    // `--yes` adds without asking.
    let mut cfg = wingetrs::config::Config::default();
    let (ui, screen) = scripted_ui(vec![], false);
    assert!(add(&ui, &mut cfg, true).0.unwrap());
    assert!(has(&cfg));
    assert_eq!(screen.lock().unwrap().reads, 0);
}
