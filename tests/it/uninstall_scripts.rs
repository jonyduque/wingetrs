#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 159).
//!
//! Removal and upgrade run the manifest's uninstall hooks (`pre_uninstall`, the uninstaller
//! file, `uninstaller.script`, `post_uninstall`) from the manifest written at install time.
//! The script host and the uninstaller runner are fakes: nothing here runs PowerShell.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::executor::{
    ExecCtx, ExecError, NoHook, Runners, ScriptHost, ScriptJob, ScriptOutcome, ToolRun,
    execute_install, execute_uninstall,
};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, HookKind, resolve};
use wingetrs::install::plan::{InstallPlan, Step, UninstallPlan};
use wingetrs::install::planner::{InstalledInfo, PlanError, PlanInput, plan_install};
use wingetrs::install::runner::ps_single_quoted;
use wingetrs::install::state::{StateDb, installed_manifest};
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{Effects, EnvStore, EnvValue, Shortcuts};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Minimal PE-looking buffer: MZ header with e_lfanew = 0x80, "PE\0\0", optional header subsystem at +24+68.
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

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            z.start_file(*name, o).unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn fixture_zip() -> Vec<u8> {
    zip_of(&[
        ("pkg/tool.exe", &fake_pe(3)),
        ("pkg/setup.exe", &fake_pe(3)),
        ("pkg/unins.ps1", b"Write-Host 'bye'\r\n"),
        ("pkg/conf/default.ini", b"[default]\r\n"),
    ])
}

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn no_progress(_: &wingetrs::install::progress::ProgressEvent) {}
fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

const ORIGINAL_PATH: &str = r"C:\Existing";
const EVENTS: &str = "events.log";

/// Appends one line to `<tempdir>\events.log` (outside the snapshotted root).
fn log_event(base: &Path, what: &str) {
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(base.join(EVENTS))
        .unwrap();
    writeln!(f, "{what}").unwrap();
}

/// `uninstaller.file` runner: `dir` is `<tempdir>\root\apps\<app>\<version>`.
fn fake_uninstaller(
    exe: &Path,
    _args: &[String],
    dir: &Path,
) -> std::io::Result<(i32, Vec<String>)> {
    let name = exe.file_name().unwrap().to_string_lossy().into_owned();
    log_event(dir.ancestors().nth(4).unwrap(), &format!("run:{name}"));
    Ok((0, Vec::new()))
}

/// What the fake host saw for one job.
#[derive(Clone)]
struct Seen {
    job: ScriptJob,
    /// The runner text without its BOM.
    runner: String,
    /// `manifest.json` of the work folder (`$manifest`).
    manifest: String,
    /// Whether each `watch` path existed when the hook ran.
    watch: Vec<bool>,
    /// The `probe_env` variables as the hook found them.
    env: Vec<(String, Option<String>)>,
    /// Whether each `probe_files` path existed when the hook ran.
    files: Vec<bool>,
}

#[derive(Default)]
struct FakeHost {
    seen: Mutex<Vec<Seen>>,
    /// Exit code of every hook.
    code: i32,
    /// Only this hook fails (exit 1); the others exit 0. Overrides `code`.
    fail_on: Option<HookKind>,
    events: Option<PathBuf>,
    watch: Vec<PathBuf>,
    /// `effects.jsonl` text written by the given hook.
    effects: Vec<(HookKind, String)>,
    probe_env: Option<(Arc<FakeEnv>, Vec<String>)>,
    probe_files: Vec<PathBuf>,
}

impl FakeHost {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
    fn seen_of(&self, kind: HookKind) -> Seen {
        self.seen()
            .into_iter()
            .find(|s| s.job.hook == kind)
            .expect("hook ran")
    }
    fn kinds(&self) -> Vec<HookKind> {
        self.seen().iter().map(|s| s.job.hook).collect()
    }
}

impl ScriptHost for FakeHost {
    fn run(&self, job: &ScriptJob) -> std::io::Result<ScriptOutcome> {
        let runner = std::fs::read(&job.runner)?;
        let env = match &self.probe_env {
            Some((env, names)) => names
                .iter()
                .map(|n| (n.clone(), env.get(n).unwrap().map(|v| v.value)))
                .collect(),
            None => Vec::new(),
        };
        self.seen.lock().unwrap().push(Seen {
            job: job.clone(),
            runner: String::from_utf8(runner[3..].to_vec()).unwrap(),
            manifest: std::fs::read_to_string(job.workdir.join("manifest.json"))?,
            watch: self.watch.iter().map(|p| p.exists()).collect(),
            env,
            files: self.probe_files.iter().map(|p| p.exists()).collect(),
        });
        if let Some(base) = &self.events {
            log_event(base, &format!("{:?}", job.hook));
        }
        for (kind, text) in &self.effects {
            if *kind == job.hook {
                std::fs::write(job.workdir.join("effects.jsonl"), text)?;
            }
        }
        let code = match self.fail_on {
            Some(k) if k == job.hook => 1,
            Some(_) => 0,
            None => self.code,
        };
        Ok(ScriptOutcome {
            code,
            lines: vec!["linha1".into()],
        })
    }
}

struct World {
    _server: MockServer,
    t: TempDir,
    layout: Layout,
    data: DataDir,
    env: Arc<FakeEnv>,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    state: StateDb,
    state_path: PathBuf,
    http: Http,
    url: String,
    sha: String,
}

impl World {
    fn base(&self) -> PathBuf {
        self.t.path().to_path_buf()
    }

