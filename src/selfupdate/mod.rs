//! `wingetrs self update` (Task 213): the newest release of wingetrs replaces the running
//! executable. Platform-neutral logic; the scope check and the real UAC launch live in
//! `apply_win.rs`.
use crate::github::{self, GhError, GhRepo};
use crate::http::{Http, HttpError};
use crate::install::progress::{Phase, ProgressEvent, SelfUpdateStep};
use crate::winget_install::launch::{LaunchSpec, Launcher, Show, Verb};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub mod apply_win;

/// Owner of the wingetrs repository on GitHub.
pub const REPO_OWNER: &str = "jonyduque";
/// The wingetrs repository on GitHub.
pub const REPO_NAME: &str = "wingetrs";
/// Base of the web redirect `…/releases/latest` (no API quota).
pub const WEB_BASE: &str = "https://github.com";
/// Base of the release assets: `{DL_BASE}/{tag}/wingetrs.exe`.
pub const DL_BASE: &str = "https://github.com/jonyduque/wingetrs/releases/download";
/// The only parameters the elevated copy is started with: nothing from the unelevated side.
pub const ELEVATED_PARAMS: &str = "self apply-elevated";
/// Name of the release asset (and of its `.sha256`).
const ASSET: &str = "wingetrs.exe";
/// Ceiling of the `.sha256` file.
const MAX_SUM: usize = 1024;
/// Ceiling of the downloaded executable.
const MAX_EXE: u64 = 256 * 1024 * 1024;

/// Exit codes of `self apply-elevated`.
pub const CODE_UPDATED: i32 = 0;
pub const CODE_LATEST: i32 = 10;
pub const CODE_OFFLINE: i32 = 20;
pub const CODE_HASH: i32 = 21;
pub const CODE_OTHER: i32 = 22;

#[derive(Debug, thiserror::Error)]
pub enum SelfUpdateError {
    #[error("sem conexão com o GitHub")]
    Offline,
    #[error("limite de requisições do GitHub atingido; tente mais tarde")]
    RateLimited,
    #[error("o SHA-256 do executável baixado não confere com o da release")]
    HashMismatch,
    #[error("o arquivo wingetrs.exe.sha256 da release não tem um SHA-256 válido")]
    BadSumFile,
    #[error("build de desenvolvimento: atualize com cargo")]
    DevBuild,
    #[error("a atualização como administrador falhou (código {0})")]
    Elevation(i32),
    #[error("{}", crate::install::progress::ELEVATION_REFUSED)]
    ElevationRefused,
    #[error("a release tem uma tag fora do formato vX.Y.Z: {0}")]
    BadTag(String),
    #[error("falha ao baixar a atualização: {0}")]
    Other(String),
    #[error("falha de E/S: {0}")]
    Io(#[from] std::io::Error),
}

impl SelfUpdateError {
    fn from_gh(e: GhError) -> SelfUpdateError {
        match e {
            GhError::Offline => SelfUpdateError::Offline,
            GhError::RateLimited { .. } => SelfUpdateError::RateLimited,
            other => SelfUpdateError::Other(other.to_string()),
        }
    }

