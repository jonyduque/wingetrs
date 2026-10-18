//! Full merged winget manifest (Task 170).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use wingetrs::sources::winget::manifest::{self, InstallerKind};

const HEAD: &str = "PackageIdentifier: A.B\nPackageVersion: 1.0\n";

fn eff(body: &str) -> Vec<manifest::EffectiveInstaller> {
    let yaml = format!("{HEAD}{body}");
    manifest::parse(yaml.as_bytes())
        .expect("manifest parses")
        .effective()
}

#[test]
fn installer_field_falls_back_to_root() {
    let e = eff(
        "InstallerType: inno\nScope: machine\nElevationRequirement: elevatesSelf\nProductCode: \"{A}\"\nInstallers:\n- Architecture: x64\n  Scope: user\n  InstallerUrl: https://x/a\n  InstallerSha256: AA\n- Architecture: x86\n  InstallerUrl: https://x/b\n  InstallerSha256: BB\n",
    );
    assert_eq!(e.len(), 2);
    assert_eq!(e[0].scope.as_deref(), Some("user"));
    assert_eq!(e[0].kind, InstallerKind::Inno);
    assert_eq!(e[0].elevation.as_deref(), Some("elevatesSelf"));
    assert_eq!(e[1].scope.as_deref(), Some("machine"));
    assert_eq!(e[1].product_code.as_deref(), Some("{A}"));
}

#[test]
fn switches_merge_key_by_key() {
    let e = eff(
        "InstallerSwitches:\n  Silent: /s-root\n  Custom: /c-root\nInstallers:\n- Architecture: x64\n  InstallerUrl: https://x/a\n  InstallerSha256: AA\n  InstallerSwitches:\n    Silent: /s-inst\n    Log: /l\n",
    );
    let s = &e[0].switches;
    assert_eq!(s.silent.as_deref(), Some("/s-inst"));
    assert_eq!(s.custom.as_deref(), Some("/c-root"));
    assert_eq!(s.log.as_deref(), Some("/l"));
    assert_eq!(s.upgrade, None);
}

#[test]
fn codes_and_expected_parse() {
    let e = eff(
        "InstallerSuccessCodes: [3010, -1, 4294967295]\nExpectedReturnCodes:\n- InstallerReturnCode: 1\n  ReturnResponse: packageInUse\nInstallers:\n- Architecture: x64\n  InstallerUrl: https://x/a\n  InstallerSha256: AA\n",
    );
    assert_eq!(e[0].success_codes, vec![3010, -1, 4294967295]);
    assert_eq!(e[0].expected_codes[0].installer_return_code, 1);
    assert_eq!(e[0].expected_codes[0].return_response, "packageInUse");
}

#[test]
fn minimum_os_version_key_is_exact() {
    let e = eff(
        "Installers:\n- Architecture: x64\n  MinimumOSVersion: 10.0.22000.0\n  InstallerUrl: https://x/a\n  InstallerSha256: AA\n",
    );
    assert_eq!(e[0].minimum_os_version.as_deref(), Some("10.0.22000.0"));
}

#[test]
fn agreements_and_dependencies_parse() {
    let yaml = format!(
        "{HEAD}Agreements:\n- AgreementLabel: EULA\n  AgreementUrl: https://x/eula\nDependencies:\n  PackageDependencies:\n  - PackageIdentifier: Microsoft.VCRedist.2015+.x64\n    MinimumVersion: \"14.0\"\nInstallers:\n- Architecture: x64\n  InstallerUrl: https://x/a\n  InstallerSha256: AA\n- Architecture: x86\n  InstallerUrl: https://x/b\n  InstallerSha256: BB\n  Dependencies:\n    WindowsFeatures: [NetFx3]\n"
    );
    let m = manifest::parse(yaml.as_bytes()).expect("parses");
    let e = m.effective();
    let d0 = e[0].dependencies.as_ref().expect("root deps");
    assert_eq!(d0.package_dependencies.len(), 1);
    let d1 = e[1].dependencies.as_ref().expect("installer deps");
    assert_eq!(d1.package_dependencies.len(), 0);
    assert_eq!(d1.windows_features, vec!["NetFx3".to_string()]);
    assert_eq!(
        m.agreements[0].agreement_url.as_deref(),
        Some("https://x/eula")
    );
}

#[test]
fn installer_kind_parse_case_insensitive() {
    assert_eq!(InstallerKind::parse(Some("MSI")), InstallerKind::Msi);
    assert_eq!(InstallerKind::parse(Some("wix")), InstallerKind::Wix);
    assert_eq!(
        InstallerKind::parse(Some("Nullsoft")),
        InstallerKind::Nullsoft
    );
    assert_eq!(InstallerKind::parse(Some("appx")), InstallerKind::Appx);
    assert_eq!(
        InstallerKind::parse(None),
        InstallerKind::Unknown(String::new())
    );
    assert_eq!(
        InstallerKind::parse(Some("weird")),
        InstallerKind::Unknown("weird".into())
    );
    assert_eq!(InstallerKind::parse(Some("Inno")).name(), "inno");
}

#[test]
fn nested_installer_inherits() {
    let e = eff(
        "NestedInstallerType: portable\nNestedInstallerFiles:\n- RelativeFilePath: 'bin\\x.exe'\n  PortableCommandAlias: x\nInstallers:\n- Architecture: x64\n  InstallerType: zip\n  InstallerUrl: https://x/a\n  InstallerSha256: AA\n",
    );
    assert_eq!(e[0].kind, InstallerKind::Zip);
    assert_eq!(e[0].nested_kind, Some(InstallerKind::Portable));
    assert_eq!(e[0].nested_files[0].relative_file_path, "bin\\x.exe");
}

#[test]
fn explicit_upgrade_defaults_false() {
    let inst = |extra: &str| {
        format!(
            "Installers:\n- Architecture: x64\n  InstallerUrl: https://x/a\n  InstallerSha256: AA\n{extra}"
        )
    };
    assert!(!eff(&inst("")).is_empty());
    assert!(!eff(&inst(""))[0].require_explicit_upgrade);
    let root = |extra: &str| format!("RequireExplicitUpgrade: true\n{}", inst(extra));
    assert!(eff(&root(""))[0].require_explicit_upgrade);
    assert!(!eff(&root("  RequireExplicitUpgrade: false\n"))[0].require_explicit_upgrade);
}
