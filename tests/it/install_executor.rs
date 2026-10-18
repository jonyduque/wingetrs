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

fn no_progress(_: &wingetrs::install::progress::ProgressEvent) {}
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
    /// Manifest JSON of the 1.0 plan built by `setup` (same url/sha).
    v1_manifest: String,
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
            manifest_sha256: "",
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
            msiexec: None,
            launch: Default::default(),
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
            manifest_sha256: String::new(),
            steps: Vec::new(),
        },
        v1_manifest: String::new(),
    };
    let (url, sha) = w.serve("pkg-1.0.zip", fixture_zip()).await;
    w.v1_manifest = manifest_json(&url, &sha);
    w.plan = w.plan_from("app", &w.v1_manifest, false);
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

#[tokio::test]
async fn shortcut_of_another_app_is_refused_and_rolled_back() {
    let s = setup().await;
    install(&s, &s.plan).await;
    let (url, sha) = s.serve("other.zip", fixture_zip()).await;
    // different command name, same Start Menu entry "Tool"
    let o = ManifestOpts {
        bin: serde_json::json!([["tool.exe", "tool2"]]),
        ..default_opts()
    };
    let plan = s.plan_from("other", &manifest_custom(&url, &sha, &o), false);
    let snap = s.snapshot();
    let env = s.env.dump();
    let sc = s.shortcuts.dump();
    let mut ctx = s.ctx_with_hook(&NoHook);
    let err = wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("o atalho"), "{err}");
    assert!(err.to_string().contains("já pertence a app"), "{err}");
    assert_eq!(s.shortcuts.dump(), sc);
    assert_eq!(s.snapshot(), snap);
    assert_eq!(s.env.dump(), env);
    assert!(s.state.get("other").unwrap().is_none());
}

/// v1 declares bins a.exe and b.exe; v2 only a.exe.
async fn two_bin_world() -> (World, InstallPlan) {
    let s = setup().await;
    let zip = |marker: &[u8]| {
        let mut tool = fake_pe(3);
        tool.extend_from_slice(marker);
        zip_of(&[
            ("pkg/tool.exe", &tool),
            ("pkg/a.exe", &fake_pe(3)),
            ("pkg/b.exe", &fake_pe(3)),
            ("pkg/conf/default.ini", b"[default]\r\n"),
        ])
    };
    let (url1, sha1) = s.serve("ab-1.0.zip", zip(b"")).await;
    let o1 = ManifestOpts {
        bin: serde_json::json!(["a.exe", "b.exe"]),
        ..default_opts()
    };
    let p1 = s.plan_from("app", &manifest_custom(&url1, &sha1, &o1), false);
    install(&s, &p1).await;
    assert!(s.layout.shims().join("b.shim").exists());
    let (url2, sha2) = s.serve("ab-2.0.zip", zip(b"v2")).await;
    let o2 = ManifestOpts {
        version: "2.0",
        bin: serde_json::json!(["a.exe"]),
        ..default_opts()
    };
    let p2 = s.plan_from("app", &manifest_custom(&url2, &sha2, &o2), false);
    (s, p2)
}

fn shim_names(s: &World) -> Vec<String> {
    s.state
        .get("app")
        .unwrap()
        .unwrap()
        .1
        .into_iter()
        .filter(|a| a.kind == wingetrs::install::state::ArtifactKind::Shim)
        .map(|a| a.target)
        .collect()
}

#[tokio::test]
async fn upgrade_removes_artifacts_the_new_version_no_longer_declares() {
    let (s, p2) = two_bin_world().await;
    install(&s, &p2).await;
    for ext in ["exe", "shim"] {
        assert!(
            !s.layout.shims().join(format!("b.{ext}")).exists(),
            "b.{ext} left behind"
        );
    }
    assert!(s.layout.shims().join("a.shim").exists());
    assert_eq!(shim_names(&s), ["a"]);
}

#[tokio::test]
async fn failed_upgrade_keeps_artifacts_the_new_version_drops() {
    let (s, p2) = two_bin_world().await;
    let k = p2
        .steps
        .iter()
        .position(|st| matches!(st, Step::RecordState))
        .unwrap();
    let mut ctx = s.ctx_with_hook(&FailAt(k));
    wingetrs::install::executor::execute_install(&p2, &mut ctx)
        .await
        .unwrap_err();
    assert!(s.layout.shims().join("b.shim").exists());
    assert!(s.layout.shims().join("b.exe").exists());
    assert_eq!(shim_names(&s), ["a", "b"]);
}

#[tokio::test]
async fn record_state_must_be_the_last_main_step() {
    let s = setup().await;
    let mut plan = s.plan.clone();
    let r = plan
        .steps
        .iter()
        .position(|st| matches!(st, Step::RecordState))
        .unwrap();
    // an otherwise valid plan: only the record moved before the PATH step
    plan.steps.swap(r - 1, r);
    let snap = s.snapshot();
    let mut ctx = s.ctx_with_hook(&NoHook);
    let err = wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("o registro deve ser o último passo"),
        "{err}"
    );
    assert_eq!(s.snapshot(), snap);
    assert!(s.state.get("app").unwrap().is_none());
}

