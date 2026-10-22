//! Win32 side of the elevated bridge helper: the pipe (only the current user may open it), the
//! UAC launch, the connection deadline and the PID checks on both ends.
use super::elevated::Helper;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use windows::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_CANCELLED, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, ERROR_SUCCESS,
    GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT,
    WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
    WriteFile,
};
use windows::Win32::Storage::Packaging::Appx::{
    GetPackagePathByFullName, GetPackagesByPackageFamily,
};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId,
    PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, GetCurrentProcessId, GetProcessId, INFINITE, OpenProcess,
    OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, SetEvent,
    WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC,
    SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHGetKnownFolderPath, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

/// Every helper pipe is `PIPE_PREFIX` + 32 lowercase hex digits.
pub const PIPE_PREFIX: &str = r"\\.\pipe\wingetrs-elevated-";
/// The helper exits after this long without a request.
const IDLE_LIMIT: Duration = Duration::from_secs(10 * 60);
/// How long the helper has to connect once UAC let it start.
const CONNECT_LIMIT: Duration = Duration::from_secs(30);
const PIPE_BUFFER: u32 = 64 * 1024;
/// A `TOKEN_USER` is a few dozen bytes; anything bigger is refused before allocating.
const TOKEN_USER_MAX: u32 = 4096;

fn raw(h: &OwnedHandle) -> HANDLE {
    HANDLE(h.as_raw_handle())
}

/// `h` owned; an invalid handle is the thread's last error.
fn own(h: HANDLE) -> std::io::Result<OwnedHandle> {
    if h.is_invalid() || h.0.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a valid handle just returned to us, owned by nobody else.
    Ok(unsafe { OwnedHandle::from_raw_handle(h.0) })
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

fn is(e: &windows::core::Error, code: WIN32_ERROR) -> bool {
    e.code() == code.to_hresult()
}

/// A manual-reset event.
fn event() -> std::io::Result<OwnedHandle> {
    // SAFETY: no attributes, no name.
    own(unsafe { CreateEventW(None, true, false, None) }.map_err(|e| io_err(&e))?)
}

/// Runs one overlapped operation on `h` to completion; the bytes it moved.
/// An operation still pending after `limit` is cancelled and fails with `TimedOut`.
fn overlapped(
    h: HANDLE,
    limit: Option<Duration>,
    start: impl FnOnce(*mut OVERLAPPED) -> windows::core::Result<()>,
) -> std::io::Result<u32> {
    let done = event()?;
    let mut ov = OVERLAPPED {
        hEvent: raw(&done),
        ..Default::default()
    };
    match start(&mut ov) {
        Ok(()) => {}
        Err(e) if is(&e, ERROR_IO_PENDING) => {}
        Err(e) => return Err(io_err(&e)),
    }
    let mut n = 0u32;
    if let Some(limit) = limit {
        // SAFETY: the event is open for the whole wait.
        let woke = unsafe { WaitForSingleObject(raw(&done), millis(limit)) };
        if woke == WAIT_TIMEOUT {
            // SAFETY: cancels our own operation, then waits until it is over so `ov` can go.
            let finished = unsafe {
                let _ = CancelIoEx(h, Some(&ov));
                GetOverlappedResult(h, &ov, &mut n, true)
            };
            // Completed in the meantime: the data is ours.
            return match finished {
                Ok(()) => Ok(n),
                Err(_) => Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "nenhum pedido dentro do prazo",
                )),
            };
        }
    }
    // SAFETY: `ov` and its event outlive the operation: this waits for it to complete.
    unsafe { GetOverlappedResult(h, &ov, &mut n, true) }.map_err(|e| io_err(&e))?;
    Ok(n)
}

fn millis(d: Duration) -> u32 {
    u32::try_from(d.as_millis()).unwrap_or(u32::MAX)
}

/// Overlapped reads and writes on a pipe end; every call has its own event, so a read and a
/// write may be under way at once, and a `CancelIoEx` on the handle ends a blocked read.
/// A read that waits longer than `read_limit` is cancelled and fails with `TimedOut`.
#[derive(Clone)]
struct PipeIo {
    handle: Arc<OwnedHandle>,
    read_limit: Option<Duration>,
}

impl PipeIo {
    fn new(handle: Arc<OwnedHandle>) -> PipeIo {
        PipeIo {
            handle,
            read_limit: None,
        }
    }
}

impl Read for PipeIo {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let h = raw(&self.handle);
        // SAFETY: `buf` outlives the operation (`overlapped` waits for it).
        let read = overlapped(h, self.read_limit, |ov| unsafe {
            ReadFile(h, Some(buf), None, Some(ov))
        });
        match read {
            Ok(n) => Ok(n as usize),
            Err(e) if e.raw_os_error() == Some(ERROR_BROKEN_PIPE.0 as i32) => Ok(0),
            Err(e) => Err(e),
        }
    }
}

