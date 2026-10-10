#![cfg(windows)]
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;

use wingetrs::install::fs_win::files_in_use;
use wingetrs::install::registry_win::RegScanner;
use wingetrs::install::traits::{RegistryScan, Trash};
use wingetrs::install::trash_win::ShellTrash;
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

#[test]
fn files_in_use_lists_exclusively_opened_binaries() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("sub").join("a.exe");
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::write(&exe, b"x").unwrap();
    let txt = dir.path().join("b.txt");
    std::fs::write(&txt, b"x").unwrap();

    assert!(files_in_use(dir.path()).is_empty());
    {
        let _held = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&exe)
            .unwrap();
        let _held_txt = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&txt)
            .unwrap();
        assert_eq!(files_in_use(dir.path()), vec![exe.clone()]);
    }
    assert!(files_in_use(dir.path()).is_empty());
}

struct TestKey(String);
impl TestKey {
    fn new(tag: &str) -> TestKey {
        TestKey(format!(
            "Software\\wingetrs-test\\scan-{tag}-{}",
            std::process::id()
        ))
    }
}
impl Drop for TestKey {
    fn drop(&mut self) {
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.0);
    }
}

#[test]
#[ignore = "live: grava sob HKCU Software wingetrs-test"]
fn live_registry_scan_export_delete() {
    let tk = TestKey::new("reg");
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (k, _) = hkcu
        .create_subkey(format!("{}\\Classes\\x\\shell\\open\\command", tk.0))
        .unwrap();
    k.set_value("", &"\"C:\\W\\apps\\app\\x.exe\" \"%1\"")
        .unwrap();
    let scan = RegScanner::with_base(&tk.0);

    // case-insensitive on both needle and data
    let hits = scan
        .values_containing(&["Classes"], "c:\\w\\APPS\\app")
        .unwrap();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].key, "Classes\\x\\shell\\open\\command");
    assert_eq!(hits[0].value_name, None);
    assert!(
        scan.values_containing(&["Classes"], "nothing-here")
            .unwrap()
            .is_empty()
    );
    assert!(
        scan.values_containing(&["Missing"], "x")
            .unwrap()
            .is_empty()
    );

    assert_eq!(
        scan.subkeys_named("Classes", &["X"]).unwrap(),
        vec!["Classes\\x".to_string()]
    );

    let dir = tempfile::tempdir().unwrap();
    let reg = dir.path().join("with space").join("out file.reg");
    std::fs::create_dir_all(reg.parent().unwrap()).unwrap();
    scan.export("Classes\\x", &reg).unwrap();
    let bytes = std::fs::read(&reg).unwrap();
    let (pairs, _) = bytes[2..].as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let text = String::from_utf16_lossy(&units);
    assert!(text.contains("Classes\\x\\shell\\open\\command"), "{text}");

    scan.delete_key("Classes\\x").unwrap();
    assert!(
        scan.values_containing(&["Classes"], "x.exe")
            .unwrap()
            .is_empty()
    );
    assert!(scan.delete_key("").is_err());
}

#[test]
#[ignore = "live: grava sob HKCU Software wingetrs-test"]
fn live_classes_scan_is_targeted() {
    let tk = TestKey::new("prune");
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let data = "C:\\W\\apps\\app\\x.exe";
    for sub in [
        "Classes\\Local Settings\\shell\\open\\command",
        "Classes\\CLSID\\shell\\open\\command",
        "Classes\\.foo\\shell\\open\\command",
        "Classes\\Applications\\x.exe\\shell\\open\\command",
        "Classes\\Directory\\Background\\shell\\v\\command",
        "Classes\\Prog\\DefaultIcon",
        "Classes\\Nest\\shell\\v\\shell\\w\\command",
        // MAX_NESTED_SHELL + 1 levels deep: must not be found
        "Classes\\Deep\\shell\\a\\shell\\b\\shell\\c\\shell\\d\\shell\\e\\command",
    ] {
        let (k, _) = hkcu.create_subkey(format!("{}\\{sub}", tk.0)).unwrap();
        k.set_value("", &data).unwrap();
    }
    let scan = RegScanner::with_base(&tk.0);
    let mut keys: Vec<String> = scan
        .values_containing(&["Classes"], "apps\\app")
        .unwrap()
        .into_iter()
        .map(|h| h.key)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "Classes\\.foo\\shell\\open\\command",
            "Classes\\Applications\\x.exe\\shell\\open\\command",
            "Classes\\Directory\\Background\\shell\\v\\command",
            "Classes\\Nest\\shell\\v\\shell\\w\\command",
            "Classes\\Prog\\DefaultIcon",
        ]
    );
    // the exhaustive scan sees the pruned subtrees
    assert_eq!(
        scan.values_containing_full(&["Classes"], "apps\\app", 2)
            .len(),
        // 8 fixtures; the exhaustive walk stops at MAX_DEPTH before the deepest one
        7
    );
}

