//! Installer download into the verified store (Task 177).
//!
//! The bytes go through `install::download` (streamed to `tmp\dl-*.part`, hashed in
//! streaming, renamed to `blobs\<sha256>`); this module only checks the manifest data before
//! any request and materializes the blob under `installers\<id>\<ver>\` with a stable name.

use std::path::{Path, PathBuf};

use crate::bridge::{Tool, valid_target};
use crate::cache::DataDir;
use crate::http::Http;
use crate::install::download::{self, DownloadError, Progress};
use crate::install::manifest::{HashAlgo, HashSpec, Source};
use crate::install::safepath::safe_components;
use crate::sources::winget::manifest::{EffectiveInstaller, InstallerKind};

/// Longest sanitized version accepted as a folder name.
const MAX_VERSION_LEN: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("o instalador não usa https: {0}")]
    NotHttps(String),
    #[error("o manifesto não traz um SHA-256 válido para o instalador")]
    BadSha,
    #[error("identificador ou versão inválidos: {0}")]
    BadId(String),
    #[error(transparent)]
    Download(#[from] DownloadError),
    #[error("E/S: {0}")]
    Io(String),
    #[error("o instalador mudou depois de conferido")]
    Integrity,
}

/// File extension of the materialized installer.
pub fn extension(kind: &InstallerKind) -> &'static str {
    match kind {
        InstallerKind::Msi | InstallerKind::Wix => "msi",
        InstallerKind::Msix => "msix",
        InstallerKind::Appx => "appx",
        _ => "exe",
    }
}

/// An installer linked (or copied) out of the blob cache; dropping it removes the file and
/// the `<ver>` folder when empty. The blob stays in the cache.
///
/// Drop order: the share-deny handle from [`Materialized::verify`] is closed **before** the
/// file is deleted (it denies delete, so the other order would leave the file behind).
#[derive(Debug)]
pub struct Materialized {
    path: PathBuf,
    sha256: String,
    /// The data root: the folders from it down to `path` are held by `verify`.
    #[cfg_attr(not(windows), allow(dead_code))]
    root: PathBuf,
    #[cfg(windows)]
    verified: Option<super::verified_win::VerifiedFile>,
    #[cfg(windows)]
    folders: Vec<std::fs::File>,
}

impl Materialized {
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Lowercase hex.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    /// Holds the folders from the data root down (none may be a reparse point), opens the file
    /// share-deny without following a reparse point and checks its SHA-256 through that
    /// handle; the handles live as long as `self` (reopening replaces them).
    #[cfg(windows)]
    pub fn verify(&mut self) -> Result<&super::verified_win::VerifiedFile, FetchError> {
        self.verified = None;
        self.folders = super::verified_win::hold_chain(&self.root, &self.path)?;
        let v = super::verified_win::VerifiedFile::open(&self.path, &self.sha256)?;
        Ok(self.verified.insert(v))
    }

    /// The path an elevated launch is given: the held file's final path once verified.
    pub fn launch_path(&self) -> PathBuf {
        #[cfg(windows)]
        if let Some(v) = &self.verified {
            return v.final_path().to_path_buf();
        }
        self.path.clone()
    }

    /// Closes the share-deny handle: from then on the file may change on disk. Called only
    /// once the installer process has ended (Task 196).
    pub fn release(&mut self) {
        #[cfg(windows)]
        {
            drop(self.verified.take());
            self.folders.clear();
        }
    }
}

impl Drop for Materialized {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            drop(self.verified.take());
            self.folders.clear();
        }
        if let Err(e) = std::fs::remove_file(&self.path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("não foi possível apagar {}: {e}", self.path.display());
        }
        if let Some(dir) = self.path.parent() {
            remove_dir_if_empty(dir);
        }
    }
}

fn remove_dir_if_empty(dir: &Path) {
    match std::fs::remove_dir(dir) {
        Ok(()) => {}
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(e) => tracing::warn!("não foi possível apagar {}: {e}", dir.display()),
    }
}

