#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Operations layer: dependencies, lock, apply, uninstall, upgrade listing, repair and list.
use crate::spans;

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wingetrs::cache::{DataDir, UpdateLock};
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::core::SourceKind;
use wingetrs::http::Http;
use wingetrs::install::layout::Layout;
use wingetrs::install::ops::{
    OpsCtx, OpsError, ResidueScan, Resolution, apply, apply_groups, merge_groups, plan_for,
    plan_uninstall, repair, resolve_installed, resolve_target, uninstall, upgradable,
};
use wingetrs::install::plan::InstallPlan;
use wingetrs::install::residue::{Confidence, ResidueKind};
use wingetrs::install::state::{ArtifactKind, StateDb};
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{Effects, EnvStore, EnvValue};
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

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn fx(n: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/scoop/manifests/{n}")).unwrap()
}

fn no_progress(_: &wingetrs::install::progress::ProgressEvent) {}
fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}
fn exe_in_use(dir: &Path) -> Vec<PathBuf> {
    vec![dir.join("current").join("busy.exe")]
}

const ORIGINAL_PATH: &str = r"C:\Existing";

struct App<'a> {
    id: &'a str,
    version: &'a str,
    depends: &'a [&'a str],
    env: bool,
    delay_ms: u64,
}

fn app<'a>(id: &'a str, depends: &'a [&'a str]) -> App<'a> {
    App {
        id,
        version: "1.0",
        depends,
        env: false,
        delay_ms: 0,
    }
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
    /// Arrival instant of every download request, in arrival order.
    arrivals: std::sync::Arc<std::sync::Mutex<Vec<std::time::Instant>>>,
}

/// Responder that records when each request arrives, then answers with `template`.
struct Recording {
    arrivals: std::sync::Arc<std::sync::Mutex<Vec<std::time::Instant>>>,
    template: ResponseTemplate,
}

impl wiremock::Respond for Recording {
    fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
        self.arrivals
            .lock()
            .unwrap()
            .push(std::time::Instant::now());
        self.template.clone()
    }
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
                value: ORIGINAL_PATH.into(),
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
            arrivals: Default::default(),
        }
    }

    /// Puts `raw` in the in-memory `scoop:main` catalog as `id` (replacing an older entry).
    fn put_raw(&mut self, id: &str, version: &str, raw: String) {
        self.packages.retain(|p| p.id != id);
        self.packages.push(NewPackage {
            id: id.into(),
            name: id.into(),
            version: version.into(),
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

    /// Serves a zip with `pkg/<id>.exe` and adds a manifest pointing at it; returns the raw JSON.
    async fn add(&mut self, a: App<'_>) -> String {
        let zip = zip_of(&[(&format!("pkg/{}.exe", a.id), &fake_pe(3))]);
        let sha = sha256_hex(&zip);
        let name = format!("{}-{}.zip", a.id, a.version);
        Mock::given(method("GET"))
            .and(path(format!("/{name}")))
            .respond_with(Recording {
                arrivals: self.arrivals.clone(),
                template: ResponseTemplate::new(200)
                    .set_body_bytes(zip)
                    .set_delay(std::time::Duration::from_millis(a.delay_ms)),
            })
            .mount(&self.server)
            .await;
        let url = format!("{}/{name}", self.server.uri());
        let mut m = serde_json::json!({
            "version": a.version,
            "url": url,
            "hash": sha,
            "extract_dir": "pkg",
            "bin": format!("{}.exe", a.id),
            "depends": a.depends,
        });
        if a.env {
            m["env_set"] = serde_json::json!({ format!("{}_HOME", a.id.to_uppercase()): "$dir" });
        }
        let raw = m.to_string();
        self.put_raw(a.id, a.version, raw.clone());
        raw
    }

    fn ctx(&self) -> OpsCtx<'_> {
        self.ctx_with(
            LockWait::Block(std::time::Duration::from_millis(200)),
            &nothing_in_use,
        )
    }

    fn ctx_with<'a>(
        &'a self,
        wait: LockWait,
        in_use: &'a (dyn Fn(&Path) -> Vec<PathBuf> + Send + Sync),
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
            progress: &no_progress,
            in_use,
            residue: ResidueScan::none(),
            msiexec: wingetrs::install::executor::Msiexec::DISABLED,
            launch: Default::default(),
        }
    }

    fn candidate(&self, id: &str) -> wingetrs::install::target::Candidate {
        match resolve_target(&self.ctx(), id, None).unwrap() {
            Resolution::One(c) => c,
            other => panic!("{id}: {other:?}"),
        }
    }

    fn plans(&self, id: &str) -> Result<Vec<InstallPlan>, OpsError> {
        plan_for(&self.ctx(), &self.candidate(id), false)
    }

    async fn install(&self, id: &str) {
        let plans = self.plans(id).unwrap();
        apply(&self.ctx(), &plans).await.unwrap();
    }

    fn state(&self) -> StateDb {
        StateDb::open(&self.layout.state_db()).unwrap()
    }

    fn broadcasts(&self) -> u32 {
        self.env.dump().1
    }
}

fn names(plans: &[InstallPlan]) -> Vec<&str> {
    plans.iter().map(|p| p.app.as_str()).collect()
}

#[tokio::test]
async fn dependencies_come_first_in_topological_order_without_duplicates() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.add(app("mid", &["main/leaf"])).await;
    w.add(app("top", &["mid", "leaf"])).await;
    let plans = w.plans("top").unwrap();
    assert_eq!(names(&plans), ["leaf", "mid", "top"]);
    assert!(plans.iter().all(|p| p.bucket == "main"));
}

