//! Native install plan (Task 181): the chosen installer, its command line and elevation, and
//! the text shown by `--dry-run` and by every confirmation (CLI and TUI).

use super::launch::{Elevation, install_elevation};
use super::select::{Host, Refusal, ScopeWanted, Verdict, choose};
use super::switches::{CommandLine, Mode, Program, install_args, mode_for};
use crate::sources::winget::manifest::{
    Agreement, EffectiveInstaller, InstallerKind, MergedManifest,
};

/// Longest agreement text shown when the agreement has no URL.
const AGREEMENT_TEXT_MAX: usize = 200;

/// Everything the install flow needs, decided before anything is downloaded.
#[derive(Debug, Clone)]
pub struct WingetPlan {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: Option<String>,
    /// Position of `installer` in `MergedManifest::effective()`.
    pub index: usize,
    pub installer: EffectiveInstaller,
    pub mode: Mode,
    /// `None` for msix/appx (deployed by the package manager).
    pub command: Option<CommandLine>,
    pub elevation: Elevation,
    /// The installer has no silent mode: its window opens.
    pub interactive: bool,
    pub agreements: Vec<Agreement>,
    /// Installed version being upgraded, if any (adds the `Upgrade` switch).
    pub upgrade_from: Option<String>,
}

/// Run natively, or hand the package to winget (the text says why).
// Built once per install and matched right away; boxing would only change the brief's shape.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum PlanOrBridge {
    Native(WingetPlan),
    Bridge(String),
}

/// Chooses the installer for `host` and builds its plan.
pub fn plan(
    manifest: &MergedManifest,
    host: &Host,
    wanted: ScopeWanted,
    upgrade_from: Option<String>,
) -> Result<PlanOrBridge, Refusal> {
    let installers = manifest.effective();
    let sel = choose(&installers, host, wanted)?;
    let interactive = match sel.verdict {
        Verdict::Bridge(why) => return Ok(PlanOrBridge::Bridge(why)),
        Verdict::Native { interactive } => interactive,
    };
    let inst = sel.installer;
    let mode = mode_for(&inst.kind, &inst.switches);
    let command = if matches!(inst.kind, InstallerKind::Msix | InstallerKind::Appx) {
        None
    } else {
        Some(
            install_args(&inst.kind, &inst.switches, mode, upgrade_from.is_some())
                .map_err(|e| Refusal(e.to_string()))?,
        )
    };
    let elevation = install_elevation(&inst.kind, inst.scope.as_deref(), inst.elevation.as_deref());
    let name = manifest
        .package_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(&manifest.package_identifier)
        .to_string();
    Ok(PlanOrBridge::Native(WingetPlan {
        id: manifest.package_identifier.clone(),
        name,
        version: manifest.package_version.clone(),
        publisher: manifest.publisher.clone(),
        index: sel.index,
        installer: inst,
        mode,
        command,
        elevation,
        interactive: interactive || mode == Mode::Interactive,
        agreements: manifest.agreements.clone(),
        upgrade_from,
    }))
}

/// One line of text, at most `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let mut cut: String = flat.chars().take(max).collect();
        cut.push('…');
        cut
    }
}

impl std::fmt::Display for WingetPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let scope = self
            .installer
            .scope
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("escopo do instalador");
        writeln!(
            f,
            "{} {} ({}, {scope})",
            self.name,
            self.version,
            self.installer.kind.name()
        )?;
        write!(f, "Instalador: {}", self.installer.url)?;
        if let Some(cmd) = &self.command {
            let program = match cmd.program {
                Program::Msiexec => "msiexec.exe /i \"<instalador>\"",
                Program::Installer => "<instalador>",
            };
            write!(f, "\nLinha de comando: {program}")?;
            if !cmd.args.is_empty() {
                write!(f, " {}", cmd.args)?;
            }
        }
        if self.elevation == Elevation::Helper {
            write!(f, "\npede UAC")?;
        }
        if self.interactive {
            write!(f, "\nabre a janela do instalador (sem modo silencioso)")?;
        }
        for a in &self.agreements {
            let label = a
                .agreement_label
                .as_deref()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .unwrap_or("Contrato");
            write!(f, "\nLicença: {}", one_line(label, AGREEMENT_TEXT_MAX))?;
            match (a.agreement_url.as_deref(), a.agreement.as_deref()) {
                (Some(url), _) if !url.trim().is_empty() => write!(f, " — {}", url.trim())?,
                (_, Some(text)) if !text.trim().is_empty() => {
                    write!(f, " — {}", one_line(text, AGREEMENT_TEXT_MAX))?
                }
                _ => {}
            }
        }
        Ok(())
    }
}
