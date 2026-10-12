//! Winget package details: versionData -> merged manifest (cached by sha256).

use super::{WingetError, manifest, version_data};
use crate::cache::BlobStore;
use crate::core::PackageDetails;
use crate::core::version::Version;
use crate::http::Http;
use tracing::Instrument;

pub async fn details(
    http: &Http,
    blobs: &BlobStore,
    cdn_base: &str,
    id: &str,
    hash: &[u8],
    version: Option<&str>,
) -> Result<PackageDetails, WingetError> {
    // versionData is content-addressed: the catalog's winget_hash is its SHA-256 (probed in
    // Task 70), so it is cached in the verified BlobStore under the full hash.
    let vd_key = hex::encode(hash);
    let vd_url = format!("{cdn_base}{}", version_data::version_data_path(id, hash));
    let vd = async {
        let bytes = match blobs.get(&vd_key) {
            Some(b) => b,
            None => {
                let b = http.get(&vd_url).await?;
                match blobs.put_verified(&vd_key, &b) {
                    Ok(()) => {}
                    Err(crate::cache::CacheError::HashMismatch { expected, actual }) => {
                        tracing::warn!(%expected, %actual, url = %vd_url, "versionData does not match the catalog hash; not cached");
                    }
                    Err(other) => return Err(WingetError::Cache(other)),
                }
                b
            }
        };
        Ok::<_, WingetError>(version_data::decode_and_parse(&bytes)?)
    }
    .instrument(tracing::debug_span!("show.version_data"))
    .await?;

    let entry = match version {
        Some(v) => vd.entries.iter().find(|e| e.version == v),
        None => vd
            .entries
            .iter()
            .max_by(|a, b| Version::parse(&a.version).cmp(&Version::parse(&b.version))),
    }
    .ok_or_else(|| WingetError::VersionNotFound(version.unwrap_or("").to_owned()))?;

    let merged = async {
        let bytes = match blobs.get(&entry.sha256) {
            Some(b) => b,
            None => {
                let b = http.get(&format!("{cdn_base}{}", entry.rel_path)).await?;
                blobs.put_verified(&entry.sha256, &b).map_err(|e| match e {
                    crate::cache::CacheError::HashMismatch { .. } => WingetError::ManifestIntegrity,
                    other => WingetError::Cache(other),
                })?;
                b
            }
        };
        Ok::<_, WingetError>(manifest::parse(&bytes)?)
    }
    .instrument(tracing::debug_span!("show.manifest"))
    .await?;

    let mut versions: Vec<String> = vd.entries.iter().map(|e| e.version.clone()).collect();
    versions.sort_by(|a, b| Version::parse(b).cmp(&Version::parse(a)));
    Ok(merged.into_details(versions))
}
