#![allow(clippy::unwrap_used, clippy::expect_used)]
// The literal test bodies `drop(ctx)` to end the borrow explicitly; ExecCtx has no Drop.
#![allow(clippy::drop_non_drop)]
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::executor::{ExecCtx, ExecError, NoHook, StepHook};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::plan::{InstallPlan, Step, UninstallPlan};
use wingetrs::install::planner::{InstalledInfo, PlanInput, plan_install};
use wingetrs::install::state::StateDb;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{Effects, EnvStore, EnvValue};
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
        ("pkg/conf/default.ini", b"[default]\r\n"),
    ])
}

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

struct ManifestOpts<'a> {
    version: &'a str,
    bin: serde_json::Value,
    shortcut: &'a str,
    persist: serde_json::Value,
}

fn default_opts() -> ManifestOpts<'static> {
    ManifestOpts {
        version: "1.0",
        bin: serde_json::json!("tool.exe"),
        shortcut: "Tool",
        persist: serde_json::json!(["conf"]),
    }
}

fn manifest_custom(url: &str, sha256: &str, o: &ManifestOpts<'_>) -> String {
    serde_json::json!({
        "version": o.version,
        "url": url,
        "hash": sha256,
        "extract_dir": "pkg",
        "bin": o.bin,
        "shortcuts": [["tool.exe", o.shortcut]],
        "persist": o.persist,
        "env_set": {"TOOL_HOME": "$dir"},
        "env_add_path": ["."],
    })
    .to_string()
}

fn manifest_json(url: &str, sha256: &str) -> String {
    manifest_custom(url, sha256, &default_opts())
}

fn no_progress(_: &str, _: u64, _: Option<u64>) {}
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
    state: StateDb,
    state_path: PathBuf,
    http: Http,
    plan: InstallPlan,
}

const ORIGINAL_PATH: &str = r"C:\Existing";

