//! HTTP client helpers: conditional GET with validators.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::cache::{HttpEntry, now_unix};

#[derive(Debug, Clone, PartialEq)]
pub enum Fetch {
    NotModified,
    Modified { body: Vec<u8>, entry: HttpEntry },
}

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("sem conexão: {0}")]
    Network(String),
    #[error("tempo esgotado")]
    Timeout,
    #[error("HTTP {0}")]
    Status(u16),
    #[error("resposta maior que o limite de {0} bytes")]
    TooLarge(usize),
    #[error("redirecionamento de https para http recusado: {0} (permita em Ajustes)")]
    InsecureRedirect(String),
}

impl HttpError {
    pub fn is_offline(&self) -> bool {
        matches!(self, HttpError::Network(_) | HttpError::Timeout)
    }
}

/// Only https -> http is refused, and only while `allow_http` is false.
pub fn redirect_allowed(prev: &reqwest::Url, next: &reqwest::Url, allow_http: bool) -> bool {
    allow_http || !(prev.scheme() == "https" && next.scheme() == "http")
}

/// Strict policy for installer downloads: only https -> https, whatever the global setting;
/// a plain-http origin (loopback only, see `winget_install::fetch`) follows no redirect.
pub fn strict_redirect_allowed(prev: &reqwest::Url, next: &reqwest::Url) -> bool {
    prev.scheme() == "https" && next.scheme() == "https"
}

/// Same hop limit as reqwest's default policy.
const MAX_REDIRECTS: usize = 10;

#[derive(Debug)]
struct InsecureRedirectError(String);

impl std::fmt::Display for InsecureRedirectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "insecure redirect to {}", self.0)
    }
}

impl std::error::Error for InsecureRedirectError {}

fn redirect_policy(allow_http: Arc<AtomicBool>) -> reqwest::redirect::Policy {
    checked_policy(move |prev, next| {
        redirect_allowed(prev, next, allow_http.load(Ordering::Relaxed))
    })
}

fn checked_policy(
    allowed: impl Fn(&reqwest::Url, &reqwest::Url) -> bool + Send + Sync + 'static,
) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        let allowed = match attempt.previous().last() {
            Some(prev) => allowed(prev, attempt.url()),
            None => true,
        };
        if allowed {
            attempt.follow()
        } else {
            let to = attempt.url().to_string();
            attempt.error(InsecureRedirectError(to))
        }
    })
}

fn map_err(e: reqwest::Error) -> HttpError {
    let mut src = std::error::Error::source(&e);
    while let Some(s) = src {
        if let Some(i) = s.downcast_ref::<InsecureRedirectError>() {
            return HttpError::InsecureRedirect(i.0.clone());
        }
        src = s.source();
    }
    if e.is_timeout() {
        HttpError::Timeout
    } else {
        HttpError::Network(e.to_string())
    }
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    /// No total timeout (big downloads); stalls are caught by `chunk_timeout`.
    dl_client: reqwest::Client,
    chunk_timeout: Duration,
    connect_timeout: Duration,
    allow_http_redirect: Arc<AtomicBool>,
}

impl Http {
    pub fn new() -> Result<Http, HttpError> {
        Http::with_timeouts(Duration::from_secs(10), Duration::from_secs(60))
    }

