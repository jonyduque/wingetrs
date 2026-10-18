#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 148).
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use wingetrs::cache::DataDir;
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::core::SourceKind;
use wingetrs::http::Http;
use wingetrs::install::layout::Layout;
use wingetrs::install::ops::{
    OpsCtx, OpsError, ResidueScan, Resolution, plan_for, plan_uninstall, resolve_target,
};
use wingetrs::install::plan::InstallPlan;
use wingetrs::install::state::{AppRecord, StateDb};
use wingetrs::install::traits::Effects;
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::update::LockWait;

fn no_progress(_: &wingetrs::install::progress::ProgressEvent) {}
fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

struct World {
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
    fn new() -> World {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        World {
            data: DataDir::new(&root),
            layout: Layout::new(&root),
            env: FakeEnv::new(),
            shortcuts: FakeShortcuts::new(t.path().join("startmenu")),
            trash: FakeTrash::new(),
            registry: FakeRegistry::new(),
            http: Http::new().unwrap(),
            catalog: Catalog::open_in_memory().unwrap(),
            _t: t,
        }
    }

    /// Replaces the `scoop:<bucket>` catalog source with `(id, manifest)` entries.
    fn bucket(&mut self, bucket: &str, apps: &[(&str, serde_json::Value)]) {
        let packages: Vec<NewPackage> = apps
            .iter()
            .map(|(id, m)| NewPackage {
                id: (*id).into(),
                name: (*id).into(),
                version: "1.0".into(),
                raw: Some(m.to_string()),
                ..Default::default()
            })
            .collect();
        self.catalog
            .replace_source(
                &format!("scoop:{bucket}"),
                SourceKind::Scoop,
                "v",
                1,
                &packages,
                &WingetMaps::default(),
            )
            .unwrap();
    }

    fn installed(&self, apps: &[&str]) {
        std::fs::create_dir_all(self.layout.state_db().parent().unwrap()).unwrap();
        let mut db = StateDb::open(&self.layout.state_db()).unwrap();
        for app in apps {
            db.record(
                &AppRecord {
                    app: (*app).into(),
                    bucket: "main".into(),
                    version: "1.0".into(),
                    arch: Some("64bit".into()),
                    installed_at: 1700000000,
                    manifest_sha256: "ab".repeat(32),
                },
                &[],
            )
            .unwrap();
        }
    }

    fn ctx(&self) -> OpsCtx<'_> {
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
            wait: LockWait::Block(std::time::Duration::from_millis(200)),
            progress: &no_progress,
            in_use: &nothing_in_use,
            residue: ResidueScan::none(),
            msiexec: wingetrs::install::executor::Msiexec::DISABLED,
            launch: Default::default(),
            runners: wingetrs::install::executor::Runners::disabled(),
        }
    }

    fn plans(&self, id: &str) -> Result<Vec<InstallPlan>, OpsError> {
        let c = match resolve_target(&self.ctx(), id, None).unwrap() {
            Resolution::One(c) => c,
            other => panic!("{id}: {other:?}"),
        };
        plan_for(&self.ctx(), &c, false)
    }
}

fn plain(deps: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "version": "1.0",
        "url": "http://127.0.0.1:9/a.zip",
        "hash": "ab".repeat(32),
        "depends": deps,
    })
}

fn inno(deps: &[&str]) -> serde_json::Value {
    let mut m = plain(deps);
    m["innosetup"] = serde_json::json!(true);
    m
}

fn names(plans: &[InstallPlan]) -> Vec<&str> {
    plans.iter().map(|p| p.app.as_str()).collect()
}

#[test]
fn innosetup_app_plans_innounp_first() {
    let mut w = World::new();
    w.bucket("main", &[("innounp", plain(&[])), ("x", inno(&[]))]);
    assert_eq!(names(&w.plans("x").unwrap()), ["innounp", "x"]);
}

#[test]
fn installed_helper_not_planned() {
    let mut w = World::new();
    w.bucket("main", &[("innounp", plain(&[])), ("x", inno(&[]))]);
    w.installed(&["innounp"]);
    assert_eq!(names(&w.plans("x").unwrap()), ["x"]);
}

#[test]
fn helper_of_dependency_planned_before_it() {
    // `y` needs innounp via `innosetup`; hooks are still refused by phase 2c, so the
    // brief's `post_install` calling Expand-7zipArchive cannot reach this code path.
    let mut w = World::new();
    w.bucket(
        "main",
        &[
            ("innounp", plain(&[])),
            ("y", inno(&[])),
            ("x", plain(&["y"])),
        ],
    );
    assert_eq!(names(&w.plans("x").unwrap()), ["innounp", "y", "x"]);
}

#[test]
fn helper_comes_from_main() {
    let mut w = World::new();
    w.bucket("main", &[("innounp", plain(&[]))]);
    w.bucket("extras", &[("innounp", plain(&[])), ("x", inno(&[]))]);
    let plans = w.plans("x").unwrap();
    assert_eq!(names(&plans), ["innounp", "x"]);
    assert_eq!(plans[0].bucket, "main");
    assert_eq!(plans[1].bucket, "extras");
}

#[test]
fn removing_helper_with_dependent_refused() {
    let mut w = World::new();
    w.bucket("main", &[("innounp", plain(&[])), ("x", inno(&[]))]);
    w.installed(&["innounp", "x"]);
    match plan_uninstall(&w.ctx(), "innounp", false) {
        Err(OpsError::HasDependents(who)) => assert_eq!(who, ["x"]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn removing_helper_without_dependents_ok() {
    let mut w = World::new();
    w.bucket("main", &[("innounp", plain(&[])), ("x", plain(&[]))]);
    w.installed(&["innounp", "x"]);
    assert_eq!(
        plan_uninstall(&w.ctx(), "innounp", false).unwrap().app,
        "innounp"
    );
}

#[test]
fn helper_missing_from_catalog() {
    let mut w = World::new();
    w.bucket("main", &[("x", inno(&[]))]);
    match w.plans("x") {
        Err(OpsError::DependencyUnsupported { app, .. }) => assert_eq!(app, "innounp"),
        other => panic!("{other:?}"),
    }
}
