//! CLI commands for winget packages (Tasks 176, 186).
use super::install_cmd::{Flags, Interrupts, Stop, Ui, proceed};
use super::style;
use crate::bridge::elevated::{ElevatedBridge, SystemTransport};
use crate::bridge::worker::ScopeArg;
use crate::bridge::{self, BridgeExec};
use crate::cache::BlobStore;
use crate::config::Config;
use crate::install::ops::OpsCtx;
use crate::install::target::Candidate;
use crate::installed::correlate::InstallScope;
use crate::sources::winget::manifest::Agreement;
use crate::tui::app::InstallFail;
use anyhow::{Context, bail};
use std::path::Path;
use std::time::Duration;

/// How long the CLI waits at the end for the elevated helper to quit.
const HELPER_EXIT_LIMIT: Duration = Duration::from_secs(5);

/// One line per agreement: `Licença: <label> — <url>`, or `Licença aceita: …` once accepted.
pub fn agreement_lines(agreements: &[Agreement], accepted: bool) -> Vec<String> {
    agreements
        .iter()
        .map(|a| bridge::agreement_line(a, accepted))
        .collect()
}

/// Asks the elevated helper to quit when the command ends, on every path.
struct HelperShutdown(ElevatedBridge);

impl Drop for HelperShutdown {
    fn drop(&mut self) {
        self.0.shutdown(HELPER_EXIT_LIMIT);
    }
}

#[cfg(windows)]
fn run_tool(
    program: &str,
    args: &[String],
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    bridge::spawn_win::run(program, args, on_line)
}

#[cfg(not(windows))]
fn run_tool(
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
/// stops), confirms, and runs it elevated when the manifest or `--scope machine` asks for it.
/// Without `--scope`, a `machine` default in Settings becomes `--scope machine`, dropped once
/// if winget finds no installer for it.
pub(super) async fn install_bridge(
    ctx: &OpsCtx<'_>,
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
    let hash = pkg
        .winget_hash
        .as_deref()
        .context("pacote winget sem hash de manifesto")?;
    let blobs = BlobStore::new(ctx.data.blobs_dir());
    let version = pkg.entry.version.clone();
    let (manifest, _) = crate::sources::winget::details::merged_manifest(
        ctx.http,
        &blobs,
        crate::sources::winget::CDN_BASE,
        &pkg.entry.id,
        hash,
        Some(&version),
    )
    .await?;
    let explicit = scope.map(InstallScope::from);
    let from_settings = match explicit {
        Some(_) => None,
        None => bridge::default_scope(
            Config::load(&ctx.data.config_path())?
                .behavior
                .default_scope,
        ),
    };
    let job = bridge::install_job(
        &pkg.entry.id,
        &pkg.entry.name,
        &version,
        &manifest,
        explicit.or(from_settings),
        bridge::host_arch(),
    );
    anstream::println!("{} {version} (winget) via winget", job.name);
    if !f.yes || f.dry_run {
        for line in agreement_lines(&manifest.agreements, false) {
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
        for line in agreement_lines(&manifest.agreements, true) {
            anstream::println!("{line}");
        }
    }
    let accepted: Vec<String> = manifest
        .agreements
        .iter()
        .map(bridge::agreement_log)
        .collect();
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "bridge: own executable unknown; elevated helper unavailable");
        std::path::PathBuf::new()
    });
    let helper = HelperShutdown(ElevatedBridge::new(Box::new(SystemTransport::new(exe))));
    let msi_lock = ctx.data.install_layout().msi_lock();
    let mut exec = run_tool;
    let exec: BridgeExec<'_> = &mut exec;
    stop.interrupts.hold();
    let result = crate::tui::jobs::run_bridge_install(
        &helper.0,
        &job,
        &accepted,
        from_settings.is_some(),
        &msi_lock,
        exec,
        &mut |line: &str| anstream::println!("  {line}"),
    );
    stop.interrupts.release();
    drop(helper);
    match result {
        Ok(done) => {
            anstream::println!("{} {}", style::paint(style::OK, "Instalado:"), done.summary);
            Ok(())
        }
        Err(InstallFail::Other(message)) => bail!("{message}"),
        Err(other) => bail!("{other:?}"),
    }
}
