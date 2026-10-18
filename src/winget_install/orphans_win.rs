//! reg export and key deletion (Task 183).
use super::arp_read_win::read_details;
use super::orphans::ArpRegistry;
use super::uninstall_cmd::valid_arp_key;
use crate::installed::{Hive, missing_uninstaller};
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_System, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, REG_OPTION_OPEN_LINK};

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

/// The real registry (unelevated).
pub struct SystemArp;

/// `System32` from the known folder, never from `%SystemRoot%` or PATH.
fn system32() -> io::Result<PathBuf> {
    // SAFETY: the returned string is freed below with CoTaskMemFree.
    let p = unsafe { SHGetKnownFolderPath(&FOLDERID_System, KF_FLAG_DEFAULT, None) }
        .map_err(|e| io::Error::other(e.to_string()))?;
    // SAFETY: a NUL-terminated string from the call above.
    let text = unsafe { p.to_string() };
    // SAFETY: allocated by SHGetKnownFolderPath, freed once.
    unsafe { CoTaskMemFree(Some(p.0 as *const core::ffi::c_void)) };
    text.map(PathBuf::from)
        .map_err(|e| io::Error::other(e.to_string()))
}

impl ArpRegistry for SystemArp {
    fn export(&self, args: &[String]) -> io::Result<()> {
        let out = Command::new(system32()?.join("reg.exe"))
            .args(args)
            .stdin(Stdio::null())
            .output()?;
        if out.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "reg.exe saiu com {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }

    fn delete_user_key(&self, key: &str) -> io::Result<()> {
        if !valid_arp_key(key) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "nome de chave ARP inválido",
            ));
        }
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let full = format!(r"{UNINSTALL}\{key}");
        // Refuse to delete through a registry symbolic link (as `install::registry_win`).
        let k =
            hkcu.open_subkey_with_options_flags(&full, REG_OPTION_OPEN_LINK, KEY_QUERY_VALUE)?;
        if k.enum_values()
            .flatten()
            .any(|(n, _)| n == "SymbolicLinkValue")
        {
            return Err(io::Error::other("link simbólico de registro recusado"));
        }
        drop(k);
        hkcu.delete_subkey_all(full)
    }

    fn still_orphan(&self, hive: Hive, key: &str) -> bool {
        let Ok(Some(d)) = read_details(hive, key) else {
            return false;
        };
        let Some(cmd) = d.uninstall_string.as_deref() else {
            return false;
        };
        // Same expansion and existence rule as the scan that marked it (`installed::arp_win`).
        missing_uninstaller(
            cmd,
            |v| std::env::var(v).ok(),
            |p| !matches!(p.try_exists(), Ok(false)),
        )
        .is_some()
    }
}
