//! Verified installer file handle (Task 177).
//!
//! The file is opened denying write, delete and rename to everyone else, and the SHA-256 is
//! computed by reading that same handle: what was hashed is what stays on disk while the
//! [`VerifiedFile`] lives.

use std::fs::File;
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;

use super::fetch::FetchError;

#[derive(Debug)]
pub struct VerifiedFile {
    path: PathBuf,
    _file: File,
}

impl VerifiedFile {
    /// Opens `path` sharing only reads and checks its SHA-256 (hex, any case) through the handle.
    pub fn open(path: &Path, sha256: &str) -> Result<VerifiedFile, FetchError> {
        let io = |e: std::io::Error| FetchError::Io(format!("{}: {e}", path.display()));
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .open(path)
            .map_err(io)?;
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
        Ok(VerifiedFile {
            path: path.to_path_buf(),
            _file: file,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
