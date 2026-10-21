//! CLI commands for winget packages (Tasks 176, 186).
//!
//! Platform-neutral: every real effect arrives through [`WingetFlows`] (production:
//! `run_win::ProdFlows`), every prompt through `Ui`, every printed line through a `Say`.
use super::install_cmd::{Cancelled, Flags, Interrupts, Stop, Ui, proceed, uncancellable};
use super::{emit, output};
use crate::bridge::elevated::{ElevatedBridge, SystemTransport, Transport};
use crate::bridge::helper_folder::NoticeTransport;
use crate::bridge::worker::ScopeArg;
use crate::bridge::{self, BridgeExec, BridgeJob};
use crate::cache::BlobStore;
use crate::config::{Behavior, DefaultScope, WingetEngine};
use crate::install::executor::LaunchGate;
use crate::install::ops::OpsCtx;
use crate::install::plan::InstallPlan;
use crate::install::progress::ProgressEvent;
use crate::install::target::Candidate;
use crate::installed::Hive;
use crate::installed::correlate::{InstallScope, ListRow};
use crate::job_log::{JobInfo, JobLog, Outcome};
use crate::sources::winget::manifest::Agreement;
use crate::tui::app::{Done, InstallFail};
use crate::tui::picker::PickerItem;
use crate::winget_install::codes;
use crate::winget_install::engine::{Engine, UpgradeError, choose_engine, upgrade_scope};
use crate::winget_install::ops::{InstallReport, WingetOpsError};
use crate::winget_install::orphans::Orphan;
use crate::winget_install::plan::{
    PlanOrBridge, WingetPlan, bridge_reason, bridge_step, name_version,
};
use crate::winget_install::portable::PortableSpec;
use crate::winget_install::removal::{self, RemovalChoice, RemovalError, Removed, Steps};
use crate::winget_install::select::{Refusal, Scope, ScopeWanted};
use crate::winget_install::uninstall::{
    SharedData, UninstallPlan, UninstallReport, UninstallStatus, UninstallTarget, human_size,
    target_of,
};
use anyhow::{Context, anyhow, bail};
use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// How long the CLI waits at the end for the elevated helper to quit.
const HELPER_EXIT_LIMIT: Duration = Duration::from_secs(5);

/// Where the commands print one line (stdout in production, a buffer in tests).
pub type Say<'a> = &'a mut dyn FnMut(&str);

/// A native winget operation, built on the `spawn_blocking` thread that runs it (installers,
/// locks and the elevated helper block their thread): owns everything it touches.
pub type Op<T> = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = T>>> + Send>;

/// Where the phases of an operation go (production: the CLI's progress line).
pub type Sink = Arc<dyn Fn(&ProgressEvent) + Send + Sync>;

/// `install.lock`, held until dropped.
pub type Held = Box<dyn std::any::Any>;

/// The uninstaller of a plan, run on the removal's thread (given the progress).
pub type Uninstaller =
    Box<dyn FnOnce(&dyn Fn(&ProgressEvent)) -> Result<UninstallReport, String> + Send>;

/// The registry-only removal of a plan, run on the removal's thread (the `.reg` written).
pub type RecordRemover = Box<dyn FnOnce(&dyn Fn(&ProgressEvent)) -> Result<PathBuf, String> + Send>;

/// Said once when Ctrl+C arrives before anything was launched.
pub const CANCELLING_NOTICE: &str = "cancelando: nada foi iniciado; esperando a etapa atual parar";

/// What fails when a removal shares data folders with another installation (Kindle, D6) and
/// nobody can choose.
const SHARED_DATA: &str = "este app compartilha dados com outros";

/// `--yes` on a removal whose data folders another installation shares: the one-line form of
/// the error (`Advice` of [`SHARED_DATA`] and its action).
pub const SHARED_YES_REFUSED: &str =
    "este app compartilha dados com outros; rode sem --yes para escolher";

/// One line per agreement: `Licença: <label> — <url>`, or `Licença aceita: …` once accepted.
pub fn agreement_lines(agreements: &[Agreement], accepted: bool) -> Vec<String> {
    agreements
        .iter()
        .map(|a| bridge::agreement_line(a, accepted))
        .collect()
}

/// The elevated helper of one command; asks it to quit when the command ends, on every path.
pub struct HelperShutdown(pub Arc<ElevatedBridge>);

impl HelperShutdown {
    /// A helper started (through UAC) from this executable, only when first needed.
    pub fn for_this_exe() -> HelperShutdown {
        let exe = std::env::current_exe().unwrap_or_else(|e| {
            tracing::warn!(error = %e, "bridge: own executable unknown; elevated helper unavailable");
            PathBuf::new()
        });
        HelperShutdown::with_transport(Box::new(SystemTransport::new(exe)))
    }

