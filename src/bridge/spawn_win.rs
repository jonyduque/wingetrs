//! Runs winget or scoop with no window and stdin closed, reading stdout and stderr line by line.
use super::Tail;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CHUNK: usize = 8 * 1024;

/// A pipe reader; it ends with the pipe's unterminated last line.
type Reader = JoinHandle<std::io::Result<Option<String>>>;

/// The stdout and stderr readers. Joined on every path: explicitly in `run`, or by `Drop`
/// (an error or a panic of `on_line`), so they never outlive the call.
struct Readers {
    out: Option<Reader>,
    err: Option<Reader>,
}

/// Waits for a reader; a reader panic is propagated.
fn join(slot: &mut Option<Reader>) -> std::io::Result<Option<String>> {
    match slot.take().map(JoinHandle::join) {
        None => Ok(None),
        Some(Ok(rest)) => rest,
        Some(Err(p)) => std::panic::resume_unwind(p),
    }
}

impl Drop for Readers {
    fn drop(&mut self) {
        // While already unwinding, a second panic would abort: just reap the threads.
        if std::thread::panicking() {
            for h in [self.out.take(), self.err.take()].into_iter().flatten() {
                let _ = h.join();
            }
        } else {
            let _ = join(&mut self.out);
            let _ = join(&mut self.err);
        }
    }
}

fn spawn_reader(mut pipe: impl Read + Send + 'static, tx: Sender<String>) -> Reader {
    std::thread::spawn(move || {
        let mut tail = Tail::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = match pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            for line in tail.push(&buf[..n]) {
                // No receiver means the caller is unwinding: keep draining so the tool can exit.
                let _ = tx.send(line);
            }
        }
        Ok(tail.finish())
    })
}

/// Runs `program` and waits for it with no time limit; `on_line` gets every useful output
/// line (stdout and stderr, in arrival order). Returns the exit code and the last lines.
/// A missing program is `ErrorKind::NotFound`.
pub fn run(
    program: &str,
    args: &[String],
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
    let mut readers = Readers {
        out: child.stdout.take().map(|p| spawn_reader(p, tx.clone())),
        err: child.stderr.take().map(|p| spawn_reader(p, tx.clone())),
    };
    drop(tx);
    let mut tail = Tail::new();
    // Ends when both readers are done (each holds a sender).
    for line in rx {
        on_line(&line);
        tail.add(line);
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
    let status = child.wait()?;
    Ok((status.code().unwrap_or(-1), tail.lines()))
}