    pub fn with_timeouts(connect: Duration, total: Duration) -> Result<Http, HttpError> {
        let allow_http_redirect = Arc::new(AtomicBool::new(false));
        let client = reqwest::Client::builder()
            .redirect(redirect_policy(allow_http_redirect.clone()))
            .user_agent(concat!("wingetrs/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(connect)
            .timeout(total)
            .build()
            .map_err(|e| HttpError::Network(e.to_string()))?;
        let dl_client = reqwest::Client::builder()
            .redirect(redirect_policy(allow_http_redirect.clone()))
            .user_agent(concat!("wingetrs/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(connect)
            .build()
            .map_err(|e| HttpError::Network(e.to_string()))?;
        Ok(Http {
            client,
            dl_client,
            chunk_timeout: Duration::from_secs(30),
            connect_timeout: connect,
            allow_http_redirect,
        })
    }

    /// Whether redirects from https to http are followed (shared by all clones).
    pub fn set_allow_http_redirect(&self, allow: bool) {
        self.allow_http_redirect.store(allow, Ordering::Relaxed);
    }

    /// A copy whose downloads (`get_to_file`) follow only https -> https redirects, ignoring
    /// `allow_http_redirect` (see [`strict_redirect_allowed`]). Other requests are unchanged.
    pub fn with_strict_download_redirects(&self) -> Result<Http, HttpError> {
        let dl_client = reqwest::Client::builder()
            .redirect(checked_policy(strict_redirect_allowed))
            .user_agent(concat!("wingetrs/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(self.connect_timeout)
            .build()
            .map_err(|e| HttpError::Network(e.to_string()))?;
        Ok(Http {
            dl_client,
            ..self.clone()
        })
    }

    /// Max idle time waiting for headers or the next body chunk in `get_to_file`.
    pub fn with_chunk_timeout(mut self, d: Duration) -> Http {
        self.chunk_timeout = d;
        self
    }

    /// Sends If-None-Match / If-Modified-Since from `prev`; on 200 returns the new entry with checked_at = now.
    pub async fn get_conditional(
        &self,
        url: &str,
        prev: Option<&HttpEntry>,
    ) -> Result<Fetch, HttpError> {
        let mut req = self.client.get(url);
        if let Some(p) = prev {
            if let Some(etag) = &p.etag {
                req = req.header(reqwest::header::IF_NONE_MATCH, etag);
            }
            if let Some(lm) = &p.last_modified {
                req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm);
            }
        }
        let resp = req.send().await.map_err(map_err)?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(Fetch::NotModified);
        }
        if !status.is_success() {
            return Err(HttpError::Status(status.as_u16()));
        }
        let hv = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let entry = HttpEntry {
            etag: hv("etag"),
            last_modified: hv("last-modified"),
            source_version: hv("x-ms-meta-sourceversion"),
            checked_at: now_unix(),
        };
        let body = resp.bytes().await.map_err(map_err)?.to_vec();
        Ok(Fetch::Modified { body, entry })
    }

    pub async fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        let resp = self.client.get(url).send().await.map_err(map_err)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(HttpError::Status(status.as_u16()));
        }
        Ok(resp.bytes().await.map_err(map_err)?.to_vec())
    }

    /// Like [`Http::get`], but refuses bodies over `max` bytes: at once when `Content-Length`
    /// says so, otherwise as soon as the streamed total passes `max`.
    pub async fn get_limited(&self, url: &str, max: usize) -> Result<Vec<u8>, HttpError> {
        use futures_util::StreamExt;
        let resp = self.client.get(url).send().await.map_err(map_err)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(HttpError::Status(status.as_u16()));
        }
        if resp.content_length().is_some_and(|n| n > max as u64) {
            return Err(HttpError::TooLarge(max));
        }
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk.map_err(map_err)?);
            if body.len() > max {
                return Err(HttpError::TooLarge(max));
            }
        }
        Ok(body)
    }

    /// Streams the body of `url` into `path` (created/truncated), reporting `(done, total)`.
    /// The caller owns cleanup of `path` on error.
    pub async fn get_to_file(
        &self,
        url: &str,
        path: &std::path::Path,
        progress: impl Fn(u64, Option<u64>),
    ) -> Result<(), HttpError> {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;
        let idle = self.chunk_timeout;
        let resp = tokio::time::timeout(idle, self.dl_client.get(url).send())
            .await
            .map_err(|_| HttpError::Timeout)?
            .map_err(map_err)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(HttpError::Status(status.as_u16()));
        }
        let total = resp.content_length();
        let io = |e: std::io::Error| HttpError::Network(format!("E/S: {e}"));
        let mut file = tokio::fs::File::create(path).await.map_err(io)?;
        let mut done = 0u64;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = tokio::time::timeout(idle, stream.next())
            .await
            .map_err(|_| HttpError::Timeout)?
        {
            let chunk = chunk.map_err(map_err)?;
            file.write_all(&chunk).await.map_err(io)?;
            done += chunk.len() as u64;
            progress(done, total);
        }
        file.flush().await.map_err(io)?;
        Ok(())
    }
}