impl Write for PipeIo {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let h = raw(&self.handle);
        // SAFETY: `buf` outlives the operation (`overlapped` waits for it).
        overlapped(h, None, |ov| unsafe {
            WriteFile(h, Some(buf), None, Some(ov))
        })
        .map(|n| n as usize)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 16 random bytes from the system RNG, as hex.
pub(crate) fn random_hex() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    // SAFETY: the buffer is ours; no algorithm handle with the system-preferred flag.
    unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }
        .ok()
        .map_err(|e| io_err(&e))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
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

/// The SID of the user this process runs as, as `S-1-5-...`.
fn user_sid() -> std::io::Result<String> {
    let mut token = HANDLE::default();
    // SAFETY: the pseudo handle of this process; `token` receives a new handle.
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
        .map_err(|e| io_err(&e))?;
    let token = own(token)?;
    let mut len = 0u32;
    // SAFETY: size query; fails with ERROR_INSUFFICIENT_BUFFER and sets `len`.
    let _ = unsafe { GetTokenInformation(raw(&token), TokenUser, None, 0, &mut len) };
    if len == 0 || len > TOKEN_USER_MAX {
        return Err(std::io::Error::other(format!("TOKEN_USER com {len} bytes")));
    }
    // u64 words: aligned for the pointer inside TOKEN_USER.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` holds at least `len` bytes.
    unsafe {
        GetTokenInformation(
            raw(&token),
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut len,
        )
    }
    .map_err(|e| io_err(&e))?;
    // SAFETY: the call filled a TOKEN_USER at the start of `buf`, which stays alive below.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text = PWSTR::null();
    // SAFETY: `sid` points into `buf`; `text` receives a LocalAlloc string.
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.map_err(|e| io_err(&e))?;
    let _free = Local(text.0.cast());
    // SAFETY: a NUL-terminated string from the call above.
    unsafe { text.to_string() }.map_err(|e| std::io::Error::other(e.to_string()))
}

/// Creates the server end of `name`: one instance only (a squatter that created the name first
/// makes this fail), no remote clients, and only the current user may open it.
fn create_pipe(name: &str) -> std::io::Result<OwnedHandle> {
    let sddl = HSTRING::from(format!("D:P(A;;GRGW;;;{})", user_sid()?));
    let mut sd = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `sd` receives a LocalAlloc descriptor, freed by `_free`.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut sd, None)
    }
    .map_err(|e| io_err(&e))?;
    let _free = Local(sd.0);
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    // SAFETY: `sa` and its descriptor outlive the call.
    own(unsafe {
        CreateNamedPipeW(
            &HSTRING::from(name),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            Some(&sa),
        )
    })
}

/// One UAC launch of the helper: `exe args`, started in `dir` (System32, so neither the user's
/// current folder nor a relative name reaches the elevated process).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperLaunch {
    pub exe: PathBuf,
    pub args: String,
    pub dir: PathBuf,
}

/// Starts `spec` elevated through UAC (hidden); the process handle. The user saying no is
/// `ErrorKind::PermissionDenied`.
fn launch(spec: &HelperLaunch) -> std::io::Result<OwnedHandle> {
    // ShellExecuteEx wants COM; a thread already in another apartment is fine as it is.
    // SAFETY: paired with CoUninitialize below when it succeeded.
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    let file = HSTRING::from(spec.exe.as_os_str());
    let params = HSTRING::from(spec.args.as_str());
    let dir = HSTRING::from(spec.dir.as_os_str());
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: PCWSTR(dir.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    // SAFETY: the strings outlive the call; it blocks until the UAC prompt is answered.
    let started = unsafe { ShellExecuteExW(&mut info) };
    if com.is_ok() {
        // SAFETY: balances the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
    match started {
        Ok(()) => own(info.hProcess),
        Err(e) if is(&e, ERROR_CANCELLED) => Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "elevação recusada",
        )),
        Err(e) => Err(io_err(&e)),
    }
}

