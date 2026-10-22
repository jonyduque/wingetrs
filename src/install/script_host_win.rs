//! Real ScriptHost on Windows (Task 160).
//!
//! Runs the runner a hook step wrote with PowerShell: never through a `PATH` lookup (the
//! program comes from the known folders), never elevated, never when group policy forbids
//! scripts, and with a fixed `PATH` instead of the user's.
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_ProgramFiles, FOLDERID_System, FOLDERID_Windows, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows::core::GUID;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE};

use crate::bridge::spawn_win::{PIPE_GRACE, RunSpec, run_spec, utf8_lossy};
use crate::install::executor::{ScriptHost, ScriptJob, ScriptOutcome};
use crate::install::script_policy::{GpoValues, gpo_blocks, policy_error, refuse_if_elevated};
use crate::winget_install::launch_win::is_elevated;

/// pwsh 7 exports its own `PSModulePath`, which breaks a Windows PowerShell 5.1 child.
const CHILD_ENV_REMOVE: [&str; 1] = ["PSModulePath"];

const WINDOWS_POLICY_KEY: &str = r"SOFTWARE\Policies\Microsoft\Windows\PowerShell";
const CORE_POLICY_KEY: &str = r"SOFTWARE\Policies\Microsoft\PowerShellCore";

/// PowerShell at an absolute path, plus where its group policy and the elevation of this
/// process are read from.
pub struct PwshHost {
    program: PathBuf,
    gpo: fn(&Path) -> GpoValues,
    elevated: fn() -> io::Result<bool>,
}

impl PwshHost {
    /// `<ProgramFiles>\PowerShell\7\pwsh.exe` when installed, else Windows PowerShell 5.1
    /// (`<System32>\WindowsPowerShell\v1.0\powershell.exe`), both folders from
    /// `SHGetKnownFolderPath`: nothing is searched in `PATH`, the current folder or the App
    /// Execution Alias folder. Their DACL is not checked: the safety against a planted
    /// executable is that `run` refuses an elevated process, so a swapped program gains no
    /// more than the user already has. Neither found ⇒ `run` fails.
    pub fn locate() -> PwshHost {
        let pwsh7 = known(&FOLDERID_ProgramFiles)
            .ok()
            .map(|p| p.join("PowerShell").join("7").join("pwsh.exe"))
            .filter(|p| p.is_file());
        let program = pwsh7
            .or_else(|| {
                known(&FOLDERID_System).ok().map(|s| {
                    s.join("WindowsPowerShell")
                        .join("v1.0")
                        .join("powershell.exe")
                })
            })
            .unwrap_or_default();
        PwshHost {
            program,
            gpo: read_gpo,
            elevated: is_elevated,
        }
    }

    /// A given program (tests: a `.cmd` fake). The policy keys are not read; the elevation
    /// is the real one.
    pub fn with_program(program: PathBuf) -> PwshHost {
        PwshHost::with_program_and_checks(program, |_| GpoValues::default(), is_elevated)
    }

    /// Test seam: a given program, group policy and elevation.
    pub fn with_program_and_checks(
        program: PathBuf,
        gpo: fn(&Path) -> GpoValues,
        elevated: fn() -> io::Result<bool>,
    ) -> PwshHost {
        PwshHost {
            program,
            gpo,
            elevated,
        }
    }

    pub fn program(&self) -> &Path {
        &self.program
    }
}

impl ScriptHost for PwshHost {
    fn run(&self, job: &ScriptJob) -> io::Result<ScriptOutcome> {
        refuse_if_elevated((self.elevated)()?)
            .map_err(|m| io::Error::new(io::ErrorKind::PermissionDenied, m))?;
        if let Some(m) = gpo_blocks(&(self.gpo)(&self.program)) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, m));
        }
        if !self.program.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "PowerShell não encontrado (nem o 7 em Arquivos de Programas, nem o do Windows)",
            ));
        }
        let mut env = job.env.clone();
        // Last, so it wins over anything in `job.env`.
        env.push(("PATH".into(), child_path(&self.program)?));
        let args: Vec<String> = [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]
        .into_iter()
        .map(str::to_string)
        .chain([job.runner.to_string_lossy().into_owned()])
        .collect();
        let remove: Vec<OsString> = CHILD_ENV_REMOVE.iter().map(OsString::from).collect();
        let spec = RunSpec {
            program: &self.program,
            args: &args,
            dir: &job.workdir,
            env: &env,
            env_remove: &remove,
            decode: utf8_lossy,
            grace: PIPE_GRACE,
        };
        let hook = job.hook.key();
        let (code, lines) = run_spec(&spec, |l| {
            tracing::debug!(app = %job.app, hook, "{l}");
        })?;
        if code == 1
            && let Some(m) = policy_error(&lines)
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, m));
        }
        Ok(ScriptOutcome { code, lines })
    }
}

/// The child's whole `PATH`: `System32`, the Windows folder, `Wbem` and Windows PowerShell's
/// folder (all from the known folders), then the program's own folder. The user's `PATH` is
/// not inherited: a hook calling a bare `git` must not find one the user planted.
pub fn child_path(program: &Path) -> io::Result<OsString> {
    let system = known(&FOLDERID_System)?;
    let mut dirs = vec![
        system.clone(),
        known(&FOLDERID_Windows)?,
        system.join("Wbem"),
        system.join("WindowsPowerShell").join("v1.0"),
    ];
    if let Some(own) = program.parent().filter(|p| p.is_absolute())
        && !dirs
            .iter()
            .any(|d| d.as_os_str().eq_ignore_ascii_case(own.as_os_str()))
    {
        dirs.push(own.to_path_buf());
    }
    std::env::join_paths(dirs).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
}

/// A known folder's path.
pub(crate) fn known(id: &GUID) -> io::Result<PathBuf> {
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

/// The group policy that applies to `program` (read only): pwsh 7 reads `PowerShellCore`,
/// unless that scope says `UseWindowsPowerShellPolicySetting = 1`; Windows PowerShell reads
/// `Windows\PowerShell`.
pub(crate) fn read_gpo(program: &Path) -> GpoValues {
    let core = program
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("pwsh.exe"));
    let (machine_enable_scripts, machine_policy) = scope_gpo(HKEY_LOCAL_MACHINE, core);
    let (user_enable_scripts, user_policy) = scope_gpo(HKEY_CURRENT_USER, core);
    GpoValues {
        machine_enable_scripts,
        machine_policy,
        user_enable_scripts,
        user_policy,
    }
}

fn scope_gpo(hive: winreg::HKEY, core: bool) -> (Option<u32>, Option<String>) {
    let root = RegKey::predef(hive);
    let open = |path: &str| root.open_subkey_with_flags(path, KEY_QUERY_VALUE).ok();
    let key = if core {
        match open(CORE_POLICY_KEY) {
            Some(k)
                if k.get_value::<u32, _>("UseWindowsPowerShellPolicySetting")
                    .ok()
                    == Some(1) =>
            {
                open(WINDOWS_POLICY_KEY)
            }
            other => other,
        }
    } else {
        open(WINDOWS_POLICY_KEY)
    };
    match key {
        Some(k) => (
            k.get_value::<u32, _>("EnableScripts").ok(),
            k.get_value::<String, _>("ExecutionPolicy").ok(),
        ),
        None => (None, None),
    }
}
