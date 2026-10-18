//! Rules the elevated helper applies to an HKLM ARP entry before it runs its uninstaller or
//! deletes its key (Task 182). Pure: the folders come from the caller (known folders on
//! Windows), never from the environment.
use super::uninstall_cmd::ArpDetails;
use crate::core::version::Version;
use crate::installed::Hive;
use crate::installed::arp::{expand_vars, missing_uninstaller};
use std::path::{Component, Path, PathBuf};

/// The system folders the helper trusts, all from `SHGetKnownFolderPath`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownFolders {
    pub program_files: PathBuf,
    pub program_files_x86: PathBuf,
    pub system_root: PathBuf,
    pub program_data: PathBuf,
}

/// Folders under `system_root` that a standard user may create files in.
const USER_WRITABLE_UNDER_WINDOWS: [&str; 6] = [
    "Temp",
    "Tasks",
    "tracing",
    r"System32\Tasks",
    r"System32\spool",
    r"SysWOW64\Tasks",
];

/// The value of one `%name%` from the known folders; `None` for any other variable.
fn known_var(name: &str, k: &KnownFolders) -> Option<String> {
    let common = |p: &Path| p.join("Common Files");
    let path = match name.to_ascii_lowercase().as_str() {
        "programfiles" | "programw6432" => k.program_files.clone(),
        "programfiles(x86)" => k.program_files_x86.clone(),
        "commonprogramfiles" => common(&k.program_files),
        "commonprogramfiles(x86)" => common(&k.program_files_x86),
        "systemroot" | "windir" => k.system_root.clone(),
        "programdata" => k.program_data.clone(),
        _ => return None,
    };
    Some(path.to_string_lossy().into_owned())
}

/// `raw` with its `%variables%` expanded from the known folders only; `None` when any other
/// variable appears or a `%` is left open.
pub fn expand_known(raw: &str, k: &KnownFolders) -> Option<String> {
    expand_vars(raw, &|name: &str| known_var(name, k))
}

/// The components of `p` when it is absolute and has no `.`/`..` and no name ending in a dot
/// or a space (Windows drops those, so `Temp.` would be `Temp`).
fn plain_components(p: &Path) -> Option<Vec<String>> {
    if !p.is_absolute() {
        return None;
    }
    p.components()
        .map(|c| match c {
            Component::CurDir | Component::ParentDir => None,
            Component::Normal(n) => {
                let n = n.to_string_lossy();
                (!n.ends_with(['.', ' '])).then(|| n.to_uppercase())
            }
            other => Some(other.as_os_str().to_string_lossy().to_uppercase()),
        })
        .collect()
}

/// The absolute paths written in a command's arguments: every `X:\`/`X:/` (not preceded by a
/// letter or digit) or `\\` up to the end of its token (quotes group a token and are dropped).
pub fn arg_paths(args: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in args.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    let mut out = Vec::new();
    for t in &tokens {
        let b = t.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let drive = i + 2 < b.len()
                && b[i].is_ascii_alphabetic()
                && b[i + 1] == b':'
                && matches!(b[i + 2], b'\\' | b'/')
                && (i == 0 || !b[i - 1].is_ascii_alphanumeric());
            let unc = b[i] == b'\\' && b.get(i + 1) == Some(&b'\\');
            if drive || unc {
                out.push(t[i..].to_string());
                break;
            }
            i += 1;
        }
    }
    out
}

/// Whether every absolute path in `args` (see `arg_paths`) is in an administrator folder.
pub fn args_in_admin_folder(args: &str, k: &KnownFolders) -> bool {
    arg_paths(args)
        .iter()
        .all(|p| in_admin_folder(Path::new(p), k))
}

/// Which object of an executable's chain a security descriptor belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AclLevel {
    /// The executable itself.
    File,
    /// The folder that holds it (where its DLLs and `.local` redirection are looked up).
    Parent,
    /// Any folder above that, up to the volume root.
    Ancestor,
}

/// `FILE_WRITE_DATA`/`FILE_ADD_FILE`.
const WRITE_DATA: u32 = 0x2;
/// `FILE_APPEND_DATA`/`FILE_ADD_SUBDIRECTORY`.
const APPEND_DATA: u32 = 0x4;
const DELETE_CHILD: u32 = 0x40;
const DELETE: u32 = 0x1_0000;
const WRITE_DAC: u32 = 0x4_0000;
const WRITE_OWNER: u32 = 0x8_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;

