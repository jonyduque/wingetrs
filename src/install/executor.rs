//! Plan executor: applies an [`InstallPlan`] step by step, journaling an undo action for
//! every change, and rolls everything back in reverse order on any failure. `current`
//! only moves at `LinkCurrent`, so the previous version keeps working until then.
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::cache::DataDir;
use crate::http::Http;
use crate::install::download::{self, Progress};
use crate::install::layout::Layout;
use crate::install::plan::{
    InstallPlan, PersistKind, PersistSpec, ShimSpec, ShortcutSpec, Step, UninstallPlan,
};
use crate::install::state::{AppRecord, Artifact, ArtifactKind, StateDb};
use crate::install::traits::{EffectError, Effects, EnvStore, EnvValue};
use crate::install::{extract, link, path_edit, safepath, shim, state};

pub trait StepHook: Send + Sync {
    fn before(&self, index: usize, step: &Step) -> Result<(), ExecError>;
}

/// Production hook: never interferes.
#[derive(Debug, Clone, Copy)]
pub struct NoHook;
impl StepHook for NoHook {
    fn before(&self, _index: usize, _step: &Step) -> Result<(), ExecError> {
        Ok(())
    }
}

/// Files under a directory that another process holds open.
pub type InUseCheck<'a> = &'a (dyn Fn(&Path) -> Vec<PathBuf> + Send + Sync);

pub struct ExecCtx<'a> {
    pub layout: &'a Layout,
    pub data: &'a DataDir,
    pub http: &'a Http,
    pub effects: Effects<'a>,
    pub state: &'a mut StateDb,
    pub hook: &'a dyn StepHook,
    pub progress: Progress<'a>,
    /// Production: `fs_win::files_in_use`; tests pass a closure.
    pub in_use: InUseCheck<'a>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("falha no passo {index} ({step}): {message}; desfeito: {undone:?}")]
    Failed {
        index: usize,
        step: String,
        message: String,
        undone: Vec<String>,
    },
    #[error("arquivo em uso: {0} — feche o programa e tente de novo")]
    InUse(String),
    #[error("{0}")]
    Hook(String),
}

/// User environment variable holding the PATH.
const PATH_VAR: &str = "Path";

/// One change to revert, in the order it was made.
enum Undo {
    /// A file, directory tree or link this run created.
    RemovePath(PathBuf),
    /// A directory this run created; removed only if empty again.
    RemoveEmptyDir(PathBuf),
    /// `current` before `LinkCurrent` (`None` = there was none).
    RestoreCurrent {
        link: PathBuf,
        previous: Option<PathBuf>,
    },
    /// Contents of `shims\<name>.*` before the shim step.
    RestoreShim {
        name: String,
        saved: Vec<(PathBuf, Vec<u8>)>,
    },
    RemoveShortcut(PathBuf),
    /// The shortcut belonged to the previous install of this app: put it back.
    RecreateShortcut(ShortcutSpec),
    RestoreEnv {
        name: String,
        previous: Option<EnvValue>,
    },
    /// The PATH entry was added by this run.
    PathRemove {
        entry: String,
        var_existed: bool,
    },
    /// The app's record before `RecordState` (`None` = it had none).
    RestoreState {
        app: String,
        previous: Option<(AppRecord, Vec<Artifact>)>,
    },
}

fn io_msg(p: &Path, e: io::Error) -> String {
    format!("{}: {e}", p.display())
}

