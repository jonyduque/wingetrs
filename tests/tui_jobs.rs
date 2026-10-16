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
            msiexec: wingetrs::install::executor::Msiexec::DISABLED,
        }
    }
}

fn quiet(_: &str, _: u64, _: Option<u64>) {}

#[tokio::test]
async fn plan_install_gives_a_confirmation_for_a_scoop_package() {
    let mut w = World::new().await;
    w.add("leaf").await;
    let ctx = w.ctx(LockWait::NoWait, &quiet);
    let Ok(Planned::Ready { text, action }) = jobs::plan_install(&ctx, "scoop:main", "leaf", false)
    else {
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
    assert!(jobs::plan_install(&ctx, "scoop:main", "ghost", false).is_err());
    assert!(jobs::plan_install(&ctx, "winget", "leaf", false).is_err());
}

#[tokio::test]
async fn run_action_installs_reports_progress_and_then_removes() {
    let mut w = World::new().await;
    w.add("leaf").await;
    let seen: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    let record = |_: &str, done: u64, _: Option<u64>| seen.lock().unwrap().push(done);
    let ctx = w.ctx(LockWait::NoWait, &record);
    let Ok(Planned::Ready { action, .. }) = jobs::plan_install(&ctx, "scoop:main", "leaf", false)
    else {
        panic!("expected Ready");
    };
    let done = jobs::run_action(&ctx, action).await.unwrap();
    assert_eq!(done.summary, "Instalado: leaf");
    assert!(!seen.lock().unwrap().is_empty(), "progress was reported");
    assert!(w.layout.current("leaf").exists());

    let Ok(Planned::Ready { action, .. }) = jobs::plan_remove(&ctx, "leaf", false) else {
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
        }) = jobs::plan_install(&ctx, "scoop:main", id, false)
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
    let Ok(Planned::Ready { action, .. }) = jobs::plan_install(&ctx, "scoop:main", "leaf", false)
    else {
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

struct Writable(PathBuf);
impl Drop for Writable {
    fn drop(&mut self) {
        if let Ok(meta) = std::fs::metadata(&self.0) {
            let mut p = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            p.set_readonly(false);
            let _ = std::fs::set_permissions(&self.0, p);
        }
    }
}

#[test]
fn reading_the_catalog_never_writes_it() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("catalog.db");
    let mut c = Catalog::open(&db).unwrap();
    let pkgs: Vec<NewPackage> = ["a", "b"]
        .iter()
        .map(|id| NewPackage {
            id: (*id).into(),
            name: (*id).into(),
            version: "1".into(),
            ..Default::default()
        })
        .collect();
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "v",
        1,
        &pkgs,
        &WingetMaps::default(),
    )
    .unwrap();
    drop(c);
    let mut p = std::fs::metadata(&db).unwrap().permissions();
    p.set_readonly(true);
    std::fs::set_permissions(&db, p).unwrap();
    let _guard = Writable(db.clone());
    let listed = jobs::read_catalog(&db);
    let got = jobs::open_catalog_for_reading(&db)
        .unwrap()
        .get("scoop:main", "a");
    assert_eq!(listed.unwrap().len(), 2);
    assert!(got.unwrap().is_some());
}

#[test]
fn reading_never_runs_the_schema_ddl() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("catalog.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE unrelated (x INTEGER);")
        .unwrap();
    drop(conn);
    let before = std::fs::read(&db).unwrap();
    // The write path would create the catalog tables and succeed with an empty list.
    assert!(jobs::read_catalog(&db).is_err());
    assert_eq!(before, std::fs::read(&db).unwrap());
}

#[test]
fn reading_a_missing_catalog_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("catalog.db");
    assert_eq!(jobs::read_catalog(&db), Ok(vec![]));
    assert!(!db.exists());
}

#[tokio::test]
async fn plan_remove_text_uses_the_plan_lines() {
    let mut w = World::new().await;
    w.add("leaf").await;
    let ctx = w.ctx(LockWait::NoWait, &quiet);
    let Ok(Planned::Ready { action, .. }) = jobs::plan_install(&ctx, "scoop:main", "leaf", false)
    else {
        panic!("expected Ready");
    };
    jobs::run_action(&ctx, action).await.unwrap();
    let Ok(Planned::Ready { text, .. }) = jobs::plan_remove(&ctx, "leaf", false) else {
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
        Some("ascii"),
        None,
        None,
        "light",
    )
    .unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("name = \"x\""), "{text}");
    assert!(text.contains("purge = true"), "{text}");
    assert!(text.contains("glyphs = \"ascii\""), "{text}");
    assert!(text.contains("theme = \"light\""), "{text}");
}

