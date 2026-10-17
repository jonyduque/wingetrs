//! Whether winget would take an upgrade: UpgradeBehavior, installed scope, RequireExplicitUpgrade.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use wingetrs::bridge::{Applicability, WingetFit, winget_applicability};
use wingetrs::catalog::Catalog;
use wingetrs::installed::correlate::{
    Correlation, InstallScope, ListRow, Rule, UpdateState, correlate,
};
use wingetrs::installed::normalize::NameNormalizer;
use wingetrs::installed::winget_tracking::Tracking;
use wingetrs::installed::{Hive, InstallSource, InstalledApp};
use wingetrs::sources::winget::manifest::{MergedManifest, parse};
use wingetrs::tui::app::{App, Cmd, Modal, Msg, SourceFilter, Tab, update};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Glyphs, Theme};
use wingetrs::tui::view::view;

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn manifest(root: &str, installers: &[&str]) -> MergedManifest {
    let mut y = format!("PackageIdentifier: A.B\nPackageVersion: '2'\n{root}Installers:\n");
    for i in installers {
        y.push_str(&format!(
            "- InstallerUrl: https://x/i.exe\n  InstallerSha256: {SHA}\n{i}"
        ));
    }
    parse(y.as_bytes()).unwrap()
}

fn fit(m: &MergedManifest, scope: Option<InstallScope>) -> WingetFit {
    winget_applicability(m, scope, "x64")
}

#[test]
fn android_studio_denied() {
    let m = manifest(
        "UpgradeBehavior: deny\nRequireExplicitUpgrade: true\nScope: machine\n",
        &["  Architecture: x64\n"],
    );
    let f = fit(&m, Some(InstallScope::Machine));
    assert_eq!(f.applicability, Applicability::Denied);
    assert!(f.explicit_only);
    assert_eq!(fit(&m, None).applicability, Applicability::Denied);
}

#[test]
fn installer_overrides_root_upgrade_behavior() {
    let allowed = manifest(
        "UpgradeBehavior: deny\n",
        &["  Architecture: x64\n  UpgradeBehavior: install\n"],
    );
    let f = fit(&allowed, None);
    assert_eq!(f.applicability, Applicability::Ok);
    assert!(!f.explicit_only);
    let denied = manifest(
        "UpgradeBehavior: install\n",
        &["  Architecture: x64\n  UpgradeBehavior: deny\n"],
    );
    assert_eq!(fit(&denied, None).applicability, Applicability::Denied);
    // `RequireExplicitUpgrade` follows the same precedence.
    let explicit = manifest(
        "RequireExplicitUpgrade: false\n",
        &["  Architecture: x64\n  RequireExplicitUpgrade: true\n"],
    );
    assert!(fit(&explicit, None).explicit_only);
    let not_explicit = manifest(
        "RequireExplicitUpgrade: true\n",
        &["  Architecture: x64\n  RequireExplicitUpgrade: false\n"],
    );
    assert!(!fit(&not_explicit, None).explicit_only);
}

#[test]
fn one_allowed_installer_is_enough() {
    let m = manifest(
        "",
        &[
            "  Architecture: x64\n  UpgradeBehavior: deny\n",
            "  Architecture: x86\n",
        ],
    );
    assert_eq!(fit(&m, None).applicability, Applicability::Ok);
    // A deny installer of another architecture does not count.
    let m = manifest(
        "",
        &[
            "  Architecture: arm64\n  UpgradeBehavior: deny\n",
            "  Architecture: x64\n",
        ],
    );
    assert_eq!(fit(&m, None).applicability, Applicability::Ok);
    // Nor does `RequireExplicitUpgrade` of another architecture.
    let m = manifest(
        "",
        &[
            "  Architecture: arm64\n  RequireExplicitUpgrade: true\n",
            "  Architecture: x64\n",
        ],
    );
    assert!(!fit(&m, None).explicit_only);
}

#[test]
fn anki_scope_mismatch() {
    let m = manifest(
        "",
        &[
            "  Architecture: x64\n  Scope: user\n",
            "  Architecture: arm64\n  Scope: user\n",
        ],
    );
    assert_eq!(
        fit(&m, Some(InstallScope::Machine)).applicability,
        Applicability::ScopeMismatch {
            installed: InstallScope::Machine,
            offered: InstallScope::User
        }
    );
    // The root scope applies to installers without their own.
    let m = manifest("Scope: machine\n", &["  Architecture: x64\n"]);
    assert_eq!(
        fit(&m, Some(InstallScope::User)).applicability,
        Applicability::ScopeMismatch {
            installed: InstallScope::User,
            offered: InstallScope::Machine
        }
    );
    // One installer of the installed scope is enough.
    let m = manifest(
        "",
        &[
            "  Architecture: x64\n  Scope: user\n",
            "  Architecture: x64\n  Scope: machine\n",
        ],
    );
    assert_eq!(
        fit(&m, Some(InstallScope::Machine)).applicability,
        Applicability::Ok
    );
}

