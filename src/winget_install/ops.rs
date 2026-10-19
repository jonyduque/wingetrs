//! Native install flow (Task 181): download, verify, launch (unelevated, or elevated through
//! the installer's own UAC prompt, Task 196), classify the exit code and look for the app among
//! the installed ones.
//!
//! Order: `install.lock` → download → hash again through a share-deny handle → `msi.lock`
//! (command-line installers) → launch gate → launch → share-deny handle released → classify →
//! post-check → record.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::codes::{self, Category, Outcome, UAC_REFUSED};
use super::fetch::{FetchError, Materialized, fetch_installer_until};
use super::launch::{Elevation, LaunchSpec, Launcher, Show, Verb};
use super::msix::{self, MsixOps};
use super::plan::{WingetPlan, elevated_kind};
use super::post_check::find_installed;
use super::switches::{self, ArgsError, CommandLine, Program};
use crate::bridge::elevated::ElevatedRunner;
use crate::bridge::worker::InstallerRequest;
use crate::cache::{DataDir, UpdateLock};
use crate::http::Http;
use crate::install::download::DownloadError;
use crate::install::download::Progress;
use crate::install::executor::LaunchGate;
use crate::install::layout::Layout;
use crate::install::msi_queue::{self, Attempt, MsiQueueError, Status};
use crate::install::progress::{Phase, ProgressEvent};
use crate::install::safepath::safe_components;
use crate::installed::InstalledApp;
use crate::sources::winget::manifest::InstallerKind;
use crate::update::LockWait;

/// Shown (and recorded as "no effect") when the installer succeeded but the app is not listed.
pub const NOT_FOUND_WARNING: &str =
    "o instalador terminou sem erro, mas o app não apareceu na lista de instalados";

/// What the flow needs from the outside world; tests inject fakes for every side effect.
pub struct WingetCtx<'a> {
    pub data: &'a DataDir,
    pub http: &'a Http,
    /// CLI: `LockWait::from_env()`; TUI: `NoWait`.
    pub wait: LockWait,
    pub progress: Progress<'a>,
    /// Shared with the caller: Ctrl+C cancels through it until the installer is launched.
    pub gate: LaunchGate,
    pub launcher: &'a dyn Launcher,
    /// The session's elevated helper (uninstallers and ARP keys; never an installer).
    pub elevated: &'a dyn ElevatedRunner,
    pub msix: &'a dyn MsixOps,
    pub scan: &'a (dyn Fn() -> Vec<InstalledApp> + Send + Sync),
    /// Absolute `System32\msiexec.exe` (never looked up through PATH).
    pub msiexec: PathBuf,
    /// Whether the Windows Installer mutex is held (`Global\_MSIExecute`).
    pub busy: &'a (dyn Fn() -> bool + Send + Sync),
    pub sleep: &'a (dyn Fn(Duration) + Send + Sync),
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    /// `Ok` or `OkReboot` (a failure is an `Err`).
    pub outcome: Outcome,
    pub found: Option<InstalledApp>,
    /// Set when the job must not be reported as "Instalado" (see [`NOT_FOUND_WARNING`]).
    pub warning: Option<String>,
    /// The installer's log, when it wrote one.
    pub log: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum WingetOpsError {
    #[error("outra instalação do wingetrs está em andamento")]
    Busy,
    #[error("trava de instalação: {0}")]
    Lock(String),
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("instalação cancelada")]
    Cancelled,
    #[error("{}", UAC_REFUSED)]
    UacRefused,
    #[error("{0}")]
    Elevated(String),
    #[error("não foi possível iniciar o instalador: {0}")]
    Launch(String),
    #[error(transparent)]
    Args(#[from] ArgsError),
    #[error("{0}")]
    Queue(String),
    #[error("{message}")]
    Failed {
        category: Category,
        code: u32,
        message: String,
    },
    #[error("E/S: {0}")]
    Io(String),
}

fn phase(ctx: &WingetCtx<'_>, p: Phase) {
    (ctx.progress)(&ProgressEvent::Phase(p));
}

/// True if `name` is exactly one safe path component.
fn one_component(name: &str) -> bool {
    safe_components(name).is_some_and(|c| c.len() == 1 && c[0] == name)
}

