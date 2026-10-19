//! The only module of `cli/` that writes to stdout or stderr (`scripts/check_rules.ps1` refuses
//! `print!`/`println!`/`eprint!`/`eprintln!`/`dbg!` and any `stdout()`/`stderr()` anywhere else
//! in `src/cli/`, the selector's terminal in `picker.rs` aside). The text always comes ready
//! from `output.rs`: nothing here builds a message.
use std::io::{IsTerminal, Write};

/// Whether stdout is a terminal (color and the glyphs of stdout lines).
pub fn stdout_is_terminal() -> bool {
    std::io::stdout().is_terminal()
}

/// Whether stderr is a terminal (the animated progress line, the glyphs of stderr lines).
pub fn stderr_is_terminal() -> bool {
    std::io::stderr().is_terminal()
}

/// One line of the command's result on stdout.
pub fn out(text: &str) {
    anstream::println!("{text}");
}

/// A block of the command's result on stdout; `text` carries its own line ends.
pub fn out_block(text: &str) {
    anstream::print!("{text}");
}

/// One line on stdout, flushed at once (it shares the terminal with the progress line).
pub fn out_flushed(text: &str) {
    anstream::println!("{text}");
    let _ = std::io::stdout().flush();
}

/// One line on stderr whose text is already built (plain progress off a terminal, notices).
pub fn err(text: &str) {
    anstream::eprintln!("{text}");
}

/// One line on stderr built by `text` with the glyphs of stderr (`⚠ aviso: …`).
pub fn err_with(text: impl FnOnce() -> String) {
    let line = super::style::on_stderr(text);
    anstream::eprintln!("{line}");
}

/// `value` as pretty JSON on stdout, never colored.
pub fn json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    writeln!(std::io::stdout().lock(), "{text}")?;
    Ok(())
}

/// The failed command's error on stderr (`main`): see [`super::output::error_report`].
pub fn error(e: &anyhow::Error, verbose: bool) {
    let text = super::style::on_stderr(|| super::output::error_report(e, verbose));
    anstream::eprint!("{text}");
}

/// The progress line's hold on stderr: the redraw and the erase must not interleave with
/// another writer. Only the progress line (`install_cmd`) takes it; it offers no `Write`, so
/// nothing else can go through it. Progress is cosmetic: a broken stderr must not fail the
/// install, so write errors are dropped.
pub(super) struct Progress(std::io::StderrLock<'static>);

/// Locks stderr for the progress line.
pub(super) fn progress() -> Progress {
    Progress(std::io::stderr().lock())
}

impl Progress {
    /// Erases `width` columns of the current line and returns the cursor to its start.
    pub(super) fn erase(&mut self, width: usize) {
        if width > 0 {
            let _ = write!(self.0, "\r{:<width$}\r", "");
            let _ = self.0.flush();
        }
    }

    /// Erases `width` columns, then prints `text` as a line of its own above the progress line.
    pub(super) fn line_above(&mut self, width: usize, text: &str) {
        let _ = writeln!(self.0, "\r{:<width$}\r{text}", "");
    }

    /// Redraws the progress line: `line` then `pad` spaces over what was there.
    pub(super) fn redraw(&mut self, line: &str, pad: usize) {
        let _ = write!(self.0, "\r{line}{:<pad$}", "");
        let _ = self.0.flush();
    }

    /// Flushes (end of a draw with nothing to redraw).
    pub(super) fn flush(&mut self) {
        let _ = self.0.flush();
    }
}
