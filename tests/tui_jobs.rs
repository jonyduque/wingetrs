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
    packages: Vec<NewPackage>,
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
            packages: Vec::new(),
        }
    }

    /// Serves a zip and puts a manifest for `id` in the in-memory `scoop:main` catalog.
    async fn add(&mut self, id: &str) {
        self.add_env(id, false).await;
    }

    /// Like `add`; with `env` the manifest also sets `<ID>_HOME`. Packages accumulate.
    async fn add_env(&mut self, id: &str, env: bool) {
        let zip = zip_of(&[(&format!("pkg/{id}.exe"), &fake_pe(3))]);
        let sha = hex::encode(Sha256::digest(&zip));
        let name = format!("{id}-1.0.zip");
        Mock::given(method("GET"))
            .and(path(format!("/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
            .mount(&self.server)
            .await;
        let mut m = serde_json::json!({
            "version": "1.0",
            "url": format!("{}/{name}", self.server.uri()),
            "hash": sha,
            "extract_dir": "pkg",
            "bin": format!("{id}.exe"),
        });
        if env {
            m["env_set"] = serde_json::json!({ format!("{}_HOME", id.to_uppercase()): "$dir" });
        }
        let raw = m.to_string();
        self.packages.retain(|p| p.id != id);
        self.packages.push(NewPackage {
            id: id.into(),
            name: id.into(),
            version: "1.0".into(),
            raw: Some(raw),
            ..Default::default()
        });
        self.catalog
            .replace_source(
                "scoop:main",
                SourceKind::Scoop,
                "v",
                1,
                &self.packages,
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
async fn run_action_applies_all_groups_with_one_broadcast() {
    let mut w = World::new().await;
    w.add_env("aa", true).await;
    w.add_env("bb", true).await;
    let ctx = w.ctx(LockWait::NoWait, &quiet);
    let mut groups = Vec::new();
    for id in ["aa", "bb"] {
        let Ok(Planned::Ready {
            action: Action::Apply(g),
            ..
        }) = jobs::plan_install(&ctx, "scoop:main", id)
        else {
            panic!("expected Ready");
        };
        groups.extend(g);
    }
    let done = jobs::run_action(&ctx, Action::Apply(groups)).await.unwrap();
    assert_eq!(done.summary, "Instalado: aa, bb");
    assert_eq!(w.env.dump().1, 1);
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
    let r = |done: u64| Some(("u1".to_string(), done, Some(100)));
    assert_eq!(
        t.admit(at(0), "u1", 1, Some(100)),
        r(1),
        "first goes through"
    );
    assert_eq!(t.admit(at(10), "u1", 2, Some(100)), None);
    assert_eq!(t.admit(at(49), "u1", 3, Some(100)), None);
    assert_eq!(t.admit(at(50), "u1", 4, Some(100)), r(4));
    assert_eq!(t.admit(at(60), "u1", 5, Some(100)), None);
    assert_eq!(
        t.admit(at(61), "u1", 100, Some(100)),
        r(100),
        "the final one always does"
    );
}

#[test]
fn parallel_downloads_are_summed_not_alternated() {
    let base = Instant::now();
    let t = ProgressThrottle::new(base);
    let at = |ms: u64| base + Duration::from_millis(ms);
    assert_eq!(
        t.admit(at(0), "u1", 10, Some(100)),
        Some(("u1".to_string(), 10, Some(100)))
    );
    assert_eq!(
        t.admit(at(60), "u2", 5, Some(50)),
        Some(("2 arquivos".to_string(), 15, Some(150)))
    );
    assert_eq!(
        t.admit(at(120), "u1", 40, Some(100)),
        Some(("2 arquivos".to_string(), 45, Some(150)))
    );
}

#[test]
fn unknown_total_makes_the_sum_unknown() {
    let base = Instant::now();
    let t = ProgressThrottle::new(base);
    let at = |ms: u64| base + Duration::from_millis(ms);
    assert!(t.admit(at(0), "u1", 10, Some(100)).is_some());
    assert_eq!(
        t.admit(at(60), "u2", 5, None),
        Some(("2 arquivos".to_string(), 15, None))
    );
}

#[test]
fn one_finished_file_does_not_bypass_the_gap_while_others_run() {
    let base = Instant::now();
    let t = ProgressThrottle::new(base);
    let at = |ms: u64| base + Duration::from_millis(ms);
    assert!(t.admit(at(0), "u1", 10, Some(100)).is_some());
    assert_eq!(t.admit(at(10), "u2", 50, Some(50)), None);
    assert_eq!(
        t.admit(at(20), "u1", 100, Some(100)),
        Some(("2 arquivos".to_string(), 150, Some(150)))
    );
}

#[test]
fn finished_batch_resets_for_the_next_app() {
    let base = Instant::now();
    let t = ProgressThrottle::new(base);
    let at = |ms: u64| base + Duration::from_millis(ms);
    assert_eq!(
        t.admit(at(0), "u1", 100, Some(100)),
        Some(("u1".to_string(), 100, Some(100)))
    );
    assert_eq!(
        t.admit(at(100), "u2", 10, Some(50)),
        Some(("u2".to_string(), 10, Some(50)))
    );
}

#[test]
fn read_catalog_reports_an_unreadable_file_instead_of_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    std::fs::write(&path, vec![b'x'; 4096]).unwrap();
    let err = jobs::read_catalog(&path).unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn read_catalog_of_a_good_catalog_lists_its_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut c = Catalog::open(&path).unwrap();
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "v",
        1,
        &[NewPackage {
            id: "ripgrep".into(),
            name: "ripgrep".into(),
            version: "1".into(),
            ..Default::default()
        }],
        &WingetMaps::default(),
    )
    .unwrap();
    drop(c);
    assert_eq!(jobs::read_catalog(&path).unwrap().len(), 1);
}

#[tokio::test]
async fn plan_remove_text_uses_the_plan_lines() {
    let mut w = World::new().await;
    w.add("leaf").await;
    let ctx = w.ctx(LockWait::NoWait, &quiet);
    let Ok(Planned::Ready { action, .. }) = jobs::plan_install(&ctx, "scoop:main", "leaf") else {
        panic!("expected Ready");
    };
    jobs::run_action(&ctx, action).await.unwrap();
    let Ok(Planned::Ready { text, .. }) = jobs::plan_remove(&ctx, "leaf") else {
        panic!("expected Ready");
    };
    assert!(text.contains("Desfazer entrada no PATH: "), "{text}");
    assert!(!text.contains("Path ") && !text.contains("Shim "), "{text}");
}

#[test]
fn save_settings_keeps_buckets() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(
        &p,
        "[[buckets]]
name = \"x\"
repo = \"o/x\"
",
    )
    .unwrap();
    jobs::save_settings(
        &p,
        wingetrs::config::Behavior {
            purge: true,
            ..Default::default()
        },
        "ascii",
        "light",
    )
    .unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("name = \"x\""), "{text}");
    assert!(text.contains("purge = true"), "{text}");
    assert!(text.contains("glyphs = \"ascii\""), "{text}");
    assert!(text.contains("theme = \"light\""), "{text}");
}