/// `ver` with every byte outside `[A-Za-z0-9._+-]` replaced by `_` (as the installer folder).
fn sanitize(ver: &str) -> String {
    ver.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._+-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `<root>\log\installers\<id>-<ver>-<ms>.log`; the name goes through `safepath`.
fn installer_log(data: &DataDir, plan: &WingetPlan) -> Result<PathBuf, WingetOpsError> {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let name = format!("{}-{}-{ms}.log", plan.id, sanitize(&plan.version));
    if !one_component(&name) {
        return Err(ArgsError::Invalid("nome do log do instalador inválido").into());
    }
    Ok(data.log_dir().join("installers").join(name))
}

/// Takes `install.lock` (held for a whole flow; `install_locked` runs under it).
pub async fn lock_install(layout: &Layout, wait: LockWait) -> Result<UpdateLock, WingetOpsError> {
    let path = layout.install_lock();
    let timeout = match wait {
        LockWait::Block(d) => d,
        LockWait::NoWait => Duration::ZERO,
    };
    let handle = tokio::task::spawn_blocking(move || UpdateLock::acquire_timeout(&path, timeout));
    let r = match handle.await {
        Ok(r) => r,
        Err(e) => match e.try_into_panic() {
            Ok(p) => std::panic::resume_unwind(p),
            Err(e) => return Err(WingetOpsError::Lock(e.to_string())),
        },
    };
    r.map_err(|e| WingetOpsError::Lock(e.to_string()))?
        .ok_or(WingetOpsError::Busy)
}

/// One slice of the `install.lock` wait of [`lock_install_for`] (the gate is read between).
const LOCK_SLICE: Duration = Duration::from_millis(200);

/// [`lock_install`] that gives up once `gate` is cancelled (Ctrl+C before anything launched).
/// It waits in short slices, each one awaited: a cancel never leaves a thread waiting for the
/// lock (it would take it after the user was told "Cancelado.") nor a handle unawaited; a lock
/// taken while the cancel arrived is let go at once.
pub async fn lock_install_for(
    layout: &Layout,
    wait: LockWait,
    gate: &LaunchGate,
) -> Result<UpdateLock, WingetOpsError> {
    let path = layout.install_lock();
    let total = match wait {
        LockWait::Block(d) => d,
        LockWait::NoWait => Duration::ZERO,
    };
    let start = std::time::Instant::now();
    loop {
        if gate.cancelled() {
            return Err(WingetOpsError::Cancelled);
        }
        let slice = total.saturating_sub(start.elapsed()).min(LOCK_SLICE);
        let p = path.clone();
        let handle = tokio::task::spawn_blocking(move || UpdateLock::acquire_timeout(&p, slice));
        let got = match handle.await {
            Ok(r) => r.map_err(|e| WingetOpsError::Lock(e.to_string()))?,
            Err(e) => match e.try_into_panic() {
                Ok(p) => std::panic::resume_unwind(p),
                Err(e) => return Err(WingetOpsError::Lock(e.to_string())),
            },
        };
        match got {
            Some(_) if gate.cancelled() => return Err(WingetOpsError::Cancelled),
            Some(lock) => return Ok(lock),
            None if start.elapsed() >= total => return Err(WingetOpsError::Busy),
            None => {}
        }
    }
}

/// Hashes the materialized file again through a handle that denies write, delete and rename
/// until `file` is dropped.
#[cfg(windows)]
fn verify(file: &mut Materialized) -> Result<(), FetchError> {
    file.verify().map(|_| ())
}

#[cfg(not(windows))]
fn verify(_file: &mut Materialized) -> Result<(), FetchError> {
    Err(FetchError::Io(
        "instaladores nativos só rodam no Windows".into(),
    ))
}

/// Installer types whose runs go through the Windows Installer queue (1618 retried).
fn queued(kind: &InstallerKind) -> bool {
    matches!(
        kind,
        InstallerKind::Msi | InstallerKind::Wix | InstallerKind::Burn
    )
}

/// One launch of the installer.
type RunOnce<'a> = dyn FnMut() -> Result<i32, WingetOpsError> + 'a;

/// One finished run of a command-line installer.
struct Ran {
    code: i32,
    log: Option<PathBuf>,
    /// End of the installer log (for `msiexec_log::failure_hint`).
    tail: String,
}

/// Downloads, verifies and runs the plan's installer, then looks for the installed app.
///
/// Blocks its thread while the installer runs (locks, launcher, helper, package manager are
/// synchronous): call it only from a dedicated thread or `block_on` there — the TUI runs jobs
/// in `spawn_blocking` + `Handle::block_on`, the CLI with `block_on` — never from a task
/// sharing the runtime's worker threads.
pub async fn install(
    ctx: &WingetCtx<'_>,
    plan: &WingetPlan,
) -> Result<InstallReport, WingetOpsError> {
    let lock = lock_install_for(&ctx.data.install_layout(), ctx.wait, &ctx.gate).await?;
    install_locked(ctx, plan, &lock).await
}

/// [`install`] under an `install.lock` the caller already holds (a removal and an install that
/// must not be interleaved with another operation).
pub async fn install_locked(
    ctx: &WingetCtx<'_>,
    plan: &WingetPlan,
    install_lock: &UpdateLock,
) -> Result<InstallReport, WingetOpsError> {
    let file = fetch_verified(ctx, plan).await?;
    install_fetched(ctx, plan, file, install_lock)
}

/// Downloads the plan's installer and hashes it again through a handle that denies write,
/// delete and rename while the returned file lives (nothing is launched).
pub async fn fetch_verified(
    ctx: &WingetCtx<'_>,
    plan: &WingetPlan,
) -> Result<Materialized, WingetOpsError> {
    phase(ctx, Phase::Preparing);
    // Ctrl+C before the launch stops the download between chunks (the partial file deleted).
    let gate = &ctx.gate;
    let mut file = match fetch_installer_until(
        ctx.http,
        ctx.data,
        &plan.id,
        &plan.version,
        &plan.installer,
        ctx.progress,
        &|| gate.cancelled(),
    )
    .await
    {
        Err(FetchError::Download(DownloadError::Cancelled)) => {
            return Err(WingetOpsError::Cancelled);
        }
        other => other?,
    };
    verify(&mut file)?;
    Ok(file)
}

/// The rest of [`install_locked`] for a file [`fetch_verified`] returned: `msi.lock`, launch
/// gate, launch, classification, post-check and record.
pub fn install_fetched(
    ctx: &WingetCtx<'_>,
    plan: &WingetPlan,
    mut file: Materialized,
    _install_lock: &UpdateLock,
) -> Result<InstallReport, WingetOpsError> {
    let layout = ctx.data.install_layout();
    let (outcome, log, extra) = match &plan.command {
        Some(cmd) => {
            let _msi_lock = msi_queue::lock_msi(&layout.msi_lock(), &mut |_| {
                phase(ctx, Phase::WaitingInstaller)
            })
            .map_err(|e| WingetOpsError::Lock(e.to_string()))?;
            let ran = run_command(ctx, plan, cmd, &file);
            // Only now, with the installer process ended, may the file change on disk.
            file.release();
            let ran = ran?;
            let outcome = codes::classify_install(
                &plan.installer.kind,
                ran.code,
                &plan.installer.success_codes,
                &plan.installer.expected_codes,
            );
            if let Outcome::Failed {
                category,
                code,
                message,
            } = outcome
            {
                let hint = (cmd.program == Program::Msiexec)
                    .then(|| crate::install::msiexec_log::failure_hint(ran.code, &ran.tail))
                    .flatten();
                let message = match hint {
                    Some(h) => format!("{message} ({h})"),
                    None => message,
                };
                return Err(WingetOpsError::Failed {
                    category,
                    code,
                    message,
                });
            }
            (outcome, ran.log, None)
        }
        None => {
            if !ctx.gate.try_launch() {
                return Err(WingetOpsError::Cancelled);
            }
            phase(
                ctx,
                Phase::RunningInstaller {
                    name: plan.name.clone(),
                },
            );
            match ctx.msix.add(file.path()) {
                Ok(r) => (Outcome::Ok, None, r.user_text().map(str::to_string)),
                Err(e) => match msix::outcome(&e) {
                    Outcome::Failed { category, code, .. } => {
                        return Err(WingetOpsError::Failed {
                            category,
                            code,
                            message: format!("{}: {e}", codes::message(category)),
                        });
                    }
                    ok => (ok, None, None),
                },
            }
        }
    };

    phase(ctx, Phase::Verifying);
    let apps = (ctx.scan)();
    let found = find_installed(plan, &apps).cloned();
    let warning = match (&found, extra) {
        (_, Some(text)) => Some(text),
        (None, None) => Some(NOT_FOUND_WARNING.to_string()),
        (Some(_), None) => None,
    };
    if let Err(e) = record(&layout.state_db(), plan, file.sha256(), found.as_ref()) {
        tracing::warn!(error = %e, "winget_ops: registro não gravado");
    }
    Ok(InstallReport {
        outcome,
        found,
        warning,
        log,
    })
}

/// Runs a command-line installer: unelevated (`Never`) with the log filled in, or elevated
/// through its own UAC prompt (`Helper`: `runas` on the verified file, or on the absolute
/// `msiexec` for msi/wix, with parameters the elevation allowlist accepts and no log). The caller holds the
/// verified file's share-deny handle for the whole run. Windows Installer types wait for the
/// mutex and retry 1618.
fn run_command(
    ctx: &WingetCtx<'_>,
    plan: &WingetPlan,
    cmd: &CommandLine,
    file: &Materialized,
) -> Result<Ran, WingetOpsError> {
    let verb = match plan.elevation {
        Elevation::Helper => {
            elevation_allowed(plan, cmd, file)?;
            Verb::RunAs
        }
        Elevation::Never => Verb::Open,
    };
    // An elevated installer gets no log (Task 196): a log path in a folder the user controls
    // would let a junction send an admin write anywhere. Its exit code says enough.
    let (log, args) = match verb {
        Verb::RunAs => (None, cmd.args.clone()),
        Verb::Open => {
            let log = installer_log(ctx.data, plan)?;
            if let Some(dir) = log.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| WingetOpsError::Io(format!("{}: {e}", dir.display())))?;
            }
            let args = switches::fill_log(&cmd.args, &log)?;
            (Some(log), args)
        }
    };
    // Elevated: the held file's final path (no folder above it is a reparse point, all held)
    // and System32 as the working folder, never the user's download folder.
    // `_run` (the fresh run folder holding only the copy) lives until the installer ended.
    let (target, dir, _run) = match verb {
        Verb::RunAs => {
            let (copy, run) = elevated_copy(ctx, file)?;
            (copy, ctx.msiexec.parent().map(Path::to_path_buf), Some(run))
        }
        Verb::Open => (
            file.path().to_path_buf(),
            file.path().parent().map(Path::to_path_buf),
            None,
        ),
    };
    let (program, params) = match cmd.program {
        Program::Msiexec => (
            ctx.msiexec.clone(),
            switches::msiexec_params(&target, &args),
        ),
        Program::Installer => (target, args),
    };
    switches::validate(&params)?;
    let spec = LaunchSpec {
        program,
        params,
        dir,
        show: Show::Normal,
        verb,
    };
    let name = plan.name.clone();
    let mut once: Box<RunOnce<'_>> = Box::new(move || {
        let running = Phase::RunningInstaller { name: name.clone() };
        let launched = match verb {
            // The UAC prompt first; the installer's phase once its process exists.
            Verb::RunAs => {
                phase(ctx, Phase::WaitingElevation);
                ctx.launcher
                    .run_started(&spec, &mut || phase(ctx, running.clone()))
            }
            Verb::Open => {
                phase(ctx, running);
                ctx.launcher.run(&spec)
            }
        };
        launched.map_err(|e| {
            if uac_refused(&e) {
                WingetOpsError::UacRefused
            } else {
                WingetOpsError::Launch(e.to_string())
            }
        })
    });

    // The gate is taken right before each launch: a cancel before the first one wins.
    let mut attempt = || -> Result<i32, WingetOpsError> {
        if !ctx.gate.try_launch() {
            return Err(WingetOpsError::Cancelled);
        }
        once()
    };

    let code = if queued(&plan.installer.kind) {
        match retry_1618(
            ctx.busy,
            ctx.sleep,
            &mut || phase(ctx, Phase::WaitingInstaller),
            &mut attempt,
        ) {
            Ok(c) => c,
            Err(Queued::Attempt(e)) => return Err(e),
            Err(Queued::Queue(text)) => return Err(WingetOpsError::Queue(text)),
        }
    } else {
        attempt()?
    };

    let tail = log.as_deref().map(read_tail).unwrap_or_default();
    Ok(Ran { code, log, tail })
}