/// Waits for the helper `process` to connect to `pipe`, at most `limit`; it exiting first or
/// the time running out is an error.
fn accept(pipe: &OwnedHandle, process: &OwnedHandle, limit: Duration) -> std::io::Result<()> {
    let done = event()?;
    let mut ov = OVERLAPPED {
        hEvent: raw(&done),
        ..Default::default()
    };
    // SAFETY: `ov` lives until the operation is known complete or cancelled below.
    match unsafe { ConnectNamedPipe(raw(pipe), Some(&mut ov)) } {
        Ok(()) => return Ok(()),
        Err(e) if is(&e, ERROR_PIPE_CONNECTED) => return Ok(()),
        Err(e) if is(&e, ERROR_IO_PENDING) => {}
        Err(e) => return Err(io_err(&e)),
    }
    let ms = u32::try_from(limit.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: both handles are open for the whole wait.
    let woke = unsafe { WaitForMultipleObjects(&[raw(&done), raw(process)], false, ms) };
    let mut n = 0u32;
    if woke == WAIT_OBJECT_0 {
        // SAFETY: the event is set: the operation is complete.
        return unsafe { GetOverlappedResult(raw(pipe), &ov, &mut n, false) }
            .map_err(|e| io_err(&e));
    }
    // SAFETY: cancels our own operation, then waits until it is over so `ov` can go away.
    unsafe {
        let _ = CancelIoEx(raw(pipe), Some(&ov));
        let _ = GetOverlappedResult(raw(pipe), &ov, &mut n, true);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        if woke.0 == WAIT_OBJECT_0.0 + 1 {
            "o auxiliar elevado saiu sem conectar"
        } else {
            "o auxiliar elevado não conectou a tempo"
        },
    ))
}

/// The TUI's side of a connected helper.
pub struct PipeHelper {
    reader: BufReader<PipeIo>,
    writer: PipeIo,
    process: OwnedHandle,
}

impl Helper for PipeHelper {
    fn reader(&mut self) -> &mut dyn BufRead {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut dyn Write {
        &mut self.writer
    }

    fn wait_exit(&mut self, limit: Duration) -> bool {
        let ms = u32::try_from(limit.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: the process handle is open while `self` lives.
        let woke = unsafe { WaitForSingleObject(raw(&self.process), ms) };
        woke == WAIT_OBJECT_0
    }

    fn alive(&mut self) -> bool {
        // SAFETY: the process handle is open while `self` lives.
        let woke = unsafe { WaitForSingleObject(raw(&self.process), 0) };
        woke == WAIT_TIMEOUT
    }
}

/// The PID of `process`, which must be the client connected to `pipe`.
fn check_client(pipe: &OwnedHandle, process: &OwnedHandle) -> std::io::Result<u32> {
    let mut client = 0u32;
    // SAFETY: a connected server end.
    unsafe { GetNamedPipeClientProcessId(raw(pipe), &mut client) }.map_err(|e| io_err(&e))?;
    // SAFETY: the process handle is open.
    let helper = unsafe { GetProcessId(raw(process)) };
    if helper == 0 || client != helper {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("o pipe foi aberto pelo processo {client}, não pelo auxiliar {helper}"),
        ));
    }
    Ok(helper)
}

/// Whether the server of the connected client end `pipe` is the process `parent`.
fn check_server(pipe: &OwnedHandle, parent: u32) -> std::io::Result<()> {
    let mut server = 0u32;
    // SAFETY: a connected client end.
    unsafe { GetNamedPipeServerProcessId(raw(pipe), &mut server) }.map_err(|e| io_err(&e))?;
    if server != parent {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("o pipe é servido pelo processo {server}, não por {parent}"),
        ));
    }
    Ok(())
}

/// Creates the pipe, starts `exe bridge-worker` elevated and accepts its connection, which
/// must come from that very process. Only from a folder only administrators write (see
/// `start_with`).
pub fn start(exe: &Path) -> std::io::Result<PipeHelper> {
    start_noticed(exe, &mut || {})
}

/// [`start`], calling `notice` right before the UAC prompt opens (after the folder check, so a
/// refused folder shows no notice; Task 201).
pub fn start_noticed(exe: &Path, notice: &mut dyn FnMut()) -> std::io::Result<PipeHelper> {
    start_with(
        exe,
        &|_, f| super::arp_ops_win::handle_sddl(f),
        &mut |spec: &HelperLaunch| {
            notice();
            launch(spec)
        },
    )
}

