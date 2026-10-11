//! Install operations: the API the CLI and the TUI call. Joins target resolution, manifests,
//! the planner, the executor, `state.db` and the residue scan, with dependencies and
//! `install.lock`. Every side effect outside the root goes through `OpsCtx::effects`.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::Instrument;

use crate::cache::{DataDir, UpdateLock};
use crate::catalog::{Catalog, CatalogError};
use crate::core::SourceKind;
use crate::core::version::Version;
use crate::http::Http;
use crate::install::download::{self, DownloadError, Progress};
use crate::install::executor::{self, ExecCtx, ExecError, InUseCheck, NoHook};
use crate::install::layout::Layout;
use crate::install::manifest::{self, Arch, ManifestError, Resolved};
use crate::install::path_edit;
use crate::install::plan::{InstallPlan, Step, UninstallPlan};
use crate::install::planner::{self, InstalledInfo, PlanError, PlanInput};
use crate::install::residue::{self, Residue, ResidueInput};
use crate::install::state::{AppRecord, Artifact, ArtifactKind, StateDb, StateError};
use crate::install::target::{self, Candidate, TargetError};
use crate::install::traits::{EffectError, Effects, EnvStore, EnvValue};
use crate::update::LockWait;

/// User environment variable holding the PATH.
const PATH_VAR: &str = "Path";

/// HKCU-relative keys whose values may point into a removed app (spec §8, high confidence).
const RESIDUE_REG_ROOTS: &[&str] = &[
    r"Software\Classes",
    r"Software\Microsoft\Windows\CurrentVersion\Run",
    r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
    r"Software\Microsoft\Windows\CurrentVersion\App Paths",
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
];

/// Parent of the low-confidence registry candidates (HKCU-relative).
const SOFTWARE_KEY: &str = "Software";

/// How deep the shortcut folders are walked (Start Menu\Programs\<folder>\x.lnk).
const SHORTCUT_DEPTH: usize = 4;

/// Where the residue scan looks besides the registry. Injected so tests never read the
/// real profile; production uses [`ResidueScan::user_profile`].
pub struct ResidueScan<'a> {
    /// Folders whose `.lnk` files are checked: Desktop, Start Menu (includes Startup).
    pub shortcut_dirs: Vec<PathBuf>,
    /// Folders whose direct subfolders are low-confidence candidates: `%APPDATA%`, `%LOCALAPPDATA%`.
    pub folder_roots: Vec<PathBuf>,
    /// Target of a `.lnk` (production: `shortcut_win::link_target`).
    pub link_target: &'a (dyn Fn(&Path) -> Option<PathBuf> + Send + Sync),
    /// The same app is also installed by scoop/winget.
    pub other_installs: &'a (dyn Fn(&str) -> bool + Send + Sync),
}

fn no_link_target(_: &Path) -> Option<PathBuf> {
    None
}
fn no_other_install(_: &str) -> bool {
    false
}

impl ResidueScan<'static> {
    /// Registry only: no folders, no shortcuts.
    pub fn none() -> ResidueScan<'static> {
        ResidueScan {
            shortcut_dirs: Vec::new(),
            folder_roots: Vec::new(),
            link_target: &no_link_target,
            other_installs: &no_other_install,
        }
    }
}

impl<'a> ResidueScan<'a> {
    /// The current user's Desktop, Start Menu, `%APPDATA%` and `%LOCALAPPDATA%`.
    pub fn user_profile(
        link_target: &'a (dyn Fn(&Path) -> Option<PathBuf> + Send + Sync),
        other_installs: &'a (dyn Fn(&str) -> bool + Send + Sync),
    ) -> ResidueScan<'a> {
        let var = |n: &str| {
            std::env::var_os(n)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let appdata = var("APPDATA");
        let mut shortcut_dirs = Vec::new();
        if let Some(home) = var("USERPROFILE") {
            shortcut_dirs.push(home.join("Desktop"));
        }
        if let Some(a) = &appdata {
            shortcut_dirs.push(a.join(r"Microsoft\Windows\Start Menu"));
        }
        ResidueScan {
            shortcut_dirs,
            folder_roots: appdata.into_iter().chain(var("LOCALAPPDATA")).collect(),
            link_target,
            other_installs,
        }
    }
}

/// True when scoop has `app` installed (`~/scoop/apps/<app>/current`).
pub fn installed_by_scoop(app: &str) -> bool {
    crate::installed::scoop::scoop_root()
        .is_some_and(|r| r.join("apps").join(app).join("current").exists())
}