    /// The helper started through `inner`; the folder refusal is printed once (`helper_notice`).
    pub fn with_transport(inner: Box<dyn Transport>) -> HelperShutdown {
        let notice = Box::new(|text: &str| emit::out(&helper_notice(text)));
        HelperShutdown(Arc::new(ElevatedBridge::new(Box::new(
            NoticeTransport::new(inner, notice),
        ))))
    }
}

/// The CLI's native install plan of `manifest` for `host`, with the `installer_mode` setting
/// of `behavior` (the TUI's is `tui::run::native_install_on`; a test compares them).
pub fn install_plan(
    manifest: &crate::sources::winget::manifest::MergedManifest,
    host: &crate::winget_install::select::Host,
    wanted: ScopeWanted,
    behavior: &Behavior,
) -> Result<PlanOrBridge, Refusal> {
    crate::winget_install::plan::plan_with_mode(
        manifest,
        host,
        wanted,
        None,
        behavior.installer_mode,
    )
}

/// The CLI line of a refused helper start (`helper_folder::refusal_text`, as the TUI shows it).
pub fn helper_notice(text: &str) -> String {
    output::warn_text(text)
}

impl Drop for HelperShutdown {
    fn drop(&mut self) {
        self.0.shutdown(HELPER_EXIT_LIMIT);
    }
}

#[cfg(windows)]
pub(super) fn run_tool(
    program: &str,
    args: &[String],
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    bridge::spawn_win::run(program, args, on_line)
}

#[cfg(not(windows))]
pub(super) fn run_tool(
    _program: &str,
    _args: &[String],
    _on_line: &mut dyn FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "a ponte do winget só funciona no Windows",
    ))
}

/// `wingetrs install <winget id>`: `winget install` of the catalog version through the bridge.
/// Prints the package and its license agreements (`--dry-run`: and the command line, then
/// stops), confirms, and runs it elevated (through `helper`) when the manifest or
/// `--scope machine` asks for it. Without `--scope`, a `machine` default in Settings becomes
/// `--scope machine`, dropped once if winget finds no installer for it. `why` is the
/// planner's reason for the bridge (the plan's first step).
#[allow(clippy::too_many_arguments)]
pub(super) async fn install_bridge(
    ctx: &OpsCtx<'_>,
    helper: &ElevatedBridge,
    ui: &dyn Ui,
    cand: &Candidate,
    scope: Option<ScopeArg>,
    why: Option<&str>,
    f: Flags,
    stop: Stop<'_, impl Interrupts>,
) -> anyhow::Result<()> {
    let entry = &cand.entry;
    let pkg = ctx
        .catalog
        .get(&entry.source_name, &entry.id)?
        .context("pacote não encontrado no catálogo")?;
    let blobs = BlobStore::new(ctx.data.blobs_dir());
    let version = pkg.entry.version.clone();
    // The startup already loaded `config.toml` (the command stops when it cannot): the
    // defaults stand in only if it broke since.
    let plan = crate::tui::jobs::plan_winget_install(
        ctx.http,
        &blobs,
        crate::sources::winget::CDN_BASE,
        &pkg,
        &ctx.data.config_path(),
        crate::config::Behavior::default(),
        scope.map(InstallScope::from),
    )
    .await
    .map_err(anyhow::Error::msg)?;
    let crate::tui::jobs::WingetPlan {
        job,
        agreements,
        scope_fallback,
    } = plan;
    let command = f.dry_run.then(|| {
        let (program, args) = bridge::command(&job, Path::new(""), "");
        bridge::command_text(&program, &args)
    });
    let licenses = if !f.yes || f.dry_run {
        agreement_lines(&agreements, false)
    } else {
        Vec::new()
    };
    let text = bridge_text(
        &name_version(&job.name, Some(&version)),
        why,
        &licenses,
        command.as_deref(),
    );
    emit::out(text.trim_end());
    if f.dry_run {
        emit::out(output::DRY_RUN_DONE);
        return Ok(());
    }
    if !proceed(ui, f)? {
        emit::out(&output::cancelled());
        return Ok(());
    }
    if f.yes {
        for line in agreement_lines(&agreements, true) {
            emit::out(&output::license(&line));
        }
    }
    let msi_lock = ctx.data.install_layout().msi_lock();
    let mut exec = run_tool;
    let exec: BridgeExec<'_> = &mut exec;
    run_install_bridge(ctx.progress, ui, stop, job.tool.name(), |on_line| {
        crate::tui::jobs::run_bridge_install(helper, &job, scope_fallback, &msi_lock, exec, on_line)
    })
}

/// The plan of an install through winget, as shown (Task 203): `<name version> via winget`,
/// the numbered steps ([`bridge_step`] of the planner's reason `why`, then the `licenses`) and,
/// only for `--dry-run`, the `command` line.
pub fn bridge_text(
    name_version: &str,
    why: Option<&str>,
    licenses: &[String],
    command: Option<&str>,
) -> String {
    let steps: Vec<String> = std::iter::once(bridge_step(why))
        .chain(licenses.iter().cloned())
        .collect();
    let details: Vec<String> = command
        .map(|c| format!("Comando: {c}"))
        .into_iter()
        .collect();
    output::bridge_plan_text(name_version, &steps, &details)
}

