#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Operations layer: dependencies, lock, apply, uninstall, upgrade listing, repair and list.
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
    OpsCtx, OpsError, ResidueScan, Resolution, apply, plan_for, plan_uninstall, repair,
    resolve_target, uninstall, upgradable,
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

fn no_progress(_: &str, _: u64, _: Option<u64>) {}
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
}

fn app<'a>(id: &'a str, depends: &'a [&'a str]) -> App<'a> {
    App {
        id,
        version: "1.0",
        depends,
        env: false,
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
            .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
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
    w.put_raw("cdrtools", "1", fx("unsup-7z__main__cdrtools.json"));
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
    let t = std::time::Instant::now();
    assert!(matches!(apply(&ctx, &plans).await, Err(OpsError::Busy)));
    assert!(
        t.elapsed() < std::time::Duration::from_secs(2),
        "NoWait never waits"
    );
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
    assert!(upgradable(&w.ctx()).unwrap().is_empty());
    w.add(App {
        version: "1.10",
        ..app("leaf", &[])
    })
    .await;
    assert_eq!(
        upgradable(&w.ctx()).unwrap(),
        [("leaf".to_string(), "1.9".to_string(), "1.10".to_string())]
    );
    let plans = w.plans("leaf").unwrap();
    assert_eq!(plans[0].upgrade_from.as_deref(), Some("1.9"));
    apply(&w.ctx(), &plans).await.unwrap();
    assert!(upgradable(&w.ctx()).unwrap().is_empty());
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