/// Fails at step `k` after turning `blocked` into a directory, so deleting it fails during rollback.
#[derive(Clone)]
struct BlockAndFailAt {
    k: usize,
    blocked: PathBuf,
}
impl StepHook for BlockAndFailAt {
    fn before(&self, i: usize, _s: &Step) -> Result<(), ExecError> {
        if i != self.k {
            return Ok(());
        }
        std::fs::remove_file(&self.blocked).unwrap();
        std::fs::create_dir(&self.blocked).unwrap();
        Err(ExecError::Hook(format!("injected at {i}")))
    }
}

#[tokio::test]
async fn shim_undo_failure_is_reported_and_the_rest_is_undone() {
    let s = setup().await;
    install(&s, &s.plan).await;
    let shim_file = s.layout.shims().join("tool.shim");
    let v1_shim = std::fs::read(&shim_file).unwrap();
    let mut tool = fake_pe(3);
    tool.extend_from_slice(b"v2");
    let (url, sha) = s
        .serve(
            "pkg-2.0.zip",
            zip_of(&[
                ("pkg/tool.exe", &tool),
                ("pkg/conf/default.ini", b"[default]\r\n"),
            ]),
        )
        .await;
    // v2 shim text differs (args), so restoring v1's bytes is observable
    let o = ManifestOpts {
        version: "2.0",
        bin: serde_json::json!([["tool.exe", "tool", "--v2"]]),
        ..default_opts()
    };
    let p2 = s.plan_from("app", &manifest_custom(&url, &sha, &o), false);
    let k = p2
        .steps
        .iter()
        .position(|st| matches!(st, Step::RecordState))
        .unwrap();
    let env = s.env.dump();
    let sc = s.shortcuts.dump();
    let state = s.state.get("app").unwrap();
    let hook = BlockAndFailAt {
        k,
        blocked: s.layout.shims().join("tool.exe"),
    };
    let mut ctx = s.ctx_with_hook(&hook);
    let err = wingetrs::install::executor::execute_install(&p2, &mut ctx)
        .await
        .unwrap_err();
    let ExecError::Failed { undone, .. } = &err else {
        panic!("{err}")
    };
    assert!(
        undone
            .iter()
            .any(|u| u.starts_with("comando tool (falhou:")),
        "{undone:?}"
    );
    assert_eq!(
        std::fs::read(&shim_file).unwrap(),
        v1_shim,
        "saved shim bytes restored anyway"
    );
    assert!(current_target(&s).ends_with("1.0"));
    assert!(!s.layout.version_dir("app", "2.0").exists());
    assert_eq!(s.env.dump(), env);
    assert_eq!(s.shortcuts.dump(), sc);
    assert_eq!(s.state.get("app").unwrap(), state);
}

#[tokio::test]
async fn uninstall_removes_path_variable_the_install_created() {
    let s = setup().await;
    s.env.remove("Path").unwrap();
    let before = s.env.dump();
    install(&s, &s.plan).await;
    assert!(s.env.get("Path").unwrap().is_some());
    let mut ctx = s.ctx_with_hook(&NoHook);
    wingetrs::install::executor::execute_uninstall(&s.uninstall_plan(false), &mut ctx).unwrap();
    assert_eq!(
        s.env.get("Path").unwrap(),
        None,
        "no empty Path left behind"
    );
    assert_eq!(s.env.dump(), before);
}

/// Installs 1.0, upgrades to 2.0 keeping 1.0, marks the kept 1.0 folder and plans 1.0 again.
async fn kept_v1_world() -> (World, InstallPlan) {
    let s = setup().await;
    install(&s, &s.plan).await;
    let p2 = v2_plan(&s, true).await;
    install(&s, &p2).await;
    std::fs::write(s.layout.version_dir("app", "1.0").join("stale.txt"), b"old").unwrap();
    let back = s.plan_from("app", &s.v1_manifest, false);
    assert_eq!(back.upgrade_from.as_deref(), Some("2.0"));
    (s, back)
}

fn leftovers_in_app_dir(s: &World) -> Vec<String> {
    std::fs::read_dir(s.layout.app_dir("app"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".replaced-") || n.starts_with(".staging-"))
        .collect()
}

#[tokio::test]
async fn reinstalling_a_kept_version_replaces_its_folder() {
    let (s, back) = kept_v1_world().await;
    let mut ctx = s.ctx_with_hook(&NoHook);
    wingetrs::install::executor::execute_install(&back, &mut ctx)
        .await
        .unwrap();
    assert!(
        current_target(&s).ends_with("1.0"),
        "{}",
        current_target(&s)
    );
    let v1 = s.layout.version_dir("app", "1.0");
    assert!(v1.join("tool.exe").exists());
    assert!(!v1.join("stale.txt").exists(), "old folder replaced");
    assert!(
        leftovers_in_app_dir(&s).is_empty(),
        "{:?}",
        leftovers_in_app_dir(&s)
    );
    assert!(!s.layout.version_dir("app", "2.0").exists(), "2.0 removed");
    assert_eq!(s.state.get("app").unwrap().unwrap().0.version, "1.0");
}

