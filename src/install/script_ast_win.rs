//! The PowerShell process that parses manifest scripts for the gate (Task 227).
//!
//! One process per wingetrs run, started at the first script and reused: it reads requests on
//! stdin and answers with the facts of `script_ast` (the host is `ast_facts.ps1`, passed by
//! `-EncodedCommand`; the script under analysis only travels through the pipe, never to disk).
//! The same program as the `PwshHost`, never elevated, never under a group policy that blocks
//! scripts, with an environment built from scratch, inside a job object that kills it (and
//! anything it started) when the job handle closes. Answers are cached by the script's SHA-256.
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};
use windows::Win32::Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation};
use windows::Win32::UI::Shell::{FOLDERID_System, FOLDERID_Windows};

use crate::install::script_ast::{AstError, AstFacts, AstGate, HOST_SCRIPT, decode_lines};
use crate::install::script_host_win::{child_path, known, read_gpo};
use crate::install::script_policy::{GpoValues, gpo_blocks};
use crate::winget_install::launch_win::is_elevated;

/// Deadline of a process's first answer (its start included).
pub const FIRST_ANSWER: Duration = Duration::from_secs(20);
/// Deadline of each later answer.
pub const EACH_ANSWER: Duration = Duration::from_secs(10);
/// Cap of one answer; more is a broken answer.
const MAX_ANSWER: usize = 1024 * 1024;
/// How long a closed process gets to exit on its own before its job is closed.
const CLOSE_GRACE: Duration = Duration::from_secs(2);
const CHUNK: usize = 8 * 1024;

/// The fixed arguments of the parser process: the host as `-EncodedCommand` (UTF-16LE in
/// base64), nothing from the script or the user.
pub fn host_args() -> Vec<String> {
    let utf16: Vec<u8> = HOST_SCRIPT
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
    ]
    .into_iter()
    .map(str::to_string)
    .chain([STANDARD.encode(utf16)])
    .collect()
}

type Cache = HashMap<[u8; 32], Result<AstFacts, AstError>>;

/// PowerShell's parser as the gate's [`AstGate`].
pub struct PwshAst {
    program: PathBuf,
    elevated: fn() -> io::Result<bool>,
    gpo: fn(&Path) -> GpoValues,
    first: Duration,
    each: Duration,
    cache: Mutex<Cache>,
    proc: Mutex<Option<Proc>>,
    next_id: AtomicU64,
}

impl PwshAst {
    /// The parser in `program`: in production the `PwshHost`'s program, with the real
    /// elevation and group policy.
    pub fn for_program(program: PathBuf) -> PwshAst {
        PwshAst::with_checks(program, is_elevated, read_gpo, FIRST_ANSWER, EACH_ANSWER)
    }