pub struct OpsCtx<'a> {
    pub data: &'a DataDir,
    pub http: &'a Http,
    pub catalog: &'a Catalog,
    pub effects: Effects<'a>,
    /// CLI: `LockWait::from_env()` (30 s); TUI: `NoWait` (busy is reported, never waited).
    pub wait: LockWait,
    pub progress: Progress<'a>,
    /// Production: `install::fs_win::files_in_use`.
    pub in_use: InUseCheck<'a>,
    pub residue: ResidueScan<'a>,
}

#[derive(Debug)]
#[expect(
    clippy::large_enum_variant,
    reason = "built once per command; the brief fixes `One(Candidate)`"
)]
pub enum Resolution {
    One(Candidate),
    Many(Vec<Candidate>),
    NotFound { suggestions: Vec<String> },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RepairReport {
    /// Apps whose record was rebuilt from disk.
    pub rebuilt: Vec<String>,
    /// Apps whose record already existed and was kept.
    pub kept: Vec<String>,
    /// Records of apps no longer on disk, removed.
    pub removed: Vec<String>,
    /// App folders without a readable `current\install.json` (broken; left as they are).
    pub skipped: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    #[error("nenhum pacote encontrado para '{query}'")]
    NotFound {
        query: String,
        suggestions: Vec<String>,
    },
    #[error("mais de um pacote corresponde: {}", .0.join(", "))]
    Ambiguous(Vec<String>),
    #[error("não suportado: {0}")]
    Unsupported(String),
    #[error("dependência cíclica: {}", .0.join(" -> "))]
    DependencyCycle(Vec<String>),
    #[error("a dependência {app} não pode ser instalada: {reason}")]
    DependencyUnsupported { app: String, reason: String },
    #[error("outros apps dependem deste: {}", .0.join(", "))]
    HasDependents(Vec<String>),
    #[error("{0} não está instalado pelo wingetrs")]
    NotInstalled(String),
    #[error("arquivo em uso: {0} — feche o programa e tente de novo")]
    InUse(String),
    #[error("outra instalação do wingetrs está em andamento; tente de novo em instantes")]
    Busy,
    #[error("não foi possível obter o install.lock: {0}")]
    Lock(String),
    #[error("falha ao baixar {app}: {error}")]
    Download { app: String, error: DownloadError },
    #[error("o estado de {0} mudou desde o planejamento; tente de novo")]
    StateChanged(String),
    #[error("{error} (já instalados: {})", .applied.join(", "))]
    PartlyApplied {
        applied: Vec<String>,
        error: Box<OpsError>,
    },
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Exec(ExecError),
    #[error(transparent)]
    Manifest(ManifestError),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    State(#[from] StateError),
    #[error("erro no ambiente do usuário: {0}")]
    Effect(#[from] EffectError),
}

impl From<ExecError> for OpsError {
    fn from(e: ExecError) -> OpsError {
        match e {
            ExecError::InUse(p) => OpsError::InUse(p),
            other => OpsError::Exec(other),
        }
    }
}

impl From<ManifestError> for OpsError {
    fn from(e: ManifestError) -> OpsError {
        match e {
            ManifestError::Unsupported { reason } => OpsError::Unsupported(reason),
            other => OpsError::Manifest(other),
        }
    }
}

fn layout(ctx: &OpsCtx<'_>) -> Layout {
    ctx.data.install_layout()
}

fn open_state(layout: &Layout) -> Result<StateDb, OpsError> {
    Ok(StateDb::open(&layout.state_db())?)
}

/// Read-only view for planning and queries: never creates `state.db`; an absent file reads
/// as "nothing installed".
pub fn read_state(layout: &Layout) -> Result<StateDb, OpsError> {
    match StateDb::open_existing(&layout.state_db())? {
        Some(s) => Ok(s),
        None => Ok(StateDb::empty_in_memory()?),
    }
}

fn effects<'a>(ctx: &'a OpsCtx<'_>) -> Effects<'a> {
    Effects {
        env: ctx.effects.env,
        shortcuts: ctx.effects.shortcuts,
        trash: ctx.effects.trash,
        registry: ctx.effects.registry,
    }
}

fn exec_ctx<'a>(ctx: &'a OpsCtx<'_>, layout: &'a Layout, state: &'a mut StateDb) -> ExecCtx<'a> {
    ExecCtx {
        layout,
        data: ctx.data,
        http: ctx.http,
        effects: effects(ctx),
        state,
        hook: &NoHook,
        progress: ctx.progress,
        in_use: ctx.in_use,
    }
}

fn lock_timeout(wait: LockWait) -> Duration {
    match wait {
        LockWait::Block(d) => d,
        LockWait::NoWait => Duration::ZERO,
    }
}

fn lock_result(
    r: Result<Option<UpdateLock>, crate::cache::CacheError>,
) -> Result<UpdateLock, OpsError> {
    r.map_err(|e| OpsError::Lock(e.to_string()))?
        .ok_or(OpsError::Busy)
}

fn lock_blocking(layout: &Layout, wait: LockWait) -> Result<UpdateLock, OpsError> {
    lock_result(UpdateLock::acquire_timeout(
        &layout.install_lock(),
        lock_timeout(wait),
    ))
}

async fn lock_async(layout: &Layout, wait: LockWait) -> Result<UpdateLock, OpsError> {
    let path = layout.install_lock();
    let timeout = lock_timeout(wait);
    let handle = tokio::task::spawn_blocking(move || UpdateLock::acquire_timeout(&path, timeout));
    let r = match handle.await {
        Ok(r) => r,
        Err(e) => match e.try_into_panic() {
            Ok(p) => std::panic::resume_unwind(p),
            Err(e) => return Err(OpsError::Lock(e.to_string())),
        },
    };
    lock_result(r)
}

/// Values of `names` in the user environment (`None` = unset), to detect a net change.
type EnvSnapshot = Vec<Option<EnvValue>>;

fn env_snapshot(env: &dyn EnvStore, names: &[String]) -> Result<EnvSnapshot, EffectError> {
    names.iter().map(|n| env.get(n)).collect()
}

/// Broadcasts once when any of `names` differs from `before` (an unreadable value counts
/// as changed). A rollback that restored everything leaves nothing to announce.
fn broadcast_if_changed(env: &dyn EnvStore, names: &[String], before: &EnvSnapshot) {
    let changed = match env_snapshot(env, names) {
        Ok(after) => after != *before,
        Err(e) => {
            tracing::warn!("não foi possível reler o ambiente: {e}");
            true
        }
    };
    if changed {
        env.broadcast();
    }
}

fn env_names(extra: impl Iterator<Item = String>) -> Vec<String> {
    let mut names = vec![PATH_VAR.to_string()];
    for n in extra {
        if !names.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
            names.push(n);
        }
    }
    names
}

/// `scoop:main` -> `main`.
fn bucket_of(source_name: &str) -> Option<&str> {
    source_name.strip_prefix("scoop:")
}

/// The manifest JSON of `id` in `source`.
fn manifest_raw(catalog: &Catalog, source: &str, id: &str) -> Result<Option<String>, OpsError> {
    Ok(catalog.get(source, id)?.and_then(|p| p.raw))
}

/// Exact-id resolution (Task 35); `resolve_target` never fuzzy-matches.
pub fn resolve_target(
    ctx: &OpsCtx<'_>,
    input: &str,
    bucket: Option<&str>,
) -> Result<Resolution, OpsError> {
    let _span = tracing::debug_span!("target.resolve").entered();
    match target::resolve(ctx.catalog, input, bucket) {
        Ok(mut found) if found.len() == 1 => match found.pop() {
            Some(c) => Ok(Resolution::One(c)),
            None => Ok(Resolution::NotFound {
                suggestions: Vec::new(),
            }),
        },
        Ok(found) if found.is_empty() => Ok(Resolution::NotFound {
            suggestions: Vec::new(),
        }),
        Ok(found) => Ok(Resolution::Many(found)),
        Err(TargetError::NotFound { suggestions, .. }) => Ok(Resolution::NotFound { suggestions }),
        Err(TargetError::Catalog(e)) => Err(e.into()),
    }
}

/// One app to plan: bucket, id, manifest JSON and its resolution.
struct Node {
    bucket: String,
    id: String,
    raw: String,
    manifest: Resolved,
}

/// Finds the catalog entry of a `depends` item: `bucket/app` exactly, else `app` in the
/// dependent's bucket, else the only scoop bucket that has it, else `main`.
fn find_dependency(
    catalog: &Catalog,
    parent_bucket: &str,
    spec: &str,
) -> Result<(String, String, String), OpsError> {
    let (explicit, name) = match spec.split_once('/') {
        Some((b, n)) if !b.is_empty() && !n.is_empty() => (Some(b), n),
        _ => (None, spec),
    };
    let unsupported = |reason: String| OpsError::DependencyUnsupported {
        app: name.to_string(),
        reason,
    };
    let found: Vec<_> = catalog
        .find_by_id(name)?
        .into_iter()
        .filter(|p| p.entry.source == SourceKind::Scoop)
        .collect();
    let in_bucket = |b: &str| {
        let want = crate::sources::scoop::source_name(b);
        found
            .iter()
            .find(|p| p.entry.source_name.eq_ignore_ascii_case(&want))
    };
    let pick = match explicit {
        Some(b) => in_bucket(b),
        None => in_bucket(parent_bucket)
            .or_else(|| (found.len() == 1).then(|| &found[0]))
            .or_else(|| in_bucket("main")),
    };
    let Some(pkg) = pick else {
        if explicit.is_none() && found.len() > 1 {
            let buckets: Vec<&str> = found.iter().map(|p| p.entry.source_name.as_str()).collect();
            return Err(unsupported(format!(
                "ambígua entre buckets: {}",
                buckets.join(", ")
            )));
        }
        return Err(unsupported("não encontrada no catálogo".into()));
    };
    let bucket = bucket_of(&pkg.entry.source_name)
        .unwrap_or(parent_bucket)
        .to_string();
    let raw = pkg
        .raw
        .clone()
        .ok_or_else(|| unsupported("sem manifesto no catálogo".into()))?;
    Ok((bucket, pkg.entry.id.clone(), raw))
}

struct DepWalk<'s> {
    catalog: &'s Catalog,
    state: &'s StateDb,
    /// Ids from the target down to the item being visited.
    stack: Vec<String>,
    /// Lowercased ids already planned or already installed.
    seen: HashSet<String>,
    out: Vec<Node>,
}

impl DepWalk<'_> {
    /// Post-order walk: every dependency lands in `out` before its dependents.
    fn visit(&mut self, parent_bucket: &str, spec: &str) -> Result<(), OpsError> {
        let (bucket, id, raw) = find_dependency(self.catalog, parent_bucket, spec)?;
        if let Some(pos) = self.stack.iter().position(|s| s.eq_ignore_ascii_case(&id)) {
            let mut chain = self.stack[pos..].to_vec();
            chain.push(id);
            return Err(OpsError::DependencyCycle(chain));
        }
        let key = id.to_lowercase();
        if self.seen.contains(&key) {
            return Ok(());
        }
        if self.state.get(&id)?.is_some() {
            self.seen.insert(key);
            return Ok(());
        }
        let manifest =
            manifest::resolve(&raw, Arch::host()).map_err(|e| OpsError::DependencyUnsupported {
                app: id.clone(),
                reason: match e {
                    ManifestError::Unsupported { reason } => reason,
                    other => other.to_string(),
                },
            })?;
        self.stack.push(id.clone());
        for d in &manifest.depends {
            self.visit(&bucket, d)?;
        }
        self.stack.pop();
        self.seen.insert(key);
        self.out.push(Node {
            bucket,
            id,
            raw,
            manifest,
        });
        Ok(())
    }
}