    fn manifest(&self, version: &str, extra: serde_json::Value) -> String {
        let mut m = serde_json::json!({
            "version": version,
            "url": self.url,
            "hash": self.sha,
            "extract_dir": "pkg",
            "bin": "tool.exe",
            "persist": ["conf"],
            "env_set": {"TOOL_HOME": "$dir"},
            "env_add_path": ["."],
        });
        for (k, v) in extra.as_object().unwrap() {
            m[k] = v.clone();
        }
        m.to_string()
    }

    /// Plans `manifest` as the installer would: the hash is the real one and, for an update, the
    /// old manifest comes from the written file (never from a catalog).
    fn try_plan(&self, manifest: &str) -> Result<InstallPlan, PlanError> {
        let m = resolve(manifest, Arch::X64).unwrap();
        let installed = self.state.get("app").unwrap().map(|(r, _)| InstalledInfo {
            old_manifest: installed_manifest(&self.layout, "app", &r.version, &r.manifest_sha256)
                .unwrap()
                .map(|raw| resolve(&raw, Arch::X64).unwrap()),
            version: r.version,
        });
        let root = self.shortcuts.root();
        plan_install(&PlanInput {
            app: "app",
            bucket: "main",
            manifest: &m,
            layout: &self.layout,
            shortcut_root: &root,
            installed,
            launcher_present: true,
            keep_old: false,
            manifest_sha256: &sha256_hex(manifest.as_bytes()),
        })
    }

    fn plan(&self, manifest: &str) -> InstallPlan {
        self.try_plan(manifest).unwrap()
    }

    fn ctx(&self, scripts: Option<Arc<dyn ScriptHost>>, installer: Option<ToolRun>) -> ExecCtx<'_> {
        let state: &mut StateDb = Box::leak(Box::new(StateDb::open(&self.state_path).unwrap()));
        ExecCtx {
            layout: &self.layout,
            data: &self.data,
            http: &self.http,
            effects: Effects {
                env: &*self.env,
                shortcuts: &self.shortcuts,
                trash: &self.trash,
                registry: &self.registry,
            },
            state,
            hook: &NoHook,
            progress: &no_progress,
            in_use: &nothing_in_use,
            msiexec: None,
            launch: Default::default(),
            runners: Runners {
                installer,
                scripts,
                ..Runners::disabled()
            },
        }
    }

    /// Installs `manifest` with a host that is never asked to fail.
    async fn install(&self, manifest: &str) {
        let host: Arc<dyn ScriptHost> = Arc::new(FakeHost::default());
        let mut ctx = self.ctx(Some(host), Some(fake_uninstaller));
        execute_install(&self.plan(manifest), &mut ctx)
            .await
            .unwrap();
    }

    async fn upgrade(&self, manifest: &str, host: &Arc<FakeHost>) -> Result<(), ExecError> {
        let scripts: Arc<dyn ScriptHost> = host.clone();
        let mut ctx = self.ctx(Some(scripts), Some(fake_uninstaller));
        execute_install(&self.plan(manifest), &mut ctx).await
    }

    fn uninstall_plan(&self) -> UninstallPlan {
        UninstallPlan::from_state(&self.state, "app", false)
            .unwrap()
            .unwrap()
    }

    fn uninstall(
        &self,
        host: Option<&Arc<FakeHost>>,
        installer: Option<ToolRun>,
    ) -> Result<Vec<String>, ExecError> {
        let scripts = host.cloned().map(|h| -> Arc<dyn ScriptHost> { h });
        let mut ctx = self.ctx(scripts, installer);
        execute_uninstall(&self.uninstall_plan(), &mut ctx)
    }

    fn events(&self) -> Vec<String> {
        std::fs::read_to_string(self.base().join(EVENTS))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn written_manifest(&self, version: &str) -> PathBuf {
        self.layout
            .version_dir("app", version)
            .join("manifest.json")
    }

    fn intact(&self) -> bool {
        self.layout.current("app").join("tool.exe").exists()
            && self.state.get("app").unwrap().is_some()
    }
}

async fn setup() -> World {
    setup_at("root").await
}

