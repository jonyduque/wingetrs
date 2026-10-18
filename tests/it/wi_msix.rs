//! MSIX install and remove (Task 179).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::path::Path;
use wingetrs::winget_install::codes::{Category, Outcome};
use wingetrs::winget_install::msix::{MsixError, file_uri, outcome, remove_each};

#[test]
fn file_uri_encodes() {
    assert_eq!(
        file_uri(Path::new(r"C:\a b\x.msix")).as_deref(),
        Some("file:///C:/a%20b/x.msix")
    );
    assert_eq!(
        file_uri(Path::new(r"C:\a#b\x.msix")).as_deref(),
        Some("file:///C:/a%23b/x.msix")
    );
    assert_eq!(
        file_uri(Path::new(r"C:\100%\x.msix")).as_deref(),
        Some("file:///C:/100%25/x.msix")
    );
    assert_eq!(
        file_uri(Path::new("C:\\\u{e7}\u{e3}o\\x.msix")).as_deref(),
        Some("file:///C:/%C3%A7%C3%A3o/x.msix")
    );
    assert_eq!(file_uri(Path::new("x.msix")), None);
}

#[test]
fn file_uri_query_mark_and_relative_forms() {
    assert_eq!(
        file_uri(Path::new(r"C:\a?b\x.msix")).as_deref(),
        Some("file:///C:/a%3Fb/x.msix")
    );
    // Verbatim prefix from canonicalize() is dropped.
    assert_eq!(
        file_uri(Path::new(r"\\?\C:\d\x.msix")).as_deref(),
        Some("file:///C:/d/x.msix")
    );
    // Drive-relative and rooted-without-drive are not absolute.
    assert_eq!(file_uri(Path::new(r"C:x.msix")), None);
    assert_eq!(file_uri(Path::new(r"\d\x.msix")), None);
    assert_eq!(file_uri(Path::new(r"d\x.msix")), None);
}

#[test]
fn outcome_uses_msix_table() {
    let e = MsixError {
        hresult: 0x80073D02_u32 as i32,
        text: String::new(),
    };
    assert!(matches!(
        outcome(&e),
        Outcome::Failed {
            category: Category::PackageInUse,
            ..
        }
    ));
}

#[test]
fn remove_each_removes_every_full_name() {
    let seen = RefCell::new(Vec::new());
    let names = vec!["A_1.0_x64__p".to_string(), "A_2.0_x64__p".to_string()];
    let n = remove_each(&names, |n| {
        seen.borrow_mut().push(n.to_string());
        Ok(())
    })
    .unwrap();
    assert_eq!(n, 2);
    assert_eq!(*seen.borrow(), names);
}

#[test]
fn remove_each_stops_at_first_error() {
    let seen = RefCell::new(Vec::new());
    let names = vec!["A".to_string(), "B".to_string(), "C".to_string()];
    let err = remove_each(&names, |n| {
        seen.borrow_mut().push(n.to_string());
        if n == "B" {
            Err(MsixError {
                hresult: 0x80073D02_u32 as i32,
                text: "em uso".into(),
            })
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert_eq!(err.hresult, 0x80073D02_u32 as i32);
    assert_eq!(*seen.borrow(), vec!["A".to_string(), "B".to_string()]);
}

/// Deploys `WINGETRS_TEST_MSIX` for the current user and removes it by
/// `WINGETRS_TEST_MSIX_FAMILY`.
#[cfg(windows)]
#[test]
#[ignore = "live: implanta e remove um MSIX"]
fn live_add_and_remove() {
    use wingetrs::winget_install::msix::MsixOps;
    use wingetrs::winget_install::msix_win::SystemMsix;
    let file = std::env::var("WINGETRS_TEST_MSIX")
        .expect("WINGETRS_TEST_MSIX=<caminho absoluto de um .msix assinado>");
    let family = std::env::var("WINGETRS_TEST_MSIX_FAMILY")
        .expect("WINGETRS_TEST_MSIX_FAMILY=<família do pacote, ex.: Nome_8wekyb3d8bbwe>");
    let ops = SystemMsix;
    ops.add(Path::new(&file)).unwrap();
    let removed = ops.remove(&family).unwrap();
    assert!(removed >= 1, "nada removido para {family}");
    assert_eq!(ops.remove(&family).unwrap(), 0);
}