/// Plans `c` (install or upgrade) preceded by its missing dependencies in topological
/// order. Pure with respect to the disk: nothing is downloaded or written.
pub fn plan_for(
    ctx: &OpsCtx<'_>,
    c: &Candidate,
    keep_old: bool,
) -> Result<Vec<InstallPlan>, OpsError> {
    let _span = tracing::debug_span!("plan.build").entered();
    let bucket = match (c.selectable, bucket_of(&c.entry.source_name)) {
        (true, Some(b)) if c.entry.source == SourceKind::Scoop => b.to_string(),
        _ => {
            return Err(OpsError::Unsupported(c.note.clone().unwrap_or_else(|| {
                format!("{} não vem de um bucket do scoop", c.entry.id)
            })));
        }
    };
    let raw = manifest_raw(ctx.catalog, &c.entry.source_name, &c.entry.id)?.ok_or_else(|| {
        OpsError::Unsupported(format!("{} sem manifesto no catálogo", c.entry.id))
    })?;
    let target = manifest::resolve(&raw, Arch::host())?;
    let layout = layout(ctx);
    let state = read_state(&layout)?;
    let mut walk = DepWalk {
        catalog: ctx.catalog,
        state: &state,
        stack: vec![c.entry.id.clone()],
        seen: HashSet::new(),
        out: Vec::new(),
    };
    for d in &target.depends {
        walk.visit(&bucket, d)?;
    }
    let mut nodes = walk.out;
    nodes.push(Node {
        bucket,
        id: c.entry.id.clone(),
        raw,
        manifest: target,
    });
    let shortcut_root = ctx.effects.shortcuts.root();
    let launcher_present = layout.launcher().exists();
    let mut plans = Vec::with_capacity(nodes.len());
    for n in &nodes {
        let installed = state
            .get(&n.id)?
            .map(|(r, _)| InstalledInfo { version: r.version });
        let sha = crate::cache::sha256_hex(n.raw.as_bytes());
        plans.push(planner::plan_install(&PlanInput {
            app: &n.id,
            bucket: &n.bucket,
            manifest: &n.manifest,
            layout: &layout,
            shortcut_root: &shortcut_root,
            installed,
            launcher_present,
            keep_old,
            manifest_sha256: &sha,
        })?);
    }
    Ok(plans)
}

