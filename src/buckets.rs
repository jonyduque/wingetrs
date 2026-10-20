//! Scoop bucket management: resolve configured buckets, `add`, `list`, `rm`.
use crate::config::{BucketEntry, Config};
use crate::sources::scoop::ScoopError;
use std::path::Path;

/// Well-known buckets (official scoop `buckets.json`, checked 2026-10-10).
const KNOWN: [(&str, &str); 10] = [
    ("main", "ScoopInstaller/Main"),
    ("extras", "ScoopInstaller/Extras"),
    ("versions", "ScoopInstaller/Versions"),
    ("nirsoft", "ScoopInstaller/Nirsoft"),
    ("sysinternals", "ScoopInstaller/Sysinternals"),
    ("php", "ScoopInstaller/PHP"),
    ("nerd-fonts", "matthewjberger/scoop-nerd-fonts"),
    ("nonportable", "ScoopInstaller/Nonportable"),
    ("java", "ScoopInstaller/Java"),
    ("games", "ScoopInstaller/Games"),
];

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BucketError {
    #[error("só buckets do GitHub nesta versão: {0}")]
    NotGithub(String),
    #[error("repositório inválido (use owner/repo ou https://github.com/owner/repo): {0}")]
    InvalidRepo(String),
    #[error("nome de bucket inválido: {0}")]
    InvalidName(String),
    #[error(
        "o bucket {name} ({repo}) não é conhecido; confirme num terminal ou rode de novo com --yes"
    )]
    NeedsConfirmation { name: String, repo: String },
    #[error("o bucket {0} já existe com outro repositório")]
    Exists(String),
    #[error("bucket não configurado: {0}")]
    NotConfigured(String),
    #[error("o bucket {bucket} tem apps instalados: {}", apps.join(", "))]
    InUse { bucket: String, apps: Vec<String> },
    #[error("falha ao remover do catálogo: {0}")]
    Catalog(String),
    #[error("não foi possível ler o state.db: {0}")]
    StateDb(String),
    #[error("falha ao gravar o estado: {0}")]
    State(String),
    #[error("uma atualização está em andamento; tente de novo")]
    Busy,
    #[error("repositório não encontrado: {0}")]
    RepoNotFound(String),
    #[error("não parece um bucket do Scoop (nenhum manifesto em bucket/)")]
    NotABucket,
    #[error("falha ao baixar o bucket: {0}")]
    Download(String),
    #[error("{0} é um bucket padrão; confirme num terminal ou rode de novo com --yes")]
    DefaultNeedsConfirmation(String),
}

/// Repo (`owner/repo`) of a well-known bucket name.
pub fn known_repo(name: &str) -> Option<&'static str> {
    KNOWN.iter().find(|(n, _)| *n == name).map(|(_, r)| *r)
}

fn is_known_repo(repo: &str) -> bool {
    KNOWN.iter().any(|(_, r)| r.eq_ignore_ascii_case(repo))
}

