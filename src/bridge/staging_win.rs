//! Admin-only execution folders on the elevated helper's side (Task 180):
//! `<FOLDERID_ProgramFiles>\wingetrs\staging\<32 hex>\` (a protected DACL: SYSTEM and
//! Administrators only) holds the `TEMP`/`TMP` of what the helper runs (winget, uninstallers),
//! with an environment built from the known folders. Motivo: a DLL planted in the user's
//! `%TEMP%` would be loaded as admin. The helper no longer copies or runs installers (Task 196).
use std::ffi::OsString;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use windows::Win32::Foundation::{ERROR_SUCCESS, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::{
    CreateDirectoryW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::SystemInformation::{
    GetNativeSystemInfo, PROCESSOR_ARCHITECTURE_AMD64, PROCESSOR_ARCHITECTURE_ARM64,
    PROCESSOR_ARCHITECTURE_INTEL, SYSTEM_INFO,
};
use windows::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_ProgramData, FOLDERID_ProgramFiles,
    FOLDERID_ProgramFilesCommon, FOLDERID_ProgramFilesCommonX64, FOLDERID_ProgramFilesCommonX86,
    FOLDERID_ProgramFilesX64, FOLDERID_ProgramFilesX86, FOLDERID_Public, FOLDERID_RoamingAppData,
    FOLDERID_System, FOLDERID_Windows, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::{GUID, HSTRING, PWSTR};

/// DACL of an execution folder: SYSTEM and Administrators, inherited by what is inside, and
/// nothing inherited from above.
pub const RUN_SDDL: &str = "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";
/// Execution folders older than this are removed when the helper starts.
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// The `TEMP`/`TMP` of what runs, inside the execution folder.
pub const TMP_NAME: &str = "tmp";
/// `FILE_ATTRIBUTE_REPARSE_POINT`.
const REPARSE: u32 = 0x400;

/// A Win32 error as the `io::Error` std would build (so `kind()` works).
fn io_err(e: &windows::core::Error) -> std::io::Error {
    let code = e.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        std::io::Error::from_raw_os_error((code & 0xFFFF) as i32)
    } else {
        std::io::Error::from_raw_os_error(code as i32)
    }
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

/// Refuses `path` when it is a reparse point (junction, symlink) or not a folder.
fn plain_dir(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_attributes() & REPARSE != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} é um ponto de reanálise", path.display()),
        ));
    }
    if !meta.is_dir() {
        return Err(std::io::Error::other(format!(
            "{} não é uma pasta",
            path.display()
        )));
    }
    Ok(())
}

/// `<program_files>\wingetrs\staging`, created when missing (it inherits the ACL of
/// `program_files`); refused when `wingetrs` or `staging` is a reparse point.
pub fn staging_root(program_files: &Path) -> std::io::Result<PathBuf> {
    let base = program_files.join("wingetrs");
    let root = base.join("staging");
    for dir in [&base, &root] {
        match std::fs::create_dir(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        plain_dir(dir)?;
    }
    Ok(root)
}

/// Creates the folder `path` with the security descriptor `sddl`; fails when it exists.
fn create_dir_with(path: &Path, sddl: &str) -> std::io::Result<()> {
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `sd` receives a LocalAlloc descriptor, freed by `_free`.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            &HSTRING::from(sddl),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .map_err(|e| io_err(&e))?;
    let _free = Local(sd.0);
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    // SAFETY: `sa` and its descriptor outlive the call.
    unsafe { CreateDirectoryW(&HSTRING::from(path.as_os_str()), Some(&sa)) }.map_err(|e| io_err(&e))
}

/// The DACL of `path` as SDDL (`D:...`).
pub fn dacl_sddl(path: &Path) -> std::io::Result<String> {
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `sd` receives a LocalAlloc descriptor, freed by `_free`.
    let rc = unsafe {
        GetNamedSecurityInfoW(
            &HSTRING::from(path.as_os_str()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut sd,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(rc.0 as i32));
    }
    let _free = Local(sd.0);
    let mut text = PWSTR::null();
    // SAFETY: `sd` is a valid descriptor; `text` receives a LocalAlloc string.
    unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            sd,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            None,
        )
    }
    .map_err(|e| io_err(&e))?;
    let _text = Local(text.0.cast());
    // SAFETY: a NUL-terminated string from the call above.
    unsafe { text.to_string() }.map_err(|e| std::io::Error::other(e.to_string()))
}

