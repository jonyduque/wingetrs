//! Admin-only execution folder for verified installers (Task 180), on the elevated helper's
//! side. The installer the user downloaded is copied to
//! `<FOLDERID_ProgramFiles>\wingetrs\staging\<32 hex>\` (a protected DACL: SYSTEM and
//! Administrators only), hashed while copied, opened again denying writes and hashed through
//! that handle, and run from there with `TEMP`/`TMP` inside the same folder. Motivo: a DLL
//! planted next to the installer, or in the user's `%TEMP%`, would be loaded as admin.
use super::worker::InstallerRequest;
use crate::install::msiexec_log;
use crate::winget_install::fetch::FetchError;
use crate::winget_install::switches;
use crate::winget_install::verified_win::VerifiedFile;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};
use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows::Win32::Security::{
    DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_TYPE_DISK, GetFileInformationByHandle, GetFileType, SECURITY_IDENTIFICATION,
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
/// Largest installer copied.
pub const COPY_MAX: u64 = 8 << 30;
/// Execution folders older than this are removed when the helper starts.
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
/// The installer log in the execution folder (`<LOGPATH>`).
pub const LOG_NAME: &str = "install.log";
/// The installer's `TEMP`/`TMP`, inside the execution folder.
pub const TMP_NAME: &str = "tmp";
/// Bytes read from the end of the installer log.
const LOG_WINDOW: u64 = msiexec_log::TAIL_WINDOW;
/// `FILE_ATTRIBUTE_REPARSE_POINT`.
const REPARSE: u32 = 0x400;
/// Message of a copy whose hash differs from the expected one.
pub const HASH_MISMATCH: &str = "o instalador não confere com o hash";

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

/// The verified copy of an installer in its execution folder. The handle drops before the
/// folder is removed (field order).
#[derive(Debug)]
pub struct Staged {
    file: VerifiedFile,
    run: RunDir,
}

impl Staged {
    /// The copy, open denying writes, delete and rename while `self` lives.
    pub fn file(&self) -> &Path {
        self.file.path()
    }

    pub fn run_dir(&self) -> &Path {
        self.run.path()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Opens every folder above `path`, from the volume root down, without following a reparse
/// point (one is refused) and without delete sharing, so none can be renamed or swapped for a
/// junction while the handles live. Each open resolves through the folders already held.
fn hold_plain_folders(path: &Path) -> std::io::Result<Vec<std::fs::File>> {
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

/// Opens the user's file `src` for reading as admin: no component of the path is followed if
/// it is a reparse point (refused; the folders are held while the file opens), a server on the
/// other end of a named pipe or device may only identify us, anything but a disk file is
/// refused, and so is a file with another hard link (it may be a name of a file elsewhere).
pub fn open_source(src: &Path) -> std::io::Result<std::fs::File> {
    let _folders = hold_plain_folders(src)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .security_qos_flags(SECURITY_IDENTIFICATION.0)
        .open(src)?;
    if file.metadata()?.file_attributes() & REPARSE != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "o instalador é um ponto de reanálise",
        ));
    }
    // SAFETY: a valid handle owned by `file` for the whole call.
    let kind = unsafe { GetFileType(HANDLE(file.as_raw_handle())) };
    if kind != FILE_TYPE_DISK {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "o instalador não é um arquivo em disco",
        ));
    }
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: a valid handle owned by `file` and an output structure.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(|e| io_err(&e))?;
    if info.nNumberOfLinks > 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "o instalador tem outro vínculo físico (hard link)",
        ));
    }
    Ok(file)
}

/// Copies `src` to the new file `dst` in a stream, at most `max` bytes, and checks its SHA-256
/// against `sha256` (hex, any case).
pub fn copy_hashed(src: &Path, dst: &Path, sha256: &str, max: u64) -> std::io::Result<()> {
    let mut input = open_source(src)
        .map_err(|e| std::io::Error::new(e.kind(), format!("instalador ilegível: {e}")))?;
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)?;
    let mut sha = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = match input.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        total += n as u64;
        if total > max {
            return Err(std::io::Error::other(format!(
                "instalador maior que {max} bytes"
            )));
        }
        sha.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    out.flush()?;
    if !hex(&sha.finalize()).eq_ignore_ascii_case(sha256.trim()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            HASH_MISMATCH,
        ));
    }
    Ok(())
}

/// `name` as a single safe path component (see `install::safepath`).
fn safe_name(name: &str) -> std::io::Result<&str> {
    match crate::install::safepath::safe_components(name).as_deref() {
        Some([one]) if one == name => Ok(name),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "nome do instalador inválido",
        )),
    }
}

