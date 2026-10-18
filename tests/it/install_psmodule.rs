#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 145): native `psmodule` (junction + `PSModulePath`).
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::executor::{ExecCtx, NoHook, StepHook};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::plan::{InstallPlan, Step, UninstallPlan};
use wingetrs::install::planner::{InstalledInfo, PlanInput, plan_install};
use wingetrs::install::state::StateDb;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{Effects, EnvStore, EnvValue};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const VAR: &str = "PSModulePath";

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

fn zip_bytes() -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        z.start_file("pkg/tool.exe", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(&fake_pe()).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn no_progress(_: &wingetrs::install::progress::ProgressEvent) {}
fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

struct World {
    _server: MockServer,
    _t: TempDir,
    layout: Layout,
    data: DataDir,
    env: FakeEnv,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    state: StateDb,
    state_path: PathBuf,
    http: Http,
    url: String,
    sha: String,
}

async fn setup() -> World {
    let server = MockServer::start().await;
    let t = tempfile::tempdir().unwrap();
    let layout = Layout::new(t.path().join("root"));
    std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
    std::fs::write(layout.launcher(), fake_pe()).unwrap();
    let zip = zip_bytes();
    let sha = hex::encode(Sha256::digest(&zip));
    Mock::given(method("GET"))
        .and(path("/pkg.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
        .mount(&server)
        .await;
    let url = format!("{}/pkg.zip", server.uri());
    let state_path = t.path().join("state").join("state.db");
    World {
        data: DataDir::new(layout.root()),
        shortcuts: FakeShortcuts::new(t.path().join("startmenu")),
        state: StateDb::open(&state_path).unwrap(),
        state_path,
        _server: server,
        _t: t,
        layout,
        env: FakeEnv::new(),
        trash: FakeTrash::new(),
        registry: FakeRegistry::new(),
        http: Http::new().unwrap(),
        url,
        sha,
    }
}

impl World {
    fn manifest(&self, version: &str, module: &str, shortcuts: &[&str], env_set: bool) -> String {
        let sc: Vec<_> = shortcuts
            .iter()
            .map(|n| serde_json::json!(["tool.exe", n]))
            .collect();
        let mut m = serde_json::json!({
            "version": version,
            "url": self.url,
            "hash": self.sha,
            "extract_dir": "pkg",
            "shortcuts": sc,
            "psmodule": {"name": module},
        });
        if env_set {
            m["env_set"] = serde_json::json!({"TOOL_HOME": "$dir"});
        }
        m.to_string()
    }

    fn plan(&self, app: &str, manifest: &str) -> InstallPlan {
        use wingetrs::install::traits::Shortcuts;
        let m = resolve(manifest, Arch::X64).unwrap();
        let installed = self
            .state
            .get(app)
            .unwrap()
            .map(|(r, _)| InstalledInfo { version: r.version });
        let root = self.shortcuts.root();
        plan_install(&PlanInput {
            app,
            bucket: "main",
            manifest: &m,
            layout: &self.layout,
            shortcut_root: &root,
            installed,
            launcher_present: true,
            keep_old: false,
            manifest_sha256: "",
        })
        .unwrap()
    }

    fn ctx_with_hook<H: StepHook + Clone + 'static>(&self, hook: &H) -> ExecCtx<'_> {
        let hook: &'static H = Box::leak(Box::new(hook.clone()));
        let state: &mut StateDb = Box::leak(Box::new(StateDb::open(&self.state_path).unwrap()));
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
            hook,
            progress: &no_progress,
            in_use: &nothing_in_use,
            msiexec: None,
            launch: Default::default(),
            runners: wingetrs::install::executor::Runners::disabled(),
        }
    }

    async fn try_install(
        &self,
        plan: &InstallPlan,
    ) -> Result<(), wingetrs::install::executor::ExecError> {
        let mut ctx = self.ctx_with_hook(&NoHook);
        wingetrs::install::executor::execute_install(plan, &mut ctx).await
    }

    async fn install(&self, plan: &InstallPlan) {
        self.try_install(plan).await.unwrap();
    }

    fn uninstall(&self, app: &str) {
        let up = UninstallPlan::from_state(&self.state, app, false)
            .unwrap()
            .unwrap();
        let mut ctx = self.ctx_with_hook(&NoHook);
        wingetrs::install::executor::execute_uninstall(&up, &mut ctx).unwrap();
    }

    fn set_var(&self, value: &str) {
        self.env
            .set(
                VAR,
                &EnvValue {
                    value: value.into(),
                    expand: false,
                },
            )
            .unwrap();
    }

    fn var(&self) -> Option<EnvValue> {
        self.env.get(VAR).unwrap()
    }

    fn modules(&self) -> String {
        self.layout.modules().display().to_string()
    }

    fn link_target(&self, module: &str) -> PathBuf {
        wingetrs::install::link::target(&self.layout.modules().join(module)).unwrap()
    }
}

#[derive(Clone)]
struct FailAt(usize);
impl StepHook for FailAt {
    fn before(&self, i: usize, _s: &Step) -> Result<(), wingetrs::install::executor::ExecError> {
        if i == self.0 {
            Err(wingetrs::install::executor::ExecError::Hook(format!(
                "injected at {i}"
            )))
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn installs_junction_to_current() {
    let s = setup().await;
    let plan = s.plan("x", &s.manifest("1.0", "PSFzf", &["A"], false));
    s.install(&plan).await;
    assert_eq!(s.link_target("PSFzf"), s.layout.current("x"));
}

#[tokio::test]
async fn psmodulepath_prepended_and_preserved() {
    let s = setup().await;
    s.set_var(r"C:\A;C:\B");
    let plan = s.plan("x", &s.manifest("1.0", "M", &["A"], false));
    s.install(&plan).await;
    assert_eq!(
        s.var().unwrap().value,
        format!(r"{};C:\A;C:\B", s.modules())
    );
}

#[tokio::test]
async fn psmodulepath_absent_seeds_documents() {
    let s = setup().await;
    let plan = s.plan("x", &s.manifest("1.0", "M", &["A"], false));
    s.install(&plan).await;
    let profile = std::env::var("USERPROFILE").unwrap();
    assert_eq!(
        s.var().unwrap().value,
        format!(
            r"{};{profile}\Documents\WindowsPowerShell\Modules",
            s.modules()
        )
    );
}

#[tokio::test]
async fn psmodulepath_with_entry_unchanged() {
    let s = setup().await;
    let initial = format!(r"C:\Z;{}\", s.modules().to_uppercase());
    s.set_var(&initial);
    let before = s.var();
    let plan = s.plan("x", &s.manifest("1.0", "M", &["A"], false));
    s.install(&plan).await;
    assert_eq!(s.var(), before);
    assert_eq!(s.var().unwrap().value, initial);
}

#[tokio::test]
async fn rollback_restores_psmodulepath_and_removes_junction() {
    let s = setup().await;
    s.set_var(r"C:\Z");
    let plan = s.plan("x", &s.manifest("1.0", "M", &["A"], false));
    let k = plan
        .steps
        .iter()
        .position(|st| matches!(st, Step::PsModule { .. }))
        .unwrap();
    let mut ctx = s.ctx_with_hook(&FailAt(k + 1));
    wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap_err();
    assert_eq!(s.var().unwrap().value, r"C:\Z");
    assert!(std::fs::symlink_metadata(s.layout.modules().join("M")).is_err());
}

#[tokio::test]
async fn last_module_removes_entry_only_then() {
    let s = setup().await;
    s.set_var(r"C:\A");
    let a = s.plan("a", &s.manifest("1.0", "ModA", &["SA"], false));
    s.install(&a).await;
    let b = s.plan("b", &s.manifest("1.0", "ModB", &["SB"], false));
    s.install(&b).await;
    let both = s.var().unwrap().value;
    assert_eq!(both, format!(r"{};C:\A", s.modules()));
    s.uninstall("a");
    assert_eq!(s.var().unwrap().value, both, "B still has a module");
    assert!(std::fs::symlink_metadata(s.layout.modules().join("ModA")).is_err());
    s.uninstall("b");
    assert_eq!(s.var().unwrap().value, r"C:\A");
}

#[tokio::test]
async fn module_of_other_app_refused() {
    let s = setup().await;
    let a = s.plan("a", &s.manifest("1.0", "Same", &["SA"], false));
    s.install(&a).await;
    let b = s.plan("b", &s.manifest("1.0", "Same", &["SB"], false));
    let e = s.try_install(&b).await.unwrap_err();
    assert!(e.to_string().contains("pertence a a"), "{e}");
    assert_eq!(s.link_target("Same"), s.layout.current("a"));
}

#[tokio::test]
async fn foreign_folder_is_not_replaced() {
    let s = setup().await;
    let dir = s.layout.modules().join("Mine");
    std::fs::create_dir_all(&dir).unwrap();
    let plan = s.plan("x", &s.manifest("1.0", "Mine", &["A"], false));
    let e = s.try_install(&plan).await.unwrap_err();
    assert!(e.to_string().contains("não é do wingetrs"), "{e}");
    assert!(dir.is_dir());
}

#[tokio::test]
async fn upgrade_recreates_the_junction() {
    let s = setup().await;
    s.install(&s.plan("x", &s.manifest("1.0", "M", &["A"], false)))
        .await;
    let v2 = s.plan("x", &s.manifest("2.0", "M", &["A"], false));
    s.install(&v2).await;
    assert_eq!(s.link_target("M"), s.layout.current("x"));
}

#[tokio::test]
async fn step_order() {
    let s = setup().await;
    let plan = s.plan("x", &s.manifest("1.0", "M", &["A", "B"], true));
    let pos = |f: &dyn Fn(&Step) -> bool| plan.steps.iter().position(f).unwrap();
    let last_shortcut = plan
        .steps
        .iter()
        .rposition(|st| matches!(st, Step::Shortcut(_)))
        .unwrap();
    let psm = pos(&|st| matches!(st, Step::PsModule { .. }));
    let first_env = pos(&|st| matches!(st, Step::EnvSet { .. }));
    assert!(
        last_shortcut < psm && psm < first_env,
        "{last_shortcut} {psm} {first_env}"
    );
}

const UNSAFE_NAMES: [&str; 6] = [r"..\x", "a/b", "C:x", "CON", "x.", ""];

#[tokio::test]
async fn unsafe_module_names_refused_by_the_planner() {
    use wingetrs::install::manifest::PsModule;
    use wingetrs::install::traits::Shortcuts;
    let s = setup().await;
    let manifest = s.manifest("1.0", "ok", &["A"], false);
    let root = s.shortcuts.root();
    for name in UNSAFE_NAMES {
        let mut m = resolve(&manifest, Arch::X64).unwrap();
        m.psmodule = Some(PsModule { name: name.into() });
        let r = plan_install(&PlanInput {
            app: "x",
            bucket: "main",
            manifest: &m,
            layout: &s.layout,
            shortcut_root: &root,
            installed: None,
            launcher_present: true,
            keep_old: false,
            manifest_sha256: "",
        });
        assert!(r.is_err(), "{name:?} must be refused by the planner");
    }
}

#[tokio::test]
async fn unsafe_module_names_refused_by_the_executor() {
    let s = setup().await;
    let good = s.plan("x", &s.manifest("1.0", "ok", &["A"], false));
    for name in UNSAFE_NAMES {
        let mut plan = good.clone();
        for st in &mut plan.steps {
            if let Step::PsModule { name: n } = st {
                *n = name.to_string();
            }
        }
        s.try_install(&plan).await.unwrap_err();
        assert!(!s.layout.root().join("x").exists(), "{name:?} escaped");
        assert!(!s.layout.modules().exists(), "{name:?} created modules");
        assert!(s.var().is_none(), "{name:?} touched PSModulePath");
    }
}

#[tokio::test]
async fn renamed_module_on_upgrade_keeps_the_psmodulepath_entry() {
    let s = setup().await;
    s.set_var(r"C:\Z");
    s.install(&s.plan("x", &s.manifest("1.0", "A", &["S"], false)))
        .await;
    s.install(&s.plan("x", &s.manifest("2.0", "B", &["S"], false)))
        .await;
    assert_eq!(s.link_target("B"), s.layout.current("x"));
    assert!(std::fs::symlink_metadata(s.layout.modules().join("A")).is_err());
    assert_eq!(s.var().unwrap().value, format!(r"{};C:\Z", s.modules()));
}

#[tokio::test]
async fn expand_flag_and_literal_survive_install_and_removal() {
    let s = setup().await;
    let initial = r"%USERPROFILE%\x";
    s.env
        .set(
            VAR,
            &EnvValue {
                value: initial.into(),
                expand: true,
            },
        )
        .unwrap();
    s.install(&s.plan("a", &s.manifest("1.0", "ModA", &["SA"], false)))
        .await;
    s.install(&s.plan("b", &s.manifest("1.0", "ModB", &["SB"], false)))
        .await;
    let v = s.var().unwrap();
    assert!(v.expand, "install keeps REG_EXPAND_SZ");
    assert_eq!(v.value, format!(r"{};{initial}", s.modules()));
    s.uninstall("a");
    assert!(s.var().unwrap().expand);
    s.uninstall("b");
    assert_eq!(
        s.var(),
        Some(EnvValue {
            value: initial.into(),
            expand: true
        }),
        "removal keeps REG_EXPAND_SZ and the literal"
    );
}

#[tokio::test]
async fn upgrade_with_different_case_of_the_name_works() {
    let s = setup().await;
    s.install(&s.plan("x", &s.manifest("1.0", "psfzf", &["S"], false)))
        .await;
    s.install(&s.plan("x", &s.manifest("2.0", "PSFzf", &["S"], false)))
        .await;
    assert_eq!(s.link_target("PSFzf"), s.layout.current("x"));
}

#[tokio::test]
async fn failed_upgrade_keeps_the_previous_junction() {
    let s = setup().await;
    s.install(&s.plan("x", &s.manifest("1.0", "M", &["S"], false)))
        .await;
    let v2 = s.plan("x", &s.manifest("2.0", "M", &["S"], false));
    let k = v2
        .steps
        .iter()
        .position(|st| matches!(st, Step::PsModule { .. }))
        .unwrap();
    let mut ctx = s.ctx_with_hook(&FailAt(k + 1));
    wingetrs::install::executor::execute_install(&v2, &mut ctx)
        .await
        .unwrap_err();
    assert_eq!(s.link_target("M"), s.layout.current("x"));
    assert!(s.layout.current("x").join("tool.exe").exists());
    assert_eq!(s.state.get("x").unwrap().unwrap().0.version, "1.0");
}
