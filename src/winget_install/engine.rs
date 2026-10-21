//! Engine choice and native upgrade (Task 185).
//!
//! The Settings engine (`winget_engine`) decides who installs; the native upgrade follows the
//! manifest's `UpgradeBehavior` (`install` in place, `uninstallPrevious`, `deny`) as winget does.

use std::collections::BTreeSet;

use super::ops::{
    InstallReport, WingetCtx, WingetOpsError, fetch_verified, install, install_fetched,
    lock_install_for,
};
use super::plan::{PlanOrBridge, WingetPlan, command_for, plan_with_mode};
use super::post_check::version_warning;
use super::removal::{run_uninstaller, uninstaller_phase};
use super::select::{Host, Refusal, Scope, ScopeWanted, arch_applicable};
use super::uninstall::{MsiQueue, UninstallCtx, UninstallPlan, UninstallStatus};
use super::uninstall_cmd::ArpDetails;
use crate::config::{DefaultScope, InstallerMode, WingetEngine};
use crate::core::version::Version;
use crate::install::progress::{Phase, ProgressEvent};
use crate::installed::Hive;
use crate::installed::correlate::{InstallScope, ListRow, UpdateState};
use crate::sources::winget::manifest::{EffectiveInstaller, InstallerKind, MergedManifest};

/// Who runs the install: this program, or winget (the text says why, when it is not the
/// user's setting).
// Built once per request and matched right away; boxing would only change the shape.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Engine {
    Native(WingetPlan),
    /// Installed as a wingetrs app by the phase 2a executor (Task 189).
    Portable(super::portable::PortableSpec),
    Bridge(Option<String>),
}

/// The setting decides: `Winget` always goes to winget (even when the native planner would
/// refuse: winget decides), `Wingetrs` keeps the planner's answer.
pub fn choose_engine(
    setting: WingetEngine,
    planned: Result<PlanOrBridge, Refusal>,
) -> Result<Engine, Refusal> {
    match setting {
        WingetEngine::Winget => Ok(Engine::Bridge(None)),
        WingetEngine::Wingetrs => match planned? {
            PlanOrBridge::Native(p) => Ok(Engine::Native(p)),
            PlanOrBridge::Portable(spec) => Ok(Engine::Portable(spec)),
            PlanOrBridge::Bridge(why) => Ok(Engine::Bridge(Some(why))),
        },
    }
}

/// What the manifest's `UpgradeBehavior` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeRoute {
    InPlace,
    UninstallFirst,
    Denied,
}

/// `install` or absent: in place; `uninstallPrevious`; `deny` (any letter case).
// Values of winget-cli src/AppInstallerCommonCore/Manifest/ManifestEnums.h UpgradeBehaviorEnum.
pub fn upgrade_route(i: &EffectiveInstaller) -> UpgradeRoute {
    match i.upgrade_behavior.as_deref().map(str::trim) {
        Some(b) if b.eq_ignore_ascii_case("uninstallPrevious") => UpgradeRoute::UninstallFirst,
        Some(b) if b.eq_ignore_ascii_case("deny") => UpgradeRoute::Denied,
        _ => UpgradeRoute::InPlace,
    }
}

/// The scope an upgrade asks the planner for: the installed one when known, else the default.
pub fn upgrade_scope(installed: Option<InstallScope>, default: DefaultScope) -> ScopeWanted {
    match installed {
        Some(InstallScope::Machine) => ScopeWanted::Only(Scope::Machine),
        Some(InstallScope::User) => ScopeWanted::Only(Scope::User),
        None => ScopeWanted::Prefer(match default {
            DefaultScope::Machine => Scope::Machine,
            DefaultScope::User => Scope::User,
        }),
    }
}

/// What the registry says about the installed app (winget's `CurrentlyInstalledType` and
/// installed architecture); both optional: unknown means no preference.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstalledFacts {
    /// winget architecture name of the installed app (`x86`, `x64`).
    pub arch: Option<&'static str>,
    pub kind: Option<InstalledKind>,
}

/// Installer families that replace each other (winget's compatibility sets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstalledKind {
    /// inno, nullsoft, exe, burn
    Exe,
    /// msi, wix
    Msi,
    /// msix, msstore
    Msix,
}

// GetCompatibilitySet in winget-cli src/AppInstallerCommonCore/Manifest/ManifestCommon.cpp.
fn kind_of(k: &InstallerKind) -> Option<InstalledKind> {
    match k {
        InstallerKind::Inno
        | InstallerKind::Nullsoft
        | InstallerKind::Exe
        | InstallerKind::Burn => Some(InstalledKind::Exe),
        InstallerKind::Wix | InstallerKind::Msi => Some(InstalledKind::Msi),
        InstallerKind::Msix | InstallerKind::Appx | InstallerKind::Msstore => {
            Some(InstalledKind::Msix)
        }
        _ => None,
    }
}

