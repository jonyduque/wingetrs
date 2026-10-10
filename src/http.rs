//! HTTP client helpers: conditional GET with validators.

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
}

impl HttpError {
    pub fn is_offline(&self) -> bool {
        matches!(self, HttpError::Network(_) | HttpError::Timeout)
    }
}

fn map_err(e: reqwest::Error) -> HttpError {
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
}

impl Http {
    pub fn new() -> Result<Http, HttpError> {
        Http::with_timeouts(Duration::from_secs(10), Duration::from_secs(60))
    }

    pub fn with_timeouts(connect: Duration, total: Duration) -> Result<Http, HttpError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("wingetrs/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(connect)
            .timeout(total)
            .build()
            .map_err(|e| HttpError::Network(e.to_string()))?;
        let dl_client = reqwest::Client::builder()
            .user_agent(concat!("wingetrs/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(connect)
            .build()
            .map_err(|e| HttpError::Network(e.to_string()))?;
        Ok(Http {
            client,
            dl_client,
            chunk_timeout: Duration::from_secs(30),
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
