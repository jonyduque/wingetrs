//! One native removal sequence for the CLI and the TUI (Task 193): `install.lock`, the copy
//! of the shared data folders, the launch gate, the phase, then the uninstaller (queued on
//! `msi.lock`) or the registry-only removal.
use crate::install::executor::LaunchGate;
use crate::install::progress::{Phase, ProgressEvent};
use crate::installed::Hive;
use crate::winget_install::launch::Elevation;
use crate::winget_install::ops::WingetOpsError;
use crate::winget_install::uninstall::{
    BackupLimits, MsiQueue, UninstallCtx, UninstallPlan, UninstallReport, UninstallTarget,
    backup_folders, uninstall_queued,
};
use std::future::Future;
use std::path::{Path, PathBuf};

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

/// What a removal did.
#[derive(Debug)]
pub enum Removed {
    /// The uninstaller ran (`copied`: where the folders went).
    App {
        report: UninstallReport,
        copied: Option<PathBuf>,
    },
    /// Only the record went; its `.reg` export.
    Record(PathBuf),
}

/// Why a removal did not happen.
#[derive(Debug)]
pub enum RemovalError {
    /// `install.lock` (`Busy`: another operation of wingetrs holds it).
    Lock(WingetOpsError),
    /// Ctrl+C (or a confirmed quit) before anything was launched.
    Cancelled,
    /// The copy, the uninstaller or the record removal failed (text for the user).
    Failed(String),
}

impl std::fmt::Display for RemovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemovalError::Lock(e) => write!(f, "{e}"),
            RemovalError::Cancelled => write!(f, "{}", WingetOpsError::Cancelled),
            RemovalError::Failed(m) => write!(f, "{m}"),
        }
    }
}

/// What the removal reports to and takes from its caller.
pub struct Steps<'a> {
    pub gate: &'a LaunchGate,
    pub progress: &'a dyn Fn(&ProgressEvent),
    /// `Layout::backups` (the folder copy goes to `<backups>\<stamp>-pastas`).
    pub backups: &'a Path,
    pub stamp: &'a str,
    pub free_space: &'a dyn Fn(&Path) -> Option<u64>,
}

/// The phase of a record removal once launched: the UAC (and the elevated helper) or the
/// removal itself (orphans and the registry-only removal).
pub fn removal_phase(elevated: bool, name: &str) -> Phase {
    if elevated {
        Phase::WaitingElevation
    } else {
        Phase::Removing {
            app: name.to_string(),
        }
    }
}

/// The phase while the uninstaller of `plan` runs (the elevated helper: its UAC first).
pub fn uninstaller_phase(plan: &UninstallPlan) -> Phase {
    if plan.elevation == Elevation::Helper {
        Phase::WaitingElevation
    } else {
        Phase::Uninstalling {
            name: plan.name.clone(),
        }
    }
}

/// Runs a confirmed removal of `plan`, the same in the CLI and the TUI:
/// 1. `Preparando` while `lock` (`install.lock`) is awaited (never dropped while waiting);
/// 2. with [`RemovalChoice::BackupThenRemove`] and folders to copy, `Copiando as pastas…`
///    (a refused copy removes nothing);
/// 3. the launch gate (a cancel before this point removes nothing) and the running phase;
/// 4. `uninstall` (given the progress, for [`Phase::CheckingRemoval`]) or `registry_only`.
pub async fn remove<L>(
    s: &Steps<'_>,
    lock: impl Future<Output = Result<L, WingetOpsError>>,
    plan: &UninstallPlan,
    how: RemovalChoice,
    uninstall: impl FnOnce(&dyn Fn(&ProgressEvent)) -> Result<UninstallReport, String>,
    registry_only: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<Removed, RemovalError> {
    let phase = |p: Phase| (s.progress)(&ProgressEvent::Phase(p));
    phase(Phase::Preparing);
    let _held = match lock.await {
        Ok(l) => l,
        Err(WingetOpsError::Cancelled) => return Err(RemovalError::Cancelled),
        Err(e) => return Err(RemovalError::Lock(e)),
    };
    let mut copied = None;
    if how == RemovalChoice::BackupThenRemove
        && let Some(shared) = &plan.shared
        && !shared.folders.is_empty()
    {
        if s.gate.cancelled() {
            return Err(RemovalError::Cancelled);
        }
        phase(Phase::CopyingFolders);
        let dest = s.backups.join(format!("{}-pastas", s.stamp));
        let limits = BackupLimits::standard(s.free_space);
        let notes = backup_folders(&shared.folders, &dest, &limits).map_err(|e| {
            RemovalError::Failed(format!(
                "{}: cópia das pastas recusada ({e}); nada foi removido",
                plan.name
            ))
        })?;
        for n in &notes {
            tracing::warn!(app = %plan.name, "backup: {n}");
        }
        copied = Some(dest);
    }
    let failed = |e: String| RemovalError::Failed(format!("{}: {e}", plan.name));
    match how {
        RemovalChoice::RegistryOnly => {
            if !s.gate.try_launch_for("remoção do registro") {
                return Err(RemovalError::Cancelled);
            }
            let hklm =
                matches!(&plan.target, UninstallTarget::Arp { hive, .. } if *hive != Hive::User);
            phase(removal_phase(hklm, &plan.name));
            registry_only().map(Removed::Record).map_err(failed)
        }
        RemovalChoice::Remove | RemovalChoice::BackupThenRemove => {
            if !s.gate.try_launch_for("desinstalador") {
                return Err(RemovalError::Cancelled);
            }
            phase(uninstaller_phase(plan));
            let report = uninstall(s.progress).map_err(failed)?;
            Ok(Removed::App { report, copied })
        }
    }
}

/// The native uninstall on the calling (blocking) thread: queued on `msi.lock` (an
/// `msiexec /x` waits for another installer and retries 1618), announcing
/// [`Phase::CheckingRemoval`] once the uninstaller ended and the record is being watched.
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
            progress(&ProgressEvent::Phase(Phase::CheckingRemoval {
                name: plan.name.clone(),
            }));
        }
        (u.key_exists)(hive, key)
    };
    let watched = UninstallCtx {
        key_exists: &key_watched,
        ..*u
    };
    uninstall_queued(&watched, Some(queue), plan)
}