#[test]
fn save_settings_without_glyphs_keeps_the_file_value() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().join("config.toml");
    std::fs::write(&p, "glyphs = \"unicode\"\n").unwrap();
    jobs::save_settings(&p, Default::default(), None, None, None, "dark").unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("glyphs = \"unicode\""), "{text}");
}

#[tokio::test]
async fn purge_plan_says_data_will_be_deleted() {
    let mut w = World::new().await;
    w.add("leaf").await;
    let ctx = w.ctx(LockWait::NoWait, &quiet);
    let Ok(Planned::Ready { action, .. }) = jobs::plan_install(&ctx, "scoop:main", "leaf", false)
    else {
        panic!("expected Ready");
    };
    jobs::run_action(&ctx, action).await.unwrap();
    let Ok(Planned::Ready {
        text,
        action: Action::Remove(p),
    }) = jobs::plan_remove(&ctx, "leaf", true)
    else {
        panic!("expected Ready remove");
    };
    assert!(text.contains("APAGADOS"), "{text}");
    assert!(p.purge);
    let Ok(Planned::Ready {
        text,
        action: Action::Remove(p),
    }) = jobs::plan_remove(&ctx, "leaf", false)
    else {
        panic!("expected Ready remove");
    };
    assert!(text.contains("mantidos"), "{text}");
    assert!(!p.purge);
}

mod prefetch {
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio::sync::Semaphore;
    use wingetrs::tui::run::{MAX_LIST_FETCH, prefetch_list};

    fn keys(n: usize) -> Vec<(String, String)> {
        (0..n).map(|i| ("winget".into(), format!("k{i}"))).collect()
    }

    fn slots() -> Arc<Semaphore> {
        Arc::new(Semaphore::new(MAX_LIST_FETCH))
    }

    fn running_set() -> Arc<Mutex<HashSet<String>>> {
        Arc::default()
    }

