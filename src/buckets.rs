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
    #[error("o bucket {name} ({repo}) não é conhecido; confirme para adicioná-lo")]
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

/// Full `bucket rm`: checks `state.db`, drops the `scoop:<name>` source from the catalog,
/// forgets its URL in `http.json` (so a later `add` refetches) and rewrites `search.idx`
/// (deleting it when that fails; search then falls back to SQLite). The caller saves `cfg`.
/// Stale `refs/heads/master` URLs from phase 1 stay orphaned in `http.json` (no effect).
pub fn remove_and_cleanup(
    data: &crate::cache::DataDir,
    cfg: &mut Config,
    name: &str,
    state_db: &Path,
) -> Result<(), BucketError> {
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
    if let Some(repo) = repo {
        let path = data.http_state_path();
        let mut state = crate::cache::HttpState::load(&path);
        let url = crate::sources::scoop::bucket::repo_tarball_url(&repo);
        if state.entries.remove(&url).is_some() {
            state
                .save(&path)
                .map_err(|e| BucketError::State(e.to_string()))?;
        }
    }
    Ok(())
}
