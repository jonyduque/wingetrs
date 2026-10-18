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
use crate::install::ops::OpsCtx;
use crate::install::target::Candidate;
use crate::installed::Hive;
use crate::installed::correlate::{Correlation, InstallScope, ListRow};
use crate::job_log::{JobInfo, JobLog, Outcome};
use crate::sources::winget::manifest::Agreement;
use crate::tui::app::{Done, InstallFail};
use crate::tui::picker::PickerItem;
use crate::winget_install::codes;
use crate::winget_install::engine::{Engine, UpgradeError, choose_engine, upgrade_scope};
use crate::winget_install::ops::{InstallReport, WingetOpsError};
use crate::winget_install::orphans::Orphan;
use crate::winget_install::plan::{PlanOrBridge, WingetPlan};
use crate::winget_install::select::{Refusal, Scope, ScopeWanted};
use crate::winget_install::uninstall::{
    UninstallPlan, UninstallReport, UninstallStatus, UninstallTarget, target_of,
};
use anyhow::{Context, anyhow, bail};
use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the CLI waits at the end for the elevated helper to quit.
const HELPER_EXIT_LIMIT: Duration = Duration::from_secs(5);

/// Where the commands print one line (stdout in production, a buffer in tests).
pub type Say<'a> = &'a mut dyn FnMut(&str);

/// One line per agreement: `Licença: <label> — <url>`, or `Licença aceita: …` once accepted.
pub fn agreement_lines(agreements: &[Agreement], accepted: bool) -> Vec<String> {
    agreements
        .iter()
        .map(|a| bridge::agreement_line(a, accepted))
        .collect()
}

/// The elevated helper of one command; asks it to quit when the command ends, on every path.
pub struct HelperShutdown(pub ElevatedBridge);

impl HelperShutdown {
    /// A helper started (through UAC) from this executable, only when first needed.
    pub fn for_this_exe() -> HelperShutdown {
        let exe = std::env::current_exe().unwrap_or_else(|e| {
            tracing::warn!(error = %e, "bridge: own executable unknown; elevated helper unavailable");
            PathBuf::new()
        });
        HelperShutdown(ElevatedBridge::new(Box::new(SystemTransport::new(exe))))
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
    anstream::println!("{} {version} (winget) via winget", job.name);
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
    /// `winget_install::ops::install`.
    fn install(
        &self,
        plan: &WingetPlan,
    ) -> impl Future<Output = Result<InstallReport, WingetOpsError>>;
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
    ) -> Result<UninstallPlan, String>;
    /// `winget_install::uninstall::uninstall`.
    fn uninstall(&self, plan: &UninstallPlan) -> Result<UninstallReport, String>;
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
    ) -> impl Future<Output = Result<InstallReport, UpgradeError>>;
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

/// Runs a native install or upgrade under `job.start`/`job.end` (the agreements the user
/// accepted go to `job.start`), listening to Ctrl+C until the installer is launched.
async fn run_native<E: std::fmt::Display>(
    ui: &dyn Ui,
    stop: &mut Stop<'_, impl Interrupts>,
    info: JobInfo,
    plan: &WingetPlan,
    ok: &str,
    op: impl Future<Output = Result<InstallReport, E>>,
    say: Say<'_>,
) -> anyhow::Result<InstallReport> {
    let agreements: Vec<String> = plan.agreements.iter().map(bridge::agreement_log).collect();
    // Held from before `job.start` until after `job.end`: the log always gets its end.
    stop.interrupts.hold();
    let log = JobLog::start_accepting(info, &agreements);
    let out = stop.run(ui, op).await;
    ui.end_progress();
    let Some(out) = out else {
        log.finish(Outcome::Cancelled, "Ctrl+C");
        say(&style::paint(style::WARN, "Cancelado."));
        stop.interrupts.release();
        return Err(Cancelled.into());
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

/// `Instalado:`/`Atualizado:`, or the "did not show up" warning in yellow; then the reboot.
fn print_report(r: &InstallReport, done: &str, plan: &WingetPlan, say: Say<'_>) {
    match &r.warning {
        Some(w) => say(&format!("{} {w}", style::paint(style::WARN, "aviso:"))),
        None => say(&format!(
            "{} {} {}",
            style::paint(style::OK, done),
            plan.name,
            plan.version
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
                say(&format!("Instalando pelo winget: {why}"));
            }
            return flows.install_bridge(ui, cand, scope, f, stop).await;
        }
        Engine::Native(plan) => plan,
    };
    if !show_and_confirm(ui, &plan.to_string(), f, &mut *say)? {
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
    let report = run_native(ui, &mut stop, info, &plan, "instalado", op, &mut *say).await?;
    print_report(&report, "Instalado:", &plan, say);
    Ok(())
}

/// The winget package a row was matched to.
fn winget_id(r: &ListRow) -> Option<&str> {
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
/// Windows list (ARP or MSIX) is planned, shown, confirmed and removed natively.
pub fn uninstall<F: WingetFlows>(
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
    let plan = flows.plan_uninstall(row, &t).map_err(anyhow::Error::msg)?;
    if !show_and_confirm(ui, &plan.to_string(), f, &mut *say)? {
        return Ok(());
    }
    // Synchronous: Ctrl+C is held while the uninstaller runs, then let go.
    stop.interrupts.hold();
    let log = JobLog::start(JobInfo {
        kind: "remove",
        tool: "wingetrs",
        target: row.app_key.clone(),
        name: plan.name.clone(),
        from: plan.version.clone(),
        to: None,
        cmd: None,
    });
    let removed = flows.uninstall(&plan);
    ui.end_progress();
    match &removed {
        Ok(r) => log.finish(
            r.status.job_outcome(),
            r.warning.as_deref().unwrap_or("removido"),
        ),
        Err(e) => log.finish(Outcome::Failed, e),
    }
    stop.interrupts.release();
    let report = removed.map_err(anyhow::Error::msg)?;
    print_uninstalled(&report, &plan, say);
    Ok(())
}

fn print_uninstalled(r: &UninstallReport, plan: &UninstallPlan, say: Say<'_>) {
    match (r.status, &r.warning) {
        (UninstallStatus::Removed, None) => {
            let version = plan.version.as_deref().unwrap_or("");
            say(format!(
                "{} {} {version}",
                style::paint(style::OK, "Desinstalado:"),
                plan.name
            )
            .trim_end());
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
                say(&format!("Atualizando pelo winget: {why}"));
            }
            return upgrade_bridge(flows, ui, row, f, stop, say);
        }
        Engine::Native(plan) => plan,
    };
    if !show_and_confirm(ui, &plan.to_string(), f, &mut *say)? {
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
    let report = run_native(ui, &mut stop, info, &plan, "atualizado", op, &mut *say).await?;
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
