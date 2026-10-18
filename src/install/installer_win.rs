//! Runs installer.file without PowerShell (Task 151).
use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_System, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use crate::bridge::spawn_win::{self, PIPE_GRACE, RunSpec};
use crate::install::executor::ToolRun;
use crate::install::inno;
use crate::install::script_policy::refuse_if_elevated;
use crate::winget_install::launch_win::is_elevated;

/// Runs the installer at the absolute path `exe` (inside the app folder) in `dir`, with only
/// System32 and its own folder on `PATH`. `.cmd`/`.bat` with arguments `std` refuses fail
/// with that error.
/// Never with an elevated token (same refusal as the script host, Task 191).
pub fn run(exe: &Path, args: &[String], dir: &Path) -> io::Result<(i32, Vec<String>)> {
    run_with(exe, args, dir, is_elevated, launch)
}

/// Test seam of [`run`]: the elevation check and the process launch are given.
pub fn run_with(
    exe: &Path,
    args: &[String],
    dir: &Path,
    elevated: fn() -> io::Result<bool>,
    launch: ToolRun,
) -> io::Result<(i32, Vec<String>)> {
    refuse_if_elevated(elevated()?)
        .map_err(|m| io::Error::new(io::ErrorKind::PermissionDenied, m))?;
    launch(exe, args, dir)
}

fn launch(exe: &Path, args: &[String], dir: &Path) -> io::Result<(i32, Vec<String>)> {
    let env = inno::child_env(&system32()?, exe);
    let spec = RunSpec {
        program: exe,
        args,
        dir,
        env: &env,
        env_remove: &[],
        decode: |b| String::from_utf8_lossy(b).into_owned(),
        grace: PIPE_GRACE,
    };
    spawn_win::run_spec(&spec, |line| tracing::debug!("instalador: {line}"))
}

/// `System32` from the known folder, never from `%SystemRoot%`.
fn system32() -> io::Result<PathBuf> {
    // SAFETY: the returned string is freed below with CoTaskMemFree.
    let p = unsafe { SHGetKnownFolderPath(&FOLDERID_System, KF_FLAG_DEFAULT, None) }
        .map_err(|e| io::Error::other(e.to_string()))?;
    // SAFETY: a NUL-terminated string from the call above.
    let text = unsafe { p.to_string() };
    // SAFETY: allocated by SHGetKnownFolderPath, freed once.
    unsafe { CoTaskMemFree(Some(p.0 as *const core::ffi::c_void)) };
    text.map(PathBuf::from)
        .map_err(|e| io::Error::other(e.to_string()))
}
