//! Reads one ARP key with an explicit view (Task 172).
use super::uninstall_cmd::{ArpDetails, valid_arp_key};
use crate::installed::Hive;
use crate::installed::arp::{reg_dword, reg_string};
use std::io;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
use winreg::{HKEY, RegKey};

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_X86: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";

/// The `Uninstall` key of `hive` and the flag that picks its view: both HKLM views are opened
/// with `KEY_WOW64_64KEY` on their own path (the 32-bit one under `WOW6432Node`), so a 32-bit
/// build reads the same keys as a 64-bit one; HKCU has a single view.
pub fn uninstall_root(hive: Hive) -> (HKEY, &'static str, u32) {
    match hive {
        Hive::MachineX64 => (HKEY_LOCAL_MACHINE, UNINSTALL, KEY_WOW64_64KEY),
        Hive::MachineX86 => (HKEY_LOCAL_MACHINE, UNINSTALL_X86, KEY_WOW64_64KEY),
        Hive::User => (HKEY_CURRENT_USER, UNINSTALL, 0),
    }
}

/// Reads one ARP key; `Ok(None)` when it does not exist.
pub fn read_details(hive: Hive, key: &str) -> io::Result<Option<ArpDetails>> {
    if !valid_arp_key(key) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "nome de chave ARP invalido",
        ));
    }
    let (root, base, view) = uninstall_root(hive);
    let path = format!(r"{base}\{key}");
    let k = match RegKey::predef(root).open_subkey_with_flags(path, KEY_READ | view) {
        Ok(k) => k,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(Some(details_of(&k, hive, key)))
}

/// The ARP values of the open key `k` (named `key` in `hive`); needs `KEY_QUERY_VALUE`.
pub fn details_of(k: &RegKey, hive: Hive, key: &str) -> ArpDetails {
    let s = |n: &str| {
        k.get_raw_value(n)
            .ok()
            .and_then(|v| reg_string(v.vtype as u32, &v.bytes))
    };
    let d = |n: &str| {
        k.get_raw_value(n)
            .ok()
            .and_then(|v| reg_dword(v.vtype as u32, &v.bytes))
    };
    ArpDetails {
        hive,
        key: key.to_string(),
        display_name: s("DisplayName"),
        publisher: s("Publisher"),
        display_version: s("DisplayVersion"),
        windows_installer: d("WindowsInstaller") == Some(1),
        system_component: d("SystemComponent") == Some(1),
        quiet_uninstall_string: s("QuietUninstallString"),
        uninstall_string: s("UninstallString"),
        inno_app_path: s("Inno Setup: App Path"),
        bundle_cache_path: s("BundleCachePath"),
        bundle_upgrade_code: s("BundleUpgradeCode"),
        install_location: s("InstallLocation"),
    }
}

/// Splits a command with the shell's own rule (only for the live equivalence test).
pub fn shell_split(cmd: &str) -> io::Result<(String, String)> {
    use windows::Win32::UI::Shell::SHEvaluateSystemCommandTemplate;
    use windows::core::{HSTRING, PWSTR};
    let mut exe = PWSTR::null();
    let mut params = PWSTR::null();
    // SAFETY: out pointers are valid; returned buffers are CoTaskMem and freed below.
    unsafe {
        SHEvaluateSystemCommandTemplate(&HSTRING::from(cmd), &mut exe, None, Some(&mut params))
            .map_err(|e| io::Error::other(e.to_string()))?;
        let to_s = |p: PWSTR| {
            if p.is_null() {
                String::new()
            } else {
                p.to_string().unwrap_or_default()
            }
        };
        let r = (to_s(exe), to_s(params));
        for p in [exe, params] {
            if !p.is_null() {
                windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
            }
        }
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstall_root_picks_the_view_of_each_hive() {
        assert_eq!(
            uninstall_root(Hive::MachineX64),
            (HKEY_LOCAL_MACHINE, UNINSTALL, KEY_WOW64_64KEY)
        );
        assert_eq!(
            uninstall_root(Hive::MachineX86),
            (HKEY_LOCAL_MACHINE, UNINSTALL_X86, KEY_WOW64_64KEY)
        );
        assert_eq!(
            uninstall_root(Hive::User),
            (HKEY_CURRENT_USER, UNINSTALL, 0)
        );
        assert!(UNINSTALL_X86.contains(r"\WOW6432Node\"));
        assert!(!UNINSTALL.contains("WOW6432Node"));
    }
}
