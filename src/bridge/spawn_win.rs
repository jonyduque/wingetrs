//! Runs winget or scoop with no window and stdin closed, reading stdout and stderr line by line.
use super::Tail;
pub use super::utf8_lossy;
use std::ffi::OsString;
use std::io::Read;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CHUNK: usize = 8 * 1024;
/// `ERROR_OPERATION_ABORTED`: a read cancelled by `Readers::stop`.
const OPERATION_ABORTED: i32 = 995;
const POLL: Duration = Duration::from_millis(100);
const CANCEL_RETRY: Duration = Duration::from_millis(50);

/// How long the pipes are still read after the tool exits. A grandchild that inherited them
/// (an installer, scoop's `git`) would otherwise hold the bridge until it dies.
pub const PIPE_GRACE: Duration = Duration::from_secs(2);

/// A pipe reader; it ends with the pipe's unterminated last line. The slot shares the pipe
/// with the thread, so the handle stays open (and cannot be reused) until the slot is dropped,
/// which is always after the join, even when the thread panicked.
struct Reader {
    thread: JoinHandle<std::io::Result<Option<String>>>,
    _pipe: Arc<dyn Send + Sync>,
    raw: usize,
}

/// The stdout and stderr readers. Joined on every path: explicitly in `run`, or by `Drop`
/// (an error or a panic of `on_line`), so they never outlive the call.
struct Readers {
    out: Option<Reader>,
    err: Option<Reader>,
    stop: Arc<AtomicBool>,
}

/// Waits for a reader; a reader panic is propagated.
fn join(slot: &mut Option<Reader>) -> std::io::Result<Option<String>> {
    match slot.take().map(|r| r.thread.join()) {
        None => Ok(None),
        Some(Ok(rest)) => rest,
        Some(Err(p)) => std::panic::resume_unwind(p),
    }
}

impl Readers {
    /// Makes both readers end at their next read or in the one under way, and waits until they
    /// have. The output still in the pipes is dropped; whoever writes it is left alone.
    /// The cancel targets the pipe, not the thread: std reads child pipes overlapped
    /// (`ReadFileEx` + alertable `SleepEx`), which `CancelSynchronousIo` cannot reach.
    fn stop(&self) {
        self.stop_with(cancel_io);
    }

    /// `stop` with the cancel injected, so a test can make it take effect only on a later try.
    fn stop_with(&self, cancel: fn(usize)) {
        self.stop.store(true, Ordering::SeqCst);
        loop {
            let live: Vec<&Reader> = [&self.out, &self.err]
                .into_iter()
                .flatten()
                .filter(|r| !r.thread.is_finished())
                .collect();
            if live.is_empty() {
                return;
            }
            for r in live {
                cancel(r.raw);
            }
            std::thread::sleep(CANCEL_RETRY);
        }
    }
}

/// Cancels the read under way on the pipe `raw`.
fn cancel_io(raw: usize) {
    let pipe = windows::Win32::Foundation::HANDLE(raw as *mut std::ffi::c_void);
    // SAFETY: the caller's `Reader::_pipe` keeps the pipe open for as long as the reader lives,
    // so the handle is valid here whatever the thread did.
    // `ERROR_NOT_FOUND` (no read under way right now) is expected: retried by the caller.
    let _ = unsafe { windows::Win32::System::IO::CancelIoEx(pipe, None) };
}

impl Drop for Readers {
    fn drop(&mut self) {
        // A grandchild may still hold the pipes: never wait for their EOF here.
        self.stop();
        // While already unwinding, a second panic would abort: just reap the threads.
        if std::thread::panicking() {
            for r in [self.out.take(), self.err.take()].into_iter().flatten() {
                let _ = r.thread.join();
            }
        } else {
            let _ = join(&mut self.out);
            let _ = join(&mut self.err);
        }
    }
}

