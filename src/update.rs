//! Catalog update orchestration.
use crate::cache::{BLOB_MAX_AGE, BlobStore, DataDir, HttpEntry, HttpState, UpdateLock, now_unix};
use crate::catalog::{Catalog, NewPackage, WingetMaps, index_file};
use crate::core::SourceKind;
use crate::http::Http;
use crate::sources::scoop::bucket::{BucketImport, ScoopFetch, fetch_bucket, repo_tarball_url};
use crate::sources::scoop::source_name;
use crate::sources::winget::index::WingetImport;
use crate::sources::winget::{SOURCE_NAME, SOURCE2_URL, WingetFetch, fetch_index};
use std::time::{Duration, SystemTime};
use tracing::Instrument;

pub const MAX_AGE_SECS: i64 = 900;
const DEFAULT_LOCK_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone)]
pub struct Endpoints {
    pub winget_index: String,
    /// (bucket, url)
    pub scoop: Vec<(String, String)>,
}

impl Endpoints {
    /// Production URLs; `WINGETRS_WINGET_INDEX_URL` / `WINGETRS_SCOOP_BASE_URL`
    /// override them (offline tests).
    pub fn production(cfg: &crate::config::Config) -> Result<Endpoints, UpdateError> {
        let winget_index =
            std::env::var("WINGETRS_WINGET_INDEX_URL").unwrap_or_else(|_| SOURCE2_URL.to_string());
        let base = std::env::var("WINGETRS_SCOOP_BASE_URL").ok();
        let mut scoop = Vec::new();
        for (b, repo) in crate::buckets::resolved(cfg)? {
            let url = match &base {
                Some(base) => format!("{}/{b}.tgz", base.trim_end_matches('/')),
                None => repo_tarball_url(&repo),
            };
            scoop.push((b, url));
        }
        Ok(Endpoints {
            winget_index,
            scoop,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Fresh,
    Unchanged,
    Updated {
        packages: usize,
        skipped: usize,
    },
    /// `offline` is true only for network-level errors (not HTTP status, integrity, format).
    Failed {
        message: String,
        offline: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceReport {
    pub source: String,
    pub outcome: Outcome,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("outra atualização em andamento")]
    Busy,
    #[error("falha interna: {0}")]
    Internal(String),
    #[error(transparent)]
    Cache(#[from] crate::cache::CacheError),
    #[error(transparent)]
    Catalog(#[from] crate::catalog::CatalogError),
    #[error(transparent)]
    Scoop(#[from] crate::sources::scoop::ScoopError),
}

/// True when any URL was never checked or was checked more than MAX_AGE_SECS ago.
pub fn needs_refresh(state: &HttpState, urls: &[&str], now: i64) -> bool {
    urls.iter().any(|u| match state.get(u) {
        None => true,
        Some(e) => now - e.checked_at > MAX_AGE_SECS,
    })
}

/// Minimum seconds between detached refresh attempts.
pub const RETRY_SECS: i64 = 300;

/// A detached refresh is due when a source is stale and the last attempt is at least
/// RETRY_SECS old (or in the future: the clock went back, never block forever).
pub fn background_due(
    state: &HttpState,
    urls: &[&str],
    last_attempt: Option<i64>,
    now: i64,
) -> bool {
    needs_refresh(state, urls, now) && last_attempt.is_none_or(|t| t > now || now - t >= RETRY_SECS)
}

/// Decides whether to spawn a detached refresh and records the attempt if so.
pub fn claim_background_refresh(data: &DataDir, urls: &[&str], now: i64) -> bool {
    let state = HttpState::load(&data.http_state_path());
    // needs_refresh first: the attempt file is only read when a refresh could be due.
    if !needs_refresh(&state, urls, now) {
        return false;
    }
    let last = std::fs::read_to_string(data.refresh_attempt_path())
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok());
    if !background_due(&state, urls, last, now) {
        return false;
    }
    if let Err(e) = std::fs::write(data.refresh_attempt_path(), now.to_string()) {
        tracing::warn!(error = %e, "could not record the refresh attempt");
    }
    true
}

// Short-lived value; boxing the imports buys nothing.
#[allow(clippy::large_enum_variant)]
enum Fetched {
    NotModified,
    Winget(WingetImport, HttpEntry),
    Scoop(BucketImport, HttpEntry),
}

struct Job<'a> {
    source: String,
    kind: SourceKind,
    url: &'a str,
    /// Bucket name, for scoop jobs.
    bucket: Option<&'a str>,
}

async fn fetch_one(
    job: &Job<'_>,
    http: &Http,
    data: &DataDir,
    prev: Option<&HttpEntry>,
) -> Result<Fetched, (String, bool)> {
    match job.kind {
        SourceKind::Scoop => match fetch_bucket(http, job.url, prev)
            .await
            .map_err(|e| (e.to_string(), e.is_offline()))?
        {
            ScoopFetch::NotModified => Ok(Fetched::NotModified),
            ScoopFetch::Modified { import, entry } => Ok(Fetched::Scoop(import, entry)),
        },
        SourceKind::Winget => match fetch_index(http, data, job.url, prev)
            .await
            .map_err(|e| (e.to_string(), e.is_offline()))?
        {
            WingetFetch::NotModified => Ok(Fetched::NotModified),
            WingetFetch::Modified { import, entry } => Ok(Fetched::Winget(import, entry)),
        },
    }
}

/// How long `run_update` waits for the update lock held by another process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockWait {
    Block(Duration),
    NoWait,
}

impl LockWait {
    /// 30 s, or `WINGETRS_LOCK_TIMEOUT_MS` (test-only knob to avoid the full wait).
    pub fn from_env() -> LockWait {
        let ms = std::env::var("WINGETRS_LOCK_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_LOCK_TIMEOUT_MS);
        LockWait::Block(Duration::from_millis(ms))
    }

    fn timeout(self) -> Duration {
        match self {
            LockWait::Block(d) => d,
            LockWait::NoWait => Duration::ZERO,
        }
    }
}

pub async fn run_update(
    data: &DataDir,
    endpoints: &Endpoints,
    http: &Http,
    force: bool,
    wait: LockWait,
) -> Result<Vec<SourceReport>, UpdateError> {
    let lock_path = data.lock_path();
    let timeout = wait.timeout();
    let lock =
        tokio::task::spawn_blocking(move || UpdateLock::acquire_timeout(&lock_path, timeout))
            .instrument(tracing::debug_span!("update.lock"))
            .await
            .map_err(|e| UpdateError::Internal(e.to_string()))??;
    let Some(_lock) = lock else {
        return Err(UpdateError::Busy);
    };

    let mut state = HttpState::load(&data.http_state_path());
    let now = now_unix();
    let mut jobs = vec![Job {
        source: SOURCE_NAME.to_string(),
        kind: SourceKind::Winget,
        url: &endpoints.winget_index,
        bucket: None,
    }];
    for (b, url) in &endpoints.scoop {
        jobs.push(Job {
            source: source_name(b),
            kind: SourceKind::Scoop,
            url,
            bucket: Some(b),
        });
    }

    let fetches = jobs.iter().map(|job| {
        let mut prev = state.get(job.url).cloned();
        // Without the marker the stored `scripts/` is missing or incomplete: refetch the tarball.
        if let Some(dir) = job.bucket.and_then(|b| bucket_dir(data, b)) {
            let marker = dir.join(".scripts-ok");
            if !marker.exists() {
                prev = None;
            }
        }
        let skip = !force && !needs_refresh(&state, &[job.url], now);
        let span = tracing::debug_span!("update.fetch", source = %job.source);
        async move {
            if skip {
                return None;
            }
            Some(fetch_one(job, http, data, prev.as_ref()).await)
        }
        .instrument(span)
    });
    let results = futures_util::future::join_all(fetches).await;

    let index_path = data.search_index_path();
    let mut catalog = if needs_catalog(&results, index_path.exists()) {
        Some(
            tracing::debug_span!("update.catalog_open")
                .in_scope(|| Catalog::open(&data.catalog_path()))?,
        )
    } else {
        None
    };
    let mut reports = Vec::new();
    for (job, res) in jobs.iter().zip(results) {
        let outcome = match res {
            None => Outcome::Fresh,
            Some(Err((message, offline))) => Outcome::Failed { message, offline },
            Some(Ok(Fetched::NotModified)) => {
                let mut e = state.get(job.url).cloned().unwrap_or_default();
                e.checked_at = now;
                state.set(job.url, e);
                Outcome::Unchanged
            }
            Some(Ok(Fetched::Winget(imp, entry))) => match catalog.as_mut() {
                Some(c) => apply(c, &mut state, job, entry, now, &imp.packages, &imp.maps, 0),
                None => no_catalog(),
            },
            Some(Ok(Fetched::Scoop(imp, entry))) => match catalog.as_mut() {
                Some(c) => {
                    store_scripts(data, job, &imp.scripts);
                    apply(
                        c,
                        &mut state,
                        job,
                        entry,
                        now,
                        &imp.packages,
                        &WingetMaps::default(),
                        imp.skipped.len(),
                    )
                }
                None => no_catalog(),
            },
        };
        reports.push(SourceReport {
            source: job.source.clone(),
            outcome,
        });
    }
    // Only Unchanged/Updated change an HttpEntry; all Fresh/Failed leaves http.json as is.
    let state_changed = reports
        .iter()
        .any(|r| matches!(r.outcome, Outcome::Unchanged | Outcome::Updated { .. }));
    let saved = if state_changed {
        tracing::debug_span!("update.state_save").in_scope(|| state.save(&data.http_state_path()))
    } else {
        Ok(())
    };
    let updated = reports
        .iter()
        .any(|r| matches!(r.outcome, Outcome::Updated { .. }));
    if let Some(c) = &catalog
        && (updated || !index_path.exists())
    {
        tracing::debug_span!("update.search_idx").in_scope(|| write_search_index(c, &index_path));
    }
    tracing::debug_span!("update.blob_prune").in_scope(|| prune_blobs(data));
    saved?;
    Ok(reports)
}

/// `<root>\buckets\<bucket>` when the name is one safe path component.
fn bucket_dir(data: &DataDir, bucket: &str) -> Option<std::path::PathBuf> {
    match crate::install::safepath::safe_components(bucket) {
        Some(p) if p.len() == 1 => Some(data.buckets_dir().join(&p[0])),
        _ => None,
    }
}

/// Stores the bucket's `scripts/` folder; a failure only costs the scripts (the marker is
/// dropped so the next update refetches unconditionally) and never fails the update.
fn store_scripts(data: &DataDir, job: &Job<'_>, scripts: &[(String, Vec<u8>)]) {
    let Some(dir) = job.bucket.and_then(|b| bucket_dir(data, b)) else {
        return;
    };
    let tag = std::process::id().to_string();
    let res = tracing::debug_span!("update.scripts")
        .in_scope(|| crate::sources::scoop::bucket::write_scripts(&dir, scripts, &tag));
    if let Err(e) = res {
        tracing::warn!(source = %job.source, error = %e, "could not store the bucket scripts");
        let _ = std::fs::remove_file(dir.join(".scripts-ok"));
    }
}

/// Best-effort blob cleanup under the install lock; never fails the update.
fn prune_blobs(data: &DataDir) {
    let _lock = match UpdateLock::try_acquire(&data.install_layout().install_lock()) {
        Ok(Some(l)) => l,
        Ok(None) => {
            tracing::debug!("blob prune skipped: install lock busy");
            return;
        }
        Err(e) => {
            tracing::debug!(error = %e, "blob prune skipped: install lock unavailable");
            return;
        }
    };
    let stats = match BlobStore::new(data.blobs_dir()).prune(SystemTime::now(), BLOB_MAX_AGE) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "blob prune failed");
            return;
        }
    };
    let index_removed = match crate::install::download::prune_index(data) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "blob index prune failed");
            0
        }
    };
    tracing::info!(
        removed = stats.removed,
        bytes = stats.bytes,
        kept = stats.kept,
        index_removed,
        "blob prune"
    );
}