/// Facts from the ARP record: the view gives the architecture (HKCU says nothing), the
/// `WindowsInstaller` value or an `_is1` key name gives the type.
pub fn installed_facts(d: &ArpDetails) -> InstalledFacts {
    let kind = if d.windows_installer {
        Some(InstalledKind::Msi)
    } else if d.key.to_ascii_lowercase().ends_with("_is1") {
        Some(InstalledKind::Exe)
    } else {
        None
    };
    let bundle = [&d.bundle_cache_path, &d.bundle_upgrade_code]
        .iter()
        .any(|v| v.as_deref().is_some_and(|s| !s.trim().is_empty()));
    let arch = if bundle {
        // Burn bundles always register in the 32-bit view: the view says nothing.
        bundle_arch(d)
    } else {
        match d.hive {
            Hive::MachineX86 => Some("x86"),
            Hive::MachineX64 => Some("x64"),
            Hive::User => None,
        }
    };
    InstalledFacts { arch, kind }
}

/// The architecture a Burn bundle names in its `DisplayName` (`… (x64)`) or cached file name
/// (`…-win-x64.exe`); unknown otherwise (selection then follows the system).
fn bundle_arch(d: &ArpDetails) -> Option<&'static str> {
    let texts = [&d.display_name, &d.bundle_cache_path];
    for t in texts.iter().filter_map(|t| t.as_deref()) {
        let lower = t.to_ascii_lowercase();
        let tokens: Vec<&str> = lower
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|s| !s.is_empty())
            .collect();
        for (arch, names) in [
            ("arm64", &["arm64"][..]),
            ("x64", &["x64", "amd64"][..]),
            ("x86", &["x86"][..]),
        ] {
            if tokens.iter().any(|tok| names.contains(tok)) {
                return Some(arch);
            }
        }
    }
    None
}

/// The plan for an upgrade of `installed` to `manifest`. Like winget, installers that say
/// `UpgradeBehavior: deny` are skipped while another one fits (the denial is final only when
/// every installer that runs here denies), installers of a type that cannot replace the
/// installed one are filtered out, and the installed architecture is preferred (an x86 app on
/// an x64 machine stays x86): both engines pick the same installer for the same request.
/// `installer_mode` is the setting (progress window or quiet), as for an install.
// Rules of winget-cli src/AppInstallerCLICore/Workflows/UpdateFlow.cpp (deny) and
// src/AppInstallerCommonCore/Manifest/ManifestComparator.cpp (InstalledScopeFilter,
// InstalledTypeFilter, allowed architectures); `bridge::winget_applicability` is the other path.
pub fn plan_upgrade(
    manifest: &MergedManifest,
    host: &Host,
    wanted: ScopeWanted,
    installed: &str,
    facts: &InstalledFacts,
    installer_mode: InstallerMode,
) -> Result<PlanOrBridge, Refusal> {
    // A record of the Windows list was not installed by wingetrs: a portable one lives in
    // winget's folder with winget's ARP entry, and a native upgrade would add a second copy.
    match plan_upgrade_any(manifest, host, wanted, installed, facts, installer_mode)? {
        PlanOrBridge::Portable(_) => Ok(PlanOrBridge::Bridge(
            "tipo portable instalado fora do wingetrs: atualizado pelo winget".to_string(),
        )),
        other => Ok(other),
    }
}

fn plan_upgrade_any(
    manifest: &MergedManifest,
    host: &Host,
    wanted: ScopeWanted,
    installed: &str,
    facts: &InstalledFacts,
    installer_mode: InstallerMode,
) -> Result<PlanOrBridge, Refusal> {
    let from = Some(installed.to_string());
    let mut kept = manifest.clone();
    kept.installers.retain(|i| {
        let unsupported = i
            .unsupported_os_architectures
            .as_ref()
            .or(manifest.unsupported_os_architectures.as_ref());
        arch_applicable(
            &i.architecture,
            unsupported.map_or(&[], Vec::as_slice),
            &host.arch,
        )
    });
    if kept.installers.is_empty() {
        // Nothing runs here: the planner gives the architecture refusal.
        return plan_with_mode(manifest, host, wanted, from, installer_mode);
    }
    kept.installers.retain(|i| {
        !i.upgrade_behavior
            .as_deref()
            .or(manifest.upgrade_behavior.as_deref())
            .is_some_and(|b| b.trim().eq_ignore_ascii_case("deny"))
    });
    if kept.installers.is_empty() {
        return Err(Refusal(UpgradeError::Denied.to_string()));
    }
    if let Some(want) = facts.kind {
        let root = manifest.apps_and_features_entries.as_deref().unwrap_or(&[]);
        kept.installers.retain(|i| {
            let own = InstallerKind::parse(
                i.installer_type
                    .as_deref()
                    .or(manifest.installer_type.as_deref()),
            );
            let entries = i.apps_and_features_entries.as_deref().unwrap_or(root);
            kind_of(&own) == Some(want)
                || entries.iter().any(|e| {
                    kind_of(&InstallerKind::parse(e.installer_type.as_deref())) == Some(want)
                })
        });
        if kept.installers.is_empty() {
            return Err(Refusal(
                "nenhum instalador do catálogo é compatível com o tipo instalado".to_string(),
            ));
        }
    }
    if let Some(arch) = facts.arch
        && kept
            .installers
            .iter()
            .any(|i| i.architecture.eq_ignore_ascii_case(arch))
    {
        kept.installers
            .retain(|i| i.architecture.eq_ignore_ascii_case(arch));
    }
    plan_with_mode(&kept, host, wanted, from, installer_mode)
}

