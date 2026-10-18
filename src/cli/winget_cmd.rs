//! CLI commands for winget packages (Tasks 176, 186).
//!
//! Platform-neutral: every real effect arrives through [`WingetFlows`] (production:
//! `run_win::ProdFlows`), every prompt through `Ui`, every printed line through a `Say`.
use super::install_cmd::{Cancelled, Flags, Interrupts, Stop, Ui, proceed};
use super::style;
use crate::bridge::elevated::{ElevatedBridge, SystemTransport};
use crate::bridge::worker::ScopeArg;
use crate::bridge::{self, BridgeExec, BridgeJob};
use crate::cache::BlobStore;
use crate::config::{Behavior, DefaultScope, WingetEngine};
use crate::install::executor::LaunchGate;
use crate::install::ops::OpsCtx;
use crate::install::progress::{Phase, ProgressEvent};
use crate::install::target::Candidate;
use crate::installed::Hive;
use crate::installed::correlate::{Correlation, InstallScope, ListRow};
use crate::job_log::{JobInfo, JobLog, Outcome};
use crate::sources::winget::manifest::Agreement;
use crate::tui::app::{Done, InstallFail};
use crate::tui::picker::PickerItem;
use crate::winget_install::codes;
use crate::winget_install::engine::{Engine, UpgradeError, choose_engine, upgrade_scope};
use crate::winget_install::launch::Elevation;
use crate::winget_install::ops::{InstallReport, WingetOpsError};
use crate::winget_install::orphans::Orphan;
use crate::winget_install::plan::{PlanOrBridge, WingetPlan};
use crate::winget_install::select::{Refusal, Scope, ScopeWanted};
use crate::winget_install::uninstall::{
    BackupLimits, MsiQueue, SharedData, UninstallCtx, UninstallPlan, UninstallReport,
    UninstallStatus, UninstallTarget, backup_folders, human_size, target_of, uninstall_queued,
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

/// Said once when Ctrl+C arrives after the installer (or uninstaller) was launched.
pub const LAUNCHED_NOTICE: &str = "o instalador já começou e não pode ser interrompido; aguarde";

/// Said once when Ctrl+C arrives before anything was launched.
pub const CANCELLING_NOTICE: &str = "cancelando: nada foi iniciado; esperando a etapa atual parar";

/// `--yes` on a removal whose data folders another installation shares (Kindle, D6).
pub const SHARED_YES_REFUSED: &str =
    "este app compartilha dados com outros; rode sem --yes para escolher";

/// How often a waiting operation looks at the launch gate (Ctrl+C before the launch).
const CANCEL_POLL: Duration = Duration::from_millis(50);

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
        HelperShutdown(Arc::new(ElevatedBridge::new(Box::new(
            SystemTransport::new(exe),
        ))))
    }
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
/// `--scope machine`, dropped once if winget finds no installer for it.
pub(super) async fn install_bridge(
    ctx: &OpsCtx<'_>,
    helper: &ElevatedBridge,
    ui: &dyn Ui,
    cand: &Candidate,
    scope: Option<ScopeArg>,
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
    anstream::println!("{} via winget", name_version(&job.name, Some(&version)));
    if !f.yes || f.dry_run {
        for line in agreement_lines(&agreements, false) {
            anstream::println!("{line}");
        }
    }
    if f.dry_run {
        let (program, args) = bridge::command(&job, Path::new(""), "");
        anstream::println!("{}", bridge::command_text(&program, &args));
        anstream::println!("Simulação: nada foi alterado.");
        return Ok(());
    }
    if !proceed(ui, f)? {
        anstream::println!("{}", style::paint(style::WARN, "Cancelado."));
        return Ok(());
    }
    if f.yes {
        for line in agreement_lines(&agreements, true) {
            anstream::println!("{line}");
        }
    }
    let msi_lock = ctx.data.install_layout().msi_lock();
    let mut exec = run_tool;
    let exec: BridgeExec<'_> = &mut exec;
    stop.interrupts.hold();
    let result = crate::tui::jobs::run_bridge_install(
        helper,
        &job,
        scope_fallback,
        &msi_lock,
        exec,
        &mut |line: &str| anstream::println!("  {line}"),
    );
    stop.interrupts.release();
    match result {
        Ok(done) => {
            anstream::println!("{} {}", style::paint(style::OK, "Instalado:"), done.summary);
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
        f: Flags,
        stop: Stop<'_, I>,
    ) -> impl Future<Output = anyhow::Result<()>>;
    /// The installed apps, correlated with the catalog (`wingetrs list`).
    fn installed(&self) -> anyhow::Result<Vec<ListRow>>;
    /// `winget_install::uninstall::plan_uninstall` for the record `target` of `row`.
    fn plan_uninstall(
        &self,
        row: &ListRow,
        target: &UninstallTarget,
    ) -> impl Future<Output = Result<UninstallPlan, String>>;
    /// Waits for `install.lock` (nothing else of wingetrs runs during a removal).
    fn lock_install(&self) -> Op<Result<Held, String>>;
    /// `winget_install::uninstall::uninstall`; announces [`checking_phase`] once the
    /// uninstaller ended and the record is being watched.
    fn uninstall(&self, plan: &UninstallPlan) -> Op<Result<UninstallReport, String>>;
    /// `winget_install::uninstall::remove_registry_only` (exported to `<backups>\<ts>`).
    fn remove_registry_only(&self, plan: &UninstallPlan, ts: &str) -> Op<Result<PathBuf, String>>;
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
    /// `orphans::remove_orphan`; the `.reg` written.
    fn remove_orphan(&self, o: &Orphan, ts: &str) -> Result<PathBuf, String>;
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
        say("Simulação: nada foi alterado.");
        return Ok(false);
    }
    if !proceed(ui, f)? {
        say(&style::paint(style::WARN, "Cancelado."));
        return Ok(false);
    }
    Ok(true)
}

