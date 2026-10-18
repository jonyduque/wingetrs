//! Orphan ARP entries (Task 183): a record whose uninstaller no longer exists is exported to a
//! `.reg` backup, the backup is checked, and only then the key is deleted (HKCU in process,
//! HKLM through the elevated helper, which checks the orphan rule again).
use super::uninstall_cmd::valid_arp_key;
use crate::bridge::elevated::ElevatedRunner;
use crate::bridge::worker::{DeleteReason, MachineHive};
use crate::installed::{Hive, InstallSource, InstalledApp};
use std::io::Read;
use std::path::{Path, PathBuf};

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const REG_HEADER: &str = "Windows Registry Editor Version 5.00";
/// One ARP key exports to a few KiB; anything past this is not our export.
const MAX_EXPORT: u64 = 8 * 1024 * 1024;
/// Numbered names tried when the backup name is already taken.
const MAX_BACKUP_SUFFIX: u32 = 100;

/// An ARP record whose uninstaller is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub hive: Hive,
    pub key: String,
    pub name: String,
    /// The uninstaller path that no longer exists.
    pub missing: String,
}

/// The ARP apps marked `stale` (never pre-selected: the caller decides, one by one).
pub fn orphans(apps: &[InstalledApp]) -> Vec<Orphan> {
    apps.iter()
        .filter_map(|a| match (&a.source, &a.stale) {
            (InstallSource::Arp(hive), Some(missing)) => Some(Orphan {
                hive: *hive,
                key: a.key.clone(),
                name: a.name.clone(),
                missing: missing.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// `reg.exe` arguments exporting the key; HKLM picks its view with `/reg:`, so the 32-bit
/// view is named without `WOW6432Node`.
pub fn export_args(hive: Hive, key: &str, file: &Path) -> Vec<String> {
    let (root, view) = match hive {
        Hive::MachineX64 => ("HKLM", Some("/reg:64")),
        Hive::MachineX86 => ("HKLM", Some("/reg:32")),
        Hive::User => ("HKCU", None),
    };
    let mut args = vec![
        "export".to_string(),
        format!(r"{root}\{UNINSTALL}\{key}"),
        file.to_string_lossy().into_owned(),
        "/y".to_string(),
    ];
    if let Some(v) = view {
        args.push(v.to_string());
    }
    args
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || " ._{}+-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn hive_tag(h: Hive) -> &'static str {
    match h {
        Hive::MachineX64 => "hklm64",
        Hive::MachineX86 => "hklm32",
        Hive::User => "hkcu",
    }
}

/// `backups\<ts>\<hive>-<sanitized key>.reg`.
pub fn backup_file(backups: &Path, ts: &str, o: &Orphan) -> PathBuf {
    backups
        .join(sanitize(ts))
        .join(format!("{}-{}.reg", hive_tag(o.hive), sanitize(&o.key)))
}

/// A backup path nobody wrote yet (so a stale file never stands in for a failed export).
fn fresh_backup_file(backups: &Path, ts: &str, o: &Orphan) -> Result<PathBuf, String> {
    let first = backup_file(backups, ts, o);
    let dir = first
        .parent()
        .ok_or_else(|| "pasta de backup inválida".to_string())?
        .to_path_buf();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("não foi possível criar {}: {e}", dir.display()))?;
    let stem = format!("{}-{}", hive_tag(o.hive), sanitize(&o.key));
    let candidates = std::iter::once(first)
        .chain((2..=MAX_BACKUP_SUFFIX).map(|n| dir.join(format!("{stem}-{n}.reg"))));
    for c in candidates {
        match std::fs::symlink_metadata(&c) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(c),
            Err(e) => return Err(format!("não foi possível conferir {}: {e}", c.display())),
            Ok(_) => continue,
        }
    }
    Err(format!("nomes de backup esgotados em {}", dir.display()))
}

/// The export is a UTF-16LE `.reg` (BOM, version 5 header on the first line) whose key line is
/// exactly `[<HKLM|HKCU root>\...\Uninstall\<key>]` (case-insensitive), under either root.
pub fn check_export(file: &Path, key: &str) -> Result<(), String> {
    check_export_roots(file, key, &[MACHINE_ROOT, USER_ROOT])
}

/// `check_export` for the root of `hive` only (the view never shows in the file: `reg export
/// /reg:32` writes the path without `WOW6432Node`).
pub fn check_export_in(file: &Path, hive: Hive, key: &str) -> Result<(), String> {
    let root = match hive {
        Hive::MachineX64 | Hive::MachineX86 => MACHINE_ROOT,
        Hive::User => USER_ROOT,
    };
    check_export_roots(file, key, &[root])
}

const MACHINE_ROOT: &str = "HKEY_LOCAL_MACHINE";
const USER_ROOT: &str = "HKEY_CURRENT_USER";

fn check_export_roots(file: &Path, key: &str, roots: &[&str]) -> Result<(), String> {
    let f = std::fs::File::open(file)
        .map_err(|e| format!("backup ausente ({}): {e}", file.display()))?;
    let mut bytes = Vec::new();
    f.take(MAX_EXPORT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("backup ilegível ({}): {e}", file.display()))?;
    if bytes.is_empty() {
        return Err("backup vazio".to_string());
    }
    if bytes.len() as u64 > MAX_EXPORT {
        return Err("backup grande demais".to_string());
    }
    let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) else {
        return Err("backup sem BOM UTF-16LE".to_string());
    };
    let (pairs, rest) = body.as_chunks::<2>();
    if !rest.is_empty() {
        return Err("backup UTF-16 truncado".to_string());
    }
    let units = pairs.iter().map(|c| u16::from_le_bytes(*c));
    let text: String = char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r'));
    if lines.next() != Some(REG_HEADER) {
        return Err("backup sem o cabeçalho do Editor do Registro".to_string());
    }
    let wanted: Vec<String> = roots
        .iter()
        .map(|r| format!(r"[{r}\{UNINSTALL}\{key}]").to_lowercase())
        .collect();
    let named = lines.any(|l| wanted.contains(&l.trim().to_lowercase()));
    if named {
        Ok(())
    } else {
        Err(format!("backup não contém a chave {key}"))
    }
}

/// Registry access for orphan removal (`SystemArp` on Windows, a fake in tests).
pub trait ArpRegistry: Send + Sync {
    /// Runs `reg.exe` with `args` (from `export_args`).
    fn export(&self, args: &[String]) -> std::io::Result<()>;
    /// Deletes `HKCU\...\Uninstall\<key>` (never through a registry link).
    fn delete_user_key(&self, key: &str) -> std::io::Result<()>;
    /// Whether the key still exists and its uninstaller is still missing.
    fn still_orphan(&self, hive: Hive, key: &str) -> bool;
}

fn machine_hive(h: Hive) -> Option<MachineHive> {
    match h {
        Hive::MachineX64 => Some(MachineHive::Hklm64),
        Hive::MachineX86 => Some(MachineHive::Hklm32),
        Hive::User => None,
    }
}

/// Removes one orphan: re-check, export, verify the export, delete. Returns the `.reg` file.
pub fn remove_orphan(
    reg: &dyn ArpRegistry,
    elevated: &dyn ElevatedRunner,
    backups: &Path,
    ts: &str,
    o: &Orphan,
) -> Result<PathBuf, String> {
    if !valid_arp_key(&o.key) {
        return Err(format!("nome de chave ARP inválido: {:?}", o.key));
    }
    if !reg.still_orphan(o.hive, &o.key) {
        return Err(format!("{}: a chave deixou de ser sobra", o.key));
    }
    let file = fresh_backup_file(backups, ts, o)?;
    reg.export(&export_args(o.hive, &o.key, &file))
        .map_err(|e| format!("falha ao exportar {}: {e}", o.key))?;
    check_export_in(&file, o.hive, &o.key)?;
    match machine_hive(o.hive) {
        None => reg
            .delete_user_key(&o.key)
            .map_err(|e| format!("falha ao apagar {}: {e}", o.key))?,
        Some(h) => elevated
            .delete_arp_key(h, &o.key, DeleteReason::Orphan)
            .map_err(|e| format!("falha ao apagar {}: {e}", o.key))?,
    }
    Ok(file)
}
