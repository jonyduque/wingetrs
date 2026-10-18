//! Downloads with hash verification and a content-addressed blob cache.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

use crate::cache::DataDir;
use crate::fs_retry;
use crate::http::Http;
use crate::install::manifest::{HashAlgo, HashSpec, Source};
use crate::install::progress::ProgressEvent;

fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// True if `bytes` hash to `spec` (hex compared case-insensitively).
pub fn verify(spec: &HashSpec, bytes: &[u8]) -> bool {
    let actual = match spec.algo {
        HashAlgo::Sha256 => hex_of(&Sha256::digest(bytes)),
        HashAlgo::Sha1 => hex_of(&Sha1::digest(bytes)),
        HashAlgo::Sha512 => hex_of(&Sha512::digest(bytes)),
        HashAlgo::Md5 => hex_of(&Md5::digest(bytes)),
    };
    actual.eq_ignore_ascii_case(&spec.hex)
}

/// Streams `path` once and returns (hex of `algo`, hex of sha256); one pass, 64 KiB buffer.
fn hash_file(path: &Path, algo: HashAlgo) -> std::io::Result<(String, String)> {
    use std::io::Read;
    enum Other {
        None,
        Sha1(Sha1),
        Sha512(Sha512),
        Md5(Md5),
    }
    let mut other = match algo {
        HashAlgo::Sha256 => Other::None,
        HashAlgo::Sha1 => Other::Sha1(Sha1::new()),
        HashAlgo::Sha512 => Other::Sha512(Sha512::new()),
        HashAlgo::Md5 => Other::Md5(Md5::new()),
    };
    let mut sha = Sha256::new();
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let chunk = &buf[..n];
        sha.update(chunk);
        match &mut other {
            Other::None => {}
            Other::Sha1(h) => h.update(chunk),
            Other::Sha512(h) => h.update(chunk),
            Other::Md5(h) => h.update(chunk),
        }
    }
    let sha_hex = hex_of(&sha.finalize());
    let algo_hex = match other {
        Other::None => sha_hex.clone(),
        Other::Sha1(h) => hex_of(&h.finalize()),
        Other::Sha512(h) => hex_of(&h.finalize()),
        Other::Md5(h) => hex_of(&h.finalize()),
    };
    Ok((algo_hex, sha_hex))
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error(transparent)]
    Http(#[from] crate::http::HttpError),
    #[error("hash não confere para {url}")]
    HashMismatch { url: String },
    #[error("E/S: {0}")]
    Io(String),
}

/// Receives every [`ProgressEvent`] of an install (download bytes, phases).
pub type Progress<'a> = &'a (dyn Fn(&ProgressEvent) + Send + Sync);

const MAX_PARALLEL: usize = 4;

fn io_err(p: &Path, e: std::io::Error) -> DownloadError {
    DownloadError::Io(format!("{}: {e}", p.display()))
}

/// Cache key of a hash: `<algo>:<lowercase hex>`.
pub(crate) fn algo_key(spec: &HashSpec) -> String {
    let a = match spec.algo {
        HashAlgo::Sha256 => "sha256",
        HashAlgo::Sha1 => "sha1",
        HashAlgo::Sha512 => "sha512",
        HashAlgo::Md5 => "md5",
    };
    format!("{a}:{}", spec.hex.to_ascii_lowercase())
}

fn index_path(data: &DataDir) -> PathBuf {
    data.blobs_dir().join("index.json")
}