    /// Test seam: the elevation, the group policy and the deadlines.
    pub fn with_checks(
        program: PathBuf,
        elevated: fn() -> io::Result<bool>,
        gpo: fn(&Path) -> GpoValues,
        first: Duration,
        each: Duration,
    ) -> PwshAst {
        PwshAst {
            program,
            elevated,
            gpo,
            first,
            each,
            cache: Mutex::new(HashMap::new()),
            proc: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    fn analyse(&self, script: &str) -> Result<AstFacts, AstError> {
        if !self.program.is_absolute() || !self.program.is_file() {
            return Err(AstError::NoPowerShell);
        }
        // An error reading the token counts as elevated: the parser never starts then.
        if !matches!((self.elevated)(), Ok(false)) {
            return Err(AstError::Elevated);
        }
        if let Some(m) = gpo_blocks(&(self.gpo)(&self.program)) {
            return Err(AstError::Policy(m));
        }
        let mut slot = self.proc.lock().unwrap_or_else(PoisonError::into_inner);
        let proc = match slot.as_mut() {
            Some(p) => p,
            None => {
                let started = Proc::start(&self.program).map_err(|e| {
                    if e.kind() == io::ErrorKind::NotFound {
                        AstError::NoPowerShell
                    } else {
                        AstError::Protocol(format!("o PowerShell não iniciou: {e}"))
                    }
                })?;
                slot.insert(started)
            }
        };
        let deadline = if proc.answered { self.each } else { self.first };
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let answer = proc.ask(id, script, deadline);
        match &answer {
            Ok(_) => proc.answered = true,
            Err(e) => {
                tracing::warn!(error = ?e, "PowerShell parser process dropped");
                // The next request starts a new process.
                if let Some(mut p) = slot.take() {
                    p.kill();
                }
            }
        }
        answer
    }
}

impl AstGate for PwshAst {
    fn facts(&self, script: &str) -> Result<AstFacts, AstError> {
        let key: [u8; 32] = Sha256::digest(script.as_bytes()).into();
        let cached = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        if let Some(answer) = cached {
            return answer;
        }
        let answer = self.analyse(script);
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, answer.clone());
        answer
    }
}

impl Drop for PwshAst {
    fn drop(&mut self) {
        let slot = self.proc.get_mut().unwrap_or_else(PoisonError::into_inner);
        if let Some(mut p) = slot.take() {
            p.close();
        }
    }
}

/// One line of the parser's stdout, or why reading stopped.
type Line = Result<String, String>;

/// A running parser process.
struct Proc {
    /// Closing it kills every process of the job (`KILL_ON_JOB_CLOSE`).
    job: Option<OwnedHandle>,
    process: OwnedHandle,
    stdin: Option<File>,
    lines: Receiver<Line>,
    /// Joined on every path (`kill`, also from `Drop`).
    reader: Option<JoinHandle<()>>,
    answered: bool,
}

impl Proc {
    fn start(program: &Path) -> io::Result<Proc> {
        let windows = known(&FOLDERID_Windows)?;
        let system = known(&FOLDERID_System)?;
        let env: Vec<(OsString, OsString)> = vec![
            ("SystemRoot".into(), windows.into_os_string()),
            ("PATH".into(), child_path(program)?),
            ("POWERSHELL_TELEMETRY_OPTOUT".into(), "1".into()),
            ("POWERSHELL_UPDATECHECK".into(), "Off".into()),
        ];
        let spawned = spawn_in_job(program, &host_args(), &system, &env)?;
        let (tx, rx) = channel();
        let stdout = spawned.stdout;
        let reader = std::thread::spawn(move || read_lines(stdout, &tx));
        Ok(Proc {
            job: Some(spawned.job),
            process: spawned.process,
            stdin: Some(spawned.stdin),
            lines: rx,
            reader: Some(reader),
            answered: false,
        })
    }

    /// Sends one request and reads its answer within `deadline`.
    fn ask(&mut self, id: u64, script: &str, deadline: Duration) -> Result<AstFacts, AstError> {
        let request = format!("{id}\t{}\n", STANDARD.encode(script.as_bytes()));
        let Some(mut stdin) = self.stdin.take() else {
            return Err(AstError::Protocol("stdin do PowerShell fechado".into()));
        };
        let until = Instant::now() + deadline;
        // The write runs apart: a process that stops reading cannot hold the caller past the
        // deadline (closing the job breaks the pipe and ends the write).
        let (answer, stdin) = std::thread::scope(|s| {
            let writer = s.spawn(move || {
                let r = stdin
                    .write_all(request.as_bytes())
                    .and_then(|()| stdin.flush());
                (r, stdin)
            });
            let answer = read_answer(&self.lines, id, until, deadline);
            if answer.is_err() {
                self.kill_job();
            }
            let (written, stdin) = match writer.join() {
                Ok(w) => w,
                Err(p) => std::panic::resume_unwind(p),
            };
            let answer = match (answer, written) {
                (Ok(_), Err(e)) => Err(AstError::Protocol(format!("pedido não enviado: {e}"))),
                (a, _) => a,
            };
            (answer, stdin)
        });
        self.stdin = Some(stdin);
        answer
    }

    fn kill_job(&mut self) {
        self.stdin = None;
        self.job = None;
    }

    /// Kills the process tree and waits for the reader.
    fn kill(&mut self) {
        self.kill_job();
        if let Some(r) = self.reader.take() {
            if std::thread::panicking() {
                let _ = r.join();
            } else if let Err(p) = r.join() {
                std::panic::resume_unwind(p);
            }
        }
    }

