#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::catalog::Catalog;
use wingetrs::core::SourceKind;
use wingetrs::installed::correlate::{Correlation, Rule, UpdateState, correlate, update_state};
use wingetrs::installed::normalize::NameNormalizer;
use wingetrs::installed::{Hive, InstallSource, InstalledApp};
use wingetrs::sources::winget::index::read_index;

const DB: &[u8] = include_bytes!("fixtures/winget/index-mini.db");

fn catalog() -> Catalog {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("i.db");
    std::fs::write(&p, DB).unwrap();
    let imp = read_index(&p).unwrap();
    let mut c = Catalog::open_in_memory().unwrap();
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v",
        1,
        &imp.packages,
        &imp.maps,
    )
    .unwrap();
    c
}

fn arp(key: &str, name: &str, publisher: &str, version: &str) -> InstalledApp {
    InstalledApp {
        source: InstallSource::Arp(Hive::MachineX64),
        key: key.into(),
        name: name.into(),
        publisher: Some(publisher.into()),
        version: Some(version.into()),
        product_code: Some(key.into()),
        upgrade_code: None,
        family_name: None,
        scoop_bucket: None,
    }
}

#[test]
fn product_code_match_is_case_insensitive() {
    let rows = correlate(
        &[arp(
            "{23170F69-40C1-2701-1900-000001000000}",
            "7-Zip 19.00 (x64 edition)",
            "Igor Pavlov",
            "19.00.00.0",
        )],
        &catalog(),
        &NameNormalizer::new(),
    )
    .unwrap();
    assert_eq!(
        rows[0].correlation,
        Correlation::Matched {
            source: "winget".into(),
            package_id: "7zip.7zip".into(),
            rule: Rule::ProductCode
        }
    );
    assert_eq!(
        rows[0].state,
        UpdateState::Available {
            latest: "26.04".into()
        }
    );
}

#[test]
fn package_family_name_match() {
    let mut a = arp(
        "x",
        "Windows Terminal",
        "Microsoft Corporation",
        "1.25.2733.0",
    );
    a.source = InstallSource::Msix;
    a.product_code = None;
    a.family_name = Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe".into());
    let rows = correlate(&[a], &catalog(), &NameNormalizer::new()).unwrap();
    assert_eq!(
        rows[0].correlation,
        Correlation::Matched {
            source: "winget".into(),
            package_id: "Microsoft.WindowsTerminal".into(),
            rule: Rule::PackageFamilyName
        }
    );
    assert_eq!(rows[0].state, UpdateState::UpToDate);
}

#[test]
fn name_and_publisher_with_arch_fallback() {
    let mut a = arp(
        "Mozilla Firefox 150.0 (x64 en-US)",
        "Mozilla Firefox (x64 en-US)",
        "Mozilla",
        "150.0",
    );
    a.product_code = None;
    let rows = correlate(&[a], &catalog(), &NameNormalizer::new()).unwrap();
    assert_eq!(
        rows[0].correlation,
        Correlation::Matched {
            source: "winget".into(),
            package_id: "Mozilla.Firefox".into(),
            rule: Rule::NameAndPublisher
        }
    );
}

#[test]
fn unknown_app_is_unmatched_and_never_upgradable() {
    let rows = correlate(
        &[arp("Zzz", "Totally Unknown Thing", "Nobody", "")],
        &catalog(),
        &NameNormalizer::new(),
    )
    .unwrap();
    assert_eq!(rows[0].correlation, Correlation::Unmatched);
    assert_eq!(rows[0].state, UpdateState::Unknown);
}

#[test]
fn update_state_table() {
    assert_eq!(
        update_state(
            Some("3.13.15150.0"),
            "3.13.15",
            Some("3.13.15150.0"),
            Some("3.13.15150.0")
        ),
        UpdateState::UpToDate
    );
    assert_eq!(
        update_state(
            Some("3.13.14150.0"),
            "3.13.15",
            Some("3.13.15150.0"),
            Some("3.13.15150.0")
        ),
        UpdateState::Available {
            latest: "3.13.15".into()
        }
    );
    assert_eq!(
        update_state(Some("1.0"), "1.0.0", None, None),
        UpdateState::UpToDate
    );
    assert_eq!(
        update_state(Some("1.0"), "1.1", None, None),
        UpdateState::Available {
            latest: "1.1".into()
        }
    );
    assert_eq!(
        update_state(Some("2.0"), "1.1", None, None),
        UpdateState::UpToDate
    );
    assert_eq!(update_state(None, "1.1", None, None), UpdateState::Unknown);
    assert_eq!(
        update_state(Some("  "), "1.1", None, None),
        UpdateState::Unknown
    );
    assert_eq!(
        update_state(Some("unknown"), "1.1", None, None),
        UpdateState::Unknown
    );
}

#[test]
fn ambiguous_is_never_resolved_silently() {
    // Two packages sharing a product code key.
    let mut c = Catalog::open_in_memory().unwrap();
    let p = |id: &str| wingetrs::catalog::NewPackage {
        id: id.into(),
        name: id.into(),
        version: "1".into(),
        ..Default::default()
    };
    let maps = wingetrs::catalog::WingetMaps {
        productcodes: vec![("dup".into(), "A.A".into()), ("dup".into(), "B.B".into())],
        ..Default::default()
    };
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v",
        1,
        &[p("A.A"), p("B.B")],
        &maps,
    )
    .unwrap();
    let rows = correlate(
        &[arp("dup", "Something", "X", "1")],
        &c,
        &NameNormalizer::new(),
    )
    .unwrap();
    match &rows[0].correlation {
        Correlation::Ambiguous { candidates, rule } => {
            let mut c = candidates.clone();
            c.sort();
            assert_eq!(c, ["A.A", "B.B"]);
            assert_eq!(*rule, Rule::ProductCode);
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
    assert_eq!(rows[0].state, UpdateState::Unknown);
}

#[test]
#[ignore = "live: correlates this machine's installs against the real catalog in %LOCALAPPDATA%"]
fn live_list_runs_under_budget() {
    let t = std::time::Instant::now();
    let data = wingetrs::cache::DataDir::default_location().unwrap();
    let cat = Catalog::open(&data.catalog_path()).unwrap();
    let scan = wingetrs::installed::scan_all();
    let rows = correlate(&scan.apps, &cat, &NameNormalizer::new()).unwrap();
    let matched = rows
        .iter()
        .filter(|r| matches!(r.correlation, Correlation::Matched { .. }))
        .count();
    eprintln!(
        "rows={} matched={} elapsed_ms={}",
        rows.len(),
        matched,
        t.elapsed().as_millis()
    );
    assert!(t.elapsed().as_millis() < 500);
}
