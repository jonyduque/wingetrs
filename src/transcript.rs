//! `WINGETRS_TRANSCRIPT`: a plain-text copy of what the CLI prints and the TUI shows, for the
//! live proof (the user reads one file instead of a screen). Nothing here writes to stdout or
//! stderr: a broken transcript is a `tracing` warning and the program goes on without it.
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

/// The environment variable that names the transcript file. The file keeps everything the
/// CLI shows (URLs of `--dry-run`, text of manifest scripts): do not share it unreviewed.
pub const VAR: &str = "WINGETRS_TRANSCRIPT";

static FILE: Mutex<Option<File>> = Mutex::new(None);

thread_local! {
    /// A file for this thread only (tests), taken before the global one.
    static OVERRIDE: RefCell<Option<File>> = const { RefCell::new(None) };
}

/// Opens `path` (created if missing, appended to otherwise) as the transcript.
pub fn open(path: &Path) -> io::Result<()> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    *FILE.lock().unwrap_or_else(PoisonError::into_inner) = Some(file);
    Ok(())
}

/// Whether a transcript is open (for this thread or the whole process).
pub fn enabled() -> bool {
    OVERRIDE.with(|o| o.borrow().is_some())
        || FILE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
}

/// Appends `text` as plain lines (escape sequences and `\r` dropped), one flushed write per
/// line. A write error closes the transcript.
pub fn line(text: &str) {
    if !enabled() {
        return;
    }
    let plain = strip_ansi(text);
    let plain = plain.strip_suffix('\n').unwrap_or(&plain);
    for l in plain.split('\n') {
        let mut buf = String::with_capacity(l.len() + 1);
        buf.push_str(l);
        buf.push('\n');
        if let Err(e) = write_line(buf.as_bytes()) {
            tracing::warn!("transcript closed after a write error: {e}");
            return;
        }
    }
}

/// Writes to this thread's file or else the global one; on error closes the one used.
fn write_line(bytes: &[u8]) -> io::Result<()> {
    let local = OVERRIDE.with(|o| {
        let mut slot = o.borrow_mut();
        let file = slot.as_mut()?;
        let r = file.write_all(bytes).and_then(|()| file.flush());
        if r.is_err() {
            *slot = None;
        }
        Some(r)
    });
    if let Some(r) = local {
        return r;
    }
    let mut guard = FILE.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(file) = guard.as_mut() else {
        return Ok(());
    };
    let r = file.write_all(bytes).and_then(|()| file.flush());
    if r.is_err() {
        *guard = None;
    }
    r
}

/// `text` without CSI and OSC sequences, other two-byte escapes, and `\r`.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {}
            '\x1b' => match chars.next() {
                Some('[') => {
                    // Parameters and intermediates, then one final byte 0x40..=0x7E.
                    for n in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // Ends at BEL or at ST (`ESC \`).
                    while let Some(n) = chars.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            c => out.push(c),
        }
    }
    out
}

/// Runs `f` with `path` as this thread's transcript (tests); the previous one comes back after.
pub fn with_file<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<File>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = self.0.take();
            OVERRIDE.with(|o| *o.borrow_mut() = prev);
        }
    }
    let file = OpenOptions::new().create(true).append(true).open(path);
    let prev = OVERRIDE.with(|o| std::mem::replace(&mut *o.borrow_mut(), file.ok()));
    let _restore = Restore(prev);
    f()
}
