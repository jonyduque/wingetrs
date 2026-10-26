//! Install/remove/upgrade jobs of the TUI. Every function takes the `OpsCtx` as a parameter so
//! tests drive them with fakes; `run.rs` builds the production context.
use crate::bridge::elevated::ElevatedBridge;
use crate::bridge::{self, BridgeJob, BridgeOp, Tool};
use crate::catalog::{Catalog, CatalogError};
use crate::cli::output::removal_third_party;
use crate::core::PackageEntry;
use crate::core::version::Version;
use crate::install::executor::ExecError;
use crate::install::msi_queue;
use crate::install::ops::{self, Allow, AllowMsiexec, OpsCtx, OpsError, Resolution};
use crate::install::plan::{InstallPlan, UninstallPlan};
use crate::install::progress::{Phase, ProgressEvent};
use crate::install::residue::{self, Residue};
use crate::install::script_scan::CallRoots;
use crate::install::state::StateDb;
use crate::installed::correlate::InstallScope;
use crate::job_log::{JobInfo, JobLog, Outcome};
use crate::tui::app::{Action, Cmd, Done, InstallFail, Msg, Planned};
use crate::winget_install::engine::{self, UpgradeError, UpgradeRoute, upgrade_route};
use crate::winget_install::ops::{self as winget_ops, WingetCtx, WingetOpsError};
use crate::winget_install::orphans::{ArpRegistry, remove_orphan_with};
use crate::winget_install::plan::{
    WingetPlan as NativePlan, plan_runs_elevated, upgrade_runs_elevated,
};
use crate::winget_install::removal::{self, RemovalChoice, RemovalError, Removed, Steps};
use crate::winget_install::uninstall::{
    KEY_STAYED, MsiQueue, UninstallCtx, UninstallPlan as WingetUninstallPlan, UninstallStatus,
    UninstallTarget, removal_runs_elevated, remove_registry_only_with,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Minimum gap between two forwarded progress messages (about 20 per second).
const PROGRESS_GAP: Duration = Duration::from_millis(50);

/// Per-label progress state: `(done, total)`.
type Entries = BTreeMap<String, (u64, Option<u64>)>;

/// Sums progress across parallel downloads and lets at most one aggregate per `PROGRESS_GAP`
/// through, plus the moment the whole batch is complete.
pub struct ProgressThrottle {
    base: Instant,
    /// Milliseconds since `base` of the last admitted message; `u64::MAX` = none yet.
    last_ms: AtomicU64,
    entries: Mutex<Entries>,
}

impl ProgressThrottle {
    pub fn new(base: Instant) -> ProgressThrottle {
        ProgressThrottle {
            base,
            last_ms: AtomicU64::new(u64::MAX),
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// Records `label`'s progress; returns the aggregate `(label, done, total)` when it should
    /// be forwarded.
    pub fn admit(
        &self,
        now: Instant,
        label: &str,
        done: u64,
        total: Option<u64>,
    ) -> Option<(String, u64, Option<u64>)> {
        let ms = u64::try_from(now.saturating_duration_since(self.base).as_millis())
            .unwrap_or(u64::MAX - 1);
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.insert(label.to_string(), (done, total));
        let sum_done: u64 = entries.values().fold(0, |a, (d, _)| a.saturating_add(*d));
        let sum_total: Option<u64> = entries
            .values()
            .try_fold(0u64, |a, (_, t)| t.map(|t| a.saturating_add(t)));
        let last = self.last_ms.load(Ordering::Relaxed);
        let due = last == u64::MAX
            || ms.saturating_sub(last) >= PROGRESS_GAP.as_millis() as u64
            || sum_total == Some(sum_done);
        if !due {
            return None;
        }
        self.last_ms.store(ms, Ordering::Relaxed);
        let name = if entries.len() == 1 {
            label.to_string()
        } else {
            format!("{} arquivos", entries.len())
        };
        if sum_total == Some(sum_done) {
            entries.clear();
        }
        Some((name, sum_done, sum_total))
    }
}

/// Message for one progress report of a running action: phases (the `msiexec /a` launch
/// among them) always go through, download bytes through `throttle`.
pub fn progress_msg(throttle: &ProgressThrottle, now: Instant, ev: &ProgressEvent) -> Option<Msg> {
    match ev {
        ProgressEvent::Download { url, done, total } => throttle
            .admit(now, url, *done, *total)
            .map(|(label, done, total)| Msg::InstallProgress { label, done, total }),
        ProgressEvent::Phase(Phase::Installer { log }) => Some(Msg::InstallerRunning(log.clone())),
        ProgressEvent::Phase(
            p @ (Phase::Preparing
            | Phase::WaitingInstaller
            | Phase::Extracting { .. }
            | Phase::Placing
            | Phase::Linking
            | Phase::Recording
            | Phase::Removing { .. }
            | Phase::Scanning
            | Phase::WaitingElevation
            | Phase::RunningInstaller { .. }
            | Phase::HelperPreparing { .. }
            | Phase::Verifying
            | Phase::CopyingFolders
            | Phase::Uninstalling { .. }
            | Phase::CheckingRemoval { .. }
            | Phase::SelfElevating { .. }
            | Phase::Bridge { .. }
            | Phase::RunningScript { .. }
            | Phase::RunningPackageInstaller { .. }
            | Phase::ReleaseInstaller { .. }
            | Phase::SelfUpdate(_)
            | Phase::WaitingUpdate
            | Phase::FetchingCatalog { .. }
            | Phase::WritingCatalog
            | Phase::ReadingInstalled
            | Phase::Correlating),
        ) => Some(Msg::InstallPhase(p.clone())),
    }
}

/// Waits for a blocking job. A worker panic is handed to the main loop (never swallowed).
pub async fn join_job(handle: tokio::task::JoinHandle<Msg>) -> Msg {
    match handle.await {
        Ok(msg) => msg,
        Err(e) if e.is_panic() => Msg::Panicked(e.into_panic()),
        Err(e) => Msg::Planned(Err(format!("tarefa cancelada: {e}"))),
    }
}

/// Opens the catalog for reading without DDL or a write lock; a missing file reads as empty.
pub fn open_catalog_for_reading(path: &Path) -> Result<Catalog, CatalogError> {
    if path.exists() {
        Catalog::open_read_only(path)
    } else {
        Catalog::open_in_memory()
    }
}

/// Reads every catalog entry; an unreadable catalog is an error, never an empty list.
pub fn read_catalog(path: &Path) -> Result<Vec<PackageEntry>, String> {
    open_catalog_for_reading(path)
        .and_then(|c| c.load_entries())
        .map_err(|e| e.to_string())
}

/// Main-loop side of [`join_job`]: re-raises a worker panic, passes other messages on.
pub fn propagate(msg: Msg) -> Msg {
    match msg {
        Msg::Panicked(payload) => std::panic::resume_unwind(payload),
        other => other,
    }
}

fn plans_text(groups: &[Vec<InstallPlan>]) -> String {
    groups
        .iter()
        .flatten()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Plans each item of a batch in turn (one failure does not stop the others); only
/// `PlanInstall` and `PlanRemove` are plans.
pub fn plan_batch(c: &OpsCtx<'_>, w: &WingetSrc, items: &[Cmd]) -> Vec<Result<Planned, String>> {
    items
        .iter()
        .map(|item| match item {
            Cmd::PlanInstall {
                source_name,
                id,
                keep,
            } => plan_install(c, w, source_name, id, *keep),
            Cmd::PlanRemove { app, purge } => plan_remove(c, app, *purge),
            _ => Err("item de lote inválido".to_string()),
        })
        .collect()
}

/// A winget install planned for the bridge (the CLI and the TUI share this one sequence).
#[derive(Debug, Clone)]
pub struct WingetPlan {
    pub job: BridgeJob,
    /// The manifest's license agreements, for the confirmation and the CLI lines.
    pub agreements: Vec<crate::sources::winget::manifest::Agreement>,
    /// The `--scope machine` came from the Settings default: `run_bridge_install` drops it once
    /// when winget finds no installer for it.
    pub scope_fallback: bool,
}

/// Plans installing winget package `pkg` (its catalog version) through the bridge: reads the
/// manifest by the catalog's hash, takes `explicit` (`--scope`) or else the Settings default
/// from `config_path`, read now; a config that does not load falls back to `loaded` (the
/// settings already loaded), with a warning.
pub async fn plan_winget_install(
    http: &crate::http::Http,
    blobs: &crate::cache::BlobStore,
    cdn_base: &str,
    pkg: &crate::catalog::StoredPackage,
    config_path: &Path,
    loaded: crate::config::Behavior,
    explicit: Option<InstallScope>,
) -> Result<WingetPlan, String> {
    let hash = pkg
        .winget_hash
        .as_deref()
        .ok_or_else(|| "pacote winget sem hash de manifesto".to_string())?;
    let (manifest, _) = crate::sources::winget::details::merged_manifest(
        http,
        blobs,
        cdn_base,
        &pkg.entry.id,
        hash,
        Some(&pkg.entry.version),
    )
    .await
    .map_err(|e| e.to_string())?;
    let from_settings = match explicit {
        Some(_) => None,
        None => {
            let behavior = match crate::config::Config::load(config_path) {
                Ok(cfg) => cfg.behavior,
                Err(e) => {
                    tracing::warn!(error = %e, "config unreadable; using the settings already loaded");
                    loaded
                }
            };
            bridge::default_scope(behavior.default_scope)
        }
    };
    let job = bridge::install_job(
        &pkg.entry.id,
        &pkg.entry.name,
        &pkg.entry.version,
        &manifest,
        explicit.or(from_settings),
        bridge::host_arch(),
    );
    Ok(WingetPlan {
        job,
        agreements: manifest.agreements,
        scope_fallback: from_settings.is_some(),
    })
}

/// Plans the winget portable package `spec` as a wingetrs app (Task 189).
pub fn plan_portable(
    c: &OpsCtx<'_>,
    spec: &crate::winget_install::portable::PortableSpec,
    keep: bool,
) -> Result<Planned, String> {
    let plan =
        crate::winget_install::portable::install_plan(c, spec, keep).map_err(|e| e.to_string())?;
    Ok(one_plan(plan))
}

fn one_plan(plan: InstallPlan) -> Planned {
    let plans = vec![plan];
    Planned::Ready {
        text: plans_text(std::slice::from_ref(&plans)),
        action: Action::Apply(vec![plans]),
    }
}

#[cfg(windows)]
fn winget_host() -> Result<crate::winget_install::select::Host, String> {
    Ok(crate::winget_install::host_win::current())
}

#[cfg(not(windows))]
fn winget_host() -> Result<crate::winget_install::select::Host, String> {
    Err("pacotes do winget só no Windows".to_string())
}

/// Where a planning job reads winget manifests from: the runtime that awaits the read (the
/// job's blocking thread blocks on it) and the CDN base.
#[derive(Debug, Clone)]
pub struct WingetSrc {
    pub handle: tokio::runtime::Handle,
    pub cdn_base: String,
}

impl WingetSrc {
    /// The winget CDN, read through `handle` (the one `spawn_job` hands to the job).
    pub fn production(handle: &tokio::runtime::Handle) -> WingetSrc {
        WingetSrc {
            handle: handle.clone(),
            cdn_base: crate::sources::winget::CDN_BASE.to_string(),
        }
    }
}

/// The upgrade plan of wingetrs app `id`, a portable package of the winget catalog. Runs on a
/// job's blocking thread: the manifest read (cached blob, else the CDN) is awaited here.
fn portable_upgrade(
    c: &OpsCtx<'_>,
    w: &WingetSrc,
    id: &str,
    keep: bool,
) -> Result<InstallPlan, String> {
    let host = winget_host()?;
    w.handle
        .block_on(crate::winget_install::portable::plan_from_catalog(
            c,
            &w.cdn_base,
            &host,
            id,
            keep,
        ))
        .map_err(|e| e.to_string())
}

/// Plans installing (or upgrading) `id` from the scoop bucket in `source_name`; `keep` keeps
/// the old version on an upgrade. `winget` is the upgrade of a portable package installed as a
/// wingetrs app.
pub fn plan_install(
    c: &OpsCtx<'_>,
    w: &WingetSrc,
    source_name: &str,
    id: &str,
    keep: bool,
) -> Result<Planned, String> {
    if source_name == "winget" {
        let rec = StateDb::open(&c.data.install_layout().state_db())
            .and_then(|s| s.get(id))
            .map_err(|e| e.to_string())?;
        return match rec {
            Some((r, _)) if r.bucket == crate::winget_install::portable::WINGET_BUCKET => {
                portable_upgrade(c, w, id, keep).map(one_plan)
            }
            _ => Err(format!(
                "{id} não é um pacote do winget instalado pelo wingetrs"
            )),
        };
    }
    let bucket = source_name
        .strip_prefix("scoop:")
        .ok_or_else(|| format!("{id} não vem de um bucket do scoop"))?;
    match ops::resolve_target(c, id, Some(bucket)) {
        Ok(Resolution::One(cand)) => {
            let plans = ops::plan_for(c, &cand, keep).map_err(|e| e.to_string())?;
            Ok(Planned::Ready {
                text: plans_text(std::slice::from_ref(&plans)),
                action: Action::Apply(vec![plans]),
            })
        }
        Ok(Resolution::Many(found)) => Ok(Planned::Choose(found)),
        Ok(Resolution::NotFound { .. }) => Err(format!("{id} não encontrado no catálogo")),
        Err(e) => Err(e.to_string()),
    }
}

/// Plans removing `app`; `purge` also deletes its persistent data.
pub fn plan_remove(c: &OpsCtx<'_>, app: &str, purge: bool) -> Result<Planned, String> {
    let plan = ops::plan_uninstall(c, app, purge).map_err(|e| e.to_string())?;
    let mut text = format!("Remover {} {}\n", plan.app, plan.version);
    for l in plan.artifact_lines() {
        text.push_str(&format!("  - {l}\n"));
    }
    text.push_str(if plan.purge {
        "Os dados persistentes do app serão APAGADOS (purge).\n"
    } else {
        "Os dados persistentes do app são mantidos.\n"
    });
    // The removal runs the code of the manifest written at install time: it goes to the user.
    let (manifest, manifest_sha256) =
        ops::uninstall_manifest(c, &plan).map_err(|e| e.to_string())?;
    let layout = c.data.install_layout();
    let vdir = layout.version_dir(&plan.app, &plan.version);
    let roots = CallRoots::new(layout.root(), Some(&vdir));
    let code = manifest
        .map(|m| removal_third_party(&m, &vdir, "", &roots, &*c.runners.ast))
        .unwrap_or_default();
    let action = Action::Remove(plan);
    if code.is_empty() {
        return Ok(Planned::Ready { text, action });
    }
    Ok(Planned::ReadyScripts {
        text,
        action,
        code,
        manifest_sha256,
    })
}

/// Plans upgrading every upgradable app; `keep` keeps the old versions.
pub fn plan_upgrade_all(c: &OpsCtx<'_>, w: &WingetSrc, keep: bool) -> Result<Planned, String> {
    let (list, _pinned) = ops::upgradable(c)
        .map_err(|e| e.to_string())?
        .without_pinned(&crate::pins::load(c.data));
    if list.upgradable.is_empty() {
        return Err("nenhum app para atualizar".to_string());
    }
    let layout = c.data.install_layout();
    let buckets: Vec<(String, String)> = StateDb::open(&layout.state_db())
        .and_then(|s| s.apps())
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|r| (r.app, r.bucket))
        .collect();
    let mut groups = Vec::new();
    for (app, _, _) in &list.upgradable {
        let bucket = buckets
            .iter()
            .find(|(a, _)| a == app)
            .map(|(_, b)| b.as_str())
            .ok_or_else(|| format!("{app}: não está no registro do wingetrs"))?;
        if bucket == crate::winget_install::portable::WINGET_BUCKET {
            groups.push(vec![portable_upgrade(c, w, app, keep)?]);
            continue;
        }
        match ops::resolve_installed(c, app, bucket).map_err(|e| e.to_string())? {
            Resolution::One(cand) => {
                groups.push(ops::plan_for(c, &cand, keep).map_err(|e| e.to_string())?);
            }
            _ => return Err(format!("{app}: não foi possível resolver no catálogo")),
        }
    }
    Ok(Planned::Ready {
        text: plans_text(&groups),
        action: Action::Apply(groups),
    })
}

fn fail(e: OpsError) -> InstallFail {
    let needs = |e: &OpsError| match e {
        OpsError::Exec(ExecError::NeedsMsiexec { app, reason, .. }) => {
            Some((app.clone(), reason.clone()))
        }
        _ => None,
    };
    match e {
        OpsError::Busy => InstallFail::Busy,
        OpsError::PartlyApplied { applied, error } => match needs(&error) {
            Some((app, reason)) => InstallFail::NeedsMsiexec {
                app,
                reason,
                applied,
            },
            None => InstallFail::Other(OpsError::PartlyApplied { applied, error }.to_string()),
        },
        other => match needs(&other) {
            Some((app, reason)) => InstallFail::NeedsMsiexec {
                app,
                reason,
                applied: Vec::new(),
            },
            None => InstallFail::Other(other.to_string()),
        },
    }
}

/// Removes `plan`; `scripts` = `Some(sha)` is the user's consent to the uninstall code of the
/// manifest of hash `sha` they read (checked under `install.lock`). The notes of what did not
/// happen (a skipped uninstaller) go in the summary the user reads.
fn removed(
    o: &OpsCtx<'_>,
    plan: UninstallPlan,
    scripts: Option<&str>,
) -> Result<Done, InstallFail> {
    let done = ops::uninstall_with(o, &plan, scripts).map_err(fail)?;
    Ok(Done {
        summary: removed_summary(&plan.app, &done.notes),
        residues: Some((plan.app, done.residues)),
    })
}

/// `<app> removido`, followed by the notes of the removal.
pub fn removed_summary(app: &str, notes: &[String]) -> String {
    let mut summary = format!("{app} removido");
    for note in notes {
        summary.push_str(". ");
        summary.push_str(note);
    }
    summary
}

fn installed(applied: Result<Vec<InstallPlan>, OpsError>) -> Result<Done, InstallFail> {
    let names: Vec<String> = applied.map_err(fail)?.into_iter().map(|p| p.app).collect();
    Ok(Done {
        summary: format!("Instalado: {}", names.join(", ")),
        residues: None,
    })
}

/// The end of a wingetrs self-update as the queue shows it: the toast text, or the typed
/// error's text.
pub fn self_update_done(
    new: Option<&str>,
    result: Result<crate::selfupdate::Outcome, crate::selfupdate::SelfUpdateError>,
) -> Result<Done, InstallFail> {
    match result {
        Ok(crate::selfupdate::Outcome::Updated) => Ok(Done {
            summary: format!(
                "wingetrs atualizado para {}; feche e abra de novo para usar a versão nova",
                new.unwrap_or("a versão nova")
            ),
            residues: None,
        }),
        Ok(crate::selfupdate::Outcome::AlreadyLatest) => Ok(Done {
            summary: "o wingetrs já está na versão mais recente".to_string(),
            residues: None,
        }),
        Err(e) => Err(InstallFail::Other(e.to_string())),
    }
}

fn bucket_fail(e: crate::buckets::BucketError) -> InstallFail {
    match e {
        crate::buckets::BucketError::Busy => InstallFail::Busy,
        e => InstallFail::Other(e.to_string()),
    }
}

/// `wingetrs bucket add` for the TUI queue: reads `config.toml` again (the dialog's copy may be
/// old), applies the same rule (`buckets::add`, a bucket wingetrs does not know only with
/// `confirmed`) and downloads it under the update lock, which saves the config. A held lock is
/// `InstallFail::Busy` (the queue tries again). `url` overrides the tarball URL (tests).
pub async fn bucket_add(
    data: &crate::cache::DataDir,
    http: &crate::http::Http,
    name: &str,
    spec: &str,
    confirmed: bool,
    url: Option<&str>,
    progress: crate::install::download::Progress<'_>,
) -> Result<Done, InstallFail> {
    let mut cfg = crate::config::Config::load(&data.config_path())
        .map_err(|e| InstallFail::Other(e.to_string()))?;
    crate::buckets::add(&mut cfg, name, spec, confirmed).map_err(bucket_fail)?;
    let apps = crate::buckets::add_downloaded_progress(data, http, &cfg, name, url, progress)
        .await
        .map_err(bucket_fail)?;
    Ok(Done {
        summary: format!("Bucket adicionado: {name} ({apps} apps no catálogo)"),
        residues: None,
    })
}

/// `wingetrs bucket rm` for the TUI queue: refused while apps from the bucket are installed;
/// `confirmed` is needed for `main` and `extras`.
pub fn bucket_remove(
    data: &crate::cache::DataDir,
    name: &str,
    confirmed: bool,
) -> Result<Done, InstallFail> {
    let mut cfg = crate::config::Config::load(&data.config_path())
        .map_err(|e| InstallFail::Other(e.to_string()))?;
    let state_db = data.install_layout().state_db();
    crate::buckets::remove_and_cleanup_confirmed(data, &mut cfg, name, &state_db, confirmed)
        .map_err(bucket_fail)?;
    Ok(Done {
        summary: format!("Bucket removido: {name}"),
        residues: None,
    })
}

/// The buckets dialog's data: `config.toml`, each bucket with its catalog numbers and the apps
/// installed from it.
pub fn load_buckets(
    data: &crate::cache::DataDir,
    now: i64,
) -> Result<(crate::config::Config, Vec<crate::tui::app::BucketInfo>), String> {
    let cfg = crate::config::Config::load(&data.config_path()).map_err(|e| e.to_string())?;
    let state_db = data.install_layout().state_db();
    let rows = crate::buckets::rows(data, &cfg).map_err(|e| e.to_string())?;
    let infos = rows
        .into_iter()
        .map(|row| crate::tui::app::BucketInfo {
            age: row.updated.map(|t| (now - t).max(0)),
            installed: crate::buckets::installed_from_bucket(&state_db, &row.name)
                .map_err(|e| e.to_string()),
            row,
        })
        .collect();
    Ok((cfg, infos))
}

/// The `job.start` data of an action run by the own installer; `None` for a bridge job
/// (`run_bridge` logs it).
fn action_info(action: &Action) -> Option<JobInfo> {
    match action {
        Action::Apply(groups)
        | Action::ApplyMsiexec { groups, .. }
        | Action::ApplyScripts { groups } => {
            let plans = groups.iter().flatten();
            let kind = JobInfo::plans_kind(plans.clone());
            Some(JobInfo::for_plans(kind, plans))
        }
        Action::Remove(plan) | Action::RemoveScripts { plan, .. } => Some(JobInfo {
            kind: "remove",
            tool: "wingetrs",
            target: plan.app.clone(),
            name: plan.app.clone(),
            from: Some(plan.version.clone()),
            to: None,
            cmd: None,
        }),
        Action::Bridge(_)
        | Action::SelfUpdate { .. }
        | Action::BucketAdd { .. }
        | Action::BucketRemove { .. } => None,
        Action::WingetInstall(p) => Some(winget_info("install", p, None)),
        Action::WingetUpgrade { plan, .. } => {
            Some(winget_info("upgrade", plan, plan.upgrade_from.clone()))
        }
        Action::WingetUninstall(p) | Action::WingetRegistryOnly(p) => Some(JobInfo {
            kind: "remove",
            tool: "wingetrs",
            target: record_of(&p.target),
            name: p.name.clone(),
            from: p.version.clone(),
            to: None,
            cmd: None,
        }),
        Action::RemoveOrphan(o) => Some(JobInfo {
            kind: "remove",
            tool: "wingetrs",
            target: o.key.clone(),
            name: o.name.clone(),
            from: None,
            to: None,
            cmd: None,
        }),
    }
}

fn winget_info(kind: &'static str, p: &NativePlan, from: Option<String>) -> JobInfo {
    JobInfo {
        kind,
        tool: "wingetrs",
        target: p.id.clone(),
        name: p.name.clone(),
        from,
        to: Some(p.version.clone()),
        cmd: None,
    }
}

/// The registry key or package family a native removal acts on.
fn record_of(t: &UninstallTarget) -> String {
    match t {
        UninstallTarget::Arp { key, .. } => key.clone(),
        UninstallTarget::Msix { family } => family.clone(),
    }
}

/// Outcome and text of a failed job for `job.end`.
fn fail_log(fail: &InstallFail) -> (Outcome, String) {
    match fail {
        InstallFail::Busy => (
            Outcome::Busy,
            "outro wingetrs está com o install.lock".to_string(),
        ),
        InstallFail::NeedsMsiexec { app, reason, .. } => (
            Outcome::NeedsConfirmation,
            format!("{app}: precisa de confirmação para o msiexec /a ({reason})"),
        ),
        InstallFail::Other(m) => (Outcome::Failed, m.clone()),
    }
}

/// Logs `job.start` and `job.end` of an action whose context could not even be built.
pub fn log_not_started(action: &Action, error: &str) {
    if let Some(info) = action_info(action) {
        JobLog::start(info).finish(Outcome::Failed, error);
    }
}

/// Executes a confirmed action (the lock policy is the context's `wait`).
pub async fn run_action(o: &OpsCtx<'_>, action: Action) -> Result<Done, InstallFail> {
    let log = action_info(&action).map(JobLog::start);
    let result = run_action_inner(o, action).await;
    if let Some(log) = log {
        match &result {
            Ok(done) => log.finish(Outcome::Ok, &done.summary),
            Err(fail) => {
                let (outcome, detail) = fail_log(fail);
                log.finish(outcome, &detail);
            }
        }
    }
    result
}

async fn run_action_inner(o: &OpsCtx<'_>, action: Action) -> Result<Done, InstallFail> {
    match action {
        Action::Apply(groups) => installed(ops::apply_groups(o, groups).await),
        Action::ApplyMsiexec {
            groups,
            app,
            scripts,
        } => {
            let allow = Allow {
                msiexec: AllowMsiexec::App(app),
                scripts,
            };
            installed(ops::apply_groups_with(o, groups, &allow).await)
        }
        Action::ApplyScripts { groups } => {
            let allow = Allow {
                msiexec: AllowMsiexec::No,
                scripts: true,
            };
            installed(ops::apply_groups_with(o, groups, &allow).await)
        }
        Action::Remove(plan) => removed(o, plan, None),
        Action::RemoveScripts {
            plan,
            manifest_sha256,
        } => removed(o, plan, Some(&manifest_sha256)),
        // Runs outside `install.lock` and the ops context: see `run_bridge`.
        Action::Bridge(job) => Err(InstallFail::Other(format!(
            "{}: atualização via {} fora do executor da ponte",
            job.name,
            job.tool.name()
        ))),
        // Run by `run_winget_action`, without the scoop ops context.
        other => Err(InstallFail::Other(format!(
            "{}: ação nativa do winget fora do seu executor",
            other.label()
        ))),
    }
}

/// Whether `action` is a native winget job (`run_winget_action`), not one of the own scoop
/// installer nor of the bridge.
pub fn is_winget_action(action: &Action) -> bool {
    matches!(
        action,
        Action::WingetInstall(_)
            | Action::WingetUninstall(_)
            | Action::WingetRegistryOnly(_)
            | Action::WingetUpgrade { .. }
            | Action::RemoveOrphan(_)
    )
}

/// What a native winget job needs from the outside world (production in `run.rs`, fakes in
/// tests). `install.elevated` is the session's helper (uninstallers and ARP keys: one UAC per
/// session). Each native installer that runs elevated asks for its own UAC (Task 196), one at a
/// time: the queue runs one job at a time and every command-line installer launches under
/// `msi.lock`, so two UAC prompts never open together, not even from another wingetrs.
pub struct WingetEnv<'a> {
    pub install: WingetCtx<'a>,
    pub uninstall: UninstallCtx<'a>,
    pub registry: &'a dyn ArpRegistry,
    /// `install::layout::Layout::backups` (the `.reg` exports and the folder copies).
    pub backups: &'a Path,
    pub free_space: &'a dyn Fn(&Path) -> Option<u64>,
    /// The plan removing the installed record `row_key` before an `uninstallPrevious` upgrade.
    pub previous: &'a dyn Fn(&str, &NativePlan) -> Result<WingetUninstallPlan, String>,
    /// Name of this run's backup folder (a timestamp).
    pub stamp: String,
}

fn winget_fail(e: WingetOpsError) -> InstallFail {
    match e {
        WingetOpsError::Busy => InstallFail::Busy,
        other => InstallFail::Other(other.to_string()),
    }
}

fn upgrade_fail(e: UpgradeError) -> InstallFail {
    match e {
        UpgradeError::Install(WingetOpsError::Busy) => InstallFail::Busy,
        other => InstallFail::Other(other.to_string()),
    }
}

/// Takes the launch gate (a quit confirmed before this cancels the removal: nothing runs) and
/// announces the removal's phase, after which it is not stopped.
fn start_removal(env: &WingetEnv<'_>, p: Phase) -> Result<(), InstallFail> {
    if !env.install.gate.try_launch() {
        return Err(InstallFail::Other(WingetOpsError::Cancelled.to_string()));
    }
    phase_of(env, p);
    Ok(())
}

fn phase_of(env: &WingetEnv<'_>, p: Phase) {
    (env.install.progress)(&ProgressEvent::Phase(p));
}

/// The outcome of a native install or upgrade: a warning (the app did not show up) is an
/// error without effect, never "Instalado".
fn install_done(
    verb: &str,
    name: &str,
    report: crate::winget_install::ops::InstallReport,
) -> (Result<Done, InstallFail>, Outcome) {
    match report.warning {
        Some(w) => (
            Err(InstallFail::Other(format!("{name}: {w}"))),
            Outcome::NoEffect,
        ),
        None => (
            Ok(Done {
                summary: format!("{verb}: {name}"),
                residues: None,
            }),
            Outcome::Ok,
        ),
    }
}

/// Whether a native winget action runs anything as administrator (installer UAC or helper).
fn winget_action_elevated(action: &Action, previous: Option<&Option<WingetUninstallPlan>>) -> bool {
    match action {
        Action::WingetInstall(p) => plan_runs_elevated(p),
        Action::WingetUpgrade { plan, .. } => {
            upgrade_runs_elevated(plan, previous.and_then(|p| p.as_ref()))
        }
        Action::WingetUninstall(p) => removal_runs_elevated(p, false),
        Action::WingetRegistryOnly(p) => removal_runs_elevated(p, true),
        _ => false,
    }
}

/// The installed version an upgrade of this action removes first (`UninstallFirst` route).
fn upgrade_previous(
    env: &WingetEnv<'_>,
    action: &Action,
) -> Result<Option<WingetUninstallPlan>, String> {
    match action {
        Action::WingetUpgrade { plan, row_key }
            if upgrade_route(&plan.installer) == UpgradeRoute::UninstallFirst =>
        {
            (env.previous)(row_key, plan).map(Some)
        }
        _ => Ok(None),
    }
}

/// Executes a confirmed native winget action and logs it as a job.
pub async fn run_winget_action(env: &WingetEnv<'_>, action: Action) -> Result<Done, InstallFail> {
    let mut log = action_info(&action).map(JobLog::start);
    // The version an upgrade removes first decides `elevated` too.
    let previous = upgrade_previous(env, &action);
    if let Some(log) = &mut log {
        log.elevated = winget_action_elevated(&action, previous.as_ref().ok());
    }
    let (result, outcome) = match previous {
        Ok(previous) => run_winget_inner(env, action, previous).await,
        Err(e) => (Err(InstallFail::Other(e)), Outcome::Ok),
    };
    if let Some(log) = log {
        match &result {
            Ok(done) => log.finish(outcome, &done.summary),
            Err(fail) => {
                let (failed, detail) = fail_log(fail);
                let outcome = if outcome == Outcome::Ok {
                    failed
                } else {
                    outcome
                };
                log.finish(outcome, &detail);
            }
        }
    }
    result
}

async fn run_winget_inner(
    env: &WingetEnv<'_>,
    action: Action,
    previous: Option<WingetUninstallPlan>,
) -> (Result<Done, InstallFail>, Outcome) {
    let failed = |f: InstallFail| (Err(f), Outcome::Ok);
    match action {
        Action::WingetInstall(plan) => match winget_ops::install(&env.install, &plan).await {
            Ok(r) => install_done("Instalado", &plan.name, r),
            Err(e) => failed(winget_fail(e)),
        },
        Action::WingetUpgrade { plan, .. } => {
            match engine::upgrade(&env.install, &env.uninstall, &plan, previous.as_ref()).await {
                Ok(r) => install_done("Atualizado", &plan.name, r),
                Err(e) => failed(upgrade_fail(e)),
            }
        }
        Action::WingetUninstall(plan) => {
            // The copy is kept only when the plan still has folders (the user left it checked).
            let how = if plan.shared.as_ref().is_some_and(|s| !s.folders.is_empty()) {
                RemovalChoice::BackupThenRemove
            } else {
                RemovalChoice::Remove
            };
            winget_removal(env, &plan, how).await
        }
        Action::WingetRegistryOnly(plan) => {
            winget_removal(env, &plan, RemovalChoice::RegistryOnly).await
        }
        Action::RemoveOrphan(o) => {
            // A live line from the first instant: the `install.lock` wait is silent.
            phase_of(env, Phase::Preparing);
            let layout = env.install.data.install_layout();
            let _lock = match winget_ops::lock_install(&layout, env.install.wait).await {
                Ok(l) => l,
                Err(e) => return failed(winget_fail(e)),
            };
            // `Removendo`; an HKLM key: the helper's UAC notice while its prompt is open.
            if let Err(f) = start_removal(env, removal::removal_phase(false, &o.name)) {
                return failed(f);
            }
            match remove_orphan_with(
                env.registry,
                env.install.elevated,
                env.backups,
                &env.stamp,
                &o,
                env.install.progress,
            ) {
                Ok(file) => (
                    Ok(Done {
                        summary: format!(
                            "Registro órfão de {} removido (cópia em {})",
                            o.name,
                            file.display()
                        ),
                        residues: None,
                    }),
                    Outcome::Ok,
                ),
                Err(e) => failed(InstallFail::Other(format!("{}: {e}", o.name))),
            }
        }
        other => failed(InstallFail::Other(format!(
            "{}: não é uma ação nativa do winget",
            other.label()
        ))),
    }
}

/// A native removal through `removal::remove` (the same sequence as the CLI's).
async fn winget_removal(
    env: &WingetEnv<'_>,
    plan: &WingetUninstallPlan,
    how: RemovalChoice,
) -> (Result<Done, InstallFail>, Outcome) {
    let failed = |f: InstallFail| (Err(f), Outcome::Ok);
    let layout = env.install.data.install_layout();
    let steps = Steps {
        gate: &env.install.gate,
        progress: env.install.progress,
        backups: env.backups,
        stamp: &env.stamp,
        free_space: env.free_space,
    };
    let queue = MsiQueue::of(&env.install);
    let removed = removal::remove(
        &steps,
        winget_ops::lock_install(&layout, env.install.wait),
        plan,
        how,
        |p| removal::run_uninstaller(&env.uninstall, &queue, plan, p),
        |p| {
            remove_registry_only_with(
                plan,
                env.registry,
                env.install.elevated,
                env.backups,
                &env.stamp,
                p,
            )
        },
    )
    .await;
    match removed {
        Ok(Removed::App { report, copied })
            if report.status == UninstallStatus::Removed && report.warning.is_none() =>
        {
            let mut summary = format!("{} removido", plan.name);
            if let Some(dest) = copied {
                summary.push_str(&format!(" (pastas copiadas para {})", dest.display()));
            }
            (
                Ok(Done {
                    summary,
                    residues: None,
                }),
                Outcome::Ok,
            )
        }
        Ok(Removed::App { report, .. }) => (
            Err(InstallFail::Other(format!(
                "{}: {}",
                plan.name,
                report.warning.unwrap_or_else(|| KEY_STAYED.to_string())
            ))),
            report.status.job_outcome(),
        ),
        Ok(Removed::Record(file)) => (
            Ok(Done {
                summary: format!(
                    "Registro de {} removido (cópia em {})",
                    plan.name,
                    file.display()
                ),
                residues: None,
            }),
            Outcome::Ok,
        ),
        Err(RemovalError::Lock(e)) => failed(winget_fail(e)),
        Err(e) => failed(InstallFail::Other(e.to_string())),
    }
}

/// Progress text of a winget job queued behind another Windows Installer run of wingetrs.
pub const WAITING_INSTALLER: &str = "aguardando outro instalador";
/// Progress text once a queued winget job got `msi.lock`.
pub const STARTING_WINGET: &str = "iniciando o winget";

pub use crate::bridge::BridgeExec;

/// Upgrades an app through winget or scoop. The target is checked before anything runs; for
/// scoop, the installed version is read right before and right after the run, since its exit
/// code 0 alone does not mean it upgraded. For winget, exit 0 counts only when the version it
/// picked is the one wingetrs offered (its own catalog may be older). Start and end of the job
/// go to the log (`job.start`, `job.end`), with the last lines of output when it did not work.
/// `scoop_root` is `None` when it could not be found: a scoop job then fails before running.
/// A winget job holds `msi_lock` while winget runs (its MSI installers share the Windows
/// Installer mutex with ours); scoop never takes it.
pub fn run_bridge(
    job: &BridgeJob,
    scoop_root: Option<&Path>,
    msi_lock: &Path,
    pwsh: &str,
    exec: BridgeExec<'_>,
    on_line: &mut dyn FnMut(&str),
) -> Result<Done, InstallFail> {
    let env = BridgeEnv {
        scoop_root,
        msi_lock,
        pwsh,
    };
    run_bridge_as(job, &env, exec, false, on_line).0
}

/// Where a bridge job runs.
struct BridgeEnv<'a> {
    scoop_root: Option<&'a Path>,
    msi_lock: &'a Path,
    pwsh: &'a str,
}

/// `run_bridge` with the exec `bridge` picks: its elevated helper for a winget app installed
/// for the whole machine, `current` otherwise (see `ElevatedBridge::exec_for`).
pub fn run_bridge_elevated(
    bridge: &ElevatedBridge,
    job: &BridgeJob,
    scoop_root: Option<&Path>,
    msi_lock: &Path,
    pwsh: &str,
    current: BridgeExec<'_>,
    on_line: &mut dyn FnMut(&str),
) -> Result<Done, InstallFail> {
    let env = BridgeEnv {
        scoop_root,
        msi_lock,
        pwsh,
    };
    run_elevated_as(bridge, job, &env, current, on_line).0
}

/// Installs a winget package through the bridge (`job.op` is `Install`), elevated when
/// `job.scope` is `Some(Machine)` (see `run_bridge_elevated`). The agreements of `job.op` were
/// accepted and go to `job.start`. `scope_fallback`: the `--scope machine` came from the
/// Settings default, so when winget finds no installer for it (`NO_APPLICABLE_INSTALLER`) the
/// install runs once more without `--scope`, unelevated, saying so on `on_line`.
pub fn run_bridge_install(
    bridge: &ElevatedBridge,
    job: &BridgeJob,
    scope_fallback: bool,
    msi_lock: &Path,
    current: BridgeExec<'_>,
    on_line: &mut dyn FnMut(&str),
) -> Result<Done, InstallFail> {
    let env = BridgeEnv {
        scoop_root: None,
        msi_lock,
        pwsh: "",
    };
    let (result, code) = run_elevated_as(bridge, job, &env, &mut *current, on_line);
    if result.is_err()
        && code.is_some_and(|c| c as u32 == bridge::WINGET_SCOPE_CRASH)
        && let BridgeOp::Install {
            scope: Some(_),
            agreements,
        } = &job.op
    {
        tracing::info!(target = %job.target, "{}", bridge::SCOPE_CRASH_NOTE);
        on_line(bridge::SCOPE_CRASH_NOTE);
        // Same path as the first run: `job.scope` still says whether it is elevated.
        let retry = BridgeJob {
            op: BridgeOp::Install {
                scope: None,
                agreements: agreements.clone(),
            },
            ..job.clone()
        };
        return run_elevated_as(bridge, &retry, &env, current, on_line).0;
    }
    let no_installer = code.is_some_and(|c| c as u32 == bridge::NO_APPLICABLE_INSTALLER);
    let from_settings = matches!(
        job.op,
        BridgeOp::Install {
            scope: Some(InstallScope::Machine),
            ..
        }
    ) && scope_fallback;
    if result.is_ok() || !no_installer || !from_settings {
        return result;
    }
    tracing::info!(target = %job.target, "{}", bridge::SCOPE_FALLBACK_NOTE);
    on_line(bridge::SCOPE_FALLBACK_NOTE);
    let agreements = match &job.op {
        BridgeOp::Install { agreements, .. } => agreements.clone(),
        BridgeOp::Upgrade => Vec::new(),
    };
    // Never elevated: winget found no machine-wide installer, so the user one runs as the user.
    let retry = BridgeJob {
        op: BridgeOp::Install {
            scope: None,
            agreements,
        },
        scope: None,
        ..job.clone()
    };
    run_elevated_as(bridge, &retry, &env, current, on_line).0
}

/// `run_bridge_as` with the exec `bridge` picks for `job`.
fn run_elevated_as(
    bridge: &ElevatedBridge,
    job: &BridgeJob,
    env: &BridgeEnv<'_>,
    current: BridgeExec<'_>,
    on_line: &mut dyn FnMut(&str),
) -> (Result<Done, InstallFail>, Option<i32>) {
    let mut chosen = bridge.exec_for(job, current, on_line);
    let elevated = chosen.elevated();
    run_bridge_as(job, env, chosen.exec(), elevated, on_line)
}

/// Runs `job` once with its own `job.start`/`job.end`; also returns the tool's last exit code.
fn run_bridge_as(
    job: &BridgeJob,
    env: &BridgeEnv<'_>,
    exec: BridgeExec<'_>,
    elevated: bool,
    on_line: &mut dyn FnMut(&str),
) -> (Result<Done, InstallFail>, Option<i32>) {
    let BridgeEnv {
        scoop_root,
        msi_lock,
        pwsh,
    } = *env;
    let root = scoop_root.unwrap_or(Path::new(""));
    let (program, args) = bridge::command(job, root, pwsh);
    let info = JobInfo {
        kind: match job.op {
            BridgeOp::Upgrade => "bridge",
            BridgeOp::Install { .. } => "install",
        },
        tool: job.tool.name(),
        target: job.target.clone(),
        name: job.name.clone(),
        from: job.installed.clone(),
        to: job.available.clone(),
        cmd: Some(bridge::command_text(&program, &args)),
    };
    let mut log = match &job.op {
        BridgeOp::Upgrade => JobLog::start(info),
        BridgeOp::Install { agreements, .. } => JobLog::start_accepting(info, agreements),
    };
    log.elevated = elevated;
    let result = bridge_run(
        job,
        scoop_root,
        msi_lock,
        (&program, &args),
        exec,
        on_line,
        &mut log,
    );
    let code = log.code;
    match &result {
        Ok(done) => log.finish(Outcome::Ok, &done.summary),
        Err(fail) => {
            let (outcome, detail) = fail_log(fail);
            let outcome = if outcome == Outcome::Failed {
                log.failure
            } else {
                outcome
            };
            log.finish(outcome, &detail);
        }
    }
    (result, code)
}

fn bridge_run(
    job: &BridgeJob,
    scoop_root: Option<&Path>,
    msi_lock: &Path,
    (program, args): (&str, &[String]),
    exec: BridgeExec<'_>,
    on_line: &mut dyn FnMut(&str),
    log: &mut JobLog,
) -> Result<Done, InstallFail> {
    let fail = |message: String| Err(InstallFail::Other(format!("{}: {message}", job.name)));
    if !bridge::valid_target(job.tool, &job.target) {
        return fail(format!("identificador inválido: {}", job.target));
    }
    let install = matches!(job.op, BridgeOp::Install { .. });
    if install && job.tool != Tool::Winget {
        return fail(format!("instalação via {} pela ponte", job.tool.name()));
    }
    let scoop_root = match (job.tool, scoop_root) {
        (Tool::Scoop, None) => return fail("scoop não encontrado".to_string()),
        (_, root) => root.unwrap_or(Path::new("")),
    };
    let version = || match job.tool {
        Tool::Scoop => bridge::scoop_version(scoop_root, &job.target),
        Tool::Winget => None,
    };
    let _msi = match job.tool {
        Tool::Winget => {
            let mut waited = false;
            let mut status = |_: msi_queue::Status| {
                waited = true;
                on_line(WAITING_INSTALLER);
            };
            let lock = match msi_queue::lock_msi(msi_lock, &mut status) {
                Ok(lock) => lock,
                Err(e) => return fail(e.to_string()),
            };
            if waited {
                on_line(STARTING_WINGET);
            }
            Some(lock)
        }
        Tool::Scoop => None,
    };
    let before = version();
    let mut retried = false;
    loop {
        // Nothing of the previous run may leak into the log of this one.
        (log.failure, log.code, log.after) = (Outcome::Failed, None, None);
        (log.lines, log.installer_log) = (Vec::new(), None);
        let (code, lines, seen_chosen) = match exec_once(program, args, exec, on_line, job) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return fail(format!("{} não encontrado", job.tool.name()));
            }
            Err(e) => return fail(format!("falha ao iniciar {program}: {e}")),
        };
        let after = version();
        log.code = Some(code);
        log.after = after.clone();
        log.installer_log = match job.tool {
            Tool::Winget => bridge::winget_installer_log(&lines),
            Tool::Scoop => None,
        };
        let (mut updated, mut message) = match job.tool {
            Tool::Winget if bridge::reboot_requested(code, &lines, &bridge::REBOOT_MARKERS) => {
                let message = if install {
                    "instalado via winget; reinicie o computador para concluir"
                } else {
                    "Atualizado; reinicie o computador para concluir."
                };
                tracing::info!(target = %job.target, "{message}");
                (true, message.to_string())
            }
            Tool::Winget if install && bridge::winget_updated(code) => {
                (true, "instalado via winget".to_string())
            }
            Tool::Winget => (bridge::winget_updated(code), bridge::winget_message(code)),
            Tool::Scoop => bridge::scoop_message(code, before.as_deref(), after.as_deref(), &lines),
        };
        let mut stale = None;
        if updated && job.tool == Tool::Winget {
            let chosen = seen_chosen.or_else(|| bridge::winget_chosen_version(&job.target, &lines));
            if let (Some(chosen), Some(available)) = (&chosen, &job.available)
                && Version::parse(chosen) < Version::parse(available)
            {
                updated = false;
                message = format!(
                    "O winget instalou {chosen}, não {available}: o catálogo local do winget está mais velho que o do wingetrs. Tente de novo mais tarde."
                );
                log.failure = Outcome::NoEffect;
                stale = Some((chosen.clone(), available.clone()));
            }
            log.after = chosen;
        }
        log.lines = lines;
        if let (Some((chosen, available)), false) = (stale, retried) {
            retried = true;
            log.attempts = 2;
            tracing::info!(
                target: "wingetrs::job",
                target = %job.target, chosen = %chosen, available = %available,
                reason = "fonte do winget atrasada",
                "job.retry"
            );
            on_line(crate::bridge::winget_out::UPDATING_SOURCE);
            let (src_program, src_args) = bridge::winget_source_update();
            match exec(&src_program, &src_args, &mut |_: &str| {}) {
                Ok((0, _)) => continue,
                Ok((code, _)) => {
                    message.push_str(&format!(
                        " (a atualização da fonte do winget falhou: código {code})"
                    ));
                }
                Err(e) => {
                    message.push_str(&format!(" (a atualização da fonte do winget falhou: {e})"));
                }
            }
        }
        if updated {
            return Ok(Done {
                summary: format!("{}: {message}", job.name),
                residues: None,
            });
        }
        return fail(message);
    }
}

