#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(windows)]
use wingetrs::install::env_win::RegEnvStore;
use wingetrs::install::traits::{EnvStore, EnvValue};

fn test_key() -> String {
    let k = format!(
        r"Software\wingetrs-test\env-{}-{}",
        std::process::id(),
        line!()
    );
    unsafe { std::env::set_var("WINGETRS_TEST_ENV_KEY", &k) };
    k
}

#[test]
#[ignore = "live: writes HKCU\\Software\\wingetrs-test"]
fn expand_sz_is_preserved_verbatim() {
    let k = test_key();
    let env = RegEnvStore::new();
    let user = EnvValue {
        value: r"%USERPROFILE%\bin;;C:\Tools\".into(),
        expand: true,
    };
    env.set("Path", &user).unwrap();
    assert_eq!(
        env.get("Path").unwrap(),
        Some(user.clone()),
        "read back without expansion"
    );
    let added = wingetrs::install::path_edit::path_add(&user.value, r"C:\W\shims");
    env.set(
        "Path",
        &EnvValue {
            value: added.clone(),
            expand: true,
        },
    )
    .unwrap();
    let raw = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(&k)
        .unwrap()
        .get_raw_value("Path")
        .unwrap();
    assert_eq!(raw.vtype, winreg::enums::RegType::REG_EXPAND_SZ);
    assert_eq!(env.get("Path").unwrap().unwrap().value, added);
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .delete_subkey_all(&k)
        .unwrap();
}

/// Own key per test, without touching the process-wide env var (tests run in parallel).
fn own_key(tag: &str) -> String {
    format!(r"Software\wingetrs-test\env-{}-{tag}", std::process::id())
}

fn hkcu() -> winreg::RegKey {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
}

#[test]
#[ignore = "live: writes HKCU\\Software\\wingetrs-test"]
fn existing_reg_sz_stays_reg_sz_after_set() {
    let k = own_key("regsz");
    let (key, _) = hkcu().create_subkey(&k).unwrap();
    key.set_value("Tool", &r"C:\a").unwrap(); // &str ⇒ REG_SZ
    assert_eq!(
        key.get_raw_value("Tool").unwrap().vtype,
        winreg::enums::RegType::REG_SZ
    );
    let env = RegEnvStore::for_test_key(&k);
    assert_eq!(
        env.get("Tool").unwrap(),
        Some(EnvValue {
            value: r"C:\a".into(),
            expand: false
        })
    );
    env.set(
        "Tool",
        &EnvValue {
            value: r"%TEMP%\b".into(),
            expand: true,
        },
    )
    .unwrap();
    let raw = key.get_raw_value("Tool").unwrap();
    assert_eq!(
        raw.vtype,
        winreg::enums::RegType::REG_SZ,
        "existing type wins"
    );
    assert_eq!(
        env.get("Tool").unwrap().unwrap().value,
        r"%TEMP%\b",
        "not expanded"
    );
    hkcu().delete_subkey_all(&k).unwrap();
}

#[test]
#[ignore = "live: writes HKCU\\Software\\wingetrs-test"]
fn new_values_use_requested_type_and_remove_deletes() {
    let k = own_key("remove");
    let env = RegEnvStore::for_test_key(&k);
    assert_eq!(env.get("Nope").unwrap(), None);
    env.remove("Nope").unwrap();
    env.set(
        "Plain",
        &EnvValue {
            value: "x".into(),
            expand: false,
        },
    )
    .unwrap();
    env.set(
        "Exp",
        &EnvValue {
            value: "%X%".into(),
            expand: true,
        },
    )
    .unwrap();
    let key = hkcu().open_subkey(&k).unwrap();
    assert_eq!(
        key.get_raw_value("Plain").unwrap().vtype,
        winreg::enums::RegType::REG_SZ
    );
    assert_eq!(
        key.get_raw_value("Exp").unwrap().vtype,
        winreg::enums::RegType::REG_EXPAND_SZ
    );
    env.remove("Plain").unwrap();
    assert_eq!(env.get("Plain").unwrap(), None);
    assert!(key.get_raw_value("Plain").is_err());
    assert_eq!(
        env.get("Exp").unwrap(),
        Some(EnvValue {
            value: "%X%".into(),
            expand: true
        })
    );
    hkcu().delete_subkey_all(&k).unwrap();
}