#[tokio::test]
async fn dependency_installed_by_wingetrs_is_skipped() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.add(app("top", &["leaf"])).await;
    w.install("leaf").await;
    assert_eq!(names(&w.plans("top").unwrap()), ["top"]);
}

#[tokio::test]
async fn dependency_cycle_is_reported_with_the_chain() {
    let mut w = World::new().await;
    w.add(app("a", &["b"])).await;
    w.add(app("b", &["a"])).await;
    match w.plans("a") {
        Err(OpsError::DependencyCycle(chain)) => assert_eq!(chain, ["a", "b", "a"]),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn unsupported_dependency_fails_before_any_download() {
    let mut w = World::new().await;
    w.put_raw("cdrtools", "1", fx("unsup-script__main__7zip.json"));
    let zip = zip_of(&[("pkg/top.exe", &fake_pe(3))]);
    let sha = sha256_hex(&zip);
    Mock::given(method("GET"))
        .and(path("/top-1.0.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
        .expect(0)
        .mount(&w.server)
        .await;
    let raw = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/top-1.0.zip", w.server.uri()),
        "hash": sha,
        "extract_dir": "pkg",
        "bin": "top.exe",
        "depends": "cdrtools",
    })
    .to_string();
    w.put_raw("top", "1.0", raw);
    // The CLI flow: plan, then apply only what was planned.
    let ctx = w.ctx();
    let result = match plan_for(&ctx, &w.candidate("top"), false) {
        Ok(plans) => apply(&ctx, &plans).await.map(|()| plans.len()),
        Err(e) => Err(e),
    };
    match result {
        Err(OpsError::DependencyUnsupported { app, .. }) => assert_eq!(app, "cdrtools"),
        other => panic!("{other:?}"),
    }
    w.server.verify().await;
    assert!(!w.layout.app_dir("top").exists());
}

#[tokio::test]
async fn missing_dependency_is_unsupported() {
    let mut w = World::new().await;
    w.add(app("top", &["ghost"])).await;
    match w.plans("top") {
        Err(OpsError::DependencyUnsupported { app, reason }) => {
            assert_eq!(app, "ghost");
            assert!(reason.contains("não encontrad"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn winget_candidate_is_unsupported() {
    let mut w = World::new().await;
    w.catalog
        .replace_source(
            "winget",
            SourceKind::Winget,
            "v",
            1,
            &[NewPackage {
                id: "Foo.Bar".into(),
                name: "Foo".into(),
                version: "1".into(),
                ..Default::default()
            }],
            &WingetMaps::default(),
        )
        .unwrap();
    let c = w.candidate("Foo.Bar");
    assert!(matches!(
        plan_for(&w.ctx(), &c, false),
        Err(OpsError::Unsupported(_))
    ));
}

#[tokio::test]
async fn resolve_target_maps_one_many_and_not_found() {
    let mut w = World::new().await;
    w.add(app("ripgrep", &[])).await;
    w.catalog
        .replace_source(
            "scoop:extras",
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
    assert!(matches!(
        resolve_target(&w.ctx(), "ripgrep", None).unwrap(),
        Resolution::Many(v) if v.len() == 2
    ));
    assert!(matches!(
        resolve_target(&w.ctx(), "ripgrep", Some("extras")).unwrap(),
        Resolution::One(c) if c.entry.source_name == "scoop:extras"
    ));
    assert!(matches!(
        resolve_target(&w.ctx(), "ripgrop", None).unwrap(),
        Resolution::NotFound { suggestions } if !suggestions.is_empty()
    ));
}

#[tokio::test]
async fn busy_lock_with_nowait_is_busy() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    let plans = w.plans("leaf").unwrap();
    let held = UpdateLock::try_acquire(&w.layout.install_lock())
        .unwrap()
        .unwrap();
    let ctx = w.ctx_with(LockWait::NoWait, &nothing_in_use);
    // The lock stays held during the call: a waiting apply could only end by the hang detector.
    let r = tokio::time::timeout(std::time::Duration::from_secs(60), apply(&ctx, &plans))
        .await
        .expect("NoWait never waits");
    assert!(matches!(r, Err(OpsError::Busy)));
    assert!(!w.layout.app_dir("leaf").exists(), "nothing ran");
    drop(held);
    apply(&ctx, &plans).await.unwrap();
    assert!(w.layout.current("leaf").exists());
}

#[tokio::test]
async fn busy_lock_blocks_uninstall_and_repair_too() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.install("leaf").await;
    let plan = plan_uninstall(&w.ctx(), "leaf", false).unwrap();
    let _held = UpdateLock::try_acquire(&w.layout.install_lock())
        .unwrap()
        .unwrap();
    let ctx = w.ctx_with(LockWait::NoWait, &nothing_in_use);
    assert!(matches!(uninstall(&ctx, &plan), Err(OpsError::Busy)));
    assert!(matches!(repair(&ctx), Err(OpsError::Busy)));
    assert!(w.layout.current("leaf").exists());
}

#[tokio::test]
async fn uninstall_refuses_while_another_app_depends_on_it() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.add(app("mid", &["leaf"])).await;
    w.install("mid").await;
    match plan_uninstall(&w.ctx(), "leaf", false) {
        Err(OpsError::HasDependents(who)) => assert_eq!(who, ["mid"]),
        other => panic!("{other:?}"),
    }
    // The dependent itself can go, and then the dependency can too.
    let p = plan_uninstall(&w.ctx(), "mid", false).unwrap();
    uninstall(&w.ctx(), &p).unwrap();
    let p = plan_uninstall(&w.ctx(), "leaf", false).unwrap();
    uninstall(&w.ctx(), &p).unwrap();
    assert!(w.state().apps().unwrap().is_empty());
}

#[tokio::test]
async fn uninstall_of_unknown_app_is_not_installed() {
    let w = World::new().await;
    assert!(matches!(
        plan_uninstall(&w.ctx(), "nope", false),
        Err(OpsError::NotInstalled(_))
    ));
}

#[tokio::test]
async fn uninstall_with_file_in_use_changes_nothing() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.install("leaf").await;
    let plan = plan_uninstall(&w.ctx(), "leaf", false).unwrap();
    let ctx = w.ctx_with(LockWait::NoWait, &exe_in_use);
    match uninstall(&ctx, &plan) {
        Err(OpsError::InUse(p)) => assert!(p.ends_with("busy.exe"), "{p}"),
        other => panic!("{other:?}"),
    }
    assert!(w.layout.current("leaf").exists());
    assert!(w.state().get("leaf").unwrap().is_some());
    assert!(w.layout.shims().join("leaf.shim").exists());
}

#[tokio::test]
async fn uninstall_returns_classified_residues() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.install("leaf").await;
    let dir = w.layout.app_dir("leaf");
    let run = r"Software\Microsoft\Windows\CurrentVersion\Run";
    w.registry.insert(
        run,
        Some("leaf"),
        &format!(
            "\"{}\" --tray",
            dir.join("current").join("leaf.exe").display()
        ),
    );
    w.registry.insert(r"Software\leaf", Some("x"), "1");
    let plan = plan_uninstall(&w.ctx(), "leaf", false).unwrap();
    let residues = uninstall(&w.ctx(), &plan).unwrap();
    assert!(!dir.exists());
    assert!(w.state().get("leaf").unwrap().is_none());
    let high: Vec<_> = residues
        .iter()
        .filter(|r| r.confidence == Confidence::High)
        .collect();
    assert_eq!(high.len(), 1, "{residues:?}");
    assert_eq!(
        high[0].kind,
        ResidueKind::RegValue {
            key: run.into(),
            value_name: Some("leaf".into())
        }
    );
    assert!(residues.iter().any(|r| r.confidence == Confidence::Low
        && r.kind
            == ResidueKind::RegKey {
                key: r"Software\leaf".into()
            }));
    // Classified only: nothing was cleaned.
    assert_eq!(w.registry.dump().0.len(), 2);
}

#[tokio::test]
async fn residue_scan_finds_shortcuts_and_folders_from_the_injected_dirs() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.install("leaf").await;
    let extra = w._t.path().join("profile");
    let desktop = extra.join("Desktop");
    let appdata = extra.join("AppData");
    std::fs::create_dir_all(&desktop).unwrap();
    std::fs::create_dir_all(appdata.join("leaf")).unwrap();
    std::fs::create_dir_all(appdata.join("other")).unwrap();
    std::fs::write(desktop.join("Leaf.lnk"), b"x").unwrap();
    std::fs::write(desktop.join("Other.lnk"), b"x").unwrap();
    let app_dir = w.layout.app_dir("leaf");
    let target = move |lnk: &Path| -> Option<PathBuf> {
        (lnk.file_name()? == "Leaf.lnk").then(|| app_dir.join("current").join("leaf.exe"))
    };
    let elsewhere = |_: &str| true;
    let mut ctx = w.ctx();
    ctx.residue = ResidueScan {
        shortcut_dirs: vec![desktop.clone()],
        folder_roots: vec![appdata.clone()],
        link_target: &target,
        other_installs: &elsewhere,
    };
    let plan = plan_uninstall(&ctx, "leaf", false).unwrap();
    let residues = uninstall(&ctx, &plan).unwrap();
    let shortcut = residues
        .iter()
        .find(|r| matches!(&r.kind, ResidueKind::Shortcut { .. }))
        .unwrap();
    assert_eq!(
        shortcut.kind,
        ResidueKind::Shortcut {
            path: desktop.join("Leaf.lnk")
        }
    );
    assert!(shortcut.preselected);
    let folders: Vec<_> = residues
        .iter()
        .filter(|r| matches!(&r.kind, ResidueKind::Folder { .. }))
        .collect();
    assert_eq!(folders.len(), 1, "{residues:?}");
    assert_eq!(
        folders[0].kind,
        ResidueKind::Folder {
            path: appdata.join("leaf")
        }
    );
    assert!(!folders[0].preselected);
    assert!(folders[0].note.is_some(), "other install flagged");
}

#[tokio::test]
async fn manifest_sha256_is_recorded() {
    let mut w = World::new().await;
    let raw = w.add(app("leaf", &[])).await;
    w.install("leaf").await;
    let (rec, _) = w.state().get("leaf").unwrap().unwrap();
    assert_eq!(rec.manifest_sha256, sha256_hex(raw.as_bytes()));
    assert_eq!(rec.bucket, "main");
}

#[tokio::test]
async fn env_broadcast_happens_once_for_two_plans_with_env() {
    let mut w = World::new().await;
    w.add(App {
        env: true,
        ..app("leaf", &[])
    })
    .await;
    w.add(App {
        env: true,
        ..app("top", &["leaf"])
    })
    .await;
    let plans = w.plans("top").unwrap();
    assert_eq!(plans.len(), 2);
    apply(&w.ctx(), &plans).await.unwrap();
    assert_eq!(w.broadcasts(), 1);
    assert!(w.env.get_raw("TOP_HOME").is_some());
    let p = plan_uninstall(&w.ctx(), "top", false).unwrap();
    uninstall(&w.ctx(), &p).unwrap();
    assert_eq!(
        w.broadcasts(),
        2,
        "uninstall that removed a variable broadcasts once"
    );
}

#[tokio::test]
async fn no_broadcast_when_nothing_in_the_environment_changes() {
    let mut w = World::new().await;
    let shims = w.layout.shims().display().to_string();
    w.env
        .set(
            "Path",
            &EnvValue {
                value: format!("{ORIGINAL_PATH};{shims}"),
                expand: true,
            },
        )
        .unwrap();
    w.add(app("leaf", &[])).await;
    w.add(app("top", &["leaf"])).await;
    let plans = w.plans("top").unwrap();
    apply(&w.ctx(), &plans).await.unwrap();
    assert_eq!(w.broadcasts(), 0);
}

#[tokio::test]
async fn upgradable_lists_apps_with_a_newer_catalog_version() {
    let mut w = World::new().await;
    // 1.9 -> 1.10 is an upgrade by version order but not by text order.
    w.add(App {
        version: "1.9",
        ..app("leaf", &[])
    })
    .await;
    w.add(app("other", &[])).await;
    w.install("leaf").await;
    w.install("other").await;
    assert!(upgradable(&w.ctx()).unwrap().upgradable.is_empty());
    w.add(App {
        version: "1.10",
        ..app("leaf", &[])
    })
    .await;
    assert_eq!(
        upgradable(&w.ctx()).unwrap().upgradable,
        [("leaf".to_string(), "1.9".to_string(), "1.10".to_string())]
    );
    let plans = w.plans("leaf").unwrap();
    assert_eq!(plans[0].upgrade_from.as_deref(), Some("1.9"));
    apply(&w.ctx(), &plans).await.unwrap();
    assert!(upgradable(&w.ctx()).unwrap().upgradable.is_empty());
    assert!(!w.layout.version_dir("leaf", "1.9").exists());
}

fn remove_state_db(layout: &Layout) {
    for suffix in ["", "-wal", "-shm"] {
        let p = PathBuf::from(format!("{}{suffix}", layout.state_db().display()));
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("{}: {e}", p.display()),
        }
    }
}

type Summary = Vec<(String, String, String, Vec<String>)>;

fn summary(state: &StateDb) -> Summary {
    state
        .apps()
        .unwrap()
        .into_iter()
        .map(|r| {
            let (_, arts) = state.get(&r.app).unwrap().unwrap();
            let mut shims: Vec<String> = arts
                .iter()
                .filter(|a| a.kind == ArtifactKind::Shim)
                .map(|a| a.target.clone())
                .collect();
            shims.sort();
            (r.app, r.bucket, r.version, shims)
        })
        .collect()
}

#[tokio::test]
async fn repair_rebuilds_a_deleted_state_db() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.add(app("top", &["leaf"])).await;
    w.install("top").await;
    let before = summary(&w.state());
    assert_eq!(before.len(), 2);
    remove_state_db(&w.layout);
    let report = repair(&w.ctx()).unwrap();
    assert_eq!(report.rebuilt, ["leaf", "top"]);
    let state = w.state();
    assert_eq!(summary(&state), before);
    // The shims folder is on the PATH, so each rebuilt record carries it.
    let shims = w.layout.shims().display().to_string();
    let (_, arts) = state.get("leaf").unwrap().unwrap();
    assert!(
        arts.iter()
            .any(|a| a.kind == ArtifactKind::Path && a.target == shims),
        "{arts:?}"
    );
    drop(state);
    // The rebuilt record is enough to uninstall cleanly.
    let p = plan_uninstall(&w.ctx(), "top", false).unwrap();
    uninstall(&w.ctx(), &p).unwrap();
    assert!(!w.layout.shims().join("top.shim").exists());
}

