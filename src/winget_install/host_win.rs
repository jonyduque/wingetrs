//! Host architecture and OS facts (Task 173).
use super::select::Host;
use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

const NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

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

// GetUserPreferredUILanguages (kernel32); declared here because the `windows` feature for
// Win32_Globalization is not enabled in Cargo.toml.
unsafe extern "system" {
    fn GetUserPreferredUILanguages(
        flags: u32,
        num_languages: *mut u32,
        buffer: *mut u16,
        buffer_len: *mut u32,
    ) -> i32;
}

const MUI_LANGUAGE_NAME: u32 = 0x8;
/// Ceiling on the multi-string buffer (UTF-16 units) before allocating.
const MAX_LANGUAGES_BUF: u32 = 16 * 1024;

/// The user's preferred UI languages in order (`pt-BR`, `en-US`, ...), as winget uses them.
fn locale() -> std::io::Result<Vec<String>> {
    let mut count = 0u32;
    let mut len = 0u32;
    // SAFETY: a null buffer asks only for the required length; both out-pointers are live locals.
    let ok = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            std::ptr::null_mut(),
            &mut len,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if len == 0 || len > MAX_LANGUAGES_BUF {
        return Err(std::io::Error::other(
            "lista de idiomas vazia ou grande demais",
        ));
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` holds `len` UTF-16 units, the size the call itself requested.
    let ok = unsafe {
        GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, buf.as_mut_ptr(), &mut len)
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let langs = parse_multi_string(&buf[..(len as usize).min(buf.len())]);
    if langs.is_empty() {
        return Err(std::io::Error::other("lista de idiomas vazia"));
    }
    Ok(langs)
}

/// Splits a double-NUL-terminated UTF-16 multi-string, dropping empty entries.
fn parse_multi_string(buf: &[u16]) -> Vec<String> {
    buf.split(|&c| c == 0)
        .filter(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
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
        tracing::warn!("idiomas preferidos ilegíveis ({e}); assumindo en-US");
        vec!["en-US".to_string()]
    });
    Host {
        arch: native_arch()
            .unwrap_or_else(crate::bridge::host_arch)
            .to_string(),
        os,
        locale: locale.into(),
    }
}
