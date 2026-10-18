//! [`previous`](super::previous) against this machine's registry and known folders.
use super::previous;
use crate::installed::Hive;
use crate::sources::winget::manifest::MergedManifest;
use crate::winget_install::elevated_rules::KnownFolders;
use crate::winget_install::engine::InstalledFacts;
use crate::winget_install::plan::WingetPlan;
use crate::winget_install::uninstall::{UninstallPlan, UserFolders};
use crate::winget_install::uninstall_cmd::ArpDetails;
use std::path::Path;

fn read(hive: Hive, key: &str) -> Result<Option<ArpDetails>, String> {
    super::arp_read_win::read_details(hive, key).map_err(|e| e.to_string())
}

fn is_dir(p: &str) -> bool {
    Path::new(p).is_dir()
}

/// The system folders and the user's own (`root`: the wingetrs data root).
pub fn folders(root: &Path) -> Result<(KnownFolders, UserFolders), String> {
    let k = crate::bridge::arp_ops_win::known_folders()
        .map_err(|e| format!("pastas do sistema ilegíveis: {e}"))?;
    let u = super::folders_win::user_folders(root)
        .map_err(|e| format!("pastas do usuário ilegíveis: {e}"))?;
    Ok((k, u))
}

/// [`previous::installed_facts_of`] of the record `row_key`.
pub fn installed_facts(row_key: &str) -> InstalledFacts {
    previous::installed_facts_of(row_key, &read)
}

/// [`previous::previous_plan`] of the record `row_key` (`root`: the wingetrs data root).
pub fn previous_plan(
    root: &Path,
    row_key: &str,
    plan: &WingetPlan,
) -> Result<UninstallPlan, String> {
    previous::previous_plan(row_key, plan, &read, &|| folders(root), &is_dir)
}

/// [`previous::plain_removal_plan`] against this machine (`root`: the wingetrs data root).
pub fn plain_removal_plan(
    root: &Path,
    req: &previous::RemovalRequest,
    manifest: Option<&MergedManifest>,
) -> Result<UninstallPlan, String> {
    previous::plain_removal_plan(req, manifest, &read, &|| folders(root), &is_dir)
}
