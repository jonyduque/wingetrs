//! Local cache storage: data directory, HTTP validator state, blob store and update lock.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("diretório de dados indisponível")]
    NoDataDir,
    #[error("E/S em {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("hash não confere: esperado {expected}, obtido {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("estado HTTP inválido: {0}")]
    Json(String),
}

fn io_err(path: &Path, source: std::io::Error) -> CacheError {
    CacheError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[derive(Debug, Clone)]
pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    pub fn new(root: impl Into<PathBuf>) -> DataDir {
        DataDir { root: root.into() }
    }

    /// `%LOCALAPPDATA%\wingetrs`, honoring the env var first (the known-folder API
    /// ignores it, which would make offline tests touch the real catalog), then the known folder.
    pub fn from_env() -> Result<DataDir, CacheError> {
        Self::from_local_app_data(std::env::var_os("LOCALAPPDATA"))
    }

    /// Pure core of [`DataDir::from_env`]: a non-empty value wins over the known folder.
    pub fn from_local_app_data(var: Option<std::ffi::OsString>) -> Result<DataDir, CacheError> {
        match var {
            Some(base) if !base.is_empty() => Ok(DataDir::new(Path::new(&base).join("wingetrs"))),
            _ => Self::default_location(),
        }
    }

    pub fn default_location() -> Result<DataDir, CacheError> {
        let base = directories::BaseDirs::new().ok_or(CacheError::NoDataDir)?;
        Ok(DataDir::new(base.data_local_dir().join("wingetrs")))
    }

    pub fn install_layout(&self) -> crate::install::layout::Layout {
        crate::install::layout::Layout::new(self.root())
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn catalog_path(&self) -> PathBuf {
        self.root.join("catalog.db")
    }
    pub fn search_index_path(&self) -> PathBuf {
        self.root.join("search.idx")
    }
    pub fn http_state_path(&self) -> PathBuf {
        self.root.join("http.json")
    }
    /// Unix seconds (text) of the last detached-refresh attempt.
    pub fn refresh_attempt_path(&self) -> PathBuf {
        self.root.join("refresh.attempt")
    }
    pub fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs")
    }
    pub fn lock_path(&self) -> PathBuf {
        self.root.join("update.lock")
    }
    pub fn log_dir(&self) -> PathBuf {
        self.root.join("log")
    }
    pub fn config_path(&self) -> PathBuf {
        self.root.join("config.toml")
    }
    /// `<root>\buckets`: per-bucket data kept next to the catalog (`scripts/` folders).
    pub fn buckets_dir(&self) -> PathBuf {
        self.root.join("buckets")
    }
    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    pub fn ensure(&self) -> Result<(), CacheError> {
        for d in [
            self.root.clone(),
            self.blobs_dir(),
            self.log_dir(),
            self.tmp_dir(),
        ] {
            std::fs::create_dir_all(&d).map_err(|e| io_err(&d, e))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HttpEntry {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub source_version: Option<String>,
    /// unix seconds of the last successful check (200 or 304)
    pub checked_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HttpState {
    pub entries: BTreeMap<String, HttpEntry>,
}

impl HttpState {
    /// Missing or corrupt file yields an empty state (corrupt logs a warning).
    pub fn load(path: &Path) -> HttpState {
        let Ok(bytes) = std::fs::read(path) else {
            return HttpState::default();
        };
        match serde_json::from_slice(&bytes) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "http state corrupt; starting empty");
                HttpState::default()
            }
        }
    }

    /// Atomic write: `<path>.tmp` then rename.
    pub fn save(&self, path: &Path) -> Result<(), CacheError> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| CacheError::Json(e.to_string()))?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let mut f = std::fs::File::create(&tmp).map_err(|e| io_err(&tmp, e))?;
        f.write_all(&json).map_err(|e| io_err(&tmp, e))?;
        f.sync_all().map_err(|e| io_err(&tmp, e))?;
        drop(f);
        crate::fs_retry::retry("renomear", &tmp, || std::fs::rename(&tmp, path))
            .map_err(|e| io_err(path, e))
    }

    pub fn get(&self, url: &str) -> Option<&HttpEntry> {
        self.entries.get(url)
    }

    pub fn set(&mut self, url: &str, entry: HttpEntry) {
        self.entries.insert(url.to_owned(), entry);
    }
}

