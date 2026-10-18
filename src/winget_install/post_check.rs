//! After the installer: which installed app is the one just installed (Task 181). Pure.

use super::plan::WingetPlan;
use crate::core::version::Version;
use crate::installed::normalize::NameNormalizer;
use crate::installed::{InstallSource, InstalledApp};
use crate::sources::winget::manifest::InstallerKind;

/// A product/upgrade code without braces, uppercase; `None` when empty.
fn code(raw: Option<&str>) -> Option<String> {
    let c = raw?
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();
    (!c.is_empty()).then(|| c.to_ascii_uppercase())
}

fn same_code(want: Option<&str>, have: Option<&str>) -> bool {
    matches!((code(want), code(have)), (Some(w), Some(h)) if w == h)
}

/// The installed app matching `plan`, in this order: `ProductCode`/`UpgradeCode` of
/// `AppsAndFeaturesEntries`, the installer's `ProductCode`, the `PackageFamilyName` (MSIX
/// only), then the normalized name and publisher (as the catalog correlation does).
pub fn find_installed<'a>(plan: &WingetPlan, apps: &'a [InstalledApp]) -> Option<&'a InstalledApp> {
    let inst = &plan.installer;
    let by_entries = apps.iter().find(|app| {
        inst.arp_entries.iter().any(|e| {
            same_code(e.product_code.as_deref(), app.product_code.as_deref())
                || same_code(e.upgrade_code.as_deref(), app.upgrade_code.as_deref())
        })
    });
    if by_entries.is_some() {
        return by_entries;
    }
    if let Some(app) = apps
        .iter()
        .find(|app| same_code(inst.product_code.as_deref(), app.product_code.as_deref()))
    {
        return Some(app);
    }
    if matches!(inst.kind, InstallerKind::Msix | InstallerKind::Appx)
        && let Some(pfn) = inst
            .package_family_name
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        && let Some(app) = apps.iter().find(|app| {
            app.family_name
                .as_deref()
                .is_some_and(|f| f.trim().eq_ignore_ascii_case(pfn))
        })
    {
        return Some(app);
    }
    by_name_and_publisher(plan, apps)
}

/// Same keys as `installed::correlate::by_name_and_publisher`: ARP entries only, both the
/// normalized name and publisher non-empty and equal.
fn by_name_and_publisher<'a>(
    plan: &WingetPlan,
    apps: &'a [InstalledApp],
) -> Option<&'a InstalledApp> {
    let norm = NameNormalizer::new();
    let mut wanted: Vec<(String, String)> = Vec::new();
    let mut push = |name: Option<&str>, publisher: Option<&str>| {
        if let (Some(n), Some(p)) = (name, publisher) {
            let k = norm.index_keys(n, p);
            if !k.name.is_empty() && !k.publisher.is_empty() {
                wanted.push((k.name, k.publisher));
            }
        }
    };
    for e in &plan.installer.arp_entries {
        push(
            e.display_name.as_deref(),
            e.publisher.as_deref().or(plan.publisher.as_deref()),
        );
    }
    push(Some(&plan.name), plan.publisher.as_deref());
    if wanted.is_empty() {
        return None;
    }
    apps.iter().find(|app| {
        if !matches!(app.source, InstallSource::Arp(_)) {
            return false;
        }
        let k = norm.index_keys(&app.name, app.publisher.as_deref().unwrap_or(""));
        !k.name.is_empty()
            && !k.publisher.is_empty()
            && wanted
                .iter()
                .any(|(n, p)| *n == k.name && *p == k.publisher)
    })
}

/// After an upgrade: the warning when the app found is still below the version offered (the
/// installer exited 0 without changing it). A later version than the catalog's counts as
/// updated; an unknown version says nothing.
pub fn version_warning(offered: &str, found: Option<&InstalledApp>) -> Option<String> {
    let have = found?
        .version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    (Version::parse(have) < Version::parse(offered))
        .then(|| format!("o instalador terminou, mas a versão instalada continua {have}"))
}