/// `op` until it ends, or `None` once the gate was cancelled while it waited (it is dropped
/// at its next await; a blocking step runs to its end first).
async fn or_cancelled<T>(gate: &LaunchGate, op: impl Future<Output = T>) -> Option<T> {
    tokio::pin!(op);
    loop {
        tokio::select! {
            biased;
            out = &mut op => return Some(out),
            () = tokio::time::sleep(CANCEL_POLL) => {
                if gate.cancelled() {
                    return None;
                }
            }
        }
    }
}

/// Runs `op` on a `spawn_blocking` thread while this task keeps reading `interrupts`: before
/// the launch a request cancels through `gate` (said once, then the step is awaited); after
/// it, [`LAUNCHED_NOTICE`] is said once and the operation goes on. The handle is always
/// awaited and a panic of the worker goes on unwinding here. `None` = dropped by the gate.
async fn run_blocking<T: Send + 'static>(
    ui: &dyn Ui,
    interrupts: &mut impl Interrupts,
    gate: &LaunchGate,
    op: Op<T>,
) -> Option<T> {
    let handle = tokio::runtime::Handle::current();
    let watched = gate.clone();
    let task = tokio::task::spawn_blocking(move || handle.block_on(or_cancelled(&watched, op())));
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
                    ui.notice(LAUNCHED_NOTICE);
                }
            }
            joined = &mut task => break joined,
        }
    };
    match joined {
        Ok(out) => out,
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
            say(&style::paint(style::WARN, "Cancelado."));
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

/// `name version`, or only `name` when it already carries the version (`CrystalDiskMark
/// 9.0.3`, as many ARP display names do).
pub fn name_version(name: &str, version: Option<&str>) -> String {
    match version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v)
            if !name
                .split_whitespace()
                .any(|w| w == v || w.trim_start_matches(['v', 'V']) == v) =>
        {
            format!("{name} {v}")
        }
        _ => name.to_string(),
    }
}

/// The planner's reason for the bridge without its "instalado pelo winget" ending (the line
/// already says it).
pub fn bridge_why(why: &str) -> &str {
    why.strip_suffix(": instalado pelo winget").unwrap_or(why)
}

/// The native plan as shown; `--dry-run` adds the installer's SHA-256 (URL, hash and staging
/// appear only there, as in the Scoop dry run).
pub fn native_text(plan: &WingetPlan, f: Flags) -> String {
    let mut text = plan.to_string().trim_end().to_string();
    if f.dry_run {
        text.push_str(&format!("\nSHA-256: {}", plan.installer.sha256));
    }
    text
}