/// The run of a confirmed install through the bridge (`run` is the tool's job, given its line
/// sink): Ctrl+C held, `Bridge { tool }` with its clock from the first instant and the helper's
/// UAC notice as a phase (stderr), the tool's own lines on stdout (`Ui::output`), then the
/// result once the progress line ended.
pub fn run_install_bridge<I: Interrupts>(
    progress: &dyn Fn(&ProgressEvent),
    ui: &dyn Ui,
    stop: Stop<'_, I>,
    tool: &str,
    run: impl FnOnce(&mut dyn FnMut(&str)) -> Result<Done, InstallFail>,
) -> anyhow::Result<()> {
    stop.interrupts.hold();
    let result = super::output::bridge_progress(progress, &|line: &str| ui.output(line), tool, run);
    ui.end_progress();
    stop.interrupts.release();
    match result {
        Ok(done) => {
            ui.output(&output::success("Instalado:", &done.summary));
            Ok(())
        }
        Err(InstallFail::Other(message)) => bail!("{message}"),
        Err(other) => bail!("{other:?}"),
    }
}

/// Everything the winget commands ask of the outside world (fakes in tests).
pub trait WingetFlows {
    /// `[behavior]` of `config.toml` (engine and default scope).
    fn behavior(&self) -> Behavior;
    /// The native plan for installing the catalog package `cand`.
    fn plan_install(
        &self,
        cand: &Candidate,
        wanted: ScopeWanted,
    ) -> impl Future<Output = anyhow::Result<Result<PlanOrBridge, Refusal>>>;
    /// The launch gate the operations take before launching (Ctrl+C cancels through it until
    /// then).
    fn gate(&self) -> LaunchGate;
    /// Where the phases go.
    fn progress(&self) -> Sink;
    /// `winget_install::ops::install`.
    fn install(&self, plan: &WingetPlan) -> Op<Result<InstallReport, WingetOpsError>>;
    /// The install through winget ([`install_bridge`], Task 176).
    fn install_bridge<I: Interrupts>(
        &self,
        ui: &dyn Ui,
        cand: &Candidate,
        scope: Option<ScopeArg>,
        why: Option<&str>,
        f: Flags,
        stop: Stop<'_, I>,
    ) -> impl Future<Output = anyhow::Result<()>>;
    /// Installs portable package `spec` as a wingetrs app: the phase 2a plan, confirmation and
    /// executor (Task 189).
    fn install_portable<I: Interrupts>(
        &self,
        _ui: &dyn Ui,
        _spec: &PortableSpec,
        _f: Flags,
        _stop: Stop<'_, I>,
    ) -> impl Future<Output = anyhow::Result<()>> {
        async { bail!("este fluxo não instala pacotes portable") }
    }
    /// The phase 2a upgrade plan of wingetrs app `id`, a portable package of the winget catalog.
    fn plan_portable_upgrade(
        &self,
        _id: &str,
        _keep_old: bool,
    ) -> impl Future<Output = anyhow::Result<InstallPlan>> {
        async { bail!("este fluxo não atualiza pacotes portable") }
    }
    /// The installed apps, correlated with the catalog (`wingetrs list`).
    fn installed(&self) -> anyhow::Result<Vec<ListRow>>;
    /// `winget_install::uninstall::plan_uninstall` for the record `target` of `row`.
    fn plan_uninstall(
        &self,
        row: &ListRow,
        target: &UninstallTarget,
    ) -> impl Future<Output = Result<UninstallPlan, String>>;
    /// Waits for `install.lock` (nothing else of wingetrs runs during a removal); gives up
    /// with `Cancelled` once the gate is cancelled (never dropped while waiting).
    fn lock_install(&self) -> Op<Result<Held, WingetOpsError>>;
    /// `removal::run_uninstaller` of `plan` (queued on `msi.lock`).
    fn uninstall(&self, plan: &UninstallPlan) -> Uninstaller;
    /// `winget_install::uninstall::remove_registry_only` (exported to `<backups>\<ts>`).
    fn remove_registry_only(&self, plan: &UninstallPlan, ts: &str) -> RecordRemover;
    /// Free bytes on the volume of a path (the folder backup's limit).
    fn free_space(&self) -> fn(&Path) -> Option<u64>;
    /// Pinned apps (`pins::load`).
    fn pins(&self) -> BTreeSet<String>;
    /// `engine::plan_upgrade` of `row` to the catalog version of `package_id`, and the plan
    /// removing the installed record (for `uninstallPrevious`).
    fn plan_upgrade(
        &self,
        row: &ListRow,
        package_id: &str,
        wanted: ScopeWanted,
    ) -> impl Future<Output = anyhow::Result<(Result<PlanOrBridge, Refusal>, Option<UninstallPlan>)>>;
    /// `engine::upgrade`.
    fn upgrade(
        &self,
        plan: &WingetPlan,
        previous: Option<&UninstallPlan>,
    ) -> Op<Result<InstallReport, UpgradeError>>;
    /// `winget upgrade` through the bridge (elevated for a machine-wide app).
    fn upgrade_bridge(
        &self,
        job: &BridgeJob,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<Done, InstallFail>;
    /// ARP records whose uninstaller is gone.
    fn orphans(&self) -> anyhow::Result<Vec<Orphan>>;
    /// The backup folder (`.reg` exports go to `<backups>\<ts>`).
    fn backups(&self) -> PathBuf;
    /// `orphans::remove_orphan_with` (its phases, the helper's UAC notice among them, go to
    /// `progress`); the `.reg` written.
    fn remove_orphan(
        &self,
        o: &Orphan,
        ts: &str,
        progress: &dyn Fn(&ProgressEvent),
    ) -> Result<PathBuf, String>;
}

/// The planner's scope: `--scope` is a demand, the Settings default only a preference.
pub fn wanted_scope(scope: Option<ScopeArg>, default: DefaultScope) -> ScopeWanted {
    match scope {
        Some(ScopeArg::User) => ScopeWanted::Only(Scope::User),
        Some(ScopeArg::Machine) => ScopeWanted::Only(Scope::Machine),
        None => ScopeWanted::Prefer(match default {
            DefaultScope::User => Scope::User,
            DefaultScope::Machine => Scope::Machine,
        }),
    }
}

fn refused(r: Refusal) -> anyhow::Error {
    anyhow!("{r}")
}

/// Prints the plan, stops on `--dry-run`, confirms; `false` when nothing more runs.
fn show_and_confirm(ui: &dyn Ui, text: &str, f: Flags, say: Say<'_>) -> anyhow::Result<bool> {
    say(text.trim_end());
    if f.dry_run {
        say(output::DRY_RUN_DONE);
        return Ok(false);
    }
    if !proceed(ui, f)? {
        say(&output::cancelled());
        return Ok(false);
    }
    Ok(true)
}

/// Runs `op` on a `spawn_blocking` thread while this task keeps reading `interrupts`: before
/// the launch a request cancels through `gate` (said once; the operation sees the gate at its
/// next check and ends by itself, it is never dropped); after it, `uncancellable(<step>)` is
/// said once and the operation goes on. The handle is always awaited and a panic of the worker
/// goes on unwinding here. `None` only when the runtime shuts down.
async fn run_blocking<T: Send + 'static>(
    ui: &dyn Ui,
    interrupts: &mut impl Interrupts,
    gate: &LaunchGate,
    op: Op<T>,
) -> Option<T> {
    let handle = tokio::runtime::Handle::current();
    let task = tokio::task::spawn_blocking(move || handle.block_on(op()));
    tokio::pin!(task);
    let (mut told_launched, mut told_cancel) = (false, false);
    let joined = loop {
        tokio::select! {
            // Requests first: every Ctrl+C that arrived before the end gets its answer.
            biased;
            () = interrupts.next() => {
                if gate.try_cancel() {
                    if !told_cancel {
                        told_cancel = true;
                        ui.notice(CANCELLING_NOTICE);
                    }
                } else if !told_launched {
                    told_launched = true;
                    ui.notice(&uncancellable(gate.running().as_deref()));
                }
            }
            joined = &mut task => break joined,
        }
    };
    match joined {
        Ok(out) => Some(out),
        Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
        // Only a runtime shutting down cancels a blocking task.
        Err(_) => None,
    }
}