    /// Waits (hang detector: 10 s) until `n` reaches `want`.
    async fn wait_for(n: &AtomicUsize, want: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while n.load(Ordering::SeqCst) < want {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("never reached");
    }

    #[tokio::test]
    async fn prefetch_drops_superseded_generation() {
        let generation = AtomicU64::new(0);
        let slots = slots();
        let (old, new) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let counting = |n: &Arc<AtomicUsize>| {
            let n = Arc::clone(n);
            move |_: String, _: String| {
                let n = Arc::clone(&n);
                async move {
                    n.fetch_add(1, Ordering::SeqCst);
                }
            }
        };
        let debounce = Duration::from_millis(50);
        let running = running_set();
        tokio::join!(
            prefetch_list(
                &generation,
                1,
                debounce,
                &slots,
                &running,
                keys(3),
                counting(&old)
            ),
            prefetch_list(
                &generation,
                2,
                debounce,
                &slots,
                &running,
                keys(3),
                counting(&new)
            ),
        );
        assert_eq!(old.load(Ordering::SeqCst), 0, "the superseded one fetched");
        assert_eq!(new.load(Ordering::SeqCst), 3);
    }

    /// Fetches that count themselves and wait for `gate` before ending.
    fn gated(
        started: &Arc<AtomicUsize>,
        running: &Arc<AtomicUsize>,
        peak: &Arc<AtomicUsize>,
        gate: &Arc<Semaphore>,
    ) -> impl Fn(String, String) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + use<> {
        let (started, running, peak, gate) = (
            Arc::clone(started),
            Arc::clone(running),
            Arc::clone(peak),
            Arc::clone(gate),
        );
        move |_, _| {
            let (started, running, peak, gate) = (
                Arc::clone(&started),
                Arc::clone(&running),
                Arc::clone(&peak),
                Arc::clone(&gate),
            );
            Box::pin(async move {
                started.fetch_add(1, Ordering::SeqCst);
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                gate.acquire().await.unwrap().forget();
                running.fetch_sub(1, Ordering::SeqCst);
            })
        }
    }

    #[tokio::test]
    async fn prefetch_runs_at_most_four_at_once() {
        let counters: [Arc<AtomicUsize>; 3] = Default::default();
        let [started, running, peak] = &counters;
        let gate = Arc::new(Semaphore::new(0));
        let generation = Arc::new(AtomicU64::new(0));
        let fetch = gated(started, running, peak, &gate);
        let run = tokio::spawn({
            let generation = Arc::clone(&generation);
            async move {
                prefetch_list(
                    &generation,
                    1,
                    Duration::ZERO,
                    &slots(),
                    &running_set(),
                    keys(8),
                    fetch,
                )
                .await
            }
        });
        wait_for(started, MAX_LIST_FETCH).await;
        // Room for a fifth to start if the limit were missing.
        tokio::time::sleep(Duration::from_millis(50)).await;
        gate.add_permits(8);
        run.await.unwrap();
        assert_eq!(started.load(Ordering::SeqCst), 8);
        assert_eq!(peak.load(Ordering::SeqCst), MAX_LIST_FETCH);
    }

    #[tokio::test]
    async fn prefetch_checks_the_generation_before_each_package() {
        let counters: [Arc<AtomicUsize>; 3] = Default::default();
        let [started, running, peak] = &counters;
        let gate = Arc::new(Semaphore::new(0));
        let generation = Arc::new(AtomicU64::new(0));
        let fetch = gated(started, running, peak, &gate);
        let run = tokio::spawn({
            let generation = Arc::clone(&generation);
            async move {
                prefetch_list(
                    &generation,
                    1,
                    Duration::ZERO,
                    &slots(),
                    &running_set(),
                    keys(8),
                    fetch,
                )
                .await
            }
        });
        wait_for(started, MAX_LIST_FETCH).await;
        // A newer screen while the first four run: the other four never start.
        generation.store(2, Ordering::SeqCst);
        gate.add_permits(8);
        run.await.unwrap();
        assert_eq!(started.load(Ordering::SeqCst), MAX_LIST_FETCH);
    }

    #[tokio::test]
    async fn older_generation_never_wins() {
        let generation = AtomicU64::new(0);
        let slots = slots();
        let (newer, older) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let counting = |n: &Arc<AtomicUsize>| {
            let n = Arc::clone(n);
            move |_: String, _: String| {
                let n = Arc::clone(&n);
                async move {
                    n.fetch_add(1, Ordering::SeqCst);
                }
            }
        };
        let debounce = Duration::from_millis(50);
        let running = running_set();
        // The tasks ran out of order: generation 6 marks itself before generation 5.
        tokio::join!(
            prefetch_list(
                &generation,
                6,
                debounce,
                &slots,
                &running,
                keys(3),
                counting(&newer)
            ),
            prefetch_list(
                &generation,
                5,
                debounce,
                &slots,
                &running,
                keys(2),
                counting(&older)
            ),
        );
        assert_eq!(newer.load(Ordering::SeqCst), 3);
        assert_eq!(older.load(Ordering::SeqCst), 0, "the older one fetched");
    }

    #[tokio::test]
    async fn prefetch_reraises_a_package_panic_after_the_others_end() {
        let done = Arc::new(AtomicUsize::new(0));
        let fetch = {
            let done = Arc::clone(&done);
            move |_: String, id: String| {
                let done = Arc::clone(&done);
                async move {
                    assert_ne!(id, "k1", "boom");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    done.fetch_add(1, Ordering::SeqCst);
                }
            }
        };
        let set = running_set();
        let run = tokio::spawn({
            let set = Arc::clone(&set);
            async move {
                let generation = AtomicU64::new(0);
                prefetch_list(
                    &generation,
                    1,
                    Duration::ZERO,
                    &slots(),
                    &set,
                    keys(8),
                    fetch,
                )
                .await
            }
        });
        let err = run.await.unwrap_err();
        assert!(err.is_panic());
        assert_eq!(done.load(Ordering::SeqCst), 7);
        assert!(
            set.lock().unwrap().is_empty(),
            "the panicked key is no longer running"
        );
    }

    #[tokio::test]
    async fn running_key_is_not_started_twice() {
        let set = running_set();
        // An older generation is still fetching k1; it will report it when done.
        set.lock().unwrap().insert("winget/k1".into());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let fetch = {
            let (seen, set) = (Arc::clone(&seen), Arc::clone(&set));
            move |_: String, id: String| {
                let (seen, set) = (Arc::clone(&seen), Arc::clone(&set));
                async move {
                    assert!(set.lock().unwrap().contains(&format!("winget/{id}")));
                    seen.lock().unwrap().push(id);
                }
            }
        };
        let generation = AtomicU64::new(0);
        prefetch_list(
            &generation,
            1,
            Duration::ZERO,
            &slots(),
            &set,
            keys(3),
            fetch,
        )
        .await;
        let mut seen = seen.lock().unwrap().clone();
        seen.sort();
        assert_eq!(seen, vec!["k0".to_string(), "k2".to_string()]);
        let left: Vec<String> = set.lock().unwrap().iter().cloned().collect();
        assert_eq!(left, vec!["winget/k1".to_string()]);
    }
}

fn twin_candidate(id: &str) -> wingetrs::catalog::StoredPackage {
    wingetrs::catalog::StoredPackage {
        entry: wingetrs::core::PackageEntry {
            source: SourceKind::Winget,
            source_name: "winget".into(),
            id: id.into(),
            name: id.into(),
            moniker: None,
            version: "1".into(),
            publisher: None,
            description: None,
            tags: Vec::new(),
            commands: Vec::new(),
        },
        arp_min_version: None,
        arp_max_version: None,
        winget_hash: Some(vec![7; 32]),
        raw: None,
    }
}

#[tokio::test]
async fn offline_is_retry_not_absent() {
    use wingetrs::cache::BlobStore;
    use wingetrs::core::{IconRef, PackageDetails};
    use wingetrs::tui::app::IconOutcome;
    use wingetrs::tui::run::{fetch_icon, own_icon, twin_icon_from};
    let dir = TempDir::new().unwrap();
    let blobs = BlobStore::new(dir.path().join("blobs"));
    let http = Http::new().unwrap();
    // Bound then dropped: nothing listens there, so the connection is refused.
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/", l.local_addr().unwrap())
    };
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gone.png"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/busy.png"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let png = b"not really a png".to_vec();
    Mock::given(method("GET"))
        .and(path("/ok.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(png.clone()))
        .mount(&server)
        .await;
    // Every path answers 503: a twin candidate's details on a busy CDN.
    let busy = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&busy)
        .await;
    let icon = |url: String| IconRef {
        url,
        sha256: "ab".repeat(32),
    };

    // Details loaded, the icon host refuses the connection.
    let got = fetch_icon(&http, &blobs, &icon(format!("{closed}i.png"))).await;
    assert_eq!(got.err(), Some(IconOutcome::Retry));
    // The icon is gone from its host.
    let got = fetch_icon(&http, &blobs, &icon(format!("{}/gone.png", server.uri()))).await;
    assert_eq!(got.err(), Some(IconOutcome::Absent));
    // The icon host is in trouble for now.
    let got = fetch_icon(&http, &blobs, &icon(format!("{}/busy.png", server.uri()))).await;
    assert_eq!(got.err(), Some(IconOutcome::Retry));
    // Downloaded and verified, but the blob store cannot be written (its dir is a file).
    let not_a_dir = dir.path().join("file");
    std::fs::write(&not_a_dir, b"x").unwrap();
    let ok = IconRef {
        url: format!("{}/ok.png", server.uri()),
        sha256: hex::encode(Sha256::digest(&png)),
    };
    let got = fetch_icon(&http, &BlobStore::new(not_a_dir), &ok).await;
    assert_eq!(got.err(), Some(IconOutcome::Retry));
    // A manifest without `Icons`.
    let bare = PackageDetails {
        source: SourceKind::Winget,
        id: "Acme.Tool".into(),
        name: "Tool".into(),
        version: "1".into(),
        publisher: None,
        description: None,
        homepage: Some("https://acme.org".into()),
        license: None,
        tags: Vec::new(),
        installers: Vec::new(),
        available_versions: Vec::new(),
        icon: None,
    };
    assert_eq!(own_icon(&bare).err(), Some(IconOutcome::Absent));

    // Scoop: the twin candidate's details fail on the connection, or with a 404.
    let twin = |base: String| {
        let (http, blobs) = (&http, &blobs);
        async move {
            let pkgs = [twin_candidate("Acme.Tool")];
            twin_icon_from(&pkgs, "https://acme.org", |p| {
                let base = base.clone();
                async move {
                    let hash = p.winget_hash.clone().unwrap_or_default();
                    Ok(wingetrs::sources::winget::details::details(
                        http,
                        blobs,
                        &base,
                        &p.entry.id,
                        &hash,
                        None,
                    )
                    .await?)
                }
            })
            .await
            .err()
        }
    };
    assert_eq!(twin(closed.clone()).await, Some(IconOutcome::Retry));
    assert_eq!(
        twin(format!("{}/", busy.uri())).await,
        Some(IconOutcome::Retry)
    );
    assert_eq!(
        twin(format!("{}/", server.uri())).await,
        Some(IconOutcome::Absent)
    );
}
use wingetrs::bridge::{BridgeJob, Tool};
use wingetrs::tui::app::Done;

fn bridge_job(tool: Tool, target: &str) -> BridgeJob {
    BridgeJob {
        tool,
        target: target.into(),
        name: format!("App {target}"),
    }
}

/// A fresh `msi.lock` per call, so parallel bridge tests never queue behind each other; the
/// directory goes away with the value.
struct MsiLock {
    _dir: TempDir,
    path: PathBuf,
}

impl std::ops::Deref for MsiLock {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

fn msi_lock() -> MsiLock {
    let dir = TempDir::new().unwrap();
    let path = Layout::new(dir.path()).msi_lock();
    MsiLock { _dir: dir, path }
}

fn scoop_manifest(root: &Path, app: &str, version: &str) {
    let dir = root.join("apps").join(app).join("current");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        format!(r#"{{"version": "{version}"}}"#),
    )
    .unwrap();
}

#[test]
fn run_bridge_winget_reports_codes_lines_and_missing_tool() {
    let root = Path::new("C:/nao-usado");
    let job = bridge_job(Tool::Winget, "Git.Git");
    let mut calls: Vec<(String, Vec<String>)> = Vec::new();
    let mut seen = Vec::new();
    let mut exec = |program: &str, args: &[String], on_line: &mut dyn FnMut(&str)| {
        calls.push((program.to_string(), args.to_vec()));
        on_line("Baixando");
        Ok((0, vec!["Baixando".to_string()]))
    };
    let done = jobs::run_bridge(
        &job,
        Some(root),
        &msi_lock(),
        "pwsh",
        &mut exec,
        &mut |l: &str| seen.push(l.to_string()),
    )
    .unwrap();
    assert_eq!(
        done,
        Done {
            summary: "App Git.Git: Atualizado.".into(),
            residues: None
        }
    );
    assert_eq!(seen, ["Baixando"]);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "winget");
    assert!(calls[0].1.contains(&"--disable-interactivity".to_string()));