/// Under the lock, before any change: each plan must still match `state.db` (its
/// `upgrade_from` is the installed version). A dependency that got installed meanwhile is
/// skipped; any other divergence is `StateChanged`. Returns the plans to run.
fn revalidate<'p>(
    state: &StateDb,
    plans: &'p [InstallPlan],
    targets: &std::collections::HashSet<String>,
) -> Result<Vec<&'p InstallPlan>, OpsError> {
    let mut run = Vec::with_capacity(plans.len());
    for plan in plans {
        let now = state.get(&plan.app)?.map(|(r, _)| r.version);
        if now == plan.upgrade_from {
            run.push(plan);
        } else if !targets.contains(&plan.app.to_lowercase()) && plan.upgrade_from.is_none() {
            // A dependency someone else installed meanwhile: already satisfied.
        } else {
            return Err(OpsError::StateChanged(plan.app.clone()));
        }
    }
    Ok(run)
}

/// Applies `plans` in order under `install.lock`, stopping at the first failure (that plan
/// is rolled back; earlier ones, its dependencies, stay installed and are named in
/// `PartlyApplied`). The environment change is broadcast once at the end when any
/// variable ended up different.
pub async fn apply(ctx: &OpsCtx<'_>, plans: &[InstallPlan]) -> Result<(), OpsError> {
    let targets = plans
        .last()
        .map(|p| p.app.to_lowercase())
        .into_iter()
        .collect();
    apply_with_targets(ctx, plans, &targets).await
}