/// Reads `pipe` until EOF or `stop`; an abort (995) is the end only once `stop` is set.
fn read_loop(
    pipe: &mut impl Read,
    stop: &AtomicBool,
    tx: &Sender<String>,
    decode: fn(&[u8]) -> String,
) -> std::io::Result<Option<String>> {
    let mut tail = Tail::with_decoder(decode);
    let mut buf = vec![0u8; CHUNK];
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let n = match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e)
                if e.raw_os_error() == Some(OPERATION_ABORTED) && stop.load(Ordering::SeqCst) =>
            {
                break;
            }
            Err(e) => return Err(e),
        };
        for line in tail.push(&buf[..n]) {
            // No receiver means the caller is unwinding: keep draining so the tool can exit.
            let _ = tx.send(line);
        }
    }
    Ok(tail.finish())
}

fn spawn_reader(
    pipe: impl Read + AsRawHandle + Send + 'static,
    tx: Sender<String>,
    stop: Arc<AtomicBool>,
    decode: fn(&[u8]) -> String,
) -> Reader {
    let raw = pipe.as_raw_handle() as usize;
    let pipe = Arc::new(Mutex::new(pipe));
    let shared = Arc::clone(&pipe);
    let thread = std::thread::spawn(move || {
        // Only this thread locks it; the slot just keeps it alive.
        let mut pipe = shared.lock().unwrap_or_else(PoisonError::into_inner);
        read_loop(&mut *pipe, &stop, &tx, decode)
    });
    Reader {
        thread,
        _pipe: pipe,
        raw,
    }
}

/// Runs `program` and waits for it with no time limit; `on_line` gets every useful output
/// line (stdout and stderr, in arrival order). Returns the exit code and the last lines.
/// A missing program is `ErrorKind::NotFound`.
pub fn run(
    program: &str,
    args: &[String],
    on_line: impl FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    run_with_grace(program, args, PIPE_GRACE, on_line)
}

/// `run`, with the pipes read for at most `grace` after the tool exits. The exit code is the
/// tool's even when the reading was cut short.
pub fn run_with_grace(
    program: &str,
    args: &[String],
    grace: Duration,
    on_line: impl FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    run_command(cmd, program, grace, utf8_lossy, on_line)
}

/// `run` of the program at the absolute path `program` (no `PATH` search), started in `dir`.
pub fn run_in(
    program: &std::path::Path,
    args: &[String],
    dir: &std::path::Path,
    on_line: impl FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(dir);
    run_command(
        cmd,
        &program.display().to_string(),
        PIPE_GRACE,
        utf8_lossy,
        on_line,
    )
}

/// `run_in` with the whole environment given by `env` (`env_clear` first: nothing comes from
/// this process's environment). For the elevated helper's winget (Task 190).
pub fn run_in_env(
    program: &Path,
    args: &[String],
    dir: &Path,
    env: &[(OsString, OsString)],
    on_line: impl FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    if !program.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "caminho do programa precisa ser absoluto",
        ));
    }
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(dir)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)));
    run_command(
        cmd,
        &program.display().to_string(),
        PIPE_GRACE,
        utf8_lossy,
        on_line,
    )
}

/// Everything `run_spec` needs: an absolute program, its arguments and working dir, the
/// environment changes (applied over the inherited one), the line decoder and the pipe grace.
pub struct RunSpec<'a> {
    pub program: &'a Path,
    pub args: &'a [String],
    pub dir: &'a Path,
    pub env: &'a [(OsString, OsString)],
    pub env_remove: &'a [OsString],
    pub decode: fn(&[u8]) -> String,
    pub grace: Duration,
}

/// `run_in` with environment changes, a byte decoder and a grace. A relative program is
/// `ErrorKind::InvalidInput` (no `PATH` or current-dir search ever).
pub fn run_spec(
    spec: &RunSpec<'_>,
    on_line: impl FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    if !spec.program.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "caminho do programa precisa ser absoluto",
        ));
    }
    let mut cmd = Command::new(spec.program);
    cmd.args(spec.args).current_dir(spec.dir);
    cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));
    for name in spec.env_remove {
        cmd.env_remove(name);
    }
    run_command(
        cmd,
        &spec.program.display().to_string(),
        spec.grace,
        spec.decode,
        on_line,
    )
}