fn not_found_ok(r: io::Result<()>) -> io::Result<()> {
    match r {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Removes a file, a link (never following it) or a whole directory tree.
fn remove_any(p: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() {
        return link::remove(p).or_else(|_| fs::remove_file(p));
    }
    if meta.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
}

/// `previous = None` removes the variable.
fn restore_env(
    env: &dyn EnvStore,
    name: &str,
    previous: Option<&EnvValue>,
) -> Result<(), EffectError> {
    match previous {
        Some(v) => env.set(name, v),
        None => env.remove(name),
    }
}

fn path_remove_entry(
    env: &dyn EnvStore,
    entry: &str,
    var_existed: bool,
) -> Result<(), EffectError> {
    let Some(cur) = env.get(PATH_VAR)? else {
        return Ok(());
    };
    let value = path_edit::path_remove(&cur.value, entry);
    if value.is_empty() && !var_existed {
        return env.remove(PATH_VAR);
    }
    env.set(
        PATH_VAR,
        &EnvValue {
            value,
            expand: cur.expand,
        },
    )
}

/// A value that becomes a single directory name (app, version) must be one safe component.
fn check_component(what: &str, v: &str) -> Result<(), String> {
    match safepath::safe_components(v) {
        Some(c) if c.len() == 1 && c[0] == v => Ok(()),
        _ => Err(format!("nome inseguro de {what}: {v}")),
    }
}

fn step_label(step: &Step) -> String {
    match step {
        Step::EnsureLauncher => "lançador".into(),
        Step::Download { index, .. } => format!("baixar [{}]", index + 1),
        Step::Extract { index } => format!("extrair [{}]", index + 1),
        Step::Promote => "promover versão".into(),
        Step::Persist(p) => format!("persistir {}", p.in_version.display()),
        Step::LinkCurrent => "apontar current".into(),
        Step::Shim(s) => format!("comando {}", s.name),
        Step::Shortcut(s) => format!("atalho {}", s.link.display()),
        Step::EnvSet { name, .. } => format!("variável {name}"),
        Step::EnvAddPath { entry } => format!("PATH {}", entry.display()),
        Step::EnsureShimsOnPath => "shims no PATH".into(),
        Step::RecordState => "registro".into(),
        Step::RemoveOldVersion { version } => format!("remover versão {version}"),
    }
}

struct Run<'p> {
    plan: &'p InstallPlan,
    previous: Option<(AppRecord, Vec<Artifact>)>,
    downloads: BTreeMap<usize, PathBuf>,
    artifacts: Vec<Artifact>,
    journal: Vec<(String, Undo)>,
    /// Label of the step being run (journal entries are tagged with it).
    label: String,
}

impl Run<'_> {
    fn push(&mut self, u: Undo) {
        self.journal.push((self.label.clone(), u));
    }

    /// Creates every missing directory down to `dir`, journaling each one.
    fn ensure_dirs(&mut self, dir: &Path) -> Result<(), String> {
        let mut missing = Vec::new();
        let mut cur = Some(dir);
        while let Some(d) = cur {
            if fs::symlink_metadata(d).is_ok() {
                break;
            }
            missing.push(d.to_path_buf());
            cur = d.parent();
        }
        for d in missing.into_iter().rev() {
            match fs::create_dir(&d) {
                Ok(()) => self.push(Undo::RemoveEmptyDir(d)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io_msg(&d, e)),
            }
        }
        Ok(())
    }

    fn previous_artifact(&self, kind: ArtifactKind, target: &str) -> Option<&Artifact> {
        self.previous
            .as_ref()?
            .1
            .iter()
            .find(|a| a.kind == kind && a.target == target)
    }
}

fn apply_undo(u: Undo, ctx: &mut ExecCtx<'_>) -> Result<(), String> {
    let env = ctx.effects.env;
    match u {
        Undo::RemovePath(p) => remove_any(&p).map_err(|e| io_msg(&p, e)),
        Undo::RemoveEmptyDir(p) => not_found_ok(fs::remove_dir(&p)).map_err(|e| io_msg(&p, e)),
        Undo::RestoreCurrent { link: l, previous } => {
            not_found_ok(remove_any(&l)).map_err(|e| io_msg(&l, e))?;
            match previous {
                Some(target) => link::create(&target, &l).map_err(|e| io_msg(&l, e)),
                None => Ok(()),
            }
        }
        Undo::RestoreShim { name, saved } => {
            shim::remove_shim(ctx.layout, &name).map_err(|e| e.to_string())?;
            for (p, bytes) in saved {
                fs::write(&p, bytes).map_err(|e| io_msg(&p, e))?;
            }
            Ok(())
        }
        Undo::RemoveShortcut(p) => ctx.effects.shortcuts.remove(&p).map_err(|e| e.to_string()),
        Undo::RecreateShortcut(s) => ctx
            .effects
            .shortcuts
            .create(&s.link, &s.target, s.args.as_deref(), s.icon.as_deref())
            .map_err(|e| e.to_string()),
        Undo::RestoreEnv { name, previous } => {
            restore_env(env, &name, previous.as_ref()).map_err(|e| e.to_string())
        }
        Undo::PathRemove { entry, var_existed } => {
            path_remove_entry(env, &entry, var_existed).map_err(|e| e.to_string())
        }
        Undo::RestoreState { app, previous } => match previous {
            Some((r, arts)) => ctx.state.record(&r, &arts),
            None => ctx.state.remove(&app),
        }
        .map_err(|e| e.to_string()),
    }
}