/// [`setup`] with the wingetrs root at `<tempdir>\<root>`.
async fn setup_at(root: &str) -> World {
    let server = MockServer::start().await;
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join(root);
    let layout = Layout::new(&root);
    std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
    std::fs::write(layout.launcher(), fake_pe(3)).unwrap();
    let data = DataDir::new(&root);
    let env = Arc::new(FakeEnv::new());
    env.set(
        "Path",
        &EnvValue {
            value: ORIGINAL_PATH.into(),
            expand: true,
        },
    )
    .unwrap();
    let shortcuts = FakeShortcuts::new(t.path().join("startmenu"));
    let state_path = t.path().join("state").join("state.db");
    let state = StateDb::open(&state_path).unwrap();
    let zip = fixture_zip();
    let sha = sha256_hex(&zip);
    Mock::given(method("GET"))
        .and(path("/pkg.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
        .mount(&server)
        .await;
    let url = format!("{}/pkg.zip", server.uri());
    World {
        _server: server,
        t,
        layout,
        data,
        env,
        shortcuts,
        trash: FakeTrash::new(),
        registry: FakeRegistry::new(),
        state,
        state_path,
        http: Http::new().unwrap(),
        url,
        sha,
    }
}

/// `pre_uninstall`, the uninstaller file, `uninstaller.script` and `post_uninstall`.
fn uninstall_hooks() -> serde_json::Value {
    serde_json::json!({
        "pre_uninstall": "info 'pre'",
        "uninstaller": {"file": "setup.exe", "script": "info 'unins'"},
        "post_uninstall": "info 'post'",
    })
}

fn host_with_events(w: &World) -> Arc<FakeHost> {
    Arc::new(FakeHost {
        events: Some(w.base()),
        ..FakeHost::default()
    })
}

fn quoted(p: &Path) -> String {
    ps_single_quoted(&p.to_string_lossy())
}

#[tokio::test]
async fn uninstall_order_and_cmd() {
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let host = Arc::new(FakeHost {
        events: Some(w.base()),
        watch: vec![w.layout.app_dir("app")],
        ..FakeHost::default()
    });
    w.uninstall(Some(&host), Some(fake_uninstaller)).unwrap();
    assert_eq!(
        w.events(),
        [
            "PreUninstall",
            "run:setup.exe",
            "UninstallerScript",
            "PostUninstall"
        ]
    );
    assert!(!w.layout.app_dir("app").exists());
    assert!(w.state.get("app").unwrap().is_none());
    // The folder exists for the first three and is gone for `post_uninstall`.
    let watch: Vec<bool> = host.seen().iter().map(|s| s.watch[0]).collect();
    assert_eq!(watch, [true, true, false]);
    let vdir = w.layout.version_dir("app", "1.0");
    for kind in [
        HookKind::PreUninstall,
        HookKind::UninstallerScript,
        HookKind::PostUninstall,
    ] {
        let r = host.seen_of(kind).runner;
        assert!(r.contains("$cmd = 'uninstall'\n"), "{kind:?}: {r}");
        assert!(r.contains(&format!("$dir = {}\n", quoted(&vdir))), "{r}");
        assert!(r.contains("$version = '1.0'\n"), "{r}");
        assert!(r.contains("$purge = $false\n"), "{r}");
    }
    let r = host.seen_of(HookKind::UninstallerScript).runner;
    assert!(r.contains("$Uninstall = $true\n"), "{r}");
}

#[tokio::test]
async fn uninstall_uses_written_manifest_not_catalog() {
    let w = setup().await;
    let written = w.manifest("1.0", uninstall_hooks());
    w.install(&written).await;
    // What the catalog serves now: same version, no hooks. The executor never reads it.
    let catalog = w.manifest("1.0", serde_json::json!({}));
    let host = host_with_events(&w);
    w.uninstall(Some(&host), Some(fake_uninstaller)).unwrap();
    assert_eq!(
        host.kinds(),
        [
            HookKind::PreUninstall,
            HookKind::UninstallerScript,
            HookKind::PostUninstall
        ]
    );
    for s in host.seen() {
        assert_eq!(s.manifest, written, "$manifest is the written one");
        assert_ne!(s.manifest, catalog);
    }
}

#[tokio::test]
async fn uninstall_manifest_sha_mismatch_removes_without_scripts() {
    let w = setup().await;
    let written = w.manifest("1.0", uninstall_hooks());
    w.install(&written).await;
    std::fs::write(
        w.written_manifest("1.0"),
        written.replace("info 'pre'", "info 'evil'"),
    )
    .unwrap();
    let host = host_with_events(&w);
    let notes = w.uninstall(Some(&host), Some(fake_uninstaller)).unwrap();
    assert!(host.seen().is_empty(), "no hook ran");
    assert!(w.events().is_empty(), "no uninstaller ran");
    assert!(!w.layout.app_dir("app").exists(), "the app was removed");
    assert!(w.state.get("app").unwrap().is_none());
    assert!(!w.layout.shims().join("tool.shim").exists());
    assert_eq!(
        notes,
        [
            "o desinstalador do app não rodou: manifesto gravado foi alterado; o que ele instalou fora da pasta pode ter ficado"
        ]
    );
}

#[tokio::test]
async fn pre_uninstall_failure_keeps_everything() {
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let host = Arc::new(FakeHost {
        events: Some(w.base()),
        fail_on: Some(HookKind::PreUninstall),
        ..FakeHost::default()
    });
    let e = w
        .uninstall(Some(&host), Some(fake_uninstaller))
        .unwrap_err();
    assert!(e.to_string().contains("pre_uninstall"), "{e}");
    assert_eq!(w.events(), ["PreUninstall"], "nothing after the failure");
    assert!(w.intact());
    assert!(w.layout.shims().join("tool.shim").exists());
    assert!(w.layout.version_dir("app", "1.0").join("conf").exists());
}

#[tokio::test]
async fn post_uninstall_failure_reports_after_removal() {
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let host = Arc::new(FakeHost {
        events: Some(w.base()),
        fail_on: Some(HookKind::PostUninstall),
        ..FakeHost::default()
    });
    let e = w
        .uninstall(Some(&host), Some(fake_uninstaller))
        .unwrap_err();
    assert!(matches!(e, ExecError::Hook(_)), "{e:?}");
    assert!(
        e.to_string()
            .contains("removido, mas o post_uninstall falhou"),
        "{e}"
    );
    assert!(!w.layout.app_dir("app").exists());
    assert!(w.state.get("app").unwrap().is_none(), "record removed");
}

#[tokio::test]
async fn uninstall_without_consent_refused_before_anything() {
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let e = w.uninstall(None, Some(fake_uninstaller)).unwrap_err();
    assert!(matches!(e, ExecError::NeedsScriptConsent { .. }), "{e:?}");
    assert!(w.events().is_empty(), "the fake uninstaller was not called");
    assert!(w.intact());
    assert!(w.layout.shims().join("tool.shim").exists());
}

#[tokio::test]
async fn uninstall_file_alone_without_runner_consent_refused() {
    // Hooks consented but not the uninstaller file: still refused before the first hook.
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let host = host_with_events(&w);
    let e = w.uninstall(Some(&host), None).unwrap_err();
    assert!(matches!(e, ExecError::NeedsScriptConsent { .. }), "{e:?}");
    assert!(host.seen().is_empty());
    assert!(w.intact());
}

#[tokio::test]
async fn upgrade_old_hooks_see_new_version_old_manifest() {
    let w = setup().await;
    let old = w.manifest("1.0", uninstall_hooks());
    w.install(&old).await;
    let new = w.manifest("2.0", serde_json::json!({"pre_install": "info 'new'"}));
    let v2 = w.layout.version_dir("app", "2.0");
    let host = Arc::new(FakeHost {
        events: Some(w.base()),
        watch: vec![v2.clone()],
        ..FakeHost::default()
    });
    let plan = w.plan(&new);
    let pos = |f: fn(&Step) -> bool| plan.steps.iter().position(f).unwrap();
    let promote = pos(|s| matches!(s, Step::Promote));
    assert!(pos(|s| matches!(s, Step::UpgradeUninstallHook { .. })) < promote);
    assert!(pos(|s| matches!(s, Step::UpgradeRunUninstaller { .. })) < promote);
    w.upgrade(&new, &host).await.unwrap();
    assert_eq!(
        w.events(),
        [
            "PreUninstall",
            "run:setup.exe",
            "UninstallerScript",
            "PostUninstall",
            "PreInstall"
        ]
    );
    let vdir = w.layout.version_dir("app", "1.0");
    let pre = host.seen_of(HookKind::PreUninstall);
    for needle in [
        "$cmd = 'update'\n".to_string(),
        "$version = '2.0'\n".to_string(),
        "$old_version = '1.0'\n".to_string(),
        format!("$dir = {}\n", quoted(&vdir)),
    ] {
        assert!(pre.runner.contains(&needle), "{needle}: {}", pre.runner);
    }
    assert_eq!(pre.manifest, old, "$manifest is the OLD written manifest");
    let post = host.seen_of(HookKind::PostUninstall);
    assert_eq!(post.manifest, old);
    assert!(
        post.runner.contains("$version = '2.0'\n"),
        "{}",
        post.runner
    );
    // The new version's own hook keeps `$cmd = 'update'` and its own manifest.
    let pre_install = host.seen_of(HookKind::PreInstall);
    assert!(pre_install.runner.contains("$cmd = 'update'\n"));
    assert!(pre_install.runner.contains("$version = '2.0'\n"));
    assert_eq!(pre_install.manifest, new);
    // The old hooks ran before `Promote` (no 2.0 folder yet); the new one after.
    let watch: Vec<bool> = host.seen().iter().map(|s| s.watch[0]).collect();
    assert_eq!(watch, [false, false, false, true]);
    assert!(w.layout.current("app").join("tool.exe").exists());
}

#[tokio::test]
async fn upgrade_failure_after_old_hooks_mentions_warning() {
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let new = w.manifest("2.0", serde_json::json!({"pre_install": "info 'new'"}));
    let host = Arc::new(FakeHost {
        events: Some(w.base()),
        fail_on: Some(HookKind::PreInstall),
        ..FakeHost::default()
    });
    let e = w.upgrade(&new, &host).await.unwrap_err();
    let ExecError::Failed { message, .. } = &e else {
        panic!("{e:?}");
    };
    assert!(message.contains("pre_install"), "{message}");
    assert!(
        message.ends_with(
            "; os scripts de desinstalação da versão antiga já rodaram e não foram desfeitos; uma remoção posterior vai rodar o desinstalador antigo de novo"
        ),
        "{message}"
    );
    // The upgrade itself was rolled back.
    assert!(w.layout.version_dir("app", "1.0").exists());
    assert!(!w.layout.version_dir("app", "2.0").exists());
    assert_eq!(w.state.get("app").unwrap().unwrap().0.version, "1.0");
}

#[tokio::test]
async fn upgrade_failure_without_old_hooks_has_no_warning() {
    let w = setup().await;
    w.install(&w.manifest("1.0", serde_json::json!({}))).await;
    let new = w.manifest("2.0", serde_json::json!({"pre_install": "info 'new'"}));
    let host = Arc::new(FakeHost {
        fail_on: Some(HookKind::PreInstall),
        ..FakeHost::default()
    });
    let e = w.upgrade(&new, &host).await.unwrap_err();
    assert!(!e.to_string().contains("versão antiga"), "{e}");
}

#[tokio::test]
async fn upgrade_old_hooks_without_consent_refused_before_anything() {
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let new = w.manifest("2.0", serde_json::json!({}));
    let plan = w.plan(&new);
    let mut ctx = w.ctx(None, Some(fake_uninstaller));
    let e = execute_install(&plan, &mut ctx).await.unwrap_err();
    assert!(matches!(e, ExecError::NeedsScriptConsent { .. }), "{e:?}");
    assert!(w.events().is_empty());
    assert!(!w.layout.version_dir("app", "2.0").exists());
    assert!(w.intact());
}

#[tokio::test]
async fn upgrade_with_altered_old_manifest_fails_before_any_step() {
    let w = setup().await;
    let old = w.manifest("1.0", uninstall_hooks());
    w.install(&old).await;
    let new = w.manifest("2.0", serde_json::json!({}));
    let plan = w.plan(&new);
    std::fs::write(
        w.written_manifest("1.0"),
        old.replace("info 'pre'", "info 'evil'"),
    )
    .unwrap();
    let host = host_with_events(&w);
    let scripts: Arc<dyn ScriptHost> = host.clone();
    let mut ctx = w.ctx(Some(scripts), Some(fake_uninstaller));
    let e = execute_install(&plan, &mut ctx).await.unwrap_err();
    assert!(e.to_string().contains("não confere"), "{e}");
    assert!(e.to_string().contains("remova e instale de novo"), "{e}");
    assert!(host.seen().is_empty());
    assert!(!w.layout.version_dir("app", "2.0").exists());
}

#[tokio::test]
async fn pre_2c_install_without_manifest_json_uninstalls_without_scripts() {
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    // Installed before Phase 2c: no `manifest.json` in the version folder.
    std::fs::remove_file(w.written_manifest("1.0")).unwrap();
    // An update plans no old steps either, and says the old uninstaller will not run.
    let plan = w.plan(&w.manifest("2.0", serde_json::json!({})));
    assert_eq!(plan.notes.len(), 1, "{:?}", plan.notes);
    assert!(plan.notes[0].contains("não vão rodar"), "{:?}", plan.notes);
    assert!(plan.to_string().contains("Aviso: "));
    assert!(!plan.steps.iter().any(|s| matches!(
        s,
        Step::UpgradeUninstallHook { .. } | Step::UpgradeRunUninstaller { .. }
    )));
    let host = host_with_events(&w);
    let notes = w.uninstall(Some(&host), Some(fake_uninstaller)).unwrap();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("não existe"), "{notes:?}");
    assert!(host.seen().is_empty() && w.events().is_empty());
    assert!(!w.layout.app_dir("app").exists());
    assert!(w.state.get("app").unwrap().is_none());
    // No consent needed when there is nothing to run.
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    std::fs::remove_file(w.written_manifest("1.0")).unwrap();
    w.uninstall(None, None).unwrap();
    assert!(!w.layout.app_dir("app").exists());
}

#[tokio::test]
async fn uninstall_hook_effects_only_for_own_artifacts() {
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let current = w.layout.current("app").display().to_string();
    let line = |v: serde_json::Value| format!("{v}\n");
    let effects = [
        // own variable: removed
        line(serde_json::json!({"op": "set_env", "name": "TOOL_HOME", "value": null})),
        // someone else's variable: ignored
        line(serde_json::json!({"op": "set_env", "name": "OTHER_VAR", "value": "x"})),
        // own PATH entry: removed; an entry the app never recorded: kept
        line(serde_json::json!({"op": "remove_path", "var": "Path", "entries": [current]})),
        line(serde_json::json!({"op": "remove_path", "var": "Path", "entries": [ORIGINAL_PATH]})),
        // own shim: removed; no such shim: ignored
        line(serde_json::json!({"op": "rm_shim", "name": "tool"})),
        line(serde_json::json!({"op": "rm_shim", "name": "other"})),
        // creation effects are not allowed in an uninstall hook
        line(serde_json::json!({"op": "shim", "path": w.layout.app_dir("app").join("x.exe"), "name": "evil", "args": null})),
        line(serde_json::json!({"op": "add_path", "var": "Path", "entries": [w.layout.app_dir("app").join("x")], "prepend": false, "force": false})),
    ]
    .concat();
    let host = Arc::new(FakeHost {
        effects: vec![(HookKind::PreUninstall, effects)],
        probe_env: Some((
            w.env.clone(),
            vec!["TOOL_HOME".into(), "OTHER_VAR".into(), "Path".into()],
        )),
        probe_files: vec![
            w.layout.shims().join("tool.shim"),
            w.layout.shims().join("evil.shim"),
        ],
        ..FakeHost::default()
    });
    // Probed by the hook that runs right after `pre_uninstall`, before the removals.
    let before_path = w.env.get("Path").unwrap().unwrap().value;
    assert!(before_path.contains(&current), "{before_path}");
    w.uninstall(Some(&host), Some(fake_uninstaller)).unwrap();
    let after = host.seen_of(HookKind::UninstallerScript);
    let var = |n: &str| after.env.iter().find(|(k, _)| k == n).unwrap().1.clone();
    assert_eq!(var("TOOL_HOME"), None, "own variable removed by the hook");
    assert_eq!(var("OTHER_VAR"), None, "foreign variable not created");
    let path = var("Path").unwrap();
    assert!(!path.contains(&current), "own entry removed: {path}");
    assert!(path.contains(ORIGINAL_PATH), "foreign entry kept: {path}");
    assert_eq!(after.files, [false, false], "own shim gone, no evil shim");
    assert!(!w.layout.shims().join("evil.shim").exists());
}

#[tokio::test]
async fn ps1_uninstaller_file_runs_through_the_host_at_removal() {
    let w = setup().await;
    w.install(&w.manifest(
        "1.0",
        serde_json::json!({"uninstaller": {"file": "unins.ps1"}}),
    ))
    .await;
    let host = host_with_events(&w);
    w.uninstall(Some(&host), None).unwrap();
    assert_eq!(w.events(), ["UninstallerScript"]);
    let r = host.seen_of(HookKind::UninstallerScript).runner;
    assert!(r.contains("unins.ps1.wingetrs-run.ps1"), "{r}");
    assert!(r.contains("$cmd = 'uninstall'\n"), "{r}");
    assert!(!w.layout.app_dir("app").exists());
}

#[tokio::test]
async fn ps1_uninstaller_file_runs_through_the_host_on_upgrade() {
    let w = setup().await;
    w.install(&w.manifest(
        "1.0",
        serde_json::json!({"uninstaller": {"file": "unins.ps1"}}),
    ))
    .await;
    let host = host_with_events(&w);
    let new = w.manifest("2.0", serde_json::json!({}));
    w.upgrade(&new, &host).await.unwrap();
    assert_eq!(w.events(), ["UninstallerScript"]);
    let r = host.seen_of(HookKind::UninstallerScript).runner;
    let old_dir = w.layout.version_dir("app", "1.0");
    assert!(
        r.contains(&quoted(&old_dir.join("unins.ps1.wingetrs-run.ps1"))),
        "{r}"
    );
    assert!(r.contains("$cmd = 'update'\n"), "{r}");
}

#[tokio::test]
async fn refused_uninstall_script_stops_the_plan_and_the_removal() {
    let w = setup().await;
    let bad = serde_json::json!({"pre_uninstall": "Set-ItemProperty 'HKLM:\\Software\\x' a 1"});
    // At planning (install and update plans carry the check).
    let e = w.try_plan(&w.manifest("1.0", bad.clone())).unwrap_err();
    assert!(
        matches!(&e, PlanError::ScriptRefused { hook, .. } if hook == "pre_uninstall"),
        "{e:?}"
    );
    // At removal: the written manifest is replaced (and re-hashed in the record) by one the
    // scanner refuses; nothing runs and nothing is removed.
    w.install(&w.manifest("1.0", serde_json::json!({}))).await;
    let evil = w.manifest("1.0", bad);
    std::fs::write(w.written_manifest("1.0"), &evil).unwrap();
    let mut db = StateDb::open(&w.state_path).unwrap();
    let (mut rec, arts) = db.get("app").unwrap().unwrap();
    rec.manifest_sha256 = sha256_hex(evil.as_bytes());
    db.record(&rec, &arts).unwrap();
    let host = host_with_events(&w);
    let e = w
        .uninstall(Some(&host), Some(fake_uninstaller))
        .unwrap_err();
    assert!(e.to_string().contains("analisar scripts"), "{e}");
    assert!(host.seen().is_empty());
    assert!(w.intact());
}

#[tokio::test]
async fn upgrade_old_hook_effects_only_for_own_artifacts() {
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let effects = [
        serde_json::json!({"op": "set_env", "name": "OTHER_VAR", "value": "x"}),
        serde_json::json!({"op": "rm_shim", "name": "other"}),
        serde_json::json!({"op": "shim", "path": w.layout.app_dir("app").join("x.exe"), "name": "evil", "args": null}),
    ]
    .iter()
    .map(|v| format!("{v}\n"))
    .collect::<String>();
    let host = Arc::new(FakeHost {
        effects: vec![(HookKind::PreUninstall, effects)],
        ..FakeHost::default()
    });
    w.upgrade(&w.manifest("2.0", serde_json::json!({})), &host)
        .await
        .unwrap();
    assert_eq!(w.env.get("OTHER_VAR").unwrap(), None);
    assert!(!w.layout.shims().join("evil.shim").exists());
    assert_eq!(w.state.get("app").unwrap().unwrap().0.version, "2.0");
}

#[tokio::test]
async fn install_with_only_post_uninstall_needs_script_consent() {
    let w = setup().await;
    let plan = w.plan(&w.manifest("1.0", serde_json::json!({"post_uninstall": "info 'post'"})));
    assert!(plan.has_uninstall_scripts);
    let mut ctx = w.ctx(None, Some(fake_uninstaller));
    let e = execute_install(&plan, &mut ctx).await.unwrap_err();
    assert!(matches!(e, ExecError::NeedsScriptConsent { .. }), "{e:?}");
    assert!(!w.layout.app_dir("app").exists(), "refused before any step");
}

/// The PATH as the hook after `pre_uninstall` found it, when `pre_uninstall` asked to remove
/// the app's own entry and a foreign one, in a removal or in an update.
async fn path_after_remove_path_effect(upgrade: bool) -> (String, String) {
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let current = w.layout.current("app").display().to_string();
    let effects = format!(
        "{}\n{}\n",
        serde_json::json!({"op": "remove_path", "var": "Path", "entries": [current]}),
        serde_json::json!({"op": "remove_path", "var": "Path", "entries": [ORIGINAL_PATH]}),
    );
    let host = Arc::new(FakeHost {
        effects: vec![(HookKind::PreUninstall, effects)],
        probe_env: Some((w.env.clone(), vec!["Path".into()])),
        ..FakeHost::default()
    });
    if upgrade {
        w.upgrade(&w.manifest("2.0", serde_json::json!({})), &host)
            .await
            .unwrap();
    } else {
        w.uninstall(Some(&host), Some(fake_uninstaller)).unwrap();
    }
    let path = host.seen_of(HookKind::UninstallerScript).env[0]
        .1
        .clone()
        .unwrap();
    // The temp root differs between worlds.
    let root = w.layout.root().display().to_string();
    (
        path.replace(&root, "<root>"),
        current.replace(&root, "<root>"),
    )
}

#[tokio::test]
async fn own_path_entry_rule_is_the_same_in_removal_and_update() {
    let (removal, current) = path_after_remove_path_effect(false).await;
    let (update, _) = path_after_remove_path_effect(true).await;
    // Same entries (the shims folder differs by the temp root only through `current`).
    assert_eq!(removal, update);
    assert!(!removal.contains(&current), "{removal}");
    assert!(removal.contains(ORIGINAL_PATH), "{removal}");
}
// ---- Task 191 ----

/// Installs `manifest` (with a `post_install`) whose hook writes `effects` (one JSON per line);
/// `before` runs inside the hook, before the effects are read.
async fn install_with_effects(
    w: &World,
    effects: &[serde_json::Value],
    before: impl Fn() + Send + Sync + 'static,
) -> Result<(), ExecError> {
    struct Hook<F> {
        inner: FakeHost,
        before: F,
    }
    impl<F: Fn() + Send + Sync> ScriptHost for Hook<F> {
        fn run(&self, job: &ScriptJob) -> std::io::Result<ScriptOutcome> {
            (self.before)();
            self.inner.run(job)
        }
    }
    let text: String = effects.iter().map(|v| format!("{v}\n")).collect();
    let host: Arc<dyn ScriptHost> = Arc::new(Hook {
        inner: FakeHost {
            effects: vec![(HookKind::PostInstall, text)],
            ..FakeHost::default()
        },
        before,
    });
    let manifest = w.manifest("1.0", serde_json::json!({"post_install": "info 'x'"}));
    let mut ctx = w.ctx(Some(host), Some(fake_uninstaller));
    execute_install(&w.plan(&manifest), &mut ctx).await
}

fn shim_with_args(w: &World, args: &str) -> serde_json::Value {
    let path = w.layout.version_dir("app", "1.0").join("tool.exe");
    serde_json::json!({"op": "shim", "path": path, "name": "evil", "args": args})
}

#[tokio::test]
async fn shim_effect_args_with_control_char_refused() {
    let w = setup().await;
    let e = install_with_effects(&w, &[shim_with_args(&w, "a\r\ncalc.exe")], || {})
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("argumentos inseguros"), "{e}");
    assert!(!w.layout.shims().join("evil.cmd").exists());
    assert!(!w.layout.shims().join("evil.shim").exists());
    assert!(!w.layout.app_dir("app").exists(), "rolled back: {e}");
    // Plain arguments still make the shim.
    install_with_effects(&w, &[shim_with_args(&w, "--flag x")], || {})
        .await
        .unwrap();
    assert!(w.layout.shims().join("evil.shim").exists());
}

