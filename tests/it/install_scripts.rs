#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 156).
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
use wingetrs::install::plan::InstallPlan;
use wingetrs::install::planner::{InstalledInfo, PlanError, PlanInput, plan_install};
use wingetrs::install::prelude::{CONTRACT_VARS, PRELUDE};
use wingetrs::install::runner::ps_single_quoted;
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

fn fixture_zip() -> Vec<u8> {
    zip_of(&[
        ("pkg/tool.exe", &fake_pe(3)),
        ("pkg/setup.exe", &fake_pe(3)),
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

/// `installer.file` runner: `dir` is `<tempdir>\root\apps\<app>\<version>`.
fn fake_installer(
    _exe: &Path,
    _args: &[String],
    dir: &Path,
) -> std::io::Result<(i32, Vec<String>)> {
    log_event(dir.ancestors().nth(4).unwrap(), "installer");
    Ok((0, Vec::new()))
}

/// What the fake host saw for one job.
#[derive(Clone)]
struct Seen {
    job: ScriptJob,
    runner: Vec<u8>,
    env_json: String,
    workdir_existed: bool,
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
    events: Option<PathBuf>,
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
        let env_json = std::fs::read_to_string(job.workdir.join("env.json"))?;
        self.seen.lock().unwrap().push(Seen {
            job: job.clone(),
            runner,
            env_json,
            workdir_existed: job.workdir.is_dir(),
        });
        if let Some(base) = &self.events {
            log_event(base, &format!("{:?}", job.hook));
        }
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

    fn events(&self) -> Vec<String> {
        std::fs::read_to_string(self.base().join(EVENTS))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

async fn setup() -> World {
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
    let zip = fixture_zip();
    let sha = sha256_hex(&zip);
    Mock::given(method("GET"))
        .and(path("/pkg.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
        .mount(&server)
        .await;
    let url = format!("{}/pkg.zip", server.uri());
    World {
        server,
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

#[derive(Clone)]
struct FailAt(usize);
impl StepHook for FailAt {
    fn before(&self, i: usize, _s: &wingetrs::install::plan::Step) -> Result<(), ExecError> {
        if i == self.0 {
            Err(ExecError::Hook(format!("injected at {i}")))
        } else {
            Ok(())
        }
    }
}

fn all_hooks() -> serde_json::Value {
    serde_json::json!({
        "pre_install": "info 'pre'",
        "installer": {"file": "setup.exe", "args": ["/S"], "script": "info 'installer'"},
        "post_install": "info 'post'",
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
async fn hooks_run_in_scoop_order() {
    let w = setup().await;
    let host = host_with_events(&w);
    w.run(&w.plan(all_hooks()), &host).await.unwrap();
    assert_eq!(
        w.events(),
        ["PreInstall", "installer", "InstallerScript", "PostInstall"]
    );
    assert!(w.layout.current("app").join("tool.exe").exists());
}

#[tokio::test]
async fn installer_script_alone_runs_no_file() {
    let w = setup().await;
    let host = host_with_events(&w);
    let plan = w.plan(serde_json::json!({"installer": {"script": "info 'x'"}}));
    w.run(&plan, &host).await.unwrap();
    assert_eq!(w.events(), ["InstallerScript"]);
}

#[tokio::test]
async fn dir_per_hook() {
    let w = setup().await;
    let host = Arc::new(FakeHost::default());
    let plan = w.plan(serde_json::json!({
        "pre_install": "info 'pre'",
        "post_install": "info 'post'",
    }));
    w.run(&plan, &host).await.unwrap();
    let vdir = w.layout.version_dir("app", "1.0");
    let current = w.layout.current("app");
    let pre = host.runner_of(HookKind::PreInstall);
    assert!(
        pre.contains(&format!("$dir = {}\n", quoted(&vdir))),
        "{pre}"
    );
    let post = host.runner_of(HookKind::PostInstall);
    assert!(
        post.contains(&format!("$dir = {}\n", quoted(&current))),
        "{post}"
    );
    assert!(
        post.contains(&format!("$original_dir = {}\n", quoted(&vdir))),
        "{post}"
    );
    assert!(pre.contains("$cmd = 'install'\n"), "{pre}");
}

#[tokio::test]
async fn upgrade_install_hooks_see_cmd_update() {
    let w = setup().await;
    let host = Arc::new(FakeHost::default());
    w.run(&w.plan(serde_json::json!({})), &host).await.unwrap();
    let plan = w
        .try_plan(&w.manifest("2.0", serde_json::json!({"post_install": "info 'x'"})))
        .unwrap();
    assert_eq!(plan.upgrade_from.as_deref(), Some("1.0"));
    w.run(&plan, &host).await.unwrap();
    let post = host.runner_of(HookKind::PostInstall);
    assert!(post.contains("$cmd = 'update'\n"), "{post}");
    assert!(post.contains("$old_version = '1.0'\n"), "{post}");
}

#[tokio::test]
async fn no_consent_fails_before_anything() {
    let w = setup().await;
    let host = Arc::new(FakeHost::default());
    let plan = w.plan(serde_json::json!({"post_install": "info 'x'"}));
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
    assert!(host.seen().is_empty());
    assert!(!w.layout.app_dir("app").exists());
}

#[tokio::test]
async fn refused_script_fails_at_planning() {
    let w = setup().await;
    let r = w.try_plan(&w.manifest(
        "1.0",
        serde_json::json!({"post_install": "scoop install x"}),
    ));
    match r {
        Err(PlanError::ScriptRefused { hook, reasons }) => {
            assert_eq!(hook, "post_install");
            assert!(reasons.iter().any(|r| r.contains("scoop")), "{reasons:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn refused_pre_install_and_installer_script_name_their_hook() {
    let w = setup().await;
    for (extra, key) in [
        (
            serde_json::json!({"pre_install": "scoop install x"}),
            "pre_install",
        ),
        (
            serde_json::json!({"installer": {"script": "scoop install x"}}),
            "installer.script",
        ),
    ] {
        let e = w.try_plan(&w.manifest("1.0", extra)).unwrap_err();
        assert!(
            e.to_string()
                .starts_with(&format!("o script {key} foi recusado: ")),
            "{e}"
        );
    }
}

#[tokio::test]
async fn unknown_scoop_function_refused_with_name() {
    let w = setup().await;
    let r = w.try_plan(&w.manifest(
        "1.0",
        serde_json::json!({"post_install": "Expand-ZstdArchive x"}),
    ));
    match r {
        Err(PlanError::ScriptRefused { reasons, .. }) => assert!(
            reasons
                .iter()
                .any(|r| r == "função do Scoop ainda não suportada: Expand-ZstdArchive"),
            "{reasons:?}"
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn nonzero_exit_rolls_back_and_lists_changed_env() {
    let w = setup().await;
    let host = Arc::new(FakeHost {
        code: 1,
        set_env: Some((w.env.clone(), "X".into())),
        ..FakeHost::default()
    });
    let plan = w.plan(serde_json::json!({"post_install": "Get-EnvVar -Name 'X'"}));
    let e = w.run(&plan, &host).await.unwrap_err().to_string();
    assert!(e.contains("código 1"), "{e}");
    assert!(e.contains("X"), "{e}");
    assert!(
        e.contains("variáveis alteradas pelo script e não desfeitas: X"),
        "{e}"
    );
    assert!(e.contains("linha2") && e.contains("linha6"), "{e}");
    assert!(!e.contains("linha1"), "only the last 5 lines: {e}");
    assert!(!w.layout.app_dir("app").exists());
}

#[tokio::test]
async fn nonzero_exit_without_env_change_lists_nothing() {
    let w = setup().await;
    let host = Arc::new(FakeHost {
        code: 2,
        ..FakeHost::default()
    });
    let plan = w.plan(serde_json::json!({"post_install": "Get-EnvVar -Name 'X'"}));
    let e = w.run(&plan, &host).await.unwrap_err().to_string();
    assert!(e.contains("o script post_install saiu com código 2"), "{e}");
    assert!(!e.contains("não desfeitas"), "{e}");
}

#[tokio::test]
async fn host_error_fails_and_rolls_back() {
    let w = setup().await;
    let host = Arc::new(FakeHost {
        io_error: true,
        ..FakeHost::default()
    });
    let plan = w.plan(serde_json::json!({"pre_install": "info 'x'"}));
    let e = w.run(&plan, &host).await.unwrap_err().to_string();
    assert!(e.contains("não consegui rodar o script pre_install"), "{e}");
    assert!(e.contains("ZZK"), "{e}");
    assert!(!w.layout.app_dir("app").exists());
    assert!(w.script_dirs().is_empty());
}

#[tokio::test]
async fn host_panic_rolls_back_then_propagates() {
    use futures_util::FutureExt;
    let w = setup().await;
    let before = w.snapshot();
    let host = Arc::new(FakeHost {
        panic: true,
        ..FakeHost::default()
    });
    let plan = w.plan(serde_json::json!({"post_install": "info 'x'"}));
    let scripts: Arc<dyn ScriptHost> = host.clone();
    let mut ctx = w.ctx(&NoHook, Some(scripts));
    let caught = std::panic::AssertUnwindSafe(wingetrs::install::executor::execute_install(
        &plan, &mut ctx,
    ))
    .catch_unwind()
    .await;
    drop(ctx);
    let payload = caught.expect_err("the host panic must propagate");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("script host bug QQP")
    );
    assert_eq!(w.snapshot(), before);
    assert!(w.script_dirs().is_empty());
}

#[tokio::test]
async fn failure_at_each_step_restores_initial_state() {
    let probe = setup().await;
    let n = probe.plan(all_hooks()).steps.len();
    assert!(n >= 14, "plan must exercise every step kind: {n}");
    for k in 0..n {
        let s = setup().await;
        let plan = s.plan(all_hooks());
        let host = Arc::new(FakeHost::default());
        let before_fs = s.snapshot();
        let before_env = s.env.dump();
        let before_sc = s.shortcuts.dump();
        let scripts: Arc<dyn ScriptHost> = host.clone();
        let mut ctx = s.ctx(&FailAt(k), Some(scripts));
        let err = wingetrs::install::executor::execute_install(&plan, &mut ctx)
            .await
            .unwrap_err();
        drop(ctx);
        assert!(
            err.to_string().contains(&format!("injected at {k}")),
            "{err}"
        );
        assert_eq!(s.snapshot(), before_fs, "filesystem differs at step {k}");
        assert_eq!(s.env.dump(), before_env, "env differs at step {k}");
        assert_eq!(
            s.shortcuts.dump(),
            before_sc,
            "shortcuts differ at step {k}"
        );
        assert!(s.state.apps().unwrap().is_empty(), "state at step {k}");
        assert!(s.script_dirs().is_empty(), "script folder left at step {k}");
    }
}

#[tokio::test]
async fn workdir_removed_on_success_and_failure() {
    for code in [0, 1] {
        let w = setup().await;
        let host = Arc::new(FakeHost {
            code,
            ..FakeHost::default()
        });
        let plan = w.plan(serde_json::json!({"post_install": "info 'x'"}));
        let r = w.run(&plan, &host).await;
        assert_eq!(r.is_ok(), code == 0, "{r:?}");
        let seen = host.seen();
        assert_eq!(seen.len(), 1);
        assert!(
            seen[0].workdir_existed,
            "the folder existed while the host ran"
        );
        let name = seen[0].job.workdir.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("script-app-post_install-"), "{name}");
        assert_eq!(
            seen[0].job.workdir.parent().unwrap(),
            w.layout.root().join("tmp")
        );
        assert!(!seen[0].job.workdir.exists(), "code {code}");
        assert!(w.script_dirs().is_empty(), "code {code}");
    }
}

#[tokio::test]
async fn runner_has_bom_and_prelude_is_ascii() {
    let w = setup().await;
    let host = Arc::new(FakeHost::default());
    let plan = w.plan(serde_json::json!({"post_install": "info 'x'"}));
    w.run(&plan, &host).await.unwrap();
    let seen = host.seen();
    assert_eq!(&seen[0].runner[..3], b"\xEF\xBB\xBF");
    assert!(PRELUDE.is_ascii());
    let text = host.runner_of(HookKind::PostInstall);
    for v in CONTRACT_VARS {
        assert!(text.contains(&format!("${v} = ")), "{v}: {text}");
    }
    let root = w.layout.root();
    assert!(
        text.contains(&format!("$__wr_root = {}\n", quoted(root))),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "$__wr_shortcut_root = {}\n",
            quoted(&w.shortcuts.root())
        )),
        "{text}"
    );
    // No helper installed: an empty hashtable, never `$null` (the prelude calls ContainsKey).
    assert!(text.contains("$__wr_helpers = @{}\n"), "{text}");
    let job = &seen[0].job;
    let env: Vec<(String, PathBuf)> = job
        .env
        .iter()
        .map(|(k, v)| (k.to_string_lossy().into_owned(), PathBuf::from(v)))
        .collect();
    assert_eq!(
        env,
        [
            ("SCOOP".to_string(), root.to_path_buf()),
            ("SCOOP_GLOBAL".to_string(), root.join(".global-disabled")),
            ("SCOOP_CACHE".to_string(), root.join("cache")),
        ]
    );
}

#[tokio::test]
async fn installed_helpers_are_listed() {
    let w = setup().await;
    let seven = w.layout.current("7zip").join("7z.exe");
    std::fs::create_dir_all(seven.parent().unwrap()).unwrap();
    std::fs::write(&seven, b"x").unwrap();
    let host = Arc::new(FakeHost::default());
    let plan = w.plan(serde_json::json!({"post_install": "info 'x'"}));
    w.run(&plan, &host).await.unwrap();
    let text = host.runner_of(HookKind::PostInstall);
    assert!(
        text.contains(&format!("$__wr_helpers = @{{'7zip'={}}}\n", quoted(&seven))),
        "{text}"
    );
}

#[tokio::test]
async fn env_seed_contains_script_names() {
    let w = setup().await;
    w.env
        .set(
            "JAVA_HOME",
            &EnvValue {
                value: r"C:\jdk".into(),
                expand: false,
            },
        )
        .unwrap();
    let host = Arc::new(FakeHost::default());
    let plan = w.plan(serde_json::json!({"pre_install": "Get-EnvVar -Name 'JAVA_HOME'"}));
    w.run(&plan, &host).await.unwrap();
    let seen = host.seen();
    let env: serde_json::Value = serde_json::from_str(&seen[0].env_json).unwrap();
    assert_eq!(
        env,
        serde_json::json!({
            "Path": ORIGINAL_PATH,
            "PSModulePath": null,
            "JAVA_HOME": r"C:\jdk",
        })
    );
    // The runner hands the same seed to the prelude.
    let text = host.runner_of(HookKind::PreInstall);
    assert!(
        text.contains(&format!(
            "$__wr_env_seed = {}\n",
            ps_single_quoted(&seen[0].env_json)
        )),
        "{text}"
    );
}

#[tokio::test]
async fn env_seed_names_differing_only_in_case_appear_once() {
    let w = setup().await;
    let host = Arc::new(FakeHost::default());
    let plan = w.plan(serde_json::json!({"post_install": "Get-EnvVar -Name 'PATH'"}));
    w.run(&plan, &host).await.unwrap();
    let env: serde_json::Value = serde_json::from_str(&host.seen()[0].env_json).unwrap();
    let keys: Vec<&String> = env.as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
}

#[tokio::test]
async fn unknown_effect_op_fails_unreadable() {
    let w = setup().await;
    let before = w.snapshot();
    let host = Arc::new(FakeHost {
        write_effect: true,
        ..FakeHost::default()
    });
    let plan = w.plan(serde_json::json!({"post_install": "info 'x'"}));
    let e = w.run(&plan, &host).await.unwrap_err().to_string();
    assert!(e.contains("efeitos do script ilegíveis"), "{e}");
    assert!(!w.layout.app_dir("app").exists());
    assert_eq!(w.snapshot(), before);
}
