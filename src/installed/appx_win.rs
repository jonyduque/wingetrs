//! MSIX/AppX packages via the WinRT PackageManager.
use super::{InstallSource, InstalledApp};
use crate::core::version::Version;
use std::collections::HashMap;
use windows::ApplicationModel::{Package, PackageSignatureKind};
use windows::Management::Deployment::PackageManager;
use windows::core::HSTRING;

const WORKERS: usize = 8;

/// Cheap identity of a package (no resource lookups): family, id name and version.
struct Candidate {
    pkg: Package,
    family: String,
    id_name: String,
    version: String,
}

fn identify(p: &Package) -> Option<Candidate> {
    if p.IsResourcePackage().unwrap_or(false) {
        return None;
    }
    if p.SignatureKind().ok() == Some(PackageSignatureKind::System) {
        return None;
    }
    let id = p.Id().ok()?;
    let family = id.FamilyName().ok()?.to_string();
    let id_name = id
        .Name()
        .ok()
        .map(|n| n.to_string())
        .unwrap_or_else(|| family.clone());
    let version = id
        .Version()
        .ok()
        .map(|v| format!("{}.{}.{}.{}", v.Major, v.Minor, v.Build, v.Revision))
        .unwrap_or_default();
    Some(Candidate {
        pkg: p.clone(),
        family,
        id_name,
        version,
    })
}

/// Slow reads (display name, publisher) happen only for the winning package of a family.
fn read_one(c: &Candidate) -> InstalledApp {
    let display = c.pkg.DisplayName().ok().map(|n| n.to_string());
    let publisher = c
        .pkg
        .PublisherDisplayName()
        .ok()
        .map(|n| n.to_string())
        .filter(|n| !n.is_empty());
    InstalledApp {
        source: InstallSource::Msix,
        key: c.family.clone(),
        name: super::msix_display_name(display.as_deref(), &c.id_name),
        publisher,
        version: if c.version.is_empty() {
            None
        } else {
            Some(c.version.clone())
        },
        product_code: None,
        upgrade_code: None,
        family_name: Some(c.family.clone()),
        scoop_bucket: None,
        stale: None,
    }
}

fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> Option<R> + Sync) -> Vec<R> {
    super::par::parallel(items, WORKERS, f)
}

pub fn read_appx() -> Result<Vec<InstalledApp>, String> {
    let pkgs: Vec<Package> = super::timed("scan_msix_enumerate", || {
        let pm = PackageManager::new().map_err(|e| e.to_string())?;
        Ok::<_, String>(
            pm.FindPackagesByUserSecurityId(&HSTRING::new())
                .map_err(|e| e.to_string())?
                .into_iter()
                .collect(),
        )
    })?;
    let cands = super::timed("scan_msix_identify", || parallel(&pkgs, identify));
    // Highest version per family; first seen wins ties (same as dedupe_msix_by_family).
    let mut best: HashMap<String, usize> = HashMap::new();
    let mut order: Vec<usize> = Vec::new();
    for (i, c) in cands.iter().enumerate() {
        match best.get(&c.family.to_ascii_lowercase()) {
            Some(&j) => {
                if Version::parse(&c.version) > Version::parse(&cands[j].version) {
                    let slot = order.iter().position(|&k| k == j);
                    if let Some(slot) = slot {
                        order[slot] = i;
                    }
                    best.insert(c.family.to_ascii_lowercase(), i);
                }
            }
            None => {
                best.insert(c.family.to_ascii_lowercase(), i);
                order.push(i);
            }
        }
    }
    let winners: Vec<&Candidate> = order.iter().map(|&i| &cands[i]).collect();
    Ok(super::timed("scan_msix_props", || {
        parallel(&winners, |c| Some(read_one(c)))
    }))
}
