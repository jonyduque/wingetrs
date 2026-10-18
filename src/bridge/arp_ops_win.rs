//! The elevated helper's ARP operations (Task 182): known folders, reading ARP keys with an
//! explicit view, deleting an HKLM ARP key without following a registry link, and running a
//! checked uninstaller with an environment built from scratch.
use super::staging_win::{RUN_SDDL, RunDir, SystemEnv, TMP_NAME, installer_env, staging_root};
use super::worker::MachineHive;
use crate::installed::Hive;
use crate::winget_install::arp_read_win::read_details;
use crate::winget_install::elevated_rules::KnownFolders;
use crate::winget_install::uninstall_cmd::{ArpDetails, valid_arp_key};
use std::io;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use winreg::RegKey;
use winreg::enums::{
    DELETE, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE,
    KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_OPTION_OPEN_LINK,
};

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_X86: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
/// Most sub-keys read from one `Uninstall` key.
const KEYS_MAX: usize = 8192;

/// The folders the helper trusts, from `SHGetKnownFolderPath` (`ProgramFiles`,
/// `ProgramFilesX86`, `Windows`, `ProgramData`), never from an environment variable.
pub fn known_folders() -> io::Result<KnownFolders> {
    let sys = SystemEnv::query()?;
    Ok(KnownFolders {
        program_files: sys.program_files,
        program_files_x86: sys.program_files_x86,
        system_root: sys.windows,
        program_data: sys.program_data,
    })
}

/// The `Uninstall` key of `hive` with the flags that pick its view.
fn uninstall_root(hive: Hive) -> (winreg::HKEY, &'static str, u32) {
    match hive {
        Hive::MachineX64 => (HKEY_LOCAL_MACHINE, UNINSTALL, KEY_WOW64_64KEY),
        Hive::MachineX86 => (HKEY_LOCAL_MACHINE, UNINSTALL_X86, KEY_WOW64_64KEY),
        Hive::User => (HKEY_CURRENT_USER, UNINSTALL, 0),
    }
}

/// Every readable ARP entry of HKLM 64, HKLM 32 and HKCU (the HKCU of this token).
pub fn arp_entries() -> io::Result<Vec<ArpDetails>> {
    let mut out = Vec::new();
    for hive in [Hive::MachineX64, Hive::MachineX86, Hive::User] {
        let (root, base, view) = uninstall_root(hive);
        let parent = match RegKey::predef(root).open_subkey_with_flags(base, KEY_READ | view) {
            Ok(k) => k,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for name in parent.enum_keys().take(KEYS_MAX) {
            let Ok(name) = name else { continue };
            if !valid_arp_key(&name) {
                continue;
            }
            match read_details(hive, &name) {
                Ok(Some(d)) => out.push(d),
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e, ?hive, "arp: entry unreadable"),
            }
        }
    }
    Ok(out)
}

fn is_link(key: &RegKey) -> bool {
    key.enum_values()
        .flatten()
        .any(|(n, _)| n.eq_ignore_ascii_case("SymbolicLinkValue"))
}

/// Deletes `<Uninstall>\<key>` of `hive` and its tree, with the 64-bit flag on every open of
/// the same view; the key is opened as itself (never through a registry link) and refused when
/// it is one.
pub fn delete_key(hive: MachineHive, key: &str) -> io::Result<()> {
    if !valid_arp_key(key) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "nome de chave ARP inválido",
        ));
    }
    let (root, base, view) = uninstall_root(hive.into());
    let parent = RegKey::predef(root).open_subkey_with_flags(base, KEY_READ | view)?;
    let child = parent.open_subkey_with_options_flags(
        key,
        REG_OPTION_OPEN_LINK,
        KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS | KEY_SET_VALUE | DELETE | view,
    )?;
    if is_link(&child) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "a chave é um link do registro",
        ));
    }
    // An empty path deletes the sub-keys and values of this very handle.
    child.delete_subkey_all("")?;
    drop(child);
    parent.delete_subkey_with_flags(key, view)
}

/// Runs `program args` (already checked by the helper) from the program's folder with
/// `installer_env` (built from the known folders; `TEMP` in a fresh execution folder under the
/// staging root), waits for it and returns its exit code. Nothing is written in user folders.
pub fn run_uninstaller(program: &Path, args: &str) -> io::Result<i32> {
    let sys = SystemEnv::query()?;
    let dir = program
        .parent()
        .ok_or_else(|| io::Error::other("desinstalador sem pasta"))?;
    let root = staging_root(&sys.program_files)?;
    let run = RunDir::create(&root, RUN_SDDL)?;
    let tmp = run.path().join(TMP_NAME);
    std::fs::create_dir(&tmp)?;
    let status = Command::new(program)
        .raw_arg(args)
        .current_dir(dir)
        .env_clear()
        .envs(installer_env(&sys, &tmp))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    status
        .code()
        .ok_or_else(|| io::Error::other("o desinstalador terminou sem código de saída"))
}
