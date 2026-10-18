//! Tail of the verbose `msiexec` log (`/l*v`: UTF-16LE with a `FF FE` BOM, one
//! `File: <path>\SourceDir\<relative>;<TAB>To be installed;…` line per copied file). Pure over
//! `Read + Seek`: the caller opens the file read-only (the Windows Installer keeps writing).
use std::io::{self, Read, Seek, SeekFrom};

/// Bytes read from the end of the log on each look.
pub const TAIL_WINDOW: u64 = 8 * 1024;

/// What the end of the log says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogTail {
    /// Log size in bytes.
    pub size: u64,
    /// Path after `SourceDir\` of the last complete `File: …; To be installed` line, or the
    /// whole path when it has no `SourceDir`.
    pub last_file: Option<String>,
    /// `File:` lines written since byte `since` (the size at the previous look).
    pub new_files: u64,
    /// `new_files` was extrapolated: more than the window was written since `since`.
    pub estimated: bool,
}

/// Bytes read from the end of the log of a failed `msiexec` (the error comes before the
/// rollback lines).
pub const ERROR_WINDOW: u64 = 64 * 1024;

/// Added to the error of an `msiexec /a` that failed on a path too long for it.
pub const MAX_PATH_HINT: &str = "o caminho de algum arquivo passou de 260 caracteres (limite do Windows Installer); use uma raiz mais curta";

/// `(size, window start, UTF-16 units)` of the last `window` bytes of `r`; the start is
/// aligned to a unit and never before the BOM nor past the end.
fn read_units<R: Read + Seek>(r: &mut R, window: u64) -> io::Result<(u64, u64, Vec<u16>)> {
    let size = r.seek(SeekFrom::End(0))?;
    let mut start = size.saturating_sub(window);
    // UTF-16 units start at even offsets (the BOM is unit 0).
    start += start % 2;
    // The BOM is not text; an odd size with a zero window would land past the end.
    start = start.max(2).min(size);
    r.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity(usize::try_from(size - start).unwrap_or(0));
    r.take(size - start).read_to_end(&mut bytes)?;
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    Ok((size, start, units))
}

/// The last `window` bytes of the log as text (aligned like [`read_tail`]).
pub fn tail_text<R: Read + Seek>(r: &mut R, window: u64) -> io::Result<String> {
    let (_, _, units) = read_units(r, window)?;
    Ok(String::from_utf16_lossy(&units))
}

/// Explanation added to the error of an `msiexec` that exited with `code`, from the end of
/// its log: only 1603 with `Error 1304` (a file it could not write: a path past MAX_PATH).
pub fn failure_hint(code: i32, tail: &str) -> Option<&'static str> {
    (code == 1603 && tail.contains("Error 1304")).then_some(MAX_PATH_HINT)
}

/// Reads at most `window` bytes from the end of `r` (aligned to a UTF-16 unit, so a window
/// that starts in the middle of a character still decodes) and counts the `File:` lines that
/// start at or after byte `since`.
pub fn read_tail<R: Read + Seek>(r: &mut R, window: u64, since: u64) -> io::Result<LogTail> {
    let (size, start, units) = read_units(r, window)?;
    // (byte offset, text) of each line; the first is cut unless the window starts the file.
    let mut lines = Vec::new();
    let mut line_start = 0usize;
    for (i, u) in units.iter().enumerate() {
        if *u == u16::from(b'\n') {
            lines.push((line_start, &units[line_start..i]));
            line_start = i + 1;
        }
    }
    if line_start < units.len() {
        lines.push((line_start, &units[line_start..]));
    }
    let skip_first = usize::from(start > 2);
    let mut last_file = None;
    let (mut in_window, mut new_files) = (0u64, 0u64);
    let mut first_whole = None;
    for (unit, text) in lines.into_iter().skip(skip_first) {
        let offset = start + 2 * unit as u64;
        first_whole.get_or_insert(offset);
        let Some(file) = file_line(&String::from_utf16_lossy(text)) else {
            continue;
        };
        in_window += 1;
        if offset >= since {
            new_files += 1;
        }
        last_file = Some(file);
    }
    let covered = first_whole.unwrap_or(size);
    let estimated = skip_first == 1 && since < covered && size > covered;
    if estimated {
        // More was written than the window holds: scale the window's density.
        let span = size - covered;
        new_files = in_window.saturating_mul(size - since) / span;
    }
    Ok(LogTail {
        size,
        last_file,
        new_files,
        estimated,
    })
}

/// The relative path of a `File: …;<TAB>To be installed` line.
fn file_line(line: &str) -> Option<String> {
    let at = line.find("File: ")?;
    let rest = &line[at + "File: ".len()..];
    let semi = rest.find(';')?;
    if !rest[semi + 1..].trim_start().starts_with("To be installed") {
        return None;
    }
    let path = &rest[..semi];
    const MARK: &str = "\\sourcedir\\";
    let rel = match path.to_ascii_lowercase().find(MARK) {
        Some(i) => &path[i + MARK.len()..],
        None => path,
    };
    Some(rel.to_string())
}
