//! Scoop source.
pub mod bucket;
pub mod manifest;

#[derive(Debug, thiserror::Error)]
pub enum ScoopError {
    #[error("bucket desconhecido: {0}")]
    UnknownBucket(String),
    #[error("tarball inválido: {0}")]
    Tar(String),
    #[error("manifesto scoop inválido: {0}")]
    Json(String),
    #[error(transparent)]
    Http(#[from] crate::http::HttpError),
}

/// Catalog source name for a bucket: `scoop:<bucket>`.
pub fn source_name(bucket: &str) -> String {
    format!("scoop:{bucket}")
}