#[tokio::test]
async fn failure_after_replacing_a_kept_version_restores_it() {
    let probe = kept_v1_world().await.1;
    let link_at = probe
        .steps
        .iter()
        .position(|st| matches!(st, Step::LinkCurrent))
        .unwrap();
    let record_at = probe
        .steps
        .iter()
        .position(|st| matches!(st, Step::RecordState))
        .unwrap();
    for k in [link_at, record_at] {
        let (s, back) = kept_v1_world().await;
        let mut ctx = s.ctx_with_hook(&FailAt(k));
        let err = wingetrs::install::executor::execute_install(&back, &mut ctx)
            .await
            .unwrap_err();
        // The failure must come from the injected step, after Promote replaced the folder.
        match err {
            ExecError::Failed { index, .. } => assert_eq!(index, k, "failed elsewhere"),
            other => panic!("unexpected error: {other}"),
        }
        assert!(
            s.layout
                .version_dir("app", "1.0")
                .join("stale.txt")
                .exists(),
            "kept 1.0 restored after failing at {k}"
        );
        assert!(
            current_target(&s).ends_with("2.0"),
            "{}",
            current_target(&s)
        );
        assert!(
            leftovers_in_app_dir(&s).is_empty(),
            "{:?}",
            leftovers_in_app_dir(&s)
        );
        assert_eq!(s.state.get("app").unwrap().unwrap().0.version, "2.0");
    }
}

#[tokio::test]
async fn promote_still_refuses_the_folder_current_points_to() {
    let mut s = setup().await;
    install(&s, &s.plan).await;
    let (mut rec, arts) = s.state.get("app").unwrap().unwrap();
    rec.version = "0.9".into();
    s.state.record(&rec, &arts).unwrap();
    let again = s.plan_from("app", &s.v1_manifest, false);
    std::fs::write(s.layout.version_dir("app", "1.0").join("mine.txt"), b"keep").unwrap();
    let mut ctx = s.ctx_with_hook(&NoHook);
    let err = wingetrs::install::executor::execute_install(&again, &mut ctx)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("já existe"), "{err}");
    let v1 = s.layout.version_dir("app", "1.0");
    assert!(v1.join("tool.exe").exists());
    assert!(v1.join("mine.txt").exists(), "1.0 untouched");
    assert!(current_target(&s).ends_with("1.0"));
}

#[tokio::test]
async fn replaced_name_already_taken_picks_another() {
    let (s, back) = kept_v1_world().await;
    // Stale asides from a "crashed run" of this process, covering the next candidates.
    for seq in 0..32 {
        let d = s
            .layout
            .app_dir("app")
            .join(format!(".replaced-1.0-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("old.txt"), b"x").unwrap();
    }
    let mut ctx = s.ctx_with_hook(&NoHook);
    wingetrs::install::executor::execute_install(&back, &mut ctx)
        .await
        .unwrap();
    assert!(current_target(&s).ends_with("1.0"));
    assert!(
        !s.layout
            .version_dir("app", "1.0")
            .join("stale.txt")
            .exists()
    );
}

#[tokio::test]
async fn stale_replaced_folders_are_swept_after_success() {
    let s = setup().await;
    let app = s.layout.app_dir("app");
    let stale = app.join(".replaced-x");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("old.txt"), b"x").unwrap();
    let real = s.layout.version_dir("app", "0.5");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("keep.txt"), b"x").unwrap();
    install(&s, &s.plan).await;
    assert!(!stale.exists(), "stale .replaced-* swept");
    assert!(real.join("keep.txt").exists(), "real version folder kept");
    assert!(current_target(&s).ends_with("1.0"));
}

// ---- Task 128: 7z and MSI through the executor ----

/// Non-solid 7z with the given content method.
fn sevenz_of(entries: &[(&str, &[u8])], method: sevenz_rust2::EncoderMethod) -> Vec<u8> {
    let mut w = sevenz_rust2::ArchiveWriter::new(std::io::Cursor::new(Vec::new())).unwrap();
    w.set_content_methods(vec![method.into()]);
    for (name, data) in entries {
        w.push_archive_entry(
            sevenz_rust2::ArchiveEntry::new_file(name),
            Some(std::io::Cursor::new(data.to_vec())),
        )
        .unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn fixture_7z() -> Vec<u8> {
    sevenz_of(
        &[
            ("pkg/tool.exe", &fake_pe(3)),
            ("pkg/conf/default.ini", b"[default]\r\n"),
        ],
        sevenz_rust2::EncoderMethod::LZMA2,
    )
}

/// MSI with `pkg\one.txt` (`f_one` of `tests/fixtures/msi/one.cab`, "one\r\n"), built
/// as `tests/it/install_msi.rs` does.
fn fixture_msi() -> Vec<u8> {
    msi_with(2, "one.txt")
}

/// `fixture_msi` with another Summary word count (0 = files outside the package, which the
/// own extractor does not support) and another `FileName`.
fn msi_with(word_count: i32, file_name: &str) -> Vec<u8> {
    use msi::{Column, Insert, Package, PackageType, Value};
    let cab = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/msi/one.cab"
    ))
    .unwrap();
    let mut p = Package::create(PackageType::Installer, std::io::Cursor::new(Vec::new())).unwrap();
    p.summary_info_mut().set_word_count(word_count);
    p.create_table(
        "Directory",
        vec![
            Column::build("Directory").primary_key().id_string(72),
            Column::build("Directory_Parent").nullable().id_string(72),
            Column::build("DefaultDir").text_string(255),
        ],
    )
    .unwrap();
    p.create_table(
        "Component",
        vec![
            Column::build("Component").primary_key().id_string(72),
            Column::build("Directory_").id_string(72),
        ],
    )
    .unwrap();
    p.create_table(
        "File",
        vec![
            Column::build("File").primary_key().id_string(72),
            Column::build("Component_").id_string(72),
            Column::build("FileName").text_string(255),
            Column::build("FileSize").int32(),
            Column::build("Version").nullable().string(72),
            Column::build("Attributes").nullable().int16(),
            Column::build("Sequence").int32(),
        ],
    )
    .unwrap();
    p.create_table(
        "Media",
        vec![
            Column::build("DiskId").primary_key().int16(),
            Column::build("LastSequence").int32(),
            Column::build("Cabinet").nullable().string(255),
        ],
    )
    .unwrap();
    p.insert_rows(
        Insert::into("Directory")
            .row(vec!["TARGETDIR".into(), Value::Null, "SourceDir".into()])
            .row(vec!["D1".into(), "TARGETDIR".into(), "pkg".into()]),
    )
    .unwrap();
    p.insert_rows(Insert::into("Component").row(vec!["C_D1".into(), "D1".into()]))
        .unwrap();
    p.insert_rows(Insert::into("File").row(vec![
        "f_one".into(),
        "C_D1".into(),
        file_name.into(),
        0.into(),
        Value::Null,
        Value::Int(0),
        1.into(),
    ]))
    .unwrap();
    p.insert_rows(Insert::into("Media").row(vec![Value::Int(1), 1.into(), "#cab1".into()]))
        .unwrap();
    p.write_stream("cab1").unwrap().write_all(&cab).unwrap();
    p.into_inner().unwrap().into_inner()
}