/// Runs a native install or upgrade under `job.start`/`job.end` (the agreements the user
/// accepted go to `job.start`) on its own thread, listening to Ctrl+C the whole time.
#[allow(clippy::too_many_arguments)]
async fn run_native<E: std::fmt::Display + Send + 'static>(
    ui: &dyn Ui,
    stop: &mut Stop<'_, impl Interrupts>,
    gate: &LaunchGate,
    info: JobInfo,
    plan: &WingetPlan,
    ok: &str,
    op: Op<Result<InstallReport, E>>,
    say: Say<'_>,
) -> anyhow::Result<InstallReport> {
    let agreements: Vec<String> = plan.agreements.iter().map(bridge::agreement_log).collect();
    // Held from before `job.start` until after `job.end`: the log always gets its end.
    stop.interrupts.hold();
    let log = JobLog::start_accepting(info, &agreements);
    let out = run_blocking(ui, stop.interrupts, gate, op).await;
    ui.end_progress();
    let out = match out {
        Some(out) if !gate.cancelled() => out,
        _ => {
            log.finish(Outcome::Cancelled, "Ctrl+C");
            say(&output::cancelled());
            stop.interrupts.release();
            return Err(Cancelled.into());
        }
    };
    match &out {
        Ok(r) => match &r.warning {
            Some(w) => log.finish(Outcome::NoEffect, w),
            None => log.finish(Outcome::Ok, ok),
        },
        Err(e) => log.finish(Outcome::Failed, &e.to_string()),
    }
    stop.interrupts.release();
    out.map_err(|e| anyhow!("{e}"))
}

