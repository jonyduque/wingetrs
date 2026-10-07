//! Winget merged manifest (`ManifestType: merged`) parsing.
use crate::core::{InstallerSummary, PackageDetails, SourceKind};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MergedManifest {
    pub package_identifier: String,
    pub package_version: String,
    pub package_name: Option<String>,
    pub publisher: Option<String>,
    pub short_description: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub package_url: Option<String>,
    pub publisher_url: Option<String>,
    pub moniker: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub installer_type: Option<String>,
    pub scope: Option<String>,
    #[serde(default)]
    pub installers: Vec<Installer>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Installer {
    pub architecture: String,
    pub installer_type: Option<String>,
    pub scope: Option<String>,
    pub installer_url: String,
    pub installer_sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifesto não é UTF-8")]
    Utf8,
    #[error("manifesto inválido: {0}")]
    Yaml(String),
}

pub fn parse(bytes: &[u8]) -> Result<MergedManifest, ManifestError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ManifestError::Utf8)?;
    serde_saphyr::from_str(text).map_err(|e| ManifestError::Yaml(e.to_string()))
}

impl MergedManifest {
    pub fn into_details(self, available_versions: Vec<String>) -> PackageDetails {
        let root_type = self.installer_type;
        let root_scope = self.scope;
        let installers = self
            .installers
            .into_iter()
            .map(|i| InstallerSummary {
                architecture: i.architecture,
                installer_type: i
                    .installer_type
                    .or(root_type.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
                scope: i.scope.or(root_scope.clone()),
                url: i.installer_url,
            })
            .collect();
        PackageDetails {
            source: SourceKind::Winget,
            name: self
                .package_name
                .unwrap_or_else(|| self.package_identifier.clone()),
            id: self.package_identifier,
            version: self.package_version,
            publisher: self.publisher,
            description: self.short_description.or(self.description),
            homepage: self.package_url.or(self.publisher_url),
            license: self.license,
            tags: self.tags,
            installers,
            available_versions,
        }
    }
}
