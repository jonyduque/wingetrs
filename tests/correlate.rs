#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::catalog::Catalog;
use wingetrs::core::SourceKind;
use wingetrs::installed::correlate::{Correlation, Rule, UpdateState, correlate, update_state};
use wingetrs::installed::normalize::NameNormalizer;
use wingetrs::installed::tiebreak::Tiebreak;
use wingetrs::installed::winget_tracking::{TrackedPackage, Tracking, read_tracking};
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
        stale: None,
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
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
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
    let rows = correlate(
        &[a],
        &catalog(),
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
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
    let rows = correlate(
        &[a],
        &catalog(),
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
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
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
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
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    match &rows[0].correlation {
        Correlation::Ambiguous {
            candidates, rule, ..
        } => {
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
fn msix_is_never_matched_by_name_and_publisher() {
    let mut a = arp("x", "Mozilla Firefox (x64 en-US)", "Mozilla", "1.0.0.0");
    a.source = InstallSource::Msix;
    a.product_code = None;
    a.family_name = Some("Mozilla.Firefox_none".into());
    let rows = correlate(
        &[a],
        &catalog(),
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(rows[0].correlation, Correlation::Unmatched);
    assert_eq!(rows[0].state, UpdateState::Unknown);
}

// Run with: cargo test --release --test correlate -- --ignored live_list
#[test]
#[ignore = "live: correlates this machine's installs against the real catalog in %LOCALAPPDATA%; budget holds only with --release"]
fn live_list_runs_under_budget() {
    let t = std::time::Instant::now();
    let data = wingetrs::cache::DataDir::default_location().unwrap();
    let cat = Catalog::open(&data.catalog_path()).unwrap();
    let scan = wingetrs::installed::scan_all();
    let rows = correlate(
        &scan.apps,
        &cat,
        &NameNormalizer::new(),
        &wingetrs::installed::winget_tracking::load_default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
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

// ---- Task 21: objective tie-breaks for ambiguous correlations ----

/// (id, arp_min, arp_max) packages that all share the product code `dup`.
fn tied_catalog(pkgs: &[(&str, Option<&str>, Option<&str>)]) -> Catalog {
    let mut c = Catalog::open_in_memory().unwrap();
    let packages: Vec<_> = pkgs
        .iter()
        .map(|(id, min, max)| wingetrs::catalog::NewPackage {
            id: (*id).into(),
            name: (*id).into(),
            version: "9".into(),
            arp_min_version: min.map(Into::into),
            arp_max_version: max.map(Into::into),
            ..Default::default()
        })
        .collect();
    let maps = wingetrs::catalog::WingetMaps {
        productcodes: pkgs
            .iter()
            .map(|(id, _, _)| ("dup".into(), (*id).into()))
            .collect(),
        ..Default::default()
    };
    c.replace_source("winget", SourceKind::Winget, "v", 1, &packages, &maps)
        .unwrap();
    c
}

fn tracked(id: &str, product_code: &str) -> TrackedPackage {
    TrackedPackage {
        id: id.into(),
        product_codes: vec![product_code.into()],
        ..Default::default()
    }
}

fn matched(id: &str, rule: Rule) -> Correlation {
    Correlation::Matched {
        source: "winget".into(),
        package_id: id.into(),
        rule,
    }
}

#[test]
fn tiebreak_tracking_picks_the_id_winget_installed() {
    let cat = tied_catalog(&[("Acme.Tool", None, None), ("Other.Tool", None, None)]);
    let tracking = Tracking::new(vec![tracked("Other.Tool", "DUP")]);
    let rows = correlate(
        &[arp("dup", "Tool", "Acme", "1.0")],
        &cat,
        &NameNormalizer::new(),
        &tracking,
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        rows[0].correlation,
        matched("Other.Tool", Rule::ProductCode)
    );
    assert_eq!(rows[0].tiebreak, Some(Tiebreak::WingetTracking));
    assert_eq!(rows[0].latest.as_deref(), Some("9"));
}

#[test]
fn tiebreak_tracking_outside_the_candidates_does_not_resolve() {
    // Ruby case: winget tracked a 4.0 id, the installed product code ties two 3.4 ids.
    let cat = tied_catalog(&[("Ruby.3.4", None, None), ("RubyDevKit.3.4", None, None)]);
    let tracking = Tracking::new(vec![tracked("RubyDevKit.4.0", "dup")]);
    let rows = correlate(
        &[arp(
            "dup",
            "Ruby 3.4.9-1-x64",
            "RubyInstallerTeam",
            "3.4.9-1",
        )],
        &cat,
        &NameNormalizer::new(),
        &tracking,
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        rows[0].correlation,
        Correlation::Ambiguous {
            source: "winget".into(),
            candidates: vec!["Ruby.3.4".into(), "RubyDevKit.3.4".into()],
            rule: Rule::ProductCode
        }
    );
    assert_eq!(rows[0].tiebreak, None);
}

#[test]
fn tiebreak_arp_range_keeps_only_the_candidate_containing_the_version() {
    let cat = tied_catalog(&[
        ("Acme.Old", Some("1.0"), Some("1.9")),
        ("Acme.New", Some("2.0"), Some("2.9")),
    ]);
    let rows = correlate(
        &[arp("dup", "Tool", "Acme", "2.5")],
        &cat,
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(rows[0].correlation, matched("Acme.New", Rule::ProductCode));
    assert_eq!(rows[0].tiebreak, Some(Tiebreak::ArpVersionRange));
    assert_eq!(rows[0].state, UpdateState::Available { latest: "9".into() });
}

#[test]
fn tiebreak_arp_range_never_drops_a_candidate_without_range() {
    let cat = tied_catalog(&[
        ("Acme.Old", Some("1.0"), Some("1.9")),
        ("Acme.Any", None, None),
    ]);
    let rows = correlate(
        &[
            arp("dup", "Tool", "Acme", "1.5"),
            // Outside Acme.Old's range (e.g. an older install): still no evidence for Acme.Any.
            arp("dup", "Tool", "Acme", "0.5"),
        ],
        &cat,
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    for r in &rows {
        assert!(matches!(r.correlation, Correlation::Ambiguous { .. }));
        assert_eq!(r.tiebreak, None);
    }
}

#[test]
fn tiebreak_channel_ffmpeg_display_name_without_suffix_picks_base() {
    let cat = tied_catalog(&[
        ("Gyan.FFmpeg", None, None),
        ("Gyan.FFmpeg.Essentials", None, None),
        ("Gyan.FFmpeg.Shared", None, None),
    ]);
    let rows = correlate(
        &[arp("dup", "FFmpeg", "Gyan", "8.0")],
        &cat,
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        rows[0].correlation,
        matched("Gyan.FFmpeg", Rule::ProductCode)
    );
    assert_eq!(rows[0].tiebreak, Some(Tiebreak::Channel));
}

#[test]
fn tiebreak_channel_display_name_with_suffix_does_not_pick_base() {
    let cat = tied_catalog(&[
        ("Gyan.FFmpeg", None, None),
        ("Gyan.FFmpeg.Essentials", None, None),
        ("Gyan.FFmpeg.Shared", None, None),
    ]);
    let rows = correlate(
        &[arp("dup", "FFmpeg Essentials", "Gyan", "8.0")],
        &cat,
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert!(matches!(rows[0].correlation, Correlation::Ambiguous { .. }));
    assert_eq!(rows[0].tiebreak, None);
}

#[test]
fn tiebreak_channel_suffix_is_case_insensitive_and_needs_a_known_suffix() {
    let cat = tied_catalog(&[
        ("wez.wezterm", None, None),
        ("wez.wezterm.nightly", None, None),
    ]);
    let rows = correlate(
        &[arp(
            "dup",
            "WezTerm version 20240203",
            "Wez Furlong",
            "20240203",
        )],
        &cat,
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        rows[0].correlation,
        matched("wez.wezterm", Rule::ProductCode)
    );
    assert_eq!(rows[0].tiebreak, Some(Tiebreak::Channel));

    let cat = tied_catalog(&[("Acme.Tool", None, None), ("Acme.Tool.Lite", None, None)]);
    let rows = correlate(
        &[arp("dup", "Tool", "Acme", "1.0")],
        &cat,
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert!(matches!(rows[0].correlation, Correlation::Ambiguous { .. }));
}

#[test]
fn tiebreak_without_objective_criterion_stays_ambiguous() {
    let cat = tied_catalog(&[("Acme.Tool", None, None), ("Other.Tool", None, None)]);
    let rows = correlate(
        &[arp("dup", "Tool", "Acme", "1.0")],
        &cat,
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        rows[0].correlation,
        Correlation::Ambiguous {
            source: "winget".into(),
            candidates: vec!["Acme.Tool".into(), "Other.Tool".into()],
            rule: Rule::ProductCode
        }
    );
    assert_eq!(rows[0].tiebreak, None);
    assert_eq!(rows[0].state, UpdateState::Unknown);
}

#[test]
fn read_tracking_reads_winget_installed_db_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("installed.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch(
            "CREATE TABLE ids(rowid INTEGER PRIMARY KEY, id TEXT NOT NULL);
             CREATE TABLE manifest(rowid INTEGER PRIMARY KEY, id INT64 NOT NULL);
             CREATE TABLE productcodes(rowid INTEGER PRIMARY KEY, productcode TEXT NOT NULL);
             CREATE TABLE productcodes_map(manifest INT64 NOT NULL, productcode INT64 NOT NULL);
             CREATE TABLE upgradecodes(rowid INTEGER PRIMARY KEY, upgradecode TEXT NOT NULL);
             CREATE TABLE upgradecodes_map(manifest INT64 NOT NULL, upgradecode INT64 NOT NULL);
             CREATE TABLE pfns(rowid INTEGER PRIMARY KEY, pfn TEXT NOT NULL);
             CREATE TABLE pfns_map(manifest INT64 NOT NULL, pfn INT64 NOT NULL);
             CREATE TABLE norm_names(rowid INTEGER PRIMARY KEY, norm_name TEXT NOT NULL);
             CREATE TABLE norm_names_map(manifest INT64 NOT NULL, norm_name INT64 NOT NULL);
             CREATE TABLE norm_publishers(rowid INTEGER PRIMARY KEY, norm_publisher TEXT NOT NULL);
             CREATE TABLE norm_publishers_map(manifest INT64 NOT NULL, norm_publisher INT64 NOT NULL);
             INSERT INTO ids VALUES (1, 'Gyan.FFmpeg'), (2, 'Acme.Tool');
             INSERT INTO manifest VALUES (10, 1), (11, 2);
             INSERT INTO productcodes VALUES (1, 'gyan.ffmpeg_microsoft.winget.source_8wekyb3d8bbwe');
             INSERT INTO productcodes_map VALUES (10, 1);
             INSERT INTO norm_names VALUES (1, 'tool');
             INSERT INTO norm_names_map VALUES (11, 1);
             INSERT INTO norm_publishers VALUES (1, 'acme');
             INSERT INTO norm_publishers_map VALUES (11, 1);",
        )
        .unwrap();
    }
    let before = std::fs::read(&p).unwrap();
    let tracking = read_tracking(&p).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), before);

    let n = NameNormalizer::new();
    let mut ff = arp(
        "Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe",
        "FFmpeg",
        "Gyan",
        "8.0",
    );
    assert_eq!(tracking.ids_for(&ff, &n), ["Gyan.FFmpeg"]);
    ff.product_code = Some("other".into());
    assert!(tracking.ids_for(&ff, &n).is_empty());
    let tool = arp("x", "Tool", "Acme", "1.0");
    assert_eq!(tracking.ids_for(&tool, &n), ["Acme.Tool"]);
    let wrong_publisher = arp("x", "Tool", "Nobody", "1.0");
    assert!(tracking.ids_for(&wrong_publisher, &n).is_empty());
}

#[test]
fn read_tracking_missing_file_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    assert!(read_tracking(&dir.path().join("absent.db")).is_err());
}

#[test]
fn tracking_header_wal_detection() {
    use wingetrs::installed::winget_tracking::header_is_wal;
    let mut h = [0u8; 100];
    h[..16].copy_from_slice(b"SQLite format 3\0");
    h[18] = 1;
    h[19] = 1;
    assert!(!header_is_wal(&h));
    h[18] = 2;
    h[19] = 2;
    assert!(header_is_wal(&h));
    assert!(!header_is_wal(&h[..10]));
    assert!(!header_is_wal(&[]));
}

#[test]
fn read_tracking_refuses_wal_catalogs_without_creating_files() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("installed.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE ids(rowid INTEGER PRIMARY KEY, id TEXT NOT NULL);")
            .unwrap();
    }
    let before: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(read_tracking(&p).is_err());
    let after: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(before, after);
}

fn ambiguous_pair() -> (Catalog, InstalledApp) {
    (
        tied_catalog(&[("Acme.Tool", None, None), ("Other.Tool", None, None)]),
        arp("dup", "Tool", "Acme", "1.0"),
    )
}

fn run_with(
    cat: &Catalog,
    app: &InstalledApp,
    choice: wingetrs::installed::overrides::Choice,
) -> wingetrs::installed::correlate::ListRow {
    let overrides =
        std::collections::BTreeMap::from([(wingetrs::installed::overrides::app_key(app), choice)]);
    correlate(
        std::slice::from_ref(app),
        cat,
        &NameNormalizer::new(),
        &Tracking::default(),
        &overrides,
    )
    .unwrap()
    .remove(0)
}

#[test]
fn override_resolves_ambiguous() {
    use wingetrs::installed::overrides::Choice;
    let (cat, app) = ambiguous_pair();
    let row = run_with(
        &cat,
        &app,
        Choice::Package {
            source: "winget".into(),
            package_id: "Other.Tool".into(),
        },
    );
    assert_eq!(row.correlation, matched("Other.Tool", Rule::User));
    assert_eq!(row.latest.as_deref(), Some("9"));
    assert_eq!(row.state, UpdateState::Available { latest: "9".into() });
    assert_eq!(row.app_key, "arp/hklm64/dup");
}

#[test]
fn override_not_in_catalog_is_unmatched() {
    let (cat, app) = ambiguous_pair();
    let row = run_with(
        &cat,
        &app,
        wingetrs::installed::overrides::Choice::NotInCatalog,
    );
    assert_eq!(row.correlation, Correlation::Unmatched);
    assert!(row.not_in_catalog);
    assert_eq!(row.state, UpdateState::Unknown);
}

#[test]
fn override_to_missing_package_falls_back() {
    let (cat, app) = ambiguous_pair();
    let row = run_with(
        &cat,
        &app,
        wingetrs::installed::overrides::Choice::Package {
            source: "winget".into(),
            package_id: "Gone.Pkg".into(),
        },
    );
    assert!(matches!(row.correlation, Correlation::Ambiguous { .. }));
}

#[test]
fn cli_list_uses_overrides() {
    use wingetrs::installed::overrides::{self, Choice};
    let (cat, app) = ambiguous_pair();
    let dir = tempfile::tempdir().unwrap();
    let data = wingetrs::cache::DataDir::new(dir.path());
    let rows = |data: &wingetrs::cache::DataDir| {
        correlate(
            std::slice::from_ref(&app),
            &cat,
            &NameNormalizer::new(),
            &Tracking::default(),
            &overrides::load(data),
        )
        .unwrap()
    };
    let before = wingetrs::cli::output::list_text(&rows(&data), false);
    assert!(before.contains("Ambíguos (1)"), "{before}");
    overrides::set(
        &data,
        &overrides::app_key(&app),
        Some(Choice::Package {
            source: "winget".into(),
            package_id: "Acme.Tool".into(),
        }),
    )
    .unwrap();
    let after = wingetrs::cli::output::list_text(&rows(&data), false);
    assert!(!after.contains("Ambíguos"), "{after}");
    assert!(after.contains("Acme.Tool"), "{after}");
}

fn kindle(hive: Hive, name: &str, version: &str) -> InstalledApp {
    InstalledApp {
        source: InstallSource::Arp(hive),
        key: "Amazon Kindle".into(),
        product_code: Some("{23170F69-40C1-2701-1900-000001000000}".into()),
        ..arp("Amazon Kindle", name, "Amazon", version)
    }
}

fn run(apps: &[InstalledApp]) -> Vec<wingetrs::installed::correlate::ListRow> {
    correlate(
        apps,
        &catalog(),
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap()
}

#[test]
fn same_name_rows_become_one_with_newest_first() {
    let rows = run(&[
        kindle(Hive::MachineX86, "Amazon Kindle", "2.8.3.70995"),
        kindle(Hive::User, "Amazon kindle", "2.9.1.71006"),
    ]);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let r = &rows[0];
    assert_eq!(r.installed_version.as_deref(), Some("2.9.1.71006"));
    assert_eq!(r.location, "usuário");
    assert_eq!(r.others.len(), 1);
    let o = &r.others[0];
    assert_eq!(o.location, "máquina 32-bit");
    assert_eq!(o.installed_version.as_deref(), Some("2.8.3.70995"));
    assert_eq!(o.app_key, "arp/hklm32/Amazon Kindle");
    let json = serde_json::to_string(&rows).unwrap();
    assert!(json.contains("\"others\""), "{json}");
}

#[test]
fn tie_prefers_matched_then_first() {
    let mut unmatched = kindle(Hive::MachineX64, "Tool", "1.0");
    unmatched.product_code = None;
    unmatched.key = "other".into();
    let matched = kindle(Hive::User, "Tool", "1.0");
    let rows = run(&[unmatched.clone(), matched.clone()]);
    assert_eq!(rows.len(), 1);
    assert!(matches!(rows[0].correlation, Correlation::Matched { .. }));
    assert_eq!(rows[0].location, "usuário");
    assert_eq!(rows[0].others[0].location, "máquina 64-bit");
    // Two equal rows keep the first.
    let rows = run(&[
        kindle(Hive::MachineX64, "Tool", "1.0"),
        kindle(Hive::User, "Tool", "1.0"),
    ]);
    assert_eq!(rows[0].location, "máquina 64-bit");
}

#[test]
fn app_keys_differ_by_hive() {
    use wingetrs::installed::overrides::app_key;
    let x = |h| InstalledApp {
        key: "X".into(),
        ..kindle(h, "X", "1")
    };
    assert_eq!(app_key(&x(Hive::MachineX86)), "arp/hklm32/X");
    assert_eq!(app_key(&x(Hive::User)), "arp/hkcu/X");
    assert_eq!(app_key(&x(Hive::MachineX64)), "arp/hklm64/X");
}

#[test]
fn cli_list_shows_extra_records() {
    let rows = run(&[
        kindle(Hive::MachineX86, "Amazon Kindle", "2.8.3.70995"),
        kindle(Hive::User, "Amazon Kindle", "2.9.1.71006"),
    ]);
    let text = wingetrs::cli::output::list_text(&rows, false);
    assert!(text.contains("+1 registro"), "{text}");
}

#[test]
fn wingetrs_record_always_leads() {
    let mut scoop = kindle(Hive::User, "7zip", "26.04");
    scoop.source = InstallSource::Scoop;
    let mut ours = kindle(Hive::User, "7zip", "26.03");
    ours.source = InstallSource::Wingetrs;
    let rows = run(&[scoop, ours]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].origin, "wingetrs");
    assert_eq!(rows[0].others[0].origin, "scoop");
}
