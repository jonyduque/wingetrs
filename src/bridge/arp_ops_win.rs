//! The elevated helper's ARP operations (Task 182): known folders, reading ARP keys with an
//! explicit view, deleting an HKLM ARP key through one handle that never follows a registry
//! link, and running a checked uninstaller whose file and folders only administrators can
//! change, with an environment built from scratch.
use super::staging_win::{RUN_SDDL, RunDir, SystemEnv, TMP_NAME, installer_env, staging_root};
use super::worker::{ArpDelete, DeleteCheck, MachineHive};
use crate::installed::Hive;
use crate::winget_install::arp_read_win::{details_of, read_details, uninstall_root};
use crate::winget_install::elevated_rules::{AclLevel, KnownFolders, sddl_admin_only};
use crate::winget_install::uninstall_cmd::{ArpDetails, valid_arp_key};
use std::fs::File;
use std::io;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
};
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL,
};
use windows::core::PWSTR;
use winreg::RegKey;
use winreg::enums::{
    DELETE, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, REG_OPTION_OPEN_LINK,
};

/// Most sub-keys read from one `Uninstall` key.
const KEYS_MAX: usize = 8192;
/// `FILE_ATTRIBUTE_REPARSE_POINT`.
const REPARSE: u32 = 0x400;

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

/// Anything at `path`, links included (a dangling link counts: nothing becomes an orphan unless
/// the path is truly empty).
pub fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Every readable ARP entry of HKLM 64 and HKLM 32. HKCU is never read here: the user writes
/// it, so it must not decide what the helper deletes.
pub fn arp_entries() -> io::Result<Vec<ArpDetails>> {
    let mut out = Vec::new();
    for hive in [Hive::MachineX64, Hive::MachineX86] {
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

/// Opens `<Uninstall>\<key>` of `hive` once, as itself (`REG_OPTION_OPEN_LINK`; a link is
/// refused), with the 64-bit flag of its view; reads it through that handle and, when `confirm`
/// accepts, deletes its values and sub-keys through the same handle (`RegDeleteTreeW` with a
/// NULL sub-key: "the subkeys and values of this key are deleted"), then the empty key itself
/// (`RegDeleteKeyExW` has no handle form; it fails while sub-keys remain, so success also proves
/// the tree is gone). Only administrators write these keys, so nobody else can swap the key
/// between the two calls.
pub fn delete_key(hive: MachineHive, key: &str, confirm: DeleteCheck<'_>) -> io::Result<ArpDelete> {
    if !valid_arp_key(key) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "nome de chave ARP inválido",
        ));
    }
    let hive = Hive::from(hive);
    let (root, base, view) = uninstall_root(hive);
    let parent = RegKey::predef(root).open_subkey_with_flags(base, KEY_READ | view)?;
    let child = match parent.open_subkey_with_options_flags(
        key,
        REG_OPTION_OPEN_LINK,
        KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS | KEY_SET_VALUE | DELETE | view,
    ) {
        Ok(k) => k,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ArpDelete::Missing),
        Err(e) => return Err(e),
    };
    if is_link(&child) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "a chave é um link do registro",
        ));
    }
    let d = details_of(&child, hive, key);
    if !confirm(&d, &exists) {
        return Ok(ArpDelete::Kept);
    }
    child.delete_subkey_all("")?;
    drop(child);
    parent.delete_subkey_with_flags(key, view)?;
    Ok(ArpDelete::Deleted)
}

/// Frees a `LocalAlloc` block on drop.
struct Local(*mut core::ffi::c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: allocated by the API that returned it, freed once.
            unsafe { LocalFree(Some(HLOCAL(self.0))) };
        }
    }
}

/// Owner and DACL of the open `file` as SDDL.
fn handle_sddl(file: &File) -> io::Result<String> {
    let info = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: a valid handle owned by `file`; `sd` receives a LocalAlloc descriptor.
    let rc = unsafe {
        GetSecurityInfo(
            HANDLE(file.as_raw_handle()),
            SE_FILE_OBJECT,
            info,
            None,
            None,
            None,
            None,
            Some(&mut sd),
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(rc.0 as i32));
    }
    let _free = Local(sd.0);
    let mut text = PWSTR::null();
    // SAFETY: `sd` is a valid descriptor; `text` receives a LocalAlloc string.
    unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            sd,
            SDDL_REVISION_1,
            info,
            &mut text,
            None,
        )
    }
    .map_err(|e| io::Error::other(e.to_string()))?;
    let _text = Local(text.0.cast());
    // SAFETY: a NUL-terminated string from the call above.
    unsafe { text.to_string() }.map_err(|e| io::Error::other(e.to_string()))
}

fn not_admin(path: &Path, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "desinstalador fora de pasta de administrador: {} ({why})",
            path.display()
        ),
    )
}

/// Opens one object of the chain (never through a reparse point) and checks its security.
fn open_checked(path: &Path, level: AclLevel) -> io::Result<File> {
    let mut opts = std::fs::OpenOptions::new();
    if level == AclLevel::File {
        // No write or delete sharing: nobody can change, rename or delete the file until the
        // handle closes, after the uninstaller ran.
        opts.read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    } else {
        // No delete sharing: the folder cannot be renamed or deleted while held.
        opts.access_mode(READ_CONTROL.0 | FILE_READ_ATTRIBUTES.0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    let file = opts.open(path)?;
    let meta = file.metadata()?;
    if meta.file_attributes() & REPARSE != 0 {
        return Err(not_admin(path, "ponto de reanálise"));
    }
    if (level == AclLevel::File) != meta.is_file() {
        return Err(not_admin(path, "tipo inesperado"));
    }
    let sddl = handle_sddl(&file)?;
    if !sddl_admin_only(&sddl, level) {
        tracing::warn!(path = %path.display(), sddl, "arp: uninstaller chain not admin-only");
        return Err(not_admin(path, "permissões"));
    }
    Ok(file)
}

/// Opens `program` and every folder above it up to the volume root, each checked with
/// `sddl_admin_only`; the handles keep them from being swapped until they drop.
fn hold_admin_chain(program: &Path) -> io::Result<Vec<File>> {
    let mut held = vec![open_checked(program, AclLevel::File)?];
    for (n, dir) in program.ancestors().skip(1).enumerate() {
        let level = if n == 0 {
            AclLevel::Parent
        } else {
            AclLevel::Ancestor
        };
        held.push(open_checked(dir, level)?);
    }
    Ok(held)
}

/// Runs `program args` (already checked by the helper) once the file and its folders are
/// administrator-only by their real owner and DACL, holding them open until it ends; from the
/// program's folder, with `installer_env` (built from the known folders; `TEMP` in a fresh
/// execution folder under the staging root). The exit code. Nothing is written in user folders.
pub fn run_uninstaller(program: &Path, args: &str) -> io::Result<i32> {
    let held = hold_admin_chain(program)?;
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
    drop(held);
    status
        .code()
        .ok_or_else(|| io::Error::other("o desinstalador terminou sem código de saída"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live (reads real ACLs, writes nothing): a stock system program passes the chain check,
    /// a file in the user's temp folder does not.
    #[test]
    #[ignore = "live: lê as ACLs reais do disco"]
    fn live_admin_chain() {
        let sys = SystemEnv::query().unwrap();
        let notepad = sys.system32.join("notepad.exe");
        assert!(hold_admin_chain(&notepad).is_ok());
        let dir = tempfile::TempDir::new().unwrap();
        let temp = dir.path().join("t182-not-admin.exe");
        std::fs::write(&temp, b"x").unwrap();
        let r = hold_admin_chain(&temp);
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    }
}
