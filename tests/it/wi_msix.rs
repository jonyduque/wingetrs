//! MSIX install and remove (Task 179).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::path::Path;
use wingetrs::winget_install::codes::{Category, Outcome};
use wingetrs::winget_install::msix::{
    AddResult, Deployer, ERROR_PACKAGES_IN_USE, MsixError, add_with_deferred_fallback, file_uri,
    outcome, remove_each,
};

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
    assert_eq!(file_uri(Path::new(r"ab\x.msix")), None);
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

fn err(hresult: u32) -> MsixError {
    MsixError {
        hresult: hresult as i32,
        text: format!("0x{hresult:08x}"),
    }
}

const IN_USE: u32 = 0x80073D02;
const OTHER: u32 = 0x80073CF3;

/// Scripted `Deployer`: each step returns the configured result and is logged.
struct Fake {
    add: Result<(), MsixError>,
    full_name: Result<String, MsixError>,
    installed: Result<bool, MsixError>,
    stage: Result<(), MsixError>,
    register: Result<(), MsixError>,
    calls: RefCell<Vec<String>>,
}

impl Fake {
    fn in_use() -> Fake {
        Fake {
            add: Err(err(IN_USE)),
            full_name: Ok("App_2.0.0.0_x64__p".into()),
            installed: Ok(false),
            stage: Ok(()),
            register: Ok(()),
            calls: RefCell::new(Vec::new()),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl Deployer for Fake {
    fn add(&self) -> Result<(), MsixError> {
        self.calls.borrow_mut().push("add".into());
        self.add.clone()
    }
    fn full_name(&self) -> Result<String, MsixError> {
        self.calls.borrow_mut().push("full_name".into());
        self.full_name.clone()
    }
    fn is_installed(&self, full_name: &str) -> Result<bool, MsixError> {
        self.calls
            .borrow_mut()
            .push(format!("installed {full_name}"));
        self.installed.clone()
    }
    fn stage(&self) -> Result<(), MsixError> {
        self.calls.borrow_mut().push("stage".into());
        self.stage.clone()
    }
    fn register(&self, full_name: &str) -> Result<(), MsixError> {
        self.calls
            .borrow_mut()
            .push(format!("register {full_name}"));
        self.register.clone()
    }
    fn remove(&self, full_name: &str) -> Result<(), MsixError> {
        self.calls.borrow_mut().push(format!("remove {full_name}"));
        Ok(())
    }
}

#[test]
fn deferred_add_ok_is_installed_without_fallback() {
    let f = Fake {
        add: Ok(()),
        ..Fake::in_use()
    };
    assert_eq!(
        add_with_deferred_fallback(&f).unwrap(),
        AddResult::Installed
    );
    assert_eq!(f.calls(), vec!["add"]);
}

#[test]
fn deferred_other_add_error_is_returned_without_fallback() {
    let f = Fake {
        add: Err(err(OTHER)),
        ..Fake::in_use()
    };
    assert_eq!(
        add_with_deferred_fallback(&f).unwrap_err().hresult,
        OTHER as i32
    );
    assert_eq!(f.calls(), vec!["add"]);
}

#[test]
fn deferred_in_use_stages_and_registers() {
    let f = Fake::in_use();
    assert_eq!(
        add_with_deferred_fallback(&f).unwrap(),
        AddResult::Installed
    );
    assert_eq!(
        f.calls(),
        vec![
            "add",
            "full_name",
            "installed App_2.0.0.0_x64__p",
            "stage",
            "register App_2.0.0.0_x64__p"
        ]
    );
}

#[test]
fn deferred_register_in_use_is_deferred_success() {
    let f = Fake {
        register: Err(err(IN_USE)),
        ..Fake::in_use()
    };
    let r = add_with_deferred_fallback(&f).unwrap();
    assert_eq!(r, AddResult::Deferred);
    assert_eq!(
        r.user_text(),
        Some("o app será atualizado quando for fechado")
    );
    assert_eq!(AddResult::Installed.user_text(), None);
    assert!(!f.calls().iter().any(|c| c.starts_with("remove")));
}

#[test]
fn deferred_register_other_error_cleans_up_new_package() {
    let f = Fake {
        register: Err(err(OTHER)),
        ..Fake::in_use()
    };
    assert_eq!(
        add_with_deferred_fallback(&f).unwrap_err().hresult,
        OTHER as i32
    );
    assert_eq!(f.calls().last().unwrap(), "remove App_2.0.0.0_x64__p");
}

#[test]
fn deferred_stage_error_cleans_up_new_package() {
    let f = Fake {
        stage: Err(err(OTHER)),
        ..Fake::in_use()
    };
    assert_eq!(
        add_with_deferred_fallback(&f).unwrap_err().hresult,
        OTHER as i32
    );
    assert_eq!(
        f.calls(),
        vec![
            "add",
            "full_name",
            "installed App_2.0.0.0_x64__p",
            "stage",
            "remove App_2.0.0.0_x64__p"
        ]
    );
}

#[test]
fn deferred_no_cleanup_when_already_installed() {
    let f = Fake {
        installed: Ok(true),
        register: Err(err(OTHER)),
        ..Fake::in_use()
    };
    assert_eq!(
        add_with_deferred_fallback(&f).unwrap_err().hresult,
        OTHER as i32
    );
    assert!(!f.calls().iter().any(|c| c.starts_with("remove")));
}

#[test]
fn deferred_no_cleanup_when_install_state_unknown() {
    let f = Fake {
        installed: Err(err(OTHER)),
        stage: Err(err(OTHER)),
        ..Fake::in_use()
    };
    assert!(add_with_deferred_fallback(&f).is_err());
    assert!(f.calls().contains(&"stage".to_string()));
    assert!(!f.calls().iter().any(|c| c.starts_with("remove")));
}

#[test]
fn deferred_full_name_failure_keeps_in_use_error() {
    let f = Fake {
        full_name: Err(err(OTHER)),
        ..Fake::in_use()
    };
    assert_eq!(
        add_with_deferred_fallback(&f).unwrap_err().hresult,
        ERROR_PACKAGES_IN_USE
    );
    assert_eq!(f.calls(), vec!["add", "full_name"]);
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
    assert_eq!(ops.add(Path::new(&file)).unwrap(), AddResult::Installed);
    let removed = ops.remove(&family).unwrap();
    assert!(removed >= 1, "nada removido para {family}");
    assert_eq!(ops.remove(&family).unwrap(), 0);
}