/// Reverts the journal in reverse order; returns the labels of the steps undone.
fn rollback(journal: Vec<(String, Undo)>, ctx: &mut ExecCtx<'_>) -> Vec<String> {
    let mut undone: Vec<String> = Vec::new();
    for (label, u) in journal.into_iter().rev() {
        let entry = match apply_undo(u, ctx) {
            Ok(()) => label,
            Err(e) => {
                tracing::warn!("falha ao desfazer {label}: {e}");
                format!("{label} (falhou: {e})")
            }
        };
        if undone.last() != Some(&entry) {
            undone.push(entry);
        }
    }
    undone
}

pub async fn execute_install(plan: &InstallPlan, ctx: &mut ExecCtx<'_>) -> Result<(), ExecError> {
    let fail_early = |message: String| ExecError::Failed {
        index: 0,
        step: "validar plano".into(),
        message,
        undone: Vec::new(),
    };
    check_component("app", &plan.app).map_err(fail_early)?;
    check_component("versão", &plan.version).map_err(fail_early)?;
    for s in &plan.steps {
        if let Step::RemoveOldVersion { version } = s {
            check_component("versão", version).map_err(fail_early)?;
        }
    }
    let previous = ctx
        .state
        .get(&plan.app)
        .map_err(|e| fail_early(e.to_string()))?;
    let mut run = Run {
        plan,
        previous,
        downloads: BTreeMap::new(),
        artifacts: Vec::new(),
        journal: Vec::new(),
        label: String::new(),
    };
    // Removing the old version runs only after every other step succeeded.
    let (main, cleanup): (Vec<_>, Vec<_>) = plan
        .steps
        .iter()
        .enumerate()
        .partition(|(_, s)| !matches!(s, Step::RemoveOldVersion { .. }));
    for (i, step) in main {
        run.label = step_label(step);
        let result = match ctx.hook.before(i, step) {
            Ok(()) => run_step(&mut run, step, ctx).await,
            Err(e) => Err(e.to_string()),
        };
        if let Err(message) = result {
            let journal = std::mem::take(&mut run.journal);
            let undone = rollback(journal, ctx);
            return Err(ExecError::Failed {
                index: i,
                step: run.label,
                message,
                undone,
            });
        }
    }
    for (i, step) in cleanup {
        let Step::RemoveOldVersion { version } = step else {
            continue;
        };
        let result = ctx
            .hook
            .before(i, step)
            .map_err(|e| e.to_string())
            .and_then(|()| remove_old_version(ctx.layout, &plan.app, &plan.version, version));
        if let Err(e) = result {
            tracing::warn!(
                "{} {} instalado, mas a versão antiga {version} ficou: {e}",
                plan.app,
                plan.version
            );
        }
    }
    Ok(())
}

fn remove_old_version(layout: &Layout, app: &str, new: &str, old: &str) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    let dir = layout.version_dir(app, old);
    not_found_ok(remove_any(&dir)).map_err(|e| io_msg(&dir, e))
}