/// The removal plan as shown: the version once, and an uninstaller that is `winget` itself
/// (a portable app winget installed) is said to go through winget, not to open a window.
pub fn uninstall_text(plan: &UninstallPlan) -> String {
    let text = plan.to_string();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if let Some(first) = lines.first_mut() {
        *first = format!(
            "Desinstalar {}",
            name_version(&plan.name, plan.version.as_deref())
        );
    }
    let by_winget = plan.command.as_ref().is_some_and(|c| {
        Path::new(c.program.trim().trim_matches('"'))
            .file_stem()
            .is_some_and(|s| s.eq_ignore_ascii_case("winget"))
    });
    if by_winget {
        for l in &mut lines {
            if l.trim() == "abre a janela do desinstalador" {
                *l = "  remove pelo winget (pode mostrar o progresso dele)".to_string();
            }
        }
    }
    lines.join("\n")
}

/// `Instalado:`/`Atualizado:`, or the "did not show up" warning in yellow; then the reboot.
fn print_report(r: &InstallReport, done: &str, plan: &WingetPlan, say: Say<'_>) {
    match &r.warning {
        Some(w) => say(&format!("{} {w}", style::paint(style::WARN, "aviso:"))),
        None => say(&format!(
            "{} {}",
            style::paint(style::OK, done),
            name_version(&plan.name, Some(&plan.version))
        )),
    }
    if let codes::Outcome::OkReboot { initiated } = r.outcome {
        say(reboot_text(initiated));
    }
}

fn reboot_text(initiated: bool) -> &'static str {
    if initiated {
        "O instalador está reiniciando o computador."
    } else {
        "Reinicie o computador para concluir."
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
        Engine::Bridge(why) => {
            if let Some(why) = why {
                say(&format!("Instalando pelo winget: {}", bridge_why(&why)));
            }
            return flows.install_bridge(ui, cand, scope, f, stop).await;
        }
        Engine::Native(plan) => plan,
    };
    if !show_and_confirm(ui, &native_text(&plan, f), f, &mut *say)? {
        return Ok(());
    }
    if f.yes {
        for line in agreement_lines(&plan.agreements, true) {
            say(&line);
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
pub(super) fn winget_id(r: &ListRow) -> Option<&str> {
    match &r.correlation {
        Correlation::Matched {
            source, package_id, ..
        } if source == "winget" => Some(package_id),
        _ => None,
    }
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
            Ok(ui.pick(items)?.and_then(|i| many.get(i).copied()))
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
        say(&style::paint(style::WARN, "Cancelado."));
        return Ok(());
    };
    if purge {
        bail!(
            "--purge só vale para apps do wingetrs ({} foi instalado por outro programa)",
            row.name
        );
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
    say(uninstall_text(&plan).trim_end());
    if f.dry_run {
        say("Simulação: nada foi alterado.");
        return Ok(());
    }
    let Some(choice) = removal_choice(ui, &plan, f)? else {
        say(&style::paint(style::WARN, "Cancelado."));
        return Ok(());
    };
    run_removal(flows, ui, stop, &row.app_key, plan, choice, say).await
}

/// How a confirmed native removal goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalChoice {
    /// The uninstaller, nothing copied.
    Remove,
    /// The shared data folders copied to the backups first, then the uninstaller.
    BackupThenRemove,
    /// Only this installation's registry record (exported first); the app stays.
    RegistryOnly,
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
        bail!("{SHARED_YES_REFUSED}");
    }
    if !ui.interactive() {
        bail!("este app compartilha dados com outros; rode num terminal para escolher");
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
            item("Copiar as pastas para backups e remover", size),
        ));
    }
    options.push((
        RemovalChoice::Remove,
        item("Remover sem copiar as pastas", String::new()),
    ));
    if shared.registry_only_allowed {
        options.push((
            RemovalChoice::RegistryOnly,
            item(crate::tui::app::REGISTRY_ONLY_ITEM, String::new()),
        ));
    }
    let items = options
        .iter()
        .map(|(_, i)| i.clone())
        .chain(std::iter::once(item("Cancelar", String::new())))
        .collect();
    Ok(ui
        .pick(items)?
        .and_then(|i| options.get(i).map(|(c, _)| *c)))
}

