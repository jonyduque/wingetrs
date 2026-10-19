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

/// How the shell starts the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// `open`: unelevated (a program that asks for UAC by itself still may).
    Open,
    /// `runas`: a UAC prompt for this one program, which then runs elevated (Task 196).
    RunAs,
}

impl Verb {
    /// The shell verb.
    pub fn as_str(self) -> &'static str {
        match self {
            Verb::Open => "open",
            Verb::RunAs => "runas",
        }
    }
}

/// One program to start: absolute path, raw parameter string, optional working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub params: String,
    pub dir: Option<PathBuf>,
    pub show: Show,
    pub verb: Verb,
}

/// Starts a program and waits for it to end; the exit code.
pub trait Launcher: Send + Sync {
    fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32>;

    /// `run`, calling `started` once the process exists (after the UAC prompt of
    /// `Verb::RunAs`) and before waiting for it. By default `started` is called right before
    /// `run`.
    fn run_started(&self, spec: &LaunchSpec, started: &mut dyn FnMut()) -> std::io::Result<i32> {
        started();
        self.run(spec)
    }
}

/// Whether the program runs elevated. For an install, `Helper` is the installer's own UAC
/// prompt (`Verb::RunAs`, Task 196); for an uninstall, the elevated helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    Never,
    Helper,
}

/// Whether an install runs elevated (its own UAC prompt), from the installer type, its `Scope`
/// and its `ElevationRequirement` (all comparisons case-insensitive). A `user` scope never runs
/// elevated, not even with `elevationRequired`: elevated through another account it would
/// install into the administrator's profile; the installer asks for UAC by itself. An unknown
/// scope counts as absent.
pub fn install_elevation(
    kind: &InstallerKind,
    scope: Option<&str>,
    requirement: Option<&str>,
) -> Elevation {
    let is = |v: Option<&str>, want: &str| v.is_some_and(|s| s.eq_ignore_ascii_case(want));
    let user = is(scope, "user");
    let machine = is(scope, "machine");
    if matches!(kind, InstallerKind::Msix | InstallerKind::Appx) {
        return Elevation::Never;
    }
    if is(requirement, "elevationProhibited") {
        return Elevation::Never;
    }
    if user {
        return Elevation::Never;
    }
    if is(requirement, "elevationRequired") {
        return Elevation::Helper;
    }
    if machine {
        return Elevation::Helper;
    }
    if matches!(
        kind,
        InstallerKind::Msi | InstallerKind::Wix | InstallerKind::Burn
    ) {
        return Elevation::Helper;
    }
    // The installer asks for UAC by itself when it needs it.
    Elevation::Never
}

/// Whether an installer wingetrs launches unelevated asks for its own UAC prompt: its
/// `ElevationRequirement` is `elevatesSelf` or `elevationRequired` (a `user` scope keeps those
/// unelevated, see [`install_elevation`]); any letter case.
pub fn self_elevates(requirement: Option<&str>) -> bool {
    requirement.is_some_and(|r| {
        r.eq_ignore_ascii_case("elevatesSelf") || r.eq_ignore_ascii_case("elevationRequired")
    })
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
