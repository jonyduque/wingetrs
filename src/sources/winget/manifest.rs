//! Winget merged manifest (`ManifestType: merged`) parsing.
use crate::core::{IconRef, InstallerSummary, PackageDetails, SourceKind};

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
    /// `install`, `uninstallPrevious` or `deny`; an installer's own value wins.
    pub upgrade_behavior: Option<String>,
    /// Upgraded only when asked for by name, never by "upgrade all"; an installer's own value wins.
    pub require_explicit_upgrade: Option<bool>,
    #[serde(default)]
    pub installers: Vec<Installer>,
    #[serde(default)]
    pub icons: Vec<Icon>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Icon {
    pub icon_url: String,
    pub icon_file_type: Option<String>,
    pub icon_resolution: Option<String>,
    pub icon_theme: Option<String>,
    pub icon_sha256: Option<String>,
}

/// Picks the icon to show: https URL and valid SHA-256 only; theme `default`
/// (or absent), then `dark`, then any other; ties go to the manifest order.
pub fn pick_icon(icons: &[Icon]) -> Option<IconRef> {
    let rank = |i: &Icon| match i.icon_theme.as_deref() {
        None => 0,
        Some(t) if t.eq_ignore_ascii_case("default") => 0,
        Some(t) if t.eq_ignore_ascii_case("dark") => 1,
        Some(_) => 2,
    };
    icons
        .iter()
        .filter(|i| i.icon_url.starts_with("https://"))
        .filter_map(|i| {
            let sha = i.icon_sha256.as_deref()?;
            (sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| (rank(i), i, sha.to_ascii_lowercase()))
        })
        .min_by_key(|(r, _, _)| *r)
        .map(|(_, i, sha256)| IconRef {
            url: i.icon_url.clone(),
            sha256,
        })
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Installer {
    pub architecture: String,
    pub installer_type: Option<String>,
    pub scope: Option<String>,
    pub upgrade_behavior: Option<String>,
    pub require_explicit_upgrade: Option<bool>,
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
            icon: pick_icon(&self.icons),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn icon(url: &str, theme: Option<&str>, sha: Option<&str>) -> Icon {
        Icon {
            icon_url: url.into(),
            icon_file_type: Some("ico".into()),
            icon_resolution: Some("custom".into()),
            icon_theme: theme.map(Into::into),
            icon_sha256: sha.map(Into::into),
        }
    }

    fn url_of(icons: &[Icon]) -> Option<String> {
        pick_icon(icons).map(|r| r.url)
    }

    #[test]
    fn pick_icon_prefers_default_then_dark() {
        let s = Some(SHA);
        let dark = icon("https://x/dark", Some("dark"), s);
        let light = icon("https://x/light", Some("light"), s);
        let def = icon("https://x/default", Some("Default"), s);
        let none = icon("https://x/none", None, s);
        assert_eq!(
            url_of(&[dark.clone(), light.clone(), def]).as_deref(),
            Some("https://x/default")
        );
        assert_eq!(
            url_of(&[light.clone(), dark]).as_deref(),
            Some("https://x/dark")
        );
        assert_eq!(url_of(&[light]).as_deref(), Some("https://x/light"));
        assert_eq!(url_of(&[none]).as_deref(), Some("https://x/none"));
    }

    #[test]
    fn pick_icon_requires_https_and_a_valid_sha() {
        let u = "https://x/i.ico";
        assert_eq!(pick_icon(&[icon(u, None, None)]), None);
        assert_eq!(pick_icon(&[icon(u, None, Some(&SHA[..63]))]), None);
        let non_hex = format!("g{}", &SHA[1..]);
        assert_eq!(pick_icon(&[icon(u, None, Some(&non_hex))]), None);
        assert_eq!(pick_icon(&[icon("http://x/i.ico", None, Some(SHA))]), None);
        let upper = SHA.to_ascii_uppercase();
        let r = pick_icon(&[icon(u, None, Some(&upper))]).unwrap();
        assert_eq!(r.sha256, SHA);
    }

    #[test]
    fn manifest_with_icons_parses() {
        let yaml = format!(
            "PackageIdentifier: Notepad++.Notepad++\nPackageVersion: 8.9\nIcons:\n- IconUrl: https://x/n.ico\n  IconFileType: ico\n  IconResolution: custom\n  IconTheme: default\n  IconSha256: {SHA}\n"
        );
        let d = parse(yaml.as_bytes()).unwrap().into_details(vec![]);
        assert_eq!(
            d.icon,
            Some(IconRef {
                url: "https://x/n.ico".into(),
                sha256: SHA.into()
            })
        );
        let bare = "PackageIdentifier: A.B\nPackageVersion: 1\n";
        assert_eq!(
            parse(bare.as_bytes()).unwrap().into_details(vec![]).icon,
            None
        );
    }
}