/// One run of the bridge command: exit code, last lines and the first version winget named.
fn exec_once(
    program: &str,
    args: &[String],
    exec: BridgeExec<'_>,
    on_line: &mut dyn FnMut(&str),
    job: &BridgeJob,
) -> std::io::Result<(i32, Vec<String>, Option<String>)> {
    // The line naming the version winget picked may be older than the tail the exec keeps.
    let mut seen_chosen: Option<String> = None;
    let (code, lines) = exec(program, args, &mut |line: &str| {
        if job.tool == Tool::Winget && seen_chosen.is_none() {
            seen_chosen = bridge::winget_chosen_version(&job.target, &[line.to_string()]);
        }
        on_line(line);
    })?;
    Ok((code, lines, seen_chosen))
}

/// Removes the chosen residues; returns one message per item that failed.
pub fn clean(o: &OpsCtx<'_>, app: &str, residues: &[Residue]) -> Vec<String> {
    residue::clean(
        residues,
        app,
        &o.effects,
        &o.data.install_layout().backups(),
    )
    .into_iter()
    .filter_map(Result::err)
    .collect()
}

/// Writes the Settings tab's values into `config.toml`, keeping everything else in the file.
pub fn save_settings(
    path: &Path,
    behavior: crate::config::Behavior,
    glyphs: Option<&str>,
    icons: Option<&str>,
    list_icons: Option<&str>,
    theme: &str,
) -> Result<(), String> {
    crate::config::update_file(path, |c| {
        c.behavior = behavior;
        if let Some(g) = glyphs {
            c.glyphs = g.to_string();
        }
        if let Some(i) = icons {
            c.icons = i.to_string();
        }
        if let Some(l) = list_icons {
            c.list_icons = l.to_string();
        }
        c.theme = theme.to_string();
    })
    .map(|_| ())
    .map_err(|e| e.to_string())
}
