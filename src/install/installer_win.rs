//! Runs installer.file without PowerShell (Task 151).
use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_System, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use crate::bridge::spawn_win::{self, PIPE_GRACE, RunSpec};
use crate::install::inno;

/// Runs the installer at the absolute path `exe` (inside the app folder) in `dir`, with only
/// System32 and its own folder on `PATH`. `.cmd`/`.bat` with arguments `std` refuses fail
/// with that error.
pub fn run(exe: &Path, args: &[String], dir: &Path) -> io::Result<(i32, Vec<String>)> {
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
