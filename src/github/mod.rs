//! GitHub releases client: assets with their SHA-256 `digest`, and the latest tag through
//! the web redirect (which does not spend the API rate limit).

use serde::Deserialize;

use crate::http::{Http, HttpError};

/// Ceiling for the release JSON.
const MAX_JSON: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum GhError {
    #[error("sem conexão com o GitHub")]
    Offline,
    #[error("repositório ou release não encontrado")]
    NotFound,
    #[error("limite de requisições do GitHub atingido (renova em {reset:?})")]
    RateLimited { reset: Option<u64> },
    #[error("o repositório não tem release publicada")]
    NoRelease,
    #[error("resposta do GitHub ilegível")]
    BadJson,
    #[error("GitHub respondeu HTTP {0}")]
    Status(u16),
    #[error("falha ao falar com o GitHub: {0}")]
    Other(String),
    #[error("nome de repositório inválido: {0}")]
    InvalidRepo(String),
    #[error("{0}")]
    UntrustedCertificate(String),
}

impl GhError {
    fn from_http(e: HttpError) -> GhError {
        match e {
            e if e.is_offline() => GhError::Offline,
            HttpError::Status(404) => GhError::NotFound,
            HttpError::Status(n) => GhError::Status(n),
            HttpError::TooLarge(_) => GhError::BadJson,
            HttpError::UntrustedCertificate(m) => GhError::UntrustedCertificate(m),
            other => GhError::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhRepo {
    pub owner: String,
    pub repo: String,
}

fn valid_part(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl GhRepo {
    /// Same name rules as `buckets::parse_repo`, for the plain `owner/repo` form.
    pub fn parse(spec: &str) -> Result<GhRepo, GhError> {
        match spec.trim().split_once('/') {
            Some((o, r)) if valid_part(o) && valid_part(r) => Ok(GhRepo {
                owner: o.to_string(),
                repo: r.to_string(),
            }),
            _ => Err(GhError::InvalidRepo(spec.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub sha256: [u8; 32],
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub prerelease: bool,
    pub immutable: bool,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: Option<String>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    immutable: bool,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: Option<String>,
    size: Option<u64>,
    digest: Option<String>,
    browser_download_url: Option<String>,
}

/// Closed list of download origins, checked on the parsed URL (no string prefixes).
fn download_url_allowed(repo: &GhRepo, url: &str) -> bool {
    if url.contains('\\') {
        return false;
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https"
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return false;
    }
    if parsed.path().contains('%') {
        return false;
    }
    let Some(segments) = parsed.path_segments() else {
        return false;
    };
    let segments: Vec<&str> = segments.collect();
    if segments
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == "..")
    {
        return false;
    }
    match parsed.host_str() {
        Some("github.com") => {
            segments.len() == 6
                && segments[0].eq_ignore_ascii_case(&repo.owner)
                && segments[1].eq_ignore_ascii_case(&repo.repo)
                && segments[2] == "releases"
                && segments[3] == "download"
        }
        Some("objects.githubusercontent.com") => true,
        _ => false,
    }
}

fn parse_digest(digest: &str) -> Option<[u8; 32]> {
    let hex_part = digest.strip_prefix("sha256:")?;
    if hex_part.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    hex::decode_to_slice(hex_part, &mut out).ok()?;
    Some(out)
}

fn convert_asset(repo: &GhRepo, a: RawAsset) -> Option<Asset> {
    let name = a.name.filter(|n| !n.is_empty())?;
    let Some(sha256) = a.digest.as_deref().and_then(parse_digest) else {
        tracing::debug!(asset = %name, "release asset dropped: no valid sha256 digest");
        return None;
    };
    let Some(url) = a
        .browser_download_url
        .filter(|u| download_url_allowed(repo, u))
    else {
        tracing::debug!(asset = %name, "release asset dropped: download host not allowed");
        return None;
    };
    Some(Asset {
        name,
        size: a.size?,
        sha256,
        url,
    })
}

fn parse_release(repo: &GhRepo, body: &[u8]) -> Result<Release, GhError> {
    let raw: RawRelease = serde_json::from_slice(body).map_err(|_| GhError::BadJson)?;
    let tag = raw.tag_name.filter(|t| !t.is_empty());
    let Some(tag) = tag.filter(|_| !raw.draft) else {
        return Err(GhError::NoRelease);
    };
    Ok(Release {
        tag,
        prerelease: raw.prerelease,
        immutable: raw.immutable,
        assets: raw
            .assets
            .into_iter()
            .filter_map(|a| convert_asset(repo, a))
            .collect(),
    })
}

async fn fetch_release(http: &Http, url: &str, repo: &GhRepo) -> Result<Release, GhError> {
    let reply = http
        .get_limited_raw(url, MAX_JSON)
        .await
        .map_err(GhError::from_http)?;
    match reply.status {
        200..=299 => parse_release(repo, &reply.body),
        404 => Err(GhError::NotFound),
        403 | 429 if reply.header("x-ratelimit-remaining") == Some("0") => {
            Err(GhError::RateLimited {
                reset: reply
                    .header("x-ratelimit-reset")
                    .and_then(|v| v.trim().parse().ok()),
            })
        }
        n => Err(GhError::Status(n)),
    }
}

/// `GET {base}/repos/{o}/{r}/releases/latest`.
pub async fn latest(http: &Http, base: &str, repo: &GhRepo) -> Result<Release, GhError> {
    let url = format!(
        "{}/repos/{}/{}/releases/latest",
        base.trim_end_matches('/'),
        repo.owner,
        repo.repo
    );
    fetch_release(http, &url, repo).await
}

/// `GET {base}/repos/{o}/{r}/releases/tags/{tag}`.
pub async fn by_tag(http: &Http, base: &str, repo: &GhRepo, tag: &str) -> Result<Release, GhError> {
    let url = format!(
        "{}/repos/{}/{}/releases/tags/{}",
        base.trim_end_matches('/'),
        repo.owner,
        repo.repo,
        percent_encode(tag)
    );
    fetch_release(http, &url, repo).await
}

/// Latest tag through `{base_web}/{o}/{r}/releases/latest`, which redirects to
/// `/releases/tag/<tag>` and does not count against the API limit.
pub async fn latest_tag(http: &Http, base_web: &str, repo: &GhRepo) -> Result<String, GhError> {
    let url = format!(
        "{}/{}/{}/releases/latest",
        base_web.trim_end_matches('/'),
        repo.owner,
        repo.repo
    );
    let fin = http.final_url(&url).await.map_err(GhError::from_http)?;
    let path = fin.split(['?', '#']).next().unwrap_or("");
    let (_, encoded) = path
        .rsplit_once("/releases/tag/")
        .ok_or(GhError::NoRelease)?;
    let tag = percent_decode(encoded).ok_or(GhError::NoRelease)?;
    if tag.is_empty()
        || tag.contains(['/', '\\', '%'])
        || tag.contains("..")
        || tag.chars().any(char::is_control)
    {
        return Err(GhError::NoRelease);
    }
    Ok(tag)
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let h = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}
