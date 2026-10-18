#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 144).
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::install::executor::{ExecCtx, ExecError, NoHook, StepHook};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::plan::{InstallPlan, Step};
use wingetrs::install::planner::{InstalledInfo, PlanInput, plan_install};
use wingetrs::install::state::{StateDb, StateError, installed_manifest};
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
            runners: wingetrs::install::executor::Runners::disabled(),
        }
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

/// Same manifest, with unusual spacing and order, so a reformat would change the bytes.
fn odd_text(w: &World, version: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(&w.v1_manifest).unwrap();
    let url = v["url"].as_str().unwrap();
    let hash = v["hash"].as_str().unwrap();
    format!(
        "{{ \"version\" :\"{version}\",  \"url\":\"{url}\" ,
	\"hash\" : \"{hash}\", \"extract_dir\":\"pkg\" }}"
    )
}

async fn install_plan(s: &World, plan: &InstallPlan) {
    let mut ctx = s.ctx_with_hook(&NoHook);
    wingetrs::install::executor::execute_install(plan, &mut ctx)
        .await
        .unwrap();
}

#[tokio::test]
async fn install_writes_manifest_json_byte_exact() {
    let s = setup().await;
    let text = odd_text(&s, "1.0");
    let plan = s.plan_from("app", &text, false);
    install_plan(&s, &plan).await;
    let got = std::fs::read(s.layout.version_dir("app", "1.0").join("manifest.json")).unwrap();
    assert_eq!(got, text.as_bytes());
}

#[tokio::test]
async fn plan_puts_write_manifest_right_before_record_state() {
    let s = setup().await;
    let i = s
        .plan
        .steps
        .iter()
        .position(|st| matches!(st, Step::WriteManifest { .. }))
        .unwrap();
    assert!(matches!(s.plan.steps[i + 1], Step::RecordState));
    let Step::WriteManifest { json } = &s.plan.steps[i] else {
        panic!("not a WriteManifest");
    };
    assert_eq!(json, &s.v1_manifest);
}

#[tokio::test]
async fn rollback_removes_manifest_json() {
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
        panic!("unexpected error: {err}");
    };
    assert_eq!(*index, k);
    assert!(
        undone.iter().any(|u| u == "gravar manifesto"),
        "the manifest write is undone itself: {undone:?}"
    );
    assert!(!s.layout.app_dir("app").exists());
}

#[tokio::test]
async fn installed_manifest_is_the_written_one() {
    let s = setup().await;
    let a = odd_text(&s, "1.0");
    install_plan(&s, &s.plan_from("app", &a, false)).await;
    let got = installed_manifest(&s.layout, "app", "1.0", &sha256_hex(a.as_bytes())).unwrap();
    assert_eq!(got.as_deref(), Some(a.as_str()));
    let b = odd_text(&s, "9.9");
    let err = installed_manifest(&s.layout, "app", "1.0", &sha256_hex(b.as_bytes())).unwrap_err();
    assert!(matches!(err, StateError::ManifestMismatch(ref n) if n == "app"));
}

#[tokio::test]
async fn installed_manifest_missing_is_none() {
    let s = setup().await;
    std::fs::create_dir_all(s.layout.version_dir("app", "1.0")).unwrap();
    assert_eq!(
        installed_manifest(&s.layout, "app", "1.0", "00").unwrap(),
        None
    );
}

#[tokio::test]
async fn upgrade_keep_old_has_both_manifests() {
    let s = setup().await;
    let a = odd_text(&s, "1.0");
    install_plan(&s, &s.plan_from("app", &a, false)).await;
    let b = odd_text(&s, "2.0");
    install_plan(&s, &s.plan_from("app", &b, true)).await;
    let read = |v: &str| {
        std::fs::read_to_string(s.layout.version_dir("app", v).join("manifest.json")).unwrap()
    };
    assert_eq!(read("1.0"), a);
    assert_eq!(read("2.0"), b);
}

#[tokio::test]
async fn installed_manifest_non_utf8_is_refused() {
    let s = setup().await;
    let dir = s.layout.version_dir("app", "1.0");
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = [b'{', 0xFF, 0xFE, b'}'];
    std::fs::write(dir.join("manifest.json"), bytes).unwrap();
    let err = installed_manifest(&s.layout, "app", "1.0", &sha256_hex(&bytes)).unwrap_err();
    assert!(matches!(err, StateError::ManifestMismatch(ref n) if n == "app"));
}

#[tokio::test]
async fn installed_manifest_over_the_size_cap_is_refused() {
    let s = setup().await;
    let dir = s.layout.version_dir("app", "1.0");
    std::fs::create_dir_all(&dir).unwrap();
    let big = vec![b' '; 4 * 1024 * 1024 + 1];
    std::fs::write(dir.join("manifest.json"), &big).unwrap();
    let err = installed_manifest(&s.layout, "app", "1.0", &sha256_hex(&big)).unwrap_err();
    assert!(matches!(err, StateError::ManifestTooLarge(ref n) if n == "app"));
}
