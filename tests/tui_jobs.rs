#![allow(clippy::unwrap_used, clippy::expect_used)]
//! TUI jobs against fake effects: planning, execution with progress, lock, worker panic.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::{DataDir, UpdateLock};
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::core::SourceKind;
use wingetrs::http::Http;
use wingetrs::install::layout::Layout;
use wingetrs::install::ops::{OpsCtx, ResidueScan};
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{Effects, EnvStore, EnvValue};
use wingetrs::tui::app::{Action, InstallFail, Msg, Planned};
use wingetrs::tui::jobs::{self, ProgressThrottle};
use wingetrs::update::LockWait;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

struct World {
    server: MockServer,
    _t: TempDir,
    data: DataDir,
    layout: Layout,
    env: FakeEnv,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    http: Http,
    catalog: Catalog,
}

impl World {
    async fn new() -> World {
        let server = MockServer::start().await;
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        let layout = Layout::new(&root);
        std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
        std::fs::write(layout.launcher(), fake_pe(3)).unwrap();
        let env = FakeEnv::new();
        env.set(
            "Path",
            &EnvValue {
                value: r"C:\Existing".into(),
                expand: true,
            },
        )
        .unwrap();
        let shortcuts = FakeShortcuts::new(t.path().join("startmenu"));
        World {
            server,
            data: DataDir::new(&root),
            layout,
            _t: t,
            env,
            shortcuts,
            trash: FakeTrash::new(),
            registry: FakeRegistry::new(),
            http: Http::new().unwrap(),
            catalog: Catalog::open_in_memory().unwrap(),
        }
    }

    /// Serves a zip and puts a manifest for `id` in the in-memory `scoop:main` catalog.
    async fn add(&mut self, id: &str) {
        let zip = zip_of(&[(&format!("pkg/{id}.exe"), &fake_pe(3))]);
        let sha = hex::encode(Sha256::digest(&zip));
        let name = format!("{id}-1.0.zip");
        Mock::given(method("GET"))
            .and(path(format!("/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
            .mount(&self.server)
            .await;
        let raw = serde_json::json!({
            "version": "1.0",
            "url": format!("{}/{name}", self.server.uri()),
            "hash": sha,
            "extract_dir": "pkg",
            "bin": format!("{id}.exe"),
        })
        .to_string();
        self.catalog
            .replace_source(
                "scoop:main",
                SourceKind::Scoop,
                "v",
                1,
                &[NewPackage {
                    id: id.into(),
                    name: id.into(),
                    version: "1.0".into(),
                    raw: Some(raw),
                    ..Default::default()
                }],
                &WingetMaps::default(),
            )
            .unwrap();
    }

    fn ctx<'a>(
        &'a self,
        wait: LockWait,
        progress: &'a (dyn Fn(&str, u64, Option<u64>) + Send + Sync),
    ) -> OpsCtx<'a> {
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
            wait,
            progress,
            in_use: &nothing_in_use,
            residue: ResidueScan::none(),
        }
    }
}

fn quiet(_: &str, _: u64, _: Option<u64>) {}

#[tokio::test]
async fn plan_install_gives_a_confirmation_for_a_scoop_package() {
    let mut w = World::new().await;
    w.add("leaf").await;
    let ctx = w.ctx(LockWait::NoWait, &quiet);
    let Ok(Planned::Ready { text, action }) = jobs::plan_install(&ctx, "scoop:main", "leaf") else {
        panic!("expected Ready");
    };
    assert!(text.contains("Instalar leaf 1.0"), "{text}");
    assert!(matches!(action, Action::Apply(g) if g.len() == 1));
}

#[tokio::test]
async fn plan_install_reports_unknown_packages_and_non_scoop_sources() {
    let mut w = World::new().await;
    w.add("leaf").await;
    let ctx = w.ctx(LockWait::NoWait, &quiet);
    assert!(jobs::plan_install(&ctx, "scoop:main", "ghost").is_err());
    assert!(jobs::plan_install(&ctx, "winget", "leaf").is_err());
}

#[tokio::test]
async fn run_action_installs_reports_progress_and_then_removes() {
    let mut w = World::new().await;
    w.add("leaf").await;
    let seen: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    let record = |_: &str, done: u64, _: Option<u64>| seen.lock().unwrap().push(done);
    let ctx = w.ctx(LockWait::NoWait, &record);
    let Ok(Planned::Ready { action, .. }) = jobs::plan_install(&ctx, "scoop:main", "leaf") else {
        panic!("expected Ready");
    };
    let done = jobs::run_action(&ctx, action).await.unwrap();
    assert_eq!(done.summary, "Instalado: leaf");
    assert!(!seen.lock().unwrap().is_empty(), "progress was reported");
    assert!(w.layout.current("leaf").exists());

    let Ok(Planned::Ready { action, .. }) = jobs::plan_remove(&ctx, "leaf") else {
        panic!("expected Ready");
    };
    let done = jobs::run_action(&ctx, action).await.unwrap();
    assert_eq!(done.summary, "leaf removido");
    assert!(done.residues.is_some());
    assert!(!w.layout.current("leaf").exists());
}

#[tokio::test]
async fn held_install_lock_is_busy_not_a_failure_text() {
    let mut w = World::new().await;
    w.add("leaf").await;
    let ctx = w.ctx(LockWait::NoWait, &quiet);
    let Ok(Planned::Ready { action, .. }) = jobs::plan_install(&ctx, "scoop:main", "leaf") else {
        panic!("expected Ready");
    };
    let _held = UpdateLock::try_acquire(&w.layout.install_lock())
        .unwrap()
        .unwrap();
    let r = jobs::run_action(&ctx, action).await;
    assert_eq!(r.unwrap_err(), InstallFail::Busy);
    assert!(!w.layout.current("leaf").exists());
}

#[tokio::test]
async fn a_worker_panic_reaches_the_main_loop_and_is_reraised() {
    let handle = tokio::task::spawn_blocking(|| -> Msg { panic!("boom") });
    let msg = jobs::join_job(handle).await;
    assert!(matches!(msg, Msg::Panicked(_)));
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| jobs::propagate(msg)));
    let payload = caught.err().expect("propagate must unwind");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"boom"));
}

#[test]
fn ordinary_messages_pass_through_propagate() {
    assert!(matches!(jobs::propagate(Msg::Tick), Msg::Tick));
}

#[test]
fn progress_is_coalesced_to_about_twenty_per_second() {
    let base = Instant::now();
    let t = ProgressThrottle::new(base);
    let at = |ms: u64| base + Duration::from_millis(ms);
    assert!(t.admit(at(0), 1, Some(100)), "first goes through");
    assert!(!t.admit(at(10), 2, Some(100)));
    assert!(!t.admit(at(49), 3, Some(100)));
    assert!(t.admit(at(50), 4, Some(100)));
    assert!(!t.admit(at(60), 5, Some(100)));
    assert!(t.admit(at(61), 100, Some(100)), "the final one always does");
}
