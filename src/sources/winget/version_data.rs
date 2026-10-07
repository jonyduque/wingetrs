//! Parsing of the decoded `versionData` YAML.
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub struct VersionDataEntry {
    pub version: String,
    pub rel_path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VersionData {
    pub entries: Vec<VersionDataEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum VersionDataError {
    #[error(transparent)]
    Mszip(#[from] super::mszip::MszipError),
    #[error("versionData não é UTF-8")]
    Utf8,
    #[error("versionData inválido: {0}")]
    Yaml(String),
}

#[derive(Deserialize)]
struct RawDoc {
    #[serde(rename = "vD")]
    entries: Vec<RawEntry>,
}

#[derive(Deserialize)]
struct RawEntry {
    #[serde(rename = "v")]
    version: String,
    #[serde(rename = "rP")]
    rel_path: String,
    #[serde(rename = "s256H")]
    sha256: String,
}

pub fn parse(yaml: &str) -> Result<VersionData, VersionDataError> {
    let doc: RawDoc =
        serde_saphyr::from_str(yaml).map_err(|e| VersionDataError::Yaml(e.to_string()))?;
    Ok(VersionData {
        entries: doc
            .entries
            .into_iter()
            .map(|e| VersionDataEntry {
                version: e.version,
                rel_path: e.rel_path,
                sha256: e.sha256,
            })
            .collect(),
    })
}

pub fn decode_and_parse(mszyml: &[u8]) -> Result<VersionData, VersionDataError> {
    let bytes = super::mszip::decode(mszyml)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| VersionDataError::Utf8)?;
    parse(text)
}

/// "packages/<id>/<hex8>/versionData.mszyml" (relative to CDN_BASE)
pub fn version_data_path(id: &str, hash: &[u8]) -> String {
    let hex8 = hex::encode(&hash[..hash.len().min(4)]);
    format!("packages/{id}/{hex8}/versionData.mszyml")
}