/// Creates an execution folder under `root` with `sddl` (plus its `tmp`), copies and checks
/// `req.source` into it and opens the copy denying writes. On any error the folder is gone.
pub fn stage(root: &Path, sddl: &str, req: &InstallerRequest) -> std::io::Result<Staged> {
    let name = safe_name(&req.file_name)?;
    let run = RunDir::create(root, sddl)?;
    create_dir_with(&run.path().join(TMP_NAME), sddl)?;
    let copy = run.path().join(name);
    copy_hashed(Path::new(&req.source), &copy, &req.sha256, COPY_MAX)?;
    let file = VerifiedFile::open(&copy, &req.sha256).map_err(|e| match e {
        FetchError::Integrity => {
            std::io::Error::new(std::io::ErrorKind::InvalidData, HASH_MISMATCH)
        }
        other => std::io::Error::other(other.to_string()),
    })?;
    Ok(Staged { file, run })
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

/// The last lines of the log at `path` (UTF-16 with a BOM, as `msiexec` writes, or else
/// UTF-8), at most `max`; none when there is no log.
pub fn log_lines(path: &Path, max: usize) -> std::io::Result<Vec<String>> {
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut bom = [0u8; 2];
    let utf16 = file.read_exact(&mut bom).is_ok() && bom == [0xFF, 0xFE];
    let size = file.seek(SeekFrom::End(0))?;
    let text = if utf16 {
        msiexec_log::tail_text(&mut file, LOG_WINDOW)?
    } else {
        let start = size.saturating_sub(LOG_WINDOW);
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::with_capacity(LOG_WINDOW as usize);
        file.take(LOG_WINDOW).read_to_end(&mut bytes)?;
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let mut lines: Vec<&str> = text.split('\n').collect();
    // The window cut the first line: drop the part.
    if size > LOG_WINDOW && lines.len() > 1 {
        lines.remove(0);
    }
    let lines: Vec<String> = lines
        .into_iter()
        .map(|l| l.trim_end_matches(['\r', '\0']))
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    let skip = lines.len().saturating_sub(max);
    Ok(lines.into_iter().skip(skip).collect())
}

/// The program and parameters that run `staged`: `msiexec /i "<copy>" <params>` for msi/wix,
/// the copy itself otherwise; `<LOGPATH>` is the log in the execution folder.
pub fn command_for(
    staged: &Staged,
    req: &InstallerRequest,
    msiexec: &Path,
) -> std::io::Result<(PathBuf, String)> {
    let log = staged.run_dir().join(LOG_NAME);
    let params = switches::fill_log(&req.params, &log)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?;
    Ok(if req.kind.is_msi() {
        (
            msiexec.to_path_buf(),
            switches::msiexec_params(staged.file(), &params),
        )
    } else {
        (staged.file().to_path_buf(), params)
    })
}

/// Runs `staged` (see `command_for`) from its execution folder with `installer_env`, waits for
/// it, and sends the end of its log to `on_line`. The exit code.
pub fn run_staged(
    staged: &Staged,
    req: &InstallerRequest,
    msiexec: &Path,
    sys: &SystemEnv,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<i32> {
    let (program, params) = command_for(staged, req, msiexec)?;
    let run = staged.run_dir();
    let status = Command::new(&program)
        .raw_arg(&params)
        .current_dir(run)
        .env_clear()
        .envs(installer_env(sys, &run.join(TMP_NAME)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    let code = status
        .code()
        .ok_or_else(|| std::io::Error::other("o instalador terminou sem código de saída"))?;
    match log_lines(&run.join(LOG_NAME), super::worker::INSTALLER_LINES) {
        Ok(lines) => lines.iter().for_each(|l| on_line(l)),
        Err(e) => on_line(&format!("log do instalador ilegível: {e}")),
    }
    Ok(code)
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

/// The helper's `RunInstaller`: stages `req` under the production root (from the known folder)
/// and runs it with the system's `msiexec`.
pub fn run_installer(
    req: &InstallerRequest,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<i32> {
    let sys = SystemEnv::query()?;
    req.check(&sys.program_files_roots())
        .map_err(|r| std::io::Error::new(std::io::ErrorKind::InvalidInput, r))?;
    let root = staging_root(&sys.program_files)?;
    let msiexec = sys.system32.join("msiexec.exe");
    let staged = stage(&root, RUN_SDDL, req)?;
    run_staged(&staged, req, &msiexec, &sys, on_line)
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
