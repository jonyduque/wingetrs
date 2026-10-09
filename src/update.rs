//! Catalog update orchestration.
use crate::cache::{DataDir, HttpEntry, HttpState, UpdateLock, now_unix};
use crate::catalog::{Catalog, NewPackage, WingetMaps, index_file};
use crate::core::SourceKind;
use crate::http::Http;
use crate::sources::scoop::bucket::{BucketImport, ScoopFetch, bucket_url, fetch_bucket};
use crate::sources::scoop::source_name;
use crate::sources::winget::index::WingetImport;
use crate::sources::winget::{SOURCE_NAME, SOURCE2_URL, WingetFetch, fetch_index};
use std::time::Duration;

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
        for b in &cfg.scoop_buckets {
            let url = match &base {
                Some(base) => format!("{}/{b}.tgz", base.trim_end_matches('/')),
                None => bucket_url(b)?.to_string(),
            };
            scoop.push((b.clone(), url));
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
    }];
    for (b, url) in &endpoints.scoop {
        jobs.push(Job {
            source: source_name(b),
            kind: SourceKind::Scoop,
            url,
        });
    }

    let fetches = jobs.iter().map(|job| {
        let prev = state.get(job.url).cloned();
        let skip = !force && !needs_refresh(&state, &[job.url], now);
        async move {
            if skip {
                return None;
            }
            Some(fetch_one(job, http, data, prev.as_ref()).await)
        }
    });
    let results = futures_util::future::join_all(fetches).await;

    let mut catalog = Catalog::open(&data.catalog_path())?;
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
            Some(Ok(Fetched::Winget(imp, entry))) => apply(
                &mut catalog,
                &mut state,
                job,
                entry,
                now,
                &imp.packages,
                &imp.maps,
                0,
            ),
            Some(Ok(Fetched::Scoop(imp, entry))) => apply(
                &mut catalog,
                &mut state,
                job,
                entry,
                now,
                &imp.packages,
                &WingetMaps::default(),
                imp.skipped.len(),
            ),
        };
        reports.push(SourceReport {
            source: job.source.clone(),
            outcome,
        });
    }
    let saved = state.save(&data.http_state_path());
    let index_path = data.search_index_path();
    let updated = reports
        .iter()
        .any(|r| matches!(r.outcome, Outcome::Updated { .. }));
    if updated || !index_path.exists() {
        write_search_index(&catalog, &index_path);
    }
    saved?;
    Ok(reports)
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
