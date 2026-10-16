//! Install/remove/upgrade jobs of the TUI. Every function takes the `OpsCtx` as a parameter so
//! tests drive them with fakes; `run.rs` builds the production context.
use crate::bridge::{self, BridgeJob, Tool};
use crate::catalog::{Catalog, CatalogError};
use crate::core::PackageEntry;
use crate::install::executor::ExecError;
use crate::install::msi_queue;
use crate::install::ops::{self, AllowMsiexec, OpsCtx, OpsError, Resolution};
use crate::install::plan::InstallPlan;
use crate::install::residue::{self, Residue};
use crate::install::state::StateDb;
use crate::tui::app::{Action, Cmd, Done, InstallFail, Msg, Planned};
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

/// Message for one progress report of a running action: the `msiexec /a` launch always goes
/// through (never throttled), the rest through `throttle`.
pub fn progress_msg(
    throttle: &ProgressThrottle,
    now: Instant,
    label: &str,
    done: u64,
    total: Option<u64>,
) -> Option<Msg> {
    if label == crate::install::executor::INSTALLER_RUNNING {
        return Some(Msg::InstallerRunning);
    }
    throttle
        .admit(now, label, done, total)
        .map(|(label, done, total)| Msg::InstallProgress { label, done, total })
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
pub fn plan_batch(c: &OpsCtx<'_>, items: &[Cmd]) -> Vec<Result<Planned, String>> {
    items
        .iter()
        .map(|item| match item {
            Cmd::PlanInstall {
                source_name,
                id,
                keep,
            } => plan_install(c, source_name, id, *keep),
            Cmd::PlanRemove { app, purge } => plan_remove(c, app, *purge),
            _ => Err("item de lote inválido".to_string()),
        })
        .collect()
}

/// Plans installing (or upgrading) `id` from the scoop bucket in `source_name`; `keep` keeps
/// the old version on an upgrade.
pub fn plan_install(
    c: &OpsCtx<'_>,
    source_name: &str,
    id: &str,
    keep: bool,
) -> Result<Planned, String> {
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
    Ok(Planned::Ready {
        text,
        action: Action::Remove(plan),
    })
}

/// Plans upgrading every upgradable app; `keep` keeps the old versions.
pub fn plan_upgrade_all(c: &OpsCtx<'_>, keep: bool) -> Result<Planned, String> {
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

fn installed(applied: Result<Vec<InstallPlan>, OpsError>) -> Result<Done, InstallFail> {
    let names: Vec<String> = applied.map_err(fail)?.into_iter().map(|p| p.app).collect();
    Ok(Done {
        summary: format!("Instalado: {}", names.join(", ")),
        residues: None,
    })
}

/// Executes a confirmed action (the lock policy is the context's `wait`).
pub async fn run_action(o: &OpsCtx<'_>, action: Action) -> Result<Done, InstallFail> {
    match action {
        Action::Apply(groups) => installed(ops::apply_groups(o, groups).await),
        Action::ApplyMsiexec { groups, app } => {
            installed(ops::apply_groups_allowing(o, groups, &AllowMsiexec::App(app)).await)
        }
        Action::Remove(plan) => {
            let residues = ops::uninstall(o, &plan).map_err(fail)?;
            Ok(Done {
                summary: format!("{} removido", plan.app),
                residues: Some((plan.app, residues)),
            })
        }
        // Runs outside `install.lock` and the ops context: see `run_bridge`.
        Action::Bridge(job) => Err(InstallFail::Other(format!(
            "{}: atualização via {} fora do executor da ponte",
            job.name,
            job.tool.name()
        ))),
    }
}

/// Progress text of a winget job queued behind another Windows Installer run of wingetrs.
pub const WAITING_INSTALLER: &str = "aguardando outro instalador";
/// Progress text once a queued winget job got `msi.lock`.
pub const STARTING_WINGET: &str = "iniciando o winget";

/// Runner of a bridge command: `(program, args, on_line)` -> exit code and last lines.
pub type BridgeExec<'a> =
    &'a mut dyn FnMut(&str, &[String], &mut dyn FnMut(&str)) -> std::io::Result<(i32, Vec<String>)>;

/// Upgrades an app through winget or scoop. The target is checked before anything runs; for
/// scoop, the installed version is read right before and right after the run, since its exit
/// code 0 alone does not mean it upgraded. On failure the last lines go to the log.
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
    let fail = |message: String| Err(InstallFail::Other(format!("{}: {message}", job.name)));
    if !bridge::valid_target(job.tool, &job.target) {
        return fail(format!("identificador inválido: {}", job.target));
    }
    let scoop_root = match (job.tool, scoop_root) {
        (Tool::Scoop, None) => return fail("scoop não encontrado".to_string()),
        (_, root) => root.unwrap_or(Path::new("")),
    };
    let (program, args) = bridge::command(job, scoop_root, pwsh);
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
    let run = exec(&program, &args, on_line);
    let after = version();
    let (code, lines) = match run {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return fail(format!("{} não encontrado", job.tool.name()));
        }
        Err(e) => return fail(format!("falha ao iniciar {program}: {e}")),
    };
    let (updated, message) = match job.tool {
        Tool::Winget if bridge::reboot_requested(code, &lines, &bridge::REBOOT_MARKERS) => {
            let message = "Atualizado; reinicie o computador para concluir.";
            tracing::info!(target = %job.target, "{message}");
            (true, message.to_string())
        }
        Tool::Winget => (bridge::winget_updated(code), bridge::winget_message(code)),
        Tool::Scoop => bridge::scoop_message(code, before.as_deref(), after.as_deref(), &lines),
    };
    if updated {
        return Ok(Done {
            summary: format!("{}: {message}", job.name),
            residues: None,
        });
    }
    tracing::warn!(
        tool = job.tool.name(),
        target = %job.target,
        code,
        "bridge upgrade failed: {message}
    {}",
        lines.join("
    ")
    );
    fail(message)
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
