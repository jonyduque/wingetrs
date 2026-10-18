//! Launcher abstraction and elevation rule (Task 178).
use crate::installed::Hive;
use crate::sources::winget::manifest::InstallerKind;
use std::path::PathBuf;

/// Window state of the launched program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Show {
    Normal,
    Hidden,
}

/// One program to start: absolute path, raw parameter string, optional working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub params: String,
    pub dir: Option<PathBuf>,
    pub show: Show,
}

/// Starts a program and waits for it to end; the exit code.
pub trait Launcher: Send + Sync {
    fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32>;
}

/// Who starts the program: this (unelevated) process, or the elevated helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    Never,
    Helper,
}

/// Whether an install runs through the elevated helper, from the installer type, its `Scope`
/// and its `ElevationRequirement` (all comparisons case-insensitive).
pub fn install_elevation(
    kind: &InstallerKind,
    scope: Option<&str>,
    requirement: Option<&str>,
) -> Elevation {
    let is = |v: Option<&str>, want: &str| v.is_some_and(|s| s.eq_ignore_ascii_case(want));
    if matches!(kind, InstallerKind::Msix | InstallerKind::Appx) {
        return Elevation::Never;
    }
    if is(requirement, "elevationProhibited") {
        return Elevation::Never;
    }
    if is(requirement, "elevationRequired") {
        return Elevation::Helper;
    }
    if is(scope, "user") {
        return Elevation::Never;
    }
    if is(scope, "machine") {
        return Elevation::Helper;
    }
    if scope.is_none()
        && matches!(
            kind,
            InstallerKind::Msi | InstallerKind::Wix | InstallerKind::Burn
        )
    {
        return Elevation::Helper;
    }
    // The installer asks for UAC by itself when it needs it.
    Elevation::Never
}

/// Whether an uninstall runs through the elevated helper: per-user entries never; machine
/// entries when they go through `msiexec` or the uninstaller sits in an administrator folder.
pub fn uninstall_elevation(hive: Hive, msiexec: bool, program_in_admin_folder: bool) -> Elevation {
    match hive {
        Hive::User => Elevation::Never,
        Hive::MachineX64 | Hive::MachineX86 if msiexec || program_in_admin_folder => {
            Elevation::Helper
        }
        Hive::MachineX64 | Hive::MachineX86 => Elevation::Never,
    }
}