#[tokio::test]
async fn repair_removes_app_folders_holding_only_leftovers_of_a_killed_install() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.install("leaf").await;
    let apps = w.layout.apps();
    // Killed during `msiexec /a` (Task 140): staging, work folder, an extraction temp.
    let dead = apps.join("dead");
    std::fs::create_dir_all(dead.join(".staging-2.0.msiexec").join("SourceDir")).unwrap();
    std::fs::write(
        dead.join(".staging-2.0.msiexec")
            .join("SourceDir")
            .join("x"),
        b"x",
    )
    .unwrap();
    std::fs::create_dir_all(dead.join(".staging-2.0")).unwrap();
    std::fs::create_dir_all(dead.join(".wingetrs-extract-77-0")).unwrap();
    // Leftovers beside anything else: not ours to judge, left as it is.
    let mixed = apps.join("mixed");
    std::fs::create_dir_all(mixed.join(".staging-1.0.msiexec")).unwrap();
    std::fs::write(mixed.join("notes.txt"), b"user").unwrap();
    // A recorded app keeps its leftovers until its next install sweeps them.
    let kept_leftover = w.layout.app_dir("leaf").join(".staging-9.msiexec");
    std::fs::create_dir_all(&kept_leftover).unwrap();
    let report = repair(&w.ctx()).unwrap();
    assert_eq!(report.cleaned, ["dead"]);
    assert_eq!(report.skipped, ["mixed"]);
    assert_eq!(report.kept, ["leaf"]);
    assert!(!dead.exists());
    assert!(mixed.join("notes.txt").exists());
    assert!(kept_leftover.exists());
}