/// Rights that let someone change what runs, per level. Above the parent, adding files or
/// sub-folders is harmless (every Windows grants Authenticated Users `LC` on `C:\`); renaming,
/// deleting or re-ACLing a folder is not.
fn dangerous(level: AclLevel) -> u32 {
    let replace = DELETE | WRITE_DAC | WRITE_OWNER | GENERIC_ALL;
    match level {
        AclLevel::File => replace | WRITE_DATA | APPEND_DATA | GENERIC_WRITE,
        AclLevel::Parent => replace | WRITE_DATA | APPEND_DATA | DELETE_CHILD | GENERIC_WRITE,
        AclLevel::Ancestor => replace | DELETE_CHILD,
    }
}

/// TrustedInstaller.
const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

/// SYSTEM, Administrators, TrustedInstaller and OWNER RIGHTS (the owner is one of those).
fn trusted_sid(sid: &str) -> bool {
    let sid = sid.trim();
    ["SY", "S-1-5-18", "BA", "S-1-5-32-544", "OW", "S-1-3-4"].contains(&sid)
        || sid.eq_ignore_ascii_case(TRUSTED_INSTALLER)
}

/// An SDDL access mask (`0x...` or two-letter codes); `None` for a code not known here.
fn sddl_rights(text: &str) -> Option<u32> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    if text.is_empty() || !text.len().is_multiple_of(2) {
        return None;
    }
    let mut mask = 0u32;
    for code in text.as_bytes().chunks(2) {
        mask |= match code {
            b"GA" => GENERIC_ALL,
            b"GR" => 0x8000_0000,
            b"GW" => GENERIC_WRITE,
            b"GX" => 0x2000_0000,
            b"RC" => 0x2_0000,
            b"SD" => DELETE,
            b"WD" => WRITE_DAC,
            b"WO" => WRITE_OWNER,
            b"FA" => 0x1F_01FF,
            b"FR" => 0x12_0089,
            b"FW" => 0x12_0116,
            b"FX" => 0x12_00A0,
            b"CC" => 0x1,
            b"DC" => WRITE_DATA,
            b"LC" => APPEND_DATA,
            b"SW" => 0x8,
            b"RP" => 0x10,
            b"WP" => 0x20,
            b"DT" => DELETE_CHILD,
            b"LO" => 0x80,
            b"CR" => 0x100,
            _ => return None,
        };
    }
    Some(mask)
}

/// The sections of an SDDL string (`O`, `G`, `D`, `S`), split outside parentheses.
fn sddl_sections(sddl: &str) -> Vec<(char, &str)> {
    let b = sddl.as_bytes();
    let mut starts = Vec::new();
    let mut depth = 0i32;
    for i in 0..b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            c if depth == 0
                && matches!(c, b'O' | b'G' | b'D' | b'S')
                && b.get(i + 1) == Some(&b':') =>
            {
                starts.push(i);
            }
            _ => {}
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(n, &s)| {
            let end = starts.get(n + 1).copied().unwrap_or(sddl.len());
            (char::from(b[s]), &sddl[s + 2..end])
        })
        .collect()
}

/// Whether the security descriptor `sddl` (owner and DACL, as
/// `ConvertSecurityDescriptorToStringSecurityDescriptorW` writes it) leaves the object to
/// administrators only: the owner is SYSTEM, Administrators or TrustedInstaller, there is a
/// DACL, and no allow ACE that applies to the object gives another SID a right of
/// `dangerous(level)`. Anything not understood refuses.
pub fn sddl_admin_only(sddl: &str, level: AclLevel) -> bool {
    let sections = sddl_sections(sddl);
    let section = |name: char| sections.iter().find(|(c, _)| *c == name).map(|(_, v)| *v);
    let (Some(owner), Some(dacl)) = (section('O'), section('D')) else {
        return false;
    };
    if !trusted_sid(owner) || dacl.contains("NO_ACCESS_CONTROL") {
        return false;
    }
    let Some(open) = dacl.find('(') else {
        // Flags only: an empty DACL grants nothing.
        return !dacl.contains(')');
    };
    let bad = dangerous(level);
    let mut rest = &dacl[open..];
    while let Some(body) = rest.strip_prefix('(') {
        let Some(close) = body.find(')') else {
            return false;
        };
        let fields: Vec<&str> = body[..close].split(';').collect();
        rest = &body[close + 1..];
        let [kind, flags, rights, _, _, sid, ..] = fields.as_slice() else {
            return false;
        };
        let allow = match *kind {
            "A" | "XA" | "OA" | "ZA" => true,
            "D" | "XD" | "OD" => false,
            _ => return false,
        };
        // Inherit-only ACEs only reach children created later, never this object.
        if !allow || flags.as_bytes().chunks(2).any(|f| f == b"IO") {
            continue;
        }
        let Some(mask) = sddl_rights(rights) else {
            return false;
        };
        if mask & bad != 0 && !trusted_sid(sid) {
            return false;
        }
    }
    rest.is_empty()
}

