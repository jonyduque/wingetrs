//! Shared core types.
pub mod version;

use serde::Serialize;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Winget,
    Scoop,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Winget => "winget",
            SourceKind::Scoop => "scoop",
        }
    }
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SourceKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "winget" => Ok(SourceKind::Winget),
            "scoop" => Ok(SourceKind::Scoop),
            _ => Err(format!("fonte desconhecida: {s}")),
        }
    }
}

/// One row of the unified catalog, as loaded for search.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackageEntry {
    pub source: SourceKind,
    /// "winget" or "scoop:<bucket>"
    pub source_name: String,
    pub id: String,
    pub name: String,
    pub moniker: Option<String>,
    pub version: String,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InstallerSummary {
    pub architecture: String,
    pub installer_type: String,
    pub scope: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PackageDetails {
    pub source: SourceKind,
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: Option<String>,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub tags: Vec<String>,
    pub installers: Vec<InstallerSummary>,
    /// Newest first.
    pub available_versions: Vec<String>,
    /// Package icon (winget only); never serialized, the CLI output is unchanged.
    #[serde(skip)]
    pub icon: Option<IconRef>,
}

/// Where to fetch a package icon and the SHA-256 its bytes must have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconRef {
    pub url: String,
    pub sha256: String,
}