impl World {
    /// Serves `zip` at `/<name>` and returns (url, sha256).
    async fn serve(&self, name: &str, zip: Vec<u8>) -> (String, String) {
        let sha = sha256_hex(&zip);
        Mock::given(method("GET"))
            .and(path(format!("/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
            .mount(&self.server)
            .await;
        (format!("{}/{name}", self.server.uri()), sha)
    }

    fn plan_from(&self, app: &str, manifest: &str, keep_old: bool) -> InstallPlan {
        let m = resolve(manifest, Arch::X64).unwrap();
        let installed = self
            .state
            .get(app)
            .unwrap()
            .map(|(r, _)| InstalledInfo { version: r.version });
        let root = self.shortcuts_root();
        plan_install(&PlanInput {
            app,
            bucket: "main",
            manifest: &m,
            layout: &self.layout,
            shortcut_root: &root,
            installed,
            launcher_present: true,
            keep_old,
        })
        .unwrap()
    }

    fn shortcuts_root(&self) -> PathBuf {
        use wingetrs::install::traits::Shortcuts;
        self.shortcuts.root()
    }

    /// The hook is copied and leaked so a temporary (`&FailAt(k)`) can be passed.
    fn ctx_with_hook<H: StepHook + Clone + 'static>(&self, hook: &H) -> ExecCtx<'_> {
        let hook: &'static H = Box::leak(Box::new(hook.clone()));
        // A second connection to the same database: the executor needs `&mut`, the test keeps reading.
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
        }
    }

    fn uninstall_plan(&self, purge: bool) -> UninstallPlan {
        self.uninstall_plan_of("app", purge)
    }

    fn uninstall_plan_of(&self, app: &str, purge: bool) -> UninstallPlan {
        UninstallPlan::from_state(&self.state, app, purge)
            .unwrap()
            .unwrap()
    }

    fn snapshot(&self) -> std::collections::BTreeMap<String, String> {
        let mut s = wingetrs::install::snapshot::snapshot(self.layout.root()).unwrap();
        s.retain(|p, _| !p.starts_with("blobs") && !p.starts_with("tmp"));
        s
    }

    fn path_var(&self) -> String {
        self.env.get("Path").unwrap().unwrap().value
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
    let env = FakeEnv::new();
    env.set(
        "Path",
        &EnvValue {
            value: ORIGINAL_PATH.into(),
            expand: true,
        },
    )
    .unwrap();
    let shortcuts = FakeShortcuts::new(t.path().join("startmenu"));
    // state.db lives outside the snapshotted root: its pages change even when the content is restored.
    let state_path = t.path().join("state").join("state.db");
    let state = StateDb::open(&state_path).unwrap();
    let mut w = World {
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
        plan: InstallPlan {
            app: String::new(),
            bucket: String::new(),
            version: String::new(),
            arch: None,
            upgrade_from: None,
            root: PathBuf::new(),
            steps: Vec::new(),
        },
    };
    let (url, sha) = w.serve("pkg-1.0.zip", fixture_zip()).await;
    w.plan = w.plan_from("app", &manifest_json(&url, &sha), false);
    w
}

#[derive(Clone)]
struct FailAt(usize);
impl wingetrs::install::executor::StepHook for FailAt {
    fn before(
        &self,
        i: usize,
        _s: &wingetrs::install::plan::Step,
    ) -> Result<(), wingetrs::install::executor::ExecError> {
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
async fn failure_at_any_step_restores_the_initial_state_exactly() {
    let probe = setup().await;
    let n = probe.plan.steps.len();
    assert!(n >= 10, "plan must exercise every step kind: {n}");
    for k in 0..n {
        let s = setup().await; // fresh world each time
        let before_fs = wingetrs::install::snapshot::snapshot(s.layout.root()).unwrap();
        let before_env = s.env.dump();
        let before_sc = s.shortcuts.dump();
        let before_state = s.state.apps().unwrap();
        let mut ctx = s.ctx_with_hook(&FailAt(k));
        let err = wingetrs::install::executor::execute_install(&s.plan, &mut ctx)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains(&format!("injected at {k}")),
            "{err}"
        );
        drop(ctx);
        // blobs/ may keep the verified download (shared cache); everything else must be identical
        let mut after_fs = wingetrs::install::snapshot::snapshot(s.layout.root()).unwrap();
        let mut b = before_fs.clone();
        after_fs.retain(|p, _| !p.starts_with("blobs") && !p.starts_with("tmp"));
        b.retain(|p, _| !p.starts_with("blobs") && !p.starts_with("tmp"));
        assert_eq!(after_fs, b, "filesystem differs after failing at step {k}");
        assert_eq!(s.env.dump(), before_env, "env differs after step {k}");
        assert_eq!(
            s.shortcuts.dump(),
            before_sc,
            "shortcuts differ after step {k}"
        );
        assert_eq!(
            s.state.apps().unwrap(),
            before_state,
            "state differs after step {k}"
        );
    }
}

#[tokio::test]
async fn full_install_then_uninstall_leaves_nothing_but_persist() {
    let s = setup().await;
    let before_env = s.env.dump();
    let mut ctx = s.ctx_with_hook(&wingetrs::install::executor::NoHook);
    wingetrs::install::executor::execute_install(&s.plan, &mut ctx)
        .await
        .unwrap();
    assert!(s.layout.current("app").join("tool.exe").exists());
    assert!(
        s.layout
            .persist_dir("app")
            .join("conf/default.ini")
            .exists(),
        "persist moved out of the version dir"
    );
    assert_eq!(
        s.env.get_raw("TOOL_HOME").as_deref(),
        Some(s.layout.current("app").to_str().unwrap())
    );
    let up = s.uninstall_plan(false);
    wingetrs::install::executor::execute_uninstall(&up, &mut ctx).unwrap();
    assert!(!s.layout.app_dir("app").exists());
    assert!(
        s.layout.persist_dir("app").exists(),
        "persist kept without --purge"
    );
    assert_eq!(s.env.dump(), before_env);
    assert!(s.shortcuts.dump().is_empty());
}

async fn install(s: &World, plan: &InstallPlan) {
    let mut ctx = s.ctx_with_hook(&NoHook);
    wingetrs::install::executor::execute_install(plan, &mut ctx)
        .await
        .unwrap();
}

fn current_target(s: &World) -> String {
    std::fs::read_link(s.layout.current("app"))
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Second version of the app: different tool bytes and default config.
async fn v2_plan(s: &World, keep_old: bool) -> InstallPlan {
    let mut tool = fake_pe(3);
    tool.extend_from_slice(b"v2");
    let zip = zip_of(&[
        ("pkg/tool.exe", &tool),
        ("pkg/conf/default.ini", b"[default] v2\r\n"),
    ]);
    let (url, sha) = s.serve("pkg-2.0.zip", zip).await;
    let o = ManifestOpts {
        version: "2.0",
        ..default_opts()
    };
    s.plan_from("app", &manifest_custom(&url, &sha, &o), keep_old)
}

#[tokio::test]
async fn purge_removes_persist() {
    let s = setup().await;
    install(&s, &s.plan).await;
    assert!(s.layout.persist_dir("app").join("conf").exists());
    let mut ctx = s.ctx_with_hook(&NoHook);
    wingetrs::install::executor::execute_uninstall(&s.uninstall_plan(true), &mut ctx).unwrap();
    assert!(
        !s.layout.persist_dir("app").exists(),
        "--purge removes persist"
    );
    assert!(!s.layout.app_dir("app").exists());
    assert!(s.state.get("app").unwrap().is_none());
}

#[tokio::test]
async fn upgrade_keeps_user_data_switches_current_and_removes_v1() {
    let s = setup().await;
    install(&s, &s.plan).await;
    let user = s.layout.persist_dir("app").join("conf").join("user.ini");
    std::fs::write(&user, b"mine").unwrap();
    let p2 = v2_plan(&s, false).await;
    assert_eq!(p2.upgrade_from.as_deref(), Some("1.0"));
    install(&s, &p2).await;
    assert_eq!(std::fs::read(&user).unwrap(), b"mine", "user data intact");
    assert!(
        current_target(&s).ends_with("2.0"),
        "{}",
        current_target(&s)
    );
    let tool = std::fs::read(s.layout.current("app").join("tool.exe")).unwrap();
    assert!(tool.ends_with(b"v2"));
    // the v2 version dir sees the persisted data through the link
    assert_eq!(
        std::fs::read(s.layout.version_dir("app", "2.0").join("conf/user.ini")).unwrap(),
        b"mine"
    );
    assert!(!s.layout.version_dir("app", "1.0").exists(), "v1 removed");
    assert_eq!(s.state.get("app").unwrap().unwrap().0.version, "2.0");
}

#[tokio::test]
async fn upgrade_with_keep_old_keeps_v1() {
    let s = setup().await;
    install(&s, &s.plan).await;
    let p2 = v2_plan(&s, true).await;
    assert!(
        !p2.steps
            .iter()
            .any(|st| matches!(st, Step::RemoveOldVersion { .. }))
    );
    install(&s, &p2).await;
    assert!(
        s.layout.version_dir("app", "1.0").join("tool.exe").exists(),
        "v1 kept"
    );
    assert!(current_target(&s).ends_with("2.0"));
}

#[tokio::test]
async fn failure_during_upgrade_keeps_v1_current_and_user_data() {
    let probe = setup().await;
    install(&probe, &probe.plan).await;
    let n = v2_plan(&probe, false).await.steps.len();
    for k in 0..n {
        let s = setup().await;
        install(&s, &s.plan).await;
        let user = s.layout.persist_dir("app").join("conf").join("user.ini");
        std::fs::write(&user, b"mine").unwrap();
        let p2 = v2_plan(&s, false).await;
        if matches!(p2.steps[k], Step::RemoveOldVersion { .. }) {
            continue; // covered by remove_old_version_failure_is_only_a_warning
        }
        let before_fs = s.snapshot();
        let before_env = s.env.dump();
        let before_sc = s.shortcuts.dump();
        let before_state = s.state.get("app").unwrap();
        let mut ctx = s.ctx_with_hook(&FailAt(k));
        let err = wingetrs::install::executor::execute_install(&p2, &mut ctx)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains(&format!("injected at {k}")),
            "{err}"
        );
        drop(ctx);
        assert!(
            current_target(&s).ends_with("1.0"),
            "step {k}: {}",
            current_target(&s)
        );
        let tool = std::fs::read(s.layout.current("app").join("tool.exe")).unwrap();
        assert_eq!(tool, fake_pe(3), "v1 still works after failing at step {k}");
        assert_eq!(std::fs::read(&user).unwrap(), b"mine", "step {k}");
        assert_eq!(
            s.snapshot(),
            before_fs,
            "filesystem differs after failing at step {k}"
        );
        assert_eq!(s.env.dump(), before_env, "env differs after step {k}");
        assert_eq!(
            s.shortcuts.dump(),
            before_sc,
            "shortcuts differ after step {k}"
        );
        assert_eq!(
            s.state.get("app").unwrap(),
            before_state,
            "state differs after step {k}"
        );
    }
}

#[tokio::test]
async fn remove_old_version_failure_is_only_a_warning() {
    let s = setup().await;
    install(&s, &s.plan).await;
    let p2 = v2_plan(&s, false).await;
    let last = p2.steps.len() - 1;
    assert!(matches!(p2.steps[last], Step::RemoveOldVersion { .. }));
    let mut ctx = s.ctx_with_hook(&FailAt(last));
    wingetrs::install::executor::execute_install(&p2, &mut ctx)
        .await
        .unwrap();
    assert!(current_target(&s).ends_with("2.0"), "install not undone");
    assert_eq!(s.state.get("app").unwrap().unwrap().0.version, "2.0");
    assert!(
        s.layout.version_dir("app", "1.0").exists(),
        "old version left in place"
    );
}

#[tokio::test]
async fn env_set_over_existing_variable_is_restored_on_rollback_and_uninstall() {
    let previous = EnvValue {
        value: r"D:\old-tool".into(),
        expand: false,
    };
    // rollback
    let s = setup().await;
    s.env.set("TOOL_HOME", &previous).unwrap();
    let k = s
        .plan
        .steps
        .iter()
        .position(|st| matches!(st, Step::RecordState))
        .unwrap();
    let mut ctx = s.ctx_with_hook(&FailAt(k));
    wingetrs::install::executor::execute_install(&s.plan, &mut ctx)
        .await
        .unwrap_err();
    assert_eq!(s.env.get("TOOL_HOME").unwrap(), Some(previous.clone()));
    // uninstall
    let s = setup().await;
    s.env.set("TOOL_HOME", &previous).unwrap();
    install(&s, &s.plan).await;
    assert_eq!(
        s.env.get_raw("TOOL_HOME").as_deref(),
        Some(s.layout.current("app").to_str().unwrap())
    );
    let mut ctx = s.ctx_with_hook(&NoHook);
    wingetrs::install::executor::execute_uninstall(&s.uninstall_plan(false), &mut ctx).unwrap();
    assert_eq!(s.env.get("TOOL_HOME").unwrap(), Some(previous));
}

#[tokio::test]
async fn persist_dot_dir_in_zip_becomes_junction() {
    let s = setup().await;
    let zip = zip_of(&[("pkg/tool.exe", &fake_pe(3)), ("pkg/data.d/x.txt", b"x")]);
    let (url, sha) = s.serve("dot.zip", zip).await;
    let o = ManifestOpts {
        persist: serde_json::json!(["data.d"]),
        ..default_opts()
    };
    let plan = s.plan_from("app", &manifest_custom(&url, &sha, &o), false);
    install(&s, &plan).await;
    let link = s.layout.version_dir("app", "1.0").join("data.d");
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "a folder in the zip is linked as a junction, whatever the dot says"
    );
    assert_eq!(
        std::fs::read(s.layout.persist_dir("app").join("data.d/x.txt")).unwrap(),
        b"x"
    );
}

#[tokio::test]
async fn persist_missing_everywhere_creates_empty_dir_and_file() {
    let s = setup().await;
    let (url, sha) = s
        .serve("plain.zip", zip_of(&[("pkg/tool.exe", &fake_pe(3))]))
        .await;
    let o = ManifestOpts {
        persist: serde_json::json!(["logs", "settings.ini"]),
        ..default_opts()
    };
    let plan = s.plan_from("app", &manifest_custom(&url, &sha, &o), false);
    install(&s, &plan).await;
    let pd = s.layout.persist_dir("app");
    assert!(pd.join("logs").is_dir());
    assert_eq!(std::fs::read_dir(pd.join("logs")).unwrap().count(), 0);
    assert_eq!(std::fs::read(pd.join("settings.ini")).unwrap(), b"");
    let v = s.layout.version_dir("app", "1.0");
    assert!(
        std::fs::symlink_metadata(v.join("logs"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    // hardlink: a write through the version path lands in persist
    std::fs::write(v.join("settings.ini"), b"k=v").unwrap();
    assert_eq!(std::fs::read(pd.join("settings.ini")).unwrap(), b"k=v");
}

#[tokio::test]
async fn existing_persist_wins_over_the_version_copy() {
    let s = setup().await;
    let conf = s.layout.persist_dir("app").join("conf");
    std::fs::create_dir_all(&conf).unwrap();
    std::fs::write(conf.join("default.ini"), b"user edited").unwrap();
    install(&s, &s.plan).await;
    assert_eq!(
        std::fs::read(s.layout.current("app").join("conf/default.ini")).unwrap(),
        b"user edited"
    );
}

#[tokio::test]
async fn path_entry_present_before_is_not_removed_on_rollback() {
    let s = setup().await;
    let current = s.layout.current("app");
    let shims = s.layout.shims();
    let pre = format!("{ORIGINAL_PATH};{};{}", current.display(), shims.display());
    s.env
        .set(
            "Path",
            &EnvValue {
                value: pre.clone(),
                expand: true,
            },
        )
        .unwrap();
    let k = s
        .plan
        .steps
        .iter()
        .position(|st| matches!(st, Step::RecordState))
        .unwrap();
    let mut ctx = s.ctx_with_hook(&FailAt(k));
    wingetrs::install::executor::execute_install(&s.plan, &mut ctx)
        .await
        .unwrap_err();
    assert_eq!(s.path_var(), pre);
}

#[tokio::test]
async fn path_entries_added_on_install_are_removed_on_rollback() {
    let s = setup().await;
    let k = s
        .plan
        .steps
        .iter()
        .position(|st| matches!(st, Step::RecordState))
        .unwrap();
    let mut ctx = s.ctx_with_hook(&FailAt(k));
    wingetrs::install::executor::execute_install(&s.plan, &mut ctx)
        .await
        .unwrap_err();
    assert_eq!(s.path_var(), ORIGINAL_PATH);
}

#[tokio::test]
async fn shim_of_another_app_is_refused_and_rolled_back() {
    let s = setup().await;
    install(&s, &s.plan).await;
    let shim_file = s.layout.shims().join("tool.shim");
    let before = std::fs::read(&shim_file).unwrap();
    let (url, sha) = s.serve("other.zip", fixture_zip()).await;
    let o = ManifestOpts {
        shortcut: "Other",
        ..default_opts()
    };
    let plan = s.plan_from("other", &manifest_custom(&url, &sha, &o), false);
    let snap = s.snapshot();
    let env = s.env.dump();
    let mut ctx = s.ctx_with_hook(&NoHook);
    let err = wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("o comando tool já pertence a app"),
        "{err}"
    );
    assert_eq!(std::fs::read(&shim_file).unwrap(), before);
    assert_eq!(s.snapshot(), snap);
    assert_eq!(s.env.dump(), env);
    assert!(s.state.get("other").unwrap().is_none());
}

#[tokio::test]
async fn uninstall_keeps_shims_path_while_another_app_uses_it() {
    let s = setup().await;
    install(&s, &s.plan).await;
    let (url, sha) = s.serve("other.zip", fixture_zip()).await;
    let o = ManifestOpts {
        bin: serde_json::json!([["tool.exe", "tool2"]]),
        shortcut: "Other",
        ..default_opts()
    };
    let plan = s.plan_from("other", &manifest_custom(&url, &sha, &o), false);
    install(&s, &plan).await;
    let shims = s.layout.shims().display().to_string();
    let mut ctx = s.ctx_with_hook(&NoHook);
    wingetrs::install::executor::execute_uninstall(&s.uninstall_plan_of("app", false), &mut ctx)
        .unwrap();
    let path = s.path_var();
    assert!(path.split(';').any(|e| e == shims), "{path}");
    let app_current = s.layout.current("app").display().to_string();
    assert!(!path.split(';').any(|e| e == app_current), "{path}");
    assert!(s.layout.shims().join("tool2.shim").exists());
    assert!(!s.layout.shims().join("tool.shim").exists());
    wingetrs::install::executor::execute_uninstall(&s.uninstall_plan_of("other", false), &mut ctx)
        .unwrap();
    assert_eq!(s.path_var(), ORIGINAL_PATH);
}

#[tokio::test]
async fn uninstall_with_file_in_use_changes_nothing() {
    let s = setup().await;
    install(&s, &s.plan).await;
    let snap = s.snapshot();
    let env = s.env.dump();
    let sc = s.shortcuts.dump();
    let state = s.state.get("app").unwrap();
    let busy_file = s.layout.current("app").join("tool.exe");
    let busy = |_: &Path| vec![busy_file.clone()];
    let mut ctx = s.ctx_with_hook(&NoHook);
    ctx.in_use = &busy;
    let err = wingetrs::install::executor::execute_uninstall(&s.uninstall_plan(true), &mut ctx)
        .unwrap_err();
    assert!(
        matches!(err, ExecError::InUse(ref p) if p.contains("tool.exe")),
        "{err}"
    );
    assert_eq!(s.snapshot(), snap);
    assert_eq!(s.env.dump(), env);
    assert_eq!(s.shortcuts.dump(), sc);
    assert_eq!(s.state.get("app").unwrap(), state);
}

#[tokio::test]
async fn failure_message_lists_undone_steps() {
    let s = setup().await;
    let k = s
        .plan
        .steps
        .iter()
        .position(|st| matches!(st, Step::RecordState))
        .unwrap();
    let mut ctx = s.ctx_with_hook(&FailAt(k));
    let err = wingetrs::install::executor::execute_install(&s.plan, &mut ctx)
        .await
        .unwrap_err();
    let ExecError::Failed { index, undone, .. } = &err else {
        panic!("{err}")
    };
    assert_eq!(*index, k);
    assert!(err.to_string().contains("desfeito"), "{err}");
    assert!(
        undone.iter().any(|u| u.contains("comando tool")),
        "{undone:?}"
    );
    assert!(undone.iter().any(|u| u.contains("current")), "{undone:?}");
}

#[tokio::test]
async fn unsafe_version_in_plan_is_refused_before_any_change() {
    let s = setup().await;
    let mut plan = s.plan.clone();
    plan.version = r"..\..\evil".into();
    let snap = s.snapshot();
    let mut ctx = s.ctx_with_hook(&NoHook);
    let err = wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("inseguro"), "{err}");
    assert_eq!(s.snapshot(), snap);
}