/// Whether `p` lies strictly under `root`, comparing whole components without case.
fn under(p: &[String], root: &Path) -> bool {
    let Some(root) = plain_components(root) else {
        return false;
    };
    p.len() > root.len() && root.iter().zip(p).all(|(a, b)| a == b)
}

/// Whether `p` is inside a folder only administrators write: `Program Files` (both),
/// `Windows` (but not its user-writable folders such as `Temp`) or `ProgramData\Package Cache`.
pub fn in_admin_folder(p: &Path, k: &KnownFolders) -> bool {
    let Some(parts) = plain_components(p) else {
        return false;
    };
    if USER_WRITABLE_UNDER_WINDOWS
        .iter()
        .any(|rel| under(&parts, &k.system_root.join(rel)))
    {
        return false;
    }
    [
        k.program_files.clone(),
        k.program_files_x86.clone(),
        k.system_root.clone(),
        k.program_data.join("Package Cache"),
    ]
    .iter()
    .any(|root| under(&parts, root))
}

/// Whether the entry's `UninstallString` still names an absolute uninstaller that does not
/// exist (expanded with the known folders only).
pub fn still_orphan(d: &ArpDetails, k: &KnownFolders, exists: impl Fn(&Path) -> bool) -> bool {
    let Some(cmd) = d.uninstall_string.as_deref() else {
        return false;
    };
    missing_uninstaller(cmd, |name: &str| known_var(name, k), exists).is_some()
}

/// Whether another record (another key or hive) has the same trimmed, case-insensitive
/// `DisplayName` as `d`.
pub fn is_duplicate(d: &ArpDetails, others: &[ArpDetails]) -> bool {
    let name = |e: &ArpDetails| {
        e.display_name
            .as_deref()
            .map(|n| n.trim().to_lowercase())
            .filter(|n| !n.is_empty())
    };
    let Some(mine) = name(d) else {
        return false;
    };
    others
        .iter()
        .filter(|o| !(o.hive == d.hive && o.key.eq_ignore_ascii_case(&d.key)))
        .any(|o| name(o).as_deref() == Some(mine.as_str()))
}

fn is_machine(h: Hive) -> bool {
    matches!(h, Hive::MachineX64 | Hive::MachineX86)
}

/// The version of an entry when it has a non-empty `DisplayVersion`.
fn version_of(e: &ArpDetails) -> Option<Version> {
    let v = e.display_version.as_deref().map(str::trim)?;
    (!v.is_empty()).then(|| Version::parse(v))
}

/// The duplicate rule the elevated helper applies before deleting `d`: `d` is an HKLM record,
/// and another HKLM record (another key or view) with the same name (see `is_duplicate`) has
/// a strictly greater `DisplayVersion`, so `d` is not the principal one. HKCU records never
/// count (the user writes them), and equal or missing versions do not tell which record is
/// the principal, so they refuse.
pub fn elevated_duplicate(d: &ArpDetails, others: &[ArpDetails]) -> bool {
    if !is_machine(d.hive) {
        return false;
    }
    let Some(mine) = version_of(d) else {
        return false;
    };
    let newer: Vec<ArpDetails> = others
        .iter()
        .filter(|o| is_machine(o.hive))
        .filter(|o| version_of(o).is_some_and(|v| v > mine))
        .cloned()
        .collect();
    is_duplicate(d, &newer)
}
