//! MSIX/AppX packages via the WinRT PackageManager.
use super::{InstallSource, InstalledApp};
use windows::ApplicationModel::{Package, PackageSignatureKind};
use windows::Management::Deployment::PackageManager;
use windows::core::HSTRING;

const WORKERS: usize = 8;

fn read_one(p: &Package) -> Option<InstalledApp> {
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
    let display = p.DisplayName().ok().map(|n| n.to_string());
    let name = super::msix_display_name(display.as_deref(), &id_name);
    let version = id
        .Version()
        .ok()
        .map(|v| format!("{}.{}.{}.{}", v.Major, v.Minor, v.Build, v.Revision));
    let publisher = p
        .PublisherDisplayName()
        .ok()
        .map(|n| n.to_string())
        .filter(|n| !n.is_empty());
    Some(InstalledApp {
        source: InstallSource::Msix,
        key: family.clone(),
        name,
        publisher,
        version,
        product_code: None,
        upgrade_code: None,
        family_name: Some(family),
        scoop_bucket: None,
    })
}

/// Per-package property reads are slow (resource lookups), so they run on a few workers.
pub fn read_appx() -> Result<Vec<InstalledApp>, String> {
    let pm = PackageManager::new().map_err(|e| e.to_string())?;
    let pkgs: Vec<Package> = pm
        .FindPackagesByUserSecurityId(&HSTRING::new())
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    let chunk = pkgs.len().div_ceil(WORKERS).max(1);
    let mut out = Vec::new();
    std::thread::scope(|s| {
        let handles: Vec<_> = pkgs
            .chunks(chunk)
            .map(|c| s.spawn(move || c.iter().filter_map(read_one).collect::<Vec<_>>()))
            .collect();
        for h in handles {
            if let Ok(mut v) = h.join() {
                out.append(&mut v);
            }
        }
    });
    Ok(out)
}