#[tokio::test]
async fn repair_keeps_intact_records_and_drops_orphans() {
    let mut w = World::new().await;
    w.add(App {
        env: true,
        ..app("leaf", &[])
    })
    .await;
    w.install("leaf").await;
    let before = w.state().get("leaf").unwrap().unwrap();
    let mut state = w.state();
    let mut ghost = before.0.clone();
    ghost.app = "ghost".into();
    state.record(&ghost, &[]).unwrap();
    drop(state);
    let report = repair(&w.ctx()).unwrap();
    assert_eq!(report.kept, ["leaf"]);
    assert_eq!(report.removed, ["ghost"]);
    let state = w.state();
    assert_eq!(state.get("leaf").unwrap().unwrap(), before);
    assert!(state.get("ghost").unwrap().is_none());
}

#[tokio::test]
async fn list_shows_wingetrs_origin_and_broken_apps() {
    use wingetrs::installed::correlate::{Correlation, UpdateState, correlate};
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.add(app("other", &[])).await;
    w.install("leaf").await;
    w.install("other").await;
    wingetrs::install::link::remove(&w.layout.current("other")).unwrap();
    let apps = wingetrs::install::state::scan_wingetrs(&w.layout);
    let rows = correlate(
        &apps,
        &w.catalog,
        &wingetrs::installed::normalize::NameNormalizer::new(),
        &wingetrs::installed::winget_tracking::Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    let leaf = rows.iter().find(|r| r.name == "leaf").unwrap();
    assert_eq!(leaf.origin, "wingetrs");
    assert_eq!(leaf.state, UpdateState::UpToDate);
    assert!(matches!(
        &leaf.correlation,
        Correlation::Matched { source, package_id, .. } if source == "scoop:main" && package_id == "leaf"
    ));
    let other = rows.iter().find(|r| r.name == "other").unwrap();
    assert_eq!(other.state, UpdateState::Broken);
    let text = wingetrs::cli::output::list_text(&rows, false);
    let leaf_line = text.lines().find(|l| l.starts_with("leaf ")).unwrap();
    assert!(leaf_line.contains("scoop:main"), "{text}");
    assert!(leaf_line.trim_end().ends_with("wingetrs"), "{text}");
    assert!(text.contains("other (quebrado"), "{text}");
}

#[tokio::test]
async fn upgradable_skips_broken_and_uncatalogued_apps_with_a_reason() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.add(app("gone", &[])).await;
    w.add(app("broken", &[])).await;
    for a in ["leaf", "gone", "broken"] {
        w.install(a).await;
    }
    w.add(App {
        version: "2.0",
        ..app("broken", &[])
    })
    .await;
    w.packages.retain(|p| p.id != "gone");
    w.put_raw("leaf", "2.0", w.packages[0].raw.clone().unwrap());
    wingetrs::install::link::remove(&w.layout.current("broken")).unwrap();
    let list = upgradable(&w.ctx()).unwrap();
    assert_eq!(
        list.upgradable,
        [("leaf".to_string(), "1.0".to_string(), "2.0".to_string())]
    );
    let skipped: Vec<&str> = list.skipped.iter().map(|(a, _)| a.as_str()).collect();
    assert_eq!(skipped, ["broken", "gone"]);
    assert!(list.skipped[0].1.contains("quebrada"), "{list:?}");
    assert!(list.skipped[1].1.contains("catálogo"), "{list:?}");
}

