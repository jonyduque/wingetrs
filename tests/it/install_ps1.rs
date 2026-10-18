#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 158): `installer.file` ending in `.ps1`.
//!
//! The script host is a fake: it records each `ScriptJob`, reads the files the executor wrote
//! and answers a code. Nothing here runs PowerShell.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::executor::{
    ExecCtx, ExecError, NoHook, Runners, ScriptHost, ScriptJob, ScriptOutcome, StepHook,
};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, HookKind, resolve};
use wingetrs::install::plan::{InstallPlan, Step};
use wingetrs::install::planner::{InstalledInfo, PlanError, PlanInput, plan_install};
use wingetrs::install::runner::ps_single_quoted;
use wingetrs::install::script_scan::{Verdict, ps1_installer_report};
use wingetrs::install::state::StateDb;
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

fn fixture_zip(ps1: &[u8]) -> Vec<u8> {
    zip_of(&[
        ("pkg/setup.ps1", ps1),
        ("pkg/tool.exe", &fake_pe(3)),
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

/// `installer.file` runner: `dir` is `<tempdir>\root\apps\<app>\<version>`.
fn fake_installer(
    _exe: &Path,
    _args: &[String],
    _dir: &Path,
) -> std::io::Result<(i32, Vec<String>)> {
    Ok((0, Vec::new()))
}

/// What the fake host saw for one job.
#[derive(Clone)]
struct Seen {
    job: ScriptJob,
    runner: Vec<u8>,
}

#[derive(Default)]
struct FakeHost {
    seen: Mutex<Vec<Seen>>,
    code: i32,
    /// Simulates the script writing a user variable straight to the registry.
    set_env: Option<(Arc<FakeEnv>, String)>,
    write_effect: bool,
    io_error: bool,
    panic: bool,
}

impl FakeHost {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
    fn runner_of(&self, kind: HookKind) -> String {
        let s = self.seen();
        let seen = s.iter().find(|s| s.job.hook == kind).expect("hook ran");
        String::from_utf8(seen.runner[3..].to_vec()).unwrap()
    }
}

impl ScriptHost for FakeHost {
    fn run(&self, job: &ScriptJob) -> std::io::Result<ScriptOutcome> {
        if self.panic {
            panic!("script host bug QQP");
        }
        let runner = std::fs::read(&job.runner)?;
        self.seen.lock().unwrap().push(Seen {
            job: job.clone(),
            runner,
        });
        if let Some((env, name)) = &self.set_env {
            env.set(
                name,
                &EnvValue {
                    value: "1".into(),
                    expand: false,
                },
            )
            .unwrap();
        }
        if self.write_effect {
            std::fs::write(
                job.workdir.join("effects.jsonl"),
                b"{\"op\":\"env\",\"name\":\"A\",\"value\":\"b\"}\n",
            )?;
        }
        if self.io_error {
            return Err(std::io::Error::other("powershell ausente ZZK"));
        }
        let lines = (1..=6).map(|i| format!("linha{i}")).collect();
        Ok(ScriptOutcome {
            code: self.code,
            lines,
        })
    }
}

struct World {
    server: MockServer,
    _t: TempDir,
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
    fn manifest(&self, version: &str, extra: serde_json::Value) -> String {
        let mut m = serde_json::json!({
            "version": version,
            "url": self.url,
            "hash": self.sha,
            "extract_dir": "pkg",
            "bin": "tool.exe",
            "shortcuts": [["tool.exe", "Tool"]],
            "persist": ["conf"],
            "env_set": {"TOOL_HOME": "$dir"},
            "env_add_path": ["."],
        });
        for (k, v) in extra.as_object().unwrap() {
            m[k] = v.clone();
        }
        m.to_string()
    }

    fn try_plan(&self, manifest: &str) -> Result<InstallPlan, PlanError> {
        let m = resolve(manifest, Arch::X64).unwrap();
        let installed = self
            .state
            .get("app")
            .unwrap()
            .map(|(r, _)| InstalledInfo { version: r.version });
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
            manifest_sha256: "",
        })
    }

    fn plan(&self, extra: serde_json::Value) -> InstallPlan {
        self.try_plan(&self.manifest("1.0", extra)).unwrap()
    }

    /// The hook is copied and leaked so a temporary (`&FailAt(k)`) can be passed.
    fn ctx<H: StepHook + Clone + 'static>(
        &self,
        hook: &H,
        scripts: Option<Arc<dyn ScriptHost>>,
    ) -> ExecCtx<'_> {
        let hook: &'static H = Box::leak(Box::new(hook.clone()));
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
            hook,
            progress: &no_progress,
            in_use: &nothing_in_use,
            msiexec: None,
            launch: Default::default(),
            runners: Runners {
                installer: Some(fake_installer),
                scripts,
                ..Runners::disabled()
            },
        }
    }

    async fn run(&self, plan: &InstallPlan, host: &Arc<FakeHost>) -> Result<(), ExecError> {
        let scripts: Arc<dyn ScriptHost> = host.clone();
        let mut ctx = self.ctx(&NoHook, Some(scripts));
        wingetrs::install::executor::execute_install(plan, &mut ctx).await
    }

    fn snapshot(&self) -> std::collections::BTreeMap<String, String> {
        let mut s = wingetrs::install::snapshot::snapshot(self.layout.root()).unwrap();
        s.retain(|p, _| !p.starts_with("blobs") && !p.starts_with("tmp"));
        s
    }

    /// `script-*` folders left in `<root>\tmp`.
    fn script_dirs(&self) -> Vec<String> {
        match std::fs::read_dir(self.layout.root().join("tmp")) {
            Ok(rd) => rd
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("script-"))
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

async fn setup() -> World {
    setup_with(
        b"param($a, $b)
",
    )
    .await
}

async fn setup_with(ps1: &[u8]) -> World {
    let server = MockServer::start().await;
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("root");
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
    let zip = fixture_zip(ps1);
    let sha = sha256_hex(&zip);
    Mock::given(method("GET"))
        .and(path("/pkg.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
        .mount(&server)
        .await;
    let url = format!("{}/pkg.zip", server.uri());
    World {
        server,
        _t: t,
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

fn quoted(p: &Path) -> String {
    ps_single_quoted(&p.to_string_lossy())
}

fn ps1_manifest(args: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"installer": {"file": "setup.ps1", "args": args}})
}

#[tokio::test]
async fn ps1_runs_via_host_with_ps1_body() {
    let w = setup().await;
    let host = Arc::new(FakeHost::default());
    let plan = w.plan(ps1_manifest(serde_json::json!(["/S", "$dir"])));
    assert!(
        plan.steps
            .iter()
            .any(|s| matches!(s, Step::RunPs1Installer { file, .. } if file == "setup.ps1"))
    );
    w.run(&plan, &host).await.unwrap();
    let vdir = w.layout.version_dir("app", "1.0");
    let runner = host.runner_of(HookKind::InstallerScript);
    let call = format!(
        "& {} @('/S',{})",
        quoted(&vdir.join("setup.ps1")),
        quoted(&vdir)
    );
    assert!(runner.contains(&call), "{runner}");
    assert!(
        runner.contains(&format!("$dir = {}\n", quoted(&vdir))),
        "{runner}"
    );
    assert!(w.script_dirs().is_empty());
    assert!(w.layout.current("app").join("tool.exe").exists());
}

#[tokio::test]
async fn ps1_not_deleted_without_keep() {
    let w = setup().await;
    let host = Arc::new(FakeHost::default());
    let plan = w.plan(ps1_manifest(serde_json::json!(["/S"])));
    w.run(&plan, &host).await.unwrap();
    assert!(
        w.layout
            .version_dir("app", "1.0")
            .join("setup.ps1")
            .is_file()
    );
}

#[tokio::test]
async fn ps1_needs_consent() {
    let w = setup().await;
    let plan = w.plan(ps1_manifest(serde_json::json!([])));
    let mut ctx = w.ctx(&NoHook, None);
    let r = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        wingetrs::install::executor::execute_install(&plan, &mut ctx),
    )
    .await
    .expect("must answer at once");
    drop(ctx);
    match r {
        Err(ExecError::NeedsScriptConsent { app, .. }) => assert_eq!(app, "app"),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.server.received_requests().await.unwrap().len(), 0);
    assert!(!w.layout.app_dir("app").exists());
}

#[tokio::test]
async fn ps1_then_installer_script() {
    let w = setup().await;
    let host = Arc::new(FakeHost::default());
    let plan = w.plan(serde_json::json!({
        "installer": {"file": "setup.ps1", "script": "info 'after'"}
    }));
    let kinds: Vec<&str> = plan
        .steps
        .iter()
        .filter_map(|s| match s {
            Step::RunPs1Installer { .. } => Some("ps1"),
            Step::Hook {
                kind: HookKind::InstallerScript,
                ..
            } => Some("InstallerScript"),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, ["ps1", "InstallerScript"]);
    w.run(&plan, &host).await.unwrap();
    let seen = host.seen();
    assert_eq!(seen.len(), 2);
    let text = |i: usize| String::from_utf8(seen[i].runner[3..].to_vec()).unwrap();
    assert!(text(0).contains("setup.ps1"), "{}", text(0));
    assert!(!text(1).contains("setup.ps1"), "{}", text(1));
}

#[tokio::test]
async fn ps1_nonzero_rolls_back() {
    let w = setup().await;
    let before = w.snapshot();
    let host = Arc::new(FakeHost {
        code: 5,
        ..FakeHost::default()
    });
    let plan = w.plan(ps1_manifest(serde_json::json!([])));
    match w.run(&plan, &host).await {
        Err(ExecError::Failed { message, .. }) => {
            assert!(message.contains("código 5"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(w.snapshot(), before);
    assert!(w.script_dirs().is_empty());
}

#[test]
fn ps1_report_is_confirm_strong() {
    let r = ps1_installer_report("setup.ps1");
    assert_eq!(r.verdict, Verdict::ConfirmStrong);
    assert!(
        r.flags
            .iter()
            .any(|f| f.detail == "instalador .ps1: conteúdo não analisado"),
        "{:?}",
        r.flags
    );
}

/// The file's own content is scanned before it runs: a refused one runs nothing.
#[tokio::test]
async fn ps1_content_is_scanned_before_running() {
    let w = setup_with(b"scoop install x\r\n").await;
    let host = Arc::new(FakeHost::default());
    let before = w.snapshot();
    let plan = w.plan(ps1_manifest(serde_json::json!([])));
    match w.run(&plan, &host).await {
        Err(ExecError::Failed { message, .. }) => {
            assert!(message.contains("recusado pela análise"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert!(host.seen().is_empty());
    assert_eq!(w.snapshot(), before);
}

#[tokio::test]
async fn ps1_oversized_or_utf16_content_is_refused() {
    let big = vec![b'#'; 1024 * 1024 + 1];
    let utf16: Vec<u8> = "info 'x'"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    for (content, what) in [(big, "1 MiB"), (utf16, "UTF-16")] {
        let w = setup_with(&content).await;
        let host = Arc::new(FakeHost::default());
        let plan = w.plan(ps1_manifest(serde_json::json!([])));
        match w.run(&plan, &host).await {
            Err(ExecError::Failed { message, .. }) => {
                assert!(message.contains(what), "{message}");
            }
            other => panic!("{what}: {other:?}"),
        }
        assert!(host.seen().is_empty(), "{what}");
    }
}

#[tokio::test]
async fn ps1_missing_file_fails() {
    let w = setup().await;
    let host = Arc::new(FakeHost::default());
    let plan = w.plan(serde_json::json!({"installer": {"file": "nao-existe.ps1"}}));
    match w.run(&plan, &host).await {
        Err(ExecError::Failed { message, .. }) => {
            assert!(message.contains("não existe"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert!(host.seen().is_empty());
}

/// The manifest resolver already refuses a `file` that escapes the version folder.
#[test]
fn ps1_path_escape_is_refused_by_the_manifest() {
    let m = serde_json::json!({
        "version": "1.0", "url": "http://x/p.zip", "hash": "aa",
        "installer": {"file": "../evil.ps1"}
    })
    .to_string();
    let r = resolve(&m, Arch::X64);
    assert!(
        matches!(&r, Err(e) if e.to_string().contains("inseguro")),
        "{:?}",
        r.map(|_| ())
    );
}
