//! Scoop installed apps from the on-disk layout.
use super::{InstallSource, InstalledApp};
use std::path::Path;

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn read_one(app_dir: &Path) -> Option<InstalledApp> {
    let cur = app_dir.join("current");
    let manifest = read_json(&cur.join("manifest.json"))?;
    let install = read_json(&cur.join("install.json"))?;
    let key = app_dir.file_name()?.to_str()?.to_string();
    Some(InstalledApp {
        source: InstallSource::Scoop,
        name: key.clone(),
        key,
        publisher: None,
        version: manifest
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        product_code: None,
        upgrade_code: None,
        family_name: None,
        scoop_bucket: install
            .get("bucket")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        stale: None,
    })
}

/// Lists apps under `<root>/apps/*/current`; missing root or broken apps are skipped.
pub fn read_scoop(root: &Path) -> Vec<InstalledApp> {
    let Ok(rd) = std::fs::read_dir(root.join("apps")) else {
        return Vec::new();
    };
    let mut apps: Vec<InstalledApp> = rd.flatten().filter_map(|e| read_one(&e.path())).collect();
    apps.sort_by(|a, b| a.key.cmp(&b.key));
    apps
}

/// `%SCOOP%` or `~/scoop`.
pub fn scoop_root() -> Option<std::path::PathBuf> {
    if let Some(v) = std::env::var_os("SCOOP").filter(|v| !v.is_empty()) {
        return Some(v.into());
    }
    directories::UserDirs::new().map(|d| d.home_dir().join("scoop"))
}