#[tokio::test]
async fn dependency_with_explicit_bucket_comes_from_that_bucket() {
    let mut w = World::new().await;
    w.add(app("x", &[])).await;
    w.add(app("top", &["extras/x"])).await;
    let raw = w.packages.iter().find(|p| p.id == "x").unwrap().raw.clone();
    w.catalog
        .replace_source(
            "scoop:extras",
            SourceKind::Scoop,
            "v",
            1,
            &[NewPackage {
                id: "x".into(),
                name: "x".into(),
                version: "1.0".into(),
                raw,
                ..Default::default()
            }],
            &WingetMaps::default(),
        )
        .unwrap();
    let c = match resolve_target(&w.ctx(), "top", Some("main")).unwrap() {
        Resolution::One(c) => c,
        other => panic!("{other:?}"),
    };
    let plans = plan_for(&w.ctx(), &c, false).unwrap();
    assert_eq!(names(&plans), ["x", "top"]);
    assert_eq!(plans[0].bucket, "extras");
    assert_eq!(plans[1].bucket, "main");
}

fn disk(w: &World) -> std::collections::BTreeMap<String, String> {
    let mut s = wingetrs::install::snapshot::snapshot(w.layout.root()).unwrap();
    s.retain(|p, _| !p.starts_with("blobs") && !p.starts_with("tmp") && !p.starts_with("state.db"));
    s
}

#[tokio::test]
async fn apply_refuses_a_plan_made_stale_by_an_outside_install() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    let stale = w.plans("leaf").unwrap();
    w.install("leaf").await;
    let before = (disk(&w), w.env.dump(), w.state().get("leaf").unwrap());
    match apply(&w.ctx(), &stale).await {
        Err(OpsError::StateChanged(a)) => assert_eq!(a, "leaf"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        (disk(&w), w.env.dump(), w.state().get("leaf").unwrap()),
        before
    );
}

#[tokio::test]
async fn apply_skips_a_dependency_installed_meanwhile() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.add(app("top", &["leaf"])).await;
    let plans = w.plans("top").unwrap();
    assert_eq!(names(&plans), ["leaf", "top"]);
    w.install("leaf").await;
    apply(&w.ctx(), &plans).await.unwrap();
    assert!(w.state().get("top").unwrap().is_some());
}