#[test]
fn unscoped_installer_fits_any_scope() {
    let m = manifest("", &["  Architecture: x64\n"]);
    for s in [InstallScope::Machine, InstallScope::User] {
        assert_eq!(fit(&m, Some(s)).applicability, Applicability::Ok);
    }
    // Next to an installer of the other scope, the unscoped one still fits.
    let m = manifest(
        "",
        &[
            "  Architecture: x64\n",
            "  Architecture: x86\n  Scope: user\n",
        ],
    );
    assert_eq!(
        fit(&m, Some(InstallScope::Machine)).applicability,
        Applicability::Ok
    );
}

#[test]
fn unknown_scope_is_ok() {
    let m = manifest("", &["  Architecture: x64\n  Scope: user\n"]);
    assert_eq!(fit(&m, None).applicability, Applicability::Ok);
}

#[test]
fn no_installer_for_arch_is_unknown() {
    let m = manifest("UpgradeBehavior: deny\n", &["  Architecture: arm64\n"]);
    assert_eq!(fit(&m, None).applicability, Applicability::Unknown);
    let none = manifest("", &[]);
    assert_eq!(fit(&none, None).applicability, Applicability::Unknown);
    // x86 and neutral installers run on x64; x64 runs on arm64.
    for arch in ["x86", "neutral"] {
        let m = manifest("", &[&format!("  Architecture: {arch}\n")]);
        assert_eq!(fit(&m, None).applicability, Applicability::Ok, "{arch}");
    }
    let m = manifest("", &["  Architecture: x64\n"]);
    assert_eq!(
        winget_applicability(&m, None, "arm64").applicability,
        Applicability::Ok
    );
    assert_eq!(
        winget_applicability(&m, None, "x86").applicability,
        Applicability::Unknown
    );
}

fn app_of(hive: Option<Hive>, name: &str) -> InstalledApp {
    InstalledApp {
        source: hive.map_or(InstallSource::Msix, InstallSource::Arp),
        key: name.into(),
        name: name.into(),
        publisher: None,
        version: Some("1".into()),
        product_code: None,
        upgrade_code: None,
        family_name: None,
        scoop_bucket: None,
        stale: None,
    }
}

#[test]
fn row_scope_from_hive() {
    let apps = [
        app_of(Some(Hive::MachineX64), "a"),
        app_of(Some(Hive::MachineX86), "b"),
        app_of(Some(Hive::User), "c"),
        app_of(None, "d"),
    ];
    let rows = correlate(
        &apps,
        &Catalog::open_in_memory().unwrap(),
        &NameNormalizer::new(),
        &Tracking::default(),
        &std::collections::BTreeMap::new(),
    )
    .unwrap();
    let scopes: Vec<_> = rows.iter().map(|r| (r.name.as_str(), r.scope)).collect();
    assert_eq!(
        scopes,
        [
            ("a", Some(InstallScope::Machine)),
            ("b", Some(InstallScope::Machine)),
            ("c", Some(InstallScope::User)),
            ("d", None),
        ]
    );
}

fn brow(name: &str) -> ListRow {
    ListRow {
        name: name.into(),
        app_key: format!("arp:{name}"),
        origin: "arp".into(),
        installed_version: Some("1".into()),
        correlation: Correlation::Matched {
            source: "winget".into(),
            package_id: format!("Pub.{name}"),
            rule: Rule::ProductCode,
        },
        latest: Some("2".into()),
        state: UpdateState::Available { latest: "2".into() },
        scope: Some(InstallScope::Machine),
        ..Default::default()
    }
}

fn loaded(name: &str, result: Result<WingetFit, String>) -> Msg {
    Msg::ApplicabilityLoaded {
        app_key: format!("arp:{name}"),
        latest: "2".into(),
        result,
    }
}

fn verdict(applicability: Applicability, explicit_only: bool) -> Result<WingetFit, String> {
    Ok(WingetFit {
        applicability,
        explicit_only,
    })
}

