//! Native install plan (Task 181): the chosen installer, its command line and elevation, and
//! the text shown by `--dry-run` and by every confirmation (CLI and TUI).

use super::launch::{Elevation, install_elevation};
use super::select::{Host, Refusal, ScopeWanted, Verdict, choose};
use crate::config::InstallerMode;
use super::switches::{CommandLine, Mode, Program, install_args, mode_for};
use crate::bridge::worker::{
    ElevatedKind, InstallerRequest, exe_params_allowed, msi_params_allowed_in,
};
use crate::sources::winget::manifest::{
    Agreement, EffectiveInstaller, InstallerKind, MergedManifest,
};

/// Longest agreement text shown when the agreement has no URL.
const AGREEMENT_TEXT_MAX: usize = 200;

/// Everything the install flow needs, decided before anything is downloaded.
#[derive(Debug, Clone, PartialEq)]
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
    plan_with_mode(
        manifest,
        host,
        wanted,
        upgrade_from,
        InstallerMode::default(),
    )
}

/// `plan` with the `installer_mode` setting (progress window or quiet).
pub fn plan_with_mode(
    manifest: &MergedManifest,
    host: &Host,
    wanted: ScopeWanted,
    upgrade_from: Option<String>,
    installer_mode: InstallerMode,
) -> Result<PlanOrBridge, Refusal> {
    let installers = manifest.effective();
    let sel = choose(&installers, host, wanted)?;
    let interactive = match sel.verdict {
        Verdict::Bridge(why) => return Ok(PlanOrBridge::Bridge(why)),
        Verdict::Native { interactive } => interactive,
    };
    let inst = sel.installer;
    let mode = mode_for(&inst.kind, &inst.switches, installer_mode);
    let command = if matches!(inst.kind, InstallerKind::Msix | InstallerKind::Appx) {
        None
    } else {
        Some(
            install_args(&inst.kind, &inst.switches, mode, upgrade_from.is_some())
                .map_err(|e| Refusal(e.to_string()))?,
        )
    };
    let elevation = install_elevation(&inst.kind, inst.scope.as_deref(), inst.elevation.as_deref());
    if elevation == Elevation::Helper
        && let Some(cmd) = &command
        && let Err(why) = helper_accepts(&inst, cmd)
    {
        return Ok(PlanOrBridge::Bridge(why));
    }
    // MSIX is deployed by the package manager: no installer window, whatever the mode.
    let interactive = command.is_some() && (interactive || mode == Mode::Interactive);
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
        interactive,
        agreements: manifest.agreements.clone(),
        upgrade_from,
    }))
}

/// The helper's type for an installer that runs elevated (Task 180); `None` for the others.
pub fn elevated_kind(kind: &InstallerKind) -> Option<ElevatedKind> {
    Some(match kind {
        InstallerKind::Msi => ElevatedKind::Msi,
        InstallerKind::Wix => ElevatedKind::Wix,
        InstallerKind::Burn => ElevatedKind::Burn,
        InstallerKind::Inno => ElevatedKind::Inno,
        InstallerKind::Nullsoft => ElevatedKind::Nullsoft,
        InstallerKind::Exe => ElevatedKind::Exe,
        _ => return None,
    })
}

/// Folders the plan lets an install folder parameter point into: none (the helper's
/// `Program Files` list is not known here), so such parameters go to the bridge.
const NO_PROGRAM_FILES: &[String] = &[];

/// Runs the helper's own request check (Task 180) on what the plan would send, before
/// anything is downloaded; the reason (Portuguese) when the helper would refuse it.
fn helper_accepts(inst: &EffectiveInstaller, cmd: &CommandLine) -> Result<(), String> {
    let Some(kind) = elevated_kind(&inst.kind) else {
        return Err(format!(
            "tipo {} sem auxiliar elevado: instalado pelo winget",
            inst.kind.name()
        ));
    };
    let sha = inst.sha256.trim().to_ascii_lowercase();
    let req = InstallerRequest {
        // Stand-in for the blob path the flow will send (same shape: `...\blobs\<sha>`).
        source: format!(r"C:\wingetrs\blobs\{sha}"),
        sha256: sha,
        file_name: format!("installer.{}", if kind.is_msi() { "msi" } else { "exe" }),
        kind,
        params: cmd.args.clone(),
    };
    match req.check(NO_PROGRAM_FILES) {
        Ok(()) => Ok(()),
        Err(reason) => match refused_token(kind, &cmd.args) {
            Some(token) => Err(format!("{reason} para elevação: {token}")),
            None => Err(format!("{reason} (elevação)")),
        },
    }
}

/// `params` split on spaces outside double quotes.
fn tokens(params: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (None, false);
    for (i, c) in params.char_indices() {
        match c {
            '"' => {
                quoted = !quoted;
                start.get_or_insert(i);
            }
            ' ' if !quoted => {
                if let Some(s) = start.take() {
                    out.push(&params[s..i]);
                }
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(s) = start {
        out.push(&params[s..]);
    }
    out
}

/// The first token the allowlist turns down: where the accepted prefix stops growing (a
/// switch that takes the next token as its value is given that token too).
fn refused_token(kind: ElevatedKind, params: &str) -> Option<String> {
    let allowed = |p: &str| {
        if kind.is_msi() {
            msi_params_allowed_in(p, NO_PROGRAM_FILES)
        } else {
            exe_params_allowed(kind, p, NO_PROGRAM_FILES)
        }
    };
    let toks = tokens(params);
    let prefix = |n: usize| toks[..n].join(" ");
    let mut ok = 0;
    for n in 1..=toks.len() {
        if allowed(&prefix(n)) {
            ok = n;
        } else if n > ok + 1 {
            break;
        }
    }
    toks.get(ok).map(|t| t.to_string())
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

/// `name version`, or only `name` when it already carries the version (`CrystalDiskMark
/// 9.0.3`, as many ARP display names do). Shared by the CLI and the TUI texts.
pub fn name_version(name: &str, version: Option<&str>) -> String {
    match version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v)
            if !name
                .split_whitespace()
                .any(|w| w == v || w.trim_start_matches(['v', 'V']) == v) =>
        {
            format!("{name} {v}")
        }
        _ => name.to_string(),
    }
}

/// The planner's reason for the bridge without its "instalado pelo winget" ending, for a line
/// that already says it goes through winget (CLI `Instalando pelo winget: …`, TUI `Pelo
/// winget: …`).
pub fn bridge_reason(why: &str) -> &str {
    why.strip_suffix(": instalado pelo winget").unwrap_or(why)
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
            "{} ({}, {scope})",
            name_version(&self.name, Some(&self.version)),
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
            write!(f, "\nabre a janela do instalador (sem modo silencioso)")?;        } else if self.command.is_some() {
            match self.mode {
                Mode::SilentWithProgress => write!(f, "
mostra o progresso do instalador")?,
                Mode::Silent => write!(f, "
sem janela")?,
                Mode::Interactive => {}
            }
        }
        if self.upgrade_from.is_some()
            && super::engine::upgrade_route(&self.installer)
                == super::engine::UpgradeRoute::UninstallFirst
        {
            write!(
                f,
                "\nremove a versão instalada antes (o desinstalador pode abrir janela ou pedir \
                 permissão de administrador)"
            )?;
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