async fn run_step(run: &mut Run<'_>, step: &Step, ctx: &mut ExecCtx<'_>) -> Result<(), String> {
    let plan = run.plan;
    let layout = ctx.layout;
    let (app, version) = (plan.app.as_str(), plan.version.as_str());
    match step {
        Step::EnsureLauncher => {
            let launcher = layout.launcher();
            if launcher.exists() {
                return Ok(());
            }
            if let Some(dir) = launcher.parent() {
                run.ensure_dirs(dir)?;
            }
            run.push(Undo::RemovePath(launcher));
            shim::ensure_launcher(ctx.http, ctx.data, layout)
                .await
                .map_err(|e| e.to_string())
        }
        Step::Download { index, source } => {
            let paths = download::fetch_all(
                ctx.http,
                ctx.data,
                std::slice::from_ref(source),
                ctx.progress,
            )
            .await
            .map_err(|e| e.to_string())?;
            let p = paths
                .into_iter()
                .next()
                .ok_or_else(|| "download sem arquivo".to_string())?;
            run.downloads.insert(*index, p);
            Ok(())
        }
        Step::Extract { index } => {
            let source = plan
                .steps
                .iter()
                .find_map(|s| match s {
                    Step::Download { index: i, source } if i == index => Some(source),
                    _ => None,
                })
                .ok_or_else(|| format!("fonte [{}] sem download no plano", index + 1))?;
            let file = run
                .downloads
                .get(index)
                .cloned()
                .ok_or_else(|| format!("fonte [{}] não baixada", index + 1))?;
            let staging = layout.staging_dir(app, version);
            let owned = run
                .journal
                .iter()
                .any(|(_, u)| matches!(u, Undo::RemovePath(p) if *p == staging));
            if !owned {
                // A staging folder left by a crashed run is garbage of ours.
                not_found_ok(remove_any(&staging)).map_err(|e| io_msg(&staging, e))?;
                run.ensure_dirs(&layout.app_dir(app))?;
                run.push(Undo::RemovePath(staging.clone()));
            }
            extract::extract(
                &file,
                &source.file_name,
                source.extract_dir.as_deref(),
                source.extract_to.as_deref(),
                &staging,
            )
            .map_err(|e| e.to_string())
        }
        Step::Promote => {
            let staging = layout.staging_dir(app, version);
            let vdir = layout.version_dir(app, version);
            if fs::symlink_metadata(&vdir).is_ok() {
                return Err(format!("{} já existe", vdir.display()));
            }
            fs::rename(&staging, &vdir).map_err(|e| io_msg(&vdir, e))?;
            run.push(Undo::RemovePath(vdir.clone()));
            let arch = plan.arch.map(|a| a.key());
            state::write_install_json(&vdir, &plan.bucket, arch, version)
                .map_err(|e| io_msg(&vdir, e))
        }
        Step::Persist(spec) => persist(run, spec, layout),
        Step::LinkCurrent => {
            let current = layout.current(app);
            let previous = match fs::symlink_metadata(&current) {
                Ok(m) if m.file_type().is_symlink() => {
                    Some(link::target(&current).map_err(|e| io_msg(&current, e))?)
                }
                Ok(_) => return Err(format!("{} não é uma junção", current.display())),
                Err(_) => None,
            };
            run.push(Undo::RestoreCurrent {
                link: current.clone(),
                previous: previous.clone(),
            });
            if previous.is_some() {
                link::remove(&current).map_err(|e| io_msg(&current, e))?;
            }
            link::create(&layout.version_dir(app, version), &current)
                .map_err(|e| io_msg(&current, e))
        }
        Step::Shim(spec) => make_shim(run, spec, ctx),
        Step::Shortcut(spec) => {
            let target = spec.link.display().to_string();
            if run
                .previous_artifact(ArtifactKind::Shortcut, &target)
                .is_some()
            {
                run.push(Undo::RecreateShortcut(spec.clone()));
            } else {
                run.push(Undo::RemoveShortcut(spec.link.clone()));
            }
            ctx.effects
                .shortcuts
                .create(
                    &spec.link,
                    &spec.target,
                    spec.args.as_deref(),
                    spec.icon.as_deref(),
                )
                .map_err(|e| e.to_string())?;
            run.artifacts.push(Artifact {
                kind: ArtifactKind::Shortcut,
                target,
                previous: None,
            });
            Ok(())
        }
        Step::EnvSet { name, value } => {
            let env = ctx.effects.env;
            let before = env.get(name).map_err(|e| e.to_string())?;
            // An upgrade keeps the value from before the first install.
            let recorded_previous = match run.previous_artifact(ArtifactKind::Env, name) {
                Some(a) => a.previous.clone(),
                None => before.as_ref().map(|v| v.value.clone()),
            };
            run.push(Undo::RestoreEnv {
                name: name.clone(),
                previous: before,
            });
            env.set(
                name,
                &EnvValue {
                    value: value.clone(),
                    expand: value.contains('%'),
                },
            )
            .map_err(|e| e.to_string())?;
            run.artifacts.push(Artifact {
                kind: ArtifactKind::Env,
                target: name.clone(),
                previous: recorded_previous,
            });
            Ok(())
        }
        Step::EnvAddPath { entry } => add_path(run, ctx.effects.env, &entry.display().to_string()),
        Step::EnsureShimsOnPath => {
            add_path(run, ctx.effects.env, &layout.shims().display().to_string())
        }
        Step::RecordState => {
            run.push(Undo::RestoreState {
                app: app.to_string(),
                previous: run.previous.clone(),
            });
            let rec = AppRecord {
                app: app.to_string(),
                bucket: plan.bucket.clone(),
                version: version.to_string(),
                arch: plan.arch.map(|a| a.key().to_string()),
                installed_at: now_secs(),
                manifest_sha256: String::new(),
            };
            let arts = run.artifacts.clone();
            ctx.state.record(&rec, &arts).map_err(|e| e.to_string())
        }
        Step::RemoveOldVersion { .. } => Ok(()),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn add_path(run: &mut Run<'_>, env: &dyn EnvStore, entry: &str) -> Result<(), String> {
    let cur = env.get(PATH_VAR).map_err(|e| e.to_string())?;
    let existing = cur.as_ref().map(|v| v.value.as_str()).unwrap_or("");
    if !path_edit::path_contains(existing, entry) {
        run.push(Undo::PathRemove {
            entry: entry.to_string(),
            var_existed: cur.is_some(),
        });
        env.set(
            PATH_VAR,
            &EnvValue {
                value: path_edit::path_add(existing, entry),
                expand: cur.as_ref().is_none_or(|v| v.expand),
            },
        )
        .map_err(|e| e.to_string())?;
    }
    run.artifacts.push(Artifact {
        kind: ArtifactKind::Path,
        target: entry.to_string(),
        previous: None,
    });
    Ok(())
}

fn make_shim(run: &mut Run<'_>, spec: &ShimSpec, ctx: &mut ExecCtx<'_>) -> Result<(), String> {
    let app = run.plan.app.as_str();
    for rec in ctx.state.apps().map_err(|e| e.to_string())? {
        if rec.app == app {
            continue;
        }
        let owns = ctx
            .state
            .get(&rec.app)
            .map_err(|e| e.to_string())?
            .is_some_and(|(_, arts)| {
                arts.iter().any(|a| {
                    a.kind == ArtifactKind::Shim && a.target.eq_ignore_ascii_case(&spec.name)
                })
            });
        if owns {
            return Err(format!("o comando {} já pertence a {}", spec.name, rec.app));
        }
    }
    let dir = ctx.layout.shims();
    run.ensure_dirs(&dir)?;
    let mut saved = Vec::new();
    for ext in ["exe", "shim", "cmd", "ps1"] {
        let p = dir.join(format!("{}.{ext}", spec.name));
        match fs::read(&p) {
            Ok(bytes) => saved.push((p, bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_msg(&p, e)),
        }
    }
    run.push(Undo::RestoreShim {
        name: spec.name.clone(),
        saved,
    });
    shim::create_shim(ctx.layout, spec).map_err(|e| e.to_string())?;
    run.artifacts.push(Artifact {
        kind: ArtifactKind::Shim,
        target: spec.name.clone(),
        previous: None,
    });
    Ok(())
}

/// Scoop's `persist_data`: the disk decides the kind; the plan's kind only applies when
/// the item exists neither in the version nor in `persist\`.
fn persist(run: &mut Run<'_>, spec: &PersistSpec, layout: &Layout) -> Result<(), String> {
    let (app, version) = (run.plan.app.as_str(), run.plan.version.as_str());
    let src = layout.version_dir(app, version).join(&spec.in_version);
    let dst = layout.persist_dir(app).join(&spec.in_persist);
    let src_meta = fs::symlink_metadata(&src).ok();
    let dst_meta = fs::symlink_metadata(&dst).ok();
    let is_dir = match (&src_meta, &dst_meta) {
        (Some(m), _) | (None, Some(m)) => m.is_dir(),
        (None, None) => spec.kind == PersistKind::Dir,
    };
    if let Some(parent) = dst.parent() {
        run.ensure_dirs(parent)?;
    }
    // Only what this step creates in persist\ may be deleted by its undo.
    let mut created = false;
    match (src_meta.is_some(), dst_meta.is_some()) {
        (true, false) => {
            fs::rename(&src, &dst).map_err(|e| io_msg(&dst, e))?;
            created = true;
        }
        (_, true) => {
            // The user's data wins; the version's copy is part of the version dir.
            remove_any(&src).map_err(|e| io_msg(&src, e))?;
        }
        (false, false) => {
            let r = if is_dir {
                fs::create_dir(&dst)
            } else {
                fs::File::create(&dst).map(drop)
            };
            r.map_err(|e| io_msg(&dst, e))?;
            created = true;
        }
    }
    if created {
        run.push(Undo::RemovePath(dst.clone()));
    }
    if let Some(parent) = src.parent() {
        run.ensure_dirs(parent)?;
    }
    if is_dir {
        link::create(&dst, &src).map_err(|e| io_msg(&src, e))?;
    } else {
        fs::hard_link(&dst, &src).map_err(|e| io_msg(&src, e))?;
    }
    run.push(Undo::RemovePath(src));
    run.artifacts.push(Artifact {
        kind: ArtifactKind::Persist,
        target: spec.in_persist.display().to_string(),
        previous: None,
    });
    Ok(())
}

/// Removes what the app's record lists. Nothing changes when a file under `apps\<app>` is in use.
pub fn execute_uninstall(plan: &UninstallPlan, ctx: &mut ExecCtx<'_>) -> Result<(), ExecError> {
    let fail = |index: usize, step: String, message: String| ExecError::Failed {
        index,
        step,
        message,
        undone: Vec::new(),
    };
    check_component("app", &plan.app).map_err(|m| fail(0, "validar plano".into(), m))?;
    let layout = ctx.layout;
    let app_dir = layout.app_dir(&plan.app);
    if let Some(p) = (ctx.in_use)(&app_dir).first() {
        return Err(ExecError::InUse(p.display().to_string()));
    }
    let env = ctx.effects.env;
    for (i, a) in plan.artifacts.iter().enumerate() {
        let (label, r) = match a.kind {
            ArtifactKind::Shim => (
                format!("comando {}", a.target),
                shim::remove_shim(layout, &a.target).map_err(|e| e.to_string()),
            ),
            ArtifactKind::Shortcut => (
                format!("atalho {}", a.target),
                ctx.effects
                    .shortcuts
                    .remove(Path::new(&a.target))
                    .map_err(|e| e.to_string()),
            ),
            ArtifactKind::Env => {
                let previous = a.previous.as_ref().map(|v| EnvValue {
                    value: v.clone(),
                    expand: v.contains('%'),
                });
                (
                    format!("variável {}", a.target),
                    restore_env(env, &a.target, previous.as_ref()).map_err(|e| e.to_string()),
                )
            }
            ArtifactKind::Path => {
                let r = if plan.remove_path_entry {
                    ctx.state
                        .path_users(&plan.app, &a.target)
                        .map_err(|e| e.to_string())
                        .and_then(|users| {
                            if users.is_empty() {
                                path_remove_entry(env, &a.target, true).map_err(|e| e.to_string())
                            } else {
                                Ok(())
                            }
                        })
                } else {
                    Ok(())
                };
                (format!("PATH {}", a.target), r)
            }
            // persist\<app> as a whole is handled below.
            ArtifactKind::Persist => continue,
        };
        r.map_err(|m| fail(i, label, m))?;
    }
    let n = plan.artifacts.len();
    remove_any(&app_dir).map_err(|e| fail(n, "remover app".into(), io_msg(&app_dir, e)))?;
    if plan.purge {
        let pd = layout.persist_dir(&plan.app);
        remove_any(&pd).map_err(|e| fail(n + 1, "remover persist".into(), io_msg(&pd, e)))?;
    }
    ctx.state
        .remove(&plan.app)
        .map_err(|e| fail(n + 2, "registro".into(), e.to_string()))
}