    let mut not_found = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| {
        Ok((-1978335212, vec!["Nenhum pacote".to_string()]))
    };
    assert_eq!(
        jobs::run_bridge(
            &job,
            Some(root),
            &msi_lock(),
            "pwsh",
            &mut not_found,
            &mut |_: &str| {}
        ),
        Err(InstallFail::Other(
            "App Git.Git: O winget não encontrou este pacote instalado.".into()
        ))
    );

    let mut missing = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| {
        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
    };
    assert_eq!(
        jobs::run_bridge(
            &job,
            Some(root),
            &msi_lock(),
            "pwsh",
            &mut missing,
            &mut |_: &str| {}
        ),
        Err(InstallFail::Other(
            "App Git.Git: winget não encontrado".into()
        ))
    );
    let scoop = bridge_job(Tool::Scoop, "7zip");
    assert_eq!(
        jobs::run_bridge(
            &scoop,
            Some(root),
            &msi_lock(),
            "pwsh",
            &mut missing,
            &mut |_: &str| {}
        ),
        Err(InstallFail::Other("App 7zip: scoop não encontrado".into()))
    );
}

#[test]
fn run_bridge_refuses_an_invalid_target_before_running() {
    let mut ran = false;
    let mut exec = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| {
        ran = true;
        Ok((0, Vec::new()))
    };
    for target in ["-x", "a&b", r"..\x"] {
        let r = jobs::run_bridge(
            &bridge_job(Tool::Scoop, target),
            Some(Path::new("C:/s")),
            &msi_lock(),
            "pwsh",
            &mut exec,
            &mut |_: &str| {},
        );
        assert!(matches!(r, Err(InstallFail::Other(_))), "{target}");
    }
    assert!(!ran);
}

#[test]
fn run_bridge_scoop_compares_versions_read_around_the_run() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    scoop_manifest(&root, "7zip", "1.0");
    let job = bridge_job(Tool::Scoop, "7zip");