    fn from_http(e: HttpError) -> SelfUpdateError {
        match e {
            e if e.is_offline() => SelfUpdateError::Offline,
            other => SelfUpdateError::Other(other.to_string()),
        }
    }
}

/// A release version `X.Y.Z`, compared field by field as numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// `^v?\d+\.\d+\.\d+$` (ASCII digits) as a [`Version`]; anything else is `None`.
pub fn parse_tag(tag: &str) -> Option<Version> {
    let body = tag.strip_prefix('v').unwrap_or(tag);
    let mut parts = body.split('.');
    let mut next = || -> Option<u64> {
        let p = parts.next()?;
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse().ok()
    };
    let v = Version {
        major: next()?,
        minor: next()?,
        patch: next()?,
    };
    parts.next().is_none().then_some(v)
}

/// The version of `tag` when it is newer than `current`; `None` when it is not, or when either
/// is outside `v?X.Y.Z` (never compared as text).
pub fn newer(current: &str, tag: &str) -> Option<Version> {
    let cur = parse_tag(current)?;
    let new = parse_tag(tag)?;
    (new > cur).then_some(new)
}

/// The SHA-256 of a `.sha256` file: `<hex64>` alone or followed by `wingetrs.exe` (optionally
/// `*wingetrs.exe`), one line.
pub fn parse_sum(body: &[u8]) -> Result<[u8; 32], SelfUpdateError> {
    let text = std::str::from_utf8(body).map_err(|_| SelfUpdateError::BadSumFile)?;
    let mut words = text.split_ascii_whitespace();
    let hash = words.next().ok_or(SelfUpdateError::BadSumFile)?;
    match words.next() {
        None => {}
        Some(name) if name.strip_prefix('*').unwrap_or(name) == ASSET => {}
        Some(_) => return Err(SelfUpdateError::BadSumFile),
    }
    if words.next().is_some() || hash.len() != 64 {
        return Err(SelfUpdateError::BadSumFile);
    }
    let mut out = [0u8; 32];
    hex::decode_to_slice(hash, &mut out).map_err(|_| SelfUpdateError::BadSumFile)?;
    Ok(out)
}

/// `<exe>.old`: the previous executable after a swap.
pub fn old_path(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// Deletes `<exe>.old` when it exists, silently (a machine-wide install without permission
/// keeps it until an elevated update).
pub fn remove_old(exe: &Path) {
    let old = old_path(exe);
    if let Err(e) = std::fs::remove_file(&old)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::debug!(error = %e, path = %old.display(), "self update: .old kept");
    }
}

/// A development build (a `Cargo.toml` two folders above the executable's folder, as in
/// `target/release`) is never replaced.
pub fn refuse_dev_build(exe: &Path) -> Result<(), SelfUpdateError> {
    let root = exe.parent().and_then(Path::parent).and_then(Path::parent);
    match root {
        Some(r) if r.join("Cargo.toml").exists() => Err(SelfUpdateError::DevBuild),
        _ => Ok(()),
    }
}

/// Deletes the temporary download when dropped, unless it was moved into place.
struct TempFile {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A new, empty `<dir>\wingetrs.exe.download-<random>` (never an existing file).
fn create_temp(dir: &Path) -> Result<TempFile, SelfUpdateError> {
    use std::hash::{BuildHasher, Hasher};
    for _ in 0..8 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
        );
        h.write_u32(std::process::id());
        let path = dir.join(format!("{ASSET}.download-{:016x}", h.finish()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(TempFile { path, armed: true }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(std::io::Error::other("nome temporário em uso").into())
}

fn sha256_file(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

/// [`apply_with`] without progress.
pub async fn apply(
    http: &Http,
    base_dl: &str,
    tag: &str,
    exe: &Path,
) -> Result<(), SelfUpdateError> {
    apply_with(http, base_dl, tag, exe, &|_| {}).await
}

/// Downloads `{base_dl}/{tag}/wingetrs.exe` (the pinned tag, never `latest/download`, so the
/// executable and its `.sha256` come from the same release) next to `exe`, checks its SHA-256
/// against `{base_dl}/{tag}/wingetrs.exe.sha256`, then swaps: the old `.old` is deleted when
/// possible, `exe` (running) is renamed to `.old` and the new file moved in. The temporary is
/// deleted on every error.
pub async fn apply_with(
    http: &Http,
    base_dl: &str,
    tag: &str,
    exe: &Path,
    progress: &(dyn Fn(&ProgressEvent) + Sync),
) -> Result<(), SelfUpdateError> {
    apply_capped(http, base_dl, tag, exe, MAX_EXE, progress).await
}

/// [`apply_with`] refusing an executable larger than `max` bytes (stopped mid-download).
pub async fn apply_capped(
    http: &Http,
    base_dl: &str,
    tag: &str,
    exe: &Path,
    max: u64,
    progress: &(dyn Fn(&ProgressEvent) + Sync),
) -> Result<(), SelfUpdateError> {
    if parse_tag(tag).is_none() {
        return Err(SelfUpdateError::BadTag(tag.to_string()));
    }
    let dir = exe
        .parent()
        .ok_or_else(|| std::io::Error::other("executável sem pasta"))?;
    let base = base_dl.trim_end_matches('/');
    let exe_url = format!("{base}/{tag}/{ASSET}");
    let sum_url = format!("{exe_url}.sha256");
    let body = http
        .get_limited(&sum_url, MAX_SUM)
        .await
        .map_err(|e| match e {
            HttpError::TooLarge(_) => SelfUpdateError::BadSumFile,
            other => SelfUpdateError::from_http(other),
        })?;
    let want = parse_sum(&body)?;
    let temp = create_temp(dir)?;
    let dl = http
        .with_strict_download_redirects()
        .map_err(SelfUpdateError::from_http)?;
    let too_big = std::sync::atomic::AtomicBool::new(false);
    let report = |done: u64, total: Option<u64>| {
        if done > max || total.is_some_and(|t| t > max) {
            too_big.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        progress(&ProgressEvent::Download {
            url: exe_url.clone(),
            done,
            total,
        });
    };
    let stop = || too_big.load(std::sync::atomic::Ordering::Relaxed);
    let finished = dl
        .get_to_file_until(&exe_url, &temp.path, report, &stop)
        .await
        .map_err(SelfUpdateError::from_http)?;
    if !finished || stop() {
        return Err(SelfUpdateError::Other(format!(
            "executável maior que {max} bytes"
        )));
    }
    progress(&ProgressEvent::Phase(Phase::SelfUpdate(
        SelfUpdateStep::CheckingHash,
    )));
    if sha256_file(&temp.path)? != want {
        return Err(SelfUpdateError::HashMismatch);
    }
    progress(&ProgressEvent::Phase(Phase::SelfUpdate(
        SelfUpdateStep::Swapping,
    )));
    swap(temp, exe)
}

/// `exe` → `<exe>.old`, `temp` → `exe`; on failure the old executable is put back.
fn swap(mut temp: TempFile, exe: &Path) -> Result<(), SelfUpdateError> {
    let old = old_path(exe);
    remove_old(exe);
    std::fs::rename(exe, &old)?;
    if let Err(e) = std::fs::rename(&temp.path, exe) {
        if let Err(back) = std::fs::rename(&old, exe) {
            tracing::warn!(error = %back, "self update: could not restore the old executable");
        }
        return Err(e.into());
    }
    temp.armed = false;
    Ok(())
}

/// What an update ended with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Updated,
    AlreadyLatest,
}

/// Elevated scope: starts `exe` itself through `launcher` with `Verb::RunAs` and the literal
/// parameters [`ELEVATED_PARAMS`] (nothing from this side reaches the elevated copy), from the
/// executable's own folder; `notice` runs right before (the elevation notice). The elevated
/// copy answers only by its exit code ([`from_exit_code`]).
pub fn elevated_apply(
    launcher: &dyn Launcher,
    exe: &Path,
    notice: &mut dyn FnMut(),
) -> Result<Outcome, SelfUpdateError> {
    elevated_apply_started(launcher, exe, notice, &mut || {})
}

/// [`elevated_apply`] calling `started` once the elevated copy runs (the UAC prompt was
/// accepted).
pub fn elevated_apply_started(
    launcher: &dyn Launcher,
    exe: &Path,
    notice: &mut dyn FnMut(),
    started: &mut dyn FnMut(),
) -> Result<Outcome, SelfUpdateError> {
    let spec = LaunchSpec {
        program: exe.to_path_buf(),
        params: ELEVATED_PARAMS.to_string(),
        dir: exe.parent().map(Path::to_path_buf),
        show: Show::Hidden,
        verb: Verb::RunAs,
    };
    notice();
    match launcher.run_started(&spec, started) {
        Ok(code) => from_exit_code(code),
        Err(e) if crate::winget_install::ops::uac_refused(&e) => {
            Err(SelfUpdateError::ElevationRefused)
        }
        Err(e) => Err(e.into()),
    }
}

/// The unelevated side's reading of an exit code of `self apply-elevated`.
pub fn from_exit_code(code: i32) -> Result<Outcome, SelfUpdateError> {
    match code {
        CODE_UPDATED => Ok(Outcome::Updated),
        CODE_LATEST => Ok(Outcome::AlreadyLatest),
        CODE_OFFLINE => Err(SelfUpdateError::Offline),
        CODE_HASH => Err(SelfUpdateError::HashMismatch),
        other => Err(SelfUpdateError::Elevation(other)),
    }
}

/// The exit code `self apply-elevated` answers with.
pub fn exit_code(r: &Result<Outcome, SelfUpdateError>) -> i32 {
    match r {
        Ok(Outcome::Updated) => CODE_UPDATED,
        Ok(Outcome::AlreadyLatest) => CODE_LATEST,
        Err(SelfUpdateError::Offline) => CODE_OFFLINE,
        Err(SelfUpdateError::HashMismatch) => CODE_HASH,
        Err(_) => CODE_OTHER,
    }
}

/// The work of the elevated copy: the latest tag of the wingetrs repository (fetched here,
/// never received), checked as `vX.Y.Z`, and [`apply`] when newer than `current`.
pub async fn elevated_run(
    http: &Http,
    web_base: &str,
    dl_base: &str,
    exe: &Path,
    current: &str,
) -> Result<Outcome, SelfUpdateError> {
    let tag = latest_tag(http, web_base).await?;
    if newer(current, &tag).is_none() {
        return Ok(Outcome::AlreadyLatest);
    }
    apply(http, dl_base, &tag, exe).await?;
    Ok(Outcome::Updated)
}

/// The latest release tag of wingetrs, refused unless it is `vX.Y.Z`.
pub async fn latest_tag(http: &Http, web_base: &str) -> Result<String, SelfUpdateError> {
    let repo = GhRepo {
        owner: REPO_OWNER.to_string(),
        repo: REPO_NAME.to_string(),
    };
    let tag = github::latest_tag(http, web_base, &repo)
        .await
        .map_err(SelfUpdateError::from_gh)?;
    if parse_tag(&tag).is_none() {
        return Err(SelfUpdateError::BadTag(tag));
    }
    Ok(tag)
}
