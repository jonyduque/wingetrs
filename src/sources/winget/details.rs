//! Winget package details: versionData -> merged manifest (cached by sha256).

use super::{WingetError, manifest, version_data};
use crate::cache::BlobStore;
use crate::core::PackageDetails;
use crate::core::version::Version;
use crate::http::Http;

pub async fn details(
    http: &Http,
    blobs: &BlobStore,
    cdn_base: &str,
    id: &str,
    hash: &[u8],
    version: Option<&str>,
) -> Result<PackageDetails, WingetError> {
    // versionData has no known content hash, so it is always downloaded (~5 KB).
    let vd_url = format!("{cdn_base}{}", version_data::version_data_path(id, hash));
    let vd = version_data::decode_and_parse(&http.get(&vd_url).await?)?;

    let entry = match version {
        Some(v) => vd.entries.iter().find(|e| e.version == v),
        None => vd
            .entries
            .iter()
            .max_by(|a, b| Version::parse(&a.version).cmp(&Version::parse(&b.version))),
    }
    .ok_or_else(|| WingetError::VersionNotFound(version.unwrap_or("").to_owned()))?;

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

    let mut versions: Vec<String> = vd.entries.iter().map(|e| e.version.clone()).collect();
    versions.sort_by(|a, b| Version::parse(b).cmp(&Version::parse(a)));
    Ok(manifest::parse(&bytes)?.into_details(versions))
}