/// True when the catalog must be opened: some source has an import to apply, or the
/// search index is missing and must be rebuilt from the catalog.
fn needs_catalog(results: &[Option<Result<Fetched, (String, bool)>>], index_exists: bool) -> bool {
    !index_exists
        || results
            .iter()
            .any(|r| matches!(r, Some(Ok(Fetched::Winget(..) | Fetched::Scoop(..)))))
}

/// Unreachable by construction: `needs_catalog` is true whenever an import exists.
fn no_catalog() -> Outcome {
    Outcome::Failed {
        message: "erro interno: catálogo não aberto para importar".to_string(),
        offline: false,
    }
}

/// Rewrites `search.idx`; on failure removes a possibly stale file so `search` falls back to
/// the catalog instead of returning outdated results.
fn write_search_index(catalog: &Catalog, path: &std::path::Path) {
    if let Err(e) = index_file::write(catalog, path) {
        tracing::warn!("falha ao gravar o índice de busca: {e}");
        let _ = std::fs::remove_file(path);
    }
}

#[allow(clippy::too_many_arguments)]
fn apply(
    catalog: &mut Catalog,
    state: &mut HttpState,
    job: &Job<'_>,
    mut entry: HttpEntry,
    now: i64,
    packages: &[NewPackage],
    maps: &WingetMaps,
    skipped: usize,
) -> Outcome {
    let _span = tracing::debug_span!("update.catalog_write", source = %job.source).entered();
    let version = entry
        .source_version
        .clone()
        .or_else(|| entry.etag.clone())
        .unwrap_or_default();
    match catalog.replace_source(&job.source, job.kind, &version, now, packages, maps) {
        Ok(stats) => {
            entry.checked_at = now;
            state.set(job.url, entry);
            Outcome::Updated {
                packages: stats.packages,
                skipped,
            }
        }
        Err(e) => Outcome::Failed {
            message: e.to_string(),
            offline: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_catalog_only_for_imports_or_missing_index() {
        let idle = || {
            vec![
                None,
                Some(Ok(Fetched::NotModified)),
                Some(Err(("x".to_string(), true))),
            ]
        };
        assert!(!needs_catalog(&idle(), true));
        assert!(needs_catalog(&idle(), false));
        let scoop = vec![Some(Ok(Fetched::Scoop(
            BucketImport {
                packages: vec![],
                skipped: vec![],
                scripts: vec![],
            },
            HttpEntry::default(),
        )))];
        assert!(needs_catalog(&scoop, true));
        let winget = vec![Some(Ok(Fetched::Winget(
            WingetImport {
                packages: vec![],
                maps: WingetMaps::default(),
            },
            HttpEntry::default(),
        )))];
        assert!(needs_catalog(&winget, true));
    }
}
