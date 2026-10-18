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
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
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
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetLongPathNameW, READ_CONTROL,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_UserProfiles, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::{HSTRING, PWSTR};
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
        user_profiles: known(&FOLDERID_UserProfiles)?,
    })
}

/// A known folder's path.
fn known(id: &windows::core::GUID) -> io::Result<PathBuf> {
    // SAFETY: the returned string is freed below with CoTaskMemFree.
    let p = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }
        .map_err(|e| io::Error::other(e.to_string()))?;
    // SAFETY: a NUL-terminated string from the call above.
    let text = unsafe { p.to_string() };
    // SAFETY: allocated by SHGetKnownFolderPath, freed once.
    unsafe { CoTaskMemFree(Some(p.0 as *const core::ffi::c_void)) };
    text.map(PathBuf::from)
        .map_err(|e| io::Error::other(e.to_string()))
}

/// Longest path `long_path` returns, in UTF-16 units.
const LONG_PATH_MAX: usize = 32_768;

/// `path` with its 8.3 names expanded (`GetLongPathNameW`); an error when it does not exist.
pub fn long_path(path: &Path) -> io::Result<PathBuf> {
    let mut buf = vec![0u16; LONG_PATH_MAX];
    // SAFETY: `buf` is a writable buffer of the length passed.
    let n = unsafe { GetLongPathNameW(&HSTRING::from(path.as_os_str()), Some(&mut buf)) } as usize;
    if n == 0 {
        return Err(io::Error::last_os_error());
    }
    let text = buf
        .get(..n)
        .filter(|_| n < LONG_PATH_MAX)
        .ok_or_else(|| io::Error::other("caminho longo demais"))?;
    Ok(PathBuf::from(OsString::from_wide(text)))
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
    if !confirm(&d, &exists, &folder_sddl) {
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

/// Runs `program args` (already checked by the helper) once the file and its folders (and each
/// file of `also_checked`, a DLL it loads, with its folders) are administrator-only by their real owner and DACL, holding them open until it ends; from the
/// program's folder, with `installer_env` (built from the known folders; `TEMP` in a fresh
/// execution folder under the staging root). The exit code. Nothing is written in user folders.
pub fn run_uninstaller(
    program: &Path,
    args: &str,
    also_checked: &[PathBuf],
    on_line: &mut dyn FnMut(&str),
) -> io::Result<i32> {
    let mut held = hold_admin_chain(program)?;
    for dll in also_checked {
        held.extend(hold_admin_chain(dll)?);
    }
    let sys = SystemEnv::query()?;
    let dir = program
        .parent()
        .ok_or_else(|| io::Error::other("desinstalador sem pasta"))?;
    let root = staging_root(&sys.program_files)?;
    let code = run_waiting(
        program,
        args,
        dir,
        &root,
        RUN_SDDL,
        &sys,
        UNINSTALL_LINGER,
        on_line,
    );
    drop(held);
    code
}

/// How long the helper waits for the processes an uninstaller left in its job after the
/// uninstaller itself exited.
pub const UNINSTALL_LINGER: std::time::Duration = std::time::Duration::from_secs(10 * 60);

pub use super::worker::WAITING_UNINSTALLER;

/// Runs `program args` from `dir` in a job object (see `spawn_win::run_in_job`) with
/// `installer_env`, `TEMP` in a fresh execution folder under `root` (created with `sddl`).
/// After the program exits, the other processes of the job (an uninstaller that copied itself
/// to `TEMP`) are waited for up to `linger`, with `WAITING_UNINSTALLER` sent to `on_line`. The
/// folder is removed once they all ended; when some still run at the deadline they are left
/// alone and so is the folder (the helper's 24 h `prune` removes it), which `on_line` reports.
/// The exit code of the program started.
#[allow(clippy::too_many_arguments)]
pub fn run_waiting(
    program: &Path,
    args: &str,
    dir: &Path,
    root: &Path,
    sddl: &str,
    sys: &SystemEnv,
    linger: std::time::Duration,
    on_line: &mut dyn FnMut(&str),
) -> io::Result<i32> {
    let run = RunDir::create(root, sddl)?;
    let tmp = run.path().join(TMP_NAME);
    std::fs::create_dir(&tmp)?;
    let end = super::spawn_win::run_in_job(
        program,
        args,
        dir,
        &installer_env(sys, &tmp),
        linger,
        &mut || on_line(WAITING_UNINSTALLER),
    )?;
    if !end.all_ended {
        let kept = run.keep();
        tracing::warn!(folder = %kept.display(), "arp: uninstaller left processes running; folder kept for prune");
        on_line(&format!(
            "o desinstalador deixou processos rodando após {} min; a pasta de execução fica para a limpeza de 24 h",
            linger.as_secs() / 60
        ));
    }
    Ok(end.code)
}

/// Owner and DACL of the folder `path` as SDDL. Every folder from the volume root down to it is
/// opened without following a reparse point (and held while the descriptor is read), so a
/// junction anywhere on the way cannot point the check at another folder; `None` when one is a
/// reparse point or anything is unreadable (then nothing is an orphan there).
pub fn folder_sddl(path: &Path) -> Option<String> {
    let _above = super::staging_win::hold_plain_folders(path).ok()?;
    let file = std::fs::OpenOptions::new()
        .access_mode(READ_CONTROL.0 | FILE_READ_ATTRIBUTES.0)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if meta.file_attributes() & REPARSE != 0 || !meta.is_dir() {
        return None;
    }
    handle_sddl(&file).ok()
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