fn read_index(data: &DataDir) -> BTreeMap<String, String> {
    // A missing or corrupt index only costs a re-download.
    std::fs::read(index_path(data))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_index(data: &DataDir, idx: &BTreeMap<String, String>) -> Result<(), DownloadError> {
    let p = index_path(data);
    let json = serde_json::to_vec(idx).map_err(|e| DownloadError::Io(e.to_string()))?;
    let tmp = p.with_extension(format!(
        "json.{}-{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, json).map_err(|e| io_err(&tmp, e))?;
    fs_retry::retry("renomear", &tmp, || std::fs::rename(&tmp, &p)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io_err(&p, e)
    })
}

/// Drops index entries whose blob file is gone; rewrites the index only if any went.
pub fn prune_index(data: &DataDir) -> Result<usize, DownloadError> {
    let Ok(bytes) = std::fs::read(index_path(data)) else {
        return Ok(0);
    };
    let Ok(mut idx) = serde_json::from_slice::<BTreeMap<String, String>>(&bytes) else {
        return Ok(0);
    };
    let blobs = data.blobs_dir();
    let before = idx.len();
    idx.retain(|_, name| blobs.join(name).exists());
    let removed = before - idx.len();
    if removed > 0 {
        write_index(data, &idx)?;
    }
    Ok(removed)
}

/// Path of a cached, content-verified blob for `spec`, if any.
fn cached(data: &DataDir, idx: &BTreeMap<String, String>, spec: &HashSpec) -> Option<PathBuf> {
    let name = if spec.algo == HashAlgo::Sha256 {
        spec.hex.to_ascii_lowercase()
    } else {
        idx.get(&algo_key(spec))?.clone()
    };
    let path = data.blobs_dir().join(&name);
    if !path.is_file() {
        return None;
    }
    let (algo_hex, sha_hex) = hash_file(&path, spec.algo).ok()?;
    // The blob name is its sha256; for other algorithms the manifest hash must match too.
    (sha_hex == name && algo_hex.eq_ignore_ascii_case(&spec.hex)).then_some(path)
}

/// `cached` on the blocking pool: hashing a large blob must not stall the runtime workers.
async fn cached_blocking(
    data: &DataDir,
    idx: &BTreeMap<String, String>,
    spec: &HashSpec,
) -> Result<Option<PathBuf>, DownloadError> {
    let job = tokio::task::spawn_blocking({
        let (data, idx, spec) = (data.clone(), idx.clone(), spec.clone());
        move || cached(&data, &idx, &spec)
    });
    match job.await {
        Ok(r) => Ok(r),
        Err(e) => match e.try_into_panic() {
            Ok(p) => std::panic::resume_unwind(p),
            Err(e) => Err(DownloadError::Io(e.to_string())),
        },
    }
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Removes the temp file on drop unless disarmed (covers every error path).
struct TmpGuard(Option<PathBuf>);
impl Drop for TmpGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Returns (blob path, sha256 name) after a verified download.
async fn download_one(
    http: &Http,
    data: &DataDir,
    src: &Source,
    progress: Progress<'_>,
) -> Result<(PathBuf, String), DownloadError> {
    let tmp_dir = data.tmp_dir();
    std::fs::create_dir_all(&tmp_dir).map_err(|e| io_err(&tmp_dir, e))?;
    let tmp = tmp_dir.join(format!(
        "dl-{}-{}.part",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let mut guard = TmpGuard(Some(tmp.clone()));
    http.get_to_file(&src.url, &tmp, |done, total| {
        progress(&ProgressEvent::Download {
            url: src.url.clone(),
            done,
            total,
        })
    })
    .await?;
    let hashing = tokio::task::spawn_blocking({
        let (tmp, algo) = (tmp.clone(), src.hash.algo);
        move || hash_file(&tmp, algo)
    });
    let (algo_hex, name) = match hashing.await {
        Ok(r) => r.map_err(|e| io_err(&tmp, e))?,
        Err(e) => match e.try_into_panic() {
            Ok(p) => std::panic::resume_unwind(p),
            Err(e) => return Err(DownloadError::Io(e.to_string())),
        },
    };
    if !algo_hex.eq_ignore_ascii_case(&src.hash.hex) {
        return Err(DownloadError::HashMismatch {
            url: src.url.clone(),
        }); // guard deletes the temp file
    }
    let blobs = data.blobs_dir();
    std::fs::create_dir_all(&blobs).map_err(|e| io_err(&blobs, e))?;
    let target = blobs.join(&name);
    // std rename replaces an existing (e.g. tampered) blob of the same name.
    // The retry sleeps: keep it off the runtime worker shared with the other downloads.
    let rename = tokio::task::spawn_blocking({
        let (tmp, target) = (tmp.clone(), target.clone());
        move || fs_retry::retry("renomear", &tmp, || std::fs::rename(&tmp, &target))
    });
    match rename.await {
        Ok(r) => r.map_err(|e| io_err(&target, e))?,
        Err(e) => match e.try_into_panic() {
            Ok(p) => std::panic::resume_unwind(p),
            Err(e) => return Err(DownloadError::Io(e.to_string())),
        },
    }
    guard.0 = None;
    Ok((target, name))
}

/// Returns the local path of each source's verified file (in blobs), same order as `sources`;
/// the first failure wins.
pub async fn fetch_all(
    http: &Http,
    data: &DataDir,
    sources: &[Source],
    progress: Progress<'_>,
) -> Result<Vec<PathBuf>, DownloadError> {
    fetch_each(http, data, sources, progress)
        .await
        .into_iter()
        .collect()
}

/// One result per source, same order as `sources`. Cache first; at most `MAX_PARALLEL`
/// downloads at once across all sources; `index.json` written once with the new entries.
pub async fn fetch_each(
    http: &Http,
    data: &DataDir,
    sources: &[Source],
    progress: Progress<'_>,
) -> Vec<Result<PathBuf, DownloadError>> {
    let idx = read_index(data);
    let sem = tokio::sync::Semaphore::new(MAX_PARALLEL);
    let tasks = sources.iter().map(|src| {
        let (sem, idx) = (&sem, &idx);
        async move {
            if let Some(p) = cached_blocking(data, idx, &src.hash).await? {
                return Ok((p, None));
            }
            let _permit = sem
                .acquire()
                .await
                .map_err(|e| DownloadError::Io(e.to_string()))?;
            let (p, name) = download_one(http, data, src, progress).await?;
            Ok::<_, DownloadError>((p, Some((algo_key(&src.hash), name))))
        }
    });
    let results = futures_util::future::join_all(tasks).await;
    let mut new_idx = idx.clone();
    let mut dirty = false;
    let paths = results
        .into_iter()
        .map(|r| {
            r.map(|(p, entry)| {
                if let Some((k, v)) = entry
                    && !k.starts_with("sha256:")
                {
                    new_idx.insert(k, v);
                    dirty = true;
                }
                p
            })
        })
        .collect();
    if dirty {
        // Blobs are already verified; a lost index only costs a re-download.
        if let Err(e) = write_index(data, &new_idx) {
            tracing::warn!("não foi possível gravar blobs/index.json: {e}");
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_file_matches_in_memory_at_block_edges() {
        let dir = tempfile::tempdir().unwrap();
        for len in [0usize, 1, 65_535, 65_536, 65_537, 1_048_579] {
            let data: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
            let p = dir.path().join(format!("f{len}"));
            std::fs::write(&p, &data).unwrap();
            for algo in [
                HashAlgo::Sha256,
                HashAlgo::Sha1,
                HashAlgo::Sha512,
                HashAlgo::Md5,
            ] {
                let (h, s) = hash_file(&p, algo).unwrap();
                let want = match algo {
                    HashAlgo::Sha256 => hex_of(&Sha256::digest(&data)),
                    HashAlgo::Sha1 => hex_of(&Sha1::digest(&data)),
                    HashAlgo::Sha512 => hex_of(&Sha512::digest(&data)),
                    HashAlgo::Md5 => hex_of(&Md5::digest(&data)),
                };
                assert_eq!(h, want, "{algo:?} len {len}");
                assert_eq!(s, hex_of(&Sha256::digest(&data)), "sha256 len {len}");
            }
        }
    }

    #[test]
    fn cached_rejects_tampered_blob() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path().to_path_buf());
        std::fs::create_dir_all(data.blobs_dir()).unwrap();
        let good = b"hello".to_vec();
        let name = hex_of(&Sha256::digest(&good));
        let spec = HashSpec {
            algo: HashAlgo::Sha256,
            hex: name.clone(),
        };
        let idx = BTreeMap::new();
        let path = data.blobs_dir().join(&name);
        std::fs::write(&path, &good).unwrap();
        assert_eq!(cached(&data, &idx, &spec), Some(path.clone()));
        std::fs::write(&path, b"evil").unwrap();
        assert_eq!(cached(&data, &idx, &spec), None);
    }

    #[test]
    fn cached_rejects_blob_whose_name_is_not_its_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path().to_path_buf());
        std::fs::create_dir_all(data.blobs_dir()).unwrap();
        let content = b"hello";
        let spec = HashSpec {
            algo: HashAlgo::Sha1,
            hex: hex_of(&Sha1::digest(content)),
        };
        let name = "0".repeat(64);
        std::fs::write(data.blobs_dir().join(&name), content).unwrap();
        let idx = BTreeMap::from([(algo_key(&spec), name)]);
        assert_eq!(cached(&data, &idx, &spec), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cached_lookup_does_not_block_the_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path().to_path_buf());
        std::fs::create_dir_all(data.blobs_dir()).unwrap();
        let content: Vec<u8> = (0..64 << 20).map(|i: usize| (i % 251) as u8).collect();
        let name = hex_of(&Sha256::digest(&content));
        std::fs::write(data.blobs_dir().join(&name), &content).unwrap();
        let spec = HashSpec {
            algo: HashAlgo::Sha256,
            hex: name,
        };
        let ticks = std::sync::Arc::new(AtomicU64::new(0));
        let ticker = tokio::spawn({
            let ticks = ticks.clone();
            async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                    ticks.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        let found = cached_blocking(&data, &BTreeMap::new(), &spec)
            .await
            .unwrap();
        ticker.abort();
        assert!(found.is_some());
        assert!(ticks.load(Ordering::Relaxed) >= 2, "runtime was blocked");
    }
}