/// The native plan as shown: the same steps as the TUI confirmation (`WingetPlan::steps`);
/// URL, command line and SHA-256 only with `--dry-run`, as in the Scoop dry run.
pub fn native_text(plan: &WingetPlan, f: Flags) -> String {
    output::winget_plan_text(plan, f.dry_run)
}

/// `Instalado:`/`Atualizado:`, or the "did not show up" warning in yellow; then the reboot.
fn print_report(r: &InstallReport, done: &str, plan: &WingetPlan, say: Say<'_>) {
    match &r.warning {
        Some(w) => say(&output::warning(w)),
        None => say(&output::success(
            done,
            &name_version(&plan.name, Some(&plan.version)),
        )),
    }
    if let codes::Outcome::OkReboot { initiated } = r.outcome {
        say(&output::reboot(initiated));
    }
}

/// `wingetrs install <winget id>`: natively when the engine and the planner allow it,
/// through winget otherwise (saying why).
pub async fn install<F: WingetFlows>(
    flows: &F,
    ui: &dyn Ui,
    cand: &Candidate,
    scope: Option<ScopeArg>,
    f: Flags,
    mut stop: Stop<'_, impl Interrupts>,
    say: Say<'_>,
) -> anyhow::Result<()> {
    let b = flows.behavior();
    let engine = match b.winget_engine {
        // winget decides everything: no manifest needed here.
        WingetEngine::Winget => Engine::Bridge(None),
        setting => {
            let wanted = wanted_scope(scope, b.default_scope);
            choose_engine(setting, flows.plan_install(cand, wanted).await?).map_err(refused)?
        }
    };
    let plan = match engine {
        // The reason becomes the plan's first step (`Instala pelo winget (zip)`, Task 203).
        Engine::Bridge(why) => {
            return flows
                .install_bridge(ui, cand, scope, why.as_deref(), f, stop)
                .await;
        }
        Engine::Portable(spec) => {
            for line in agreement_lines(&spec.agreements, f.yes) {
                say(&output::license(&line));
            }
            return flows.install_portable(ui, &spec, f, stop).await;
        }
        Engine::Native(plan) => plan,
    };
    if !show_and_confirm(ui, &native_text(&plan, f), f, &mut *say)? {
        return Ok(());
    }
    if f.yes {
        for line in agreement_lines(&plan.agreements, true) {
            say(&output::license(&line));
        }
    }
    let info = JobInfo {
        kind: "install",
        tool: "wingetrs",
        target: plan.id.clone(),
        name: plan.name.clone(),
        from: None,
        to: Some(plan.version.clone()),
        cmd: None,
    };
    let op = flows.install(&plan);
    let gate = flows.gate();
    let report = run_native(
        ui,
        &mut stop,
        &gate,
        info,
        &plan,
        "instalado",
        op,
        &mut *say,
    )
    .await?;
    print_report(&report, "Instalado:", &plan, say);
    Ok(())
}

/// The winget package a row was matched to.
fn winget_id(r: &ListRow) -> Option<&str> {
    crate::winget_install::previous::winget_package(r)
}

/// The installed row (ARP or MSIX) named `target`: by matched winget id or by name, both
/// without case. More than one: the user picks in a terminal; `--yes` or no terminal fails.
/// `Ok(None)` = the user cancelled.
fn pick_row<'a>(
    ui: &dyn Ui,
    rows: &'a [ListRow],
    target: &str,
    yes: bool,
) -> anyhow::Result<Option<&'a ListRow>> {
    let target = target.trim();
    let found: Vec<&ListRow> = rows
        .iter()
        .filter(|r| matches!(r.origin.as_str(), "arp" | "msix"))
        .filter(|r| {
            winget_id(r).is_some_and(|id| id.eq_ignore_ascii_case(target))
                || r.name.eq_ignore_ascii_case(target)
        })
        .collect();
    match found.as_slice() {
        [] => {
            bail!("'{target}' não está instalado (nem pelo wingetrs, nem entre os apps do Windows)")
        }
        [one] => Ok(Some(one)),
        many => {
            // Never pick on the user's behalf: `--yes` and a missing terminal both fail.
            if yes || !ui.interactive() {
                let mut text = format!(
                    "'{target}' corresponde a mais de um app instalado; rode sem --yes num terminal para escolher:"
                );
                for r in many {
                    text.push_str(&format!("\n  {} ({})", r.name, r.location));
                }
                bail!("{text}");
            }
            let items = many
                .iter()
                .map(|r| PickerItem {
                    label: r.name.clone(),
                    detail: r.installed_version.clone().unwrap_or_default(),
                    selectable: true,
                    note: Some(r.location.clone()),
                })
                .collect();
            Ok(ui.pick_target(items)?.and_then(|i| many.get(i).copied()))
        }
    }
}

