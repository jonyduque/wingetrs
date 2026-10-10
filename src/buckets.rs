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
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Normalizes `owner/repo` or a GitHub URL (optional `.git`, trailing slash) to `owner/repo`.
pub fn parse_repo(spec: &str) -> Result<String, BucketError> {
    let spec = spec.trim();
    let path = if let Some((_, rest)) = spec.split_once("://") {
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

/// Apps in `state.db` installed from `bucket`; empty when the file is missing or unreadable.
pub fn installed_from_bucket(state_db: &Path, bucket: &str) -> Vec<String> {
    if !state_db.exists() {
        return Vec::new();
    }
    let Ok(conn) =
        rusqlite::Connection::open_with_flags(state_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare("SELECT app FROM apps WHERE bucket = ?1 ORDER BY app") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([bucket], |r| r.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.filter_map(Result::ok).collect()
}

/// `rm`: refuses while apps from the bucket are installed. Config only; see `remove_from_catalog`.
pub fn remove(cfg: &mut Config, name: &str, state_db: &Path) -> Result<(), BucketError> {
    let configured =
        cfg.scoop_buckets.iter().any(|n| n == name) || cfg.buckets.iter().any(|e| e.name == name);
    if !configured {
        return Err(BucketError::NotConfigured(name.to_string()));
    }
    let apps = installed_from_bucket(state_db, name);
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

/// Drops the `scoop:<name>` source (packages, map rows, meta) from the catalog database.
pub fn remove_from_catalog(catalog_path: &Path, name: &str) -> Result<(), BucketError> {
    if !catalog_path.exists() {
        return Ok(());
    }
    let source = crate::sources::scoop::source_name(name);
    let run = || -> Result<(), rusqlite::Error> {
        let mut conn = rusqlite::Connection::open(catalog_path)?;
        let tx = conn.transaction()?;
        for t in [
            "map_productcode",
            "map_upgradecode",
            "map_pfn",
            "map_norm_name",
            "map_norm_publisher",
        ] {
            tx.execute(
                &format!(
                    "DELETE FROM {t} WHERE package IN (SELECT rowid FROM packages WHERE source = ?1)"
                ),
                [&source],
            )?;
        }
        tx.execute("DELETE FROM packages WHERE source = ?1", [&source])?;
        tx.execute("DELETE FROM meta WHERE source = ?1", [&source])?;
        tx.commit()
    };
    run().map_err(|e| BucketError::Catalog(e.to_string()))
}