fn run_command(
    mut cmd: Command,
    program: &str,
    grace: Duration,
    decode: fn(&[u8]) -> String,
    mut on_line: impl FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    let (tx, rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let mut readers = Readers {
        out: child
            .stdout
            .take()
            .map(|p| spawn_reader(p, tx.clone(), Arc::clone(&stop), decode)),
        err: child
            .stderr
            .take()
            .map(|p| spawn_reader(p, tx.clone(), Arc::clone(&stop), decode)),
        stop,
    };
    drop(tx);
    let mut tail = Tail::new();
    let mut exited: Option<(Instant, i32)> = None;
    let mut cut = false;
    loop {
        // Ends when both readers are done (each holds a sender).
        match rx.recv_timeout(POLL) {
            Ok(line) => {
                on_line(&line);
                tail.add(line);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        // Checked on every turn, so a grandchild that never stops writing cannot delay the cut.
        if exited.is_none()
            && let Some(status) = child.try_wait()?
        {
            exited = Some((Instant::now(), status.code().unwrap_or(-1)));
        }
        if let Some((at, _)) = exited
            && !cut
            && at.elapsed() >= grace
        {
            tracing::warn!(
                program,
                ?grace,
                "tool exited but its output is still held open; stopped reading"
            );
            readers.stop();
            cut = true;
        }
    }
    let rests = join(&mut readers.out).and_then(|o| Ok((o, join(&mut readers.err)?)));
    let (out_rest, err_rest) = match rests {
        Ok(r) => r,
        Err(e) => {
            // Reap the tool before reporting; its own exit status is moot after a read error.
            let _ = child.wait();
            return Err(e);
        }
    };
    for line in [out_rest, err_rest].into_iter().flatten() {
        on_line(&line);
        tail.add(line);
    }
    let code = match exited {
        Some((_, code)) => code,
        None => child.wait()?.code().unwrap_or(-1),
    };
    Ok((code, tail.lines()))
}

/// How often `run_in_job` looks at the job while it waits.
const JOB_POLL: Duration = Duration::from_millis(50);

/// `h` owned (closed on drop).
fn owned(h: windows::Win32::Foundation::HANDLE) -> std::os::windows::io::OwnedHandle {
    use std::os::windows::io::FromRawHandle;
    // SAFETY: `h` is a fresh handle this process owns and closes only through the result.
    unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(h.0) }
}

fn handle(h: &std::os::windows::io::OwnedHandle) -> windows::Win32::Foundation::HANDLE {
    windows::Win32::Foundation::HANDLE(h.as_raw_handle())
}

fn win_err(e: windows::core::Error) -> std::io::Error {
    let code = e.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        std::io::Error::from_raw_os_error((code & 0xFFFF) as i32)
    } else {
        std::io::Error::other(e.to_string())
    }
}

/// `env` as a `CREATE_UNICODE_ENVIRONMENT` block, sorted by name without case.
fn env_block(env: &[(OsString, OsString)]) -> std::io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut pairs: Vec<&(OsString, OsString)> = env.iter().collect();
    pairs.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (k, v) in pairs {
        let (k, v): (Vec<u16>, Vec<u16>) = (k.encode_wide().collect(), v.encode_wide().collect());
        if k.is_empty() || k.contains(&0) || k[1..].contains(&u16::from(b'=')) || v.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "variável de ambiente inválida",
            ));
        }
        block.extend(k);
        block.push(u16::from(b'='));
        block.extend(v);
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

/// Processes of `job` still running.
fn job_active(job: &std::os::windows::io::OwnedHandle) -> std::io::Result<u32> {
    use windows::Win32::System::JobObjects::{
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
        QueryInformationJobObject,
    };
    let mut acct = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    // SAFETY: `acct` is the structure of this class, with its size.
    unsafe {
        QueryInformationJobObject(
            Some(handle(job)),
            JobObjectBasicAccountingInformation,
            (&raw mut acct).cast(),
            std::mem::size_of_val(&acct) as u32,
            None,
        )
    }
    .map_err(win_err)?;
    Ok(acct.ActiveProcesses)
}