    // Exit 0, new version written by the "tool" during the run: upgraded.
    let mut upgrades = |program: &str, args: &[String], _: &mut dyn FnMut(&str)| {
        assert_eq!(program, "pwsh");
        assert!(args[4].contains("update 7zip"));
        scoop_manifest(&root, "7zip", "2.0");
        Ok((0, Vec::new()))
    };
    let done = jobs::run_bridge(
        &job,
        Some(&root),
        &msi_lock(),
        "pwsh",
        &mut upgrades,
        &mut |_: &str| {},
    )
    .unwrap();
    assert_eq!(done.summary, "App 7zip: Atualizado para a versão 2.0.");

    // Exit 0, same version: not an upgrade, whatever the code says.
    let mut stays = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| {
        Ok((0, vec!["7zip: 2.0 (latest version)".to_string()]))
    };
    assert_eq!(
        jobs::run_bridge(
            &job,
            Some(&root),
            &msi_lock(),
            "pwsh",
            &mut stays,
            &mut |_: &str| {}
        ),
        Err(InstallFail::Other(
            "App 7zip: Já está na versão mais recente.".into()
        ))
    );

    let mut fails = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| Ok((1, Vec::new()));
    assert_eq!(
        jobs::run_bridge(
            &job,
            Some(&root),
            &msi_lock(),
            "pwsh",
            &mut fails,
            &mut |_: &str| {}
        ),
        Err(InstallFail::Other(
            "App 7zip: O scoop falhou (código 1).".into()
        ))
    );
}