/// The phase of a removal once launched: the UAC (and the elevated helper) or the removal.
fn removal_phase(elevated: bool, name: &str) -> Phase {
    if elevated {
        Phase::WaitingElevation
    } else {
        Phase::Removing {
            app: name.to_string(),
        }
    }
}

/// The native uninstall of a CLI removal, on its thread: queued on `msi.lock` (an
/// `msiexec /x` waits for another installer and retries 1618, Task 194), announcing
/// [`checking_phase`] once the uninstaller ended and the record is being watched.
pub fn run_uninstaller(
    u: &UninstallCtx<'_>,
    queue: &MsiQueue<'_>,
    plan: &UninstallPlan,
    progress: &dyn Fn(&ProgressEvent),
) -> Result<UninstallReport, String> {
    let watching = std::sync::atomic::AtomicBool::new(false);
    // The first look at the record comes right after the uninstaller ended.
    let key_watched = |hive: Hive, key: &str| {
        if !watching.swap(true, std::sync::atomic::Ordering::Relaxed) {
            progress(&ProgressEvent::Phase(checking_phase(&plan.name)));
        }
        (u.key_exists)(hive, key)
    };
    let watched = UninstallCtx {
        key_exists: &key_watched,
        ..*u
    };
    uninstall_queued(&watched, Some(queue), plan)
}

/// The phase while the record is watched after the uninstaller ended (up to two minutes).
pub fn checking_phase(name: &str) -> Phase {
    Phase::Removing {
        app: format!("{name} (conferindo a remoção)"),
    }
}

/// What a removal did.
enum Removed {
    App(UninstallReport, Option<PathBuf>),
    Record(PathBuf),
}