/// `wingetrs uninstall <id or name>` of an app wingetrs did not install: its record in the
/// Windows list (ARP or MSIX) is planned, shown, confirmed and removed natively. When another
/// installation shares its data folders the user picks how (never `--yes`).
pub async fn uninstall<F: WingetFlows>(
    flows: &F,
    ui: &dyn Ui,
    target: &str,
    purge: bool,
    f: Flags,
    stop: Stop<'_, impl Interrupts>,
    say: Say<'_>,
) -> anyhow::Result<()> {
    let rows = flows.installed()?;
    let Some(row) = pick_row(ui, &rows, target, f.yes)? else {
        say(&output::cancelled());
        return Ok(());
    };
    if purge {
        return Err(output::Advice::new(
            format!(
                "--purge só vale para apps do wingetrs ({} foi instalado por outro programa)",
                row.name
            ),
            "rode de novo sem --purge",
        )
        .into());
    }
    let t = target_of(&row.app_key).ok_or_else(|| {
        anyhow!(
            "{}: registro sem desinstalação conhecida ({})",
            row.name,
            row.app_key
        )
    })?;
    let plan = flows
        .plan_uninstall(row, &t)
        .await
        .map_err(anyhow::Error::msg)?;
    say(output::winget_uninstall_text(&plan, f.dry_run).trim_end());
    if f.dry_run {
        say(output::DRY_RUN_DONE);
        return Ok(());
    }
    let Some(choice) = removal_choice(ui, &plan, f)? else {
        say(&output::cancelled());
        return Ok(());
    };
    run_removal(flows, ui, stop, &row.app_key, plan, choice, say).await
}

/// The data another installation shares with this one, when there is something to choose
/// (folders to copy or the registry-only removal).
fn shared_choice(plan: &UninstallPlan) -> Option<&SharedData> {
    plan.shared
        .as_ref()
        .filter(|s| !s.folders.is_empty() || s.registry_only_allowed)
}

/// Confirms the removal: the usual question, or, when data is shared, the same options as the
/// TUI (copy the folders and remove / remove without copying / only the registry / cancel).
/// `--yes` never chooses for the user. `None` = cancelled.
fn removal_choice(
    ui: &dyn Ui,
    plan: &UninstallPlan,
    f: Flags,
) -> anyhow::Result<Option<RemovalChoice>> {
    let Some(shared) = shared_choice(plan) else {
        return Ok(proceed(ui, f)?.then_some(RemovalChoice::Remove));
    };
    if f.yes {
        return Err(output::Advice::new(SHARED_DATA, "rode sem --yes para escolher").into());
    }
    if !ui.interactive() {
        return Err(output::Advice::new(SHARED_DATA, "rode num terminal para escolher").into());
    }
    let item = |label: &str, detail: String| PickerItem {
        label: label.to_string(),
        detail,
        selectable: true,
        note: None,
    };
    let mut options: Vec<(RemovalChoice, PickerItem)> = Vec::new();
    if !shared.folders.is_empty() {
        let size = shared.size.map(|s| human_size(s.bytes)).unwrap_or_default();
        options.push((
            RemovalChoice::BackupThenRemove,
            item(removal::BACKUP_THEN_REMOVE_LABEL, size),
        ));
    }
    options.push((
        RemovalChoice::Remove,
        item(removal::REMOVE_LABEL, String::new()),
    ));
    if shared.registry_only_allowed {
        options.push((
            RemovalChoice::RegistryOnly,
            item(removal::REGISTRY_ONLY_LABEL, String::new()),
        ));
    }
    let items = options
        .iter()
        .map(|(_, i)| i.clone())
        .chain(std::iter::once(item(removal::CANCEL_LABEL, String::new())))
        .collect();
    Ok(ui
        .pick(items)?
        .and_then(|i| options.get(i).map(|(c, _)| *c)))
}