#[tokio::test]
async fn uninstall_refuses_a_plan_made_stale_by_an_outside_change() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.install("leaf").await;
    let stale = plan_uninstall(&w.ctx(), "leaf", false).unwrap();
    w.add(App {
        version: "2.0",
        ..app("leaf", &[])
    })
    .await;
    w.install("leaf").await;
    let before = (disk(&w), w.env.dump(), w.state().get("leaf").unwrap());
    match uninstall(&w.ctx(), &stale) {
        Err(OpsError::StateChanged(a)) => assert_eq!(a, "leaf"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        (disk(&w), w.env.dump(), w.state().get("leaf").unwrap()),
        before
    );
    // Uninstalled by someone else meanwhile: the same answer.
    let fresh = plan_uninstall(&w.ctx(), "leaf", false).unwrap();
    uninstall(&w.ctx(), &fresh).unwrap();
    assert!(matches!(
        uninstall(&w.ctx(), &fresh),
        Err(OpsError::StateChanged(_))
    ));
}

#[tokio::test]
async fn failure_after_a_dependency_names_what_was_installed() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    // `top` downloads fine but is not a zip: it fails in the executor, after `leaf`.
    let junk = b"this is not a zip".to_vec();
    Mock::given(method("GET"))
        .and(path("/top-junk.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(junk.clone()))
        .mount(&w.server)
        .await;
    let raw = w.add(app("top", &["leaf"])).await;
    let mut m: serde_json::Value = serde_json::from_str(&raw).unwrap();
    m["url"] = serde_json::json!(format!("{}/top-junk.zip", w.server.uri()));
    m["hash"] = serde_json::json!(sha256_hex(&junk));
    w.put_raw("top", "1.0", m.to_string());
    let plans = w.plans("top").unwrap();
    let err = apply(&w.ctx(), &plans).await.unwrap_err();
    assert!(err.to_string().contains("já instalados: leaf"), "{err}");
    match err {
        OpsError::PartlyApplied { applied, .. } => assert_eq!(applied, ["leaf"]),
        other => panic!("{other:?}"),
    }
    assert!(w.state().get("leaf").unwrap().is_some());
    assert!(w.state().get("top").unwrap().is_none());
}

#[tokio::test]
async fn repair_ignores_shims_that_escape_the_app_folder() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.install("leaf").await;
    let escape = w.layout.app_dir("leaf").join(r"..\..\x\evil.exe");
    std::fs::write(
        w.layout.shims().join("evil.shim"),
        format!("path = \"{}\"\r\n", escape.display()),
    )
    .unwrap();
    remove_state_db(&w.layout);
    repair(&w.ctx()).unwrap();
    let shims = summary(&w.state())[0].3.clone();
    assert_eq!(shims, ["leaf"]);
}

#[tokio::test]
async fn merge_groups_keeps_order_and_drops_repeated_apps() {
    let mut w = World::new().await;
    for id in ["leaf", "a", "b", "c"] {
        w.add(app(id, &[])).await;
    }
    let mut g1 = w.plans("leaf").unwrap();
    g1.extend(w.plans("a").unwrap());
    let mut g2 = w.plans("leaf").unwrap();
    for p in &mut g2 {
        p.app = p.app.to_uppercase();
    }
    g2.extend(w.plans("b").unwrap());
    let g3 = w.plans("c").unwrap();
    let merged = merge_groups(vec![g1, g2, g3]);
    assert_eq!(names(&merged), ["leaf", "a", "b", "c"]);
}

#[tokio::test]
async fn apply_groups_broadcasts_once_for_two_env_apps() {
    let mut w = World::new().await;
    w.add(App {
        env: true,
        ..app("a", &[])
    })
    .await;
    w.add(App {
        env: true,
        ..app("b", &[])
    })
    .await;
    let groups = vec![w.plans("a").unwrap(), w.plans("b").unwrap()];
    let done = apply_groups(&w.ctx(), groups).await.unwrap();
    assert_eq!(names(&done), ["a", "b"]);
    assert_eq!(w.broadcasts(), 1);
    assert!(w.state().get("a").unwrap().is_some());
    assert!(w.state().get("b").unwrap().is_some());
}

#[tokio::test]
async fn apply_groups_reports_what_was_applied_when_a_later_app_fails() {
    let mut w = World::new().await;
    w.add(app("a", &[])).await;
    let junk = b"this is not a zip".to_vec();
    Mock::given(method("GET"))
        .and(path("/b-1.0.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(junk.clone()))
        .mount(&w.server)
        .await;
    let raw = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/b-1.0.zip", w.server.uri()),
        "hash": sha256_hex(&junk),
        "bin": "b.exe",
    })
    .to_string();
    w.put_raw("b", "1.0", raw);
    let groups = vec![w.plans("a").unwrap(), w.plans("b").unwrap()];
    match apply_groups(&w.ctx(), groups).await {
        Err(OpsError::PartlyApplied { applied, .. }) => assert_eq!(applied, ["a"]),
        other => panic!("{other:?}"),
    }
    assert!(w.state().get("a").unwrap().is_some());
    assert!(w.state().get("b").unwrap().is_none());
    assert!(!w.layout.root().join("apps").join("b").exists());
}

