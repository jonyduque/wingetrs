//! Windows Installer queue policy (phase 2b, Task 127).
//!
//! One MSI runs at a time on the machine (`Global\_MSIExecute`); a second `msiexec` exits with
//! 1618. Inside wingetrs, `msi.lock` orders our own MSI runs (and bridge winget jobs); against
//! other programs the queue polls the mutex and retries 1618 with a backoff.
use crate::cache::UpdateLock;
use std::io;
use std::path::Path;
use std::time::Duration;

/// Outcome of one `msiexec` run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    Done,
    /// `ERROR_INSTALL_ALREADY_RUNNING`: another installer holds the mutex.
    Busy1618,
    Failed(i32),
}

#[derive(Debug, Clone)]
pub struct Policy {
    pub poll: Duration,
    pub wait_max: Duration,
    pub retries: [Duration; 3],
}

/// poll 2 s, wait_max 600 s, retries 5 s, 15 s, 45 s.
pub const POLICY: Policy = Policy {
    poll: Duration::from_secs(2),
    wait_max: Duration::from_secs(600),
    retries: [
        Duration::from_secs(5),
        Duration::from_secs(15),
        Duration::from_secs(45),
    ],
};

/// State reported while queued or running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    WaitingLock,
    WaitingInstaller,
    Running { attempt: u8 },
    Retrying { in_secs: u64 },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MsiQueueError {
    #[error("o Windows Installer está ocupado por outro programa")]
    InstallerBusy,
    #[error("o msiexec terminou com código {0}")]
    Failed(i32),
    #[error("{0}")]
    Io(String),
}

/// `msiexec` exit code: 0 and 3010 (reboot required) succeed, 1618 is a busy installer.
pub fn attempt_from_code(code: i32) -> Attempt {
    match code {
        0 | 3010 => Attempt::Done,
        1618 => Attempt::Busy1618,
        other => Attempt::Failed(other),
    }
}

/// Command line of `msiexec /a`: `/a "<msi>" /qn TARGETDIR="<target>" /l*v "<log>"`. Trailing
/// separators are stripped from the target, since a trailing `\` would escape the closing quote.
pub fn admin_args(msi: &Path, target_dir: &Path, log: &Path) -> String {
    let target = target_dir.to_string_lossy();
    format!(
        r#"/a "{}" /qn TARGETDIR="{}" /l*v "{}""#,
        msi.display(),
        target.trim_end_matches(['\\', '/']),
        log.display()
    )
}

/// Waits while `busy()` (Global\_MSIExecute), retries 1618 per `policy`, reports each state.
pub fn run_queued(
    policy: &Policy,
    busy: &mut dyn FnMut() -> bool,
    launch: &mut dyn FnMut() -> io::Result<Attempt>,
    sleep: &mut dyn FnMut(Duration),
    status: &mut dyn FnMut(Status),
) -> Result<(), MsiQueueError> {
    let max_checks = policy.wait_max.as_millis() / policy.poll.as_millis().max(1);
    let mut retries = policy.retries.iter();
    // `wait_max` is a total budget across retries, not one per attempt.
    let mut checks = 0u128;
    for attempt in 1u8.. {
        let mut reported = false;
        while busy() {
            checks += 1;
            if checks >= max_checks {
                return Err(MsiQueueError::InstallerBusy);
            }
            if !reported {
                status(Status::WaitingInstaller);
                reported = true;
            }
            sleep(policy.poll);
        }
        status(Status::Running { attempt });
        match launch().map_err(|e| MsiQueueError::Io(e.to_string()))? {
            Attempt::Done => return Ok(()),
            Attempt::Failed(code) => return Err(MsiQueueError::Failed(code)),
            Attempt::Busy1618 => {}
        }
        let Some(&delay) = retries.next() else {
            return Err(MsiQueueError::InstallerBusy);
        };
        status(Status::Retrying {
            in_secs: delay.as_secs(),
        });
        sleep(delay);
    }
    Err(MsiQueueError::InstallerBusy)
}

/// Takes `msi.lock` (same file lock as `install.lock`), waiting with no limit; reports
/// `WaitingLock` once when another holder makes it wait.
pub fn lock_msi(path: &Path, status: &mut dyn FnMut(Status)) -> Result<UpdateLock, MsiQueueError> {
    let io = |e: crate::cache::CacheError| MsiQueueError::Io(e.to_string());
    if let Some(lock) = UpdateLock::try_acquire(path).map_err(io)? {
        return Ok(lock);
    }
    status(Status::WaitingLock);
    UpdateLock::acquire_timeout(path, Duration::MAX)
        .map_err(io)?
        .ok_or_else(|| MsiQueueError::Io(format!("{}: trava não obtida", path.display())))
}
