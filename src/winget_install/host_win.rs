//! Host architecture and OS facts (Task 173).
use super::select::Host;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

const NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const INTERNATIONAL: &str = r"Control Panel\International";

fn os_version() -> std::io::Result<[u32; 4]> {
    let k = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(NT, KEY_READ | KEY_WOW64_64KEY)?;
    let build: String = k.get_value("CurrentBuildNumber")?;
    let build = build
        .trim()
        .parse()
        .map_err(|_| std::io::Error::other("CurrentBuildNumber não numérico"))?;
    Ok([
        k.get_value("CurrentMajorVersionNumber")?,
        k.get_value("CurrentMinorVersionNumber")?,
        build,
        k.get_value("UBR")?,
    ])
}

fn locale() -> std::io::Result<String> {
    let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(INTERNATIONAL, KEY_READ)?;
    let name: String = k.get_value("LocaleName")?;
    if name.trim().is_empty() {
        return Err(std::io::Error::other("LocaleName vazio"));
    }
    Ok(name.trim().to_string())
}

/// Native architecture of the machine, even for an emulated process (an x64 build on ARM64
/// reports `arm64`; the x64 installers then come from the fallback list).
fn native_arch() -> Option<&'static str> {
    use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE;
    use windows::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
    let mut process = IMAGE_FILE_MACHINE(0);
    let mut native = IMAGE_FILE_MACHINE(0);
    // SAFETY: both out-pointers refer to live locals; the pseudo handle needs no closing.
    unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, Some(&mut native)) }.ok()?;
    match native.0 {
        0xAA64 => Some("arm64"),
        0x8664 => Some("x64"),
        0x014c => Some("x86"),
        _ => None,
    }
}

/// This machine's facts; a failed registry read falls back to `[10,0,0,0]` / `en-US`.
pub fn current() -> Host {
    let os = os_version().unwrap_or_else(|e| {
        tracing::warn!("versão do Windows ilegível ({e}); assumindo 10.0.0.0");
        [10, 0, 0, 0]
    });
    let locale = locale().unwrap_or_else(|e| {
        tracing::warn!("locale ilegível ({e}); assumindo en-US");
        "en-US".to_string()
    });
    Host {
        arch: native_arch()
            .unwrap_or_else(crate::bridge::host_arch)
            .to_string(),
        os,
        locale,
    }
}