/// One execution folder, removed with everything inside when dropped.
#[derive(Debug)]
pub struct RunDir {
    path: PathBuf,
}

impl RunDir {
    /// Creates `<root>\<32 random hex>` with the descriptor `sddl` (`RUN_SDDL` in production).
    pub fn create(root: &Path, sddl: &str) -> std::io::Result<RunDir> {
        let path = root.join(super::elevate_win::random_hex()?);
        create_dir_with(&path, sddl)?;
        Ok(RunDir { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Leaves the folder in place (a process may still use it); `prune` removes it after
    /// `STALE_AFTER`. Its path.
    pub fn keep(self) -> PathBuf {
        let this = std::mem::ManuallyDrop::new(self);
        this.path.clone()
    }
}

impl Drop for RunDir {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.path) {
            // A folder this process may not list is still removable when empty (the right to
            // delete children of the parent).
            if std::fs::remove_dir(&self.path).is_err() {
                tracing::warn!(error = %e, "staging: execution folder left behind");
            }
        }
    }
}

/// Opens every folder above `path`, from the volume root down, without following a reparse
/// point (one is refused) and without delete sharing, so none can be renamed or swapped for a
/// junction while the handles live. Each open resolves through the folders already held.
pub(super) fn hold_plain_folders(path: &Path) -> std::io::Result<Vec<std::fs::File>> {
    let mut dirs: Vec<&Path> = path
        .ancestors()
        .skip(1)
        .filter(|d| !d.as_os_str().is_empty())
        .collect();
    dirs.reverse();
    let mut held = Vec::with_capacity(dirs.len());
    for dir in dirs {
        let f = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(dir)?;
        let meta = f.metadata()?;
        if meta.file_attributes() & REPARSE != 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{} é um ponto de reanálise", dir.display()),
            ));
        }
        if !meta.is_dir() {
            return Err(std::io::Error::other(format!(
                "{} não é uma pasta",
                dir.display()
            )));
        }
        held.push(f);
    }
    Ok(held)
}

/// The system folders and processor facts the installer's environment is built from, all from
/// the known folders and `GetNativeSystemInfo` (never from this process's environment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemEnv {
    pub windows: PathBuf,
    pub system32: PathBuf,
    pub program_files: PathBuf,
    pub program_files_x86: PathBuf,
    pub program_w6432: PathBuf,
    pub common_files: PathBuf,
    pub common_files_x86: PathBuf,
    pub common_w6432: PathBuf,
    pub program_data: PathBuf,
    pub public: PathBuf,
    pub profile: PathBuf,
    pub app_data: PathBuf,
    pub local_app_data: PathBuf,
    /// `PROCESSOR_ARCHITECTURE` (`AMD64`, `ARM64`, `x86`).
    pub architecture: String,
    pub level: u16,
    pub revision: u16,
    pub processors: u32,
}

/// A known folder's path.
fn known(id: &GUID) -> std::io::Result<PathBuf> {
    // SAFETY: the returned string is freed below with CoTaskMemFree.
    let p = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.map_err(|e| io_err(&e))?;
    // SAFETY: a NUL-terminated string from the call above.
    let text = unsafe { p.to_string() };
    // SAFETY: allocated by SHGetKnownFolderPath, freed once.
    unsafe { CoTaskMemFree(Some(p.0 as *const core::ffi::c_void)) };
    text.map(PathBuf::from)
        .map_err(|e| std::io::Error::other(e.to_string()))
}

