//! Production wiring of the native install flow (Task 181).

use std::time::Duration;

use super::launch_win::{ShellLauncher, msiexec_path};
use super::msix_win::SystemMsix;
use super::ops::WingetCtx;
use crate::bridge::elevated::ElevatedRunner;
use crate::cache::DataDir;
use crate::http::Http;
use crate::install::download::Progress;
use crate::install::executor::LaunchGate;
use crate::installed::InstalledApp;
use crate::update::LockWait;

fn scan_apps() -> Vec<InstalledApp> {
    crate::installed::scan_all().apps
}

fn busy() -> bool {
    crate::install::msiexec_win::installer_busy()
}

fn sleep(d: Duration) {
    std::thread::sleep(d);
}

/// The context with the real launcher (`ShellExecuteExW`), package manager, installed-app
/// scan, `System32\msiexec.exe` and Windows Installer mutex; `elevated` is the session's
/// helper (`bridge::elevated::ElevatedBridge`).
pub fn build<'a>(
    data: &'a DataDir,
    http: &'a Http,
    wait: LockWait,
    progress: Progress<'a>,
    gate: LaunchGate,
    elevated: &'a dyn ElevatedRunner,
) -> std::io::Result<WingetCtx<'a>> {
    Ok(WingetCtx {
        data,
        http,
        wait,
        progress,
        gate,
        launcher: &ShellLauncher,
        elevated,
        msix: &SystemMsix,
        scan: &scan_apps,
        msiexec: msiexec_path()?,
        busy: &busy,
        sleep: &sleep,
    })
}
