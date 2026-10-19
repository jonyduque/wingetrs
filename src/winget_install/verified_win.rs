//! Verified installer file handle (Task 177).
//!
//! The file is opened denying write, delete and rename to everyone else, and the SHA-256 is
//! computed by reading that same handle: what was hashed is what stays on disk while the
//! [`VerifiedFile`] lives. The file itself is opened without following a reparse point (one
//! is refused), and [`hold_chain`] holds the folders above it, so the path that is launched
//! names the held file (Task 196).

use std::fs::File;
use std::io::Read;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, GETFINALPATHNAMEBYHANDLE_FLAGS,
    GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
};

use super::fetch::FetchError;

/// `FILE_ATTRIBUTE_REPARSE_POINT`.
const REPARSE: u32 = 0x400;

#[derive(Debug)]
pub struct VerifiedFile {
    path: PathBuf,
    final_path: PathBuf,
    _file: File,
}

fn reparse_refused(path: &Path) -> FetchError {
    FetchError::Io(format!(
        "{}: ponto de reanálise (junção ou link) recusado",
        path.display()
    ))
}

/// Opens every folder from `root` down to the parent of `file` (`file` is inside `root`),
/// without following a reparse point (one is refused, as `staging_win::hold_plain_folders`
/// does on the elevated side) and without delete sharing: none can be renamed or swapped for a
/// junction while the handles live.
pub fn hold_chain(root: &Path, file: &Path) -> Result<Vec<File>, FetchError> {
    let rel = file
        .strip_prefix(root)
        .map_err(|_| FetchError::Io(format!("{} fora de {}", file.display(), root.display())))?;
    let mut dir = root.to_path_buf();
    let mut dirs = vec![dir.clone()];
    let parts: Vec<_> = rel.components().collect();
    for c in parts.iter().take(parts.len().saturating_sub(1)) {
        dir.push(c.as_os_str());
        dirs.push(dir.clone());
    }
    let mut held = Vec::with_capacity(dirs.len());
    for d in dirs {
        let io = |e: std::io::Error| FetchError::Io(format!("{}: {e}", d.display()));
        let f = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(&d)
            .map_err(io)?;
        let meta = f.metadata().map_err(io)?;
        if meta.file_attributes() & REPARSE != 0 {
            return Err(reparse_refused(&d));
        }
        if !meta.is_dir() {
            return Err(FetchError::Io(format!("{} não é uma pasta", d.display())));
        }
        held.push(f);
    }
    Ok(held)
}

/// The path of the open `file` as the system resolves it (`GetFinalPathNameByHandleW`), in
/// the usual form (`C:\...`, `\\server\...`).
fn final_path_of(file: &File) -> std::io::Result<PathBuf> {
    let mut buf = vec![0u16; 1024];
    loop {
        // SAFETY: a valid handle owned by `file`; `buf` is writable for its length.
        let n = unsafe {
            GetFinalPathNameByHandleW(
                HANDLE(file.as_raw_handle()),
                &mut buf,
                GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0),
            )
        } as usize;
        if n == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n < buf.len() {
            let text = String::from_utf16(&buf[..n])
                .map_err(|_| std::io::Error::other("caminho final não é UTF-16 válido"))?;
            let text = match text.strip_prefix(r"\\?\UNC\") {
                Some(rest) => format!(r"\\{rest}"),
                None => text.strip_prefix(r"\\?\").unwrap_or(&text).to_string(),
            };
            return Ok(PathBuf::from(text));
        }
        // Too small: `n` is the size needed, with the terminator. Bounded by the long path
        // limit.
        if n > 32_768 {
            return Err(std::io::Error::other("caminho final longo demais"));
        }
        buf = vec![0u16; n];
    }
}

impl VerifiedFile {
    /// Opens `path` sharing only reads and checks its SHA-256 (hex, any case) through the handle.
    pub fn open(path: &Path, sha256: &str) -> Result<VerifiedFile, FetchError> {
        let io = |e: std::io::Error| FetchError::Io(format!("{}: {e}", path.display()));
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)
            .map_err(io)?;
        if file.metadata().map_err(io)?.file_attributes() & REPARSE != 0 {
            return Err(reparse_refused(path));
        }
        let mut sha = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(io(e)),
            };
            sha.update(&buf[..n]);
        }
        let actual = sha
            .finalize()
            .iter()
            .fold(String::with_capacity(64), |mut s, b| {
                use std::fmt::Write;
                let _ = write!(s, "{b:02x}");
                s
            });
        if !actual.eq_ignore_ascii_case(sha256.trim()) {
            return Err(FetchError::Integrity);
        }
        let final_path = final_path_of(&file).map_err(io)?;
        Ok(VerifiedFile {
            path: path.to_path_buf(),
            final_path,
            _file: file,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The held file's path as the system resolves it: what an elevated launch is given.
    pub fn final_path(&self) -> &Path {
        &self.final_path
    }
}