/// Concatenates plan groups in order, dropping a plan whose app (case-insensitive) already
/// appeared earlier (a shared dependency runs once, at its first position).
pub fn merge_groups(groups: Vec<Vec<InstallPlan>>) -> Vec<InstallPlan> {
    let mut seen = std::collections::HashSet::new();
    groups
        .into_iter()
        .flatten()
        .filter(|p| seen.insert(p.app.to_lowercase()))
        .collect()
}

/// Applies several plan groups in one `apply` (one lock, one broadcast). The target of each
/// group (its last plan) is never skipped by revalidation. Returns the merged plans.
pub async fn apply_groups(
    ctx: &OpsCtx<'_>,
    groups: Vec<Vec<InstallPlan>>,
) -> Result<Vec<InstallPlan>, OpsError> {
    let targets = groups
        .iter()
        .filter_map(|g| g.last())
        .map(|p| p.app.to_lowercase())
        .collect();
    let merged = merge_groups(groups);
    apply_with_targets(ctx, &merged, &targets).await?;
    Ok(merged)
}

/// Downloads every source of every plan at once (inside the lock: the blob index is not
/// safe across processes), so a failed download aborts before any change and the executor
/// then finds each file in the blob cache. Only the revalidated plans are fetched.
async fn prefetch(ctx: &OpsCtx<'_>, plans: &[&InstallPlan]) -> Result<(), OpsError> {
    // The same file in two plans is fetched once; an error names its first owner.
    let mut seen = HashSet::new();
    let (apps, sources): (Vec<&str>, Vec<_>) = plans
        .iter()
        .flat_map(|p| {
            p.steps.iter().filter_map(|s| match s {
                Step::Download { source, .. } => Some((p.app.as_str(), source.clone())),
                _ => None,
            })
        })
        .filter(|(_, src)| seen.insert(download::algo_key(&src.hash)))
        .unzip();
    let results = download::fetch_each(ctx.http, ctx.data, &sources, ctx.progress).await;
    for (app, r) in apps.into_iter().zip(results) {
        if let Err(error) = r {
            return Err(OpsError::Download {
                app: app.to_string(),
                error,
            });
        }
    }
    Ok(())
}

