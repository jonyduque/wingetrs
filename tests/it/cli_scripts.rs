#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(windows)]
//! Phase 2c tests (Task 161).
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use clap::{CommandFactory, Parser};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::bridge::BridgeJob;
use wingetrs::bridge::worker::ScopeArg;
use wingetrs::cache::DataDir;
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::cli::Cli;
use wingetrs::cli::install_cmd::{self, Flags, Interrupts, Stop, Ui};
use wingetrs::cli::winget_cmd::WingetFlows;
use wingetrs::config::Behavior;
use wingetrs::core::SourceKind;
use wingetrs::http::Http;
use wingetrs::install::executor::{ExecError, Runners, ScriptHost, ScriptJob, ScriptOutcome};
use wingetrs::install::ops::{self, Allow, OpsCtx, OpsError, ResidueScan};
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
use wingetrs::winget_install::uninstall::{UninstallPlan, UninstallReport, UninstallTarget};
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
    async fn install(&self, _: &WingetPlan) -> Result<InstallReport, WingetOpsError> {
        unreachable!("winget")
    }
    async fn install_bridge<I: Interrupts>(
        &self,
        _: &dyn Ui,
        _: &Candidate,
        _: Option<ScopeArg>,
        _: Flags,
        _: Stop<'_, I>,
    ) -> anyhow::Result<()> {
        unreachable!("winget")
    }
    fn installed(&self) -> anyhow::Result<Vec<ListRow>> {
        unreachable!("winget")
    }
    fn plan_uninstall(&self, _: &ListRow, _: &UninstallTarget) -> Result<UninstallPlan, String> {
        unreachable!("winget")
    }
    fn uninstall(&self, _: &UninstallPlan) -> Result<UninstallReport, String> {
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
    async fn upgrade(
        &self,
        _: &WingetPlan,
        _: Option<&UninstallPlan>,
    ) -> Result<InstallReport, UpgradeError> {
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
    fn remove_orphan(&self, _: &Orphan, _: &str) -> Result<PathBuf, String> {
        unreachable!("winget")
    }
}

// ---- world ----

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
                "scoop:main",
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
                ..Runners::disabled()
            },
        }
    }

    /// `wingetrs <args>` in process, with `ui`.
    async fn run(&self, ui: &TestUi, args: &[&str]) -> anyhow::Result<()> {
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
