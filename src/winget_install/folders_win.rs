//! The user's data folders and the free space of a volume (Task 186).
use super::uninstall::UserFolders;
use std::io;
use std::path::{Path, PathBuf};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_Documents, FOLDERID_LocalAppData, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT,
    SHGetKnownFolderPath,
};
use windows::core::{GUID, HSTRING};

/// A known folder's path.
fn known(id: &GUID) -> io::Result<PathBuf> {
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

/// `LocalAppData`, `RoamingAppData` and `Documents` from `SHGetKnownFolderPath`, never from
/// `%LOCALAPPDATA%`/`%APPDATA%` (the environment is the caller's to change), and the wingetrs
/// data root (`DataDir::root`).
pub fn user_folders(wingetrs_root: &Path) -> io::Result<UserFolders> {
    Ok(UserFolders {
        local_app_data: known(&FOLDERID_LocalAppData)?,
        app_data: known(&FOLDERID_RoamingAppData)?,
        documents: known(&FOLDERID_Documents)?,
        wingetrs_root: wingetrs_root.to_path_buf(),
    })
}

/// Free bytes for this user on the volume of `path` (`GetDiskFreeSpaceExW`), asked of the
/// nearest folder that exists (a backup destination is created only after the check).
pub fn free_space(path: &Path) -> Option<u64> {
    let dir = path.ancestors().find(|a| a.is_dir())?;
    let mut free = 0u64;
    // SAFETY: `free` is a live local the call writes once; the other outputs are not asked.
    let r = unsafe {
        GetDiskFreeSpaceExW(&HSTRING::from(dir.as_os_str()), Some(&mut free), None, None)
    };
    match r {
        Ok(()) => Some(free),
        Err(e) => {
            tracing::warn!(path = %dir.display(), error = %e, "free space unreadable");
            None
        }
    }
}