#[test]
fn run_bridge_scoop_without_a_root_fails_before_running() {
    let mut ran = false;
    let mut exec = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| {
        ran = true;
        Ok((0, Vec::new()))
    };
    assert_eq!(
        jobs::run_bridge(
            &bridge_job(Tool::Scoop, "7zip"),
            None,
            &msi_lock(),
            "pwsh",
            &mut exec,
            &mut |_: &str| {}
        ),
        Err(InstallFail::Other("App 7zip: scoop não encontrado".into()))
    );
    assert!(!ran);
    // winget needs no scoop root.
    let mut ok = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| Ok((0, Vec::new()));
    assert!(
        jobs::run_bridge(
            &bridge_job(Tool::Winget, "Git.Git"),
            None,
            &msi_lock(),
            "pwsh",
            &mut ok,
            &mut |_: &str| {}
        )
        .is_ok()
    );
}

#[test]
fn bridge_job_toasts_reboot() {
    let job = bridge_job(Tool::Winget, "Git.Git");
    let mut exec = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| {
        Ok((
            0,
            vec!["Reiniciar seu PC para terminar a instalação.".to_string()],
        ))
    };
    let done = jobs::run_bridge(
        &job,
        Some(Path::new("C:/x")),
        &msi_lock(),
        "pwsh",
        &mut exec,
        &mut |_: &str| {},
    )
    .unwrap();
    assert!(done.summary.contains("reinicie"), "{}", done.summary);
}

