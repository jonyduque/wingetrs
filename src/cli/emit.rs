//! The only module of `cli/` that writes to stdout or stderr (`scripts/check_rules.ps1` refuses
//! `print!`/`println!`/`eprint!`/`eprintln!` and a locked stdout/stderr anywhere else in
//! `src/cli/`). The text always comes ready from `output.rs`: nothing here builds a message.
use std::io::Write;

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

/// One line on stderr (warnings, plain progress off a terminal).
pub fn err(text: &str) {
    anstream::eprintln!("{text}");
}

/// `value` as pretty JSON on stdout, never colored.
pub fn json<T: serde::Serialize>(value: &T) -> anyhow::Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    writeln!(std::io::stdout().lock(), "{text}")?;
    Ok(())
}

/// The failed command's error on stderr (`main`): see [`super::output::error_report`].
pub fn error(e: &anyhow::Error, verbose: bool) {
    anstream::eprint!("{}", super::output::error_report(e, verbose));
}

/// Stderr, locked, for the progress line: the redraw and the erase must not interleave.
pub fn stderr() -> std::io::StderrLock<'static> {
    std::io::stderr().lock()
}

// Progress line (stderr). Progress is cosmetic: a broken stderr must not fail the install, so
// write errors are dropped.

/// Erases `width` columns of the current line and returns the cursor to its start.
pub fn erase(w: &mut impl Write, width: usize) {
    if width > 0 {
        let _ = write!(w, "\r{:<width$}\r", "");
        let _ = w.flush();
    }
}

/// Erases `width` columns, then prints `text` as a line of its own above the progress line.
pub fn line_above(w: &mut impl Write, width: usize, text: &str) {
    let _ = writeln!(w, "\r{:<width$}\r{text}", "");
}

/// Redraws the progress line: `line` then `pad` spaces over what was there.
pub fn redraw(w: &mut impl Write, line: &str, pad: usize) {
    let _ = write!(w, "\r{line}{:<pad$}", "");
    let _ = w.flush();
}

/// Flushes `w` (end of a draw with nothing to redraw).
pub fn flush(w: &mut impl Write) {
    let _ = w.flush();
}