#[tokio::test]
async fn shim_effect_args_with_quote_refused() {
    let w = setup().await;
    let e = install_with_effects(&w, &[shim_with_args(&w, "x\" & calc.exe & \"")], || {})
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("argumentos inseguros"), "{e}");
    assert!(!w.layout.shims().join("evil.shim").exists());
    assert!(!w.layout.app_dir("app").exists(), "rolled back: {e}");
}

fn add_path_effect(entry: &Path) -> serde_json::Value {
    serde_json::json!({"op": "add_path", "var": "Path", "entries": [entry], "prepend": true, "force": false})
}

/// Makes the junction `link` -> `target` when the hook runs.
fn junction(target: PathBuf, link: PathBuf) -> impl Fn() + Send + Sync + 'static {
    move || wingetrs::install::link::create(&target, &link).unwrap()
}

/// A junction the hook makes inside the app folder cannot carry a PATH entry, a shim or a
/// shortcut outside it; a dangling one is refused too. A junction that stays inside the
/// app's persist folder is fine.
#[tokio::test]
async fn effect_path_through_junction_refused() {
    let w = setup().await;
    let outside = w.base().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), b"x").unwrap();
    std::fs::write(outside.join("evil.exe"), fake_pe(3)).unwrap();
    let vdir = w.layout.version_dir("app", "1.0");
    let link = vdir.join("x");
    let path_before = w.env.get("Path").unwrap().unwrap().value;
    let cases = [
        add_path_effect(&link),
        add_path_effect(&link.join("bin")),
        serde_json::json!({"op": "shim", "path": link.join("evil.exe"), "name": "evil", "args": null}),
        serde_json::json!({"op": "shortcut", "target": link.join("evil.exe"), "name": "Evil", "args": null, "icon": null}),
        serde_json::json!({"op": "shortcut", "target": vdir.join("tool.exe"), "name": "Evil", "args": null, "icon": link.join("evil.exe")}),
    ];
    for effect in cases {
        let make = junction(outside.clone(), link.clone());
        let e = install_with_effects(&w, std::slice::from_ref(&effect), make)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("junção"), "{effect}: {e}");
        assert!(!w.layout.app_dir("app").exists(), "rolled back: {e}");
        assert_eq!(w.env.get("Path").unwrap().unwrap().value, path_before);
        assert!(!w.layout.shims().join("evil.shim").exists());
        assert!(w.shortcuts.dump().is_empty(), "{e}");
        assert!(
            outside.join("keep.txt").exists(),
            "rollback stayed out of the junction"
        );
    }
    // Dangling: the target may be created anywhere later.
    let make = junction(w.base().join("missing"), link.clone());
    let e = install_with_effects(&w, &[add_path_effect(&link)], make)
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("junção") && e.contains("quebrada"), "{e}");
    assert!(!w.layout.app_dir("app").exists(), "rolled back: {e}");
    // Inside the app's persist folder: applied.
    let bin = w.layout.persist_dir("app").join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    install_with_effects(&w, &[add_path_effect(&link)], junction(bin, link.clone()))
        .await
        .unwrap();
    let path = w.env.get("Path").unwrap().unwrap().value;
    assert!(path.contains(&link.display().to_string()), "{path}");
}

