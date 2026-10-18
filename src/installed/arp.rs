//! Pure ARP (Add/Remove Programs) filtering.
use super::{Hive, InstallSource, InstalledApp, RawArpEntry};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
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
        stale: None,
    })
}

/// Names that run other things: a missing one says nothing about the uninstaller.
const GENERIC_LAUNCHERS: [&str; 6] = [
    "msiexec.exe",
    "rundll32.exe",
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "winget.exe",
];

/// Expands `%VAR%` with `expand`; `None` when a variable is unknown or unbalanced.
pub fn expand_vars(s: &str, expand: &impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        let j = tail.find('%')?;
        out.push_str(&expand(&tail[..j])?);
        rest = &tail[j + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// Splits a command line into (program without expansion, trimmed rest): the quoted program, or
/// up to the first `.exe` that closes a token (a name like foo.exe.d is not cut).
pub fn split_command(cmd: &str) -> Option<(String, String)> {
    let cmd = cmd.trim();
    if let Some(rest) = cmd.strip_prefix('"') {
        let close = rest.find('"')?;
        return Some((
            rest[..close].to_string(),
            rest[close + 1..].trim().to_string(),
        ));
    }
    let lower = cmd.to_ascii_lowercase();
    let end = lower.match_indices(".exe").find_map(|(i, m)| {
        let end = i + m.len();
        let next = cmd[end..].chars().next();
        next.is_none_or(|c| c.is_whitespace() || c == '/')
            .then_some(end)
    })?;
    Some((cmd[..end].to_string(), cmd[end..].trim().to_string()))
}

/// The executable path of an uninstall command when it is absolute, not a generic launcher
/// and does not exist; `None` whenever nothing can be claimed.
pub fn missing_uninstaller(
    cmd: &str,
    expand: impl Fn(&str) -> Option<String>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let (raw, _) = split_command(cmd)?;
    let expanded = expand_vars(&raw, &expand)?;
    let b = expanded.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        return None;
    }
    let path = PathBuf::from(expanded);
    let file = path.file_name()?.to_str()?.to_ascii_lowercase();
    if GENERIC_LAUNCHERS.contains(&file.as_str()) || exists(&path) {
        return None;
    }
    Some(path)
}
