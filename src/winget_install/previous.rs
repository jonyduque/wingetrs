//! What a removal or an upgrade knows of the installed record, shared by the CLI and the TUI
//! (Task 193): the facts the upgrade planner filters by, the NSIS hint, and the plan removing
//! the record (alone or before an `uninstallPrevious` upgrade). Platform-neutral: the registry
//! and the folders come in as functions (production: `previous_win`).
use crate::installed::Hive;
use crate::sources::winget::manifest::{InstallerKind, MergedManifest};
use crate::winget_install::elevated_rules::KnownFolders;
use crate::winget_install::engine::{InstalledFacts, InstalledKind, installed_facts};
use crate::winget_install::launch::Elevation;
use crate::winget_install::plan::WingetPlan;
use crate::winget_install::uninstall::{
    UninstallPlan, UninstallTarget, UserFolders, plan_uninstall, target_of,
};
use crate::winget_install::uninstall_cmd::ArpDetails;

/// Reads one ARP record (`Ok(None)`: it is gone).
pub type ReadDetails<'a> = &'a dyn Fn(Hive, &str) -> Result<Option<ArpDetails>, String>;

/// The system folders and the user's own.
pub type ReadFolders<'a> = &'a dyn Fn() -> Result<(KnownFolders, UserFolders), String>;

/// The installed facts of the record `row_key`: an MSIX package is of the MSIX family; an ARP
/// record is read (unreadable or gone: nothing known, the planner does not filter).
pub fn installed_facts_of(row_key: &str, details: ReadDetails<'_>) -> InstalledFacts {
    match target_of(row_key) {
        Some(UninstallTarget::Arp { hive, key }) => details(hive, &key)
            .ok()
            .flatten()
            .map(|d| installed_facts(&d))
            .unwrap_or_default(),
        Some(UninstallTarget::Msix { .. }) => InstalledFacts {
            arch: None,
            kind: Some(InstalledKind::Msix),
        },
        None => InstalledFacts::default(),
    }
}

/// Whether the package's manifest says its uninstaller is NSIS (`/S`): some installer is
/// nullsoft (a zip's nested one counts) and none is of a type whose uninstaller takes other
/// switches. MSI, MSIX and portable types do not count: their records never reach the NSIS
/// rule.
pub fn manifest_nsis(m: &MergedManifest) -> bool {
    let kinds: Vec<InstallerKind> = m
        .effective()
        .into_iter()
        .map(|i| match (i.kind, i.nested_kind) {
            (InstallerKind::Zip, Some(nested)) => nested,
            (kind, _) => kind,
        })
        .collect();
    kinds.contains(&InstallerKind::Nullsoft)
        && !kinds.iter().any(|k| {
            matches!(
                k,
                InstallerKind::Exe
                    | InstallerKind::Inno
                    | InstallerKind::Burn
                    | InstallerKind::Unknown(_)
            )
        })
}

/// What [`removal_plan`] removes, besides the record.
pub struct Removal<'a> {
    /// Shown for an MSIX package (an ARP record has its own).
    pub name: &'a str,
    pub version: Option<String>,
    /// The NSIS hint (from a manifest).
    pub nsis: bool,
    /// The app's other ARP records (shared data).
    pub others: &'a [ArpDetails],
    /// Whether this record is the principal one.
    pub is_main: bool,
}

/// The plan removing the record `row_key`.
pub fn removal_plan(
    row_key: &str,
    r: Removal<'_>,
    details: ReadDetails<'_>,
    folders: ReadFolders<'_>,
    is_dir: &dyn Fn(&str) -> bool,
) -> Result<UninstallPlan, String> {
    match target_of(row_key) {
        Some(UninstallTarget::Arp { hive, key }) => {
            let d = details(hive, &key)
                .map_err(|e| format!("registro ilegível ({key}): {e}"))?
                .ok_or_else(|| format!("o registro {key} não existe mais"))?;
            let (k, u) = folders()?;
            plan_uninstall(&d, r.nsis, r.others, r.is_main, &k, &u, is_dir)
        }
        Some(UninstallTarget::Msix { family }) => Ok(UninstallPlan {
            target: UninstallTarget::Msix { family },
            name: r.name.to_string(),
            version: r.version,
            command: None,
            elevation: Elevation::Never,
            shared: None,
        }),
        None => Err(format!("registro inválido: {row_key}")),
    }
}

/// The plan removing the installed record `row_key` before an `uninstallPrevious` upgrade to
/// `plan`. The NSIS hint comes from the new plan's installer type; no other record shares
/// data here (the upgrade puts the app back).
pub fn previous_plan(
    row_key: &str,
    plan: &WingetPlan,
    details: ReadDetails<'_>,
    folders: ReadFolders<'_>,
    is_dir: &dyn Fn(&str) -> bool,
) -> Result<UninstallPlan, String> {
    let r = Removal {
        name: &plan.name,
        version: plan.upgrade_from.clone(),
        nsis: plan.installer.kind == InstallerKind::Nullsoft,
        others: &[],
        is_main: true,
    };
    removal_plan(row_key, r, details, folders, is_dir)
}