/// A removal that fails after its uninstall code ran says what that code already changed
/// (nothing undoes it).
#[tokio::test]
async fn removal_failure_after_scripts_mentions_effects() {
    let unset_home = format!(
        "{}\n",
        serde_json::json!({"op": "set_env", "name": "TOOL_HOME", "value": null})
    );
    // A later script fails.
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let host = Arc::new(FakeHost {
        fail_on: Some(HookKind::UninstallerScript),
        effects: vec![(HookKind::PreUninstall, unset_home.clone())],
        ..FakeHost::default()
    });
    let e = w
        .uninstall(Some(&host), Some(fake_uninstaller))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("os scripts de desinstalação já rodaram e alteraram: variável TOOL_HOME"),
        "{e}"
    );
    assert!(w.intact());
    // The scripts ran without changing anything wingetrs applies: still said.
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let host = Arc::new(FakeHost {
        fail_on: Some(HookKind::UninstallerScript),
        ..FakeHost::default()
    });
    let e = w
        .uninstall(Some(&host), Some(fake_uninstaller))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("os scripts de desinstalação já rodaram e não foram desfeitos"),
        "{e}"
    );
    // Removing the app folder fails (a file held open without sharing).
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let w = setup().await;
        w.install(&w.manifest("1.0", uninstall_hooks())).await;
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(w.layout.version_dir("app", "1.0").join("tool.exe"))
            .unwrap();
        let host = Arc::new(FakeHost {
            effects: vec![(HookKind::PreUninstall, unset_home)],
            ..FakeHost::default()
        });
        let e = w
            .uninstall(Some(&host), Some(fake_uninstaller))
            .unwrap_err()
            .to_string();
        drop(held);
        assert!(e.contains("remover app"), "{e}");
        assert!(
            e.contains("os scripts de desinstalação já rodaram e alteraram: variável TOOL_HOME"),
            "{e}"
        );
    }
    // Nothing ran yet (no consent): no such text.
    let w = setup().await;
    w.install(&w.manifest("1.0", uninstall_hooks())).await;
    let e = w
        .uninstall(None, Some(fake_uninstaller))
        .unwrap_err()
        .to_string();
    assert!(!e.contains("já rodaram"), "{e}");
}

