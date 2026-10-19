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

/// Prefix of a run folder: `<root>\installers\run-<32 hex>`.
pub const RUN_PREFIX: &str = "run-";
/// Run folders older than this are removed before the next elevated run.
pub const RUN_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// A fresh folder holding only a copy of the verified installer, for one elevated run (Task
/// 196): nothing planted beside the downloaded file (a DLL the installer would load as admin)
/// is next to the copy. The copy is re-hashed through a handle that denies write, delete and
/// rename, the folders from the data root down are held (no reparse point), and the folder is
/// removed when this is dropped.
#[derive(Debug)]
pub struct RunCopy {
    dir: PathBuf,
    file: Option<VerifiedFile>,
    folders: Vec<File>,
}

impl RunCopy {
    /// Copies `source` (already verified and held by the caller) into a new
    /// `<root>\installers\run-<32 random hex>\`; stale run folders are pruned first.
    pub fn create(root: &Path, source: &Path, sha256: &str) -> Result<RunCopy, FetchError> {
        let installers = root.join("installers");
        prune_runs(&installers, std::time::SystemTime::now());
        let hex = crate::bridge::elevate_win::random_hex()
            .map_err(|e| FetchError::Io(format!("gerador aleatório: {e}")))?;
        RunCopy::create_named(root, &format!("{RUN_PREFIX}{hex}"), source, sha256)
    }

    /// `create` with the folder name `name` (one component); refused when the folder exists.
    pub fn create_named(
        root: &Path,
        name: &str,
        source: &Path,
        sha256: &str,
    ) -> Result<RunCopy, FetchError> {
        let file_name = source
            .file_name()
            .ok_or_else(|| FetchError::Io(format!("{}: sem nome", source.display())))?;
        let dir = root.join("installers").join(name);
        // `create_new`: an existing folder (planted or left behind) is never reused.
        std::fs::create_dir(&dir).map_err(|e| FetchError::Io(format!("{}: {e}", dir.display())))?;
        let mut run = RunCopy {
            dir,
            file: None,
            folders: Vec::new(),
        };
        let copy = run.dir.join(file_name);
        run.folders = hold_chain(root, &copy)?;
        let io = |p: &Path, e: std::io::Error| FetchError::Io(format!("{}: {e}", p.display()));
        {
            let mut from = File::open(source).map_err(|e| io(source, e))?;
            let mut to = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&copy)
                .map_err(|e| io(&copy, e))?;
            std::io::copy(&mut from, &mut to).map_err(|e| io(&copy, e))?;
            to.sync_all().map_err(|e| io(&copy, e))?;
        }
        run.file = Some(VerifiedFile::open(&copy, sha256)?);
        Ok(run)
    }

    /// The run folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The held copy's final path: what the elevated launch is given.
    pub fn final_path(&self) -> &Path {
        match &self.file {
            Some(f) => f.final_path(),
            None => &self.dir,
        }
    }
}

impl Drop for RunCopy {
    fn drop(&mut self) {
        drop(self.file.take());
        self.folders.clear();
        if let Err(e) = std::fs::remove_dir_all(&self.dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(error = %e, "pasta de execução não apagada: {}", self.dir.display());
        }
    }
}

/// Removes the run folders in `installers` older than `RUN_STALE_AFTER` at `now` (only plain
/// folders named like one); how many were removed.
pub fn prune_runs(installers: &Path, now: std::time::SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(installers) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_run = name.to_str().is_some_and(|n| {
            n.strip_prefix(RUN_PREFIX).is_some_and(|h| {
                h.len() == 32 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            })
        });
        if !is_run {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || meta.file_attributes() & REPARSE != 0 {
            continue;
        }
        let stale = meta
            .modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > RUN_STALE_AFTER);
        if stale {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => removed += 1,
                Err(e) => tracing::warn!(error = %e, "pasta de execução antiga não apagada"),
            }
        }
    }
    removed
}
