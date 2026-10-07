//! Pure ARP (Add/Remove Programs) filtering.
use super::{Hive, InstallSource, InstalledApp, RawArpEntry};
use std::collections::HashMap;

const SKIPPED_RELEASE_TYPES: [&str; 4] = ["update", "hotfix", "security update", "service pack"];

fn non_blank(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Ported rules from winget's ARP enumeration: hide system components, children, updates.
pub fn filter_arp(
    raw: RawArpEntry,
    upgrade_codes: &HashMap<String, String>,
) -> Option<InstalledApp> {
    let name = non_blank(raw.display_name)?;
    if raw.system_component == Some(1) || raw.parent_key_name.is_some() {
        return None;
    }
    if let Some(rt) = &raw.release_type {
        let rt = rt.trim().to_ascii_lowercase();
        if SKIPPED_RELEASE_TYPES.contains(&rt.as_str()) {
            return None;
        }
    }
    let upgrade_code = if raw.windows_installer == Some(1) {
        upgrade_codes
            .get(&raw.key_name.to_ascii_uppercase())
            .cloned()
    } else {
        None
    };
    Some(InstalledApp {
        source: InstallSource::Arp(raw.hive.unwrap_or(Hive::MachineX64)),
        key: raw.key_name.clone(),
        name,
        publisher: non_blank(raw.publisher),
        version: non_blank(raw.display_version),
        product_code: Some(raw.key_name),
        upgrade_code,
        family_name: None,
        scoop_bucket: None,
    })
}