fn valid_part(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Normalizes `owner/repo` or a GitHub URL (optional `.git`, trailing slash) to `owner/repo`.
pub fn parse_repo(spec: &str) -> Result<String, BucketError> {
    let spec = spec.trim();
    let path = if let Some((scheme, rest)) = spec.split_once("://") {
        if !scheme.eq_ignore_ascii_case("https") {
            return Err(BucketError::InvalidRepo(spec.to_string()));
        }
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com")
        {
            return Err(BucketError::NotGithub(spec.to_string()));
        }
        path
    } else if spec.contains(':') || spec.contains('@') {
        return Err(BucketError::NotGithub(spec.to_string()));
    } else {
        spec
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    match path.split_once('/') {
        Some((o, r)) if valid_part(o) && valid_part(r) => Ok(format!("{o}/{r}")),
        _ => Err(BucketError::InvalidRepo(spec.to_string())),
    }
}

/// Effective `(name, repo)` list: `scoop_buckets` first (entry override or known repo), then extras.
pub fn resolved(cfg: &Config) -> Result<Vec<(String, String)>, ScoopError> {
    let mut out: Vec<(String, String)> = Vec::new();
    for name in &cfg.scoop_buckets {
        let repo = match cfg.buckets.iter().find(|e| &e.name == name) {
            Some(e) => e.repo.clone(),
            None => known_repo(name)
                .ok_or_else(|| ScoopError::UnknownBucket(name.clone()))?
                .to_string(),
        };
        out.push((name.clone(), repo));
    }
    for e in &cfg.buckets {
        if !out.iter().any(|(n, _)| n == &e.name) {
            out.push((e.name.clone(), e.repo.clone()));
        }
    }
    Ok(out)
}

/// `add`: a repo that is not well-known needs `confirmed`.
pub fn add(cfg: &mut Config, name: &str, spec: &str, confirmed: bool) -> Result<(), BucketError> {
    if !valid_part(name) {
        return Err(BucketError::InvalidName(name.to_string()));
    }
    let repo = parse_repo(spec)?;
    if let Ok(existing) = resolved(cfg)
        && let Some((_, r)) = existing.iter().find(|(n, _)| n == name)
    {
        return if r.eq_ignore_ascii_case(&repo) {
            Ok(())
        } else {
            Err(BucketError::Exists(name.to_string()))
        };
    }
    if known_repo(name).is_some_and(|r| r.eq_ignore_ascii_case(&repo)) {
        cfg.scoop_buckets.push(name.to_string());
        return Ok(());
    }
    if !is_known_repo(&repo) && !confirmed {
        return Err(BucketError::NeedsConfirmation {
            name: name.to_string(),
            repo,
        });
    }
    cfg.buckets.push(BucketEntry {
        name: name.to_string(),
        repo,
    });
    Ok(())
}

/// Apps in `state.db` installed from `bucket`. A missing file means none; an unreadable
/// database is an error (fail closed, so `rm` is not allowed by mistake).
pub fn installed_from_bucket(state_db: &Path, bucket: &str) -> Result<Vec<String>, BucketError> {
    if !state_db.exists() {
        return Ok(Vec::new());
    }
    let err = |e: rusqlite::Error| BucketError::StateDb(e.to_string());
    let conn =
        rusqlite::Connection::open_with_flags(state_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(err)?;
    let mut stmt = conn
        .prepare("SELECT app FROM apps WHERE bucket = ?1 ORDER BY app")
        .map_err(err)?;
    let rows = stmt
        .query_map([bucket], |r| r.get::<_, String>(0))
        .map_err(err)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(err)
}

/// `rm`: refuses while apps from the bucket are installed. Config only; the full flow is
/// `remove_and_cleanup`.
pub fn remove(cfg: &mut Config, name: &str, state_db: &Path) -> Result<(), BucketError> {
    let configured =
        cfg.scoop_buckets.iter().any(|n| n == name) || cfg.buckets.iter().any(|e| e.name == name);
    if !configured {
        return Err(BucketError::NotConfigured(name.to_string()));
    }
    let apps = installed_from_bucket(state_db, name)?;
    if !apps.is_empty() {
        return Err(BucketError::InUse {
            bucket: name.to_string(),
            apps,
        });
    }
    cfg.scoop_buckets.retain(|n| n != name);
    cfg.buckets.retain(|e| e.name != name);
    Ok(())
}

/// Buckets that ship with Scoop; `rm` asks before dropping them.
fn is_default_bucket(name: &str) -> bool {
    matches!(name, "main" | "extras")
}

/// Holds the `update` lock; busy means another update (or bucket command) is running.
fn lock(data: &crate::cache::DataDir) -> Result<crate::cache::UpdateLock, BucketError> {
    match crate::cache::UpdateLock::try_acquire(&data.lock_path()) {
        Ok(Some(l)) => Ok(l),
        Ok(None) => Err(BucketError::Busy),
        Err(e) => Err(BucketError::State(e.to_string())),
    }
}

/// `<root>\buckets\<name>` when `name` is one safe path component.
fn bucket_dir(data: &crate::cache::DataDir, name: &str) -> Option<std::path::PathBuf> {
    match crate::install::safepath::safe_components(name) {
        Some(p) if p.len() == 1 => Some(data.buckets_dir().join(&p[0])),
        _ => None,
    }
}

/// Downloads bucket `name` (already added to `cfg`, in memory) right now and, only when the
/// tarball is a Scoop bucket, stores its source in the catalog, rewrites `search.idx` and
/// saves `cfg`. Any failure leaves config, catalog and index untouched. Holds the update
/// lock. `url` overrides the tarball URL (tests). Returns the number of apps.
pub async fn add_downloaded(
    data: &crate::cache::DataDir,
    http: &crate::http::Http,
    cfg: &Config,
    name: &str,
    url: Option<&str>,
) -> Result<usize, BucketError> {
    use crate::sources::scoop::bucket::{ScoopFetch, fetch_bucket, repo_tarball_url};
    let _lock = lock(data)?;
    let repo = resolved(cfg)
        .map_err(|e| BucketError::Catalog(e.to_string()))?
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, r)| r)
        .ok_or_else(|| BucketError::NotConfigured(name.to_string()))?;
    let url = url.map_or_else(|| repo_tarball_url(&repo), str::to_string);
    let (import, mut entry) = match fetch_bucket(http, &url, None).await {
        Ok(ScoopFetch::Modified { import, entry }) => (import, entry),
        Ok(ScoopFetch::NotModified) => {
            return Err(BucketError::Download("resposta sem conteúdo".to_string()));
        }
        Err(ScoopError::Http(crate::http::HttpError::Status(404))) => {
            return Err(BucketError::RepoNotFound(repo));
        }
        Err(e) => return Err(BucketError::Download(e.to_string())),
    };
    if import.packages.is_empty() {
        return Err(BucketError::NotABucket);
    }
    let cat_err = |e: crate::catalog::CatalogError| BucketError::Catalog(e.to_string());
    let now = crate::cache::now_unix();
    let version = entry
        .source_version
        .clone()
        .or_else(|| entry.etag.clone())
        .unwrap_or_default();
    let mut catalog = crate::catalog::Catalog::open(&data.catalog_path()).map_err(cat_err)?;
    let stats = catalog
        .replace_source(
            &crate::sources::scoop::source_name(name),
            crate::core::SourceKind::Scoop,
            &version,
            now,
            &import.packages,
            &crate::catalog::WingetMaps::default(),
        )
        .map_err(cat_err)?;
    let idx = data.search_index_path();
    if let Err(e) = crate::catalog::index_file::write(&catalog, &idx) {
        tracing::warn!("falha ao gravar o índice de busca: {e}");
        let _ = std::fs::remove_file(&idx);
    }
    if let Some(dir) = bucket_dir(data, name) {
        let tag = std::process::id().to_string();
        if let Err(e) = crate::sources::scoop::bucket::write_scripts(&dir, &import.scripts, &tag) {
            tracing::warn!(bucket = name, error = %e, "could not store the bucket scripts");
            let _ = std::fs::remove_file(dir.join(".scripts-ok"));
        }
    }
    entry.checked_at = now;
    let path = data.http_state_path();
    let mut state = crate::cache::HttpState::load(&path);
    state.set(&url, entry);
    state
        .save(&path)
        .map_err(|e| BucketError::State(e.to_string()))?;
    cfg.save(&data.config_path())
        .map_err(|e| BucketError::State(e.to_string()))?;
    Ok(stats.packages)
}

/// One line of `bucket list`.
#[derive(Debug, Clone, PartialEq)]
pub struct BucketRow {
    pub name: String,
    pub repo: String,
    /// Apps in the catalog; `None` when the bucket was never downloaded.
    pub apps: Option<usize>,
    /// Unix seconds of the last download.
    pub updated: Option<i64>,
}

/// The configured buckets with their app count and last download from the catalog.
pub fn rows(data: &crate::cache::DataDir, cfg: &Config) -> Result<Vec<BucketRow>, BucketError> {
    let resolved = resolved(cfg).map_err(|e| BucketError::Catalog(e.to_string()))?;
    let conn = Some(data.catalog_path())
        .filter(|p| p.exists())
        .and_then(|p| {
            rusqlite::Connection::open_with_flags(p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .ok()
        });
    Ok(resolved
        .into_iter()
        .map(|(name, repo)| {
            let source = crate::sources::scoop::source_name(&name);
            let updated = conn.as_ref().and_then(|c| {
                c.query_row(
                    "SELECT fetched_at FROM meta WHERE source = ?1",
                    [&source],
                    |r| r.get::<_, i64>(0),
                )
                .ok()
            });
            let apps = updated.and(conn.as_ref()).and_then(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM packages WHERE source = ?1",
                    [&source],
                    |r| r.get::<_, i64>(0),
                )
                .ok()
                .map(|n| n as usize)
            });
            BucketRow {
                name,
                repo,
                apps,
                updated,
            }
        })
        .collect())
}

