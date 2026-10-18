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
    pub nested_installer_type: Option<String>,
    #[serde(default)]
    pub nested_installer_files: Option<Vec<NestedInstallerFile>>,
    #[serde(default)]
    pub installer_locale: Option<String>,
    #[serde(default)]
    pub platform: Option<Vec<String>>,
    #[serde(default, rename = "MinimumOSVersion")]
    pub minimum_os_version: Option<String>,
    #[serde(default)]
    pub install_modes: Option<Vec<String>>,
    #[serde(default)]
    pub installer_switches: Option<InstallerSwitches>,
    #[serde(default)]
    pub installer_success_codes: Option<Vec<i64>>,
    #[serde(default)]
    pub expected_return_codes: Option<Vec<ExpectedReturnCode>>,
    #[serde(default)]
    pub elevation_requirement: Option<String>,
    #[serde(default)]
    pub product_code: Option<String>,
    #[serde(default)]
    pub package_family_name: Option<String>,
    #[serde(default)]
    pub apps_and_features_entries: Option<Vec<ArpEntry>>,
    #[serde(default)]
    pub dependencies: Option<Dependencies>,
    #[serde(default)]
    pub agreements: Vec<Agreement>,
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
    #[serde(default)]
    pub nested_installer_type: Option<String>,
    #[serde(default)]
    pub nested_installer_files: Option<Vec<NestedInstallerFile>>,
    #[serde(default)]
    pub installer_locale: Option<String>,
    #[serde(default)]
    pub platform: Option<Vec<String>>,
    #[serde(default, rename = "MinimumOSVersion")]
    pub minimum_os_version: Option<String>,
    #[serde(default)]
    pub install_modes: Option<Vec<String>>,
    #[serde(default)]
    pub installer_switches: Option<InstallerSwitches>,
    #[serde(default)]
    pub installer_success_codes: Option<Vec<i64>>,
    #[serde(default)]
    pub expected_return_codes: Option<Vec<ExpectedReturnCode>>,
    #[serde(default)]
    pub elevation_requirement: Option<String>,
    #[serde(default)]
    pub product_code: Option<String>,
    #[serde(default)]
    pub package_family_name: Option<String>,
    #[serde(default)]
    pub apps_and_features_entries: Option<Vec<ArpEntry>>,
    #[serde(default)]
    pub dependencies: Option<Dependencies>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct InstallerSwitches {
    #[serde(default)]
    pub silent: Option<String>,
    #[serde(default)]
    pub silent_with_progress: Option<String>,
    #[serde(default)]
    pub interactive: Option<String>,
    #[serde(default)]
    pub install_location: Option<String>,
    #[serde(default)]
    pub log: Option<String>,
    #[serde(default)]
    pub upgrade: Option<String>,
    #[serde(default)]
    pub custom: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ExpectedReturnCode {
    #[serde(default)]
    pub installer_return_code: i64,
    #[serde(default)]
    pub return_response: String,
    #[serde(default)]
    pub return_response_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PackageDependency {
    #[serde(default)]
    pub package_identifier: String,
    #[serde(default)]
    pub minimum_version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Dependencies {
    #[serde(default)]
    pub windows_features: Vec<String>,
    #[serde(default)]
    pub windows_libraries: Vec<String>,
    #[serde(default)]
    pub package_dependencies: Vec<PackageDependency>,
    #[serde(default)]
    pub external_dependencies: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Agreement {
    #[serde(default)]
    pub agreement_label: Option<String>,
    #[serde(default)]
    pub agreement: Option<String>,
    #[serde(default)]
    pub agreement_url: Option<String>,
}

/// One `AppsAndFeaturesEntries` item.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ArpEntry {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub display_version: Option<String>,
    #[serde(default)]
    pub product_code: Option<String>,
    #[serde(default)]
    pub upgrade_code: Option<String>,
    #[serde(default)]
    pub installer_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NestedInstallerFile {
    #[serde(default)]
    pub relative_file_path: String,
    #[serde(default)]
    pub portable_command_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallerKind {
    Msi,
    Wix,
    Burn,
    Inno,
    Nullsoft,
    Msix,
    Appx,
    Exe,
    Zip,
    Portable,
    Font,
    Pwa,
    Msstore,
    Unknown(String),
}

impl InstallerKind {
    /// Case-insensitive; `None` or empty yields `Unknown("")`.
    pub fn parse(s: Option<&str>) -> InstallerKind {
        let raw = s.unwrap_or("");
        match raw.to_ascii_lowercase().as_str() {
            "msi" => InstallerKind::Msi,
            "wix" => InstallerKind::Wix,
            "burn" => InstallerKind::Burn,
            "inno" => InstallerKind::Inno,
            "nullsoft" => InstallerKind::Nullsoft,
            "msix" => InstallerKind::Msix,
            "appx" => InstallerKind::Appx,
            "exe" => InstallerKind::Exe,
            "zip" => InstallerKind::Zip,
            "portable" => InstallerKind::Portable,
            "font" => InstallerKind::Font,
            "pwa" => InstallerKind::Pwa,
            "msstore" => InstallerKind::Msstore,
            _ => InstallerKind::Unknown(raw.to_string()),
        }
    }

    /// Lowercase name as written in manifests.
    pub fn name(&self) -> &str {
        match self {
            InstallerKind::Msi => "msi",
            InstallerKind::Wix => "wix",
            InstallerKind::Burn => "burn",
            InstallerKind::Inno => "inno",
            InstallerKind::Nullsoft => "nullsoft",
            InstallerKind::Msix => "msix",
            InstallerKind::Appx => "appx",
            InstallerKind::Exe => "exe",
            InstallerKind::Zip => "zip",
            InstallerKind::Portable => "portable",
            InstallerKind::Font => "font",
            InstallerKind::Pwa => "pwa",
            InstallerKind::Msstore => "msstore",
            InstallerKind::Unknown(s) => s,
        }
    }
}

/// An installer with every root-level default already applied.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveInstaller {
    pub architecture: String,
    pub kind: InstallerKind,
    pub nested_kind: Option<InstallerKind>,
    pub nested_files: Vec<NestedInstallerFile>,
    pub scope: Option<String>,
    pub locale: Option<String>,
    pub platform: Vec<String>,
    pub minimum_os_version: Option<String>,
    pub install_modes: Vec<String>,
    pub switches: InstallerSwitches,
    pub success_codes: Vec<i64>,
    pub expected_codes: Vec<ExpectedReturnCode>,
    pub upgrade_behavior: Option<String>,
    pub require_explicit_upgrade: bool,
    pub elevation: Option<String>,
    pub product_code: Option<String>,
    pub package_family_name: Option<String>,
    pub arp_entries: Vec<ArpEntry>,
    pub dependencies: Option<Dependencies>,
    pub url: String,
    pub sha256: String,
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
    /// One entry per installer, in manifest order; each field is the installer's own value
    /// or else the root's (switches merge key by key; dependencies replace as a whole).
    pub fn effective(&self) -> Vec<EffectiveInstaller> {
        let root = self;
        self.installers
            .iter()
            .map(|i| {
                let ri = root.installer_switches.clone().unwrap_or_default();
                let ii = i.installer_switches.clone().unwrap_or_default();
                let switches = InstallerSwitches {
                    silent: ii.silent.or(ri.silent),
                    silent_with_progress: ii.silent_with_progress.or(ri.silent_with_progress),
                    interactive: ii.interactive.or(ri.interactive),
                    install_location: ii.install_location.or(ri.install_location),
                    log: ii.log.or(ri.log),
                    upgrade: ii.upgrade.or(ri.upgrade),
                    custom: ii.custom.or(ri.custom),
                };
                let kind = i.installer_type.clone().or(root.installer_type.clone());
                let nested = i
                    .nested_installer_type
                    .clone()
                    .or(root.nested_installer_type.clone());
                EffectiveInstaller {
                    architecture: i.architecture.clone(),
                    kind: InstallerKind::parse(kind.as_deref()),
                    nested_kind: nested.as_deref().map(|n| InstallerKind::parse(Some(n))),
                    nested_files: i
                        .nested_installer_files
                        .clone()
                        .or(root.nested_installer_files.clone())
                        .unwrap_or_default(),
                    scope: i.scope.clone().or(root.scope.clone()),
                    locale: i.installer_locale.clone().or(root.installer_locale.clone()),
                    platform: i
                        .platform
                        .clone()
                        .or(root.platform.clone())
                        .unwrap_or_default(),
                    minimum_os_version: i
                        .minimum_os_version
                        .clone()
                        .or(root.minimum_os_version.clone()),
                    install_modes: i
                        .install_modes
                        .clone()
                        .or(root.install_modes.clone())
                        .unwrap_or_default(),
                    switches,
                    success_codes: i
                        .installer_success_codes
                        .clone()
                        .or(root.installer_success_codes.clone())
                        .unwrap_or_default(),
                    expected_codes: i
                        .expected_return_codes
                        .clone()
                        .or(root.expected_return_codes.clone())
                        .unwrap_or_default(),
                    upgrade_behavior: i.upgrade_behavior.clone().or(root.upgrade_behavior.clone()),
                    require_explicit_upgrade: i
                        .require_explicit_upgrade
                        .or(root.require_explicit_upgrade)
                        .unwrap_or(false),
                    elevation: i
                        .elevation_requirement
                        .clone()
                        .or(root.elevation_requirement.clone()),
                    product_code: i.product_code.clone().or(root.product_code.clone()),
                    package_family_name: i
                        .package_family_name
                        .clone()
                        .or(root.package_family_name.clone()),
                    arp_entries: i
                        .apps_and_features_entries
                        .clone()
                        .or(root.apps_and_features_entries.clone())
                        .unwrap_or_default(),
                    dependencies: i.dependencies.clone().or(root.dependencies.clone()),
                    url: i.installer_url.clone(),
                    sha256: i.installer_sha256.clone(),
                }
            })
            .collect()
    }

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