/// `start` with the descriptors read by `sddl_of` and the UAC launch done by `launcher`.
/// Before `launcher` runs, `exe` and every folder above it must be administrator-only by owner
/// and DACL and none a reparse point (`arp_ops_win::hold_admin_chain_with`); the file stays
/// open without write or delete sharing, and the folders without delete sharing, until
/// `launcher` returns. A DLL the helper loads from its own folder is covered by the same
/// check: nobody else may create a file there. Otherwise no UAC prompt and the error is
/// `helper_folder::refusal_text`; no variable, setting or flag skips this. The helper starts
/// in System32 (from the known folder).
pub fn start_with(
    exe: &Path,
    sddl_of: super::arp_ops_win::SddlOf<'_>,
    launcher: &mut dyn FnMut(&HelperLaunch) -> std::io::Result<OwnedHandle>,
) -> std::io::Result<PipeHelper> {
    let folder = exe.parent().unwrap_or(exe);
    let held = super::arp_ops_win::hold_admin_chain_with(exe, sddl_of).map_err(|e| {
        tracing::warn!(error = %e, exe = %exe.display(), "bridge: helper folder not admin-only; no UAC");
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            super::helper_folder::refusal_text(folder),
        )
    })?;
    let system32 = super::staging_win::SystemEnv::query()?.system32;
    let name = format!("{PIPE_PREFIX}{}", random_hex()?);
    let pipe = create_pipe(&name)?;
    // SAFETY: no arguments.
    let me = unsafe { GetCurrentProcessId() };
    let process = launcher(&HelperLaunch {
        exe: exe.to_path_buf(),
        args: format!("bridge-worker --pipe {name} --parent {me}"),
        dir: system32,
    });
    drop(held);
    let process = process?;
    accept(&pipe, &process, CONNECT_LIMIT)?;
    let helper = check_client(&pipe, &process)?;
    tracing::info!(pid = helper, "bridge: elevated helper connected");
    let io = PipeIo::new(Arc::new(pipe));
    Ok(PipeHelper {
        reader: BufReader::new(io.clone()),
        writer: io,
        process,
    })
}

/// Whether `name` is a pipe name `start` makes.
fn valid_pipe_name(name: &str) -> bool {
    name.strip_prefix(PIPE_PREFIX).is_some_and(|hex| {
        hex.len() == 32
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Cancels the pipe's I/O when the parent exits, so a blocked read ends; stopped and joined
/// on drop, on every path.
struct ParentWatch {
    stop: Arc<OwnedHandle>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ParentWatch {
    fn spawn(parent: OwnedHandle, pipe: Arc<OwnedHandle>) -> std::io::Result<ParentWatch> {
        let stop = Arc::new(event()?);
        let wait_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            // SAFETY: the handles are owned by this closure (or shared with the guard) for the
            // whole wait.
            let woke = unsafe {
                WaitForMultipleObjects(&[raw(&parent), raw(&wait_stop)], false, INFINITE)
            };
            if woke == WAIT_OBJECT_0 {
                tracing::info!("bridge worker: parent exited; closing");
                // SAFETY: the pipe stays open while this thread holds its Arc.
                let _ = unsafe { CancelIoEx(raw(&pipe), None) };
            }
        });
        Ok(ParentWatch {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for ParentWatch {
    fn drop(&mut self) {
        // SAFETY: the event is open while the guard lives.
        let _ = unsafe { SetEvent(raw(&self.stop)) };
        if let Some(t) = self.thread.take()
            && let Err(p) = t.join()
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(p);
        }
    }
}

/// `wingetrs bridge-worker`: connects to `pipe`, checks that its server is `parent`, and serves
/// it with the real bridge exec until it quits, closes or the parent exits.
pub fn worker_main(pipe: &str, parent: u32) -> std::io::Result<()> {
    worker_main_with(pipe, parent, &harden_process)
}

/// Hardening of the elevated helper, before anything else runs: DLLs are loaded from System32
/// only (never from the user's PATH or current folder, for a missing or delay-loaded DLL), and
/// the current folder is System32 (from the known folder).
pub fn harden_process() -> std::io::Result<()> {
    // SAFETY: plain process-wide setting.
    unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) }.map_err(|e| io_err(&e))?;
    std::env::set_current_dir(super::staging_win::SystemEnv::query()?.system32)
}

/// `worker_main` with the process hardening done by `harden`, first; its failure stops the
/// helper before the pipe is opened.
pub fn worker_main_with(
    pipe: &str,
    parent: u32,
    harden: &dyn Fn() -> std::io::Result<()>,
) -> std::io::Result<()> {
    harden()?;
    if !valid_pipe_name(pipe) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "nome de pipe inválido",
        ));
    }
    // Opened before the check, so the PID cannot be reused by another process meanwhile.
    // SAFETY: plain open by PID.
    let parent_handle = own(unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            parent,
        )
    }
    .map_err(|e| io_err(&e))?)?;
    // Identification only: the server may learn who we are but never act as us.
    // SAFETY: plain open of a pipe name.
    let handle = own(unsafe {
        CreateFileW(
            &HSTRING::from(pipe),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            None,
        )
    }
    .map_err(|e| io_err(&e))?)?;
    check_server(&handle, parent)?;
    tracing::info!(parent, "bridge worker: connected");
    let handle = Arc::new(handle);
    let _watch = ParentWatch::spawn(parent_handle, Arc::clone(&handle))?;
    let io = PipeIo::new(handle);
    // The helper only waits on a read for the next request: a read that long means idle.
    let mut reader = BufReader::new(PipeIo {
        read_limit: Some(IDLE_LIMIT),
        ..io.clone()
    });
    let mut writer = io;
    super::staging_win::prune_at_start();
    super::worker::serve(&mut reader, &mut writer, &mut SystemOps)
}