impl SystemEnv {
    pub fn query() -> std::io::Result<SystemEnv> {
        let mut info = SYSTEM_INFO::default();
        // SAFETY: `info` is a SYSTEM_INFO the call fills.
        unsafe { GetNativeSystemInfo(&mut info) };
        // SAFETY: the union's struct member is what GetNativeSystemInfo fills.
        let arch = unsafe { info.Anonymous.Anonymous.wProcessorArchitecture };
        let architecture = match arch {
            PROCESSOR_ARCHITECTURE_AMD64 => "AMD64",
            PROCESSOR_ARCHITECTURE_ARM64 => "ARM64",
            PROCESSOR_ARCHITECTURE_INTEL => "x86",
            _ => "UNKNOWN",
        };
        Ok(SystemEnv {
            windows: known(&FOLDERID_Windows)?,
            system32: known(&FOLDERID_System)?,
            program_files: known(&FOLDERID_ProgramFiles)?,
            program_files_x86: known(&FOLDERID_ProgramFilesX86)?,
            program_w6432: known(&FOLDERID_ProgramFilesX64)?,
            common_files: known(&FOLDERID_ProgramFilesCommon)?,
            common_files_x86: known(&FOLDERID_ProgramFilesCommonX86)?,
            common_w6432: known(&FOLDERID_ProgramFilesCommonX64)?,
            program_data: known(&FOLDERID_ProgramData)?,
            public: known(&FOLDERID_Public)?,
            profile: known(&FOLDERID_Profile)?,
            app_data: known(&FOLDERID_RoamingAppData)?,
            local_app_data: known(&FOLDERID_LocalAppData)?,
            architecture: architecture.to_string(),
            level: info.wProcessorLevel,
            revision: info.wProcessorRevision,
            processors: info.dwNumberOfProcessors,
        })
    }

    /// Folders an install folder may be in (`Program Files` and `Program Files (x86)`).
    pub fn program_files_roots(&self) -> Vec<String> {
        [&self.program_files, &self.program_files_x86]
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect()
    }
}

/// `PATHEXT` of a clean Windows install.
pub const PATHEXT: &str = ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC";

/// The installer's whole environment: a closed list built from `sys` and constants, with
/// `TEMP`/`TMP` set to `tmp`. Nothing comes from this process's (the user's) environment: no
/// user `PATH` entry, no proxy, no planted variable.
pub fn installer_env(sys: &SystemEnv, tmp: &Path) -> Vec<(OsString, OsString)> {
    let s32 = &sys.system32;
    let path = [
        s32.clone(),
        sys.windows.clone(),
        s32.join("Wbem"),
        s32.join("WindowsPowerShell").join("v1.0"),
    ]
    .iter()
    .map(|p| p.to_string_lossy().into_owned())
    .collect::<Vec<_>>()
    .join(";");
    let drive: String = sys.windows.to_string_lossy().chars().take(2).collect();
    let p = |p: &Path| p.as_os_str().to_owned();
    vec![
        ("SystemRoot".into(), p(&sys.windows)),
        ("windir".into(), p(&sys.windows)),
        ("SystemDrive".into(), drive.into()),
        ("ComSpec".into(), p(&s32.join("cmd.exe"))),
        ("PATH".into(), path.into()),
        ("PATHEXT".into(), PATHEXT.into()),
        (
            "PSModulePath".into(),
            p(&s32.join("WindowsPowerShell").join("v1.0").join("Modules")),
        ),
        ("ProgramFiles".into(), p(&sys.program_files)),
        ("ProgramFiles(x86)".into(), p(&sys.program_files_x86)),
        ("ProgramW6432".into(), p(&sys.program_w6432)),
        ("CommonProgramFiles".into(), p(&sys.common_files)),
        ("CommonProgramFiles(x86)".into(), p(&sys.common_files_x86)),
        ("CommonProgramW6432".into(), p(&sys.common_w6432)),
        ("ProgramData".into(), p(&sys.program_data)),
        ("ALLUSERSPROFILE".into(), p(&sys.program_data)),
        ("PUBLIC".into(), p(&sys.public)),
        ("OS".into(), "Windows_NT".into()),
        (
            "PROCESSOR_ARCHITECTURE".into(),
            sys.architecture.clone().into(),
        ),
        ("PROCESSOR_LEVEL".into(), sys.level.to_string().into()),
        (
            "PROCESSOR_REVISION".into(),
            format!("{:04x}", sys.revision).into(),
        ),
        (
            "NUMBER_OF_PROCESSORS".into(),
            sys.processors.to_string().into(),
        ),
        ("USERPROFILE".into(), p(&sys.profile)),
        ("APPDATA".into(), p(&sys.app_data)),
        ("LOCALAPPDATA".into(), p(&sys.local_app_data)),
        ("TEMP".into(), p(tmp)),
        ("TMP".into(), p(tmp)),
    ]
}

