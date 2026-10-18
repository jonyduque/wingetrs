//! innounp process runner (Task 147).
use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_System, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use crate::bridge::spawn_win::{self, PIPE_GRACE, RunSpec};
use crate::install::inno;

/// Runs innounp at the absolute path `exe` in `dir`, with only System32 and its own folder on
/// `PATH`; output is decoded as CP850.
pub fn run(exe: &Path, args: &[String], dir: &Path) -> io::Result<(i32, Vec<String>)> {
    let env = inno::child_env(&system32()?, exe);
    let spec = RunSpec {
        program: exe,
        args,
        dir,
        env: &env,
        env_remove: &[],
        decode: inno::decode_cp850,
        grace: PIPE_GRACE,
    };
    spawn_win::run_spec(&spec, |line| tracing::debug!("innounp: {line}"))
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
