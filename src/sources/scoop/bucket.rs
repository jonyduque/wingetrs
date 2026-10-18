//! Scoop bucket tarball import.
use super::{ScoopError, manifest};
use crate::cache::HttpEntry;
use crate::catalog::NewPackage;
use crate::http::{Fetch, Http};
use crate::install::safepath::safe_components;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct BucketImport {
    pub packages: Vec<NewPackage>,
    pub skipped: Vec<String>,
    /// Regular files under `<top>/scripts/`: path relative to `scripts/` (with `/`) and content.
    pub scripts: Vec<(String, Vec<u8>)>,
}

const MAX_SCRIPT_FILES: usize = 2000;
const MAX_SCRIPT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SCRIPT_NOTES: usize = 2000;

/// Notes a skipped script, up to a cap; the excess is only counted.
fn note_skipped(skipped: &mut Vec<String>, overflow: &mut usize, note: String) {
    if skipped.len() < MAX_SCRIPT_NOTES {
        skipped.push(note);
    } else {
        *overflow += 1;
    }
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

/// Tarball URL of a GitHub repo's default branch (`HEAD` answers 200 for any default branch;
/// `refs/heads/HEAD` answers 404 - measured 2026-10-10).
pub fn repo_tarball_url(repo: &str) -> String {
    format!("https://codeload.github.com/{repo}/tar.gz/HEAD")
}

/// Returns the path relative to `scripts/` when the path is `<top>/scripts/<rest>`.
fn scripts_rel(path: &str) -> Option<&str> {
    let (_top, rest) = path.split_once('/')?;
    rest.strip_prefix("scripts/")
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
    let mut scripts: Vec<(String, Vec<u8>)> = Vec::new();
    let mut script_bytes = 0u64;
    let mut limit_hit = false;
    let mut overflow = 0usize;
    // Lowercased paths already taken (Windows is case-insensitive) and their parent folders.
    let mut seen_files = std::collections::HashSet::new();
    let mut seen_dirs = std::collections::HashSet::new();
    let entries = archive
        .entries()
        .map_err(|e| ScoopError::Tar(e.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| ScoopError::Tar(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| ScoopError::Tar(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let Some(file) = manifest_file(&path) else {
            if let Some(rel) = scripts_rel(&path) {
                let kind = entry.header().entry_type();
                if kind.is_dir() || rel.is_empty() {
                    continue;
                }
                if !kind.is_file() {
                    note_skipped(&mut skipped, &mut overflow, format!("scripts/{rel}"));
                    continue;
                }
                let Some(parts) = safe_components(rel) else {
                    note_skipped(&mut skipped, &mut overflow, format!("scripts/{rel}"));
                    continue;
                };
                if parts.is_empty() {
                    continue;
                }
                let lower: Vec<String> = parts.iter().map(|p| p.to_lowercase()).collect();
                let full = lower.join("/");
                let prefixes: Vec<String> =
                    (1..lower.len()).map(|n| lower[..n].join("/")).collect();
                if seen_files.contains(&full)
                    || seen_dirs.contains(&full)
                    || prefixes.iter().any(|p| seen_files.contains(p))
                {
                    note_skipped(&mut skipped, &mut overflow, format!("scripts/{rel}"));
                    continue;
                }
                let size = entry.size();
                if limit_hit
                    || scripts.len() >= MAX_SCRIPT_FILES
                    || script_bytes.saturating_add(size) > MAX_SCRIPT_BYTES
                {
                    if !limit_hit {
                        skipped.push("scripts: limite excedido".to_string());
                    }
                    limit_hit = true;
                    continue;
                }
                let mut buf = Vec::new();
                entry
                    .read_to_end(&mut buf)
                    .map_err(|e| ScoopError::Tar(e.to_string()))?;
                script_bytes += buf.len() as u64;
                seen_files.insert(full);
                seen_dirs.extend(prefixes);
                scripts.push((parts.join("/"), buf));
            }
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
    if overflow > 0 {
        skipped.push(format!("scripts: mais {overflow} ignorados"));
    }
    Ok(BucketImport {
        packages,
        skipped,
        scripts,
    })
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
        Fetch::Modified { body, entry } => {
            let import = tracing::debug_span!("update.parse").in_scope(|| read_tarball(&body))?;
            Ok(ScoopFetch::Modified { import, entry })
        }
    }
}

/// Replaces `<bucket_dir>\scripts` with `files` through a staging directory and writes the
/// `.scripts-ok` marker. On failure the staging directory is removed and the old `scripts`
/// stays intact.
pub fn write_scripts(
    bucket_dir: &Path,
    files: &[(String, Vec<u8>)],
    tag: &str,
) -> std::io::Result<()> {
    let new_dir = bucket_dir.join(format!(".scripts-new-{tag}"));
    let old_dir = bucket_dir.join(format!(".scripts-old-{tag}"));
    let live = bucket_dir.join("scripts");
    // Drop the marker first: from here until the swap is done, `scripts/` may be missing.
    match std::fs::remove_file(bucket_dir.join(".scripts-ok")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    remove_stale_staging(bucket_dir, &[&new_dir, &old_dir]);
    let result = stage_and_swap(bucket_dir, files, &new_dir, &old_dir, &live);
    if result.is_err() {
        // A leftover that is not a directory (or not ours) is left alone by remove_dir_all.
        let _ = std::fs::remove_dir_all(&new_dir);
        return result;
    }
    std::fs::write(bucket_dir.join(".scripts-ok"), b"")
}

/// Removes `.scripts-new-*`/`.scripts-old-*` left by a dead run (other tags). Best effort.
fn remove_stale_staging(bucket_dir: &Path, keep: &[&Path]) {
    let Ok(rd) = std::fs::read_dir(bucket_dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let p = e.path();
        if (name.starts_with(".scripts-new-") || name.starts_with(".scripts-old-"))
            && !keep.contains(&p.as_path())
        {
            let _ = std::fs::remove_dir_all(&p).or_else(|_| std::fs::remove_file(&p));
        }
    }
}

fn stage_and_swap(
    bucket_dir: &Path,
    files: &[(String, Vec<u8>)],
    new_dir: &Path,
    old_dir: &Path,
    live: &Path,
) -> std::io::Result<()> {
    std::fs::create_dir_all(bucket_dir)?;
    // `create_dir` (not `_all`) so a leftover with the same tag fails instead of being merged.
    std::fs::create_dir(new_dir)?;
    for (rel, bytes) in files {
        let invalid = || std::io::Error::new(std::io::ErrorKind::InvalidInput, rel.clone());
        let parts = safe_components(rel).ok_or_else(invalid)?;
        if parts.is_empty() {
            return Err(invalid());
        }
        let mut target = new_dir.to_path_buf();
        target.extend(&parts);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, bytes)?;
    }
    let had_old = live.exists();
    if had_old {
        crate::fs_retry::retry("renomear", live, || std::fs::rename(live, old_dir))?;
    }
    if let Err(e) = crate::fs_retry::retry("renomear", new_dir, || std::fs::rename(new_dir, live)) {
        if had_old && let Err(r) = std::fs::rename(old_dir, live) {
            tracing::warn!(error = %r, "could not restore the old scripts folder");
        }
        return Err(e);
    }
    if had_old {
        let _ = crate::fs_retry::retry("apagar", old_dir, || std::fs::remove_dir_all(old_dir));
    }
    Ok(())
}
