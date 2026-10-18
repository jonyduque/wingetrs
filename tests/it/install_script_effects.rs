#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 157): `effects.jsonl` applied through the phase 2a paths.
//!
//! The script host is a fake: it writes the `effects.jsonl` chosen by the test and exits 0.
//! Nothing here runs PowerShell or touches the real environment.
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
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::plan::{InstallPlan, UninstallPlan};
use wingetrs::install::planner::{InstalledInfo, PlanError, PlanInput, plan_install};
use wingetrs::install::state::{AppRecord, Artifact, ArtifactKind, StateDb};
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

/// `installer.file` runner (never reached here: no plan has one).
fn fake_installer(
    _exe: &Path,
    _args: &[String],
    _dir: &Path,
) -> std::io::Result<(i32, Vec<String>)> {
    Ok((0, Vec::new()))
}

/// Writes `effects[i]` (raw bytes) into `effects.jsonl` on its i-th run and exits 0; records
/// the user `Path` it saw when each run started.
#[derive(Default)]
struct FakeHost {
    effects: Vec<Vec<u8>>,
    calls: Mutex<usize>,
    env: Option<Arc<FakeEnv>>,
    path_seen: Mutex<Vec<Option<String>>>,
}

impl FakeHost {
    fn writing(lines: &[serde_json::Value]) -> Arc<FakeHost> {
        Arc::new(FakeHost {
            effects: vec![jsonl(lines)],
            ..FakeHost::default()
        })
    }
}

fn jsonl(lines: &[serde_json::Value]) -> Vec<u8> {
    let mut s = String::new();
    for l in lines {
        s.push_str(&l.to_string());
        s.push('\n');
    }
    s.into_bytes()
}