#[test]
#[ignore = "live: grava sob HKCU Software wingetrs-test"]
fn live_registry_delete_value() {
    let tk = TestKey::new("val");
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (k, _) = hkcu.create_subkey(format!("{}\\Run", tk.0)).unwrap();
    k.set_value("app", &"C:\\W\\x.exe").unwrap();
    let scan = RegScanner::with_base(&tk.0);
    assert_eq!(scan.values_containing(&["Run"], "x.exe").unwrap().len(), 1);
    scan.delete_value("Run", "app").unwrap();
    assert!(
        scan.values_containing(&["Run"], "x.exe")
            .unwrap()
            .is_empty()
    );
}

#[test]
#[ignore = "live: varre HKCU real (somente leitura) e mede tempo"]
fn live_scan_speed() {
    let scan = RegScanner::real();
    let roots = [
        "Software\\Classes",
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths",
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
    ];
    // equivalence: thread count does not change hits or order
    let seq = scan.values_containing_with(&roots, "microsoft", 1);
    let par = scan.values_containing(&roots, "microsoft").unwrap();
    assert_eq!(seq, par);

    let mut times = Vec::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        scan.values_containing(&roots, "wingetrs-no-such-app")
            .unwrap();
        times.push(t.elapsed().as_millis());
    }
    times.sort();
    eprintln!(
        "targeted scan, 5 runs sorted: {times:?} ms; median {} ms",
        times[2]
    );
    assert!(times[2] < 300, "{times:?}");
}

#[test]
#[ignore = "live: varre HKCU real (somente leitura); prova de cobertura da varredura direcionada"]
fn live_targeted_coverage() {
    let scan = RegScanner::real();
    let roots = ["Software\\Classes"];
    let pruned = [
        "Local Settings",
        "CLSID",
        "Interface",
        "TypeLib",
        "WOW6432Node",
        "shellex",
    ];
    for needle in ["\\appdata\\", "c:\\"] {
        let full = scan.values_containing_full(&roots, needle, 8);
        let targeted = scan.values_containing_with(&roots, needle, 8);
        let only_full: Vec<_> = full.iter().filter(|h| !targeted.contains(h)).collect();
        let only_targeted = targeted.iter().filter(|h| !full.contains(h)).count();
        let mut groups: std::collections::BTreeMap<String, usize> = Default::default();
        for h in &only_full {
            let g = h.key.split('\\').take(3).collect::<Vec<_>>().join("\\");
            *groups.entry(g).or_default() += 1;
        }
        eprintln!(
            "needle {needle:?}: full {} hits, targeted {} hits, only-full {}, only-targeted {only_targeted}",
            full.len(),
            targeted.len(),
            only_full.len()
        );
        for (g, n) in &groups {
            eprintln!("    {n:5}  {g}");
        }
        for h in targeted.iter().filter(|h| !full.contains(h)) {
            eprintln!("    only-targeted: {} = {}", h.key, h.data);
        }
        assert_eq!(only_targeted, 0);
        let outside: Vec<_> = only_full
            .iter()
            .filter(|h| {
                let second = h.key.split('\\').nth(2).unwrap_or("");
                !pruned.iter().any(|p| p.eq_ignore_ascii_case(second))
            })
            .map(|h| (&h.key, &h.data))
            .collect();
        // Everything missed must live in a pruned subtree, or be a non-shell part of a
        // ProgID/extension key (reported above for review).
        assert!(
            outside.is_empty(),
            "missed outside pruned subtrees: {outside:?}"
        );
        eprintln!("    outside pruned subtrees: 0");
        for (k, d) in outside.iter().take(40) {
            eprintln!("      {k} = {d}");
        }
    }
}

#[test]
#[ignore = "live: usa a Lixeira real com arquivo do próprio tempdir"]
fn live_recycle_removes_file() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("recycle-me.txt");
    std::fs::write(&f, b"x").unwrap();
    ShellTrash.recycle(&f).unwrap();
    assert!(!f.exists());
}

#[test]
fn base_resolution_never_yields_real_hkcu_when_variable_is_present() {
    assert_eq!(RegScanner::resolve_base(None), None);
    assert_eq!(
        RegScanner::resolve_base(Some("")).as_deref(),
        Some(r"Software\wingetrs-test\env-empty")
    );
    assert_eq!(
        RegScanner::resolve_base(Some(r"Software\x")).as_deref(),
        Some(r"Software\x")
    );
}
