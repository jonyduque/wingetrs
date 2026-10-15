//! Runs winget or scoop with no window and stdin closed, reading stdout and stderr line by line.
use super::Tail;
use std::io::Read;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
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

/// What a reader thread ends with: the pipe's unterminated last line, and the pipe itself, so
/// its handle stays open (and cannot be reused) until the reader is joined.
type ReaderEnd = (std::io::Result<Option<String>>, Box<dyn Send>);

/// A pipe reader and its pipe's raw handle, used only to cancel the read under way.
struct Reader {
    thread: JoinHandle<ReaderEnd>,
    pipe: usize,
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
        Some(Ok((rest, _pipe))) => rest,
        Some(Err(p)) => std::panic::resume_unwind(p),
    }
}

impl Readers {
    /// Makes both readers end at their next read or in the one under way, and waits until they
    /// have. The output still in the pipes is dropped; whoever writes it is left alone.
    /// The cancel targets the pipe, not the thread: std reads child pipes overlapped
    /// (`ReadFileEx` + alertable `SleepEx`), which `CancelSynchronousIo` cannot reach.
    fn stop(&self) {
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
                let pipe = windows::Win32::Foundation::HANDLE(r.pipe as *mut std::ffi::c_void);
                // SAFETY: the reader owns the pipe until it is joined, so the handle is open.
                // `ERROR_NOT_FOUND` (no read under way right now) is expected: retried below.
                let _ = unsafe { windows::Win32::System::IO::CancelIoEx(pipe, None) };
            }
            std::thread::sleep(CANCEL_RETRY);
        }
    }
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

fn spawn_reader(
    mut pipe: impl Read + AsRawHandle + Send + 'static,
    tx: Sender<String>,
    stop: Arc<AtomicBool>,
) -> Reader {
    let raw = pipe.as_raw_handle() as usize;
    let thread = std::thread::spawn(move || {
        let mut tail = Tail::new();
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
                    if e.raw_os_error() == Some(OPERATION_ABORTED)
                        && stop.load(Ordering::SeqCst) =>
                {
                    break;
                }
                Err(e) => return (Err(e), Box::new(pipe) as Box<dyn Send>),
            };
            for line in tail.push(&buf[..n]) {
                // No receiver means the caller is unwinding: keep draining so the tool can exit.
                let _ = tx.send(line);
            }
        }
        (Ok(tail.finish()), Box::new(pipe) as Box<dyn Send>)
    });
    Reader { thread, pipe: raw }
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
    mut on_line: impl FnMut(&str),
) -> std::io::Result<(i32, Vec<String>)> {
    let mut child = Command::new(program)
        .args(args)
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
            .map(|p| spawn_reader(p, tx.clone(), Arc::clone(&stop))),
        err: child
            .stderr
            .take()
            .map(|p| spawn_reader(p, tx.clone(), Arc::clone(&stop))),
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
            out: Some(spawn_reader(open(), tx.clone(), Arc::clone(&stop))),
            err: Some(spawn_reader(open(), tx, Arc::clone(&stop))),
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
        let start = Instant::now();
        readers.stop();
        assert!(start.elapsed() < CANCEL_RETRY, "{:?}", start.elapsed());
    }
}