/// A copy of the verified installer alone in a fresh `installers\run-<hex>\` folder, re-hashed
/// and held (Task 196): what the elevated launch runs; the folder goes when the guard drops.
#[cfg(windows)]
fn elevated_copy(
    ctx: &WingetCtx<'_>,
    file: &Materialized,
) -> Result<(PathBuf, super::verified_win::RunCopy), WingetOpsError> {
    let run = super::verified_win::RunCopy::create(ctx.data.root(), file.path(), file.sha256())?;
    Ok((run.final_path().to_path_buf(), run))
}

#[cfg(not(windows))]
fn elevated_copy(
    _ctx: &WingetCtx<'_>,
    _file: &Materialized,
) -> Result<(PathBuf, ()), WingetOpsError> {
    Err(WingetOpsError::Io(
        "instaladores elevados só rodam no Windows".into(),
    ))
}

/// The elevation allowlist (the rules the helper applied before Task 196) on what an elevated
/// launch runs: the installer type, the verified file's SHA-256 and the parameters.
fn elevation_allowed(
    plan: &WingetPlan,
    cmd: &CommandLine,
    file: &Materialized,
) -> Result<(), WingetOpsError> {
    let kind = elevated_kind(&plan.installer.kind).ok_or_else(|| {
        WingetOpsError::Elevated("tipo de instalador que não roda elevado".into())
    })?;
    if cmd.args.contains(switches::LOG_TOKEN) {
        return Err(WingetOpsError::Elevated(
            "o instalador elevado não grava log (elevação)".into(),
        ));
    }
    // `fetch` already required 64 hex digits.
    let sha = file.sha256();
    let req = InstallerRequest {
        // The download cache's name for the file (the shape the allowlist expects).
        source: format!(r"C:\wingetrs\blobs\{sha}"),
        sha256: sha.to_string(),
        file_name: format!("installer.{}", if kind.is_msi() { "msi" } else { "exe" }),
        kind,
        params: cmd.args.clone(),
    };
    req.check(&[])
        .map_err(|reason| WingetOpsError::Elevated(format!("{reason} (elevação)")))
}
/// `ERROR_CANCELLED`: the user said no to the UAC prompt of a program that elevates itself.
const ERROR_CANCELLED: i32 = 1223;

