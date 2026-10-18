//! Engine choice and native upgrade (Task 185).
//!
//! The Settings engine (`winget_engine`) decides who installs; the native upgrade follows the
//! manifest's `UpgradeBehavior` (`install` in place, `uninstallPrevious`, `deny`) as winget does.

use std::collections::BTreeSet;

use super::ops::{InstallReport, WingetCtx, WingetOpsError, install, install_locked, lock_install};
use super::plan::{PlanOrBridge, WingetPlan, plan};
use super::select::{Host, Refusal, Scope, ScopeWanted, arch_rank};
use super::switches::install_args;
use super::uninstall::{UninstallCtx, UninstallPlan, UninstallStatus, uninstall};
use super::uninstall_cmd::ArpDetails;
use crate::config::{DefaultScope, WingetEngine};
use crate::core::version::Version;
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
    let arch = match d.hive {
        Hive::MachineX86 => Some("x86"),
        Hive::MachineX64 => Some("x64"),
        Hive::User => None,
    };
    InstalledFacts { arch, kind }
}

/// The plan for an upgrade of `installed` to `manifest`. Like winget, installers that say
/// `UpgradeBehavior: deny` are skipped while another one fits (the denial is final only when
/// every installer that runs here denies), installers of a type that cannot replace the
/// installed one are filtered out, and the installed architecture is preferred (an x86 app on
/// an x64 machine stays x86): both engines pick the same installer for the same request.
// Rules of winget-cli src/AppInstallerCLICore/Workflows/UpdateFlow.cpp (deny) and
// src/AppInstallerCommonCore/Manifest/ManifestComparator.cpp (InstalledScopeFilter,
// InstalledTypeFilter, allowed architectures); `bridge::winget_applicability` is the other path.
pub fn plan_upgrade(
    manifest: &MergedManifest,
    host: &Host,
    wanted: ScopeWanted,
    installed: &str,
    facts: &InstalledFacts,
) -> Result<PlanOrBridge, Refusal> {
    let from = Some(installed.to_string());
    let mut kept = manifest.clone();
    kept.installers
        .retain(|i| arch_rank(&i.architecture, &host.arch).is_some());
    if kept.installers.is_empty() {
        // Nothing runs here: the planner gives the architecture refusal.
        return plan(manifest, host, wanted, from);
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
    plan(&kept, host, wanted, from)
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
    match route {
        UpgradeRoute::Denied | UpgradeRoute::InPlace => Ok(install(wctx, plan).await?),
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
                    install_args(
                        &fresh.installer.kind,
                        &fresh.installer.switches,
                        fresh.mode,
                        false,
                    )
                    .map_err(WingetOpsError::from)?,
                );
            }
            // Removal and install under one `install.lock`: nothing runs in between.
            let lock = lock_install(&wctx.data.install_layout(), wctx.wait).await?;
            let report = uninstall(uctx, previous).map_err(UpgradeError::Uninstall)?;
            if report.status != UninstallStatus::Removed || report.warning.is_some() {
                return Err(UpgradeError::NotRemoved(
                    report
                        .warning
                        .unwrap_or_else(|| "o registro continua".to_string()),
                ));
            }
            install_locked(wctx, &fresh, &lock)
                .await
                .map_err(UpgradeError::InstallAfterRemoval)
        }
    }
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