/// The worker's real operations: winget (`run_winget`), HKLM uninstallers, ARP deletions and
/// installers it derives from the signed catalog (Task 202; never one named by the request).
struct SystemOps;

impl super::worker::WorkerOps for SystemOps {
    fn run(
        &mut self,
        program: &str,
        args: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<(i32, Vec<String>)> {
        run_winget(program, args, on_line)
    }

    fn program_files(&self) -> Vec<String> {
        match super::staging_win::SystemEnv::query() {
            Ok(sys) => sys.program_files_roots(),
            Err(e) => {
                tracing::warn!(error = %e, "bridge worker: known folders unavailable");
                Vec::new()
            }
        }
    }

    fn known_folders(
        &self,
    ) -> std::io::Result<crate::winget_install::elevated_rules::KnownFolders> {
        super::arp_ops_win::known_folders()
    }

    fn read_arp(
        &mut self,
        hive: crate::installed::Hive,
        key: &str,
    ) -> std::io::Result<Option<crate::winget_install::uninstall_cmd::ArpDetails>> {
        crate::winget_install::arp_read_win::read_details(hive, key)
    }

    fn arp_entries(
        &mut self,
    ) -> std::io::Result<Vec<crate::winget_install::uninstall_cmd::ArpDetails>> {
        super::arp_ops_win::arp_entries()
    }

    fn exists(&self, path: &Path) -> bool {
        super::arp_ops_win::exists(path)
    }

    fn msiexec(&self) -> std::io::Result<PathBuf> {
        crate::winget_install::launch_win::msiexec_path()
    }

    fn long_path(&self, path: &Path) -> std::io::Result<PathBuf> {
        super::arp_ops_win::long_path(path)
    }

    fn run_uninstaller(
        &mut self,
        program: &Path,
        args: &str,
        also_checked: &[PathBuf],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<i32> {
        super::arp_ops_win::run_uninstaller(program, args, also_checked, on_line)
    }

    fn delete_arp_key(
        &mut self,
        hive: super::worker::MachineHive,
        key: &str,
        confirm: super::worker::DeleteCheck<'_>,
    ) -> std::io::Result<super::worker::ArpDelete> {
        super::arp_ops_win::delete_key(hive, key, confirm)
    }

    fn install_package(
        &mut self,
        req: &super::worker::PackageRequest,
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<i32> {
        install_package(req, on_line)
    }
}

/// The real `install_package` (Task 202): a fresh run folder under the staging root; the catalog
/// and the installer downloaded through `Http::for_elevated` (never `Http::new`: the user's proxy
/// and roots), the catalog verified by `WinTrustVerifier` (`signed_install::prepare`); then
/// `run_staged`. The folder goes once everything ended, or stays for the 24 h prune when the
/// installer left processes running.
fn install_package(
    req: &super::worker::PackageRequest,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<i32> {
    use super::signed_install::{INSTALLER_RUNNING, SignedCtx, prepare};
    use super::staging_win::{
        INSTALL_LINGER, RUN_SDDL, RunDir, SystemEnv, run_staged, staging_root,
    };
    let sys = SystemEnv::query()?;
    let root = staging_root(&sys.program_files)?;
    let run = RunDir::create(&root, RUN_SDDL)?;
    let roots = crate::selfupdate::apply_win::machine_roots()?;
    let http = crate::http::Http::for_elevated(&roots)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let program_files = sys.program_files_roots();
    let msiexec = sys.system32.join("msiexec.exe");
    let ctx = SignedCtx {
        http: &http,
        cdn_base: crate::sources::winget::CDN_BASE,
        verifier: &super::catalog_verify_win::WinTrustVerifier,
        now: std::time::SystemTime::now(),
        program_files: &program_files,
        msiexec: &msiexec,
        allow_http: false,
        installer_max: super::signed_install::INSTALLER_MAX,
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let staged = rt
        .block_on(prepare(&ctx, req, run.path(), on_line))
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    drop(rt);
    on_line(INSTALLER_RUNNING);
    let end = run_staged(&staged, &sys, INSTALL_LINGER, on_line)?;
    if !end.all_ended {
        let kept = run.keep();
        tracing::warn!(folder = %kept.display(), "bridge worker: installer left processes running; folder kept for prune");
    }
    Ok(end.code)
}

/// The worker's exec: only `winget`, and only the one Windows installed (see `winget_exe`),
/// run by absolute path from its own folder; never what `PATH` or an alias would find.
fn run_winget(
    program: &str,
    args: &[String],
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    if program != "winget" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("o auxiliar elevado só roda o winget, não {program}"),
        ));
    }
    let exe = winget_exe()?;
    let sys = super::staging_win::SystemEnv::query()?;
    let root = super::staging_win::staging_root(&sys.program_files)?;
    run_winget_at(
        &exe,
        args,
        &root,
        super::staging_win::RUN_SDDL,
        &sys,
        on_line,
    )
}

/// Runs the winget at `exe` (already resolved by `winget_exe`) from its own folder. The
/// environment is built from scratch, `TEMP`/`TMP` in an execution folder under `root`
/// (created with `sddl`): the worker's own environment comes from the user
/// (`HKCU\Environment`).
pub fn run_winget_at(
    exe: &Path,
    args: &[String],
    root: &Path,
    sddl: &str,
    sys: &super::staging_win::SystemEnv,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    let dir = exe
        .parent()
        .ok_or_else(|| std::io::Error::other("winget sem pasta"))?;
    super::staging_win::run_tool(exe, args, dir, root, sddl, sys, on_line)
}

/// Package family of App Installer, which ships `winget.exe`.
const WINGET_FAMILY: &str = "Microsoft.DesktopAppInstaller_8wekyb3d8bbwe";
/// Most packages of one family looked at.
const PACKAGES_MAX: u32 = 64;
/// Longest package name list or package path read, in UTF-16 units.
const NAMES_MAX: u32 = 64 * 1024;

/// `<package_dir>\winget.exe` when `package_dir` is an App Installer folder right under
/// `<program_files>\WindowsApps` (only TrustedInstaller writes there); `None` otherwise.
fn winget_in(program_files: &Path, package_dir: &Path) -> Option<PathBuf> {
    let same = |a: &Component<'_>, b: &Component<'_>| {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    };
    let apps = program_files.join("WindowsApps");
    let root: Vec<Component<'_>> = apps.components().collect();
    let dir: Vec<Component<'_>> = package_dir.components().collect();
    if !program_files.is_absolute()
        || dir.len() != root.len() + 1
        || !root.iter().zip(&dir).all(|(a, b)| same(a, b))
    {
        return None;
    }
    let Some(Component::Normal(name)) = dir.last() else {
        return None;
    };
    let name = name.to_str()?.to_ascii_lowercase();
    let ours =
        name.starts_with("microsoft.desktopappinstaller_") && name.ends_with("__8wekyb3d8bbwe");
    ours.then(|| package_dir.join("winget.exe"))
}

/// `Program Files` from the known folder, never from an environment variable.
pub(super) fn program_files() -> std::io::Result<PathBuf> {
    // SAFETY: the returned string is freed below with CoTaskMemFree.
    let p = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, None) }
        .map_err(|e| io_err(&e))?;
    // SAFETY: a NUL-terminated string from the call above.
    let text = unsafe { p.to_string() };
    // SAFETY: allocated by SHGetKnownFolderPath, freed once.
    unsafe { CoTaskMemFree(Some(p.0 as *const core::ffi::c_void)) };
    text.map(PathBuf::from)
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Full names of the installed packages of `family`.
fn family_packages(family: &str) -> std::io::Result<Vec<String>> {
    let family = HSTRING::from(family);
    let (mut count, mut len) = (0u32, 0u32);
    // SAFETY: size query only.
    let _ = unsafe { GetPackagesByPackageFamily(&family, &mut count, None, &mut len, None) };
    if count == 0 {
        return Ok(Vec::new());
    }
    if count > PACKAGES_MAX || len > NAMES_MAX {
        return Err(std::io::Error::other(format!(
            "{count} pacotes, {len} caracteres"
        )));
    }
    let mut names = vec![PWSTR::null(); count as usize];
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `names` holds `count` pointers and `buf` `len` units, as the call is told.
    let status = unsafe {
        GetPackagesByPackageFamily(
            &family,
            &mut count,
            Some(names.as_mut_ptr()),
            &mut len,
            Some(PWSTR(buf.as_mut_ptr())),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status.0 as i32));
    }
    names
        .iter()
        .take(count as usize)
        // SAFETY: each pointer is a NUL-terminated name inside `buf`, still alive.
        .map(|n| unsafe { n.to_string() }.map_err(|e| std::io::Error::other(e.to_string())))
        .collect()
}

