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
    /// The server's certificate chain does not end in a trusted root.
    #[error("certificado do servidor não confiável: {0}")]
    UntrustedCertificate(String),
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

/// The error the causes of a request error (`e` itself excluded) stand for, when one is
/// recognized: a refused insecure redirect or an untrusted certificate chain.
pub fn classify_causes(e: &(dyn std::error::Error + 'static)) -> Option<HttpError> {
    let mut src = e.source();
    while let Some(s) = src {
        if let Some(i) = s.downcast_ref::<InsecureRedirectError>() {
            return Some(HttpError::InsecureRedirect(i.0.clone()));
        }
        // Text of rustls's `Error::InvalidCertificate` Display (0.23.45, locked in Cargo.lock).
        let text = s.to_string();
        if text.contains("invalid peer certificate") {
            return Some(HttpError::UntrustedCertificate(text));
        }
        src = s.source();
    }
    None
}

fn map_err(e: reqwest::Error) -> HttpError {
    if let Some(known) = classify_causes(&e) {
        return known;
    }
    if e.is_timeout() {
        HttpError::Timeout
    } else {
        HttpError::Network(e.to_string())
    }
}

/// How often a download waiting for its next chunk looks at its stop signal.
const STOP_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// A reply as [`Http::get_limited_raw`] returns it; header names are lower case.
#[derive(Debug, Clone)]
pub struct RawReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RawReply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
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
    /// [`Http::for_elevated`]: no proxy and only these roots, on every client it builds.
    pinned_roots: Option<Arc<Vec<reqwest::Certificate>>>,
}