#[test]
fn bridge_winget_takes_msi_lock() {
    use std::sync::mpsc::channel;
    const HANG: Duration = Duration::from_secs(60);
    let dir = TempDir::new().unwrap();
    let lock_path = Layout::new(dir.path()).msi_lock();
    let (tx, rx) = channel::<String>();
    std::thread::scope(|s| {
        // Declared inside the scope: a failing assertion drops it before the scope joins.
        let held = UpdateLock::try_acquire(&lock_path).unwrap().unwrap();

        // scoop never queues: it runs and finishes while the lock is held.
        let scoop = s.spawn(|| {
            let mut exec =
                |_: &str, _: &[String], _: &mut dyn FnMut(&str)| Ok((1, Vec::<String>::new()));
            jobs::run_bridge(
                &bridge_job(Tool::Scoop, "7zip"),
                Some(Path::new("C:/nao-usado")),
                &lock_path,
                "pwsh",
                &mut exec,
                &mut |_: &str| {},
            )
        });
        let scoop_result = match scoop.join() {
            Ok(r) => r,
            Err(p) => std::panic::resume_unwind(p),
        };
        assert_eq!(
            scoop_result,
            Err(InstallFail::Other(
                "App 7zip: O scoop falhou (código 1).".into()
            ))
        );

        let winget = {
            let tx = tx.clone();
            let lock_path = &lock_path;
            s.spawn(move || {
                let exec_tx = tx.clone();
                let mut exec = move |program: &str, _: &[String], _: &mut dyn FnMut(&str)| {
                    // msi.lock stays held while winget runs.
                    let held = UpdateLock::try_acquire(lock_path).unwrap().is_none();
                    exec_tx.send(format!("exec {program} held={held}")).unwrap();
                    Ok((0, Vec::new()))
                };
                jobs::run_bridge(
                    &bridge_job(Tool::Winget, "Git.Git"),
                    None,
                    lock_path,
                    "pwsh",
                    &mut exec,
                    &mut |l: &str| tx.send(format!("line {l}")).unwrap(),
                )
            })
        };
        let first = rx.recv_timeout(HANG);
        tx.send("released".to_string()).unwrap();
        drop(held);
        let done = match winget.join() {
            Ok(r) => r,
            Err(p) => std::panic::resume_unwind(p),
        };
        assert_eq!(first.unwrap(), format!("line {}", jobs::WAITING_INSTALLER));
        assert!(done.is_ok(), "{done:?}");
    });
    drop(tx);
    let events: Vec<String> = rx.iter().collect();
    assert_eq!(
        events,
        [
            "released".to_string(),
            format!("line {}", jobs::STARTING_WINGET),
            "exec winget held=true".to_string()
        ]
    );
}
// ---- msiexec /a fallback in the jobs (Task 129) ----

use wingetrs::install::executor::Msiexec;
use wingetrs::install::msi_queue::Attempt;

/// The smallest MSI the own extractor refuses: files outside the package (word count 0).
fn uncompressed_msi() -> Vec<u8> {
    use msi::{Package, PackageType};
    let mut p = Package::create(PackageType::Installer, std::io::Cursor::new(Vec::new())).unwrap();
    p.summary_info_mut().set_word_count(0);
    p.into_inner().unwrap().into_inner()
}

