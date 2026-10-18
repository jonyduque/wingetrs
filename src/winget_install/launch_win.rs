//! ShellExecuteExW launcher (Task 178): starts a program unelevated with the shell's `open`
//! verb, waits for it and returns its exit code; plus `is_elevated` and `msiexec_path`.
use super::launch::{LaunchSpec, Launcher, Show};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;
use windows::Win32::Foundation::{ERROR_CANCELLED, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    FOLDERID_System, KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC,
    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHGetKnownFolderPath, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};
use windows::core::{HSTRING, PCWSTR, w};

/// Starts programs with `ShellExecuteExW` (verb `open`), never elevating on its own.
pub struct ShellLauncher;

impl Launcher for ShellLauncher {
    fn run(&self, spec: &LaunchSpec) -> std::io::Result<i32> {
        in_sta(|| shell_run(spec))
    }
}

/// A Win32 error as the `io::Error` std would build (so `kind()` works).
fn io_err(e: &windows::core::Error) -> std::io::Error {
    let code = e.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        std::io::Error::from_raw_os_error((code & 0xFFFF) as i32)
    } else {
        std::io::Error::from_raw_os_error(code as i32)
    }
}

fn own(h: HANDLE) -> Option<OwnedHandle> {
    if h.is_invalid() || h.0.is_null() {
        return None;
    }
    // SAFETY: a valid handle just returned to us, owned by nobody else.
    Some(unsafe { OwnedHandle::from_raw_handle(h.0) })
}

fn raw(h: &OwnedHandle) -> HANDLE {
    HANDLE(h.as_raw_handle())
}

/// Balances a successful `CoInitializeEx` even when the work panics.
struct ComGuard(bool);

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful CoInitializeEx on this same thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// Runs `f` on a fresh single-threaded-apartment thread, always joined; a panic inside `f`
/// is resumed on the caller.
fn in_sta<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            // SAFETY: a fresh thread; paired with CoUninitialize by the guard.
            let com =
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
            let _guard = ComGuard(com.is_ok());
            f()
        });
        match worker.join() {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    })
}

fn shell_run(spec: &LaunchSpec) -> std::io::Result<i32> {
    let file = HSTRING::from(spec.program.as_os_str());
    let params = HSTRING::from(spec.params.as_str());
    let dir = spec.dir.as_ref().map(|d| HSTRING::from(d.as_os_str()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("open"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: match spec.show {
            Show::Normal => SW_SHOWNORMAL.0,
            Show::Hidden => SW_HIDE.0,
        },
        ..Default::default()
    };
    // SAFETY: the strings outlive the call.
    match unsafe { ShellExecuteExW(&mut info) } {
        Ok(()) => {}
        // UAC refused for a program that elevates itself: the raw code, which
        // `ops::uac_refused` turns into a cancellation.
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => {
            return Err(std::io::Error::from_raw_os_error(ERROR_CANCELLED.0 as i32));
        }
        Err(e) => return Err(io_err(&e)),
    }
    // No process handle: the shell handed the file to something else; never invent a code.
    let process = own(info.hProcess).ok_or_else(|| {
        std::io::Error::other(format!(
            "o shell não criou um processo para {}",
            spec.program.display()
        ))
    })?;
    // SAFETY: the handle is open for the whole wait.
    let woke = unsafe { WaitForSingleObject(raw(&process), INFINITE) };
    if woke != WAIT_OBJECT_0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut code = 0u32;
    // SAFETY: the process has ended; `code` receives its exit code.
    unsafe { GetExitCodeProcess(raw(&process), &mut code) }.map_err(|e| io_err(&e))?;
    Ok(code as i32)
}

/// Whether this process runs with an elevated token.
pub fn is_elevated() -> std::io::Result<bool> {
    let mut token = HANDLE::default();
    // SAFETY: the pseudo handle of this process; `token` receives a new handle.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| io_err(&e))?;
    let token = own(token).ok_or_else(std::io::Error::last_os_error)?;
    let mut elevation = TOKEN_ELEVATION::default();
    let mut len = 0u32;
    // SAFETY: `elevation` is a TOKEN_ELEVATION of exactly the size passed.
    unsafe {
        GetTokenInformation(
            raw(&token),
            TokenElevation,
            Some((&raw mut elevation).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    }
    .map_err(|e| io_err(&e))?;
    Ok(elevation.TokenIsElevated != 0)
}

/// `System32\msiexec.exe` from the known folder, never from PATH or `%SystemRoot%`.
pub fn msiexec_path() -> std::io::Result<PathBuf> {
    // SAFETY: the returned string is freed below with CoTaskMemFree.
    let p = unsafe { SHGetKnownFolderPath(&FOLDERID_System, KF_FLAG_DEFAULT, None) }
        .map_err(|e| io_err(&e))?;
    // SAFETY: a NUL-terminated string from the call above.
    let text = unsafe { p.to_string() };
    // SAFETY: allocated by SHGetKnownFolderPath, freed once.
    unsafe { CoTaskMemFree(Some(p.0 as *const core::ffi::c_void)) };
    text.map(|t| PathBuf::from(t).join("msiexec.exe"))
        .map_err(|e| std::io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sta_worker_panic_is_propagated() {
        let caught = std::panic::catch_unwind(|| in_sta(|| -> i32 { panic!("t178-boom") }));
        let payload = caught.err();
        let text = payload
            .as_ref()
            .and_then(|p| p.downcast_ref::<&str>().copied());
        assert_eq!(text, Some("t178-boom"));
    }

    #[test]
    fn sta_worker_value_is_returned() {
        assert_eq!(in_sta(|| 41 + 1), 42);
    }
}