    /// Ends the session: stdin closed (the host leaves its loop), a short wait, then `kill`.
    fn close(&mut self) {
        use windows::Win32::System::Threading::WaitForSingleObject;
        self.stdin = None;
        let ms = u32::try_from(CLOSE_GRACE.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: a valid process handle owned by `self`.
        let _ = unsafe { WaitForSingleObject(HANDLE(self.process.as_raw_handle()), ms) };
        self.kill();
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Lines of one answer up to `END <id>`, then decoded.
fn read_answer(
    lines: &Receiver<Line>,
    id: u64,
    until: Instant,
    deadline: Duration,
) -> Result<AstFacts, AstError> {
    let timeout = || AstError::Timeout(deadline.as_secs().max(1));
    let mut got: Vec<String> = Vec::new();
    let mut size = 0usize;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(timeout());
        }
        let line = match lines.recv_timeout(left) {
            Ok(Ok(line)) => line,
            Ok(Err(e)) => return Err(AstError::Protocol(e)),
            Err(RecvTimeoutError::Timeout) => return Err(timeout()),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(AstError::Protocol(
                    "o PowerShell terminou sem responder".into(),
                ));
            }
        };
        size += line.len() + 1;
        if size > MAX_ANSWER {
            return Err(AstError::Protocol("resposta acima de 1 MiB".into()));
        }
        if let Some(end) = line.strip_prefix("END\t") {
            if end != id.to_string() {
                return Err(AstError::Protocol(format!(
                    "resposta de outro pedido ({end:?}, esperado {id})"
                )));
            }
            return decode_lines(got.iter().map(String::as_str)).map_err(AstError::Protocol);
        }
        got.push(line);
    }
}

/// Splits the stdout into lines (`\n`, an optional `\r` dropped) until EOF. A line that is not
/// UTF-8 or passes the answer cap ends the reading with an error.
fn read_lines(mut out: File, tx: &Sender<Line>) {
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = match out.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(at) = pending.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = pending.drain(..=at).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let sent = match String::from_utf8(line) {
                Ok(l) => tx.send(Ok(l)),
                Err(_) => {
                    let _ = tx.send(Err("linha da resposta não é UTF-8".into()));
                    return;
                }
            };
            if sent.is_err() {
                return;
            }
        }
        if pending.len() > MAX_ANSWER {
            let _ = tx.send(Err("linha da resposta acima de 1 MiB".into()));
            return;
        }
    }
}

struct Spawned {
    job: OwnedHandle,
    process: OwnedHandle,
    stdin: File,
    stdout: File,
}

fn owned(h: HANDLE) -> OwnedHandle {
    // SAFETY: `h` is a fresh handle this process owns and closes only through the result.
    unsafe { OwnedHandle::from_raw_handle(h.0) }
}

fn raw(h: &impl AsRawHandle) -> HANDLE {
    HANDLE(h.as_raw_handle())
}

fn win_err(e: windows::core::Error) -> io::Error {
    let code = e.code().0 as u32;
    if code & 0xFFFF_0000 == 0x8007_0000 {
        io::Error::from_raw_os_error((code & 0xFFFF) as i32)
    } else {
        io::Error::other(e.to_string())
    }
}

/// `env` as a `CREATE_UNICODE_ENVIRONMENT` block, sorted by name without case.
fn env_block(env: &[(OsString, OsString)]) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut pairs: Vec<&(OsString, OsString)> = env.iter().collect();
    pairs.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (k, v) in pairs {
        let (k, v): (Vec<u16>, Vec<u16>) = (k.encode_wide().collect(), v.encode_wide().collect());
        if k.is_empty() || k.contains(&0) || k[1..].contains(&u16::from(b'=')) || v.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "variável de ambiente inválida",
            ));
        }
        block.extend(k);
        block.push(u16::from(b'='));
        block.extend(v);
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

/// An anonymous pipe; both ends not inheritable.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    use windows::Win32::System::Pipes::CreatePipe;
    let (mut r, mut w) = (HANDLE::default(), HANDLE::default());
    // SAFETY: both out-pointers are valid; default (non-inheritable) security.
    unsafe { CreatePipe(&mut r, &mut w, None, 0) }.map_err(win_err)?;
    Ok((owned(r), owned(w)))
}

fn inheritable(h: HANDLE) -> io::Result<()> {
    // SAFETY: a valid handle owned by the caller.
    unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }.map_err(win_err)
}

/// A job whose processes all die when its last handle closes.
fn kill_on_close_job() -> io::Result<OwnedHandle> {
    use windows::Win32::System::JobObjects::{
        CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows::core::HSTRING;
    // SAFETY: an unnamed job with default security.
    let job = owned(unsafe { CreateJobObjectW(None, &HSTRING::new()) }.map_err(win_err)?);
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `limits` is the structure of this class, with its size.
    unsafe {
        SetInformationJobObject(
            raw(&job),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    }
    .map_err(win_err)?;
    Ok(job)
}

/// A `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` naming `handles` (borrowed: the array outlives the
/// list), deleted on drop. The same as `bridge::spawn_win`'s.
struct HandleList<'a> {
    _buf: Vec<usize>,
    list: windows::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST,
    _handles: &'a [HANDLE],
}

impl<'a> HandleList<'a> {
    fn new(handles: &'a [HANDLE]) -> io::Result<HandleList<'a>> {
        use windows::Win32::System::Threading::{
            InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, UpdateProcThreadAttribute,
        };
        let mut size = 0usize;
        // SAFETY: size query (no list); it fails with ERROR_INSUFFICIENT_BUFFER by design.
        let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(buf.as_mut_ptr().cast());
        // SAFETY: `buf` is writable, aligned and at least `size` bytes.
        unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut size) }
            .map_err(win_err)?;
        let this = HandleList {
            _buf: buf,
            list,
            _handles: handles,
        };
        // SAFETY: an initialized list; `handles` outlives it (borrowed by `this`).
        unsafe {
            UpdateProcThreadAttribute(
                this.list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                std::mem::size_of_val(handles),
                None,
                None,
            )
        }
        .map_err(win_err)?;
        Ok(this)
    }
}

