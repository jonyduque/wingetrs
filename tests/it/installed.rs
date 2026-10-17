#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::HashMap;
use wingetrs::installed::{
    Hive, InstallSource, RawArpEntry, filter_arp, pack_guid, read_scoop, unpack_guid,
};

fn raw(name: &str) -> RawArpEntry {
    RawArpEntry {
        hive: Some(Hive::MachineX64),
        key_name: "K".into(),
        display_name: Some(name.into()),
        ..Default::default()
    }
}

#[test]
fn guid_packing_round_trip() {
    assert_eq!(
        pack_guid("{23170F69-40C1-2701-1900-000001000000}").unwrap(),
        "96F071321C0410729100000010000000"
    );
    assert_eq!(
        unpack_guid("96F071321C0410729100000010000000").unwrap(),
        "{23170F69-40C1-2701-1900-000001000000}"
    );
    assert_eq!(
        pack_guid("{23170f69-40c1-2701-1900-000001000000}").unwrap(),
        "96F071321C0410729100000010000000"
    );
    assert!(pack_guid("not-a-guid").is_none());
    assert!(unpack_guid("XYZ").is_none());
}

#[test]
fn filters_system_children_updates_and_nameless() {
    let none = HashMap::new();
    assert!(
        filter_arp(
            RawArpEntry {
                display_name: None,
                ..raw("x")
            },
            &none
        )
        .is_none()
    );
    assert!(filter_arp(raw("   "), &none).is_none());
    assert!(
        filter_arp(
            RawArpEntry {
                system_component: Some(1),
                ..raw("x")
            },
            &none
        )
        .is_none()
    );
    assert!(
        filter_arp(
            RawArpEntry {
                parent_key_name: Some("P".into()),
                ..raw("x")
            },
            &none
        )
        .is_none()
    );
    assert!(
        filter_arp(
            RawArpEntry {
                release_type: Some("security update".into()),
                ..raw("x")
            },
            &none
        )
        .is_none()
    );
    assert!(
        filter_arp(
            RawArpEntry {
                system_component: Some(0),
                ..raw("Keep")
            },
            &none
        )
        .is_some()
    );
}

#[test]
fn msi_entry_gets_product_and_upgrade_code() {
    let mut up = HashMap::new();
    up.insert(
        "{23170F69-40C1-2701-1900-000001000000}".to_string(),
        "{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}".to_string(),
    );
    let app = filter_arp(
        RawArpEntry {
            key_name: "{23170F69-40C1-2701-1900-000001000000}".into(),
            windows_installer: Some(1),
            display_version: Some("19.00.00.0".into()),
            ..raw("7-Zip 19.00 (x64 edition)")
        },
        &up,
    )
    .unwrap();
    assert_eq!(
        app.product_code.as_deref(),
        Some("{23170F69-40C1-2701-1900-000001000000}")
    );
    assert_eq!(
        app.upgrade_code.as_deref(),
        Some("{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}")
    );
    assert_eq!(app.source, InstallSource::Arp(Hive::MachineX64));
    assert_eq!(app.version.as_deref(), Some("19.00.00.0"));
}

#[test]
fn non_msi_key_name_is_still_a_product_code_candidate_and_blank_version_is_none() {
    let app = filter_arp(
        RawArpEntry {
            key_name: "Notepad++".into(),
            display_version: Some("  ".into()),
            ..raw("Notepad++ (64-bit x64)")
        },
        &HashMap::new(),
    )
    .unwrap();
    assert_eq!(app.product_code.as_deref(), Some("Notepad++"));
    assert_eq!(app.version, None);
}

#[test]
fn scoop_apps_from_directory_layout() {
    let dir = tempfile::tempdir().unwrap();
    let cur = dir.path().join("apps/ripgrep/current");
    std::fs::create_dir_all(&cur).unwrap();
    std::fs::write(
        cur.join("manifest.json"),
        r#"{"version":"15.2.0","description":"rg"}"#,
    )
    .unwrap();
    std::fs::write(
        cur.join("install.json"),
        r#"{"bucket":"main","architecture":"64bit"}"#,
    )
    .unwrap();
    let broken = dir.path().join("apps/broken/current");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("manifest.json"), "{oops").unwrap();
    let apps = read_scoop(dir.path());
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].key, "ripgrep");
    assert_eq!(apps[0].scoop_bucket.as_deref(), Some("main"));
    assert_eq!(apps[0].version.as_deref(), Some("15.2.0"));
    assert_eq!(apps[0].source, InstallSource::Scoop);
}