#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    #[error("o pacote não permite atualizar por cima; remova e instale de novo")]
    Denied,
    #[error("sem a versão instalada: a atualização precisa do plano com a versão anterior")]
    NoPrevious,
    #[error("a versão {offered} do catálogo não é maior que a instalada ({installed})")]
    NotNewer { offered: String, installed: String },
    #[error("o pacote pede para remover a versão anterior, mas não há plano de remoção")]
    NoUninstallPlan,
    #[error("remoção da versão anterior: {0}")]
    Uninstall(String),
    #[error("a versão anterior não foi removida ({0}); a instalação nova não rodou")]
    NotRemoved(String),
    #[error("a versão anterior foi removida e a nova não instalou: {0}")]
    InstallAfterRemoval(WingetOpsError),
    #[error(transparent)]
    Install(#[from] WingetOpsError),
}

/// Upgrades natively. `plan` was built with `upgrade_from = Some(installed)`; `previous` is the
/// plan to remove the installed record (needed for `uninstallPrevious`).
pub async fn upgrade(
    wctx: &WingetCtx<'_>,
    uctx: &UninstallCtx<'_>,
    plan: &WingetPlan,
    previous: Option<&UninstallPlan>,
) -> Result<InstallReport, UpgradeError> {
    let route = upgrade_route(&plan.installer);
    if route == UpgradeRoute::Denied {
        return Err(UpgradeError::Denied);
    }
    let installed = plan
        .upgrade_from
        .as_deref()
        .ok_or(UpgradeError::NoPrevious)?;
    if Version::parse(&plan.version) <= Version::parse(installed) {
        return Err(UpgradeError::NotNewer {
            offered: plan.version.clone(),
            installed: installed.to_string(),
        });
    }
    let mut report = match route {
        UpgradeRoute::Denied | UpgradeRoute::InPlace => install(wctx, plan).await?,
        UpgradeRoute::UninstallFirst => {
            let previous = previous.ok_or(UpgradeError::NoUninstallPlan)?;
            // A fresh install after the removal: no `Upgrade` switch (built before removing
            // anything, so a bad command line cannot leave the app removed).
            let mut fresh = plan.clone();
            if !matches!(
                fresh.installer.kind,
                InstallerKind::Msix | InstallerKind::Appx
            ) {
                fresh.command = Some(
                    command_for(&fresh.installer, fresh.mode, fresh.elevation, false)
                        .map_err(WingetOpsError::from)?,
                );
            }
            // A live line from the first instant: the `install.lock` wait is silent.
            (wctx.progress)(&ProgressEvent::Phase(Phase::Preparing));
            // Removal and install under one `install.lock`: nothing runs in between.
            let lock = lock_install_for(&wctx.data.install_layout(), wctx.wait, &wctx.gate).await?;
            // Like winget: the new installer is downloaded and verified (and kept open, so it
            // cannot change) before the old version goes; a failed download removes nothing.
            let file = fetch_verified(wctx, &fresh).await?;
            // The removal cannot be undone: Ctrl+C counts only until here.
            if !wctx.gate.try_launch() {
                return Err(WingetOpsError::Cancelled.into());
            }
            // The same phases as a removal: the uninstaller (or the helper's UAC notice), then
            // the record watch.
            (wctx.progress)(&ProgressEvent::Phase(uninstaller_phase(previous)));
            let removed = run_uninstaller(uctx, &MsiQueue::of(wctx), previous, wctx.progress)
                .map_err(UpgradeError::Uninstall)?;
            if removed.status != UninstallStatus::Removed || removed.warning.is_some() {
                return Err(UpgradeError::NotRemoved(
                    removed
                        .warning
                        .unwrap_or_else(|| "o registro continua".to_string()),
                ));
            }
            install_fetched(wctx, &fresh, file, &lock).map_err(UpgradeError::InstallAfterRemoval)?
        }
    };
    if report.warning.is_none() {
        report.warning = version_warning(&plan.version, report.found.as_ref());
    }
    Ok(report)
}

/// The rows "upgrade all" takes: with an update, not pinned, not `RequireExplicitUpgrade`.
/// An app the user named is upgraded even if explicit-only (the caller does not use this).
pub fn upgrade_all_targets<'a>(
    rows: &'a [ListRow],
    pins: &BTreeSet<String>,
    explicit_only: &dyn Fn(&ListRow) -> bool,
) -> Vec<&'a ListRow> {
    rows.iter()
        .filter(|r| matches!(r.state, UpdateState::Available { .. }))
        .filter(|r| !crate::pins::is_pinned(pins, &r.origin, &r.name))
        .filter(|r| !explicit_only(r))
        .collect()
}
