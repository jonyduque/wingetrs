//! Package icons: download by content hash into the blob store, and the
//! `icons.json` map from `<source>/<id>` to the cached icon's SHA-256.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cache::{BlobStore, CacheError, DataDir};
use crate::core::IconRef;
use crate::http::{Http, HttpError};

pub const MAX_ICON_BYTES: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum IconError {
    #[error("falha ao baixar o ícone: {0}")]
    Http(#[from] HttpError),
    #[error("ícone maior que o limite de {MAX_ICON_BYTES} bytes")]
    TooLarge,
    #[error("o ícone não confere com o hash do manifesto")]
    Integrity,
    #[error("falha no cache do ícone: {0}")]
    Cache(CacheError),
}

/// Bytes of the icon: from the blob store when the hash is there, else downloaded,
/// size-checked, hash-verified and stored.
pub async fn fetch(http: &Http, blobs: &BlobStore, icon: &IconRef) -> Result<Vec<u8>, IconError> {
    if let Some(hit) = blobs.get(&icon.sha256) {
        return Ok(hit);
    }
    let bytes = match http.get_limited(&icon.url, MAX_ICON_BYTES).await {
        Ok(b) => b,
        Err(HttpError::TooLarge(_)) => return Err(IconError::TooLarge),
        Err(e) => return Err(e.into()),
    };
    match blobs.put_verified(&icon.sha256, &bytes) {
        Ok(()) => Ok(bytes),
        Err(CacheError::HashMismatch { .. }) => Err(IconError::Integrity),
        Err(e) => Err(IconError::Cache(e)),
    }
}

static MAP_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn map_path(data: &DataDir) -> std::path::PathBuf {
    data.root().join("icons.json")
}

/// Missing or corrupt file yields an empty map.
pub fn load_map(data: &DataDir) -> BTreeMap<String, String> {
    let Ok(bytes) = std::fs::read(map_path(data)) else {
        return BTreeMap::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Records that `<source>/<id>` has its icon cached under `sha`. Failures are only logged.
pub fn remember(data: &DataDir, source: &str, id: &str, sha: &str) {
    if let Err(e) = try_remember(data, source, id, sha) {
        tracing::debug!(error = %e, "icons.json not updated");
    }
}

fn try_remember(data: &DataDir, source: &str, id: &str, sha: &str) -> std::io::Result<()> {
    let mut map = load_map(data);
    map.insert(format!("{source}/{id}"), sha.to_owned());
    let json = serde_json::to_vec(&map).map_err(std::io::Error::other)?;
    let p = map_path(data);
    std::fs::create_dir_all(data.root())?;
    let tmp = p.with_extension(format!(
        "json.{}-{}.tmp",
        std::process::id(),
        MAP_TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, json)?;
    crate::fs_retry::retry("renomear", &tmp, || std::fs::rename(&tmp, &p)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// The cached icon of `<source>/<id>`, if the map names a blob that is still valid.
pub fn cached_icon(
    data: &DataDir,
    map: &BTreeMap<String, String>,
    source: &str,
    id: &str,
) -> Option<Vec<u8>> {
    let sha = map.get(&format!("{source}/{id}"))?;
    BlobStore::new(data.blobs_dir()).get(sha)
}
