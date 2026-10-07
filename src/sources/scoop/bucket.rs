//! Scoop bucket tarball import.
use super::{ScoopError, manifest};
use crate::cache::HttpEntry;
use crate::catalog::NewPackage;
use crate::http::{Fetch, Http};
use std::io::Read;

#[derive(Debug, Clone, PartialEq)]
pub struct BucketImport {
    pub packages: Vec<NewPackage>,
    pub skipped: Vec<String>,
}

pub fn bucket_url(bucket: &str) -> Result<&'static str, ScoopError> {
    match bucket {
        "main" => Ok("https://codeload.github.com/ScoopInstaller/Main/tar.gz/refs/heads/master"),
        "extras" => {
            Ok("https://codeload.github.com/ScoopInstaller/Extras/tar.gz/refs/heads/master")
        }
        "versions" => {
            Ok("https://codeload.github.com/ScoopInstaller/Versions/tar.gz/refs/heads/master")
        }
        other => Err(ScoopError::UnknownBucket(other.to_string())),
    }
}

/// Returns the `<app>.json` file name when the path is `<any>/bucket/<app>.json`.
fn manifest_file(path: &str) -> Option<&str> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [_, "bucket", file] if file.ends_with(".json") && file.len() > 5 => Some(file),
        _ => None,
    }
}

pub fn read_tarball(tar_gz: &[u8]) -> Result<BucketImport, ScoopError> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tar_gz));
    let mut packages = Vec::new();
    let mut skipped = Vec::new();
    let entries = archive
        .entries()
        .map_err(|e| ScoopError::Tar(e.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| ScoopError::Tar(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| ScoopError::Tar(e.to_string()))?
            .to_string_lossy()
            .replace(char::from(92), "/");
        let Some(file) = manifest_file(&path) else {
            continue;
        };
        let file = file.to_string();
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| ScoopError::Tar(e.to_string()))?;
        let app = file.trim_end_matches(".json");
        match manifest::parse(app, &buf) {
            Ok(p) => packages.push(p),
            Err(_) => skipped.push(file),
        }
    }
    Ok(BucketImport { packages, skipped })
}

pub enum ScoopFetch {
    NotModified,
    Modified {
        import: BucketImport,
        entry: HttpEntry,
    },
}

pub async fn fetch_bucket(
    http: &Http,
    url: &str,
    prev: Option<&HttpEntry>,
) -> Result<ScoopFetch, ScoopError> {
    match http.get_conditional(url, prev).await? {
        Fetch::NotModified => Ok(ScoopFetch::NotModified),
        Fetch::Modified { body, entry } => Ok(ScoopFetch::Modified {
            import: read_tarball(&body)?,
            entry,
        }),
    }
}