async fn apply_with_targets(
    ctx: &OpsCtx<'_>,
    plans: &[InstallPlan],
    targets: &std::collections::HashSet<String>,
) -> Result<(), OpsError> {
    let layout = layout(ctx);
    let _lock = lock_async(&layout, ctx.wait)
        .instrument(tracing::debug_span!("install.lock"))
        .await?;
    let mut state = open_state(&layout)?;
    let plans = tracing::debug_span!("install.revalidate")
        .in_scope(|| revalidate(&state, plans, targets))?;
    prefetch(ctx, &plans)
        .instrument(tracing::debug_span!(
            "install.prefetch",
            plans = plans.len()
        ))
        .await?;
    let names = env_names(plans.iter().flat_map(|p| &p.steps).filter_map(|s| match s {
        Step::EnvSet { name, .. } => Some(name.clone()),
        _ => None,
    }));
    let env = ctx.effects.env;
    let before = env_snapshot(env, &names)?;
    let mut result = Ok(());
    let mut applied: Vec<String> = Vec::new();
    for plan in plans {
        let mut exec = exec_ctx(ctx, &layout, &mut state);
        let step_span = tracing::debug_span!("install.execute", app = %plan.app);
        if let Err(e) = executor::execute_install(plan, &mut exec)
            .instrument(step_span)
            .await
        {
            let error = OpsError::from(e);
            result = Err(if applied.is_empty() {
                error
            } else {
                OpsError::PartlyApplied {
                    applied,
                    error: Box::new(error),
                }
            });
            break;
        }
        applied.push(plan.app.clone());
    }
    tracing::debug_span!("install.broadcast")
        .in_scope(|| broadcast_if_changed(env, &names, &before));
    result
}

/// `depends` items of a manifest JSON (string or list); unreadable JSON has none.
fn raw_depends(raw: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    match v.get("depends") {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Other wingetrs apps whose catalog manifest depends on `app` (by name, any bucket).
fn dependents(ctx: &OpsCtx<'_>, state: &StateDb, app: &str) -> Result<Vec<String>, OpsError> {
    let mut out = Vec::new();
    for rec in state.apps()? {
        if rec.app.eq_ignore_ascii_case(app) {
            continue;
        }
        let source = crate::sources::scoop::source_name(&rec.bucket);
        let Some(raw) = manifest_raw(ctx.catalog, &source, &rec.app)? else {
            tracing::warn!(
                "{}: manifesto ausente do catálogo; dependências desconhecidas",
                rec.app
            );
            continue;
        };
        let depends_on_app = raw_depends(&raw).iter().any(|d| {
            let name = d.rsplit_once('/').map_or(d.as_str(), |(_, n)| n);
            name.eq_ignore_ascii_case(app)
        });
        if depends_on_app {
            out.push(rec.app);
        }
    }
    Ok(out)
}

fn refuse_if_needed(ctx: &OpsCtx<'_>, state: &StateDb, app: &str) -> Result<(), OpsError> {
    let who = dependents(ctx, state, app)?;
    if who.is_empty() {
        Ok(())
    } else {
        Err(OpsError::HasDependents(who))
    }
}

/// What uninstalling `app` removes; refuses while another wingetrs app depends on it.
pub fn plan_uninstall(ctx: &OpsCtx<'_>, app: &str, purge: bool) -> Result<UninstallPlan, OpsError> {
    let state = read_state(&layout(ctx))?;
    let plan = UninstallPlan::from_state(&state, app, purge)?
        .ok_or_else(|| OpsError::NotInstalled(app.to_string()))?;
    refuse_if_needed(ctx, &state, &plan.app)?;
    Ok(plan)
}

/// Uninstalls under `install.lock` (dependents are checked here and files in use by the
/// executor, both before anything changes), broadcasts the environment change once, and returns the
/// classified residues; the caller confirms and calls `residue::clean`.
pub fn uninstall(ctx: &OpsCtx<'_>, plan: &UninstallPlan) -> Result<Vec<Residue>, OpsError> {
    let layout = layout(ctx);
    let _lock =
        tracing::debug_span!("uninstall.lock").in_scope(|| lock_blocking(&layout, ctx.wait))?;
    let mut state = open_state(&layout)?;
    let homepage = match state.get(&plan.app)? {
        Some((rec, _)) if rec.version == plan.version => homepage_of(ctx.catalog, &rec)?,
        _ => return Err(OpsError::StateChanged(plan.app.clone())),
    };
    refuse_if_needed(ctx, &state, &plan.app)?;
    let app_dir = layout.app_dir(&plan.app);
    let names = env_names(
        plan.artifacts
            .iter()
            .filter(|a| a.kind == ArtifactKind::Env)
            .map(|a| a.target.clone()),
    );
    let env = ctx.effects.env;
    let before = env_snapshot(env, &names)?;
    let result = {
        let mut exec = exec_ctx(ctx, &layout, &mut state);
        tracing::debug_span!("uninstall.execute")
            .in_scope(|| executor::execute_uninstall(plan, &mut exec))
    };
    tracing::debug_span!("uninstall.broadcast")
        .in_scope(|| broadcast_if_changed(env, &names, &before));
    result?;
    Ok(tracing::debug_span!("uninstall.residue_scan")
        .in_scope(|| scan_residues(ctx, &app_dir, &plan.app, homepage.as_deref())))
}

fn homepage_of(catalog: &Catalog, rec: &AppRecord) -> Result<Option<String>, OpsError> {
    let source = crate::sources::scoop::source_name(&rec.bucket);
    Ok(manifest_raw(catalog, &source, &rec.app)?.and_then(|raw| {
        serde_json::from_str::<serde_json::Value>(&raw)
            .ok()?
            .get("homepage")?
            .as_str()
            .map(str::to_string)
    }))
}

/// `.lnk` files under `dir`, at most `depth` levels down.
fn lnk_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let p = entry.path();
        if ft.is_dir() && depth > 0 {
            lnk_files(&p, depth - 1, out);
        } else if ft.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
            out.push(p);
        }
    }
}