impl World {
    /// Serves `uncompressed_msi` as `<id>-1.0.msi` and puts its manifest in the catalog.
    async fn add_msi(&mut self, id: &str) {
        let bytes = uncompressed_msi();
        let sha = hex::encode(Sha256::digest(&bytes));
        let name = format!("{id}-1.0.msi");
        Mock::given(method("GET"))
            .and(path(format!("/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
            .mount(&self.server)
            .await;
        let raw = serde_json::json!({
            "version": "1.0",
            "url": format!("{}/{name}", self.server.uri()),
            "hash": sha,
        })
        .to_string();
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
}

fn never_busy() -> bool {
    false
}

fn fake_admin(_: &Path, target: &Path, _: &Path) -> std::io::Result<Attempt> {
    std::fs::create_dir_all(target)?;
    std::fs::write(target.join("tool.txt"), b"tool")?;
    Ok(Attempt::Done)
}

fn planned_groups(
    ctx: &wingetrs::install::ops::OpsCtx<'_>,
    id: &str,
) -> Vec<Vec<wingetrs::install::plan::InstallPlan>> {
    match jobs::plan_install(ctx, "scoop:main", id, false) {
        Ok(Planned::Ready {
            action: Action::Apply(groups),
            ..
        }) => groups,
        other => panic!("expected Ready: {other:?}"),
    }
}

#[tokio::test]
async fn run_action_needs_msiexec_then_runs_the_allowed_item_only() {
    let mut w = World::new().await;
    w.add_msi("m").await;
    let mut ctx = w.ctx(LockWait::NoWait, &quiet);
    ctx.msiexec = Msiexec {
        busy: never_busy,
        launch: fake_admin,
    };
    let groups = planned_groups(&ctx, "m");
    let r = jobs::run_action(&ctx, Action::Apply(groups.clone())).await;
    match r {
        Err(InstallFail::NeedsMsiexec {
            app,
            reason,
            applied,
        }) => {
            assert_eq!(app, "m");
            assert!(reason.contains("sem compressão"), "{reason}");
            assert!(applied.is_empty());
        }
        other => panic!("expected NeedsMsiexec: {other:?}"),
    }
    // Allowed for another app: `m` still needs its own confirmation.
    let other = Action::ApplyMsiexec {
        groups: groups.clone(),
        app: "outro".into(),
    };
    assert!(matches!(
        jobs::run_action(&ctx, other).await,
        Err(InstallFail::NeedsMsiexec { .. })
    ));
    let allowed = Action::ApplyMsiexec {
        groups,
        app: "m".into(),
    };
    let done = jobs::run_action(&ctx, allowed).await.unwrap();
    assert_eq!(done.summary, "Instalado: m");
    assert_eq!(
        std::fs::read(w.layout.current("m").join("tool.txt")).unwrap(),
        b"tool"
    );
}

static ORDER: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn order_push(e: &str) {
    ORDER.lock().unwrap().push(e.to_string());
}

/// Waits (10 s hang detector) until `ORDER` has `e`.
fn order_wait(e: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ORDER.lock().unwrap().iter().any(|x| x == e) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

/// Holds the Windows Installer turn until the bridge job queued behind `msi.lock`.
fn admin_waiting_for_bridge(_: &Path, target: &Path, _: &Path) -> std::io::Result<Attempt> {
    // SourceDir < _msiexec < .staging-1.0 < app < apps < root.
    let root = target.ancestors().nth(5).unwrap();
    let layout = Layout::new(root);
    let install_held = UpdateLock::try_acquire(&layout.install_lock())
        .unwrap()
        .is_none();
    let msi_held = UpdateLock::try_acquire(&layout.msi_lock())
        .unwrap()
        .is_none();
    order_push(&format!(
        "msiexec start install.lock={install_held} msi.lock={msi_held}"
    ));
    if !order_wait(&format!("bridge: {}", jobs::WAITING_INSTALLER)) {
        return Err(std::io::Error::other("a ponte não entrou na fila"));
    }
    std::fs::create_dir_all(target)?;
    std::fs::write(target.join("tool.txt"), b"tool")?;
    order_push("msiexec end");
    Ok(Attempt::Done)
}

#[tokio::test(flavor = "multi_thread")]
async fn msiexec_fallback_and_bridge_job_queue_without_deadlock() {
    let mut w = World::new().await;
    w.add_msi("m").await;
    let mut ctx = w.ctx(LockWait::NoWait, &quiet);
    ctx.msiexec = Msiexec {
        busy: never_busy,
        launch: admin_waiting_for_bridge,
    };
    let groups = planned_groups(&ctx, "m");
    let lock_path = w.layout.msi_lock();
    let bridge = std::thread::spawn(move || {
        if !order_wait("msiexec start install.lock=true msi.lock=true") {
            return Err(InstallFail::Other("o msiexec não começou".into()));
        }
        let mut exec = |program: &str, _: &[String], _: &mut dyn FnMut(&str)| {
            order_push(&format!("{program} run"));
            Ok((0, Vec::new()))
        };
        jobs::run_bridge(
            &bridge_job(Tool::Winget, "Git.Git"),
            None,
            &lock_path,
            "pwsh",
            &mut exec,
            &mut |line: &str| order_push(&format!("bridge: {line}")),
        )
    });
    let install = jobs::run_action(
        &ctx,
        Action::ApplyMsiexec {
            groups,
            app: "m".into(),
        },
    )
    .await;
    let bridged = match bridge.join() {
        Ok(r) => r,
        Err(p) => std::panic::resume_unwind(p),
    };
    install.unwrap();
    bridged.unwrap();
    assert_eq!(
        *ORDER.lock().unwrap(),
        [
            "msiexec start install.lock=true msi.lock=true".to_string(),
            format!("bridge: {}", jobs::WAITING_INSTALLER),
            "msiexec end".to_string(),
            format!("bridge: {}", jobs::STARTING_WINGET),
            "winget run".to_string(),
        ]
    );
}
