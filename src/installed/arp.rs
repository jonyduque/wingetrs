//! Pure ARP (Add/Remove Programs) filtering.
use super::{Hive, InstallSource, InstalledApp, RawArpEntry};
use std::collections::HashMap;

const SKIPPED_RELEASE_TYPES: [&str; 4] = ["update", "hotfix", "security update", "service pack"];

/// Registry value type codes (winnt.h), kept here so the conversion needs no `winreg`.
pub const REG_SZ: u32 = 1;
pub const REG_EXPAND_SZ: u32 = 2;
pub const REG_DWORD: u32 = 4;

/// Only REG_SZ / REG_EXPAND_SZ count as strings; anything else is treated as absent.
/// Tolerates odd lengths (trailing byte dropped) and a missing NUL terminator.
pub fn reg_string(vtype: u32, bytes: &[u8]) -> Option<String> {
    if vtype != REG_SZ && vtype != REG_EXPAND_SZ {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Some(
        String::from_utf16_lossy(&units)
            .trim_end_matches('\0')
            .to_string(),
    )
}

/// REG_DWORD with at least 4 bytes; anything else is treated as absent.
pub fn reg_dword(vtype: u32, bytes: &[u8]) -> Option<u32> {
    if vtype != REG_DWORD {
        return None;
    }
    let b = bytes.get(..4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

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