#[test]
#[ignore = "live: reads this machine's registry and MSIX packages"]
fn live_scan_is_fast_and_nonempty() {
    let scan = wingetrs::installed::scan_all();
    eprintln!(
        "apps={} errors={:?} elapsed_ms={}",
        scan.apps.len(),
        scan.errors,
        scan.elapsed_ms
    );
    assert!(scan.apps.len() > 50);
    assert!(scan.elapsed_ms < 500);
}

#[test]
#[ignore = "live: registro real"]
fn live_stale_arp_records() {
    let scan = wingetrs::installed::scan_all();
    let stale: Vec<_> = scan.apps.iter().filter(|a| a.stale.is_some()).collect();
    for a in &stale {
        eprintln!("stale: {} -> {:?}", a.name, a.stale);
    }
    eprintln!("stale count = {}", stale.len());
}

fn lookup_env(name: &str) -> Option<String> {
    (name == "LOCALAPPDATA").then(|| r"C:\L".to_string())
}

#[test]
fn missing_uninstaller_cases() {
    let missing = |cmd: &str| {
        wingetrs::installed::missing_uninstaller(cmd, lookup_env, |_| false)
            .map(|p| p.to_string_lossy().into_owned())
    };
    assert_eq!(
        missing(r#""C:\X\u.exe" /S"#).as_deref(),
        Some(r"C:\X\u.exe")
    );
    assert_eq!(
        missing(r"C:\Program Files (x86)\Steam\steam.exe steam://uninstall/255710").as_deref(),
        Some(r"C:\Program Files (x86)\Steam\steam.exe")
    );
    assert_eq!(
        missing(r"%LOCALAPPDATA%\A\u.exe").as_deref(),
        Some(r"C:\L\A\u.exe")
    );
    assert_eq!(missing(r"%NOPE%\u.exe"), None);
    assert_eq!(missing("winget uninstall --product-code x"), None);
    assert_eq!(missing(r"C:\Windows\System32\winget.exe uninstall x"), None);
    assert_eq!(missing("MsiExec.exe /X{ABC}"), None);
    assert_eq!(missing(r"C:\Windows\System32\MsiExec.exe /X{ABC}"), None);
    assert_eq!(
        missing(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe -c x"),
        None
    );
    assert_eq!(missing(r"C:\Windows\System32\rundll32.exe x,y"), None);
    assert_eq!(
        missing(r"C:\Program Files\PowerShell\7\pwsh.exe -c x"),
        None
    );
    assert_eq!(missing(r"C:\Windows\System32\cmd.exe /c x"), None);
    assert_eq!(
        missing(r"C:\foo.exe.d\u.exe /S").as_deref(),
        Some(r"C:\foo.exe.d\u.exe")
    );
    assert_eq!(
        missing(r"C:\Steam\steam.exe/uninstall").as_deref(),
        Some(r"C:\Steam\steam.exe")
    );
    assert_eq!(missing("powershell -c x"), None);
    assert_eq!(missing("u.exe"), None);
}

#[test]
fn existing_uninstaller_is_not_stale() {
    let found = wingetrs::installed::missing_uninstaller(r#""C:\X\u.exe" /S"#, lookup_env, |p| {
        p == std::path::Path::new(r"C:\X\u.exe")
    });
    assert_eq!(found, None);
}

fn utf16le(s: &str, nul: bool) -> Vec<u8> {
    let mut v: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    if nul {
        v.extend([0, 0]);
    }
    v
}

#[test]
fn reg_string_reads_sz_and_expand_sz() {
    use wingetrs::installed::arp::{REG_EXPAND_SZ, REG_SZ, reg_string};
    assert_eq!(
        reg_string(REG_SZ, &utf16le("Olá 7-Zip", true)).as_deref(),
        Some("Olá 7-Zip")
    );
    assert_eq!(
        reg_string(REG_EXPAND_SZ, &utf16le("%X%", true)).as_deref(),
        Some("%X%")
    );
}

#[test]
fn reg_string_ignores_non_string_types() {
    use wingetrs::installed::arp::{REG_DWORD, reg_string};
    assert_eq!(reg_string(REG_DWORD, &[1, 0, 0, 0]), None);
}

#[test]
fn reg_string_tolerates_odd_length_and_missing_terminator() {
    use wingetrs::installed::arp::{REG_SZ, reg_string};
    assert_eq!(
        reg_string(REG_SZ, &utf16le("abc", false)).as_deref(),
        Some("abc")
    );
    let mut odd = utf16le("abc", true);
    odd.push(0x41);
    assert_eq!(reg_string(REG_SZ, &odd).as_deref(), Some("abc"));
    assert_eq!(reg_string(REG_SZ, &[0x41]).as_deref(), Some(""));
    assert_eq!(reg_string(REG_SZ, &[]).as_deref(), Some(""));
}

#[test]
fn reg_dword_needs_four_bytes() {
    use wingetrs::installed::arp::{REG_DWORD, REG_SZ, reg_dword};
    assert_eq!(reg_dword(REG_DWORD, &[1, 0, 0, 0]), Some(1));
    assert_eq!(reg_dword(REG_DWORD, &[1, 0]), None);
    assert_eq!(reg_dword(REG_SZ, &[1, 0, 0, 0]), None);
}

#[cfg(windows)]
#[test]
fn reg_type_constants_match_winreg() {
    use winreg::enums;
    assert_eq!(wingetrs::installed::arp::REG_SZ, enums::REG_SZ as u32);
    assert_eq!(
        wingetrs::installed::arp::REG_EXPAND_SZ,
        enums::REG_EXPAND_SZ as u32
    );
    assert_eq!(wingetrs::installed::arp::REG_DWORD, enums::REG_DWORD as u32);
}

#[test]
fn msix_frameworks_are_deduped_by_family_keeping_highest_version() {
    use wingetrs::installed::{InstallSource, InstalledApp, dedupe_msix_by_family};
    let m = |fam: &str, v: &str| InstalledApp {
        source: InstallSource::Msix,
        key: fam.into(),
        name: fam.into(),
        publisher: None,
        version: Some(v.into()),
        product_code: None,
        upgrade_code: None,
        family_name: Some(fam.into()),
        scoop_bucket: None,
        stale: None,
    };
    let out = dedupe_msix_by_family(vec![
        m("Microsoft.VCLibs.140.00_8wekyb3d8bbwe", "14.0.33519.0"),
        m("Microsoft.VCLibs.140.00_8wekyb3d8bbwe", "14.0.33728.0"),
        m("Microsoft.VCLibs.140.00_8wekyb3d8bbwe", "14.0.30704.0"),
        m("Microsoft.UI.Xaml.2.8_8wekyb3d8bbwe", "8.2310.30001.0"),
    ]);
    assert_eq!(out.len(), 2);
    let vc = out
        .iter()
        .find(|a| a.key.starts_with("Microsoft.VCLibs"))
        .unwrap();
    assert_eq!(vc.version.as_deref(), Some("14.0.33728.0"));
}

#[test]
fn msix_display_name_falls_back_on_resource_strings() {
    use wingetrs::installed::msix_display_name;
    assert_eq!(
        msix_display_name(Some("ms-resource:AppName"), "Microsoft.VCLibs.140.00"),
        "Microsoft.VCLibs.140.00"
    );
    assert_eq!(msix_display_name(Some("  "), "X.Y"), "X.Y");
    assert_eq!(msix_display_name(None, "X.Y"), "X.Y");
    assert_eq!(
        msix_display_name(Some("Windows Terminal"), "Microsoft.WindowsTerminal"),
        "Windows Terminal"
    );
}

#[test]
fn parallel_preserves_order_and_items() {
    use std::time::Duration;
    let items: Vec<u32> = (0..1000).collect();
    let out = wingetrs::installed::par::parallel(&items, 8, |&i| {
        std::thread::sleep(Duration::from_micros(u64::from((i * 7919) % 50)));
        Some(i)
    });
    assert_eq!(out, items);
}

#[test]
fn parallel_propagates_worker_panic() {
    let items: Vec<u32> = (0..50).collect();
    let r = std::panic::catch_unwind(|| {
        wingetrs::installed::par::parallel(&items, 4, |&i| {
            assert!(i != 20, "boom");
            Some(i)
        })
    });
    assert!(r.is_err());
}
