//! Engine choice and native upgrade (Task 185).
//!
//! The Settings engine (`winget_engine`) decides who installs; the native upgrade follows the
//! manifest's `UpgradeBehavior` (`install` in place, `uninstallPrevious`, `deny`) as winget does.

use std::collections::BTreeSet;

use super::ops::{InstallReport, WingetCtx, WingetOpsError, install};
use super::plan::{PlanOrBridge, WingetPlan, plan};
use super::select::{Host, Refusal, Scope, ScopeWanted};
use super::switches::install_args;
use super::uninstall::{UninstallCtx, UninstallPlan, UninstallStatus, uninstall};
use crate::config::{DefaultScope, WingetEngine};
use crate::core::version::Version;
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

/// The plan for an upgrade of `installed` to `manifest`. Like winget, installers that say
/// `UpgradeBehavior: deny` are skipped while another one fits (the denial is final only when
/// every installer denies), so both engines pick the same installer for the same request.
// Rule of winget-cli src/AppInstallerCLICore/Workflows/UpdateFlow.cpp (deny) and
// ManifestComparator.cpp (installed scope); `bridge::winget_applicability` is the other path.
pub fn plan_upgrade(
    manifest: &MergedManifest,
    host: &Host,
    wanted: ScopeWanted,
    installed: &str,
) -> Result<PlanOrBridge, Refusal> {
    let mut kept = manifest.clone();
    kept.installers.retain(|i| {
        !i.upgrade_behavior
            .as_deref()
            .or(manifest.upgrade_behavior.as_deref())
            .is_some_and(|b| b.trim().eq_ignore_ascii_case("deny"))
    });
    if kept.installers.is_empty() && !manifest.installers.is_empty() {
        return Err(Refusal(UpgradeError::Denied.to_string()));
    }
    plan(&kept, host, wanted, Some(installed.to_string()))
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
            let report = uninstall(uctx, previous).map_err(UpgradeError::Uninstall)?;
            if report.status != UninstallStatus::Removed || report.warning.is_some() {
                return Err(UpgradeError::NotRemoved(
                    report
                        .warning
                        .unwrap_or_else(|| "o registro continua".to_string()),
                ));
            }
            // A fresh install after the removal: no `Upgrade` switch.
            let mut fresh = plan.clone();
            fresh.upgrade_from = None;
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
            Ok(install(wctx, &fresh).await?)
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
