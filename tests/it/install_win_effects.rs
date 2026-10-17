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

fn hkcu() -> winreg::RegKey {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
}

/// Own key per test (no process-wide env var: tests run in parallel); deleted on drop,
/// also when an assertion panics. Only ever this test's own subkey.
struct OwnKey(String);
impl OwnKey {
    fn new(tag: &str) -> OwnKey {
        OwnKey(format!(
            r"Software\wingetrs-test\env-{}-{tag}",
            std::process::id()
        ))
    }
}
impl Drop for OwnKey {
    fn drop(&mut self) {
        let _ = hkcu().delete_subkey_all(&self.0);
    }
}

fn val(value: &str, expand: bool) -> EnvValue {
    EnvValue {
        value: value.into(),
        expand,
    }
}

#[test]
#[ignore = "live: writes HKCU\\Software\\wingetrs-test"]
fn existing_reg_sz_stays_reg_sz_after_set() {
    let k = OwnKey::new("regsz");
    let (key, _) = hkcu().create_subkey(&k.0).unwrap();
    key.set_value("Tool", &r"C:\a").unwrap(); // &str ⇒ REG_SZ
    assert_eq!(
        key.get_raw_value("Tool").unwrap().vtype,
        winreg::enums::RegType::REG_SZ
    );
    let env = RegEnvStore::for_test_key(&k.0);
    assert_eq!(env.get("Tool").unwrap(), Some(val(r"C:\a", false)));
    env.set("Tool", &val(r"%TEMP%\b", true)).unwrap();
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
}

#[test]
#[ignore = "live: writes HKCU\\Software\\wingetrs-test"]
fn new_values_use_requested_type_and_remove_deletes() {
    let k = OwnKey::new("remove");
    let env = RegEnvStore::for_test_key(&k.0);
    assert_eq!(env.get("Nope").unwrap(), None);
    env.remove("Nope").unwrap();
    env.set("Plain", &val("x", false)).unwrap();
    env.set("Exp", &val("%X%", true)).unwrap();
    let key = hkcu().open_subkey(&k.0).unwrap();
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
    assert_eq!(env.get("Exp").unwrap(), Some(val("%X%", true)));
}

#[test]
#[ignore = "live: writes HKCU\\Software\\wingetrs-test"]
fn new_path_defaults_to_expand_sz() {
    let k = OwnKey::new("pathdefault");
    let env = RegEnvStore::for_test_key(&k.0);
    env.set("Path", &val(r"C:\x", false)).unwrap();
    let raw = hkcu()
        .open_subkey(&k.0)
        .unwrap()
        .get_raw_value("Path")
        .unwrap();
    assert_eq!(raw.vtype, winreg::enums::RegType::REG_EXPAND_SZ);
}

#[test]
#[ignore = "live: writes HKCU\\Software\\wingetrs-test"]
fn non_text_value_is_refused_by_get_and_set() {
    let k = OwnKey::new("dword");
    let (key, _) = hkcu().create_subkey(&k.0).unwrap();
    key.set_value("Num", &7u32).unwrap();
    let env = RegEnvStore::for_test_key(&k.0);
    assert!(env.get("Num").is_err());
    assert!(env.set("Num", &val("x", false)).is_err());
    let raw = key.get_raw_value("Num").unwrap();
    assert_eq!(raw.vtype, winreg::enums::RegType::REG_DWORD, "untouched");
    assert_eq!(&raw.bytes[..], &7u32.to_le_bytes()[..]);
}

#[test]
fn key_path_and_broadcast_follow_the_raw_test_variable() {
    use std::ffi::OsString;
    use wingetrs::install::env_win::EMPTY_TEST_KEY;
    // Absent ⇒ the real user environment, with broadcast.
    assert_eq!(RegEnvStore::key_path(None), "Environment");
    assert!(RegEnvStore::should_broadcast(None));
    // Present but empty ⇒ still a test: never Environment, never broadcast.
    let empty = Some(OsString::new());
    assert_ne!(RegEnvStore::key_path(empty.clone()), "Environment");
    assert_eq!(RegEnvStore::key_path(empty.clone()), EMPTY_TEST_KEY);
    assert!(EMPTY_TEST_KEY.starts_with(r"Software\wingetrs-test\"));
    assert!(!RegEnvStore::should_broadcast(empty));
    // Present ⇒ that key, no broadcast.
    let some = Some(OsString::from(r"Software\x"));
    assert_eq!(RegEnvStore::key_path(some.clone()), r"Software\x");
    assert!(!RegEnvStore::should_broadcast(some));
    assert!(!RegEnvStore::for_test_key(r"Software\wingetrs-test\x").broadcasts());
}

mod shortcuts {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use wingetrs::install::shortcut_win::{ComShortcuts, link_target};
    use wingetrs::install::traits::Shortcuts;

    #[test]
    fn test_root_follows_the_raw_test_variable() {
        assert_eq!(ComShortcuts::test_root(None).unwrap(), None);
        assert!(ComShortcuts::test_root(Some(OsString::new())).is_err());
        assert_eq!(
            ComShortcuts::test_root(Some(OsString::from(r"C:\t"))).unwrap(),
            Some(PathBuf::from(r"C:\t"))
        );
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ComShortcuts::with_root(dir.path()).root(), dir.path());
    }

    #[test]
    #[ignore = "live: creates a .lnk via COM in a tempdir"]
    fn create_and_remove_shortcut_in_test_dir() {
        let dir = tempfile::tempdir().unwrap();
        let sc = ComShortcuts::with_root(dir.path());
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
        assert!(
            dir.path().is_dir(),
            "a root not named wingetrs Apps is kept"
        );
        sc.remove(&other).unwrap(); // idempotent
    }

    #[test]
    #[ignore = "live: creates a .lnk via COM in a tempdir"]
    fn empty_wingetrs_apps_root_is_removed_but_never_its_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wingetrs Apps");
        let sc = ComShortcuts::with_root(&root);
        let target = std::env::current_exe().unwrap();
        let link = root.join("Sub").join("App.lnk");
        sc.create(&link, &target, None, None).unwrap();
        assert!(link.is_file());
        sc.remove(&link).unwrap();
        assert!(!root.join("Sub").exists());
        assert!(!root.exists(), "empty wingetrs Apps removed");
        assert!(tmp.path().is_dir(), "climb stops at the root");
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
