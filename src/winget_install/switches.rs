//! Default switches and command line (Task 174).

use crate::sources::winget::manifest::{InstallerKind, InstallerSwitches};
use std::path::Path;

/// Placeholder replaced by the real log path (winget `ARG_TOKEN_LOGPATH`).
pub const LOG_TOKEN: &str = "<LOGPATH>";

const MAX_ARGS_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Silent,
    SilentWithProgress,
    Interactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Program {
    Msiexec,
    Installer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    pub program: Program,
    pub args: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArgsError {
    #[error("este tipo de instalador não usa linha de comando")]
    NoCommandLine,
    #[error("argumentos inválidos: {0}")]
    Invalid(&'static str),
}

/// Default switches for a kind: (silent, silent_with_progress, log).
// Port of GetDefaultKnownSwitches, AppInstallerCommonCore_Manifest_ManifestCommon.cpp:1034-1062
fn defaults(kind: &InstallerKind) -> Option<(&'static str, &'static str, Option<&'static str>)> {
    match kind {
        InstallerKind::Msi | InstallerKind::Wix | InstallerKind::Burn => Some((
            "/quiet /norestart",
            "/passive /norestart",
            Some("/log \"<LOGPATH>\""),
        )),
        InstallerKind::Nullsoft => Some(("/S", "/S", None)),
        InstallerKind::Inno => Some((
            "/SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART",
            "/SP- /SILENT /SUPPRESSMSGBOXES /NORESTART",
            Some("/LOG=\"<LOGPATH>\""),
        )),
        _ => None,
    }
}

fn has_command_line(kind: &InstallerKind) -> bool {
    matches!(
        kind,
        InstallerKind::Msi
            | InstallerKind::Wix
            | InstallerKind::Burn
            | InstallerKind::Inno
            | InstallerKind::Nullsoft
            | InstallerKind::Exe
    )
}

fn present(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|v| !v.trim().is_empty())
}

/// Silent unless the installer is an exe with no Silent switch.
// Port of the experience-switch choice, AppInstallerCLICore_Workflows_ShellExecuteInstallerHandler.cpp:86-105
pub fn mode_for(kind: &InstallerKind, sw: &InstallerSwitches) -> Mode {
    if defaults(kind).is_some() || present(&sw.silent).is_some() {
        Mode::Silent
    } else if present(&sw.silent_with_progress).is_some() {
        Mode::SilentWithProgress
    } else {
        Mode::Interactive
    }
}

/// Order: mode, Log, Custom, Upgrade; InstallLocation is left out.
// Port of AppInstallerCLICore_Workflows_ShellExecuteInstallerHandler.cpp:106-150
pub fn install_args(
    kind: &InstallerKind,
    sw: &InstallerSwitches,
    mode: Mode,
    upgrade: bool,
) -> Result<CommandLine, ArgsError> {
    if !has_command_line(kind) {
        return Err(ArgsError::NoCommandLine);
    }
    let def = defaults(kind);
    let mode_part = match mode {
        Mode::Silent => present(&sw.silent).or(def.map(|d| d.0)),
        Mode::SilentWithProgress => present(&sw.silent_with_progress).or(def.map(|d| d.1)),
        Mode::Interactive => present(&sw.interactive),
    };
    let log = present(&sw.log).or(def.and_then(|d| d.2));
    let mut parts: Vec<&str> = vec![];
    parts.extend(mode_part);
    parts.extend(log);
    parts.extend(present(&sw.custom));
    if upgrade {
        parts.extend(present(&sw.upgrade));
    }
    let args = parts.join(" ");
    validate(&args)?;
    let program = match kind {
        InstallerKind::Msi | InstallerKind::Wix => Program::Msiexec,
        _ => Program::Installer,
    };
    Ok(CommandLine { program, args })
}

pub fn fill_log(args: &str, log: &Path) -> Result<String, ArgsError> {
    let p = log.to_string_lossy();
    if p.contains(['"', '\r', '\n', '\0']) {
        return Err(ArgsError::Invalid("caminho de log inválido"));
    }
    Ok(args.replace(LOG_TOKEN, &p))
}

pub fn msiexec_params(file: &Path, args: &str) -> String {
    let base = format!("/i \"{}\"", file.display());
    if args.is_empty() {
        base
    } else {
        format!("{base} {args}")
    }
}

pub fn validate(args: &str) -> Result<(), ArgsError> {
    if args.len() > MAX_ARGS_BYTES {
        return Err(ArgsError::Invalid("argumentos grandes demais"));
    }
    if args.contains(['\0', '\r', '\n']) {
        return Err(ArgsError::Invalid("caractere de controle nos argumentos"));
    }
    Ok(())
}
