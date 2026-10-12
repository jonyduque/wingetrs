//! Downloads with hash verification and a content-addressed blob cache.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};

use crate::cache::{BlobStore, DataDir};
use crate::fs_retry;
use crate::http::Http;
use crate::install::manifest::{HashAlgo, HashSpec, Source};

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

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error(transparent)]
    Http(#[from] crate::http::HttpError),
    #[error("hash não confere para {url}")]
    HashMismatch { url: String },
    #[error("E/S: {0}")]
    Io(String),
}

pub type Progress<'a> = &'a (dyn Fn(&str, u64, Option<u64>) + Send + Sync);

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

/// Path of a cached, content-verified blob for `spec`, if any.
fn cached(data: &DataDir, idx: &BTreeMap<String, String>, spec: &HashSpec) -> Option<PathBuf> {
    let name = if spec.algo == HashAlgo::Sha256 {
        spec.hex.to_ascii_lowercase()
    } else {
        idx.get(&algo_key(spec))?.clone()
    };
    let blobs = BlobStore::new(data.blobs_dir());
    let bytes = blobs.get(&name)?; // checks sha256(content) == name
    // For sha256 the blob name is the manifest hash, so `get` already proved it.
    (spec.algo == HashAlgo::Sha256 || verify(spec, &bytes)).then(|| data.blobs_dir().join(name))
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
    http.get_to_file(&src.url, &tmp, |d, t| progress(&src.url, d, t))
        .await?;
    let bytes = std::fs::read(&tmp).map_err(|e| io_err(&tmp, e))?;
    if !verify(&src.hash, &bytes) {
        return Err(DownloadError::HashMismatch {
            url: src.url.clone(),
        }); // guard deletes the temp file
    }
    // For sha256 the verified manifest hash is the blob name; no second digest.
    let name = if src.hash.algo == HashAlgo::Sha256 {
        src.hash.hex.to_ascii_lowercase()
    } else {
        hex_of(&Sha256::digest(&bytes))
    };
    drop(bytes);
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
            if let Some(p) = cached(data, idx, &src.hash) {
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