/// After a purge uninstall nothing of the app is left: every entry from before is intact
/// and the only new entries are empty shared folders (`apps`, `persist`).
fn assert_nothing_left(s: &World, before: &std::collections::BTreeMap<String, String>) {
    let after = s.snapshot();
    for (k, v) in before {
        assert_eq!(after.get(k), Some(v), "{k} mudou");
    }
    let extra: Vec<_> = after
        .iter()
        .filter(|(k, v)| !before.contains_key(*k) && v.as_str() != "dir:")
        .collect();
    assert!(extra.is_empty(), "sobras: {extra:?}");
    assert!(!s.layout.app_dir("app").exists());
    assert!(!s.layout.persist_dir("app").exists());
    assert!(s.state.apps().unwrap().is_empty());
    assert!(s.shortcuts.dump().is_empty());
}

#[tokio::test]
async fn install_remove_7z_leaves_nothing() {
    let s = setup().await;
    let before = s.snapshot();
    let before_env = s.env.dump();
    let (url, sha) = s.serve("pkg-1.0.7z", fixture_7z()).await;
    let plan = s.plan_from("app", &manifest_json(&url, &sha), false);
    install(&s, &plan).await;
    assert_eq!(
        std::fs::read(s.layout.current("app").join("tool.exe")).unwrap(),
        fake_pe(3)
    );
    assert_eq!(
        std::fs::read(s.layout.persist_dir("app").join("conf/default.ini")).unwrap(),
        b"[default]\r\n"
    );
    let mut ctx = s.ctx_with_hook(&NoHook);
    wingetrs::install::executor::execute_uninstall(&s.uninstall_plan(true), &mut ctx).unwrap();
    drop(ctx);
    assert_nothing_left(&s, &before);
    assert_eq!(s.env.dump(), before_env);
}

#[tokio::test]
async fn install_remove_msi_leaves_nothing() {
    let s = setup().await;
    let before = s.snapshot();
    let before_env = s.env.dump();
    let (url, sha) = s.serve("pkg-1.0.msi", fixture_msi()).await;
    let manifest = serde_json::json!({
        "version": "1.0",
        "url": url,
        "hash": sha,
        "extract_dir": "pkg",
    })
    .to_string();
    let plan = s.plan_from("app", &manifest, false);
    install(&s, &plan).await;
    let current = s.layout.current("app");
    assert_eq!(std::fs::read(current.join("one.txt")).unwrap(), b"one\r\n");
    let names: Vec<_> = std::fs::read_dir(&current)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "install.json")
        .collect();
    assert_eq!(names, ["one.txt"]);
    let mut ctx = s.ctx_with_hook(&NoHook);
    wingetrs::install::executor::execute_uninstall(&s.uninstall_plan(true), &mut ctx).unwrap();
    drop(ctx);
    assert_nothing_left(&s, &before);
    assert_eq!(s.env.dump(), before_env);
}

fn root_list(s: &World) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(s.layout.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "blobs" && n != "tmp")
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn corrupt_7z_rolls_back_identically() {
    let s = setup().await;
    let mut a = sevenz_of(
        &[("pkg/tool.exe", &fake_pe(3))],
        sevenz_rust2::EncoderMethod::COPY,
    );
    a[32] ^= 0xFF;
    let (url, sha) = s.serve("pkg-1.0.7z", a).await;
    let plan = s.plan_from("app", &manifest_json(&url, &sha), false);
    let (before_list, before) = (root_list(&s), s.snapshot());
    let (before_env, before_sc) = (s.env.dump(), s.shortcuts.dump());
    let mut ctx = s.ctx_with_hook(&NoHook);
    let err = wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap_err()
        .to_string();
    drop(ctx);
    assert!(
        err.contains("(extrair [1])") && err.contains("ChecksumVerificationFailed"),
        "{err}"
    );
    assert_eq!(root_list(&s), before_list);
    assert_eq!(s.snapshot(), before);
    assert_eq!(s.env.dump(), before_env);
    assert_eq!(s.shortcuts.dump(), before_sc);
    assert!(s.state.apps().unwrap().is_empty());
}

static TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Real extraction, then waits until the runtime ticked twice since it began. On the
/// runtime thread the ticker cannot run: fails after 10 s instead of hanging.
fn extract_while_ticking(
    file: &Path,
    name: &str,
    dir: Option<&str>,
    to: Option<&str>,
    dest: &Path,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    use std::sync::atomic::Ordering;
    let start = TICKS.load(Ordering::SeqCst);
    let r = wingetrs::install::extract::extract(file, name, dir, to, dest);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while TICKS.load(Ordering::SeqCst) < start + 2 {
        if std::time::Instant::now() > deadline {
            return Err(wingetrs::install::extract::ExtractError::Archive(
                "runtime parado durante a extração".into(),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    r
}

#[tokio::test(flavor = "current_thread")]
async fn extract_runs_off_the_runtime() {
    let s = setup().await;
    // The tick wait in `extract_while_ticking`, not the archive size, proves the extraction
    // left the runtime thread: the smallest real 7z is enough.
    let (url, sha) = s.serve("pkg-1.0.7z", fixture_7z()).await;
    let plan = s.plan_from("app", &manifest_json(&url, &sha), false);
    let ticker = tokio::spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            TICKS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    let mut ctx = s.ctx_with_hook(&NoHook);
    let r =
        wingetrs::install::executor::execute_install_with(&plan, &mut ctx, extract_while_ticking)
            .await;
    ticker.abort();
    drop(ctx);
    r.unwrap();
    assert_eq!(
        std::fs::read(s.layout.current("app").join("tool.exe")).unwrap(),
        fake_pe(3)
    );
}

const PANIC_MARK: &str = "decoder blew up ZZQ";

/// Writes part of the tree, then panics inside `catch_decoder`, as the real decoders do
/// (D4-1): caught on the extraction thread and turned into `ExtractError::Decoder`.
fn panicking_decoder(
    _file: &Path,
    _name: &str,
    _dir: Option<&str>,
    _to: Option<&str>,
    dest: &Path,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    std::fs::create_dir_all(dest.join("pkg")).unwrap();
    std::fs::write(dest.join("pkg/partial.bin"), b"half").unwrap();
    wingetrs::install::extract::catch_decoder(
        || -> Result<(), wingetrs::install::extract::ExtractError> { panic!("{PANIC_MARK}") },
    )
    .unwrap_or_else(|_| {
        Err(wingetrs::install::extract::ExtractError::Decoder(
            PANIC_MARK.into(),
        ))
    })
}

#[tokio::test]
async fn decoder_panic_rolls_back() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let s = setup().await;
    let before = s.snapshot();
    let before_env = s.env.dump();
    let restored = std::sync::Arc::new(AtomicUsize::new(0));
    // The panic hook is process-wide: while it is swapped, a panic in any other test of this
    // binary goes through it too. It only counts panics carrying PANIC_MARK and still calls
    // the previous hook, so other tests keep their messages and cannot move the counter.
    let previous = std::panic::take_hook();
    // The TUI hook, with a counter in place of the terminal restore.
    wingetrs::tui::run::install_panic_hook({
        let restored = restored.clone();
        move |info| {
            if info.to_string().contains(PANIC_MARK) {
                restored.fetch_add(1, Ordering::SeqCst);
            }
            previous(info);
        }
    });
    let mut ctx = s.ctx_with_hook(&NoHook);
    let r = wingetrs::install::executor::execute_install_with(&s.plan, &mut ctx, panicking_decoder)
        .await;
    drop(ctx);
    // Back to the default hook before any assertion can panic.
    drop(std::panic::take_hook());
    let err = r.unwrap_err().to_string();
    assert!(
        err.contains("(extrair [1])")
            && err.contains(&format!("falha no decodificador: {PANIC_MARK}")),
        "{err}"
    );
    assert_eq!(
        restored.load(Ordering::SeqCst),
        0,
        "o hook restaurou o terminal"
    );
    assert_eq!(s.snapshot(), before);
    assert_eq!(s.env.dump(), before_env);
    assert!(s.state.apps().unwrap().is_empty());
}

/// A panic outside any decoder guard: our own bug, not a decoder's.
fn extractor_bug(
    _file: &Path,
    _name: &str,
    _dir: Option<&str>,
    _to: Option<&str>,
    dest: &Path,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    std::fs::create_dir_all(dest.join("pkg")).unwrap();
    std::fs::write(dest.join("pkg/partial.bin"), b"half").unwrap();
    panic!("extractor bug ZZW")
}

#[tokio::test]
async fn extraction_worker_panic_rolls_back_then_propagates() {
    use futures_util::FutureExt;
    let s = setup().await;
    let (before_list, before) = (root_list(&s), s.snapshot());
    let mut ctx = s.ctx_with_hook(&NoHook);
    let caught = std::panic::AssertUnwindSafe(wingetrs::install::executor::execute_install_with(
        &s.plan,
        &mut ctx,
        extractor_bug,
    ))
    .catch_unwind()
    .await;
    drop(ctx);
    let payload = caught.expect_err("the worker panic must propagate");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("extractor bug ZZW")
    );
    assert_eq!(root_list(&s), before_list);
    assert_eq!(s.snapshot(), before);
    assert!(s.state.apps().unwrap().is_empty());
}
// ---- msiexec /a fallback (Task 129) ----

use wingetrs::install::executor::Msiexec;
use wingetrs::install::msi_queue::Attempt;

fn msi_manifest(url: &str, sha: &str, extract_dir: Option<&str>) -> String {
    let mut m = serde_json::json!({ "version": "1.0", "url": url, "hash": sha });
    if let Some(d) = extract_dir {
        m["extract_dir"] = serde_json::json!(d);
    }
    m.to_string()
}

fn never_busy() -> bool {
    false
}

/// What `msiexec /a` leaves in `TARGETDIR`: the tree (here with a different case than the
/// manifest's `extract_dir`) and the administrative copy of the package at its root.
fn fake_admin(msi: &Path, target: &Path, _log: &Path) -> std::io::Result<Attempt> {
    // A sibling of the staging folder, never inside it.
    let work = target.parent().unwrap();
    assert!(target.ends_with("SourceDir"), "{}", target.display());
    assert_eq!(
        work.file_name().unwrap(),
        ".staging-1.0.msiexec",
        "{}",
        target.display()
    );
    std::fs::create_dir_all(target.join("PKG"))?;
    std::fs::write(target.join("PKG").join("one.txt"), b"one\r\n")?;
    std::fs::write(target.join("readme.txt"), b"x")?;
    std::fs::copy(msi, target.join(msi.file_name().unwrap()))?;
    Ok(Attempt::Done)
}

fn must_not_launch(_: &Path, _: &Path, _: &Path) -> std::io::Result<Attempt> {
    panic!("msiexec lançado para um erro que não é de formato");
}

const FAKE_MSIEXEC: Msiexec = Msiexec {
    busy: never_busy,
    launch: fake_admin,
};

const NO_LAUNCH: Msiexec = Msiexec {
    busy: never_busy,
    launch: must_not_launch,
};

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn unsupported_msi_needs_msiexec() {
    let s = setup().await;
    let (before_list, before) = (root_list(&s), s.snapshot());
    let (url, sha) = s.serve("pkg-1.0.msi", msi_with(0, "one.txt")).await;
    let plan = s.plan_from("app", &msi_manifest(&url, &sha, Some("pkg")), false);
    let mut ctx = s.ctx_with_hook(&NoHook);
    let r = wingetrs::install::executor::execute_install(&plan, &mut ctx).await;
    drop(ctx);
    match r {
        Err(ExecError::NeedsMsiexec {
            app,
            source_index,
            reason,
        }) => {
            assert_eq!(app, "app");
            assert_eq!(source_index, 0);
            assert!(reason.contains("sem compressão"), "{reason}");
        }
        other => panic!("esperava NeedsMsiexec: {other:?}"),
    }
    // Rolled back: the app folder the extraction created is gone again.
    assert!(!s.layout.app_dir("app").exists());
    assert_eq!(root_list(&s), before_list);
    assert_eq!(s.snapshot(), before);
    assert!(s.state.apps().unwrap().is_empty());
}

#[tokio::test]
async fn allowed_msiexec_extracts_via_fake() {
    let s = setup().await;
    let (url, sha) = s.serve("pkg-1.0.msi", msi_with(0, "one.txt")).await;
    let plan = s.plan_from("app", &msi_manifest(&url, &sha, None), false);
    let mut ctx = s.ctx_with_hook(&NoHook);
    ctx.msiexec = Some(FAKE_MSIEXEC);
    wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap();
    drop(ctx);
    let current = s.layout.current("app");
    // No `pkg-1.0.msi` (the administrative copy) and no `_msiexec` work folder.
    assert_eq!(
        names_in(&current),
        ["PKG", "install.json", "readme.txt"],
        "{}",
        current.display()
    );
    assert_eq!(
        std::fs::read(current.join("PKG").join("one.txt")).unwrap(),
        b"one\r\n"
    );
    assert_eq!(names_in(&s.layout.app_dir("app")), ["1.0", "current"]);
    assert_eq!(s.state.apps().unwrap().len(), 1);
}

#[tokio::test]
async fn msiexec_tree_goes_through_extract_dir_without_case() {
    let s = setup().await;
    let (url, sha) = s.serve("pkg-1.0.msi", msi_with(0, "one.txt")).await;
    let plan = s.plan_from("app", &msi_manifest(&url, &sha, Some("pkg")), false);
    let mut ctx = s.ctx_with_hook(&NoHook);
    ctx.msiexec = Some(FAKE_MSIEXEC);
    wingetrs::install::executor::execute_install(&plan, &mut ctx)
        .await
        .unwrap();
    drop(ctx);
    let current = s.layout.current("app");
    assert_eq!(names_in(&current), ["install.json", "one.txt"]);
    assert_eq!(std::fs::read(current.join("one.txt")).unwrap(), b"one\r\n");
}

#[tokio::test]
async fn integrity_error_never_falls_back() {
    let s = setup().await;
    // Unsafe name inside an MSI the own extractor reads: refused as such, never offered to
    // msiexec, with or without authorization.
    let (url, sha) = s.serve("pkg-1.0.msi", msi_with(2, "..")).await;
    let plan = s.plan_from("app", &msi_manifest(&url, &sha, Some("pkg")), false);
    for allowed in [None, Some(NO_LAUNCH)] {
        let before = s.snapshot();
        let mut ctx = s.ctx_with_hook(&NoHook);
        ctx.msiexec = allowed;
        let r = wingetrs::install::executor::execute_install(&plan, &mut ctx).await;
        drop(ctx);
        match r {
            Err(ExecError::Failed { message, .. }) => {
                assert!(message.contains("nome inseguro"), "{message}");
            }
            other => panic!("esperava Failed: {other:?}"),
        }
        assert_eq!(s.snapshot(), before);
    }
    // Wrong hash: the download fails before any extraction.
    let (url, _) = s.serve("bad-1.0.msi", msi_with(0, "one.txt")).await;
    let wrong = sha256_hex(b"something else");
    let plan = s.plan_from("app", &msi_manifest(&url, &wrong, Some("pkg")), false);
    for allowed in [None, Some(NO_LAUNCH)] {
        let mut ctx = s.ctx_with_hook(&NoHook);
        ctx.msiexec = allowed;
        let r = wingetrs::install::executor::execute_install(&plan, &mut ctx).await;
        drop(ctx);
        assert!(
            matches!(&r, Err(ExecError::Failed { step, .. }) if step.starts_with("baixar")),
            "{r:?}"
        );
    }
    assert!(s.state.apps().unwrap().is_empty());
}

/// `msiexec` left a name `safepath` refuses (trailing dot, writable only through `\\?\`).
fn fake_admin_unsafe_name(_: &Path, target: &Path, _: &Path) -> std::io::Result<Attempt> {
    std::fs::create_dir_all(target.join("pkg"))?;
    std::fs::write(target.join("pkg").join("ok.txt"), b"ok")?;
    let verbatim = format!(r"\\?\{}", target.join("pkg").join("evil.").display());
    std::fs::write(verbatim, b"x")?;
    Ok(Attempt::Done)
}

/// `msiexec` left a name that is not valid UTF-16 (a lone surrogate, which NTFS accepts).
fn fake_admin_non_utf8(_: &Path, target: &Path, _: &Path) -> std::io::Result<Attempt> {
    use std::os::windows::ffi::OsStringExt;
    std::fs::create_dir_all(target.join("pkg"))?;
    let name = std::ffi::OsString::from_wide(&[0xD800, u16::from(b'a')]);
    std::fs::write(target.join("pkg").join(name), b"x")?;
    Ok(Attempt::Done)
}

/// `msiexec` left a junction in the tree.
fn fake_admin_junction(_: &Path, target: &Path, _: &Path) -> std::io::Result<Attempt> {
    std::fs::create_dir_all(target.join("pkg"))?;
    let outside = target.parent().unwrap().join("outside");
    std::fs::create_dir_all(&outside)?;
    wingetrs::install::link::create(&outside, &target.join("pkg").join("link"))?;
    Ok(Attempt::Done)
}

#[tokio::test]
async fn msiexec_tree_with_unsafe_entries_is_refused_and_rolled_back() {
    let s = setup().await;
    let (url, sha) = s.serve("pkg-1.0.msi", msi_with(0, "one.txt")).await;
    let plan = s.plan_from("app", &msi_manifest(&url, &sha, Some("pkg")), false);
    for (launch, what) in [
        (
            fake_admin_unsafe_name as fn(&Path, &Path, &Path) -> std::io::Result<Attempt>,
            "pkg/evil.",
        ),
        (fake_admin_junction, "pkg/link"),
        (fake_admin_non_utf8, "pkg/\u{FFFD}a"),
    ] {
        let before = s.snapshot();
        let mut ctx = s.ctx_with_hook(&NoHook);
        ctx.msiexec = Some(Msiexec {
            busy: never_busy,
            launch,
        });
        let r = wingetrs::install::executor::execute_install(&plan, &mut ctx).await;
        drop(ctx);
        match r {
            Err(ExecError::Failed { message, .. }) => assert!(
                message.contains(&format!("caminho inseguro no arquivo: {what}")),
                "{message}"
            ),
            other => panic!("esperava Failed: {other:?}"),
        }
        assert!(!s.layout.app_dir("app").exists());
        // `msi.lock` stays once created, like `install.lock`.
        let without_lock = |mut m: std::collections::BTreeMap<String, String>| {
            m.remove("msi.lock");
            m
        };
        assert_eq!(without_lock(s.snapshot()), without_lock(before));
    }
}

fn admin_tree(files: &[(&str, &[u8])]) -> (TempDir, PathBuf, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let msi = t.path().join("x.msi");
    std::fs::write(&msi, [0u8; 10]).unwrap();
    let src = t.path().join("SourceDir");
    for (name, bytes) in files {
        let p = src.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    (t, msi, src)
}

fn import(
    msi: &Path,
    src: &Path,
    dest: &Path,
    extract_dir: Option<&str>,
    limits: &wingetrs::install::extract::Limits,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    wingetrs::install::extract::import_admin_tree(
        &wingetrs::install::extract::ExtractSource {
            file: msi,
            file_name: "x.msi",
            extract_dir,
            extract_to: None,
        },
        src,
        "x.msi",
        dest,
        limits,
    )
}

#[test]
fn admin_tree_import_keeps_the_extraction_limits() {
    use wingetrs::install::extract::{DEFAULT_LIMITS, ExtractError, Limits};
    // 10-byte package, ratio 1: a 10-byte budget.
    let tight = Limits {
        ratio: 1,
        floor: 0,
        ceiling: 10,
        max_entries: 3,
    };
    let (t, msi, src) = admin_tree(&[("a/b.txt", b"0123456789"), ("x.msi", b"admin copy")]);
    // The administrative copy at the root is neither moved nor charged.
    import(&msi, &src, &t.path().join("ok"), None, &tight).unwrap();
    assert_eq!(names_in(&t.path().join("ok")), ["a"]);

    let (t, msi, src) = admin_tree(&[("a/b.txt", b"0123456789A")]);
    let r = import(&msi, &src, &t.path().join("big"), None, &tight);
    assert!(
        matches!(r, Err(ExtractError::TooLarge { limit: 10 })),
        "{r:?}"
    );
    assert!(!t.path().join("big").exists());

    let (t, msi, src) = admin_tree(&[("a/1", b""), ("a/2", b""), ("a/3", b"")]);
    let r = import(&msi, &src, &t.path().join("many"), None, &tight);
    assert!(
        matches!(r, Err(ExtractError::TooManyEntries { limit: 3 })),
        "{r:?}"
    );

    let (t, msi, src) = admin_tree(&[("a/b.txt", b"b")]);
    let r = import(
        &msi,
        &src,
        &t.path().join("nodir"),
        Some("pkg"),
        &DEFAULT_LIMITS,
    );
    assert!(
        matches!(&r, Err(ExtractError::MissingExtractDir(d)) if d == "pkg"),
        "{r:?}"
    );
}

static CANCEL_EVENTS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
static CANCEL_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static CANCEL_RELEASE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Starts, waits to be released (10 s hang detector), then writes into the staging folder.
fn slow_extract(
    _file: &Path,
    _name: &str,
    _dir: Option<&str>,
    _to: Option<&str>,
    dest: &Path,
) -> Result<(), wingetrs::install::extract::ExtractError> {
    use std::sync::atomic::Ordering;
    CANCEL_STARTED.store(true, Ordering::SeqCst);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !CANCEL_RELEASE.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    std::fs::create_dir_all(dest.join("pkg")).unwrap();
    std::fs::write(dest.join("pkg/late.bin"), b"late").unwrap();
    CANCEL_EVENTS.lock().unwrap().push("worker ended");
    Ok(())
}

#[tokio::test]
async fn cancelled_install_waits_for_the_extraction_worker() {
    use std::sync::atomic::Ordering;
    let s = setup().await;
    let mut ctx = s.ctx_with_hook(&NoHook);
    let mut fut = Box::pin(wingetrs::install::executor::execute_install_with(
        &s.plan,
        &mut ctx,
        slow_extract,
    ));
    while !CANCEL_STARTED.load(Ordering::SeqCst) {
        tokio::select! {
            r = &mut fut => panic!("terminou antes de cancelar: {r:?}"),
            () = tokio::time::sleep(std::time::Duration::from_millis(1)) => {}
        }
    }
    let releaser = std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(50));
        CANCEL_RELEASE.store(true, Ordering::SeqCst);
    });
    // Cancelling = dropping the future while the worker still runs.
    drop(fut);
    CANCEL_EVENTS.lock().unwrap().push("cancel returned");
    releaser.join().unwrap();
    drop(ctx);
    assert_eq!(
        *CANCEL_EVENTS.lock().unwrap(),
        ["worker ended", "cancel returned"]
    );
    // Removed only after the worker wrote its last file.
    assert!(!s.layout.staging_dir("app", "1.0").exists());
}

#[tokio::test]
async fn only_an_msi_the_extractor_refuses_falls_back() {
    let s = setup().await;
    // Unsupported, but not an MSI to unpack: a `.msi` without the OLE signature, and an MSI
    // file named `.7z`. Never offered to msiexec, with or without authorization.
    for (name, bytes, why) in [
        ("pkg-1.0.msi", fake_pe(2), "conteúdo não é MSI"),
        (
            "pkg-1.0.7z",
            msi_with(0, "one.txt"),
            "conteúdo não reconhecido como 7z",
        ),
    ] {
        let (url, sha) = s.serve(name, bytes).await;
        let plan = s.plan_from("app", &msi_manifest(&url, &sha, Some("pkg")), false);
        for allowed in [None, Some(NO_LAUNCH)] {
            let mut ctx = s.ctx_with_hook(&NoHook);
            ctx.msiexec = allowed;
            let r = wingetrs::install::executor::execute_install(&plan, &mut ctx).await;
            drop(ctx);
            match r {
                Err(ExecError::Failed { message, .. }) => {
                    assert!(message.contains(why), "{message}");
                }
                other => panic!("esperava Failed: {other:?}"),
            }
        }
    }
    assert!(!s.layout.app_dir("app").exists());
}