/// Install folder of the package `full_name`.
fn package_path(full_name: &str) -> std::io::Result<PathBuf> {
    let name = HSTRING::from(full_name);
    let mut len = 0u32;
    // SAFETY: size query only.
    let _ = unsafe { GetPackagePathByFullName(&name, &mut len, None) };
    if len == 0 || len > NAMES_MAX {
        return Err(std::io::Error::other(format!(
            "caminho com {len} caracteres"
        )));
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` holds `len` units, as the call is told.
    let status =
        unsafe { GetPackagePathByFullName(&name, &mut len, Some(PWSTR(buf.as_mut_ptr()))) };
    if status != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(status.0 as i32));
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(PathBuf::from(String::from_utf16_lossy(&buf[..end])))
}

/// `winget.exe` of the App Installer package Windows installed for this user, under
/// `Program Files\WindowsApps`; `NotFound` when there is none in that place.
pub fn winget_exe() -> std::io::Result<PathBuf> {
    let root = program_files()?;
    // The first package of the family (in the system's order) that sits in the protected
    // folder and has winget.exe wins; versions are not compared, since any copy there was
    // put by Windows. A package elsewhere (a developer registration, say) is skipped, and
    // none in place fails closed: nothing runs rather than a winget from a writable folder.
    for full_name in family_packages(WINGET_FAMILY)? {
        let found = package_path(&full_name)
            .ok()
            .and_then(|dir| winget_in(&root, &dir))
            .filter(|exe| exe.is_file());
        if let Some(exe) = found {
            return Ok(exe);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        r"winget do App Installer não encontrado em Program Files\WindowsApps",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The DACL of `h` as SDDL.
    fn dacl(h: &OwnedHandle) -> String {
        use windows::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
        };
        use windows::Win32::Security::DACL_SECURITY_INFORMATION;
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let status = unsafe {
            GetSecurityInfo(
                raw(h),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                None,
                None,
                Some(&mut sd),
            )
        };
        assert_eq!(status.0, 0);
        let _free = Local(sd.0);
        let mut text = PWSTR::null();
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                None,
            )
        }
        .unwrap();
        let _text = Local(text.0.cast());
        unsafe { text.to_string() }.unwrap()
    }

    #[test]
    fn pipe_names_are_only_ours() {
        let good = format!("{PIPE_PREFIX}{}", "0123456789abcdef".repeat(2));
        assert!(valid_pipe_name(&good));
        assert!(!valid_pipe_name(&good.to_uppercase()));
        assert!(!valid_pipe_name(&format!("{good}0")));
        assert!(!valid_pipe_name(&good[..good.len() - 1]));
        assert!(!valid_pipe_name(&good.replace("wingetrs", "outro")));
        assert!(!valid_pipe_name(&format!(
            "{PIPE_PREFIX}{}",
            "g".repeat(32)
        )));
    }

    #[test]
    fn random_names_differ() {
        let (a, b) = (random_hex().unwrap(), random_hex().unwrap());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(valid_pipe_name(&format!("{PIPE_PREFIX}{a}")));
    }

    fn open_client(name: &str) -> OwnedHandle {
        own(unsafe {
            CreateFileW(
                &HSTRING::from(name),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        }
        .unwrap())
        .unwrap()
    }

    fn process(pid: u32) -> OwnedHandle {
        let access = PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
        own(unsafe { OpenProcess(access, false, pid) }.unwrap()).unwrap()
    }

    fn me() -> u32 {
        unsafe { GetCurrentProcessId() }
    }

    /// A handle to a process that already exited, and its PID.
    fn exited_process() -> (OwnedHandle, u32) {
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "exit", "0"])
            .spawn()
            .unwrap();
        let handle = process(child.id());
        child.wait().unwrap();
        (handle, child.id())
    }

    fn new_pipe() -> (String, OwnedHandle) {
        let name = format!("{PIPE_PREFIX}{}", random_hex().unwrap());
        let server = create_pipe(&name).unwrap();
        (name, server)
    }

    /// The DACL and the overlapped I/O over a real pipe, both ends in this process (no UAC).
    #[test]
    fn pipe_round_trip_and_dacl() {
        let (name, server) = new_pipe();
        // A second instance of the same name is refused (one instance, first-instance flag).
        assert!(create_pipe(&name).is_err());
        // GRGW is stored as FILE_GENERIC_READ | FILE_GENERIC_WRITE; protected, nobody else.
        let expected = format!("D:P(A;;0x12019f;;;{})", user_sid().unwrap());
        assert_eq!(dacl(&server), expected);
        let client = open_client(&name);
        accept(&server, &process(me()), Duration::from_secs(5)).unwrap();
        let mut s = PipeIo::new(Arc::new(server));
        let mut c = BufReader::new(PipeIo::new(Arc::new(client)));
        s.write_all(
            b"ola
",
        )
        .unwrap();
        let mut line = String::new();
        c.read_line(&mut line).unwrap();
        assert_eq!(
            line,
            "ola
"
        );
        // The server end closing reads as the end of the stream.
        drop(s);
        line.clear();
        assert_eq!(c.read_line(&mut line).unwrap(), 0);
    }

    #[test]
    fn pid_checks_accept_only_the_expected_process() {
        let (name, server) = new_pipe();
        let client = open_client(&name);
        accept(&server, &process(me()), Duration::from_secs(5)).unwrap();
        assert_eq!(check_client(&server, &process(me())).unwrap(), me());
        let (other, other_pid) = exited_process();
        let e = check_client(&server, &other).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
        check_server(&client, me()).unwrap();
        let e = check_server(&client, other_pid).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn accept_gives_up_when_the_helper_exits_or_never_connects() {
        let (_, server) = new_pipe();
        let (dead, _) = exited_process();
        let e = accept(&server, &dead, Duration::from_secs(30)).unwrap_err();
        assert!(e.to_string().contains("saiu sem conectar"), "{e}");
        // The pipe is still usable afterwards; nobody connects within the limit.
        let e = accept(&server, &process(me()), Duration::from_millis(50)).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
        assert!(e.to_string().contains("não conectou a tempo"), "{e}");
    }

    #[test]
    fn idle_read_times_out_and_data_in_time_is_read() {
        let (name, server) = new_pipe();
        let client = open_client(&name);
        accept(&server, &process(me()), Duration::from_secs(5)).unwrap();
        let mut s = PipeIo::new(Arc::new(server));
        let mut c = PipeIo {
            read_limit: Some(Duration::from_millis(100)),
            ..PipeIo::new(Arc::new(client))
        };
        let mut buf = [0u8; 16];
        std::thread::scope(|scope| {
            // Detector: without the limit the read would return this, much later, instead.
            let late = scope.spawn(|| {
                std::thread::sleep(Duration::from_secs(3));
                s.write_all(b"tarde").unwrap();
            });
            let e = c.read(&mut buf).unwrap_err();
            assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
            if let Err(p) = late.join() {
                std::panic::resume_unwind(p);
            }
        });
        // Data already there is read within the limit.
        assert_eq!(c.read(&mut buf).unwrap(), 5);
        assert_eq!(&buf[..5], b"tarde");
    }

    #[test]
    fn winget_only_from_an_app_installer_folder_under_windowsapps() {
        let pf = Path::new(r"C:\Program Files");
        let pkg = "Microsoft.DesktopAppInstaller_1.29.380.0_x64__8wekyb3d8bbwe";
        let inside = pf.join("WindowsApps").join(pkg);
        assert_eq!(winget_in(pf, &inside), Some(inside.join("winget.exe")));
        let lower = Path::new(r"c:\program files\windowsapps").join(pkg);
        assert_eq!(winget_in(pf, &lower), Some(lower.join("winget.exe")));
        for bad in [
            format!(r"C:\Users\u\AppData\Local\Microsoft\WindowsApps\{pkg}"),
            format!(r"C:\Program Files\WindowsAppsX\{pkg}"),
            format!(r"C:\Program Files\WindowsApps\..\{pkg}"),
            format!(r"C:\Program Files\WindowsApps\{pkg}\.."),
            format!(r"C:\Program Files\WindowsApps\{pkg}\sub"),
            format!(r"D:\Program Files\WindowsApps\{pkg}"),
            format!(r"Program Files\WindowsApps\{pkg}"),
            r"C:\Program Files\WindowsApps".to_string(),
            r"C:\Program Files\WindowsApps\Evil.App_1.0.0.0_x64__8wekyb3d8bbwe".to_string(),
            r"C:\Program Files\WindowsApps\Microsoft.DesktopAppInstaller_1.0_x64__outro"
                .to_string(),
        ] {
            assert_eq!(winget_in(pf, Path::new(&bad)), None, "{bad}");
        }
        assert_eq!(winget_in(Path::new("Program Files"), &inside), None);
    }

    /// Resolves the real winget and runs only `--version` with it (no elevation).
    #[test]
    #[ignore = "live: winget instalado nesta máquina"]
    fn live_winget_exe_resolves_and_runs_version() {
        let exe = winget_exe().unwrap();
        let dir = exe.parent().unwrap();
        let mut lines = Vec::new();
        let version = ["--version".to_string()];
        let (code, _) = crate::bridge::spawn_win::run_in(&exe, &version, dir, |l: &str| {
            lines.push(l.to_string())
        })
        .unwrap();
        assert_eq!(code, 0, "{lines:?}");
        assert!(
            lines.iter().any(|l| l.starts_with('v')),
            "{exe:?} {lines:?}"
        );
        // Through the worker's exec too, and nothing but winget.
        let mut seen = Vec::new();
        let (code, _) =
            run_winget("winget", &version, &mut |l: &str| seen.push(l.to_string())).unwrap();
        assert_eq!((code, seen), (0, lines));
    }

    #[test]
    fn worker_exec_runs_nothing_but_winget() {
        let e = run_winget("cmd", &["/c".into(), "exit".into()], &mut |_: &str| {}).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
    }
}