fn subdirs(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect()
}

/// Classifies what the removed app may have left (spec §8). A failing probe only narrows
/// the result: the uninstall itself already succeeded.
fn scan_residues(
    ctx: &OpsCtx<'_>,
    app_dir: &Path,
    app: &str,
    homepage: Option<&str>,
) -> Vec<Residue> {
    let registry = ctx.effects.registry;
    let reg_hits = registry
        .values_containing(RESIDUE_REG_ROOTS, &app_dir.display().to_string())
        .unwrap_or_else(|e| {
            tracing::warn!("varredura do registro falhou: {e}");
            Vec::new()
        });
    let mut names = vec![app];
    if let Some(owner) = homepage.and_then(residue::github_owner) {
        names.push(owner);
    }
    let software_keys = registry
        .subkeys_named(SOFTWARE_KEY, &names)
        .unwrap_or_else(|e| {
            tracing::warn!("varredura de HKCU\\Software falhou: {e}");
            Vec::new()
        });
    let scan = &ctx.residue;
    let mut links = Vec::new();
    for d in &scan.shortcut_dirs {
        lnk_files(d, SHORTCUT_DEPTH, &mut links);
    }
    let shortcuts: Vec<(PathBuf, PathBuf)> = links
        .into_iter()
        .filter_map(|l| (scan.link_target)(&l).map(|t| (l, t)))
        .collect();
    let folders: Vec<PathBuf> = scan.folder_roots.iter().flat_map(|r| subdirs(r)).collect();
    residue::classify(&ResidueInput {
        app,
        removed_dir: app_dir,
        homepage,
        reg_hits: &reg_hits,
        shortcuts: &shortcuts,
        folders: &folders,
        software_keys: &software_keys,
        other_installs: (scan.other_installs)(app),
    })
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UpgradeList {
    /// `(app, installed, latest)` whose catalog version is newer.
    pub upgradable: Vec<(String, String, String)>,
    /// `(app, reason)` left out: broken install or no longer in the catalog.
    pub skipped: Vec<(String, String)>,
}

/// Every `state.db` app whose catalog version is newer (`upgrade --all`).
pub fn upgradable(ctx: &OpsCtx<'_>) -> Result<UpgradeList, OpsError> {
    let _span = tracing::debug_span!("upgrade.list").entered();
    let layout = layout(ctx);
    let state = read_state(&layout)?;
    let mut out = UpgradeList::default();
    for rec in state.apps()? {
        if read_json(&layout.current(&rec.app).join("install.json")).is_none() {
            out.skipped
                .push((rec.app, "instalação quebrada (rode repair)".into()));
            continue;
        }
        let source = crate::sources::scoop::source_name(&rec.bucket);
        let Some(pkg) = ctx.catalog.get(&source, &rec.app)? else {
            out.skipped
                .push((rec.app, format!("não está mais no catálogo ({source})")));
            continue;
        };
        if Version::parse(&pkg.entry.version) > Version::parse(&rec.version) {
            out.upgradable
                .push((rec.app, rec.version, pkg.entry.version));
        }
    }
    Ok(out)
}

fn read_json(p: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

fn json_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_string)
}

