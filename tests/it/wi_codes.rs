//! Exit code classification (Task 175).

use wingetrs::sources::winget::manifest::{ExpectedReturnCode, InstallerKind};
use wingetrs::winget_install::codes::{
    Category, Outcome, category_of, classify_install, classify_msix, classify_uninstall,
};

fn failed(c: Category) -> impl Fn(&Outcome) -> bool {
    move |o| matches!(o, Outcome::Failed { category, .. } if *category == c)
}

fn exp(code: i64, resp: &str) -> ExpectedReturnCode {
    ExpectedReturnCode {
        installer_return_code: code,
        return_response: resp.to_string(),
        return_response_url: None,
    }
}

#[test]
fn zero_and_success_codes_are_ok() {
    assert_eq!(
        classify_install(&InstallerKind::Exe, 0, &[], &[]),
        Outcome::Ok
    );
    assert_eq!(
        classify_install(&InstallerKind::Msi, 3010, &[3010], &[]),
        Outcome::Ok
    );
}

#[test]
fn msi_defaults() {
    let k = &InstallerKind::Msi;
    let c = |code| classify_install(k, code, &[], &[]);
    assert_eq!(c(3010), Outcome::OkReboot { initiated: false });
    assert_eq!(c(1641), Outcome::OkReboot { initiated: true });
    assert!(failed(Category::InstallInProgress)(&c(1618)));
    assert!(failed(Category::CancelledByUser)(&c(1602)));
    assert!(failed(Category::BlockedByPolicy)(&c(1625)));
    assert!(failed(Category::DiskFull)(&c(112)));
    assert!(failed(Category::AlreadyInstalled)(&c(1638)));
    assert_eq!(
        classify_install(&InstallerKind::Wix, 1618, &[], &[]),
        c(1618)
    );
    assert_eq!(
        classify_install(&InstallerKind::Burn, 3010, &[], &[]),
        c(3010)
    );
}

#[test]
fn inno_defaults_are_per_type() {
    let i = |code| classify_install(&InstallerKind::Inno, code, &[], &[]);
    assert!(failed(Category::CancelledByUser)(&i(2)));
    assert!(failed(Category::CancelledByUser)(&i(5)));
    assert!(failed(Category::RebootRequiredForInstall)(&i(8)));
    assert!(failed(Category::Unknown)(&classify_install(
        &InstallerKind::Nullsoft,
        2,
        &[],
        &[]
    )));
    assert!(failed(Category::Unknown)(&classify_install(
        &InstallerKind::Msi,
        2,
        &[],
        &[]
    )));
}

#[test]
fn manifest_expected_beats_default() {
    let o = classify_install(
        &InstallerKind::Msi,
        1618,
        &[],
        &[exp(1618, "contactSupport")],
    );
    assert!(failed(Category::ContactSupport)(&o));
}

#[test]
fn success_beats_expected() {
    let o = classify_install(&InstallerKind::Inno, 5, &[5], &[exp(5, "cancelledByUser")]);
    assert_eq!(o, Outcome::Ok);
}

#[test]
fn compares_as_u32() {
    assert_eq!(
        classify_install(&InstallerKind::Exe, -1, &[-1], &[]),
        Outcome::Ok
    );
    let o = classify_install(&InstallerKind::Exe, -1, &[], &[exp(4294967295, "diskFull")]);
    assert!(failed(Category::DiskFull)(&o));
    let o = classify_install(
        &InstallerKind::Exe,
        0x80070005_u32 as i32,
        &[],
        &[exp(-2147024891, "blockedByPolicy")],
    );
    assert!(failed(Category::BlockedByPolicy)(&o));
}

#[test]
fn unknown_code_message() {
    match classify_install(&InstallerKind::Exe, 7, &[], &[]) {
        Outcome::Failed {
            category: Category::Unknown,
            code: 7,
            message,
        } => {
            assert!(message.contains("código 7"), "{message}");
            assert!(message.contains("0x7"), "{message}");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn uninstall_only_zero() {
    assert!(matches!(
        classify_uninstall(false, 3010),
        Outcome::Failed { .. }
    ));
    assert_eq!(
        classify_uninstall(true, 3010),
        Outcome::OkReboot { initiated: false }
    );
    assert_eq!(
        classify_uninstall(true, 1641),
        Outcome::OkReboot { initiated: true }
    );
    assert_eq!(classify_uninstall(false, 0), Outcome::Ok);
    assert!(failed(Category::InstallInProgress)(&classify_uninstall(
        true, 1618
    )));
}

#[test]
fn msix_table_entry() {
    let o = classify_msix(0x80073D02_u32 as i32);
    assert!(failed(Category::PackageInUse)(&o));
}

#[test]
fn response_names_case_insensitive() {
    assert_eq!(category_of("PackageInUse"), Category::PackageInUse);
    assert_eq!(category_of("packageInUse"), Category::PackageInUse);
    assert_eq!(category_of("xyz"), Category::Custom);
}