#[tokio::test]
async fn apply_groups_refuses_a_target_installed_meanwhile_even_in_the_middle() {
    let mut w = World::new().await;
    w.add(app("a", &[])).await;
    w.add(app("b", &[])).await;
    let groups = vec![w.plans("a").unwrap(), w.plans("b").unwrap()];
    w.install("a").await;
    match apply_groups(&w.ctx(), groups).await {
        Err(OpsError::StateChanged(a)) => assert_eq!(a, "a"),
        other => panic!("{other:?}"),
    }
    assert!(w.state().get("b").unwrap().is_none());
}

#[tokio::test]
async fn apply_groups_targets_are_the_last_plan_of_each_group() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.add(app("a", &["leaf"])).await;
    w.add(app("b", &[])).await;
    let groups = vec![w.plans("a").unwrap(), w.plans("b").unwrap()];
    assert_eq!(names(&groups[0]), ["leaf", "a"]);
    w.install("a").await;
    match apply_groups(&w.ctx(), groups).await {
        Err(OpsError::StateChanged(a)) => assert_eq!(a, "a"),
        other => panic!("{other:?}"),
    }
    assert!(w.state().get("b").unwrap().is_none());
}

#[tokio::test]
async fn apply_groups_skips_a_dependency_installed_meanwhile_but_not_targets() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.add(app("a", &["leaf"])).await;
    w.add(app("b", &[])).await;
    let groups = vec![w.plans("a").unwrap(), w.plans("b").unwrap()];
    w.install("leaf").await;
    apply_groups(&w.ctx(), groups).await.unwrap();
    assert!(w.state().get("a").unwrap().is_some());
    assert!(w.state().get("b").unwrap().is_some());
}

#[tokio::test]
async fn apply_downloads_the_batch_in_parallel() {
    let mut w = World::new().await;
    for id in ["a", "b", "c"] {
        w.add(App {
            delay_ms: 2000,
            ..app(id, &[])
        })
        .await;
    }
    let mut plans = w.plans("a").unwrap();
    plans.extend(w.plans("b").unwrap());
    plans.extend(w.plans("c").unwrap());
    apply(&w.ctx(), &plans).await.unwrap();
    // Each response is delayed 2000 ms: sequential downloads would put the last request at least
    // one full delay after the first (structural), parallel ones a few ms. The bound is the
    // delay itself, >=10x the expected gap; it is a ratio, not an absolute machine-speed budget.
    let arrivals = w.arrivals.lock().unwrap().clone();
    assert_eq!(arrivals.len(), 3);
    let (first, last) = (
        arrivals.iter().min().unwrap(),
        arrivals.iter().max().unwrap(),
    );
    assert!(
        last.duration_since(*first) < std::time::Duration::from_millis(2000),
        "downloads did not overlap: {:?}",
        last.duration_since(*first)
    );
    for id in ["a", "b", "c"] {
        assert!(w.state().get(id).unwrap().is_some(), "{id}");
    }
}

#[tokio::test]
async fn apply_fails_before_any_change_when_a_download_fails() {
    let mut w = World::new().await;
    w.add(app("a", &[])).await;
    let raw = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/b-1.0.zip", w.server.uri()),
        "hash": "0".repeat(64),
        "bin": "b.exe",
    })
    .to_string();
    w.put_raw("b", "1.0", raw);
    let mut plans = w.plans("a").unwrap();
    plans.extend(w.plans("b").unwrap());
    match apply(&w.ctx(), &plans).await {
        Err(OpsError::Download { app, .. }) => assert_eq!(app, "b"),
        other => panic!("{other:?}"),
    }
    assert!(w.state().apps().unwrap().is_empty());
    assert!(!w.layout.root().join("apps").join("a").exists());
    assert_eq!(w.broadcasts(), 0);
}