/// How `run_in_job` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobEnd {
    /// Exit code of the process started (`u32` bits in an `i32`).
    pub code: i32,
    /// Every process of the job ended; `false` when some still ran at the deadline.
    pub all_ended: bool,
}

/// Runs `"<program>" <args>` from `dir` with exactly `env`, stdin/stdout/stderr on `NUL`, inside
/// a new job object (created suspended, assigned, then resumed, so no child escapes it). Waits
/// for the program itself with no limit, then for the other processes of the job (an NSIS/Inno
/// uninstaller that copies itself to `TEMP` and exits) for at most `linger`; `on_wait` is
/// called once when that second wait starts. What still runs at the deadline is left running
/// (an uninstaller may reopen the app, a browser or an updater). The code is the program's.
pub fn run_in_job(
    program: &Path,
    args: &str,
    dir: &Path,
    env: &[(OsString, OsString)],
    linger: Duration,
    on_wait: &mut dyn FnMut(),
) -> std::io::Result<JobEnd> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0};
    use windows::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
    use windows::Win32::System::Threading::{
        CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, GetExitCodeProcess, INFINITE,
        PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOW, TerminateProcess,
        WaitForSingleObject,
    };
    use windows::core::{HSTRING, PWSTR};
    let shown = program.display().to_string();
    if !program.is_absolute() || shown.contains('"') || args.contains('\0') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "programa ou argumentos inválidos",
        ));
    }
    let block = env_block(env)?;
    // SAFETY: an unnamed job with default security.
    let job = owned(unsafe { CreateJobObjectW(None, &HSTRING::new()) }.map_err(win_err)?);
    let nul = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("NUL")?;
    let nul_h = windows::Win32::Foundation::HANDLE(nul.as_raw_handle());
    // SAFETY: a valid handle owned by `nul`.
    unsafe { SetHandleInformation(nul_h, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }
        .map_err(win_err)?;
    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: nul_h,
        hStdOutput: nul_h,
        hStdError: nul_h,
        ..Default::default()
    };
    let line = if args.is_empty() {
        format!("\"{shown}\"")
    } else {
        format!("\"{shown}\" {args}")
    };
    let mut line: Vec<u16> = std::ffi::OsStr::new(&line)
        .encode_wide()
        .chain([0])
        .collect();
    let mut pi = PROCESS_INFORMATION::default();
    // SAFETY: every pointer outlives the call; `line` is writable and NUL-terminated; `block`
    // is a double-NUL-terminated UTF-16 environment.
    unsafe {
        CreateProcessW(
            &HSTRING::from(program.as_os_str()),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
            Some(block.as_ptr().cast()),
            &HSTRING::from(dir.as_os_str()),
            &si,
            &mut pi,
        )
    }
    .map_err(win_err)?;
    drop(nul);
    let process = owned(pi.hProcess);
    let thread = owned(pi.hThread);
    let kill = |e: std::io::Error| {
        // SAFETY: valid handle; the suspended process never ran.
        let _ = unsafe { TerminateProcess(handle(&process), 1) };
        e
    };
    // SAFETY: valid job and process handles.
    unsafe { AssignProcessToJobObject(handle(&job), handle(&process)) }
        .map_err(|e| kill(win_err(e)))?;
    // SAFETY: valid thread handle.
    if unsafe { ResumeThread(handle(&thread)) } == u32::MAX {
        return Err(kill(std::io::Error::last_os_error()));
    }
    drop(thread);
    // SAFETY: valid process handle.
    if unsafe { WaitForSingleObject(handle(&process), INFINITE) } != WAIT_OBJECT_0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut code = 0u32;
    // SAFETY: valid handle and a u32 output.
    unsafe { GetExitCodeProcess(handle(&process), &mut code) }.map_err(win_err)?;
    let end = |all_ended| JobEnd {
        code: code as i32,
        all_ended,
    };
    if job_active(&job)? == 0 {
        return Ok(end(true));
    }
    on_wait();
    let start = Instant::now();
    loop {
        if job_active(&job)? == 0 {
            return Ok(end(true));
        }
        if start.elapsed() >= linger {
            return Ok(end(false));
        }
        std::thread::sleep(JOB_POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_ignores_finished_readers() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("out.txt");
        std::fs::write(&file, b"a\nb").unwrap();
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let open = || std::fs::File::open(&file).unwrap();
        let readers = Readers {
            out: Some(spawn_reader(
                open(),
                tx.clone(),
                Arc::clone(&stop),
                utf8_lossy,
            )),
            err: Some(spawn_reader(open(), tx, Arc::clone(&stop), utf8_lossy)),
            stop,
        };
        assert_eq!(rx.iter().count(), 2, "both readers sent `a` and ended");
        while [&readers.out, &readers.err]
            .into_iter()
            .flatten()
            .any(|r| !r.thread.is_finished())
        {
            std::thread::yield_now();
        }
        // Finished readers are never cancelled, so `stop` neither cancels nor waits a retry
        // (counted, not timed: a 50 ms bound failed under load).
        static CANCELS_AFTER_END: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        fn count(_: usize) {
            CANCELS_AFTER_END.fetch_add(1, Ordering::SeqCst);
        }
        readers.stop_with(count);
        assert_eq!(CANCELS_AFTER_END.load(Ordering::SeqCst), 0);
    }

    struct Failing(i32);
    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from_raw_os_error(self.0))
        }
    }

    #[test]
    fn read_loop_995_without_flag_is_error() {
        let (tx, _rx) = channel();
        let stop = AtomicBool::new(false);
        let err = read_loop(&mut Failing(OPERATION_ABORTED), &stop, &tx, utf8_lossy).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(OPERATION_ABORTED));
        // A flag set during the read turns the same error into the end.
        struct SetsFlag<'a>(&'a AtomicBool);
        impl Read for SetsFlag<'_> {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                self.0.store(true, Ordering::SeqCst);
                Err(std::io::Error::from_raw_os_error(OPERATION_ABORTED))
            }
        }
        let stop = AtomicBool::new(false);
        assert!(
            read_loop(&mut SetsFlag(&stop), &stop, &tx, utf8_lossy)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn read_loop_checks_flag_before_read() {
        struct Panics;
        impl Read for Panics {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("read called with the flag set");
            }
        }
        let (tx, _rx) = channel();
        let stop = AtomicBool::new(true);
        assert!(
            read_loop(&mut Panics, &stop, &tx, utf8_lossy)
                .unwrap()
                .is_none()
        );
    }

    static CANCELS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static RELEASE: Mutex<Option<Sender<()>>> = Mutex::new(None);

    fn cancel_on_second_call(_: usize) {
        if CANCELS.fetch_add(1, Ordering::SeqCst) + 1 >= 2
            && let Some(tx) = RELEASE
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
        {
            let _ = tx.send(());
        }
    }

    #[test]
    fn stop_retries_until_readers_finish() {
        let (release, held) = channel::<()>();
        *RELEASE.lock().unwrap_or_else(PoisonError::into_inner) = Some(release.clone());
        let thread = std::thread::spawn(move || {
            let _ = held.recv();
            Ok(None)
        });
        let readers = Readers {
            out: Some(Reader {
                thread,
                _pipe: Arc::new(()),
                raw: 0,
            }),
            err: None,
            stop: Arc::new(AtomicBool::new(false)),
        };
        readers.stop_with(cancel_on_second_call);
        // Let the thread end even if the asserts fail: `Drop` would otherwise wait for it.
        let _ = release.send(());
        let calls = CANCELS.load(Ordering::SeqCst);
        assert!(calls >= 2, "cancel called {calls} times");
        assert!(readers.stop.load(Ordering::SeqCst));
    }
}