/// Full `bucket rm`: checks `state.db`, drops the `scoop:<name>` source from the catalog,
/// forgets its URL in `http.json` (so a later `add` refetches) and rewrites `search.idx`
/// (deleting it when that fails; search then falls back to SQLite). The caller saves `cfg`.
pub fn remove_and_cleanup(
    data: &crate::cache::DataDir,
    cfg: &mut Config,
    name: &str,
    state_db: &Path,
) -> Result<(), BucketError> {
    remove_and_cleanup_confirmed(data, cfg, name, state_db, false)
}

/// [`remove_and_cleanup`] under the update lock; `main` and `extras` need `confirmed`.
pub fn remove_and_cleanup_confirmed(
    data: &crate::cache::DataDir,
    cfg: &mut Config,
    name: &str,
    state_db: &Path,
    confirmed: bool,
) -> Result<(), BucketError> {
    let _lock = lock(data)?;
    if is_default_bucket(name) && !confirmed {
        return Err(BucketError::DefaultNeedsConfirmation(name.to_string()));
    }
    let repo = resolved(cfg)
        .ok()
        .and_then(|v| v.into_iter().find(|(n, _)| n == name).map(|(_, r)| r));
    remove(cfg, name, state_db)?;
    let cat_err = |e: crate::catalog::CatalogError| BucketError::Catalog(e.to_string());
    let catalog_path = data.catalog_path();
    if catalog_path.exists() {
        let mut catalog = crate::catalog::Catalog::open(&catalog_path).map_err(cat_err)?;
        let removed = catalog
            .remove_source(&crate::sources::scoop::source_name(name))
            .map_err(cat_err)?;
        let idx = data.search_index_path();
        // A bucket that was never downloaded has no rows: the index is still correct.
        if removed > 0 || !idx.exists() {
            let written = tracing::debug_span!("bucket.rm.index_write")
                .in_scope(|| crate::catalog::index_file::write(&catalog, &idx));
            if let Err(e) = written {
                tracing::warn!("falha ao regravar o índice de busca: {e}");
                let _ = std::fs::remove_file(&idx);
            }
        }
    }
    if crate::install::safepath::safe_components(name).is_some_and(|p| p.len() == 1) {
        let dir = data.buckets_dir().join(name);
        if dir.exists() {
            crate::fs_retry::retry("apagar", &dir, || std::fs::remove_dir_all(&dir))
                .map_err(|e| BucketError::State(e.to_string()))?;
        }
    }
    if let Some(repo) = repo {
        let path = data.http_state_path();
        let mut state = crate::cache::HttpState::load(&path);
        // Phase 1 stored `refs/heads/master` URLs; they would linger orphaned.
        let legacy = format!("https://codeload.github.com/{repo}/tar.gz/refs/heads/master");
        let url = crate::sources::scoop::bucket::repo_tarball_url(&repo);
        let current = state.entries.remove(&url).is_some();
        let old = state.entries.remove(&legacy).is_some();
        if current || old {
            state
                .save(&path)
                .map_err(|e| BucketError::State(e.to_string()))?;
        }
    }
    Ok(())
}