#[tokio::test]
async fn prefetch_downloads_a_shared_source_once() {
    let mut w = World::new().await;
    let zip = zip_of(&[("pkg/a.exe", &fake_pe(3)), ("pkg/b.exe", &fake_pe(3))]);
    let sha = sha256_hex(&zip);
    Mock::given(method("GET"))
        .and(path("/shared.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
        .expect(1)
        .mount(&w.server)
        .await;
    let url = format!("{}/shared.zip", w.server.uri());
    for id in ["a", "b"] {
        let raw = serde_json::json!({
            "version": "1.0",
            "url": url,
            "hash": sha,
            "extract_dir": "pkg",
            "bin": format!("{id}.exe"),
        })
        .to_string();
        w.put_raw(id, "1.0", raw);
    }
    let mut plans = w.plans("a").unwrap();
    plans.extend(w.plans("b").unwrap());
    apply(&w.ctx(), &plans).await.unwrap();
    assert!(w.state().get("a").unwrap().is_some());
    assert!(w.state().get("b").unwrap().is_some());
    w.server.verify().await;
}

#[tokio::test]
async fn prefetch_skips_dependencies_installed_meanwhile() {
    let mut w = World::new().await;
    w.add(app("a", &["leaf"])).await;
    // `leaf`'s zip may be fetched once: by the outside install, never by the batch.
    let zip = zip_of(&[("pkg/leaf.exe", &fake_pe(3))]);
    let sha = sha256_hex(&zip);
    Mock::given(method("GET"))
        .and(path("/leaf-once.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
        .expect(1)
        .mount(&w.server)
        .await;
    let raw = serde_json::json!({
        "version": "1.0",
        "url": format!("{}/leaf-once.zip", w.server.uri()),
        "hash": sha,
        "extract_dir": "pkg",
        "bin": "leaf.exe",
    })
    .to_string();
    w.put_raw("leaf", "1.0", raw);
    let plans = w.plans("a").unwrap();
    assert_eq!(names(&plans), ["leaf", "a"]);
    w.install("leaf").await;
    // The outside install left the blob cached; drop it so a second fetch would hit the mock.
    std::fs::remove_dir_all(w.data.blobs_dir()).unwrap();
    apply(&w.ctx(), &plans).await.unwrap();
    assert!(w.state().get("a").unwrap().is_some());
    w.server.verify().await;
}

#[tokio::test]
async fn install_emits_one_span_per_stage_and_per_app() {
    let mut w = World::new().await;
    w.add(App {
        env: true,
        ..app("leaf", &[])
    })
    .await;
    w.add(app("top", &["leaf"])).await;
    let plans = w.plans("top").unwrap();
    assert_eq!(plans.len(), 2);
    let closed = spans::Closed::default();
    let _g = closed.install();
    apply(&w.ctx(), &plans).await.unwrap();
    for once in [
        "install.lock",
        "install.revalidate",
        "install.prefetch",
        "install.broadcast",
    ] {
        assert_eq!(closed.count(once), 1, "{once}: {:?}", closed.names());
    }
    assert_eq!(closed.count("install.execute"), 2, "{:?}", closed.names());
    for per_app in [
        "install.step:download",
        "install.step:extract",
        "install.step:shim",
        "install.step:state",
    ] {
        assert_eq!(closed.count(per_app), 2, "{per_app}: {:?}", closed.names());
    }
    assert_eq!(closed.count("install.step:env"), 1, "só leaf tem env_set");
    let step_parents = closed.parents("install.step");
    assert_eq!(step_parents.len(), 15, "{:?}", closed.names());
    assert!(
        step_parents.iter().all(|p| p == "install.execute"),
        "{step_parents:?}"
    );
    let filter = wingetrs::cli::file_log_filter();
    for (name, _, target, level) in closed.0.lock().unwrap().iter() {
        if name.starts_with("install.") {
            assert!(
                filter.would_enable(target, level),
                "{name} ({target}, {level}) não chega ao log"
            );
        }
    }
}

#[tokio::test]
async fn uninstall_emits_its_stage_spans() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.install("leaf").await;
    let p = plan_uninstall(&w.ctx(), "leaf", false).unwrap();
    let closed = spans::Closed::default();
    let _g = closed.install();
    uninstall(&w.ctx(), &p).unwrap();
    for once in [
        "uninstall.lock",
        "uninstall.execute",
        "uninstall.broadcast",
        "uninstall.residue_scan",
    ] {
        assert_eq!(closed.count(once), 1, "{once}: {:?}", closed.names());
    }
}

#[tokio::test]
async fn planning_spans_wrap_resolve_and_plan() {
    let mut w = World::new().await;
    w.add(app("leaf", &[])).await;
    w.add(app("top", &["leaf"])).await;
    let closed = spans::Closed::default();
    let _g = closed.install();
    w.plans("top").unwrap();
    upgradable(&w.ctx()).unwrap();
    for once in ["target.resolve", "plan.build", "upgrade.list"] {
        assert_eq!(closed.count(once), 1, "{once}: {:?}", closed.names());
    }
}

/// Catalog with `vscode` in two buckets, a mixed-case id and a package whose name is another id.
async fn world_with_extras() -> World {
    let mut w = World::new().await;
    w.add(app("ripgrep", &[])).await;
    w.add(app("Git", &[])).await;
    w.add(app("vscode", &[])).await;
    w.catalog
        .replace_source(
            "scoop:extras",
            SourceKind::Scoop,
            "v",
            1,
            &[
                NewPackage {
                    id: "vscode".into(),
                    name: "vscode".into(),
                    version: "2".into(),
                    ..Default::default()
                },
                NewPackage {
                    id: "zed".into(),
                    name: "ripgrep".into(),
                    version: "1".into(),
                    ..Default::default()
                },
            ],
            &WingetMaps::default(),
        )
        .unwrap();
    w
}

#[tokio::test]
async fn resolve_installed_matches_resolve_target_for_every_installed_shape() {
    let w = world_with_extras().await;
    let ctx = w.ctx();
    for (bucket, id) in [
        ("main", "ripgrep"),
        ("main", "Git"),
        ("main", "vscode"),
        ("extras", "vscode"),
        ("extras", "zed"),
    ] {
        match (
            resolve_target(&ctx, id, Some(bucket)).unwrap(),
            resolve_installed(&ctx, id, bucket).unwrap(),
        ) {
            (Resolution::One(old), Resolution::One(new)) => assert_eq!(old, new, "{bucket}/{id}"),
            (old, new) => panic!("{bucket}/{id}: antigo {old:?}, novo {new:?}"),
        }
    }
    for (bucket, id) in [("main", "zed"), ("main", "nada")] {
        assert!(
            matches!(
                resolve_target(&ctx, id, Some(bucket)).unwrap(),
                Resolution::NotFound { .. }
            ),
            "{bucket}/{id}"
        );
        assert!(
            matches!(
                resolve_installed(&ctx, id, bucket).unwrap(),
                Resolution::NotFound { .. }
            ),
            "{bucket}/{id}"
        );
    }
}

#[tokio::test]
async fn resolve_installed_never_matches_by_name() {
    let w = world_with_extras().await;
    let ctx = w.ctx();
    match resolve_target(&ctx, "ripgrep", Some("extras")).unwrap() {
        Resolution::One(c) => assert_eq!(c.entry.id, "zed"),
        other => panic!("esperado One(zed), veio {other:?}"),
    }
    assert!(matches!(
        resolve_installed(&ctx, "ripgrep", "extras").unwrap(),
        Resolution::NotFound { .. }
    ));
}