/// Whether a launch failed because the user refused the UAC prompt (`launch_win` reports
/// `ERROR_CANCELLED` as that raw OS error).
pub fn uac_refused(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(ERROR_CANCELLED)
}

/// Why [`retry_1618`] gave no exit code.
pub enum Queued<E> {
    /// The attempt itself failed (launch, UAC, helper).
    Attempt(E),
    /// The queue gave up (mutex held too long, I/O).
    Queue(String),
}

/// Runs `attempt` through the Windows Installer queue (installs and `msiexec /x` alike): waits
/// while the mutex is held, retries 1618, and returns the last exit code (a 1618 after the
/// last retry included, for the caller to classify). `waiting` is told each time it waits.
pub fn retry_1618<E: std::fmt::Display>(
    busy: &dyn Fn() -> bool,
    sleep: &dyn Fn(Duration),
    waiting: &mut dyn FnMut(),
    attempt: &mut dyn FnMut() -> Result<i32, E>,
) -> Result<i32, Queued<E>> {
    // Per-attempt state, reset at the start of every attempt.
    let mut last: Option<Result<i32, E>> = None;
    let r = msi_queue::run_queued(
        &msi_queue::POLICY,
        &mut || busy(),
        &mut || {
            last = None;
            match attempt() {
                Ok(1618) => {
                    last = Some(Ok(1618));
                    Ok(Attempt::Busy1618)
                }
                Ok(c) => {
                    last = Some(Ok(c));
                    Ok(Attempt::Done)
                }
                Err(e) => {
                    let text = e.to_string();
                    last = Some(Err(e));
                    Err(std::io::Error::other(text))
                }
            }
        },
        &mut |d| sleep(d),
        &mut |s| match s {
            Status::WaitingLock | Status::WaitingInstaller | Status::Retrying { .. } => waiting(),
            // The launch itself reports its phase.
            Status::Running { .. } => {}
        },
    );
    match (r, last) {
        (_, Some(Err(e))) => Err(Queued::Attempt(e)),
        // Done, or 1618 after the last retry: classified by the caller like any other code.
        (Ok(()) | Err(MsiQueueError::InstallerBusy), Some(Ok(c))) => Ok(c),
        (Err(e), _) => Err(Queued::Queue(e.to_string())),
        (Ok(()), None) => Err(Queued::Queue("o instalador não rodou".into())),
    }
}

/// The end of the installer log, empty when unreadable.
fn read_tail(path: &Path) -> String {
    std::fs::File::open(path)
        .and_then(|mut f| {
            crate::install::msiexec_log::tail_text(
                &mut f,
                crate::install::msiexec_log::ERROR_WINDOW,
            )
        })
        .unwrap_or_default()
}

/// One `winget_ops` row in `state.db` (a hint for later; nothing decides on it).
fn record(
    db: &Path,
    plan: &WingetPlan,
    sha256: &str,
    found: Option<&InstalledApp>,
) -> rusqlite::Result<()> {
    let conn = rusqlite::Connection::open(db)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS winget_ops(id TEXT, version TEXT, installer_type TEXT, \
         scope TEXT, sha256 TEXT, arp_key TEXT, at INTEGER)",
    )?;
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    conn.execute(
        "INSERT INTO winget_ops(id, version, installer_type, scope, sha256, arp_key, at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            plan.id,
            plan.version,
            plan.installer.kind.name(),
            plan.installer.scope,
            sha256,
            found.map(|a| a.key.as_str()),
            at
        ],
    )?;
    Ok(())
}
