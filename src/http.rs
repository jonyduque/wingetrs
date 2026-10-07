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
        Ok(Http { client })
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
}
