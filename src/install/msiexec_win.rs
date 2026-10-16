//! msiexec /a launcher and installer mutex probe (phase 2b, Task 127).
use super::msi_queue::{Attempt, attempt_from_code};
use std::io;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND};
use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
use windows::core::w;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLL: Duration = Duration::from_millis(250);
/// After this long the run is still awaited (killing the client does not cancel the
/// service's install), with a warning in the log every `WARN_EVERY`.
const WARN_AFTER: Duration = Duration::from_secs(600);
const WARN_EVERY: Duration = Duration::from_secs(60);

/// Whether another Windows Installer run holds `Global\_MSIExecute`. Only opens the mutex,
/// never creates it.
pub fn installer_busy() -> bool {
    // SAFETY: a constant NUL-terminated name; the handle is closed right after.
    match unsafe {
        OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE,
            false,
            w!("Global\\_MSIExecute"),
        )
    } {
        Ok(handle) => {
            // SAFETY: `handle` was just opened by this call and is closed once.
            if let Err(e) = unsafe { CloseHandle(handle) } {
                tracing::warn!("CloseHandle(_MSIExecute): {e}");
            }
            true
        }
        // It exists, but this user may not open it.
        Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => true,
        // `ERROR_FILE_NOT_FOUND`: no installer running. Anything else: let msiexec decide
        // (a busy installer still comes back as 1618).
        Err(e) => {
            if e.code() != ERROR_FILE_NOT_FOUND.to_hresult() {
                tracing::warn!("OpenMutexW(_MSIExecute): {e}");
            }
            false
        }
    }
}

/// `msiexec.exe /a "<msi>" /qn TARGETDIR="<target>" /l*v "<log>"`, awaited without ever
/// killing it.
pub fn msiexec_admin(msi: &Path, target_dir: &Path, log: &Path) -> io::Result<Attempt> {
    // msiexec parses its own command line: quote the paths verbatim.
    let args = format!(
        r#"/a "{}" /qn TARGETDIR="{}" /l*v "{}""#,
        msi.display(),
        target_dir.display(),
        log.display()
    );
    let mut child = Command::new("msiexec.exe")
        .raw_arg(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    let start = Instant::now();
    let mut next_warn = WARN_AFTER;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        let elapsed = start.elapsed();
        if elapsed >= next_warn {
            tracing::warn!(
                msi = %msi.display(),
                secs = elapsed.as_secs(),
                "msiexec /a still running; waiting (it is never killed)"
            );
            next_warn += WARN_EVERY;
        }
        std::thread::sleep(POLL);
    };
    let code = status.code().unwrap_or(-1);
    if code == 3010 {
        tracing::warn!(msi = %msi.display(), "msiexec /a asked for a reboot (3010)");
    }
    Ok(attempt_from_code(code))
}