/// Runs the helper's fixed tool (winget) at the absolute path `program` from `dir` with
/// `installer_env` and `TEMP`/`TMP` in a fresh execution folder under `root` (created with
/// `sddl`, removed after): nothing comes from this process's (the user's) environment.
pub fn run_tool(
    program: &Path,
    args: &[String],
    dir: &Path,
    root: &Path,
    sddl: &str,
    sys: &SystemEnv,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    let run = RunDir::create(root, sddl)?;
    let tmp = run.path().join(TMP_NAME);
    create_dir_with(&tmp, sddl)?;
    let out = super::spawn_win::run_in_env(program, args, dir, &installer_env(sys, &tmp), on_line);
    drop(run);
    out
}

/// How long the helper waits for the processes an installer left in its job after it exited.
pub const INSTALL_LINGER: Duration = Duration::from_secs(60);
/// Line the helper sends while it waits for them.
pub const WAITING_INSTALLER: &str = "aguardando os processos do instalador terminarem";

/// Runs a staged installer (Task 202): `staged.program` must be the checked installer in its run
/// folder `staged.dir` (or the system `msiexec` for a package there); it runs from that folder in
/// a job (`spawn_win::run_in_job`) with `installer_env` and `TEMP`/`TMP` in a new `tmp` inside it,
/// so nothing it loads or unpacks sits in a folder the user writes. The other processes of the
/// job are waited for up to `linger`. The installer's exit code and whether they all ended.
pub fn run_staged(
    staged: &super::signed_install::Staged,
    sys: &SystemEnv,
    linger: Duration,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<super::spawn_win::JobEnd> {
    let in_dir = staged.installer.parent() == Some(staged.dir.as_path());
    let program_ok = staged.program == staged.installer
        || (staged.kind.is_msi() && staged.program == sys.system32.join("msiexec.exe"));
    if !in_dir || !program_ok {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "instalador fora da pasta de execução",
        ));
    }
    let tmp = staged.dir.join(TMP_NAME);
    std::fs::create_dir(&tmp)?;
    super::spawn_win::run_in_job(
        &staged.program,
        &staged.params,
        &staged.dir,
        &installer_env(sys, &tmp),
        linger,
        &mut || on_line(WAITING_INSTALLER),
    )
}

/// Whether `name` is an execution folder name (32 lowercase hex).
fn run_name(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Removes the execution folders in `root` older than `STALE_AFTER` at `now` (only plain
/// folders named like one); how many were removed.
pub fn prune(root: &Path, now: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_str().is_some_and(run_name) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || meta.file_attributes() & REPARSE != 0 {
            continue;
        }
        let born = meta.created().or_else(|_| meta.modified());
        let stale = born
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > STALE_AFTER);
        if stale {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => removed += 1,
                Err(e) => tracing::warn!(error = %e, "staging: stale folder not removed"),
            }
        }
    }
    removed
}

/// At the helper's start: prunes the production root when it exists and is not a reparse
/// point. Never creates it.
pub fn prune_at_start() {
    let Ok(pf) = super::elevate_win::program_files() else {
        return;
    };
    let base = pf.join("wingetrs");
    let root = base.join("staging");
    if plain_dir(&base).is_ok() && plain_dir(&root).is_ok() {
        prune(&root, SystemTime::now());
    }
}