/// Runs a confirmed removal on its own thread with `removal::remove`, the same sequence as
/// the TUI (`install.lock`, folder copy, launch gate, phase, uninstaller or registry-only).
/// Ctrl+C is read the whole time (see [`run_blocking`]).
async fn run_removal<F: WingetFlows>(
    flows: &F,
    ui: &dyn Ui,
    stop: Stop<'_, impl Interrupts>,
    record: &str,
    plan: UninstallPlan,
    choice: RemovalChoice,
    say: Say<'_>,
) -> anyhow::Result<()> {
    let gate = flows.gate();
    let progress = flows.progress();
    let lock = flows.lock_install();
    let stamp = crate::cache::now_unix().to_string();
    let uninstaller = flows.uninstall(&plan);
    let remover = flows.remove_registry_only(&plan, &stamp);
    let (backups, free) = (flows.backups(), flows.free_space());
    let (launch, job_plan) = (gate.clone(), plan.clone());
    let job: Op<Result<Removed, RemovalError>> = Box::new(move || {
        Box::pin(async move {
            let steps = Steps {
                gate: &launch,
                progress: &*progress,
                backups: &backups,
                stamp: &stamp,
                free_space: &free,
            };
            removal::remove(&steps, lock(), &job_plan, choice, uninstaller, remover).await
        })
    });
    stop.interrupts.hold();
    let log = JobLog::start(JobInfo {
        kind: "remove",
        tool: "wingetrs",
        target: record.to_string(),
        name: plan.name.clone(),
        from: plan.version.clone(),
        to: None,
        cmd: None,
    });
    let out = run_blocking(ui, stop.interrupts, &gate, job).await;
    ui.end_progress();
    let out = match out {
        Some(out) if !gate.cancelled() && !matches!(out, Err(RemovalError::Cancelled)) => out,
        _ => {
            log.finish(Outcome::Cancelled, "Ctrl+C");
            say(&output::cancelled());
            stop.interrupts.release();
            return Err(Cancelled.into());
        }
    };
    match &out {
        Ok(Removed::App { report, .. }) => log.finish(
            report.status.job_outcome(),
            report.warning.as_deref().unwrap_or("removido"),
        ),
        Ok(Removed::Record(_)) => log.finish(Outcome::Ok, "registro removido"),
        Err(e) => log.finish(Outcome::Failed, &e.to_string()),
    }
    stop.interrupts.release();
    match out.map_err(|e| anyhow!("{e}"))? {
        Removed::App { report, copied } => {
            print_uninstalled(&report, &plan, &mut *say);
            if let Some(dest) = copied {
                say(&output::folders_copied(&dest));
            }
        }
        Removed::Record(file) => say(&output::record_removed(&plan.name, &file)),
    }
    Ok(())
}

fn print_uninstalled(r: &UninstallReport, plan: &UninstallPlan, say: Say<'_>) {
    match (r.status, &r.warning) {
        (UninstallStatus::Removed, None) => {
            say(&output::success(
                "Desinstalado:",
                &name_version(&plan.name, plan.version.as_deref()),
            ));
        }
        (_, w) => say(&output::warning(
            w.as_deref()
                .unwrap_or(crate::winget_install::uninstall::KEY_STAYED),
        )),
    }
    if let Some(initiated) = r.reboot {
        say(&output::reboot(initiated));
    }
}

/// `wingetrs upgrade <id or name>` of an app wingetrs did not install, matched to a winget
/// package: natively (`engine::upgrade`) or through winget, as the engine decides.
pub async fn upgrade<F: WingetFlows>(
    flows: &F,
    ui: &dyn Ui,
    target: &str,
    f: Flags,
    mut stop: Stop<'_, impl Interrupts>,
    say: Say<'_>,
) -> anyhow::Result<()> {
    let rows = flows.installed()?;
    let Some(row) = pick_row(ui, &rows, target, f.yes)? else {
        say(&output::cancelled());
        return Ok(());
    };
    let Some(id) = winget_id(row) else {
        bail!(
            "{} não foi associado a um pacote do winget: não há de onde atualizar",
            row.name
        );
    };
    if crate::pins::is_pinned(&flows.pins(), &row.origin, &row.name) {
        return Err(output::Advice::new(
            format!("{} está com a versão fixada", row.name),
            "desafixe na TUI (menu de ações)",
        )
        .into());
    }
    let b = flows.behavior();
    let (engine, previous) = match b.winget_engine {
        WingetEngine::Winget => (Engine::Bridge(None), None),
        setting => {
            let wanted = upgrade_scope(row.scope, b.default_scope);
            let (planned, previous) = flows.plan_upgrade(row, id, wanted).await?;
            (choose_engine(setting, planned).map_err(refused)?, previous)
        }
    };
    let plan = match engine {
        Engine::Bridge(why) => {
            if let Some(why) = why {
                say(&output::through_winget(true, bridge_reason(&why)));
            }
            return upgrade_bridge(flows, ui, row, f, stop, say);
        }
        // `plan_upgrade` never answers portable for a record of the Windows list.
        Engine::Portable(_) => return upgrade_bridge(flows, ui, row, f, stop, say),
        Engine::Native(plan) => plan,
    };
    if !show_and_confirm(ui, &native_text(&plan, f), f, &mut *say)? {
        return Ok(());
    }
    if f.yes {
        for line in agreement_lines(&plan.agreements, true) {
            say(&output::license(&line));
        }
    }
    let info = JobInfo {
        kind: "upgrade",
        tool: "wingetrs",
        target: plan.id.clone(),
        name: plan.name.clone(),
        from: plan.upgrade_from.clone(),
        to: Some(plan.version.clone()),
        cmd: None,
    };
    let op = flows.upgrade(&plan, previous.as_ref());
    let gate = flows.gate();
    let report = run_native(
        ui,
        &mut stop,
        &gate,
        info,
        &plan,
        "atualizado",
        op,
        &mut *say,
    )
    .await?;
    print_report(&report, "Atualizado:", &plan, say);
    Ok(())
}