/// Task 191: shim arguments with cmd syntax (`&`, `%VAR%`) or PowerShell syntax (`$(…)`, `'`, `{}`,
/// a typographic quote) are refused from a hook effect and from the manifest `bin` alike; a
/// quoted path passes the `bin` rule.
#[tokio::test]
async fn shim_args_with_cmd_syntax_refused() {
    let w = setup().await;
    let typographic = "\u{201C}calc\u{201D}";
    for args in [
        "x & calc",
        "%PATH%",
        "$(calc)",
        "'; calc; '",
        "{calc}",
        typographic,
    ] {
        let e = install_with_effects(&w, &[shim_with_args(&w, args)], || {})
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("argumentos inseguros"), "{args}: {e}");
        assert!(!w.layout.shims().join("evil.cmd").exists());
        assert!(!w.layout.app_dir("app").exists(), "rolled back: {e}");
        let bin = serde_json::json!({"bin": [["tool.exe", "evil", args]]});
        let r = w.try_plan(&w.manifest("1.0", bin));
        assert!(matches!(r, Err(PlanError::UnsafeArgs(_))), "{args}: {r:?}");
    }
    // A quoted path passes the manifest rule; a hook's shim takes no quotes at all.
    let ok = r#"--flag "C:\Program Files\x""#;
    let bin = serde_json::json!({"bin": [["tool.exe", "ok", ok]]});
    assert!(w.try_plan(&w.manifest("1.0", bin)).is_ok());
    install_with_effects(&w, &[shim_with_args(&w, "--flag x")], || {})
        .await
        .unwrap();
}

