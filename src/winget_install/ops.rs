//! Native install flow (Task 181): download, verify, launch (or elevate), classify the exit
//! code and look for the app among the installed ones.
//!
//! Order: `install.lock` → download → hash again through a share-deny handle → `msi.lock`
//! (command-line installers) → launch gate → launch → classify → post-check → record.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::codes::{self, Category, Outcome, UAC_REFUSED};
use super::fetch::{FetchError, Materialized, fetch_installer};
use super::launch::{Elevation, LaunchSpec, Launcher, Show};
use super::msix::{self, MsixOps};
use super::plan::{WingetPlan, elevated_kind};
use super::post_check::find_installed;
use super::switches::{self, ArgsError, CommandLine, Program};
use crate::bridge::elevated::{ElevatedError, ElevatedRunner};
use crate::bridge::worker::InstallerRequest;
use crate::cache::{DataDir, UpdateLock};
use crate::http::Http;
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
    /// The installer's log, when it wrote one (or the lines the elevated helper sent back).
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

/// One launch of the installer; gets the helper's log lines (cleared per attempt).
type RunOnce<'a> = dyn FnMut(&mut Vec<String>) -> Result<i32, WingetOpsError> + 'a;

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
    let lock = lock_install(&ctx.data.install_layout(), ctx.wait).await?;
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
    let mut file = fetch_installer(
        ctx.http,
        ctx.data,
        &plan.id,
        &plan.version,
        &plan.installer,
        ctx.progress,
    )
    .await?;
    verify(&mut file)?;
    Ok(file)
}

/// The rest of [`install_locked`] for a file [`fetch_verified`] returned: `msi.lock`, launch
/// gate, launch, classification, post-check and record.
pub fn install_fetched(
    ctx: &WingetCtx<'_>,
    plan: &WingetPlan,
    file: Materialized,
    _install_lock: &UpdateLock,
) -> Result<InstallReport, WingetOpsError> {
    let layout = ctx.data.install_layout();
    let (outcome, log, extra) = match &plan.command {
        Some(cmd) => {
            let _msi_lock = msi_queue::lock_msi(&layout.msi_lock(), &mut |_| {
                phase(ctx, Phase::WaitingInstaller)
            })
            .map_err(|e| WingetOpsError::Lock(e.to_string()))?;
            let ran = run_command(ctx, plan, cmd, &file)?;
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

/// Runs a command-line installer: through the elevated helper (`Helper`) with the blob as
/// source and `<LOGPATH>` left for it, or unelevated with the log filled in. Windows
/// Installer types wait for the mutex and retry 1618.
fn run_command(
    ctx: &WingetCtx<'_>,
    plan: &WingetPlan,
    cmd: &CommandLine,
    file: &Materialized,
) -> Result<Ran, WingetOpsError> {
    let name = plan.name.clone();
    let mut lines: Vec<String> = Vec::new();
    let (log, mut once): (Option<PathBuf>, Box<RunOnce<'_>>) = match plan.elevation {
        Elevation::Helper => {
            let kind = elevated_kind(&plan.installer.kind).ok_or_else(|| {
                WingetOpsError::Elevated("tipo de instalador sem auxiliar elevado".into())
            })?;
            let sha = file.sha256();
            let file_name = file
                .path()
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !one_component(sha) {
                return Err(FetchError::BadSha.into());
            }
            let req = InstallerRequest {
                source: ctx
                    .data
                    .blobs_dir()
                    .join(sha)
                    .to_string_lossy()
                    .into_owned(),
                sha256: sha.to_string(),
                file_name,
                kind,
                params: cmd.args.clone(),
            };
            let run = move |lines: &mut Vec<String>| -> Result<i32, WingetOpsError> {
                phase(ctx, Phase::WaitingElevation);
                lines.clear();
                ctx.elevated
                    .run_installer(&req, &mut |l| lines.push(l.to_string()))
                    .map_err(|e| match e {
                        ElevatedError::Refused => WingetOpsError::UacRefused,
                        other => WingetOpsError::Elevated(other.to_string()),
                    })
            };
            (installer_log(ctx.data, plan).ok(), Box::new(run))
        }
        Elevation::Never => {
            let log = installer_log(ctx.data, plan)?;
            if let Some(dir) = log.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| WingetOpsError::Io(format!("{}: {e}", dir.display())))?;
            }
            let args = switches::fill_log(&cmd.args, &log)?;
            let (program, params) = match cmd.program {
                Program::Msiexec => (
                    ctx.msiexec.clone(),
                    switches::msiexec_params(file.path(), &args),
                ),
                Program::Installer => (file.path().to_path_buf(), args),
            };
            switches::validate(&params)?;
            let spec = LaunchSpec {
                program,
                params,
                dir: file.path().parent().map(Path::to_path_buf),
                show: Show::Normal,
            };
            let name = name.clone();
            let run = move |_: &mut Vec<String>| -> Result<i32, WingetOpsError> {
                phase(ctx, Phase::RunningInstaller { name: name.clone() });
                ctx.launcher.run(&spec).map_err(|e| {
                    if uac_refused(&e) {
                        WingetOpsError::UacRefused
                    } else {
                        WingetOpsError::Launch(e.to_string())
                    }
                })
            };
            (Some(log), Box::new(run))
        }
    };

    // The gate is taken right before each launch: a cancel before the first one wins.
    let mut attempt = |lines: &mut Vec<String>| -> Result<i32, WingetOpsError> {
        if !ctx.gate.try_launch() {
            return Err(WingetOpsError::Cancelled);
        }
        once(lines)
    };

    let code = if queued(&plan.installer.kind) {
        match retry_1618(
            ctx.busy,
            ctx.sleep,
            &mut || phase(ctx, Phase::WaitingInstaller),
            &mut || attempt(&mut lines),
        ) {
            Ok(c) => c,
            Err(Queued::Attempt(e)) => return Err(e),
            Err(Queued::Queue(text)) => return Err(WingetOpsError::Queue(text)),
        }
    } else {
        attempt(&mut lines)?
    };

    let (log, tail) = match plan.elevation {
        Elevation::Helper => {
            let tail = lines.join("\n");
            let log = log.filter(|p| write_lines(p, &tail));
            (log, tail)
        }
        Elevation::Never => {
            let tail = log.as_deref().map(read_tail).unwrap_or_default();
            (log, tail)
        }
    };
    Ok(Ran { code, log, tail })
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

/// Saves the log lines sent back by the elevated helper; whether it worked.
fn write_lines(path: &Path, text: &str) -> bool {
    let r = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(path, text));
    if let Err(e) = &r {
        tracing::warn!(error = %e, "log do instalador não gravado: {}", path.display());
    }
    r.is_ok()
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