/// `winget upgrade` of `row` through the bridge (its own `job.start`/`job.end`).
fn upgrade_bridge<F: WingetFlows>(
    flows: &F,
    ui: &dyn Ui,
    row: &ListRow,
    f: Flags,
    stop: Stop<'_, impl Interrupts>,
    say: Say<'_>,
) -> anyhow::Result<()> {
    // Pins were checked by the caller.
    let job =
        bridge::bridge_target(row, &BTreeSet::new()).map_err(|e| anyhow!("{}: {e}", row.name))?;
    let (program, args) = bridge::command(&job, Path::new(""), "");
    let text = format!(
        "{} {} → {} via winget\n{}",
        row.name,
        job.installed.as_deref().unwrap_or("?"),
        job.available.as_deref().unwrap_or("?"),
        bridge::command_text(&program, &args)
    );
    if !show_and_confirm(ui, &text, f, &mut *say)? {
        return Ok(());
    }
    stop.interrupts.hold();
    // Spinner and clock from the first instant; the UAC notice becomes the phase.
    let progress = flows.progress();
    let result = super::output::bridge_progress(
        &*progress,
        &|line: &str| ui.output(line),
        job.tool.name(),
        |on_line| flows.upgrade_bridge(&job, on_line),
    );
    ui.end_progress();
    stop.interrupts.release();
    match result {
        Ok(done) => {
            say(&output::success("Atualizado:", &done.summary));
            Ok(())
        }
        Err(InstallFail::Other(message)) => bail!("{message}"),
        Err(other) => bail!("{other:?}"),
    }
}

/// What `wingetrs cleanup` cleans.
#[derive(Debug, Clone, Copy, Default)]
pub struct CleanupArgs {
    pub orphans: bool,
    pub dry_run: bool,
    pub yes: bool,
}

fn hive_text(h: Hive) -> &'static str {
    match h {
        Hive::MachineX64 => "HKLM 64 bits",
        Hive::MachineX86 => "HKLM 32 bits",
        Hive::User => "HKCU",
    }
}

/// `wingetrs cleanup --orphans`: lists the records whose uninstaller is gone; removes the ones
/// the user picks (none pre-selected) or, with `--yes`, all listed; each exported to a `.reg`
/// first. `--dry-run` lists only.
pub fn cleanup<F: WingetFlows>(
    flows: &F,
    ui: &dyn Ui,
    a: CleanupArgs,
    say: Say<'_>,
) -> anyhow::Result<()> {
    if !a.orphans {
        return Err(output::Advice::new("diga o que limpar", "use --orphans").into());
    }
    let found = flows.orphans()?;
    if found.is_empty() {
        say(output::NO_ORPHANS);
        return Ok(());
    }
    for o in &found {
        say(&output::orphan_line(&o.name, hive_text(o.hive), &o.missing));
    }
    say(&output::orphans_may_have_moved());
    let ts = format!("orphans-{}", crate::cache::now_unix());
    say(&output::orphans_backup(&flows.backups().join(&ts)));
    if a.dry_run {
        say(output::DRY_RUN_DONE);
        return Ok(());
    }
    let chosen: Vec<&Orphan> = if a.yes {
        found.iter().collect()
    } else if ui.interactive() {
        let items = found
            .iter()
            .map(|o| PickerItem {
                label: o.name.clone(),
                detail: hive_text(o.hive).to_string(),
                selectable: true,
                note: Some(format!("falta {}", o.missing)),
            })
            .collect();
        // Never pre-selected: the app may only have moved.
        let initial = vec![false; found.len()];
        match ui.pick_many(items, &initial)? {
            Some(idx) => idx.into_iter().filter_map(|i| found.get(i)).collect(),
            None => Vec::new(),
        }
    } else {
        say(&output::orphans_not_removed());
        return Ok(());
    };
    if chosen.is_empty() {
        say(output::NONE_REMOVED);
        return Ok(());
    }
    // Each removal shows its phase (an HKLM key: the helper's UAC notice first); the results
    // are printed once the progress line is gone.
    let progress = flows.progress();
    let results: Vec<_> = chosen
        .into_iter()
        .map(|o| (o, flows.remove_orphan(o, &ts, &*progress)))
        .collect();
    ui.end_progress();
    let mut failed = 0usize;
    for (o, result) in results {
        match result {
            Ok(file) => say(&output::orphan_removed(&o.name, &file)),
            Err(e) => {
                failed += 1;
                say(&output::orphan_failed(&o.name, &e.to_string()));
            }
        }
    }
    if failed > 0 {
        bail!("{failed} registro(s) órfão(s) não foram removidos");
    }
    Ok(())
}