/// Task 191: a hook's shim argument may start with our own folder even when the user profile
/// holds `João` and a space; it reaches both shims quoted. What follows the folder still
/// passes the allowlist.
#[tokio::test]
async fn shim_effect_args_under_non_ascii_profile() {
    let w = setup_at(r"João Silva\root").await;
    let vdir = w.layout.version_dir("app", "1.0").display().to_string();
    let data = format!(r"{vdir}\data");
    let args = format!("--data {data} -q");
    let shim_to = |file: &str, name: &str, args: &str| {
        let path = w.layout.version_dir("app", "1.0").join(file);
        serde_json::json!({"op": "shim", "path": path, "name": name, "args": args})
    };
    let bad = format!(r"{data}\x & calc");
    let e = install_with_effects(&w, &[shim_to("t.cmd", "tc", &bad)], || {})
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("argumentos inseguros"), "{e}");
    assert!(!w.layout.app_dir("app").exists(), "rolled back: {e}");
    let effects = [shim_to("t.cmd", "tc", &args), shim_to("t.ps1", "tp", &args)];
    install_with_effects(&w, &effects, || {}).await.unwrap();
    let cmd = std::fs::read_to_string(w.layout.shims().join("tc.cmd")).unwrap();
    assert!(cmd.contains(&format!("--data \"{data}\" -q %*")), "{cmd}");
    let ps1 = std::fs::read_to_string(w.layout.shims().join("tp.ps1")).unwrap();
    assert!(
        ps1.contains(&format!("$shimArgs = @('--data', '{data}', '-q')")),
        "{ps1}"
    );
}