/// Lowercased path text with `\` separators and no trailing separator.
fn norm_path(s: &str) -> String {
    s.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// `true` when `path` is `dir` or inside it. A path with a `.` or `..` component is never
/// under anything (`apps\foo\..\..\x` would otherwise pass the prefix test).
fn is_under(path: &str, dir: &str) -> bool {
    let (p, d) = (norm_path(path), norm_path(dir));
    if p.split('\\').any(|c| c == "." || c == "..") {
        return false;
    }
    p == d || p.starts_with(&format!("{d}\\"))
}

/// `(shim name, target)` from `shims\*.shim` (`path = "<target>"`) and `shims\*.cmd`
/// (`@rem <target>`).
fn shim_targets(layout: &Layout) -> Vec<(String, String)> {
    let Ok(rd) = std::fs::read_dir(layout.shims()) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        let (Some(stem), Some(ext)) = (
            p.file_stem().and_then(|s| s.to_str()),
            p.extension().and_then(|e| e.to_str()),
        ) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let first = text.lines().next().unwrap_or_default().trim();
        let target = match ext.to_ascii_lowercase().as_str() {
            "shim" => first
                .strip_prefix("path")
                .map(str::trim_start)
                .and_then(|r| r.strip_prefix('='))
                .map(|r| r.trim().trim_matches('"').to_string()),
            "cmd" => first.strip_prefix("@rem ").map(|r| r.trim().to_string()),
            _ => None,
        };
        if let Some(t) = target
            && !out.iter().any(|(n, _)| n.eq_ignore_ascii_case(stem))
        {
            out.push((stem.to_string(), t));
        }
    }
    out.sort();
    out
}

/// Rebuilds `state.db` records from disk under `install.lock`: each `apps\<app>` with a
/// readable `current\install.json` and no record gets one, with its shims (targets under
/// `apps\<app>\`), its PATH entries under `apps\<app>\` and, when `…\wingetrs\shims` is on
/// the PATH, that entry. Intact records are kept; records of apps gone from disk are removed.
pub fn repair(ctx: &OpsCtx<'_>) -> Result<RepairReport, OpsError> {
    let layout = layout(ctx);
    let _lock = lock_blocking(&layout, ctx.wait)?;
    let mut state = open_state(&layout)?;
    let path = ctx
        .effects
        .env
        .get(PATH_VAR)?
        .map(|v| v.value)
        .unwrap_or_default();
    let shims_dir = layout.shims().display().to_string();
    let shims_on_path = path_edit::path_contains(&path, &shims_dir);
    let shims = shim_targets(&layout);
    let mut report = RepairReport::default();
    let mut dirs: Vec<(String, PathBuf)> = std::fs::read_dir(layout.apps())
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    for (app, dir) in &dirs {
        if state.get(app)?.is_some() {
            report.kept.push(app.clone());
            continue;
        }
        let install = read_json(&dir.join("current").join("install.json"));
        let (Some(bucket), Some(version)) = (
            install.as_ref().and_then(|j| json_str(j, "bucket")),
            install.as_ref().and_then(|j| json_str(j, "version")),
        ) else {
            report.skipped.push(app.clone());
            continue;
        };
        let dir_text = dir.display().to_string();
        let mut artifacts: Vec<Artifact> = shims
            .iter()
            .filter(|(_, t)| is_under(t, &dir_text))
            .map(|(name, _)| Artifact {
                kind: ArtifactKind::Shim,
                target: name.clone(),
                previous: None,
            })
            .collect();
        for entry in path.split(';').filter(|e| is_under(e, &dir_text)) {
            artifacts.push(Artifact {
                kind: ArtifactKind::Path,
                target: entry.to_string(),
                previous: None,
            });
        }
        if shims_on_path {
            artifacts.push(Artifact {
                kind: ArtifactKind::Path,
                target: shims_dir.clone(),
                previous: None,
            });
        }
        let rec = AppRecord {
            app: app.clone(),
            bucket,
            version,
            arch: install.as_ref().and_then(|j| json_str(j, "architecture")),
            installed_at: crate::cache::now_unix(),
            manifest_sha256: String::new(),
        };
        state.record(&rec, &artifacts)?;
        report.rebuilt.push(app.clone());
    }
    for rec in state.apps()? {
        if !dirs.iter().any(|(a, _)| *a == rec.app) {
            state.remove(&rec.app)?;
            report.removed.push(rec.app);
        }
    }
    Ok(report)
}
