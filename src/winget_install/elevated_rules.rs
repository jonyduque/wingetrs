//! Rules the elevated helper applies to an HKLM ARP entry before it runs its uninstaller or
//! deletes its key (Task 182). Pure: the folders come from the caller (known folders on
//! Windows), never from the environment.
use super::uninstall_cmd::ArpDetails;
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

/// The components of `p` when it is absolute and has no `.`/`..`.
fn plain_components(p: &Path) -> Option<Vec<String>> {
    if !p.is_absolute() {
        return None;
    }
    p.components()
        .map(|c| match c {
            Component::CurDir | Component::ParentDir => None,
            other => Some(other.as_os_str().to_string_lossy().to_uppercase()),
        })
        .collect()
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