/// `ver` with every byte outside `[A-Za-z0-9._+-]` replaced by `_`.
fn sanitize_version(ver: &str) -> String {
    ver.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._+-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// True if `name` is exactly one safe path component.
fn one_component(name: &str) -> bool {
    safe_components(name).is_some_and(|c| c.len() == 1 && c[0] == name)
}

/// `https`, or plain `http` only to a loopback host (`127.0.0.1`, `::1`, `localhost`) and
/// without credentials in the URL. Parsed with `reqwest::Url`, never by text prefix.
fn scheme_allowed(raw: &str) -> bool {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    let Ok(u) = reqwest::Url::parse(raw) else {
        return false;
    };
    match u.scheme() {
        "https" => true,
        "http" => {
            // `host_str` is already normalized by the parser (lowercase, canonical IPv4,
            // IPv6 in brackets); IPs are compared as addresses.
            let host = u.host_str().unwrap_or("");
            let loopback = host == "localhost"
                || host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<IpAddr>()
                    .is_ok_and(|a| {
                        a == IpAddr::V4(Ipv4Addr::LOCALHOST) || a == IpAddr::V6(Ipv6Addr::LOCALHOST)
                    });
            loopback && u.username().is_empty() && u.password().is_none()
        }
        _ => false,
    }
}

/// Downloads (or reuses from the cache) the installer, verified against its SHA-256, and
/// materializes it at `<root>\installers\<id>\<ver>\<id>-<ver>.<ext>`.
pub async fn fetch_installer(
    http: &Http,
    data: &DataDir,
    id: &str,
    version: &str,
    inst: &EffectiveInstaller,
    progress: Progress<'_>,
) -> Result<Materialized, FetchError> {
    fetch_installer_until(http, data, id, version, inst, progress, &|| false).await
}

/// [`fetch_installer`] whose download stops (`DownloadError::Cancelled`, the partial file
/// deleted) once `stop()` says so (Ctrl+C before the launch).
pub async fn fetch_installer_until(
    http: &Http,
    data: &DataDir,
    id: &str,
    version: &str,
    inst: &EffectiveInstaller,
    progress: Progress<'_>,
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<Materialized, FetchError> {
    if !scheme_allowed(&inst.url) {
        return Err(FetchError::NotHttps(inst.url.clone()));
    }
    let sha = inst.sha256.trim().to_ascii_lowercase();
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(FetchError::BadSha);
    }
    // The id also names the `installers\<id>` folder: one safe component (no `NUL`, no
    // trailing dot that would collide with the id without it).
    if !valid_target(Tool::Winget, id) || !one_component(id) {
        return Err(FetchError::BadId(id.to_string()));
    }
    let ver = sanitize_version(version);
    let file_name = format!("{id}-{ver}.{}", extension(&inst.kind));
    if ver.len() > MAX_VERSION_LEN || !one_component(&ver) || !one_component(&file_name) {
        return Err(FetchError::BadId(format!("{id} {version}")));
    }

    let source = Source {
        url: inst.url.clone(),
        file_name: file_name.clone(),
        hash: HashSpec {
            algo: HashAlgo::Sha256,
            hex: sha.clone(),
        },
        extract_dir: None,
        extract_to: None,
    };
    let http = http
        .with_strict_download_redirects()
        .map_err(|e| FetchError::Download(DownloadError::Http(e)))?;
    let blob =
        download::fetch_all_until(&http, data, std::slice::from_ref(&source), progress, stop)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| FetchError::Io("download sem resultado".into()))?;

    let ver_dir = data.root().join("installers").join(id).join(&ver);
    let target = ver_dir.join(&file_name);
    let job = tokio::task::spawn_blocking({
        let (blob, ver_dir, target) = (blob, ver_dir, target.clone());
        move || materialize(&blob, &ver_dir, &target)
    });
    match job.await {
        Ok(r) => r?,
        Err(e) => match e.try_into_panic() {
            Ok(p) => std::panic::resume_unwind(p),
            Err(e) => return Err(FetchError::Io(e.to_string())),
        },
    }
    Ok(Materialized {
        path: target,
        sha256: sha,
        root: data.root().to_path_buf(),
        #[cfg(windows)]
        verified: None,
        #[cfg(windows)]
        folders: Vec::new(),
    })
}

/// Replaces `target` with a hard link to `blob` (a copy when linking fails).
fn materialize(blob: &Path, ver_dir: &Path, target: &Path) -> Result<(), FetchError> {
    let io = |p: &Path, e: std::io::Error| FetchError::Io(format!("{}: {e}", p.display()));
    std::fs::create_dir_all(ver_dir).map_err(|e| io(ver_dir, e))?;
    let result = (|| {
        match std::fs::remove_file(target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(target, e)),
        }
        if let Err(e) = std::fs::hard_link(blob, target) {
            tracing::debug!("hardlink de {} falhou ({e}); copiando", blob.display());
            std::fs::copy(blob, target).map_err(|e| io(target, e))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(target);
        remove_dir_if_empty(ver_dir);
    }
    result
}
