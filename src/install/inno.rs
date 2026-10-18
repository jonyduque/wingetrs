//! innounp extraction (S-inno) (Task 147): arguments, output decoding and exit-code reading.
//! The process itself runs in `inno_win`.
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

use crate::install::extract::ExtractError;
use crate::install::safepath;

/// innounp arguments: extract everything under `{app}` (or under the `extract_dir` given)
/// into `dest`, flattening the prefix, answering yes to overwrites.
// Ported from scoop lib/decompress.ps1 (Expand-InnoArchive).
pub fn inno_args(
    installer: &Path,
    dest: &Path,
    extract_dir: Option<&str>,
) -> Result<Vec<String>, ExtractError> {
    let filter = match extract_dir {
        None => "-c{app}".to_string(),
        Some(raw) => format!("-c{}", inno_filter(raw)?),
    };
    Ok(vec![
        "-x".into(),
        format!("-d{}", dest.display()),
        installer.display().to_string(),
        "-y".into(),
        filter,
    ])
}

/// `extract_dir` as an innounp path: `{name}\rest` as is, anything else under `{app}`.
/// The rest goes through `safepath`; a `{name}` is accepted only as the first component.
fn inno_filter(raw: &str) -> Result<String, ExtractError> {
    let unsafe_path = || ExtractError::UnsafePath(raw.to_string());
    let dir = raw.replace('/', "\\");
    let (root, rest) = if dir.starts_with('{') {
        let (first, rest) = dir.split_once('\\').unwrap_or((dir.as_str(), ""));
        let name = first
            .strip_prefix('{')
            .and_then(|f| f.strip_suffix('}'))
            .ok_or_else(unsafe_path)?;
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(unsafe_path());
        }
        (first.to_string(), rest.to_string())
    } else {
        ("{app}".to_string(), dir)
    };
    let comps = safepath::safe_components(&rest).ok_or_else(unsafe_path)?;
    if comps.iter().any(|c| c.contains(['{', '}'])) {
        return Err(unsafe_path());
    }
    Ok(std::iter::once(root)
        .chain(comps)
        .collect::<Vec<_>>()
        .join("\\"))
}

/// Code page 850 (OEM Latin-1), bytes `0x80..=0xFF`: innounp's console output.
const CP850_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', // 0x80
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', 'ø', '£', 'Ø', '×', 'ƒ', // 0x90
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '®', '¬', '½', '¼', '¡', '«', '»', // 0xA0
    '░', '▒', '▓', '│', '┤', 'Á', 'Â', 'À', '©', '╣', '║', '╗', '╝', '¢', '¥', '┐', // 0xB0
    '└', '┴', '┬', '├', '─', '┼', 'ã', 'Ã', '╚', '╔', '╩', '╦', '╠', '═', '╬', '¤', // 0xC0
    'ð', 'Ð', 'Ê', 'Ë', 'È', 'ı', 'Í', 'Î', 'Ï', '┘', '┌', '█', '▄', '¦', 'Ì', '▀', // 0xD0
    'Ó', 'ß', 'Ô', 'Ò', 'õ', 'Õ', 'µ', 'þ', 'Þ', 'Ú', 'Û', 'Ù', 'ý', 'Ý', '¯', '´', // 0xE0
    // 0xF0: starts with a soft hyphen, ends with a no-break space.
    '\u{AD}', '±', '‗', '¾', '¶', '§', '÷', '¸', '°', '¨', '·', '¹', '³', '²', '■', '\u{A0}',
];

/// Decodes CP850 bytes (ASCII unchanged).
pub fn decode_cp850(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0..=0x7F => char::from(b),
            _ => CP850_HIGH[usize::from(b - 0x80)],
        })
        .collect()
}

/// Regular files under `dir`, recursively; links and junctions are neither followed nor counted.
pub fn count_files(dir: &Path) -> io::Result<u64> {
    let mut n = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            n += count_files(&entry.path())?;
        } else if ty.is_file() {
            n += 1;
        }
    }
    Ok(n)
}

/// Reads innounp's exit: code 0 with nothing extracted means the `-c` filter matched nothing.
pub fn check_exit(
    code: i32,
    files: u64,
    extract_dir: Option<&str>,
    lines: &[String],
) -> Result<(), ExtractError> {
    let last = lines
        .iter()
        .rev()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    match code {
        0 => {
            if files == 0 {
                return Err(ExtractError::MissingExtractDir(
                    extract_dir.unwrap_or("{app}").to_string(),
                ));
            }
            Ok(())
        }
        2 => Err(ExtractError::Archive(
            "o innounp não reconhece o arquivo como instalador Inno Setup".into(),
        )),
        3 => Err(ExtractError::Io(format!(
            "o innounp não conseguiu abrir o instalador: {last}"
        ))),
        c => Err(ExtractError::Archive(format!(
            "innounp saiu com código {c}: {last}"
        ))),
    }
}

/// The child's whole environment change: a fixed `PATH` (System32 and innounp's own folder),
/// so it never finds tools through the user's `PATH`.
pub fn child_env(system32: &Path, exe: &Path) -> Vec<(OsString, OsString)> {
    let mut path = system32.as_os_str().to_os_string();
    if let Some(dir) = exe.parent().filter(|d| !d.as_os_str().is_empty()) {
        path.push(";");
        path.push(dir.as_os_str());
    }
    vec![(OsString::from("PATH"), path)]
}