/// Runs a confirmed removal on its own thread, as the TUI does: `install.lock`, the folder
/// copy (a refused copy removes nothing), the launch gate, the phase, then the uninstaller or
/// the registry-only removal. Ctrl+C is read the whole time (see [`run_blocking`]).
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
    let backup = match (choice, &plan.shared) {
        (RemovalChoice::BackupThenRemove, Some(s)) if !s.folders.is_empty() => Some((
            s.folders.clone(),
            flows.backups().join(format!("{stamp}-pastas")),
            flows.free_space(),
        )),
        _ => None,
    };
    let (elevated, work): (bool, Op<Result<Removed, String>>) = match choice {
        RemovalChoice::RegistryOnly => {
            let hklm =
                matches!(&plan.target, UninstallTarget::Arp { hive, .. } if *hive != Hive::User);
            let op = flows.remove_registry_only(&plan, &stamp);
            (
                hklm,
                Box::new(move || Box::pin(async move { op().await.map(Removed::Record) })),
            )
        }
        RemovalChoice::Remove | RemovalChoice::BackupThenRemove => {
            let op = flows.uninstall(&plan);
            (
                plan.elevation == Elevation::Helper,
                Box::new(move || {
                    Box::pin(async move { op().await.map(|r| Removed::App(r, None)) })
                }),
            )
        }
    };
    let phase = removal_phase(elevated, &plan.name);
    let name = plan.name.clone();
    let launch = gate.clone();
    let job: Op<Result<Removed, String>> = Box::new(move || {
        Box::pin(async move {
            let _held = lock().await?;
            let mut copied = None;
            if let Some((folders, dest, free)) = backup {
                let limits = BackupLimits::standard(&free);
                let notes = backup_folders(&folders, &dest, &limits).map_err(|e| {
                    format!("{name}: cópia das pastas recusada ({e}); nada foi removido")
                })?;
                for n in &notes {
                    tracing::warn!(app = %name, "backup: {n}");
                }
                copied = Some(dest);
            }
            if !launch.try_launch_for("desinstalador") {
                return Err(WingetOpsError::Cancelled.to_string());
            }
            progress(&ProgressEvent::Phase(phase));
            match work().await? {
                Removed::App(r, _) => Ok(Removed::App(r, copied)),
                other => Ok(other),
            }
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
        Some(out) if !gate.cancelled() => out,
        _ => {
            log.finish(Outcome::Cancelled, "Ctrl+C");
            say(&style::paint(style::WARN, "Cancelado."));
            stop.interrupts.release();
            return Err(Cancelled.into());
        }
    };
    match &out {
        Ok(Removed::App(r, _)) => log.finish(
            r.status.job_outcome(),
            r.warning.as_deref().unwrap_or("removido"),
        ),
        Ok(Removed::Record(_)) => log.finish(Outcome::Ok, "registro removido"),
        Err(e) => log.finish(Outcome::Failed, e),
    }
    stop.interrupts.release();
    match out.map_err(anyhow::Error::msg)? {
        Removed::App(report, copied) => {
            print_uninstalled(&report, &plan, &mut *say);
            if let Some(dest) = copied {
                say(&format!("Pastas copiadas para {}", dest.display()));
            }
        }
        Removed::Record(file) => say(&format!(
            "{} registro de {} (cópia em {})",
            style::paint(style::OK, "Removido:"),
            plan.name,
            file.display()
        )),
    }
    Ok(())
}
fn print_uninstalled(r: &UninstallReport, plan: &UninstallPlan, say: Say<'_>) {
    match (r.status, &r.warning) {
        (UninstallStatus::Removed, None) => {
            say(&format!(
                "{} {}",
                style::paint(style::OK, "Desinstalado:"),
                name_version(&plan.name, plan.version.as_deref())
            ));
        }
        (_, w) => say(&format!(
            "{} {}",
            style::paint(style::WARN, "aviso:"),
            w.as_deref()
                .unwrap_or(crate::winget_install::uninstall::KEY_STAYED)
        )),
    }
    if let Some(initiated) = r.reboot {
        say(reboot_text(initiated));
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
        say(&style::paint(style::WARN, "Cancelado."));
        return Ok(());
    };
    let Some(id) = winget_id(row) else {
        bail!(
            "{} não foi associado a um pacote do winget: não há de onde atualizar",
            row.name
        );
    };
    if crate::pins::is_pinned(&flows.pins(), &row.origin, &row.name) {
        bail!(
            "{} está com a versão fixada; desafixe na TUI (menu de ações)",
            row.name
        );
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
                say(&format!("Atualizando pelo winget: {}", bridge_why(&why)));
            }
            return upgrade_bridge(flows, ui, row, f, stop, say);
        }
        Engine::Native(plan) => plan,
    };
    if !show_and_confirm(ui, &native_text(&plan, f), f, &mut *say)? {
        return Ok(());
    }
    if f.yes {
        for line in agreement_lines(&plan.agreements, true) {
            say(&line);
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
    let result = flows.upgrade_bridge(&job, &mut |line: &str| say(&format!("  {line}")));
    stop.interrupts.release();
    match result {
        Ok(done) => {
            say(&format!(
                "{} {}",
                style::paint(style::OK, "Atualizado:"),
                done.summary
            ));
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
        bail!("diga o que limpar: --orphans");
    }
    let found = flows.orphans()?;
    if found.is_empty() {
        say("Nenhum registro órfão.");
        return Ok(());
    }
    for o in &found {
        say(&format!(
            "Órfão: {} — {} — falta {}",
            o.name,
            hive_text(o.hive),
            o.missing
        ));
    }
    say("O app pode ter mudado de lugar (ex.: Steam em outra pasta).");
    let ts = format!("orphans-{}", crate::cache::now_unix());
    say(&format!(
        "Antes de apagar, cada registro é exportado para um .reg em {}",
        flows.backups().join(&ts).display()
    ));
    if a.dry_run {
        say("Simulação: nada foi alterado.");
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
        say("Nada foi removido: sem terminal, use --yes para remover todos os listados.");
        return Ok(());
    };
    if chosen.is_empty() {
        say("Nenhum registro removido.");
        return Ok(());
    }
    let mut failed = 0usize;
    for o in chosen {
        match flows.remove_orphan(o, &ts) {
            Ok(file) => say(&format!(
                "{} {} (backup {})",
                style::paint(style::OK, "Removido:"),
                o.name,
                file.display()
            )),
            Err(e) => {
                failed += 1;
                say(&format!(
                    "{} {}: {e}",
                    style::paint(style::ERROR, "Erro:"),
                    o.name
                ));
            }
        }
    }
    if failed > 0 {
        bail!("{failed} registro(s) órfão(s) não foram removidos");
    }
    Ok(())
}
