#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::core::SourceKind;
use wingetrs::sources::winget::manifest::parse;

const PT: &[u8] = include_bytes!("fixtures/winget/powertoys-0.101.2362.0.merged.yaml");

#[test]
fn parses_real_powertoys_manifest() {
    let m = parse(PT).unwrap();
    assert_eq!(m.package_identifier, "Microsoft.PowerToys");
    assert_eq!(m.package_version, "0.101.2362.0");
    assert_eq!(m.installers.len(), 4);
    assert_eq!(m.tags.len(), 10);
}

#[test]
fn details_inherit_root_installer_type_and_keep_scope() {
    let d = parse(PT)
        .unwrap()
        .into_details(vec!["0.101.2362.0".into(), "0.100.2".into()]);
    assert_eq!(d.source, SourceKind::Winget);
    assert_eq!(d.name, "PowerToys");
    assert_eq!(d.publisher.as_deref(), Some("Microsoft Corporation"));
    assert_eq!(d.license.as_deref(), Some("MIT"));
    assert_eq!(
        d.homepage.as_deref(),
        Some("https://github.com/microsoft/PowerToys")
    );
    assert!(
        d.description
            .as_deref()
            .unwrap()
            .starts_with("Microsoft PowerToys is a set of utilities")
    );
    let types: Vec<&str> = d
        .installers
        .iter()
        .map(|i| i.installer_type.as_str())
        .collect();
    assert_eq!(types, ["burn", "burn", "burn", "burn"]);
    let archs: Vec<&str> = d
        .installers
        .iter()
        .map(|i| i.architecture.as_str())
        .collect();
    assert_eq!(archs, ["x64", "x64", "arm64", "arm64"]);
    let scopes: Vec<Option<&str>> = d.installers.iter().map(|i| i.scope.as_deref()).collect();
    assert_eq!(
        scopes,
        [Some("user"), Some("machine"), Some("user"), Some("machine")]
    );
    assert_eq!(d.available_versions, ["0.101.2362.0", "0.100.2"]);
}

#[test]
fn installer_level_type_overrides_root() {
    let y = "PackageIdentifier: A.B\nPackageVersion: 1.0\nInstallerType: exe\nInstallers:\n- Architecture: x86\n  InstallerType: msi\n  InstallerUrl: https://x/a.msi\n  InstallerSha256: AA\n- Architecture: x64\n  InstallerUrl: https://x/b.exe\n  InstallerSha256: BB\n";
    let d = parse(y.as_bytes()).unwrap().into_details(vec![]);
    assert_eq!(d.installers[0].installer_type, "msi");
    assert_eq!(d.installers[1].installer_type, "exe");
    assert_eq!(d.name, "A.B");
    assert_eq!(d.version, "1.0");
}

#[test]
fn short_description_wins_over_description() {
    let y = "PackageIdentifier: A.B\nPackageVersion: 1.0\nShortDescription: curta\nDescription: longa\n";
    let d = parse(y.as_bytes()).unwrap().into_details(vec![]);
    assert_eq!(d.description.as_deref(), Some("curta"));
}

#[test]
fn minimal_manifest_without_optional_fields() {
    let d = parse(b"PackageIdentifier: X.Y\nPackageVersion: '2'\n")
        .unwrap()
        .into_details(vec![]);
    assert!(d.installers.is_empty());
    assert_eq!(d.installers.len(), 0);
    assert_eq!(d.publisher, None);
}

#[test]
fn invalid_utf8_and_yaml_are_errors() {
    assert!(parse(&[0xFF, 0xFE, 0x00]).is_err());
    assert!(parse(b"Installers: [").is_err());
}