impl Drop for HandleList<'_> {
    fn drop(&mut self) {
        // SAFETY: the list was initialized in `new` and is deleted once.
        unsafe { windows::Win32::System::Threading::DeleteProcThreadAttributeList(self.list) };
    }
}

/// Starts `"<program>" <args>` from `dir` with exactly `env`, stdin and stdout on pipes and
/// stderr on `NUL`, created suspended, put in a kill-on-close job, then resumed: nothing it
/// starts escapes the job.
fn spawn_in_job(
    program: &Path,
    args: &[String],
    dir: &Path,
    env: &[(OsString, OsString)],
) -> io::Result<Spawned> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
        EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
        STARTUPINFOEXW, STARTUPINFOW, TerminateProcess,
    };
    use windows::core::{HSTRING, PWSTR};
    let shown = program.display().to_string();
    if !program.is_absolute()
        || shown.contains('"')
        || args.iter().any(|a| a.contains(['"', ' ', '\0']))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "programa ou argumentos inválidos",
        ));
    }
    let block = env_block(env)?;
    let job = kill_on_close_job()?;
    let (in_read, in_write) = pipe()?;
    let (out_read, out_write) = pipe()?;
    let nul = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("NUL")?;
    let child_ends = [raw(&in_read), raw(&out_write), raw(&nul)];
    for h in child_ends {
        inheritable(h)?;
    }
    // Only these three are inherited (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`).
    let attrs = HandleList::new(&child_ends)?;
    let si = STARTUPINFOEXW {
        StartupInfo: STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOEXW>() as u32,
            dwFlags: STARTF_USESTDHANDLES,
            hStdInput: raw(&in_read),
            hStdOutput: raw(&out_write),
            hStdError: raw(&nul),
            ..Default::default()
        },
        lpAttributeList: attrs.list,
    };
    // A batch file (only the tests' fake parser) runs through `cmd.exe /d /c`, as
    // `std::process` does, and without the arguments: `cmd.exe` caps its command line at 8191
    // characters, less than the encoded host.
    let batch = program
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    let (app, line) = if batch {
        let cmd = known(&FOLDERID_System)?.join("cmd.exe");
        let line = format!("\"{}\" /d /c \"\"{shown}\"\"", cmd.display());
        (cmd, line)
    } else {
        (
            program.to_path_buf(),
            format!("\"{shown}\" {}", args.join(" ")),
        )
    };
    let mut line: Vec<u16> = std::ffi::OsStr::new(&line)
        .encode_wide()
        .chain([0])
        .collect();
    let mut pi = PROCESS_INFORMATION::default();
    // SAFETY: every pointer outlives the call; `line` is writable and NUL-terminated; `block`
    // is a double-NUL-terminated UTF-16 environment; `si` is a STARTUPINFOEXW whose attribute
    // list (and the handle array it points to) live in `attrs`/`child_ends` until after it.
    unsafe {
        CreateProcessW(
            &HSTRING::from(app.as_os_str()),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT
                | CREATE_NO_WINDOW,
            Some(block.as_ptr().cast()),
            &HSTRING::from(dir.as_os_str()),
            &si.StartupInfo,
            &mut pi,
        )
    }
    .map_err(win_err)?;
    drop(attrs);
    // The child's ends: closed here so the pipes break when the child (tree) is gone.
    drop((in_read, out_write, nul));
    let process = owned(pi.hProcess);
    let thread = owned(pi.hThread);
    let kill = |e: io::Error| {
        // SAFETY: valid handle; the suspended process never ran.
        let _ = unsafe { TerminateProcess(raw(&process), 1) };
        e
    };
    // SAFETY: valid job and process handles.
    unsafe { AssignProcessToJobObject(raw(&job), raw(&process)) }.map_err(|e| kill(win_err(e)))?;
    // SAFETY: valid thread handle.
    if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
        return Err(kill(io::Error::last_os_error()));
    }
    Ok(Spawned {
        job,
        process,
        stdin: File::from(in_write),
        stdout: File::from(out_read),
    })
}
