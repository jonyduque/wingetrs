//! Default switches and command line (Task 174).

use crate::config::InstallerMode;
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

/// Progress window (the default) or quiet, unless the installer is an exe with no silent switch.
// Port of the experience-switch choice, AppInstallerCLICore_Workflows_ShellExecuteInstallerHandler.cpp:86-105
pub fn mode_for(kind: &InstallerKind, sw: &InstallerSwitches, setting: InstallerMode) -> Mode {
    let has_defaults = defaults(kind).is_some();
    if setting == InstallerMode::Progress
        && (has_defaults || present(&sw.silent_with_progress).is_some())
    {
        Mode::SilentWithProgress
    } else if has_defaults || present(&sw.silent).is_some() {
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
    args_with(kind, sw, mode, upgrade, true)
}

/// `install_args` without the Log switch (the manifest's or the default one): for an installer
/// that runs elevated, which must not write to a path the user controls (Task 196).
pub fn install_args_unlogged(
    kind: &InstallerKind,
    sw: &InstallerSwitches,
    mode: Mode,
    upgrade: bool,
) -> Result<CommandLine, ArgsError> {
    args_with(kind, sw, mode, upgrade, false)
}

fn args_with(
    kind: &InstallerKind,
    sw: &InstallerSwitches,
    mode: Mode,
    upgrade: bool,
    with_log: bool,
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
    let log = present(&sw.log)
        .or(def.and_then(|d| d.2))
        .filter(|_| with_log);
    let mut parts: Vec<&str> = vec![];
    parts.extend(mode_part);
    parts.extend(log);
    parts.extend(present(&sw.custom));
    if upgrade {
        parts.extend(present(&sw.upgrade));
    }
    let mut args = parts.join(" ");
    if matches!(kind, InstallerKind::Inno) {
        args = dedupe_tokens(&args);
    }
    validate(&args)?;
    let program = match kind {
        InstallerKind::Msi | InstallerKind::Wix => Program::Msiexec,
        _ => Program::Installer,
    };
    Ok(CommandLine { program, args })
}

/// `args` without the tokens (split on spaces outside quotes) already seen earlier, compared
/// without case: a manifest that repeats an Inno switch (`/CurrentUser ... /CURRENTUSER`) keeps
/// the first. Inno switches are single tokens, so no value is left behind.
fn dedupe_tokens(args: &str) -> String {
    let mut tokens: Vec<&str> = Vec::new();
    let mut start = None;
    let mut quoted = false;
    for (i, c) in args.char_indices() {
        match c {
            '"' => {
                quoted = !quoted;
                start.get_or_insert(i);
            }
            ' ' | '\t' if !quoted => {
                if let Some(s) = start.take() {
                    tokens.push(&args[s..i]);
                }
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(s) = start {
        tokens.push(&args[s..]);
    }
    let mut seen: Vec<String> = Vec::new();
    let mut kept: Vec<&str> = Vec::new();
    for t in tokens {
        let low = t.to_lowercase();
        if !seen.contains(&low) {
            seen.push(low);
            kept.push(t);
        }
    }
    kept.join(" ")
}

pub fn fill_log(args: &str, log: &Path) -> Result<String, ArgsError> {
    let p = log.to_string_lossy();
    if p.contains(['"', '\r', '\n', '\0']) {
        return Err(ArgsError::Invalid("caminho de log inválido"));
    }
    // A trailing backslash would escape the closing quote of "<LOGPATH>" in CreateProcess.
    if p.ends_with('\\') {
        return Err(ArgsError::Invalid(
            "caminho de log termina em barra invertida",
        ));
    }
    // A bare placeholder gets its own quotes: the log folder may hold a space
    // (`C:\Program Files\wingetrs\staging\...`) and the path must stay one argument.
    let mut out = String::with_capacity(args.len() + p.len() + 2);
    let mut rest = args;
    while let Some(i) = rest.find(LOG_TOKEN) {
        out.push_str(&rest[..i]);
        rest = &rest[i + LOG_TOKEN.len()..];
        if out.ends_with('"') && rest.starts_with('"') {
            out.push_str(&p);
        } else {
            out.push('"');
            out.push_str(&p);
            out.push('"');
        }
    }
    out.push_str(rest);
    Ok(out)
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