/// No proxy (neither the environment's nor the system's: both are the user's to set) and
/// `roots` as the only trusted roots, when given.
fn lock_down(
    b: reqwest::ClientBuilder,
    roots: Option<&Arc<Vec<reqwest::Certificate>>>,
) -> reqwest::ClientBuilder {
    match roots {
        Some(r) => b.no_proxy().tls_certs_only(r.iter().cloned()),
        None => b,
    }
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
            pinned_roots: None,
        })
    }

    /// The client of code that runs elevated (`self apply-elevated`): no proxy at all and only
    /// `roots` (DER, read from the machine's certificate stores, which only an administrator
    /// writes) as trusted roots, instead of `Http::new`'s user proxy and the platform verifier
    /// (which trusts `CurrentUser\Root`). Roots that rustls cannot use are skipped; none left
    /// is an error. Same timeouts and redirect policies as [`Http::new`].
    pub fn for_elevated(roots: &[Vec<u8>]) -> Result<Http, HttpError> {
        let usable: Vec<reqwest::Certificate> = roots
            .iter()
            .filter_map(|der| reqwest::Certificate::from_der(der).ok())
            .filter(|c| {
                reqwest::Client::builder()
                    .no_proxy()
                    .tls_certs_only([c.clone()])
                    .build()
                    .is_ok()
            })
            .collect();
        if usable.is_empty() {
            return Err(HttpError::UntrustedCertificate(
                "nenhuma raiz utilizável no repositório de certificados da máquina".into(),
            ));
        }
        let roots = Some(Arc::new(usable));
        let (connect, total) = (Duration::from_secs(10), Duration::from_secs(60));
        let allow_http_redirect = Arc::new(AtomicBool::new(false));
        let client = lock_down(reqwest::Client::builder(), roots.as_ref())
            .redirect(redirect_policy(allow_http_redirect.clone()))
            .user_agent(concat!("wingetrs/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(connect)
            .timeout(total)
            .build()
            .map_err(|e| HttpError::Network(e.to_string()))?;
        let dl_client = lock_down(reqwest::Client::builder(), roots.as_ref())
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
            pinned_roots: roots,
        })
    }

    /// Whether any client of this `Http` may go through a proxy (explicit or the system's).
    pub fn may_use_proxy(&self) -> bool {
        // reqwest lists `proxies` in a client's `Debug` exactly when it has any, the system
        // one included.
        [&self.client, &self.dl_client]
            .iter()
            .any(|c| format!("{c:?}").contains("proxies"))
    }

    /// Whether the trusted roots are only the ones given to [`Http::for_elevated`].
    pub fn pinned_roots(&self) -> Option<usize> {
        self.pinned_roots.as_ref().map(|r| r.len())
    }

    /// Whether redirects from https to http are followed (shared by all clones).
    pub fn set_allow_http_redirect(&self, allow: bool) {
        self.allow_http_redirect.store(allow, Ordering::Relaxed);
    }

    /// A copy whose downloads (`get_to_file`) follow only https -> https redirects, ignoring
    /// `allow_http_redirect` (see [`strict_redirect_allowed`]). Other requests are unchanged.
    pub fn with_strict_download_redirects(&self) -> Result<Http, HttpError> {
        let dl_client = lock_down(reqwest::Client::builder(), self.pinned_roots.as_ref())
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
        self.get_conditional_progress(url, prev, |_, _| {}).await
    }

    /// [`Http::get_conditional`] reporting `(done, total)` as the body streams in.
    pub async fn get_conditional_progress(
        &self,
        url: &str,
        prev: Option<&HttpEntry>,
        progress: impl Fn(u64, Option<u64>),
    ) -> Result<Fetch, HttpError> {
        use futures_util::StreamExt;
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
        let total = resp.content_length();
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk.map_err(map_err)?);
            progress(body.len() as u64, total);
        }
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
        let reply = self.get_limited_raw(url, max).await?;
        if !(200..300).contains(&reply.status) {
            return Err(HttpError::Status(reply.status));
        }
        Ok(reply.body)
    }

    /// [`Http::get_limited`] that hands back a non-success reply (status and headers, empty
    /// body) instead of an error, for callers that read rate-limit headers.
    pub async fn get_limited_raw(&self, url: &str, max: usize) -> Result<RawReply, HttpError> {
        use futures_util::StreamExt;
        let resp = self.client.get(url).send().await.map_err(map_err)?;
        let status = resp.status();
        let headers = resp
            .headers()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
            .collect();
        if !status.is_success() {
            return Ok(RawReply {
                status: status.as_u16(),
                headers,
                body: Vec::new(),
            });
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
        Ok(RawReply {
            status: status.as_u16(),
            headers,
            body,
        })
    }

    /// GET that follows redirects and returns the final URL without reading the body.
    pub async fn final_url(&self, url: &str) -> Result<String, HttpError> {
        let resp = self.client.get(url).send().await.map_err(map_err)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(HttpError::Status(status.as_u16()));
        }
        Ok(resp.url().to_string())
    }

    /// Streams the body of `url` into `path` (created/truncated), reporting `(done, total)`.
    /// The caller owns cleanup of `path` on error.
    pub async fn get_to_file(
        &self,
        url: &str,
        path: &std::path::Path,
        progress: impl Fn(u64, Option<u64>),
    ) -> Result<(), HttpError> {
        self.get_to_file_until(url, path, progress, &|| false)
            .await
            .map(|_| ())
    }

    /// [`Http::get_to_file`] that stops once `stop()` says so: looked at between chunks and
    /// every `STOP_POLL` while waiting for one. The file is flushed and closed before
    /// returning (no write left in flight), so the caller can delete it. `Ok(false)` =
    /// stopped before the end.
    pub async fn get_to_file_until(
        &self,
        url: &str,
        path: &std::path::Path,
        progress: impl Fn(u64, Option<u64>),
        stop: &(dyn Fn() -> bool + Sync),
    ) -> Result<bool, HttpError> {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;
        let idle = self.chunk_timeout;
        let stopped = || async {
            while !stop() {
                tokio::time::sleep(STOP_POLL).await;
            }
        };
        let resp = tokio::select! {
            biased;
            // Stopped before the answer: nothing was written.
            () = stopped() => return Ok(false),
            resp = tokio::time::timeout(idle, self.dl_client.get(url).send()) => {
                resp.map_err(|_| HttpError::Timeout)?.map_err(map_err)?
            }
        };
        let status = resp.status();
        if !status.is_success() {
            return Err(HttpError::Status(status.as_u16()));
        }
        let total = resp.content_length();
        let io = |e: std::io::Error| HttpError::Network(format!("E/S: {e}"));
        let mut file = tokio::fs::File::create(path).await.map_err(io)?;
        let mut done = 0u64;
        let mut stream = resp.bytes_stream();
        loop {
            let next = tokio::select! {
                biased;
                () = stopped() => None,
                next = tokio::time::timeout(idle, stream.next()) => {
                    Some(next.map_err(|_| HttpError::Timeout)?)
                }
            };
            let chunk = match next {
                // Stopped: the body stream is dropped between chunks (nothing half-written).
                None => {
                    file.flush().await.map_err(io)?;
                    return Ok(false);
                }
                Some(None) => break,
                Some(Some(chunk)) => chunk.map_err(map_err)?,
            };
            file.write_all(&chunk).await.map_err(io)?;
            done += chunk.len() as u64;
            progress(done, total);
        }
        file.flush().await.map_err(io)?;
        Ok(true)
    }
}