/// Content blobs unused for this long are removed by `BlobStore::prune`.
pub const BLOB_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);
const TMP_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PruneStats {
    pub removed: usize,
    pub bytes: u64,
    pub kept: usize,
}

static BLOB_TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn is_blob_name(n: &str) -> bool {
    n.len() == 64 && n.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub struct BlobStore {
    dir: PathBuf,
}

impl BlobStore {
    pub fn new(dir: PathBuf) -> BlobStore {
        BlobStore { dir }
    }

    /// None if missing or the content hash differs from the name.
    pub fn get(&self, sha256_hex_name: &str) -> Option<Vec<u8>> {
        let path = self.dir.join(sha256_hex_name);
        let bytes = std::fs::read(&path).ok()?;
        if !sha256_hex(&bytes).eq_ignore_ascii_case(sha256_hex_name) {
            return None;
        }
        // A hit counts as use: `prune` goes by mtime. A failed touch changes nothing else.
        let touched = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|f| f.set_modified(SystemTime::now()));
        if let Err(e) = touched {
            tracing::debug!(path = %path.display(), error = %e, "blob mtime not refreshed");
        }
        Some(bytes)
    }

    /// Removes content blobs unused for `max_age` and leftover temp files older than a day.
    pub fn prune(&self, now: SystemTime, max_age: Duration) -> Result<PruneStats, CacheError> {
        let rd = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(PruneStats::default());
            }
            Err(e) => return Err(io_err(&self.dir, e)),
        };
        let stale = |meta: &std::fs::Metadata, limit: Duration| {
            // A modification time in the future (fast clock) errors out: keep.
            meta.modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age > limit)
        };
        let mut stats = PruneStats::default();
        for entry in rd {
            let entry = entry.map_err(|e| io_err(&self.dir, e))?;
            let is_file = entry.file_type().is_ok_and(|t| t.is_file());
            let name = entry.file_name();
            let limit = name.to_str().and_then(|n| {
                if is_blob_name(n) {
                    Some(max_age)
                } else if n.ends_with(".tmp") {
                    Some(TMP_MAX_AGE)
                } else {
                    None
                }
            });
            let (true, Some(limit)) = (is_file, limit) else {
                stats.kept += 1;
                continue;
            };
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                stats.kept += 1;
                continue;
            };
            if !stale(&meta, limit) {
                stats.kept += 1;
                continue;
            }
            // Re-read right before deleting: someone may have used the file since the listing.
            let fresh = std::fs::symlink_metadata(&path);
            if fresh.as_ref().map(|m| !stale(m, limit)).unwrap_or(true) {
                stats.kept += 1;
                continue;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    stats.removed += 1;
                    stats.bytes += meta.len();
                }
                Err(e) => {
                    tracing::debug!(path = %path.display(), error = %e, "blob not removed");
                    stats.kept += 1;
                }
            }
        }
        Ok(stats)
    }

    pub fn put_verified(&self, expected_sha256_hex: &str, bytes: &[u8]) -> Result<(), CacheError> {
        let actual = sha256_hex(bytes);
        if !actual.eq_ignore_ascii_case(expected_sha256_hex) {
            return Err(CacheError::HashMismatch {
                expected: expected_sha256_hex.to_owned(),
                actual,
            });
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| io_err(&self.dir, e))?;
        let target = self.dir.join(&actual);
        let seq = BLOB_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = self
            .dir
            .join(format!("{actual}.{}-{seq}.tmp", std::process::id()));
        if let Err(e) = std::fs::write(&tmp, bytes) {
            let _ = std::fs::remove_file(&tmp);
            return Err(io_err(&tmp, e));
        }
        crate::fs_retry::retry("renomear", &tmp, || std::fs::rename(&tmp, &target)).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io_err(&target, e)
        })
    }
}

pub struct UpdateLock {
    _file: std::fs::File,
}

impl UpdateLock {
    pub fn try_acquire(path: &Path) -> Result<Option<UpdateLock>, CacheError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .map_err(|e| io_err(path, e))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(UpdateLock { _file: file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(io_err(path, e)),
        }
    }

    pub fn acquire_timeout(
        path: &Path,
        timeout: Duration,
    ) -> Result<Option<UpdateLock>, CacheError> {
        let start = Instant::now();
        loop {
            if let Some(l) = UpdateLock::try_acquire(path)? {
                return Ok(Some(l));
            }
            let left = timeout.saturating_sub(start.elapsed());
            if left.is_zero() {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(100).min(left));
        }
    }
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