fn menu_upgrade(a: &mut App) -> (String, bool, Option<String>) {
    a.modal = None;
    update(
        a,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    match &a.modal {
        Some(Modal::Actions { picker, .. }) => {
            let i = &picker.items()[0];
            (i.label.clone(), i.selectable, i.note.clone())
        }
        _ => panic!("actions menu not open"),
    }
}

fn installed(rows: Vec<ListRow>) -> App {
    let mut a = App::new(Vec::new(), GlyphSet::Nerd);
    a.tab = Tab::Installed;
    a.installed = rows;
    a
}

#[test]
fn installed_rows_ask_for_applicability() {
    let mut a = App::new(Vec::new(), GlyphSet::Nerd);
    let ours = ListRow {
        origin: "wingetrs".into(),
        ..brow("Ours")
    };
    let current = ListRow {
        state: UpdateState::UpToDate,
        ..brow("Current")
    };
    let scoop = ListRow {
        origin: "scoop".into(),
        ..brow("Scooped")
    };
    let cmds = update(
        &mut a,
        Msg::InstalledLoaded(vec![brow("Git"), ours, current, scoop]),
    );
    let asked: Vec<_> = cmds
        .iter()
        .filter_map(|c| match c {
            Cmd::FetchApplicability { rows } => Some(rows.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(asked.len(), 1, "{cmds:?}");
    assert_eq!(asked[0].len(), 1);
    let q = &asked[0][0];
    assert_eq!(
        (
            q.app_key.as_str(),
            q.package_id.as_str(),
            q.latest.as_str(),
            q.scope
        ),
        ("arp:Git", "Pub.Git", "2", Some(InstallScope::Machine))
    );
}

#[test]
fn menu_note_for_denied_and_mismatch() {
    let enabled = ("Atualizar (via winget)".to_string(), true, None);
    let mut a = installed(vec![brow("Studio")]);
    assert_eq!(menu_upgrade(&mut a), enabled);
    update(
        &mut a,
        loaded("Studio", verdict(Applicability::Denied, true)),
    );
    assert_eq!(
        menu_upgrade(&mut a),
        (
            "Atualizar".to_string(),
            false,
            Some("o editor não permite atualizar pelo winget".to_string())
        )
    );
    update(
        &mut a,
        loaded(
            "Studio",
            verdict(
                Applicability::ScopeMismatch {
                    installed: InstallScope::Machine,
                    offered: InstallScope::User,
                },
                false,
            ),
        ),
    );
    assert_eq!(
        menu_upgrade(&mut a),
        (
            "Atualizar".to_string(),
            false,
            Some("a versão nova só instala para usuário; a instalada é para máquina".to_string())
        )
    );
    update(
        &mut a,
        loaded(
            "Studio",
            verdict(
                Applicability::ScopeMismatch {
                    installed: InstallScope::User,
                    offered: InstallScope::Machine,
                },
                false,
            ),
        ),
    );
    assert_eq!(
        menu_upgrade(&mut a).2.as_deref(),
        Some("a versão nova só instala para máquina; a instalada é para usuário")
    );
    // `explicit_only` alone and `Unknown` leave the item as it is.
    update(&mut a, loaded("Studio", verdict(Applicability::Ok, true)));
    assert_eq!(menu_upgrade(&mut a), enabled);
    update(
        &mut a,
        loaded("Studio", verdict(Applicability::Unknown, false)),
    );
    assert_eq!(menu_upgrade(&mut a), enabled);
    // A verdict for another catalog version does not apply.
    update(
        &mut a,
        loaded("Studio", verdict(Applicability::Denied, false)),
    );
    a.installed[0].state = UpdateState::Available { latest: "3".into() };
    a.installed[0].latest = Some("3".into());
    assert_eq!(menu_upgrade(&mut a), enabled);
}

/// "Atualizar tudo" (`U` with no row in view) only ever plans wingetrs's own apps: no bridge
/// row rides along, `explicit_only` or not. The `explicit_only` one still upgrades alone.
#[test]
fn explicit_only_left_out_of_upgrade_all() {
    let mut a = installed(vec![brow("Studio"), brow("Git")]);
    a.tab = Tab::Updates;
    update(&mut a, loaded("Studio", verdict(Applicability::Ok, true)));
    update(&mut a, loaded("Git", verdict(Applicability::Ok, false)));
    // Rows hidden by the source filter: the view is empty, as when `U` plans everything.
    a.filter = SourceFilter::Scoop;
    assert_eq!(a.visible_len(), 0);
    let u = || Msg::Key(KeyEvent::new(KeyCode::Char('U'), KeyModifiers::NONE));
    assert_eq!(
        update(&mut a, u()),
        vec![Cmd::PlanUpgradeAll { keep: false }]
    );
    assert!(a.modal.is_none() && a.queue.is_empty());
    let mut alone = installed(vec![brow("Studio")]);
    alone.tab = Tab::Updates;
    update(
        &mut alone,
        loaded("Studio", verdict(Applicability::Ok, true)),
    );
    assert!(update(&mut alone, u()).is_empty());
    match &alone.modal {
        Some(Modal::Confirm { items, .. }) => {
            assert_eq!(items.len(), 1);
            assert_eq!(
                (items[0].name.as_str(), items[0].manager),
                ("Studio", "winget")
            );
        }
        _ => panic!("confirmation not open"),
    }
}

#[test]
fn fetch_failure_keeps_today_behavior() {
    let mut a = installed(vec![brow("Git")]);
    update(&mut a, loaded("Git", Err("sem rede".into())));
    assert_eq!(
        menu_upgrade(&mut a),
        ("Atualizar (via winget)".to_string(), true, None)
    );
    // A failure after a verdict does not keep the old verdict either.
    update(&mut a, loaded("Git", verdict(Applicability::Denied, false)));
    update(&mut a, loaded("Git", Err("sem rede".into())));
    assert!(menu_upgrade(&mut a).1);
}

/// Foreground and boldness of the first `needle` after the first `anchor` on its line.
fn styled(a: &App, anchor: &str, needle: &str) -> (Option<Color>, bool) {
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let mut t = Terminal::new(TestBackend::new(100, 20)).unwrap();
    t.draw(|f| view(a, &theme, f)).unwrap();
    let buf = t.backend().buffer().clone();
    for y in 0..buf.area.height {
        let cells: Vec<String> = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect();
        let Some(start) = (0..cells.len()).find(|&x| cells[x..].concat().starts_with(anchor))
        else {
            continue;
        };
        let x = (start..cells.len())
            .find(|&x| cells[x..].concat().starts_with(needle))
            .unwrap_or_else(|| panic!("{needle} not after {anchor}: {}", cells.concat()));
        let c = &buf[(x as u16, y)];
        return (Some(c.fg), c.modifier.contains(Modifier::BOLD));
    }
    panic!("{anchor} not on screen");
}

#[test]
fn refused_upgrade_is_drawn_muted() {
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let muted = (theme.muted().fg, false);
    let row = || ListRow {
        state: UpdateState::Available {
            latest: "9.8.7".into(),
        },
        latest: Some("9.8.7".into()),
        ..brow("Studio")
    };
    let denied = |a: &mut App| {
        update(
            a,
            Msg::ApplicabilityLoaded {
                app_key: "arp:Studio".into(),
                latest: "9.8.7".into(),
                result: verdict(Applicability::Denied, false),
            },
        );
    };
    // Updates: the new version.
    let mut a = installed(vec![row()]);
    a.tab = Tab::Updates;
    assert_eq!(styled(&a, "Studio", "9.8.7"), (theme.ok().fg, true));
    denied(&mut a);
    assert_eq!(styled(&a, "Studio", "9.8.7"), muted);
    // Installed: the state glyph after the installed version.
    let mut a = installed(vec![row()]);
    let up = Glyphs::for_set(GlyphSet::Nerd).upgrade;
    assert_eq!(styled(&a, "Studio", up).0, theme.warn().fg);
    denied(&mut a);
    assert_eq!(styled(&a, "Studio", up), muted);
}

// Run with: cargo test --release --test it -- --ignored --nocapture live_applicability
#[test]
#[ignore = "live: scans this machine, reads a copy of the real catalog and fetches manifests"]
fn live_applicability_of_installed_winget_rows() {
    use wingetrs::bridge::{applicability_note, host_arch};
    let data = wingetrs::cache::DataDir::default_location().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("catalog.db");
    std::fs::copy(data.catalog_path(), &copy).unwrap();
    let cat = Catalog::open_read_only(&copy).unwrap();
    let scan = wingetrs::installed::scan_all();
    let rows = correlate(
        &scan.apps,
        &cat,
        &NameNormalizer::new(),
        &wingetrs::installed::winget_tracking::load_default(),
        &wingetrs::installed::overrides::load(&data),
    )
    .unwrap();
    let blobs = wingetrs::cache::BlobStore::new(dir.path().join("blobs"));
    let http = wingetrs::http::Http::new().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    for r in &rows {
        let (
            Correlation::Matched {
                source, package_id, ..
            },
            UpdateState::Available { latest },
        ) = (&r.correlation, &r.state)
        else {
            continue;
        };
        if source != "winget" || !matches!(r.origin.as_str(), "arp" | "msix") {
            continue;
        }
        let p = cat.get("winget", package_id).unwrap().unwrap();
        let got = rt.block_on(wingetrs::sources::winget::details::merged_manifest(
            &http,
            &blobs,
            wingetrs::sources::winget::CDN_BASE,
            package_id,
            p.winget_hash.as_deref().unwrap(),
            Some(latest),
        ));
        match got {
            Ok((m, _)) => {
                let f = winget_applicability(&m, r.scope, host_arch());
                eprintln!(
                    "{package_id} {latest} scope={:?}: {:?} explicit_only={} note={:?}",
                    r.scope,
                    f.applicability,
                    f.explicit_only,
                    applicability_note(f.applicability)
                );
            }
            Err(e) => eprintln!("{package_id} {latest}: {e}"),
        }
    }
}
