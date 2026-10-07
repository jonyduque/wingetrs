//! Winget source.
pub mod details;
pub mod index;
pub mod manifest;
pub mod mszip;
pub mod version_data;

pub const CDN_BASE: &str = "https://cdn.winget.microsoft.com/cache/";
pub const SOURCE2_URL: &str = "https://cdn.winget.microsoft.com/cache/source2.msix";
pub const SOURCE_NAME: &str = "winget";

#[derive(Debug, thiserror::Error)]
pub enum WingetError {
    #[error(transparent)]
    Http(#[from] crate::http::HttpError),
    #[error(transparent)]
    Index(#[from] index::IndexError),
    #[error(transparent)]
    VersionData(#[from] version_data::VersionDataError),
    #[error(transparent)]
    Manifest(#[from] manifest::ManifestError),
    #[error(transparent)]
    Cache(#[from] crate::cache::CacheError),
    #[error("versão {0} não encontrada")]
    VersionNotFound(String),
    #[error("manifesto com hash divergente")]
    ManifestIntegrity,
}

impl WingetError {
    /// True when the failure is a network problem (not integrity/format).
    pub fn is_offline(&self) -> bool {
        matches!(self, WingetError::Http(e) if e.is_offline())
    }
}

// Interface fixed by the brief; the value is short-lived, boxing buys nothing.
#[allow(clippy::large_enum_variant)]
pub enum WingetFetch {
    NotModified,
    Modified {
        import: index::WingetImport,
        entry: crate::cache::HttpEntry,
    },
}

/// Conditional download of source2.msix, integrity validation and index.db read.
/// `url` is SOURCE2_URL in production, the mock server in tests.
pub async fn fetch_index(
    http: &crate::http::Http,
    data: &crate::cache::DataDir,
    url: &str,
    prev: Option<&crate::cache::HttpEntry>,
) -> Result<WingetFetch, WingetError> {
    let (body, entry) = match http.get_conditional(url, prev).await? {
        crate::http::Fetch::NotModified => return Ok(WingetFetch::NotModified),
        crate::http::Fetch::Modified { body, entry } => (body, entry),
    };
    let db = index::validate_msix(&body)?;
    let tmp = data
        .tmp_dir()
        .join(format!("index-{}.db", std::process::id()));
    let result = std::fs::write(&tmp, &db)
        .map_err(|e| index::IndexError::Io(e.to_string()))
        .and_then(|()| index::read_index(&tmp));
    let _ = std::fs::remove_file(&tmp);
    Ok(WingetFetch::Modified {
        import: result?,
        entry,
    })
}