impl ScriptHost for FakeHost {
    fn run(&self, job: &ScriptJob) -> std::io::Result<ScriptOutcome> {
        let i = {
            let mut c = self.calls.lock().unwrap();
            *c += 1;
            *c - 1
        };
        if let Some(env) = &self.env {
            let path = env.get("Path").unwrap().map(|v| v.value);
            self.path_seen.lock().unwrap().push(path);
        }
        if let Some(bytes) = self.effects.get(i) {
            std::fs::write(job.workdir.join("effects.jsonl"), bytes)?;
        }
        Ok(ScriptOutcome {
            code: 0,
            lines: Vec::new(),
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
        let installed = self.state.get("app").unwrap().map(|(r, _)| InstalledInfo {
            version: r.version,
            old_manifest: None,
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
            manifest_sha256: "",
        })
    }

    /// Plan of version 1.0 with a `post_install` hook.
    fn plan(&self) -> InstallPlan {
        self.try_plan(&self.manifest("1.0", serde_json::json!({"post_install": "info 'x'"})))
            .unwrap()
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

    fn uninstall(&self) {
        let up = UninstallPlan::from_state(&self.state, "app", false)
            .unwrap()
            .unwrap();
        let mut ctx = self.ctx(&NoHook, None);
        wingetrs::install::executor::execute_uninstall(&up, &mut ctx).unwrap();
    }

    fn snapshot(&self) -> std::collections::BTreeMap<String, String> {
        let mut s = wingetrs::install::snapshot::snapshot(self.layout.root()).unwrap();
        s.retain(|p, _| !p.starts_with("blobs") && !p.starts_with("tmp"));
        s
    }

    fn path_var(&self) -> String {
        self.env.get("Path").unwrap().unwrap().value
    }

    fn var(&self, name: &str) -> Option<String> {
        self.env.get(name).unwrap().map(|v| v.value)
    }

    fn app_dir(&self) -> String {
        self.layout.app_dir("app").display().to_string()
    }

    fn vdir(&self) -> String {
        self.layout.version_dir("app", "1.0").display().to_string()
    }

    fn artifacts(&self) -> Vec<Artifact> {
        self.state
            .get("app")
            .unwrap()
            .map(|(_, a)| a)
            .unwrap_or_default()
    }

    fn shim_exists(&self, name: &str) -> bool {
        self.layout.shims().join(format!("{name}.exe")).exists()
    }

    /// Records `other` as installed, owning `artifacts`.
    fn record_other(&mut self, artifacts: &[Artifact]) {
        self.state
            .record(
                &AppRecord {
                    app: "other".into(),
                    bucket: "main".into(),
                    version: "1.0".into(),
                    arch: None,
                    installed_at: 0,
                    manifest_sha256: String::new(),
                },
                artifacts,
            )
            .unwrap();
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

fn add_path(var: &str, entries: &[&str]) -> serde_json::Value {
    serde_json::json!({"op": "add_path", "var": var, "entries": entries, "prepend": true, "force": false})
}

fn remove_path(var: &str, entries: &[&str]) -> serde_json::Value {
    serde_json::json!({"op": "remove_path", "var": var, "entries": entries})
}

fn set_env(name: &str, value: Option<&str>) -> serde_json::Value {
    serde_json::json!({"op": "set_env", "name": name, "value": value})
}

fn shim(path: &str, name: &str) -> serde_json::Value {
    serde_json::json!({"op": "shim", "path": path, "name": name, "args": null})
}

fn rm_shim(name: &str) -> serde_json::Value {
    serde_json::json!({"op": "rm_shim", "name": name})
}

fn shortcut(target: &str, name: &str, icon: Option<&str>) -> serde_json::Value {
    serde_json::json!({"op": "shortcut", "target": target, "name": name, "args": null, "icon": icon})
}

/// Runs a plan whose hook writes `lines`; expects a failure containing `needle`, and nothing
/// left behind (no app folder, PATH and shortcuts as before).
async fn refused(w: &World, lines: &[serde_json::Value], needle: &str) {
    let before_env = w.env.dump();
    let before_sc = w.shortcuts.dump();
    let host = FakeHost::writing(lines);
    let e = w.run(&w.plan(), &host).await.unwrap_err().to_string();
    assert!(e.contains(needle), "{e}");
    assert!(!w.layout.app_dir("app").exists(), "{e}");
    assert_eq!(w.env.dump(), before_env, "{e}");
    assert_eq!(w.shortcuts.dump(), before_sc, "{e}");
    assert!(w.state.apps().unwrap().is_empty());
}

#[tokio::test]
async fn add_path_inside_app_applied_and_recorded() {
    let w = setup().await;
    let entry = format!(r"{}\current\bin", w.app_dir());
    let host = FakeHost::writing(&[add_path("PATH", &[&entry])]);
    w.run(&w.plan(), &host).await.unwrap();
    assert!(
        w.path_var().split(';').any(|s| s == entry),
        "{}",
        w.path_var()
    );
    assert!(
        w.artifacts()
            .iter()
            .any(|a| a.kind == ArtifactKind::Path && a.target == entry)
    );
    w.uninstall();
    assert!(!w.path_var().contains(&entry), "{}", w.path_var());
    assert_eq!(w.path_var(), ORIGINAL_PATH);
}

#[tokio::test]
async fn add_path_inside_persist_applied() {
    let w = setup().await;
    let entry = format!(r"{}\bin", w.layout.persist_dir("app").display());
    let host = FakeHost::writing(&[add_path("Path", &[&entry])]);
    w.run(&w.plan(), &host).await.unwrap();
    assert!(
        w.path_var().split(';').any(|s| s == entry),
        "{}",
        w.path_var()
    );
}

#[tokio::test]
async fn add_path_outside_app_rolls_back() {
    let w = setup().await;
    refused(
        &w,
        &[add_path("PATH", &["C:\\Windows"])],
        "fora da pasta do app",
    )
    .await;
    assert_eq!(w.path_var(), ORIGINAL_PATH);
}

#[tokio::test]
async fn add_path_unsafe_entries_refused() {
    let w = setup().await;
    let app = w.app_dir();
    for entry in [
        format!(r"{app}\..\other"),
        format!(r"{app}\bin;C:\Windows"),
        format!(r"{app}\%EVIL%"),
        format!(r"{app}\bin:ads"),
        format!(r"{app}\NUL"),
        format!(r"{app}x\bin"),
        format!(r"{}\apps", w.layout.root().display()),
    ] {
        refused(&w, &[add_path("PATH", &[&entry])], "fora da pasta do app").await;
    }
}

#[tokio::test]
async fn set_env_applied_restored_on_rollback() {
    let probe = setup().await;
    let n = probe.plan().steps.len();
    let w = setup().await;
    w.env
        .set(
            "PROTO_VERSION_CHECK",
            &EnvValue {
                value: "antigo".into(),
                expand: false,
            },
        )
        .unwrap();
    let host = FakeHost::writing(&[set_env("PROTO_VERSION_CHECK", Some("v"))]);
    let plan = w.plan();
    let hook_at = plan
        .steps
        .iter()
        .position(|s| matches!(s, wingetrs::install::plan::Step::Hook { .. }))
        .unwrap();
    assert!(hook_at + 1 < n);
    let scripts: Arc<dyn ScriptHost> = host.clone();
    let mut ctx = w.ctx(&FailAt(hook_at + 1), Some(scripts));
    let err = wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap_err();
    drop(ctx);
    assert!(err.to_string().contains("injected"), "{err}");
    assert_eq!(w.var("PROTO_VERSION_CHECK").as_deref(), Some("antigo"));
}

#[tokio::test]
async fn set_env_applied_recorded_and_removed() {
    let w = setup().await;
    w.env
        .set(
            "RIPCORD_ALLOW_UPDATES",
            &EnvValue {
                value: "1".into(),
                expand: false,
            },
        )
        .unwrap();
    let host = FakeHost::writing(&[
        set_env("PROTO_VERSION_CHECK", Some("v")),
        set_env("RIPCORD_ALLOW_UPDATES", None),
    ]);
    w.run(&w.plan(), &host).await.unwrap();
    assert_eq!(w.var("PROTO_VERSION_CHECK").as_deref(), Some("v"));
    assert_eq!(w.var("RIPCORD_ALLOW_UPDATES"), None);
    let arts = w.artifacts();
    let envs: Vec<&Artifact> = arts
        .iter()
        .filter(|a| a.kind == ArtifactKind::Env)
        .collect();
    assert_eq!(envs.len(), 3, "{envs:?}");
    w.uninstall();
    assert_eq!(w.var("PROTO_VERSION_CHECK"), None);
    assert_eq!(w.var("RIPCORD_ALLOW_UPDATES").as_deref(), Some("1"));
}

#[tokio::test]
async fn set_env_path_refused() {
    let w = setup().await;
    for name in ["Path", "PATH", "path"] {
        refused(
            &w,
            &[set_env(name, Some("C:\\x"))],
            "o script tentou gravar o PATH inteiro; use Add-Path",
        )
        .await;
    }
}

#[tokio::test]
async fn set_env_denied_var_refused() {
    let w = setup().await;
    for name in ["PSModulePath", "psmodulepath", "ComSpec", "TEMP"] {
        refused(
            &w,
            &[set_env(name, Some("C:\\x"))],
            &format!("a variável {name} não pode ser gravada por script"),
        )
        .await;
    }
}

#[tokio::test]
async fn set_env_denied_exact_names_refused() {
    let w = setup().await;
    for name in [
        "TMP",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "HOME",
        "HOMEDRIVE",
        "HOMEPATH",
        "SystemRoot",
        "WINDIR",
        "PSExecutionPolicyPreference",
        "__PSLockdownPolicy",
        "JAVA_TOOL_OPTIONS",
        "_java_options",
        "JDK_JAVA_OPTIONS",
        "BASH_ENV",
        "env",
        "HTTP_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "NO_PROXY",
        "REQUESTS_CA_BUNDLE",
        "CURL_CA_BUNDLE",
        "PHPRC",
        "CLASSPATH",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "GRADLE_USER_HOME",
    ] {
        refused(
            &w,
            &[set_env(name, Some("x"))],
            &format!("a variável {name} não pode ser gravada por script"),
        )
        .await;
    }
}

#[tokio::test]
async fn set_env_denied_families_refused() {
    let w = setup().await;
    for name in [
        "COR_PROFILER",
        "cor_enable_profiling",
        "CORECLR_PROFILER_PATH",
        "COMPlus_EnableDiagnostics",
        "DOTNET_STARTUP_HOOKS",
        "DOTNET_ROOT",
        "NODE_OPTIONS",
        "PYTHONSTARTUP",
        "PythonPath",
        "PERL5OPT",
        "RUBYOPT",
        "GIT_SSH_COMMAND",
        "LD_PRELOAD",
        "PSModuleAnalysisCachePath",
        "PROCESSOR_ARCHITECTURE",
        "WINGETRS_TEST_ENV_KEY",
        "SCOOP",
        "scoop_global",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "PIP_INDEX_URL",
        "NPM_CONFIG_REGISTRY",
        "LUA_INIT",
        "LUA_INIT_5_4",
        "GIT_INSTALL_ROOT",
        "NODE_PATH",
        "UV_INDEX_URL",
        "uv_extra_index_url",
        "UV_CACHE_DIR",
        "POETRY_REPOSITORIES_X_URL",
        "CONDA_CHANNELS",
        "BUN_INSTALL",
        "YARN_NPM_REGISTRY_SERVER",
        "GOPROXY",
        "GOPRIVATE",
        "GONOSUMDB",
        "GOFLAGS",
        "RUSTUP_DIST_SERVER",
        "ELECTRON_GET_USE_PROXY",
        "npm_config_registry",
    ] {
        refused(
            &w,
            &[set_env(name, Some("x"))],
            &format!("a variável {name} não pode ser gravada por script"),
        )
        .await;
    }
}

#[tokio::test]
async fn set_env_allowed_names_applied() {
    let w = setup().await;
    let inside = format!(r"{}\jdk", w.vdir());
    let names = [
        ("JAVA_HOME", inside.clone()),
        ("GOPATH", inside.clone()),
        ("ANDROID_SDK_ROOT", inside.clone()),
        ("PKG_CONFIG_PATH", format!(r"{inside};{}\lib", w.vdir())),
        (
            "OPENSSL_CONF",
            format!(r"{}\ssl.cnf", w.layout.persist_dir("app").display()),
        ),
        ("PROTO_VERSION_CHECK", "false".to_string()),
        ("codex_sparkle_enabled", "0".to_string()),
    ];
    let lines: Vec<serde_json::Value> = names.iter().map(|(n, v)| set_env(n, Some(v))).collect();
    let host = FakeHost::writing(&lines);
    w.run(&w.plan(), &host).await.unwrap();
    for (n, v) in &names {
        assert_eq!(w.var(n).as_deref(), Some(v.as_str()), "{n}");
    }
}

#[tokio::test]
async fn set_env_unlisted_name_refused() {
    let w = setup().await;
    for name in ["X", "VLC_X", "MY_TOOL_HOME", "TOOL_HOME"] {
        refused(
            &w,
            &[set_env(name, Some("v"))],
            &format!("a variável {name} não está entre as que um script pode gravar"),
        )
        .await;
    }
}

#[tokio::test]
async fn set_env_path_value_outside_app_refused() {
    let w = setup().await;
    let inside = format!(r"{}\jdk", w.vdir());
    for (name, value) in [
        ("JAVA_HOME", r"C:\qualquer".to_string()),
        ("JAVA_HOME", "jdk".to_string()),
        ("PKG_CONFIG_PATH", format!(r"{inside};C:\qualquer")),
        ("PROTO_VERSION_CHECK", r"C:\qualquer".to_string()),
        ("PROTO_VERSION_CHECK", "//server/share".to_string()),
    ] {
        refused(&w, &[set_env(name, Some(&value))], "fora da pasta do app").await;
    }
}

#[tokio::test]
async fn set_env_bad_name_refused() {
    let w = setup().await;
    for name in ["", "A=B", "A B", "1X", "A\u{0}"] {
        refused(&w, &[set_env(name, Some("v"))], "nome de variável inválido").await;
    }
}

#[tokio::test]
async fn psmodulepath_add_path_refused() {
    let w = setup().await;
    let entry = format!(r"{}\current\modules", w.app_dir());
    refused(
        &w,
        &[add_path("PSModulePath", &[&entry])],
        "Add-Path em PSModulePath não suportado",
    )
    .await;
    assert_eq!(w.var("PSModulePath"), None);
}

#[tokio::test]
async fn other_var_add_path_refused() {
    let w = setup().await;
    let entry = format!(r"{}\current", w.app_dir());
    refused(
        &w,
        &[add_path("PATHEXT", &[&entry])],
        "Add-Path em PATHEXT não suportado",
    )
    .await;
}

#[tokio::test]
async fn remove_path_only_own_entries() {
    let mut w = setup().await;
    let other = format!(r"{}\apps\other\bin", w.layout.root().display());
    w.env
        .set(
            "Path",
            &EnvValue {
                value: format!("{ORIGINAL_PATH};{other}"),
                expand: true,
            },
        )
        .unwrap();
    w.record_other(&[Artifact {
        kind: ArtifactKind::Path,
        target: other.clone(),
        previous: None,
    }]);
    let current = format!(r"{}\current", w.app_dir());
    let host = FakeHost::writing(&[remove_path("PATH", &[&other, "C:\\Exist*"])]);
    w.run(&w.plan(), &host).await.unwrap();
    let path = w.path_var();
    assert!(path.split(';').any(|s| s == other), "{path}");
    assert!(path.split(';').any(|s| s == ORIGINAL_PATH), "{path}");
    assert!(path.split(';').any(|s| s == current), "{path}");
}

#[tokio::test]
async fn remove_path_own_entry_by_pattern_removed_and_unrecorded() {
    let w = setup().await;
    let current = format!(r"{}\current", w.app_dir());
    let pattern = format!(r"{}\apps\APP\cur*", w.layout.root().display());
    let host = FakeHost::writing(&[remove_path("PATH", &[&pattern])]);
    w.run(&w.plan(), &host).await.unwrap();
    let path = w.path_var();
    assert!(!path.split(';').any(|s| s == current), "{path}");
    assert!(
        !w.artifacts()
            .iter()
            .any(|a| a.kind == ArtifactKind::Path && a.target == current)
    );
}

#[tokio::test]
async fn remove_path_entry_shared_with_other_app_kept() {
    let mut w = setup().await;
    let current = format!(r"{}\current", w.app_dir());
    w.record_other(&[Artifact {
        kind: ArtifactKind::Path,
        target: current.clone(),
        previous: None,
    }]);
    let host = FakeHost::writing(&[remove_path("PATH", &[&current])]);
    w.run(&w.plan(), &host).await.unwrap();
    assert!(w.path_var().split(';').any(|s| s == current));
    assert!(
        !w.artifacts()
            .iter()
            .any(|a| a.kind == ArtifactKind::Path && a.target == current)
    );
}

#[tokio::test]
async fn remove_path_of_previous_record_removed_before_the_record() {
    let w = setup().await;
    let bin2 = format!(r"{}\current\bin2", w.app_dir());
    let first = w
        .try_plan(&w.manifest("1.0", serde_json::json!({"env_add_path": ["bin2"]})))
        .unwrap();
    w.run(&first, &Arc::new(FakeHost::default())).await.unwrap();
    assert!(w.path_var().split(';').any(|s| s == bin2));
    let upgrade = w
        .try_plan(&w.manifest(
            "2.0",
            serde_json::json!({"pre_install": "info 'a'", "post_install": "info 'b'"}),
        ))
        .unwrap();
    let host = Arc::new(FakeHost {
        effects: vec![jsonl(&[remove_path("PATH", &[&bin2])])],
        env: Some(w.env.clone()),
        ..FakeHost::default()
    });
    w.run(&upgrade, &host).await.unwrap();
    let seen = host.path_seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].as_deref().unwrap().contains(&bin2));
    // Removed by the pre_install effect, before the record and the leftover cleanup.
    assert!(!seen[1].as_deref().unwrap().contains(&bin2), "{seen:?}");
}

#[tokio::test]
async fn remove_path_other_var_ignored() {
    let w = setup().await;
    let entry = format!(r"{}\current", w.app_dir());
    w.env
        .set(
            "PSModulePath",
            &EnvValue {
                value: entry.clone(),
                expand: false,
            },
        )
        .unwrap();
    let host = FakeHost::writing(&[remove_path("PSModulePath", &[&entry])]);
    w.run(&w.plan(), &host).await.unwrap();
    assert_eq!(w.var("PSModulePath").unwrap(), entry);
}

#[tokio::test]
async fn remove_path_pattern_too_long_refused() {
    let w = setup().await;
    let long = "a".repeat(5000);
    refused(
        &w,
        &[remove_path("PATH", &[&long])],
        "padrão de Remove-Path longo demais",
    )
    .await;
}

#[tokio::test]
async fn shim_effect_created_and_removed_on_uninstall() {
    let w = setup().await;
    let target = format!(r"{}\setup.exe", w.vdir());
    let host = FakeHost::writing(&[shim(&target, "setup2")]);
    w.run(&w.plan(), &host).await.unwrap();
    assert!(w.shim_exists("setup2"));
    assert!(
        w.artifacts()
            .iter()
            .any(|a| a.kind == ArtifactKind::Shim && a.target == "setup2")
    );
    w.uninstall();
    assert!(!w.shim_exists("setup2"));
}

#[tokio::test]
async fn shim_effect_outside_app_or_bad_name_refused() {
    let w = setup().await;
    let target = format!(r"{}\setup.exe", w.vdir());
    refused(
        &w,
        &[shim(r"C:\Windows\System32\cmd.exe", "cmd2")],
        "fora da pasta do app",
    )
    .await;
    for name in [r"..\evil", "a/b", "NUL", ""] {
        refused(&w, &[shim(&target, name)], "nome inseguro de comando").await;
    }
    assert!(!w.shim_exists("cmd2"));
}

#[tokio::test]
async fn rm_shim_of_other_app_ignored() {
    let mut w = setup().await;
    std::fs::create_dir_all(w.layout.shims()).unwrap();
    std::fs::write(w.layout.shims().join("other.exe"), b"x").unwrap();
    w.record_other(&[Artifact {
        kind: ArtifactKind::Shim,
        target: "other".into(),
        previous: None,
    }]);
    let host = FakeHost::writing(&[rm_shim("other")]);
    w.run(&w.plan(), &host).await.unwrap();
    assert!(w.shim_exists("other"));
}

#[tokio::test]
async fn rm_shim_own_removed_and_unrecorded() {
    let w = setup().await;
    let host = FakeHost::writing(&[rm_shim("tool")]);
    w.run(&w.plan(), &host).await.unwrap();
    assert!(!w.shim_exists("tool"));
    assert!(
        !w.artifacts()
            .iter()
            .any(|a| a.kind == ArtifactKind::Shim && a.target == "tool")
    );
}

#[tokio::test]
async fn rm_shim_bad_name_refused() {
    let w = setup().await;
    refused(&w, &[rm_shim(r"..\x")], "nome inseguro de comando").await;
}

#[tokio::test]
async fn shortcut_effect_created_and_removed() {
    let w = setup().await;
    let target = format!(r"{}\setup.exe", w.vdir());
    let host = FakeHost::writing(&[shortcut(&target, r"Sub\Setup", None)]);
    w.run(&w.plan(), &host).await.unwrap();
    let link = w.shortcuts.root().join("Sub").join("Setup.lnk");
    assert!(
        w.shortcuts.dump().contains_key(&link),
        "{:?}",
        w.shortcuts.dump()
    );
    assert!(
        w.artifacts()
            .iter()
            .any(|a| a.kind == ArtifactKind::Shortcut && a.target == link.display().to_string())
    );
    w.uninstall();
    assert!(!w.shortcuts.dump().contains_key(&link));
}

#[tokio::test]
async fn shortcut_effect_unsafe_refused() {
    let w = setup().await;
    let target = format!(r"{}\setup.exe", w.vdir());
    refused(
        &w,
        &[shortcut(r"C:\Windows\notepad.exe", "N", None)],
        "fora da pasta do app",
    )
    .await;
    refused(
        &w,
        &[shortcut(&target, "N", Some(r"C:\Windows\x.ico"))],
        "fora da pasta do app",
    )
    .await;
    for name in [r"..\..\Startup\x", r"C:\x", "a:b", ""] {
        refused(
            &w,
            &[shortcut(&target, name, None)],
            "nome inseguro de atalho",
        )
        .await;
    }
    assert!(!w.t.path().join("Startup").exists());
}

#[tokio::test]
async fn unreadable_effects_rolls_back() {
    let w = setup().await;
    let before = w.snapshot();
    let host = Arc::new(FakeHost {
        effects: vec![b"lixo".to_vec()],
        ..FakeHost::default()
    });
    let e = w.run(&w.plan(), &host).await.unwrap_err().to_string();
    assert!(e.contains("ilegíveis"), "{e}");
    assert!(!w.layout.app_dir("app").exists());
    assert_eq!(w.snapshot(), before);
}

#[tokio::test]
async fn effects_not_utf8_rolls_back() {
    let w = setup().await;
    let host = Arc::new(FakeHost {
        effects: vec![b"\xff\xfe\n".to_vec()],
        ..FakeHost::default()
    });
    let e = w.run(&w.plan(), &host).await.unwrap_err().to_string();
    assert!(e.contains("efeitos do script ilegíveis"), "{e}");
    assert!(!w.layout.app_dir("app").exists());
}

#[tokio::test]
async fn effects_over_64_mib_refused_by_size_check_and_bounded_read() {
    let w = setup().await;
    let host = Arc::new(FakeHost {
        effects: vec![vec![b'\n'; 64 * 1024 * 1024 + 1]],
        ..FakeHost::default()
    });
    let e = w.run(&w.plan(), &host).await.unwrap_err().to_string();
    assert!(e.contains("efeitos do script ilegíveis"), "{e}");
    assert!(e.contains("64 MiB"), "{e}");
    assert!(!w.layout.app_dir("app").exists());
}

#[tokio::test]
async fn refusal_after_applied_effects_restores_everything() {
    let w = setup().await;
    let before_fs = w.snapshot();
    let entry = format!(r"{}\current\bin", w.app_dir());
    let target = format!(r"{}\setup.exe", w.vdir());
    refused(
        &w,
        &[
            set_env("PROTO_VERSION_CHECK", Some("v")),
            add_path("PATH", &[&entry]),
            shim(&target, "setup2"),
            shortcut(&target, "S", None),
            add_path("PATH", &["C:\\Windows"]),
        ],
        "fora da pasta do app",
    )
    .await;
    assert_eq!(w.snapshot(), before_fs);
}

#[tokio::test]
async fn failure_at_each_step_with_effects() {
    let probe = setup().await;
    let n = probe.plan().steps.len();
    for k in 0..n {
        let s = setup().await;
        let plan = s.plan();
        let entry = format!(r"{}\current\bin", s.app_dir());
        let target = format!(r"{}\setup.exe", s.vdir());
        let host = FakeHost::writing(&[
            add_path("PATH", &[&entry]),
            set_env("PROTO_VERSION_CHECK", Some("v")),
            shim(&target, "setup2"),
        ]);
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
    }
}

#[tokio::test]
async fn variant_hook_refused_at_planning() {
    let w = setup().await;
    let r = w.try_plan(&w.manifest(
        "1.0",
        serde_json::json!({"architecture": {"64bit": {"post_install": "scoop install x"}}}),
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
async fn remove_path_like_wildcards() {
    let root = setup().await.layout.root().display().to_string();
    for (pattern, removed) in [
        (format!(r"{root}\apps\app\curre?t"), true),
        (format!(r"{root}\apps\[a-c]pp\current"), true),
        (format!(r"{root}\apps\[x-z]pp\current"), false),
        (format!(r"{root}\apps\a`pp\current"), true),
        (format!(r"{root}\*\current"), true),
        (format!(r"{root}\apps\app\current?"), false),
    ] {
        let w = setup().await;
        let pattern = pattern.replace(&root, &w.layout.root().display().to_string());
        let current = format!(r"{}\current", w.app_dir());
        let host = FakeHost::writing(&[remove_path("PATH", &[&pattern])]);
        w.run(&w.plan(), &host).await.unwrap();
        let still = w.path_var().split(';').any(|s| s == current);
        assert_eq!(!still, removed, "{pattern}");
    }
}

#[tokio::test]
async fn remove_path_own_entry_outside_root_ignored() {
    let w = setup().await;
    let first = w
        .try_plan(&w.manifest("1.0", serde_json::json!({})))
        .unwrap();
    w.run(&first, &Arc::new(FakeHost::default())).await.unwrap();
    // A record listing an entry outside the root (hand-edited or older format).
    let (rec, mut arts) = w.state.get("app").unwrap().unwrap();
    arts.push(Artifact {
        kind: ArtifactKind::Path,
        target: ORIGINAL_PATH.into(),
        previous: None,
    });
    let mut state = StateDb::open(&w.state_path).unwrap();
    state.record(&rec, &arts).unwrap();
    let upgrade = w
        .try_plan(&w.manifest(
            "2.0",
            serde_json::json!({"pre_install": "info 'a'", "post_install": "info 'b'"}),
        ))
        .unwrap();
    let host = Arc::new(FakeHost {
        effects: vec![jsonl(&[remove_path("PATH", &[ORIGINAL_PATH])])],
        env: Some(w.env.clone()),
        ..FakeHost::default()
    });
    w.run(&upgrade, &host).await.unwrap();
    let seen = host.path_seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert!(
        seen[1]
            .as_deref()
            .unwrap()
            .split(';')
            .any(|s| s == ORIGINAL_PATH),
        "{seen:?}"
    );
}

const USER_PATHEXT: &str = ".COM;.EXE;.BAT";

fn set_user_pathext(w: &World) {
    w.env
        .set(
            "PATHEXT",
            &EnvValue {
                value: USER_PATHEXT.into(),
                expand: false,
            },
        )
        .unwrap();
}

#[tokio::test]
async fn pathext_append_accepted_and_restored_on_uninstall() {
    let w = setup().await;
    set_user_pathext(&w);
    let value = format!("{USER_PATHEXT};.PY;.PYW");
    let host = FakeHost::writing(&[set_env("PATHEXT", Some(&value))]);
    w.run(&w.plan(), &host).await.unwrap();
    assert_eq!(w.var("PATHEXT").unwrap(), value);
    w.uninstall();
    assert_eq!(w.var("PATHEXT").unwrap(), USER_PATHEXT);
}

#[tokio::test]
async fn pathext_append_undone_on_rollback() {
    let w = setup().await;
    set_user_pathext(&w);
    let value = format!("{USER_PATHEXT};.PY");
    let host = FakeHost::writing(&[set_env("PATHEXT", Some(&value))]);
    let plan = w.plan();
    let hook_at = plan
        .steps
        .iter()
        .position(|s| matches!(s, wingetrs::install::plan::Step::Hook { .. }))
        .unwrap();
    let scripts: Arc<dyn ScriptHost> = host.clone();
    let mut ctx = w.ctx(&FailAt(hook_at + 1), Some(scripts));
    let err = wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap_err();
    drop(ctx);
    assert!(err.to_string().contains("injected"), "{err}");
    assert_eq!(w.var("PATHEXT").unwrap(), USER_PATHEXT);
}

#[tokio::test]
async fn pathext_other_forms_refused() {
    let w = setup().await;
    set_user_pathext(&w);
    let refuse = "o script só pode acrescentar extensões no fim do PATHEXT";
    for value in [
        Some(".COM;.BAT;.PY".to_string()),
        Some(format!("{USER_PATHEXT};.EXE")),
        Some(format!("{USER_PATHEXT};.PY;.py")),
        Some(".EXE;.COM;.BAT;.PY".to_string()),
        Some(format!("{USER_PATHEXT};.P")),
        Some(format!("{USER_PATHEXT};.PYTHON")),
        Some(format!("{USER_PATHEXT};.P1")),
        Some(format!("{USER_PATHEXT};PY")),
        Some("%PATHEXT%;.PY".to_string()),
        None,
    ] {
        refused(&w, &[set_env("PATHEXT", value.as_deref())], refuse).await;
        assert_eq!(w.var("PATHEXT").unwrap(), USER_PATHEXT);
    }
}

#[tokio::test]
async fn pathext_without_user_value_refused() {
    let w = setup().await;
    refused(
        &w,
        &[set_env("PATHEXT", Some(";.PY;.PYW"))],
        "o script só pode acrescentar extensões no fim do PATHEXT",
    )
    .await;
    assert_eq!(w.var("PATHEXT"), None);
}

/// The PATHEXT lines of `main/python`'s `installer.script` (catalog text, 2026-10-18) sit
/// inside `if ($global)`: informational since Task 164, so they plan; without the guard
/// they are refused at planning for `-Global`. The same lines for the user (no `-Global`)
/// pass the gate, and the effect they would write is accepted.
#[tokio::test]
async fn python_catalog_pathext_script() {
    let w = setup().await;
    set_user_pathext(&w);
    let lines = [
        "    $pathext = (Get-EnvVar -Name PATHEXT -Global) -replace ';.PYW?', ''",
        "    Set-EnvVar -Name PATHEXT -Value \"$pathext;.PY;.PYW\" -Global",
    ];
    let catalog = serde_json::json!(["if ($global) {", lines[0], lines[1], "}"]);
    w.try_plan(&w.manifest("1.0", serde_json::json!({"installer": {"script": catalog}})))
        .unwrap();
    let unguarded = serde_json::json!(lines);
    match w.try_plan(&w.manifest(
        "1.0",
        serde_json::json!({"installer": {"script": unguarded}}),
    )) {
        Err(PlanError::ScriptRefused { reasons, .. }) => {
            assert!(reasons.iter().all(|r| r.contains("-Global")), "{reasons:?}")
        }
        other => panic!("{other:?}"),
    }
    let user = serde_json::json!([
        "$pathext = (Get-EnvVar -Name PATHEXT) -replace ';.PYW?', ''",
        "Set-EnvVar -Name PATHEXT -Value \"$pathext;.PY;.PYW\""
    ]);
    let plan = w
        .try_plan(&w.manifest("1.0", serde_json::json!({"installer": {"script": user}})))
        .unwrap();
    let value = format!("{USER_PATHEXT};.PY;.PYW");
    let host = FakeHost::writing(&[set_env("PATHEXT", Some(&value))]);
    w.run(&plan, &host).await.unwrap();
    assert_eq!(w.var("PATHEXT").unwrap(), value);
}

#[tokio::test]
async fn url_var_https_accepted() {
    let w = setup().await;
    let value = "https://npmmirror.com/mirrors/electron/";
    let host = FakeHost::writing(&[set_env("electron_mirror", Some(value))]);
    w.run(&w.plan(), &host).await.unwrap();
    assert_eq!(w.var("electron_mirror").as_deref(), Some(value));
}

#[tokio::test]
async fn url_like_names_off_the_list_refused() {
    let w = setup().await;
    for name in ["TOOL_DOWNLOAD_URL", "TOOL_MIRROR"] {
        refused(
            &w,
            &[set_env(name, Some("https://example.com/"))],
            &format!("a variável {name} não está entre as que um script pode gravar"),
        )
        .await;
    }
}

#[tokio::test]
async fn url_var_not_https_refused() {
    let w = setup().await;
    for value in [
        "http://example.com/",
        "file:///C:/x",
        r"C:\x",
        "https://",
        "https://a b",
        r"https://x;C:\y",
        "https://%EVIL%",
        r"https://x\y",
    ] {
        refused(
            &w,
            &[set_env("ELECTRON_MIRROR", Some(value))],
            "só endereço https é aceito",
        )
        .await;
    }
}
