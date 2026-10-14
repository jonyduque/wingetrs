//! Windows registry reading for ARP entries and MSI upgrade codes.
use super::arp::{reg_dword, reg_string};
use super::{Hive, InstalledApp, RawArpEntry, filter_arp, missing_uninstaller, unpack_guid};
use std::collections::HashMap;
use std::path::Path;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
use winreg::{RegKey, RegValue};

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_X86: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
const UPGRADE_CODES: &str = r"SOFTWARE\Classes\Installer\UpgradeCodes";

fn as_string(v: RegValue) -> Option<String> {
    reg_string(v.vtype as u32, &v.bytes)
}

fn as_dword(v: RegValue) -> Option<u32> {
    reg_dword(v.vtype as u32, &v.bytes)
}

/// ProductCode -> UpgradeCode, both unpacked.
fn upgrade_code_map() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(root) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(UPGRADE_CODES, KEY_READ | KEY_WOW64_64KEY)
    else {
        return map;
    };
    for packed_upgrade in root.enum_keys().flatten() {
        let Some(upgrade) = unpack_guid(&packed_upgrade) else {
            continue;
        };
        let Ok(k) = root.open_subkey_with_flags(&packed_upgrade, KEY_READ) else {
            continue;
        };
        for (packed_product, _) in k.enum_values().flatten() {
            if let Some(product) = unpack_guid(&packed_product) {
                map.insert(product, upgrade.clone());
            }
        }
    }
    map
}

fn read_hive(
    root: &RegKey,
    path: &str,
    flags: u32,
    hive: Hive,
    codes: &HashMap<String, String>,
    out: &mut Vec<InstalledApp>,
) {
    let Ok(base) = root.open_subkey_with_flags(path, flags) else {
        return;
    };
    let step = match hive {
        Hive::MachineX64 => "scan_arp_x64",
        Hive::MachineX86 => "scan_arp_x86",
        Hive::User => "scan_arp_user",
    };
    super::timed(step, || read_keys(&base, hive, codes, out));
}

/// Reads the disk only: the uninstaller path from `UninstallString` when that file is gone.
fn stale_uninstaller(cmd: Option<&str>) -> Option<String> {
    missing_uninstaller(cmd?, |v| std::env::var(v).ok(), Path::exists)
        .map(|p| p.to_string_lossy().into_owned())
}

fn read_keys(
    base: &RegKey,
    hive: Hive,
    codes: &HashMap<String, String>,
    out: &mut Vec<InstalledApp>,
) {
    for name in base.enum_keys().flatten() {
        let Ok(k) = base.open_subkey_with_flags(&name, KEY_READ) else {
            continue;
        };
        let s = |n: &str| k.get_raw_value(n).ok().and_then(as_string);
        let d = |n: &str| k.get_raw_value(n).ok().and_then(as_dword);
        let raw = RawArpEntry {
            hive: Some(hive),
            key_name: name.clone(),
            display_name: s("DisplayName"),
            publisher: s("Publisher"),
            display_version: s("DisplayVersion"),
            windows_installer: d("WindowsInstaller"),
            system_component: d("SystemComponent"),
            parent_key_name: s("ParentKeyName"),
            release_type: s("ReleaseType"),
            uninstall_string: s("UninstallString"),
        };
        let uninstall = raw.uninstall_string.clone();
        if let Some(mut app) = filter_arp(raw, codes) {
            app.stale = stale_uninstaller(uninstall.as_deref());
            out.push(app);
        }
    }
}

pub fn read_arp() -> Result<Vec<InstalledApp>, String> {
    let codes = super::timed("scan_upgrade_codes", upgrade_code_map);
    let mut out = Vec::new();
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    read_hive(
        &hklm,
        UNINSTALL,
        KEY_READ | KEY_WOW64_64KEY,
        Hive::MachineX64,
        &codes,
        &mut out,
    );
    read_hive(
        &hklm,
        UNINSTALL_X86,
        KEY_READ | KEY_WOW64_64KEY,
        Hive::MachineX86,
        &codes,
        &mut out,
    );
    read_hive(
        &RegKey::predef(HKEY_CURRENT_USER),
        UNINSTALL,
        KEY_READ,
        Hive::User,
        &codes,
        &mut out,
    );
    Ok(out)
}