#[test]
#[ignore = "live: writes HKCU\\Software\\wingetrs-test"]
fn new_path_defaults_to_expand_sz() {
    let k = own_key("pathdefault");
    let env = RegEnvStore::for_test_key(&k);
    env.set(
        "Path",
        &EnvValue {
            value: r"C:\x".into(),
            expand: false,
        },
    )
    .unwrap();
    let raw = hkcu()
        .open_subkey(&k)
        .unwrap()
        .get_raw_value("Path")
        .unwrap();
    assert_eq!(raw.vtype, winreg::enums::RegType::REG_EXPAND_SZ);
    hkcu().delete_subkey_all(&k).unwrap();
}

#[test]
fn key_path_defaults_to_environment() {
    assert_eq!(RegEnvStore::key_path(None), "Environment");
    assert_eq!(
        RegEnvStore::key_path(Some(r"Software\wingetrs-test\env-1")),
        r"Software\wingetrs-test\env-1"
    );
}

#[test]
fn broadcast_is_off_under_test_key() {
    assert!(RegEnvStore::should_broadcast(None));
    assert!(!RegEnvStore::should_broadcast(Some(
        r"Software\wingetrs-test\env-1"
    )));
    assert!(!RegEnvStore::for_test_key(r"Software\wingetrs-test\x").broadcasts());
}

mod shortcuts {
    use std::path::Path;
    use wingetrs::install::shortcut_win::{ComShortcuts, link_target};
    use wingetrs::install::traits::Shortcuts;

    #[test]
    fn root_comes_from_test_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ComShortcuts::with_root(dir.path()).root(), dir.path());
    }

    #[test]
    #[ignore = "live: creates a .lnk via COM in a tempdir"]
    fn create_and_remove_shortcut_in_test_dir() {
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("WINGETRS_TEST_SHORTCUT_DIR", dir.path()) };
        let sc = ComShortcuts::new().unwrap();
        assert_eq!(sc.root(), dir.path());
        let target = std::env::current_exe().unwrap();
        let link = dir.path().join("Sub").join("Deep").join("App.lnk");
        sc.create(&link, &target, Some("--flag"), Some(&target))
            .unwrap();
        assert!(link.is_file());
        assert_eq!(link_target(&link).unwrap(), target);
        // A sibling keeps "Sub" alive; only "Deep" becomes empty.
        let other = dir.path().join("Sub").join("Other.lnk");
        sc.create(&other, &target, None, None).unwrap();
        sc.remove(&link).unwrap();
        assert!(!link.exists());
        assert!(
            !dir.path().join("Sub").join("Deep").exists(),
            "empty subfolder removed"
        );
        assert!(dir.path().join("Sub").is_dir(), "non-empty folder kept");
        sc.remove(&other).unwrap();
        assert!(!dir.path().join("Sub").exists());
        assert!(dir.path().is_dir(), "test root itself is never removed");
        sc.remove(&other).unwrap(); // idempotent
    }

    #[test]
    fn refuses_links_outside_root() {
        let dir = tempfile::tempdir().unwrap();
        let sc = ComShortcuts::with_root(dir.path());
        let t = Path::new(r"C:\x.exe");
        assert!(
            sc.create(&dir.path().join("..").join("evil.lnk"), t, None, None)
                .is_err()
        );
        assert!(sc.create(Path::new(r"C:\evil.lnk"), t, None, None).is_err());
        assert!(sc.remove(Path::new(r"C:\evil.lnk")).is_err());
    }
}
