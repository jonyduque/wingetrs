#![allow(clippy::unwrap_used, clippy::expect_used)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use wingetrs::core::{PackageEntry, SourceKind};
use wingetrs::tui::app::{App, Cmd, Msg, SourceFilter, Tab, update};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Glyphs, detect_depth};

fn entry(source: SourceKind, id: &str, name: &str) -> PackageEntry {
    PackageEntry {
        source,
        source_name: if source == SourceKind::Winget {
            "winget".into()
        } else {
            "scoop:main".into()
        },
        id: id.into(),
        name: name.into(),
        moniker: None,
        version: "1".into(),
        publisher: None,
        description: None,
        tags: vec![],
        commands: vec![],
    }
}
fn app_entries() -> Vec<PackageEntry> {
    vec![
        entry(SourceKind::Winget, "Microsoft.PowerToys", "PowerToys"),
        entry(SourceKind::Scoop, "ripgrep", "ripgrep"),
        entry(SourceKind::Winget, "Zeal.Zeal", "Zeal"),
    ]
}
fn app() -> App {
    App::new(app_entries(), GlyphSet::Nerd)
}
fn key(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}
fn ctrl(c: char) -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

#[test]
fn typing_filters_and_resets_selection() {
    let mut a = app();
    update(&mut a, key(KeyCode::Down));
    assert_eq!(a.selected, 1);
    for c in "zeal".chars() {
        step(&mut a, key(KeyCode::Char(c)));
    }
    assert_eq!(a.query, "zeal");
    assert_eq!(a.selected, 0);
    assert_eq!(a.visible_len(), 1);
}

#[test]
fn j_k_are_text_when_query_is_not_empty() {
    let mut a = app();
    update(&mut a, key(KeyCode::Char('j')));
    assert_eq!(a.query, "", "j moves when query is empty");
    assert_eq!(a.selected, 1);
    update(&mut a, key(KeyCode::Char('z')));
    update(&mut a, key(KeyCode::Char('j')));
    assert_eq!(a.query, "zj");
}

#[test]
fn escape_ladder_and_quit() {
    let mut a = app();
    step(&mut a, key(KeyCode::Char('x')));
    // Clearing the query changes the selection, so it asks for the new row's details.
    assert_eq!(step(&mut a, key(KeyCode::Esc)), vec![details_for(&a)]);
    assert_eq!(a.query, "");
    assert_eq!(update(&mut a, key(KeyCode::Esc)), vec![Cmd::Quit]);
    assert_eq!(update(&mut app(), ctrl('c')), vec![Cmd::Quit]);
}

#[test]
fn tabs_cycle_and_filter_toggles() {
    let mut a = app();
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Installed);
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Updates);
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Settings);
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Search);
    update(&mut a, key(KeyCode::BackTab));
    assert_eq!(a.tab, Tab::Settings);
    let mut b = app();
    step(&mut b, ctrl('s'));
    assert_eq!(b.filter, SourceFilter::Winget);
    assert_eq!(b.visible_len(), 2);
}

#[test]
fn selection_change_requests_details_once() {
    let mut a = app();
    let cmds = update(&mut a, key(KeyCode::Down));
    assert!(
        matches!(cmds.as_slice(), [Cmd::LoadDetails { .. }]),
        "{cmds:?}"
    );
    let (source, id) = a.selected_key().unwrap();
    update(
        &mut a,
        Msg::DetailsLoaded {
            key: format!("{source}/{id}"),
            result: Err("x".into()),
        },
    );
    update(&mut a, key(KeyCode::Up));
    let cmds = update(&mut a, key(KeyCode::Down));
    assert!(
        cmds.is_empty(),
        "cached details must not be reloaded: {cmds:?}"
    );
}

#[test]
fn ctrl_r_refreshes_and_selection_is_clamped() {
    let mut a = app();
    assert_eq!(
        update(&mut a, ctrl('r')),
        vec![Cmd::Refresh { force: true }]
    );
    for _ in 0..10 {
        update(&mut a, key(KeyCode::Down));
    }
    assert_eq!(a.selected, 2);
    update(&mut a, key(KeyCode::Home));
    assert_eq!(a.selected, 0);
}

#[test]
fn depth_detection() {
    assert_eq!(detect_depth(Some("truecolor"), None), ColorDepth::TrueColor);
    assert_eq!(detect_depth(Some("24bit"), None), ColorDepth::TrueColor);
    assert_eq!(detect_depth(None, None), ColorDepth::Ansi256);
    assert_eq!(
        detect_depth(Some("truecolor"), Some("1")),
        ColorDepth::NoColor
    );
    assert_eq!(
        detect_depth(None, Some("")),
        ColorDepth::Ansi256,
        "empty NO_COLOR is ignored"
    );
}

#[test]
fn glyph_sets_are_distinct_and_ascii_is_ascii() {
    let a = Glyphs::for_set(GlyphSet::Ascii);
    for g in [
        a.winget,
        a.scoop,
        a.search,
        a.ok,
        a.upgrade,
        a.unknown,
        a.ambiguous,
        a.refresh,
        a.offline,
        a.license,
        a.publisher,
        a.id,
        a.selected,
    ] {
        assert!(g.is_ascii(), "{g:?}");
    }
    assert_eq!(Glyphs::for_set(GlyphSet::Nerd).winget, "\u{f17a}");
    assert_eq!(Glyphs::for_set(GlyphSet::Unicode).upgrade, "↑");
}

fn row(
    name: &str,
    state: wingetrs::installed::correlate::UpdateState,
) -> wingetrs::installed::correlate::ListRow {
    use wingetrs::installed::correlate::{Correlation, ListRow, Rule};
    ListRow {
        name: name.into(),
        app_key: String::new(),
        not_in_catalog: false,
        origin: "arp".into(),
        installed_version: Some("1".into()),
        correlation: Correlation::Matched {
            source: "winget".into(),
            package_id: format!("Pkg.{name}"),
            rule: Rule::ProductCode,
        },
        latest: None,
        state,
        tiebreak: None,
    }
}

#[test]
fn no_color_keeps_modifiers_and_truecolor_uses_rgb() {
    use ratatui::style::{Color, Modifier};
    use wingetrs::tui::theme::Theme;
    let t = Theme::new(true, ColorDepth::NoColor);
    assert!(t.name().add_modifier.contains(Modifier::BOLD));
    assert_eq!(t.name().fg, None);
    assert!(t.description().add_modifier.contains(Modifier::ITALIC));
    assert!(t.matched().add_modifier.contains(Modifier::UNDERLINED));
    assert_eq!(t.matched().fg, None);
    let c = Theme::new(true, ColorDepth::TrueColor);
    assert!(matches!(c.accent().fg, Some(Color::Rgb(..))));
}

#[test]
fn selection_is_clamped_when_installed_list_shrinks() {
    use wingetrs::installed::correlate::UpdateState;
    let mut a = app();
    update(&mut a, key(KeyCode::Tab));
    let rows: Vec<_> = (0..5)
        .map(|i| row(&format!("p{i}"), UpdateState::UpToDate))
        .collect();
    update(&mut a, Msg::InstalledLoaded(rows));
    update(&mut a, key(KeyCode::End));
    assert_eq!(a.selected, 4);
    let rows: Vec<_> = (0..2)
        .map(|i| row(&format!("p{i}"), UpdateState::UpToDate))
        .collect();
    update(&mut a, Msg::InstalledLoaded(rows));
    assert!(a.selected < a.visible_len());
    assert!(a.selected_key().is_some());
}

#[test]
fn updates_tab_counts_only_available() {
    use wingetrs::installed::correlate::UpdateState;
    let mut a = app();
    update(
        &mut a,
        Msg::InstalledLoaded(vec![
            row("a", UpdateState::Available { latest: "2".into() }),
            row("b", UpdateState::UpToDate),
            row("c", UpdateState::Unknown),
            row("d", UpdateState::Available { latest: "3".into() }),
        ]),
    );
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.visible_len(), 4);
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Updates);
    assert_eq!(a.visible_len(), 2);
}

#[test]
fn refresh_busy_is_not_offline_and_says_in_progress() {
    let mut a = app();
    update(&mut a, Msg::RefreshStarted);
    update(&mut a, Msg::RefreshBusy);
    assert!(!a.refreshing);
    assert!(!a.offline);
    assert_eq!(a.toasts[0].kind, wingetrs::tui::app::ToastKind::Info);
    let status = a.toasts[0].text.clone();
    assert!(status.contains("atualização em andamento"), "{status}");
}

fn failed_report(offline: bool) -> Vec<wingetrs::update::SourceReport> {
    vec![wingetrs::update::SourceReport {
        source: "winget".into(),
        outcome: wingetrs::update::Outcome::Failed {
            message: "x".into(),
            offline,
        },
    }]
}

#[test]
fn only_network_failures_mark_offline() {
    let mut a = app();
    update(&mut a, Msg::RefreshFinished(failed_report(false)));
    assert!(!a.offline);
    assert!(!a.toasts.is_empty());
    update(&mut a, Msg::RefreshFinished(failed_report(true)));
    assert!(a.offline);
}

#[test]
fn installed_failure_shows_status() {
    let mut a = app();
    update(&mut a, Msg::InstalledFailed("pânico interno".into()));
    let status = a.toasts[0].text.clone();
    assert!(status.contains("falha ao ler instalados"), "{status}");
}
// ---- install / remove / upgrade (Task 39) ----

use std::path::PathBuf;
use wingetrs::install::plan::{InstallPlan, UninstallPlan};
use wingetrs::install::residue::{Confidence, Residue, ResidueKind};
use wingetrs::install::target::Candidate;
use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
use wingetrs::tui::app::{Action, Done, InstallFail, Modal, Planned};

fn plan(app: &str) -> InstallPlan {
    InstallPlan {
        app: app.into(),
        bucket: "main".into(),
        version: "2".into(),
        arch: None,
        upgrade_from: None,
        root: PathBuf::from("C:/w"),
        manifest_sha256: "x".into(),
        steps: vec![],
    }
}

fn apply_action(app: &str) -> Action {
    Action::Apply(vec![vec![plan(app)]])
}

fn ready(app: &str) -> Msg {
    Msg::Planned(Ok(Planned::Ready {
        text: format!("Instalar {app}"),
        action: apply_action(app),
    }))
}

fn select_source(a: &mut App, kind: SourceKind) {
    let pos = a
        .hits
        .iter()
        .position(|h| a.search.entries()[h.index].source == kind)
        .unwrap();
    a.selected = pos;
}

fn irow(name: &str, origin: &str, source: &str, available: bool) -> ListRow {
    ListRow {
        name: name.into(),
        app_key: String::new(),
        not_in_catalog: false,
        origin: origin.into(),
        installed_version: Some("1".into()),
        correlation: Correlation::Matched {
            source: source.into(),
            package_id: name.into(),
            rule: Rule::ScoopManifest,
        },
        latest: Some("2".into()),
        state: if available {
            UpdateState::Available { latest: "2".into() }
        } else {
            UpdateState::UpToDate
        },
        tiebreak: None,
    }
}

fn on_tab(tab: Tab, rows: Vec<ListRow>) -> App {
    let mut a = app();
    a.installed = rows;
    a.tab = tab;
    a
}

fn confirm_open(a: &App) -> bool {
    matches!(a.modal, Some(Modal::Confirm { .. }))
}

#[test]
fn i_on_scoop_plans_and_planned_opens_confirm() {
    let mut a = app();
    select_source(&mut a, SourceKind::Scoop);
    let cmds = update(&mut a, key(KeyCode::Char('i')));
    assert_eq!(
        cmds,
        vec![Cmd::PlanInstall {
            source_name: "scoop:main".into(),
            id: "ripgrep".into(),
            keep: false
        }]
    );
    assert!(a.modal.is_none());
    assert!(update(&mut a, ready("ripgrep")).is_empty());
    assert!(confirm_open(&a));
}

#[test]
fn i_on_winget_does_not_open_anything_and_says_why() {
    let mut a = app();
    select_source(&mut a, SourceKind::Winget);
    let cmds = update(&mut a, key(KeyCode::Char('i')));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert!(a.modal.is_none());
    assert!(a.toasts[0].text.contains("fase 2d"));
}

#[test]
fn i_is_text_when_query_is_not_empty() {
    let mut a = app();
    update(&mut a, key(KeyCode::Char('r')));
    let cmds = update(&mut a, key(KeyCode::Char('i')));
    assert_eq!(a.query, "ri");
    assert!(!cmds.iter().any(|c| matches!(c, Cmd::PlanInstall { .. })));
}

#[test]
fn esc_on_confirm_closes_without_executing() {
    let mut a = app();
    update(&mut a, ready("ripgrep"));
    let cmds = update(&mut a, key(KeyCode::Esc));
    assert!(cmds.is_empty());
    assert!(a.modal.is_none());
}

#[test]
fn enter_on_confirm_runs_the_action() {
    let mut a = app();
    update(&mut a, ready("ripgrep"));
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(cmds, vec![Cmd::Execute(apply_action("ripgrep"))]);
    assert!(a.modal.is_none());
    assert!(a.running.is_some());
}

#[test]
fn confirm_ignores_other_keys_and_ctrl_r() {
    let mut a = app();
    update(&mut a, ready("ripgrep"));
    assert!(update(&mut a, key(KeyCode::Char('q'))).is_empty());
    assert!(update(&mut a, ctrl('r')).is_empty());
    assert!(confirm_open(&a));
}

#[test]
fn busy_shows_status_and_rescans() {
    let mut a = app();
    update(&mut a, ready("ripgrep"));
    update(&mut a, key(KeyCode::Enter));
    let cmds = update(&mut a, Msg::InstallFinished(Err(InstallFail::Busy)));
    assert_eq!(cmds, vec![Cmd::ScanInstalled, Cmd::RetryQueue]);
    assert!(a.modal.is_none());
    assert_eq!(a.toasts[0].text, "outra instalação em andamento");
}

#[test]
fn finished_ok_rescans_and_reports() {
    let mut a = app();
    update(&mut a, ready("ripgrep"));
    update(&mut a, key(KeyCode::Enter));
    let done = Done {
        summary: "Instalado: ripgrep".into(),
        residues: None,
    };
    let cmds = update(&mut a, Msg::InstallFinished(Ok(done)));
    assert_eq!(cmds, vec![Cmd::ScanInstalled]);
    assert!(a.modal.is_none());
    assert_eq!(a.toasts[0].text, "Instalado: ripgrep");
}

fn residue(path: &str, preselected: bool) -> Residue {
    Residue {
        kind: ResidueKind::Folder { path: path.into() },
        confidence: Confidence::High,
        preselected,
        note: None,
    }
}

#[test]
fn residues_modal_cleans_only_the_checked_items() {
    let mut a = app();
    let done = Done {
        summary: "x removido".into(),
        residues: Some((
            "x".into(),
            vec![residue("C:/a", true), residue("C:/b", false)],
        )),
    };
    update(&mut a, Msg::InstallFinished(Ok(done)));
    assert!(matches!(a.modal, Some(Modal::Residues { .. })));
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(
        cmds,
        vec![Cmd::CleanResidues {
            app: "x".into(),
            residues: vec![residue("C:/a", true)]
        }]
    );
    assert!(a.modal.is_none());
    assert!(a.running.is_some());
    update(&mut a, Msg::ResiduesCleaned(vec![]));
    assert!(a.running.is_none());
}

#[test]
fn esc_on_residues_keeps_everything() {
    let mut a = app();
    let done = Done {
        summary: "x removido".into(),
        residues: Some(("x".into(), vec![residue("C:/a", true)])),
    };
    update(&mut a, Msg::InstallFinished(Ok(done)));
    assert!(update(&mut a, key(KeyCode::Esc)).is_empty());
    assert!(a.modal.is_none());
}

fn candidate(bucket: &str) -> Candidate {
    let mut entry = entry(SourceKind::Scoop, "tool", "tool");
    entry.source_name = format!("scoop:{bucket}");
    Candidate {
        entry,
        selectable: true,
        note: None,
    }
}

#[test]
fn many_candidates_open_the_picker_and_pick_replans() {
    let mut a = app();
    update(
        &mut a,
        Msg::Planned(Ok(Planned::Choose(vec![
            candidate("extras"),
            candidate("main"),
        ]))),
    );
    assert!(matches!(a.modal, Some(Modal::Picking { .. })));
    update(&mut a, key(KeyCode::Down));
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(
        cmds,
        vec![Cmd::PlanInstall {
            source_name: "scoop:main".into(),
            id: "tool".into(),
            keep: false
        }]
    );
    assert!(a.modal.is_none());
}

#[test]
fn planning_error_becomes_status() {
    let mut a = app();
    update(&mut a, Msg::Planned(Err("não suportado: x".into())));
    assert!(a.modal.is_none());
    assert_eq!(a.toasts[0].text, "não suportado: x");
}

#[test]
fn u_removes_only_wingetrs_apps_on_installed_tab() {
    let mut a = on_tab(
        Tab::Installed,
        vec![
            irow("alpha", "scoop", "scoop:main", false),
            irow("beta", "wingetrs", "scoop:main", false),
        ],
    );
    assert!(update(&mut a, key(KeyCode::Char('u'))).is_empty());
    assert!(!a.toasts.is_empty());
    a.selected = 1;
    assert_eq!(
        update(&mut a, key(KeyCode::Char('u'))),
        vec![Cmd::PlanRemove {
            app: "beta".into(),
            purge: false
        }]
    );
    let removal = UninstallPlan {
        app: "beta".into(),
        version: "1".into(),
        purge: false,
        artifacts: vec![],
        path_users: vec![],
        remove_path_entry: true,
    };
    update(
        &mut a,
        Msg::Planned(Ok(Planned::Ready {
            text: "Remover beta".into(),
            action: Action::Remove(removal.clone()),
        })),
    );
    assert_eq!(
        update(&mut a, key(KeyCode::Enter)),
        vec![Cmd::Execute(Action::Remove(removal))]
    );
}

#[test]
fn u_is_text_outside_the_installed_tab() {
    let mut a = app();
    update(&mut a, key(KeyCode::Char('u')));
    assert_eq!(a.query, "u");
}

#[test]
fn upper_u_upgrades_selected_or_all() {
    let mut a = on_tab(
        Tab::Updates,
        vec![irow("beta", "wingetrs", "scoop:main", true)],
    );
    assert_eq!(
        update(&mut a, key(KeyCode::Char('U'))),
        vec![Cmd::PlanInstall {
            source_name: "scoop:main".into(),
            id: "beta".into(),
            keep: false
        }]
    );
    let mut empty = on_tab(Tab::Updates, vec![]);
    assert_eq!(
        update(&mut empty, key(KeyCode::Char('U'))),
        vec![Cmd::PlanUpgradeAll { keep: false }]
    );
    let mut foreign = on_tab(Tab::Updates, vec![irow("gamma", "arp", "winget", true)]);
    assert!(update(&mut foreign, key(KeyCode::Char('U'))).is_empty());
    assert!(!foreign.toasts.is_empty());
}

#[test]
fn repeated_plan_keys_are_ignored_while_planning() {
    let mut a = app();
    select_source(&mut a, SourceKind::Scoop);
    assert_eq!(update(&mut a, key(KeyCode::Char('i'))).len(), 1);
    assert!(a.planning);
    assert_eq!(a.status.as_deref(), Some("planejando…"));
    assert!(update(&mut a, key(KeyCode::Char('i'))).is_empty());
    update(&mut a, ready("ripgrep"));
    assert!(!a.planning);
    assert!(confirm_open(&a));
    update(&mut a, key(KeyCode::Esc));
    assert_eq!(update(&mut a, key(KeyCode::Char('i'))).len(), 1);
}

#[test]
fn a_failed_plan_frees_the_keys_again() {
    let mut a = on_tab(Tab::Updates, vec![]);
    assert_eq!(
        update(&mut a, key(KeyCode::Char('U'))),
        vec![Cmd::PlanUpgradeAll { keep: false }]
    );
    assert!(update(&mut a, key(KeyCode::Char('U'))).is_empty());
    update(
        &mut a,
        Msg::Planned(Err("nenhum app para atualizar".into())),
    );
    assert!(!a.planning);
    assert_eq!(update(&mut a, key(KeyCode::Char('U'))).len(), 1);
}

#[test]
fn update_reraises_a_worker_panic() {
    let mut a = app();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        update(&mut a, Msg::Panicked(Box::new("boom")));
    }));
    assert!(r.is_err());
}

#[test]
fn boot_with_an_unreadable_catalog_keeps_entries_shows_the_error_and_refreshes() {
    let mut a = app();
    let before = a.search.entries().len();
    assert!(before > 0);
    let cmds = update(
        &mut a,
        Msg::Booted {
            catalog: Err("file is not a database".into()),
            age: None,
            needs_refresh: false,
        },
    );
    let status = a.toasts.first().map(|t| t.text.clone()).unwrap_or_default();
    assert!(status.contains("falha ao ler o catálogo"), "{status}");
    assert!(status.contains("file is not a database"), "{status}");
    assert_eq!(a.search.entries().len(), before);
    assert_eq!(
        cmds,
        vec![Cmd::ScanInstalled, Cmd::Refresh { force: false }]
    );
}

#[test]
fn boot_with_a_populated_fresh_catalog_loads_it_and_does_not_refresh() {
    let mut a = App::new(Vec::new(), GlyphSet::Nerd);
    a.status = Some("carregando catálogo…".into());
    let cmds = update(
        &mut a,
        Msg::Booted {
            catalog: Ok(vec![
                entry(SourceKind::Scoop, "ripgrep", "ripgrep"),
                entry(SourceKind::Winget, "Zeal.Zeal", "Zeal"),
            ]),
            age: Some(42),
            needs_refresh: false,
        },
    );
    assert_eq!(a.search.entries().len(), 2);
    assert_eq!(a.status, None);
    assert_eq!(a.index_age_secs, Some(42));
    assert_eq!(cmds, vec![Cmd::ScanInstalled, details_for(&a)]);
}

#[test]
fn boot_refreshes_when_the_catalog_is_empty_or_stale() {
    let mut a = App::new(Vec::new(), GlyphSet::Nerd);
    let cmds = update(
        &mut a,
        Msg::Booted {
            catalog: Ok(vec![]),
            age: None,
            needs_refresh: false,
        },
    );
    assert_eq!(
        cmds,
        vec![Cmd::ScanInstalled, Cmd::Refresh { force: false }]
    );
    let mut b = App::new(Vec::new(), GlyphSet::Nerd);
    let cmds = update(
        &mut b,
        Msg::Booted {
            catalog: Ok(vec![entry(SourceKind::Scoop, "ripgrep", "ripgrep")]),
            age: Some(5000),
            needs_refresh: true,
        },
    );
    assert_eq!(
        cmds,
        vec![
            Cmd::ScanInstalled,
            details_for(&b),
            Cmd::Refresh { force: false }
        ]
    );
}

#[test]
fn a_failed_reload_keeps_the_entries_and_shows_the_error() {
    let mut a = app();
    let before = a.search.entries().len();
    assert!(before > 0);
    update(&mut a, Msg::CatalogFailed("database is locked".into()));
    assert_eq!(a.search.entries().len(), before);
    let status = a.toasts.first().map(|t| t.text.clone()).unwrap_or_default();
    assert!(status.contains("falha ao ler o catálogo"), "{status}");
    assert!(status.contains("database is locked"), "{status}");
}

fn details_for(a: &App) -> Cmd {
    let (source, id) = a.selected_key().unwrap();
    Cmd::LoadDetails { source, id }
}

fn loaded(key: &str) -> Msg {
    Msg::DetailsLoaded {
        key: key.into(),
        result: Err("x".into()),
    }
}

fn three_updates() -> App {
    on_tab(
        Tab::Updates,
        vec![
            irow("a", "wingetrs", "scoop:main", true),
            irow("b", "wingetrs", "scoop:main", true),
            irow("c", "wingetrs", "scoop:main", true),
        ],
    )
}

#[test]
fn selection_moves_while_details_are_pending() {
    let mut a = three_updates();
    assert_eq!(
        update(&mut a, key(KeyCode::Down)),
        vec![Cmd::LoadDetails {
            source: "scoop:main".into(),
            id: "b".into()
        }]
    );
    let cmds = update(&mut a, key(KeyCode::Down));
    assert_eq!(a.selected, 2);
    assert!(
        matches!(cmds.as_slice(), [Cmd::LoadDetails { id, .. }] if id == "c"),
        "{cmds:?}"
    );
    assert_eq!(a.details_pending, Some("scoop:main/c".into()));
}

#[test]
fn every_view_change_requests_details_for_the_new_selection() {
    // (a) catalog arrives, by reload and by boot.
    let mut a = App::new(vec![], GlyphSet::Nerd);
    let cmds = update(&mut a, Msg::CatalogLoaded(app_entries()));
    assert!(cmds.contains(&details_for(&a)), "{cmds:?}");
    let mut a = App::new(vec![], GlyphSet::Nerd);
    let cmds = update(
        &mut a,
        Msg::Booted {
            catalog: Ok(app_entries()),
            age: None,
            needs_refresh: false,
        },
    );
    assert!(cmds.contains(&Cmd::ScanInstalled), "{cmds:?}");
    assert!(cmds.contains(&details_for(&a)), "{cmds:?}");
    // (b) installed rows arrive.
    let mut a = on_tab(Tab::Updates, vec![]);
    let cmds = update(
        &mut a,
        Msg::InstalledLoaded(vec![irow("a", "wingetrs", "scoop:main", true)]),
    );
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::LoadDetails { id, .. } if id == "a")),
        "{cmds:?}"
    );
    // (c) tab switch.
    let mut a = on_tab(
        Tab::Search,
        vec![irow("a", "wingetrs", "scoop:main", false)],
    );
    let cmds = update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Installed);
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::LoadDetails { id, .. } if id == "a")),
        "{cmds:?}"
    );
    // (d) Esc clears the query.
    let mut a = app();
    step(&mut a, key(KeyCode::Char('z')));
    let cmds = step(&mut a, key(KeyCode::Esc));
    assert!(a.query.is_empty());
    assert!(cmds.contains(&details_for(&a)), "{cmds:?}");
}

#[test]
fn spinner_stops_when_the_selected_details_arrive() {
    let mut a = App::new(vec![], GlyphSet::Nerd);
    update(&mut a, Msg::CatalogLoaded(app_entries()));
    let pending = a.details_pending.clone().unwrap();
    update(&mut a, loaded(&pending));
    assert!(!a.animating());
    let frame = a.spinner_frame;
    update(&mut a, Msg::Tick);
    assert_eq!(a.spinner_frame, frame);
}

#[test]
fn stale_details_keep_the_spinner_for_the_current_item() {
    let mut a = three_updates();
    update(&mut a, key(KeyCode::Down));
    update(&mut a, key(KeyCode::Down));
    update(&mut a, loaded("scoop:main/b"));
    assert!(a.animating());
    assert!(a.details.contains_key("scoop:main/b"));
    update(&mut a, loaded("scoop:main/c"));
    assert!(!a.animating());
}

#[test]
fn right_and_left_arrows_cycle_tabs() {
    let mut a = app();
    for want in [Tab::Installed, Tab::Updates, Tab::Settings, Tab::Search] {
        update(&mut a, key(KeyCode::Right));
        assert_eq!(a.tab, want);
    }
    update(&mut a, key(KeyCode::Left));
    assert_eq!(a.tab, Tab::Settings);
    update(&mut a, key(KeyCode::Left));
    assert_eq!(a.tab, Tab::Updates);
}

#[test]
fn arrows_match_tab_and_backtab() {
    for start in [Tab::Search, Tab::Installed, Tab::Updates, Tab::Settings] {
        for (arrow, other) in [
            (KeyCode::Right, KeyCode::Tab),
            (KeyCode::Left, KeyCode::BackTab),
        ] {
            let mut x = on_tab(start, Vec::new());
            let mut y = on_tab(start, Vec::new());
            update(&mut x, key(arrow));
            update(&mut y, key(other));
            assert_eq!(x.tab, y.tab);
        }
    }
}

#[test]
fn arrows_switch_tabs_with_text() {
    let mut a = app();
    update(&mut a, key(KeyCode::Char('p')));
    update(&mut a, key(KeyCode::Char('o')));
    update(&mut a, key(KeyCode::Right));
    assert_eq!(a.tab, Tab::Installed);
    assert_eq!(a.query, "po");
    let mut b = app();
    update(&mut b, key(KeyCode::Right));
    assert_eq!(b.tab, Tab::Installed);
}

#[test]
fn arrows_belong_to_an_open_dialog() {
    let mut a = app();
    select_source(&mut a, SourceKind::Scoop);
    update(&mut a, key(KeyCode::Char('i')));
    update(&mut a, ready("ripgrep"));
    assert!(confirm_open(&a));
    let tab = a.tab;
    update(&mut a, key(KeyCode::Right));
    update(&mut a, key(KeyCode::Left));
    assert_eq!(a.tab, tab);
    assert!(confirm_open(&a));
}

#[test]
fn arrows_are_ignored_while_help_is_open() {
    let mut a = app();
    update(&mut a, key(KeyCode::Char('?')));
    assert!(a.show_help);
    let tab = a.tab;
    update(&mut a, key(KeyCode::Right));
    update(&mut a, key(KeyCode::Left));
    assert_eq!(a.tab, tab);
    assert!(a.show_help);
}

#[test]
fn cached_installed_view_matches_a_fresh_filter() {
    let rows = vec![
        irow("Alpha", "arp", "winget", true),
        irow("beta", "scoop", "scoop:main", false),
        irow("Gamma", "arp", "winget", false),
        irow("alfa", "scoop", "scoop:extras", true),
        irow("Zed", "arp", "winget", true),
        irow("zebra", "scoop", "scoop:main", false),
    ];
    for tab in [Tab::Installed, Tab::Updates] {
        for toggles in 0..3 {
            for q in ["", "a", "ze"] {
                let mut a = on_tab(tab, rows.clone());
                for _ in 0..toggles {
                    update(&mut a, ctrl('s'));
                }
                for c in q.chars() {
                    update(&mut a, key(KeyCode::Char(c)));
                }
                // Reference: the plain filter over `installed`.
                let mut expected: Vec<&str> = rows
                    .iter()
                    .filter(|r| {
                        tab != Tab::Updates || matches!(r.state, UpdateState::Available { .. })
                    })
                    .filter(|r| r.name.to_lowercase().contains(q))
                    .filter(|r| match (toggles, &r.correlation) {
                        (0, _) => true,
                        (1, Correlation::Matched { source, .. }) => source == "winget",
                        (2, Correlation::Matched { source, .. }) => source.starts_with("scoop"),
                        _ => false,
                    })
                    .map(|r| r.name.as_str())
                    .collect();
                // The view is ordered by name (the default sort).
                expected.sort_by_key(|n| n.to_lowercase());
                let got: Vec<&str> = a.installed_view().iter().map(|r| r.name.as_str()).collect();
                assert_eq!(got, expected, "tab {tab:?} toggles {toggles} query {q:?}");
            }
        }
    }
}

use wingetrs::tui::app::run_search;

/// Runs every `Cmd::Search` in `cmds` like the event loop does and delivers the results.
/// Searches it starts in turn (a coalesced query) are run too.
fn deliver_searches(a: &mut App, cmds: Vec<Cmd>) -> Vec<Cmd> {
    let mut rest = Vec::new();
    let mut queue = std::collections::VecDeque::from(cmds);
    while let Some(c) = queue.pop_front() {
        match c {
            Cmd::Search { .. } => queue.extend(answer(a, &c)),
            other => rest.push(other),
        }
    }
    rest
}

/// Runs one `Cmd::Search` on the current index and delivers its result.
fn answer(a: &mut App, cmd: &Cmd) -> Vec<Cmd> {
    let Cmd::Search {
        generation,
        query,
        kind,
    } = cmd
    else {
        panic!("not a search: {cmd:?}");
    };
    let hits = run_search(&a.search, query, *kind);
    update(
        a,
        Msg::SearchDone {
            generation: *generation,
            hits,
        },
    )
}

/// `update` followed by the searches it asked for.
fn step(a: &mut App, msg: Msg) -> Vec<Cmd> {
    let cmds = update(a, msg);
    deliver_searches(a, cmds)
}

/// More entries than `MAX_HITS`, so a changed result limit shows up.
fn big_entries() -> Vec<PackageEntry> {
    let mut es = app_entries();
    es.extend((0..1500).map(|i| entry(SourceKind::Winget, &format!("pkg{i:04}"), "Pkg")));
    es
}

#[test]
fn async_search_equals_sync_search() {
    for entries in [app_entries(), big_entries()] {
        for (toggles, kind) in [(0, None), (1, Some(SourceKind::Winget))] {
            for q in ["", "p", "po", "zeal", "xyz"] {
                let mut a = App::new(entries.clone(), GlyphSet::Nerd);
                for _ in 0..toggles {
                    step(&mut a, ctrl('s'));
                }
                for c in q.chars() {
                    step(&mut a, key(KeyCode::Char(c)));
                }
                let expected = a.search.search(q, kind, 1000);
                assert_eq!(a.hits, expected, "query {q:?} filter {kind:?}");
            }
        }
    }
}

#[test]
fn stale_search_results_are_dropped() {
    let mut a = app();
    // "p" starts generation 1; "o" waits for it and then starts generation 2 ("po").
    let first = update(&mut a, key(KeyCode::Char('p')));
    assert!(update(&mut a, key(KeyCode::Char('o'))).is_empty());
    let second = answer(&mut a, &first[0]);
    assert!(matches!(&second[..], [Cmd::Search { query, .. }] if query == "po"));
    let expected = a.search.search("po", None, 1000);
    deliver_searches(&mut a, second);
    // The generation-1 result arriving again (late) is dropped.
    assert!(answer(&mut a, &first[0]).is_empty());
    assert_eq!(a.hits, expected);
}

#[test]
fn stale_search_after_catalog_reload_is_dropped() {
    let mut a = app();
    let cmds = update(&mut a, key(KeyCode::Char('p')));
    let Cmd::Search { generation, .. } = &cmds[0] else {
        panic!("{cmds:?}");
    };
    // Computed by the worker on the old index.
    let old_hits = run_search(&a.search, "p", None);
    update(
        &mut a,
        Msg::CatalogLoaded(vec![
            entry(SourceKind::Winget, "Zed.Zed", "Zed"),
            entry(SourceKind::Scoop, "git", "git"),
            entry(SourceKind::Scoop, "pnpm", "pnpm"),
        ]),
    );
    let fresh = run_search(&a.search, "p", None);
    assert_ne!(old_hits, fresh);
    let cmds = update(
        &mut a,
        Msg::SearchDone {
            generation: *generation,
            hits: old_hits,
        },
    );
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(a.hits, fresh);
}

#[test]
fn search_result_off_the_search_tab_is_dropped() {
    let mut a = app();
    update(
        &mut a,
        Msg::InstalledLoaded(vec![
            irow("pa", "wingetrs", "scoop:main", false),
            irow("pb", "wingetrs", "scoop:main", false),
        ]),
    );
    let cmds = update(&mut a, key(KeyCode::Char('p')));
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Installed);
    update(&mut a, key(KeyCode::Down));
    assert_eq!(a.selected, 1);
    assert!(answer(&mut a, &cmds[0]).is_empty());
    assert_eq!(a.selected, 1);
    // The slot is free again: back on Search, the query typed meanwhile is searched.
    update(&mut a, key(KeyCode::Char('x')));
    let back = update(&mut a, key(KeyCode::BackTab));
    assert_eq!(a.tab, Tab::Search);
    let search: Vec<&Cmd> = back
        .iter()
        .filter(|c| matches!(c, Cmd::Search { .. }))
        .collect();
    assert!(
        matches!(search[..], [Cmd::Search { query, .. }] if query == "px"),
        "{back:?}"
    );
    answer(&mut a, search[0]);
    assert_eq!(a.hits, a.search.search("px", None, 1000));
}

#[test]
fn filter_change_during_a_search_is_searched_next() {
    let mut a = app();
    let p = update(&mut a, key(KeyCode::Char('p')));
    assert!(update(&mut a, ctrl('s')).is_empty());
    let next = answer(&mut a, &p[0]);
    assert!(
        matches!(&next[..], [Cmd::Search { query, kind: Some(SourceKind::Winget), .. }] if query == "p"),
        "{next:?}"
    );
    answer(&mut a, &next[0]);
    assert_eq!(a.hits, a.search.search("p", Some(SourceKind::Winget), 1000));
}

#[test]
fn cancelled_search_frees_the_slot() {
    let mut a = app();
    let p = update(&mut a, key(KeyCode::Char('p')));
    let Cmd::Search { generation, .. } = p[0] else {
        panic!("{p:?}");
    };
    assert!(update(&mut a, Msg::SearchCancelled { generation }).is_empty());
    let o = update(&mut a, key(KeyCode::Char('o')));
    assert!(
        matches!(&o[..], [Cmd::Search { query, .. }] if query == "po"),
        "{o:?}"
    );
}

#[test]
fn typing_during_a_search_queues_only_the_latest() {
    let mut a = app();
    let before = a.hits.clone();
    let c = update(&mut a, key(KeyCode::Char('c')));
    assert_eq!(c.len(), 1, "{c:?}");
    assert!(matches!(&c[0], Cmd::Search { query, .. } if query == "c"));
    assert!(update(&mut a, key(KeyCode::Char('o'))).is_empty());
    assert!(update(&mut a, key(KeyCode::Char('d'))).is_empty());
    let next = answer(&mut a, &c[0]);
    assert!(
        matches!(&next[..], [Cmd::Search { query, .. }] if query == "cod"),
        "{next:?}"
    );
    assert_eq!(a.hits, before, "the 'c' result is not shown");
    answer(&mut a, &next[0]);
    assert_eq!(a.hits, a.search.search("cod", None, 1000));
}

#[test]
fn typing_does_not_search_on_the_ui_thread() {
    let mut a = app();
    let before = a.hits.clone();
    let cmds = update(&mut a, key(KeyCode::Char('p')));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::Search { query, .. } if query == "p"))
    );
    assert_eq!(a.hits, before);
}

#[test]
fn page_down_moves_ten_items() {
    let es: Vec<PackageEntry> = (0..50)
        .map(|i| {
            entry(
                SourceKind::Winget,
                &format!("pkg{i:02}"),
                &format!("pkg{i:02}"),
            )
        })
        .collect();
    let mut a = App::new(es, GlyphSet::Ascii);
    update(&mut a, key(KeyCode::PageDown));
    assert_eq!(a.selected, 10);
}

#[test]
fn end_scrolls_by_two_row_items() {
    let es: Vec<PackageEntry> = (0..50)
        .map(|i| {
            entry(
                SourceKind::Winget,
                &format!("pkg{i:02}"),
                &format!("pkg{i:02}"),
            )
        })
        .collect();
    let mut a = App::new(es, GlyphSet::Ascii);
    update(&mut a, Msg::Resize(100, 20));
    update(&mut a, key(KeyCode::End));
    // (20 - chrome_rows(20) = 8) / ITEM_ROWS 2 = 6 items per page: the window is 44..50.
    assert_eq!(a.selected, 49);
    assert_eq!(a.scroll, 44);
}

// ---- toasts (Task 66) ----

use std::time::Duration;
use wingetrs::tui::app::ToastKind;

fn toast_texts(a: &App) -> Vec<&str> {
    a.toasts.iter().map(|t| t.text.as_str()).collect()
}

#[test]
fn success_toast_expires_after_three_seconds() {
    let mut a = app();
    let t0 = a.now;
    update(
        &mut a,
        Msg::InstallFinished(Ok(Done {
            summary: "Instalado: rg".into(),
            residues: None,
        })),
    );
    assert_eq!(a.toasts[0].kind, ToastKind::Success);
    assert_eq!(a.toasts[0].text, "Instalado: rg");
    a.now = t0 + Duration::from_millis(2999);
    update(&mut a, Msg::Clock);
    assert_eq!(a.toasts.len(), 1);
    a.now = t0 + Duration::from_millis(3001);
    update(&mut a, Msg::Clock);
    assert!(a.toasts.is_empty());
}

#[test]
fn catalog_error_toast_stays_until_esc() {
    let mut a = app();
    let t0 = a.now;
    update(&mut a, Msg::CatalogFailed("database is locked".into()));
    assert_eq!(a.toasts[0].kind, ToastKind::Error);
    assert_eq!(a.toasts[0].expires, None);
    a.now = t0 + Duration::from_secs(600);
    update(&mut a, Msg::Clock);
    assert_eq!(a.toasts.len(), 1);
    let cmds = update(&mut a, key(KeyCode::Esc));
    assert!(a.toasts.is_empty());
    assert!(cmds.is_empty(), "{cmds:?}");
}

#[test]
fn esc_ignores_timed_toasts() {
    let mut a = app();
    a.push_toast(ToastKind::Success, "ok");
    update(&mut a, key(KeyCode::Char('a')));
    update(&mut a, key(KeyCode::Char('b')));
    update(&mut a, key(KeyCode::Esc));
    assert_eq!(a.query, "");
    assert_eq!(a.toasts.len(), 1);
}

#[test]
fn stack_keeps_three_and_drops_the_oldest_timed() {
    let mut a = app();
    a.push_toast(ToastKind::Error, "E");
    a.push_toast(ToastKind::Success, "S1");
    a.push_toast(ToastKind::Success, "S2");
    a.push_toast(ToastKind::Success, "S3");
    assert_eq!(toast_texts(&a), vec!["S3", "S2", "E"]);
}

#[test]
fn repeated_toast_counts_and_restarts_its_timer() {
    let mut a = app();
    let t0 = a.now;
    a.push_toast(ToastKind::Success, "x");
    a.now = t0 + Duration::from_secs(2);
    a.push_toast(ToastKind::Success, "x");
    assert_eq!(a.toasts.len(), 1);
    assert_eq!(a.toasts[0].count, 2);
    assert_eq!(a.toasts[0].expires, Some(t0 + Duration::from_secs(5)));
}

#[test]
fn next_deadline_is_the_earliest_timed_toast() {
    let mut a = app();
    let t0 = a.now;
    a.push_toast(ToastKind::Warning, "w");
    a.push_toast(ToastKind::Success, "s");
    assert_eq!(a.next_toast_deadline(), Some(t0 + Duration::from_secs(3)));
    let mut b = app();
    b.push_toast(ToastKind::Error, "e");
    assert_eq!(b.next_toast_deadline(), None);
}

#[test]
fn forced_refresh_success_toasts_and_automatic_is_silent() {
    let mut a = app();
    update(&mut a, ctrl('r'));
    update(&mut a, Msg::RefreshStarted);
    update(&mut a, Msg::RefreshFinished(vec![]));
    assert_eq!(toast_texts(&a), vec!["catálogo atualizado"]);
    let mut b = app();
    update(&mut b, Msg::RefreshStarted);
    update(&mut b, Msg::RefreshFinished(vec![]));
    assert!(b.toasts.is_empty());
}

#[test]
fn offline_refresh_failure_is_a_warning() {
    let mut a = app();
    update(&mut a, Msg::RefreshFinished(failed_report(true)));
    assert_eq!(a.toasts[0].kind, ToastKind::Warning);
    let mut b = app();
    update(&mut b, Msg::RefreshFinished(failed_report(false)));
    assert_eq!(b.toasts[0].kind, ToastKind::Error);
    let mut mixed = failed_report(true);
    mixed.extend(failed_report(false));
    let mut c = app();
    update(&mut c, Msg::RefreshFinished(mixed));
    assert_eq!(c.toasts[0].kind, ToastKind::Error);
}

#[test]
fn planning_stays_in_the_status_bar() {
    let mut a = app();
    select_source(&mut a, SourceKind::Scoop);
    update(&mut a, key(KeyCode::Char('i')));
    assert_eq!(a.status.as_deref(), Some("planejando…"));
    assert!(a.toasts.is_empty());
}

#[test]
fn expired_toast_disappears_on_any_message() {
    let mut a = app();
    let t0 = a.now;
    a.push_toast(ToastKind::Success, "ok");
    a.now = t0 + Duration::from_secs(4);
    update(&mut a, Msg::Resize(100, 24));
    assert!(a.toasts.is_empty());
}

#[test]
fn esc_in_a_dialog_closes_the_dialog_and_keeps_the_error_toast() {
    let mut a = app();
    a.push_toast(ToastKind::Error, "falhou");
    update(&mut a, ready("ripgrep"));
    assert!(confirm_open(&a));
    update(&mut a, key(KeyCode::Esc));
    assert!(a.modal.is_none());
    assert_eq!(toast_texts(&a), vec!["falhou"]);
}

#[test]
fn error_toast_adds_the_esc_hint() {
    use wingetrs::tui::help::hints;
    let mut a = app();
    assert!(!hints(&a).contains(&("esc", "fecha erro")));
    a.push_toast(ToastKind::Error, "falhou");
    let h = hints(&a);
    let esc = h.iter().position(|x| *x == ("esc", "fecha erro")).unwrap();
    assert_eq!(esc + 1, h.iter().position(|x| x.0 == "?").unwrap());
}

// ---- Settings tab (Task 68) ----
use wingetrs::config::ConfirmMode;

fn on_settings() -> App {
    let mut a = app();
    a.tab = Tab::Settings;
    a
}

#[test]
fn settings_tab_is_in_the_tab_cycle() {
    let mut a = app();
    for want in [Tab::Installed, Tab::Updates, Tab::Settings, Tab::Search] {
        update(&mut a, key(KeyCode::Tab));
        assert_eq!(a.tab, want);
    }
    update(&mut a, key(KeyCode::BackTab));
    assert_eq!(a.tab, Tab::Settings);
    let mut b = app();
    b.tab = Tab::Updates;
    update(&mut b, key(KeyCode::Right));
    assert_eq!(b.tab, Tab::Settings);
    let mut c = app();
    update(&mut c, key(KeyCode::Left));
    assert_eq!(c.tab, Tab::Settings);
}

#[test]
fn enter_changes_confirm_and_requests_save() {
    let mut a = on_settings();
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(a.settings.behavior.confirm, ConfirmMode::Removals);
    assert_eq!(
        cmds,
        vec![Cmd::SaveSettings {
            behavior: wingetrs::config::Behavior {
                confirm: ConfirmMode::Removals,
                ..Default::default()
            },
            glyphs: None,
            icons: None,
            list_icons: None,
            theme: "dark".into(),
        }]
    );
    assert_eq!(a.status.as_deref(), Some("salvando…"));
}

#[test]
fn failed_save_restores_the_last_saved_value() {
    let mut a = on_settings();
    update(&mut a, key(KeyCode::Enter));
    update(&mut a, Msg::SettingsSaved(Err("disco cheio".into())));
    assert_eq!(a.settings.behavior.confirm, ConfirmMode::Always);
    assert_eq!(a.toasts[0].kind, ToastKind::Error);
    assert!(a.toasts[0].text.contains("disco cheio"));
    assert_eq!(a.status, None);
}

#[test]
fn successful_save_is_remembered_and_announced() {
    let mut a = on_settings();
    update(&mut a, key(KeyCode::Enter));
    update(&mut a, Msg::SettingsSaved(Ok(())));
    update(&mut a, Msg::SettingsSaved(Err("x".into())));
    assert_eq!(a.settings.behavior.confirm, ConfirmMode::Removals);
    update(&mut a, Msg::SettingsSaved(Ok(())));
    assert!(
        a.toasts
            .iter()
            .any(|t| t.text == "ajustes salvos" && t.kind == ToastKind::Success)
    );
}

#[test]
fn forced_glyphs_cannot_be_changed() {
    let mut a = on_settings();
    a.settings.glyphs_forced = true;
    for _ in 0..5 {
        update(&mut a, key(KeyCode::Down));
    }
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(cmds.is_empty());
    assert_eq!(a.glyphs, GlyphSet::Nerd);
}

#[test]
fn letters_do_not_type_in_settings() {
    let mut a = on_settings();
    update(&mut a, key(KeyCode::Char('x')));
    assert!(a.query.is_empty());
}

#[test]
fn theme_setting_flips_dark() {
    let mut a = on_settings();
    for _ in 0..9 {
        update(&mut a, key(KeyCode::Down));
    }
    update(&mut a, key(KeyCode::Enter));
    assert!(!a.dark);
}

#[test]
fn h_cycles_backwards_and_arrows_leave_settings() {
    let mut a = on_settings();
    update(&mut a, key(KeyCode::Char('h')));
    assert_eq!(a.settings.behavior.confirm, ConfirmMode::Never);
    update(&mut a, key(KeyCode::Right));
    assert_eq!(a.tab, Tab::Search);
    assert_eq!(a.settings.behavior.confirm, ConfirmMode::Never);
}

#[test]
fn list_keys_do_not_break_the_settings_tab() {
    let mut a = on_settings();
    for c in [
        KeyCode::End,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::Backspace,
    ] {
        assert!(update(&mut a, key(c)).is_empty());
    }
    assert_eq!(a.selected, 0);
    assert_eq!(a.settings.selected, 0);
}

#[test]
fn forced_glyphs_are_not_written_when_another_setting_changes() {
    let mut a = on_settings();
    a.settings.glyphs_forced = true;
    update(&mut a, key(KeyCode::Down));
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(matches!(
        cmds.as_slice(),
        [Cmd::SaveSettings { glyphs: None, behavior, .. }] if behavior.purge
    ));
}

#[test]
fn changed_glyphs_are_written() {
    let mut a = on_settings();
    for _ in 0..5 {
        update(&mut a, key(KeyCode::Down));
    }
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(matches!(
        cmds.as_slice(),
        [Cmd::SaveSettings { glyphs: Some(g), .. }] if g == "unicode"
    ));
}

#[test]
fn only_one_save_in_flight_and_the_newest_follows() {
    let mut a = on_settings();
    assert_eq!(update(&mut a, key(KeyCode::Enter)).len(), 1);
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
    let cmds = update(&mut a, Msg::SettingsSaved(Ok(())));
    assert_eq!(cmds.len(), 1);
    assert!(matches!(
        &cmds[0],
        Cmd::SaveSettings { behavior, .. } if behavior.confirm == ConfirmMode::Never
    ));
    assert!(a.toasts.is_empty());
    assert!(update(&mut a, Msg::SettingsSaved(Ok(()))).is_empty());
    assert_eq!(a.settings.saved.0, a.settings.behavior);
    assert_eq!(a.toasts.len(), 1);
}

use wingetrs::config::ResidueMode;

fn removal(app: &str) -> Action {
    Action::Remove(UninstallPlan {
        app: app.into(),
        version: "1".into(),
        purge: false,
        artifacts: vec![],
        path_users: vec![],
        remove_path_entry: true,
    })
}

#[test]
fn never_confirm_executes_directly() {
    let mut a = app();
    a.settings.behavior.confirm = ConfirmMode::Never;
    let cmds = update(&mut a, ready("a"));
    assert!(a.running.is_some());
    assert_eq!(cmds, vec![Cmd::Execute(apply_action("a"))]);
}

#[test]
fn removals_mode_confirms_only_removals() {
    let mut a = app();
    a.settings.behavior.confirm = ConfirmMode::Removals;
    let cmds = update(&mut a, ready("a"));
    assert_eq!(cmds, vec![Cmd::Execute(apply_action("a"))]);
    let mut b = app();
    b.settings.behavior.confirm = ConfirmMode::Removals;
    let cmds = update(
        &mut b,
        Msg::Planned(Ok(Planned::Ready {
            text: "Remover a".into(),
            action: removal("a"),
        })),
    );
    assert!(cmds.is_empty());
    assert!(confirm_open(&b));
}

#[test]
fn ambiguous_target_still_opens_the_picker() {
    let mut a = app();
    a.settings.behavior.confirm = ConfirmMode::Never;
    let cmds = update(
        &mut a,
        Msg::Planned(Ok(Planned::Choose(vec![
            candidate("main"),
            candidate("extras"),
        ]))),
    );
    assert!(cmds.is_empty());
    assert!(matches!(a.modal, Some(Modal::Picking { .. })));
}

#[test]
fn remove_carries_the_purge_setting() {
    let mut a = on_tab(
        Tab::Installed,
        vec![irow("a", "wingetrs", "scoop:main", false)],
    );
    a.settings.behavior.purge = true;
    assert_eq!(
        update(&mut a, key(KeyCode::Char('u'))),
        vec![Cmd::PlanRemove {
            app: "a".into(),
            purge: true
        }]
    );
}

#[test]
fn upgrade_carries_the_keep_setting() {
    let mut a = on_tab(
        Tab::Updates,
        vec![irow("a", "wingetrs", "scoop:main", true)],
    );
    a.settings.behavior.keep_old = true;
    let cmds = update(&mut a, key(KeyCode::Char('U')));
    assert!(
        matches!(cmds.as_slice(), [Cmd::PlanInstall { keep: true, .. }]),
        "{cmds:?}"
    );
    let mut empty = on_tab(Tab::Updates, vec![]);
    empty.settings.behavior.keep_old = true;
    assert_eq!(
        update(&mut empty, key(KeyCode::Char('U'))),
        vec![Cmd::PlanUpgradeAll { keep: true }]
    );
}

fn removed_with_residues() -> Msg {
    Msg::InstallFinished(Ok(Done {
        summary: "a removido".into(),
        residues: Some(("a".into(), vec![residue("p1", true), residue("p2", false)])),
    }))
}

#[test]
fn high_residue_mode_cleans_only_preselected() {
    let mut a = app();
    a.settings.behavior.residues = ResidueMode::High;
    let cmds = update(&mut a, removed_with_residues());
    assert!(cmds.contains(&Cmd::ScanInstalled), "{cmds:?}");
    assert!(
        cmds.contains(&Cmd::CleanResidues {
            app: "a".into(),
            residues: vec![residue("p1", true)]
        }),
        "{cmds:?}"
    );
    assert_eq!(
        a.running.as_ref().map(|r| r.label.as_str()),
        Some("Limpando resíduos de a")
    );
}

#[test]
fn keep_residue_mode_opens_nothing() {
    let mut a = app();
    a.settings.behavior.residues = ResidueMode::Keep;
    let cmds = update(&mut a, removed_with_residues());
    assert!(cmds.contains(&Cmd::ScanInstalled), "{cmds:?}");
    assert!(a.modal.is_none());
    assert_eq!(a.toasts[0].kind, ToastKind::Info);
    assert!(a.toasts[0].text.contains("2 itens"), "{}", a.toasts[0].text);
}

fn two_candidates() -> Msg {
    Msg::Planned(Ok(Planned::Choose(vec![
        candidate("main"),
        candidate("extras"),
    ])))
}

#[test]
fn picker_after_upgrade_keeps_the_keep_setting() {
    let mut a = on_tab(
        Tab::Updates,
        vec![irow("a", "wingetrs", "scoop:main", true)],
    );
    a.settings.behavior.keep_old = true;
    assert_eq!(update(&mut a, key(KeyCode::Char('U'))).len(), 1);
    update(&mut a, two_candidates());
    assert!(matches!(a.modal, Some(Modal::Picking { .. })));
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(
        matches!(cmds.as_slice(), [Cmd::PlanInstall { keep: true, .. }]),
        "{cmds:?}"
    );
}

#[test]
fn picker_after_install_never_keeps() {
    let mut a = app();
    a.settings.behavior.keep_old = true;
    select_source(&mut a, SourceKind::Scoop);
    assert_eq!(update(&mut a, key(KeyCode::Char('i'))).len(), 1);
    update(&mut a, two_candidates());
    assert!(matches!(a.modal, Some(Modal::Picking { .. })));
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(
        matches!(cmds.as_slice(), [Cmd::PlanInstall { keep: false, .. }]),
        "{cmds:?}"
    );
}

#[test]
fn confirm_setting_is_taken_at_key_time() {
    let mut a = on_tab(
        Tab::Installed,
        vec![irow("a", "wingetrs", "scoop:main", false)],
    );
    a.settings.behavior.confirm = ConfirmMode::Always;
    assert_eq!(update(&mut a, key(KeyCode::Char('u'))).len(), 1);
    a.settings.behavior.confirm = ConfirmMode::Never;
    let cmds = update(
        &mut a,
        Msg::Planned(Ok(Planned::Ready {
            text: "Remover a".into(),
            action: removal("a"),
        })),
    );
    assert!(cmds.is_empty(), "{cmds:?}");
    assert!(confirm_open(&a));
}

#[test]
fn second_upgrade_while_running_plans_again() {
    let mut a = on_tab(
        Tab::Updates,
        vec![irow("a", "wingetrs", "scoop:main", true)],
    );
    a.settings.behavior.confirm = ConfirmMode::Never;
    assert_eq!(update(&mut a, key(KeyCode::Char('U'))).len(), 1);
    update(&mut a, ready("a"));
    assert!(a.running.is_some());
    // Running no longer blocks the screen: another plan may start.
    let cmds = update(&mut a, key(KeyCode::Char('U')));
    assert!(matches!(cmds[0], Cmd::PlanInstall { .. }), "{cmds:?}");
    assert!(a.modal.is_none());
}

#[test]
fn details_icon_failure_never_fails_details() {
    let mut a = app();
    let (source, id) = a.selected_key().unwrap();
    let key = format!("{source}/{id}");
    let details = wingetrs::core::PackageDetails {
        source: wingetrs::core::SourceKind::Winget,
        id: id.clone(),
        name: id,
        version: "1".into(),
        publisher: None,
        description: None,
        homepage: None,
        license: None,
        tags: Vec::new(),
        installers: Vec::new(),
        available_versions: Vec::new(),
        icon: None,
    };
    update(
        &mut a,
        Msg::DetailsLoaded {
            key: key.clone(),
            result: Ok(details),
        },
    );
    assert!(matches!(a.details.get(&key), Some(Ok(_))));
    assert!(!a.icon_bytes.contains_key(&key));

    // The icon arrives later, as its own message; bytes for a key that is no longer
    // selected are still stored (keyed, so harmless).
    update(
        &mut a,
        Msg::IconLoaded {
            key: key.clone(),
            bytes: vec![1, 2, 3],
        },
    );
    assert_eq!(a.icon_bytes.get(&key), Some(&vec![1, 2, 3]));
    update(
        &mut a,
        Msg::IconLoaded {
            key: "other/x".into(),
            bytes: vec![9],
        },
    );
    assert_eq!(a.icon_bytes.get("other/x"), Some(&vec![9]));
    assert!(matches!(a.details.get(&key), Some(Ok(_))));
}

fn selected_icon_key(a: &App) -> String {
    let (s, id) = a.selected_key().unwrap();
    format!("{s}/{id}")
}

fn encode_requests(cmds: &[Cmd]) -> usize {
    cmds.iter()
        .filter(|c| matches!(c, Cmd::EncodeIcon { .. }))
        .count()
}

/// A tiny valid protocol for cache tests.
fn tiny_icon() -> ratatui_image::protocol::Protocol {
    ratatui_image::picker::Picker::halfblocks()
        .new_protocol(
            image::DynamicImage::new_rgba8(10, 20),
            ratatui::layout::Size::new(1, 1),
            ratatui_image::Resize::Fit(None),
        )
        .unwrap()
}

#[test]
fn undecodable_icon_is_remembered_as_none() {
    use wingetrs::tui::icons::{IconProtocol, encode, picker};
    let mut a = app();
    a.icon_protocol = IconProtocol::Sixel;
    let k0 = selected_icon_key(&a);
    let cmds = update(
        &mut a,
        Msg::IconLoaded {
            key: k0.clone(),
            bytes: b"lixo".to_vec(),
        },
    );
    let Some(Cmd::EncodeIcon {
        key: k,
        bytes,
        size,
        ..
    }) = cmds
        .into_iter()
        .find(|c| matches!(c, Cmd::EncodeIcon { .. }))
    else {
        panic!("no encode request");
    };
    assert_eq!((k.as_str(), size), (k0.as_str(), (8, 4)));
    let proto = encode(&picker(IconProtocol::Sixel).unwrap(), &bytes, size);
    assert!(proto.is_err());
    update(
        &mut a,
        Msg::IconReady {
            generation: 0,
            key: k0.clone(),
            size,
            proto,
        },
    );
    assert!(a.icon_none.contains(&k0));
    // Never asked again: same bytes again, or leaving the package and coming back.
    let mut again = encode_requests(&update(
        &mut a,
        Msg::IconLoaded {
            key: k0.clone(),
            bytes: b"lixo".to_vec(),
        },
    ));
    update(&mut a, key(KeyCode::Down));
    again += encode_requests(&update(&mut a, key(KeyCode::Up)));
    assert_eq!(selected_icon_key(&a), k0);
    assert_eq!(again, 0);
}

#[test]
fn icon_encoding_is_requested_once_for_the_selected_package() {
    use wingetrs::tui::icons::IconProtocol;
    let loaded = |k: &str| Msg::IconLoaded {
        key: k.into(),
        bytes: vec![1],
    };
    // Off: nothing is decoded.
    let mut off = app();
    let k = selected_icon_key(&off);
    assert_eq!(encode_requests(&update(&mut off, loaded(&k))), 0);
    assert!(off.icon_bytes.contains_key(&k));

    let mut a = app();
    a.icon_protocol = IconProtocol::Sixel;
    let first = selected_icon_key(&a);
    update(&mut a, key(KeyCode::Down));
    let second = selected_icon_key(&a);
    update(&mut a, key(KeyCode::Up));
    // A package that is not selected waits until it is.
    assert_eq!(encode_requests(&update(&mut a, loaded(&second))), 0);
    assert_eq!(encode_requests(&update(&mut a, key(KeyCode::Down))), 1);
    // In flight: not asked twice.
    assert_eq!(encode_requests(&update(&mut a, loaded(&second))), 0);
    update(
        &mut a,
        Msg::IconReady {
            generation: 0,
            key: second.clone(),
            size: (8, 4),
            proto: Ok(tiny_icon()),
        },
    );
    assert!(a.icons.get(&second, (8, 4)).is_some());
    assert_eq!(encode_requests(&update(&mut a, loaded(&first))), 0);
    // Encoded: coming back does not ask again; the first package does when selected.
    assert_eq!(encode_requests(&update(&mut a, key(KeyCode::Up))), 1);
    assert_eq!(encode_requests(&update(&mut a, key(KeyCode::Down))), 0);
}

#[test]
fn icon_cache_keeps_the_newest_64() {
    use wingetrs::tui::icons::{ICON_CACHE_MAX, IconCache};
    let mut c = IconCache::default();
    for i in 0..=ICON_CACHE_MAX {
        c.insert(format!("k{i}"), (8, 4), tiny_icon());
    }
    assert_eq!(ICON_CACHE_MAX, 64);
    assert_eq!(c.len(), 64);
    assert!(c.get("k0", (8, 4)).is_none(), "the oldest goes");
    assert!(c.get("k1", (8, 4)).is_some());
    assert!(c.get("k64", (8, 4)).is_some());
    // Keyed by size too.
    assert!(c.get("k64", (4, 2)).is_none());
}

#[test]
fn icon_evicted_from_the_cache_is_encoded_again() {
    use wingetrs::tui::icons::{ICON_CACHE_MAX, IconProtocol};
    let mut a = app();
    a.icon_protocol = IconProtocol::Sixel;
    let k = selected_icon_key(&a);
    let cmds = update(
        &mut a,
        Msg::IconLoaded {
            key: k.clone(),
            bytes: vec![1],
        },
    );
    assert_eq!(encode_requests(&cmds), 1);
    update(
        &mut a,
        Msg::IconReady {
            generation: 0,
            key: k.clone(),
            size: (8, 4),
            proto: Ok(tiny_icon()),
        },
    );
    for i in 0..ICON_CACHE_MAX {
        update(
            &mut a,
            Msg::IconReady {
                generation: 0,
                key: format!("other/{i}"),
                size: (8, 4),
                proto: Ok(tiny_icon()),
            },
        );
    }
    assert!(a.icons.get(&k, (8, 4)).is_none(), "evicted");
    update(&mut a, key(KeyCode::Down));
    assert_eq!(encode_requests(&update(&mut a, key(KeyCode::Up))), 1);
}

fn many_entries(n: usize) -> Vec<PackageEntry> {
    (0..n)
        .map(|i| {
            entry(
                SourceKind::Winget,
                &format!("Pkg.N{i:02}"),
                &format!("pkg{i:02}"),
            )
        })
        .collect()
}

/// An app with `n` packages on a terminal tall enough for exactly `page` list items.
fn icon_app(n: usize, page: usize) -> App {
    let mut a = App::new(many_entries(n), GlyphSet::Nerd);
    a.icon_protocol = wingetrs::tui::icons::IconProtocol::Sixel;
    let h = (1..200u16)
        .find(|h| usize::from(*h).saturating_sub(wingetrs::tui::app::chrome_rows(*h)) / 2 == page)
        .unwrap();
    update(&mut a, Msg::Resize(100, h));
    a
}

fn map_of(a: &App, window: std::ops::Range<usize>, picks: &[usize]) -> Vec<(String, String)> {
    let keys = a.list_keys(window.clone());
    picks
        .iter()
        .map(|p| {
            let (s, id) = keys[p - window.start].clone().unwrap();
            (format!("{s}/{id}"), format!("sha{p}"))
        })
        .collect()
}

fn cached_requests(cmds: &[Cmd]) -> Vec<(String, String)> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::EncodeCachedIcon { source, id, .. } => Some((source.clone(), id.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn only_visible_cached_icons_are_requested() {
    let mut a = icon_app(50, 10);
    let visible = map_of(&a, 0..10, &[0, 4, 9]);
    let hidden = map_of(&a, 10..50, &[10, 11, 20, 30, 49]);
    let map: std::collections::BTreeMap<String, String> =
        visible.iter().chain(&hidden).cloned().collect();
    let cmds = update(&mut a, Msg::IconMapLoaded(map));
    assert_eq!(cmds.len(), 3, "{cmds:?}");
    let got: std::collections::BTreeSet<String> = cached_requests(&cmds)
        .into_iter()
        .map(|(s, id)| format!("{s}/{id}"))
        .collect();
    let want: std::collections::BTreeSet<String> = visible.into_iter().map(|(k, _)| k).collect();
    assert_eq!(got, want);
    // Asking again while they are in flight asks for nothing.
    assert!(update(&mut a, Msg::Tick).is_empty());
}

#[test]
fn icon_requests_are_capped() {
    let mut a = icon_app(50, 20);
    let all = map_of(&a, 0..20, &(0..20).collect::<Vec<_>>());
    let cmds = update(&mut a, Msg::IconMapLoaded(all.into_iter().collect()));
    let first = cached_requests(&cmds);
    assert_eq!(first.len(), 8, "capped per frame");
    assert!(update(&mut a, Msg::Tick).is_empty(), "still 8 in flight");
    let mut next = Vec::new();
    for (source, id) in &first {
        let cmds = update(
            &mut a,
            Msg::IconReady {
                generation: 0,
                key: format!("{source}/{id}"),
                size: (4, 2),
                proto: Ok(tiny_icon()),
            },
        );
        next.extend(cached_requests(&cmds));
    }
    assert_eq!(
        next.len(),
        8,
        "the waiting ones are requested as slots free"
    );
    assert!(next.iter().all(|r| !first.contains(r)));
}

#[test]
fn list_encodes_only_cached_icons() {
    // Visible rows without a cached icon produce no encode request (fetching them is
    // `Cmd::PrefetchIcons`, which answers with `Msg::IconMapped`).
    let mut a = icon_app(50, 10);
    let mut cmds = update(&mut a, Msg::IconMapLoaded(Default::default()));
    cmds.extend(
        update(&mut a, key(KeyCode::Down))
            .into_iter()
            .filter(|c| !matches!(c, Cmd::LoadDetails { .. })),
    );
    cmds.extend(update(&mut a, Msg::Resize(100, 60)));
    cmds.retain(|c| !matches!(c, Cmd::PrefetchIcons { .. }));
    assert!(cmds.is_empty(), "{cmds:?}");
    // With a cached one the only request is a read-and-encode of that blob.
    let hit = map_of(&a, 0..3, &[1]);
    let cmds = update(
        &mut a,
        Msg::IconMapped {
            key: hit[0].0.clone(),
            sha: "abc".into(),
        },
    );
    assert!(
        matches!(&cmds[..], [Cmd::EncodeCachedIcon { sha, .. }] if sha == "abc"),
        "{cmds:?}"
    );
    // The worker that serves it never touches the network.
    let src = include_str!("../src/tui/run.rs");
    let from = src.find("fn spawn_cached_icon").unwrap();
    let to = src[from..].find("/// Runs one keystroke").unwrap();
    let body = &src[from..from + to];
    assert!(!body.contains("fetch") && !body.contains("http"), "{body}");
}

#[test]
fn icons_setting_cycles_and_saves() {
    let mut a = on_settings();
    for _ in 0..6 {
        update(&mut a, key(KeyCode::Down));
    }
    let mut seen = Vec::new();
    for _ in 0..4 {
        let cmds = update(&mut a, key(KeyCode::Enter));
        seen.push(a.settings.icons.clone());
        if seen.len() == 1 {
            assert!(matches!(
                cmds.as_slice(),
                [Cmd::SaveSettings { icons: Some(i), .. }] if i == "iterm2"
            ));
        }
        update(&mut a, Msg::SettingsSaved(Ok(())));
    }
    assert_eq!(seen, ["iterm2", "sixel", "off", "auto"]);

    // Another setting does not send `icons` while Ícones is untouched.
    let mut b = on_settings();
    let cmds = update(&mut b, key(KeyCode::Enter));
    assert!(matches!(
        cmds.as_slice(),
        [Cmd::SaveSettings { icons: None, .. }]
    ));
}

#[test]
fn icon_encoded_for_an_old_protocol_is_discarded() {
    use wingetrs::tui::icons::IconProtocol;
    let mut a = app();
    a.icon_protocol = IconProtocol::Iterm2;
    a.settings.icons = "iterm2".into();
    // The list's icon fetch is not under test here.
    a.list_icons_visible = false;
    let k = selected_icon_key(&a);
    let cmds = update(
        &mut a,
        Msg::IconLoaded {
            key: k.clone(),
            bytes: b"x".to_vec(),
        },
    );
    assert!(matches!(
        cmds.as_slice(),
        [Cmd::EncodeIcon { generation: 0, .. }]
    ));
    // Switch to Sixel (Ícones is the 7th row) while that encode is in flight.
    a.tab = Tab::Settings;
    for _ in 0..6 {
        update(&mut a, key(KeyCode::Down));
    }
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(a.icon_protocol, IconProtocol::Sixel);
    assert_eq!(a.icon_gen, 1);
    assert_eq!(
        encode_requests(&cmds),
        0,
        "no package is selected on this tab"
    );
    a.tab = Tab::Search;
    let stale = update(
        &mut a,
        Msg::IconReady {
            generation: 0,
            key: k.clone(),
            size: (8, 4),
            proto: Ok(tiny_icon()),
        },
    );
    assert!(stale.is_empty());
    assert!(a.icons.get(&k, (8, 4)).is_none(), "old encoding not cached");
    assert!(!a.icon_none.contains(&k));
    // The icon is requested again under the new generation, and that result is kept.
    let cmds = update(
        &mut a,
        Msg::IconLoaded {
            key: k.clone(),
            bytes: b"x".to_vec(),
        },
    );
    assert!(matches!(
        cmds.as_slice(),
        [Cmd::EncodeIcon { generation: 1, .. }]
    ));
    update(
        &mut a,
        Msg::IconReady {
            generation: 1,
            key: k.clone(),
            size: (8, 4),
            proto: Ok(tiny_icon()),
        },
    );
    assert!(a.icons.get(&k, (8, 4)).is_some());
}

#[test]
fn http_redirect_setting_cycles_and_saves() {
    let mut a = on_settings();
    for _ in 0..4 {
        update(&mut a, key(KeyCode::Down));
    }
    assert_eq!(
        a.settings.current_id(),
        wingetrs::tui::settings::SettingId::HttpRedirect
    );
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(a.settings.behavior.allow_http_redirect);
    assert!(matches!(
        cmds.as_slice(),
        [Cmd::SaveSettings { behavior, .. }] if behavior.allow_http_redirect
    ));
    update(&mut a, key(KeyCode::Enter));
    assert!(!a.settings.behavior.allow_http_redirect);
}

fn menu_items(a: &App) -> Vec<(String, bool, Option<String>)> {
    match &a.modal {
        Some(Modal::Actions { picker, .. }) => picker
            .items()
            .iter()
            .map(|i| (i.label.clone(), i.selectable, i.note.clone()))
            .collect(),
        _ => panic!("actions menu not open"),
    }
}

#[test]
fn enter_in_search_plans_install() {
    let mut a = app();
    select_source(&mut a, SourceKind::Scoop);
    let by_enter = update(&mut a, key(KeyCode::Enter));
    assert_eq!(by_enter.len(), 1);
    assert!(matches!(by_enter[0], Cmd::PlanInstall { keep: false, .. }));
    let mut b = app();
    select_source(&mut b, SourceKind::Scoop);
    assert_eq!(by_enter, update(&mut b, key(KeyCode::Char('i'))));
}

#[test]
fn enter_in_updates_plans_upgrade_and_empty_list_does_nothing() {
    let mut a = on_tab(
        Tab::Updates,
        vec![irow("beta", "wingetrs", "scoop:main", true)],
    );
    a.settings.behavior.keep_old = true;
    assert_eq!(
        update(&mut a, key(KeyCode::Enter)),
        vec![Cmd::PlanInstall {
            source_name: "scoop:main".into(),
            id: "beta".into(),
            keep: true
        }]
    );
    let mut empty = on_tab(
        Tab::Updates,
        vec![irow("beta", "wingetrs", "scoop:main", true)],
    );
    empty.query = "zzz".into();
    empty.selected = 0;
    assert_eq!(empty.visible_len(), 0);
    assert!(update(&mut empty, key(KeyCode::Enter)).is_empty());
    let mut none = on_tab(Tab::Updates, vec![]);
    assert!(update(&mut none, key(KeyCode::Enter)).is_empty());
}

#[test]
fn enter_in_installed_opens_actions_menu() {
    let mut a = on_tab(
        Tab::Installed,
        vec![irow("beta", "wingetrs", "scoop:main", true)],
    );
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
    assert_eq!(
        menu_items(&a),
        vec![
            ("Atualizar".to_string(), true, None),
            ("Remover".to_string(), true, None),
            ("Fixar versão".to_string(), true, None)
        ]
    );
    assert_eq!(
        update(&mut a, key(KeyCode::Enter)),
        vec![Cmd::PlanInstall {
            source_name: "scoop:main".into(),
            id: "beta".into(),
            keep: false
        }]
    );

    let mut foreign = on_tab(Tab::Installed, vec![irow("gamma", "arp", "winget", true)]);
    update(&mut foreign, key(KeyCode::Enter));
    let items = menu_items(&foreign);
    assert!(!items[0].1);
    assert_eq!(items[0].2.as_deref(), Some("instalado fora do wingetrs"));
    assert!(!items[1].1);
    assert!(
        items[1]
            .2
            .as_deref()
            .is_some_and(|n| n.contains("wingetrs"))
    );
    assert!(update(&mut foreign, key(KeyCode::Enter)).is_empty());
    assert!(
        foreign.modal.is_some(),
        "a disabled item keeps the menu open"
    );
    assert!(update(&mut foreign, key(KeyCode::Esc)).is_empty());
    assert!(foreign.modal.is_none());

    let mut current = on_tab(
        Tab::Installed,
        vec![irow("delta", "wingetrs", "scoop:main", false)],
    );
    update(&mut current, key(KeyCode::Enter));
    assert_eq!(
        menu_items(&current)[0].2.as_deref(),
        Some("sem atualização")
    );
    assert!(update(&mut current, key(KeyCode::Enter)).is_empty());
}

#[test]
fn actions_menu_remove_plans_remove() {
    let mut a = on_tab(
        Tab::Installed,
        vec![irow("beta", "wingetrs", "scoop:main", false)],
    );
    a.settings.behavior.purge = true;
    update(&mut a, key(KeyCode::Enter));
    update(&mut a, key(KeyCode::Down));
    assert_eq!(
        update(&mut a, key(KeyCode::Enter)),
        vec![Cmd::PlanRemove {
            app: "beta".into(),
            purge: true
        }]
    );
}

#[test]
fn actions_menu_never_defaults_to_remove() {
    let mut a = on_tab(
        Tab::Installed,
        vec![irow("beta", "wingetrs", "scoop:main", false)],
    );
    a.settings.behavior.confirm = wingetrs::config::ConfirmMode::Never;
    update(&mut a, key(KeyCode::Enter));
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
    assert!(matches!(a.modal, Some(Modal::Actions { .. })));
    update(&mut a, key(KeyCode::Down));
    assert!(matches!(
        update(&mut a, key(KeyCode::Enter)).as_slice(),
        [Cmd::PlanRemove { .. }]
    ));
}

fn srow(name: &str, origin: &str, source: Option<&str>, state: UpdateState) -> ListRow {
    ListRow {
        name: name.into(),
        app_key: String::new(),
        not_in_catalog: false,
        origin: origin.into(),
        installed_version: Some("1".into()),
        correlation: match source {
            Some(s) => Correlation::Matched {
                source: s.into(),
                package_id: name.into(),
                rule: Rule::ScoopManifest,
            },
            None => Correlation::Unmatched,
        },
        latest: None,
        state,
        tiebreak: None,
    }
}

fn names(a: &App) -> Vec<String> {
    a.installed_view().iter().map(|r| r.name.clone()).collect()
}

fn sort_rows() -> Vec<ListRow> {
    let avail = || UpdateState::Available { latest: "2".into() };
    vec![
        srow("delta", "arp", None, UpdateState::UpToDate),
        srow("Bravo", "wingetrs", Some("scoop:main"), avail()),
        srow("charlie", "arp", Some("winget"), UpdateState::Unknown),
        srow("alpha", "arp", None, UpdateState::Broken),
    ]
}

#[test]
fn installed_sorts_by_name_source_state() {
    let mut a = on_tab(Tab::Installed, sort_rows());
    assert_eq!(names(&a), ["alpha", "Bravo", "charlie", "delta"]);
    update(&mut a, key(KeyCode::Char('o')));
    assert_eq!(names(&a), ["Bravo", "charlie", "alpha", "delta"]);
    update(&mut a, key(KeyCode::Char('o')));
    assert_eq!(names(&a), ["Bravo", "alpha", "charlie", "delta"]);
    update(&mut a, key(KeyCode::Char('o')));
    assert_eq!(names(&a), ["alpha", "Bravo", "charlie", "delta"]);
}

#[test]
fn updates_sort_cycles_name_source() {
    let avail = || UpdateState::Available { latest: "2".into() };
    let rows = vec![
        srow("zeta", "arp", Some("winget"), avail()),
        srow("beta", "scoop", Some("scoop:main"), avail()),
        srow("alpha", "arp", Some("winget"), avail()),
    ];
    let mut a = on_tab(Tab::Updates, rows);
    assert_eq!(names(&a), ["alpha", "beta", "zeta"]);
    update(&mut a, key(KeyCode::Char('o')));
    assert_eq!(names(&a), ["beta", "alpha", "zeta"]);
    update(&mut a, key(KeyCode::Char('o')));
    assert_eq!(names(&a), ["alpha", "beta", "zeta"]);
}

#[test]
fn sort_keeps_the_selected_item() {
    let mut a = on_tab(Tab::Installed, sort_rows());
    a.selected = 1;
    assert_eq!(a.selected_key().unwrap().1, "Bravo");
    update(&mut a, key(KeyCode::Char('o')));
    assert_eq!(a.selected, 0);
    assert_eq!(a.selected_key().unwrap().1, "Bravo");
}

#[test]
fn remove_acts_on_the_sorted_row() {
    let mut a = on_tab(Tab::Installed, sort_rows());
    a.selected = 1;
    update(&mut a, key(KeyCode::Char('o')));
    let cmds = update(&mut a, key(KeyCode::Char('u')));
    let shown = a.installed_view()[a.selected].name.clone();
    assert_eq!(shown, "Bravo");
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Cmd::PlanRemove { app, .. } if app == "Bravo")),
        "{cmds:?}"
    );
}

#[test]
#[ignore = "live: timing of installed_view with 2000 rows, run with --release"]
fn installed_view_sorted_by_state_timing() {
    let rows: Vec<ListRow> = (0..2000)
        .map(|i| {
            let st = match i % 4 {
                0 => UpdateState::UpToDate,
                1 => UpdateState::Unknown,
                2 => UpdateState::Broken,
                _ => UpdateState::Available { latest: "2".into() },
            };
            srow(
                &format!("App{}", (i * 7919) % 2000),
                "arp",
                Some("winget"),
                st,
            )
        })
        .collect();
    let mut a = on_tab(Tab::Installed, rows);
    a.sort_installed = wingetrs::tui::app::SortKey::State;
    let mut t: Vec<std::time::Duration> = (0..200)
        .map(|_| {
            let s = std::time::Instant::now();
            std::hint::black_box(a.installed_view());
            s.elapsed()
        })
        .collect();
    t.sort();
    eprintln!("median installed_view(2000, State) = {:?}", t[100]);
}

#[test]
fn sort_hint_shows_current_order() {
    use wingetrs::tui::help::hints;
    let mut a = on_tab(Tab::Installed, sort_rows());
    let desc = |a: &App| hints(a).iter().find(|h| h.0 == "o").unwrap().1;
    assert_eq!(desc(&a), "ordem: nome");
    update(&mut a, key(KeyCode::Char('o')));
    assert_eq!(desc(&a), "ordem: fonte");
    let mut u = on_tab(Tab::Updates, sort_rows());
    for _ in 0..4 {
        assert_ne!(desc(&u), "ordem: estado");
        update(&mut u, key(KeyCode::Char('o')));
    }
}

#[test]
fn rescan_keeps_the_selected_item() {
    let mut a = on_tab(Tab::Installed, sort_rows());
    update(&mut a, key(KeyCode::Char('o')));
    // By source: Bravo, charlie, alpha, delta.
    a.selected = 1;
    assert_eq!(a.selected_key().unwrap().1, "charlie");
    let mut rows = sort_rows();
    rows.push(srow(
        "aaa",
        "scoop",
        Some("scoop:aaa"),
        UpdateState::UpToDate,
    ));
    update(&mut a, Msg::InstalledLoaded(rows));
    assert_eq!(a.selected_key().unwrap().1, "charlie");
    assert_eq!(a.selected, 2);
}

/// `(generation, keys)` of every `Cmd::PrefetchIcons` in `cmds`.
fn prefetches(cmds: &[Cmd]) -> Vec<(u64, Vec<(String, String)>)> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::PrefetchIcons { generation, keys } => Some((*generation, keys.clone())),
            _ => None,
        })
        .collect()
}

/// The rows of `window`, in screen order.
fn row_keys(a: &App, window: std::ops::Range<usize>) -> Vec<(String, String)> {
    a.list_keys(window).into_iter().flatten().collect()
}

#[test]
fn prefetch_asks_for_visible_rows_once() {
    let mut a = app();
    a.icon_protocol = wingetrs::tui::icons::IconProtocol::Sixel;
    assert_eq!(a.list_keys(0..3).into_iter().flatten().count(), 3);
    let want = row_keys(&a, 0..3);
    assert_eq!(want.len(), 3, "{want:?}");
    let got = prefetches(&update(&mut a, Msg::Resize(100, 40)));
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, want);
    // The same screen again asks for nothing.
    assert!(prefetches(&update(&mut a, Msg::Resize(100, 40))).is_empty());
    assert!(prefetches(&update(&mut a, Msg::Tick)).is_empty());
}

#[test]
fn prefetch_skips_mapped_and_off() {
    let mut a = app();
    a.icon_protocol = wingetrs::tui::icons::IconProtocol::Sixel;
    let all = row_keys(&a, 0..3);
    let (s, id) = &all[0];
    a.icon_map.insert(format!("{s}/{id}"), "sha".into());
    let got = prefetches(&update(&mut a, Msg::Resize(100, 40)));
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, all[1..].to_vec());

    let mut off = app();
    assert_eq!(off.icon_protocol, wingetrs::tui::icons::IconProtocol::Off);
    assert!(prefetches(&update(&mut off, Msg::Resize(100, 40))).is_empty());

    let mut modal = app();
    modal.icon_protocol = wingetrs::tui::icons::IconProtocol::Sixel;
    modal.modal = Some(Modal::Queue { selected: 0 });
    assert!(prefetches(&update(&mut modal, Msg::Resize(100, 40))).is_empty());
    // Closing it asks for the rows now in view.
    modal.modal = None;
    assert_eq!(prefetches(&update(&mut modal, Msg::Tick)).len(), 1);

    let mut help = app();
    help.icon_protocol = wingetrs::tui::icons::IconProtocol::Sixel;
    help.show_help = true;
    assert!(prefetches(&update(&mut help, Msg::Resize(100, 40))).is_empty());

    let mut hidden = app();
    hidden.icon_protocol = wingetrs::tui::icons::IconProtocol::Sixel;
    hidden.list_icons_visible = false;
    assert!(prefetches(&update(&mut hidden, Msg::Resize(100, 40))).is_empty());
}

#[test]
fn scrolling_bumps_the_generation() {
    let mut a = icon_app(50, 10);
    let first = a.list_fetch_gen;
    assert!(first > 0, "the first screen was asked for");
    let mut got = Vec::new();
    for _ in 0..10 {
        got.extend(prefetches(&update(&mut a, key(KeyCode::Down))));
    }
    // One row scrolled in: the new request covers every visible row not done yet, so
    // superseding the first one loses nothing still on screen.
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].0 > first);
    assert_eq!(got[0].1, row_keys(&a, 1..11));
    let second = got[0].0;
    let got = prefetches(&update(&mut a, key(KeyCode::End)));
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].0 > second);
    assert_eq!(got[0].1, row_keys(&a, 40..50));
}

#[test]
fn icon_mapped_encodes_the_visible_row() {
    let mut a = icon_app(50, 10);
    let (s, id) = row_keys(&a, 3..4).remove(0);
    let cmds = update(
        &mut a,
        Msg::IconMapped {
            key: format!("{s}/{id}"),
            sha: "fresh".into(),
        },
    );
    assert!(
        cmds.iter().any(|c| matches!(c,
            Cmd::EncodeCachedIcon { source, id: i, sha, .. }
                if *source == s && *i == id && sha == "fresh")),
        "{cmds:?}"
    );
}

fn done(a: &mut App, (s, id): &(String, String)) -> Vec<Cmd> {
    update(
        a,
        Msg::PrefetchDone {
            key: format!("{s}/{id}"),
        },
    )
}

#[test]
fn alternating_screens_converge() {
    let mut a = icon_app(50, 10);
    let first = row_keys(&a, 0..10);
    // Nothing changes on screen: nothing more is asked.
    let size = a.size;
    for _ in 0..5 {
        assert!(prefetches(&update(&mut a, Msg::Tick)).is_empty());
        assert!(prefetches(&update(&mut a, Msg::Resize(size.0, size.1))).is_empty());
    }
    let scroll = |a: &mut App, k: KeyCode| {
        (0..10)
            .flat_map(|_| prefetches(&update(a, key(k))))
            .collect::<Vec<_>>()
    };
    let got = scroll(&mut a, KeyCode::Down);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, row_keys(&a, 1..11));
    let got = scroll(&mut a, KeyCode::Up);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, first);
    // Once every attempt is done, going back and forth asks for nothing.
    for k in row_keys(&a, 0..11) {
        assert!(prefetches(&done(&mut a, &k)).is_empty());
    }
    for _ in 0..3 {
        assert!(scroll(&mut a, KeyCode::Down).is_empty());
        assert!(scroll(&mut a, KeyCode::Up).is_empty());
    }
}

#[test]
fn shrinking_never_reemits() {
    let mut a = icon_app(50, 10);
    let keys = row_keys(&a, 0..10);
    assert!(prefetches(&done(&mut a, &keys[0])).is_empty());
    let (s, id) = &keys[1];
    let mapped = update(
        &mut a,
        Msg::IconMapped {
            key: format!("{s}/{id}"),
            sha: "x".into(),
        },
    );
    assert!(prefetches(&mapped).is_empty(), "{mapped:?}");
    // A done row scrolled out and back in is not asked again.
    let mut got = Vec::new();
    for _ in 0..10 {
        got.extend(prefetches(&update(&mut a, key(KeyCode::Down))));
    }
    for _ in 0..10 {
        got.extend(prefetches(&update(&mut a, key(KeyCode::Up))));
    }
    let mut want = keys[2..].to_vec();
    want.extend(row_keys(&a, 10..11));
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, want);
}

#[test]
fn list_icons_setting_cycles_saves_and_applies() {
    let mut a = icon_app(50, 10);
    a.tab = Tab::Settings;
    for _ in 0..7 {
        update(&mut a, key(KeyCode::Down));
    }
    assert_eq!(a.settings.current_id().label(), "Ícones da lista");
    assert!(a.list_icons_visible);
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(a.settings.list_icons, "opened");
    assert!(!a.list_icons_visible);
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::SaveSettings { list_icons: Some(l), icons: None, .. }] if l == "opened"
        ),
        "{cmds:?}"
    );
    update(&mut a, Msg::SettingsSaved(Ok(())));
    // Scrolling the list asks for no fetch now.
    a.tab = Tab::Search;
    for _ in 0..10 {
        assert!(prefetches(&update(&mut a, key(KeyCode::Down))).is_empty());
    }

    // Another setting does not send `list_icons` while it is untouched.
    let mut b = on_settings();
    let cmds = update(&mut b, key(KeyCode::Enter));
    assert!(matches!(
        cmds.as_slice(),
        [Cmd::SaveSettings {
            list_icons: None,
            ..
        }]
    ));
}

fn on_list_icons_row() -> App {
    let mut a = on_settings();
    for _ in 0..7 {
        update(&mut a, key(KeyCode::Down));
    }
    a
}

#[test]
fn list_icons_change_during_save_is_saved_next() {
    let mut a = on_list_icons_row();
    let first = update(&mut a, key(KeyCode::Enter));
    assert!(matches!(
        first.as_slice(),
        [Cmd::SaveSettings { list_icons: Some(l), .. }] if l == "opened"
    ));
    // Back to visible while the first save is in flight: no new save yet.
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
    let cmds = update(&mut a, Msg::SettingsSaved(Ok(())));
    assert!(
        matches!(
            cmds.as_slice(),
            [Cmd::SaveSettings { list_icons: Some(l), .. }] if l == "visible"
        ),
        "{cmds:?}"
    );
}

#[test]
fn list_icons_reverts_on_save_failure() {
    let mut a = on_list_icons_row();
    update(&mut a, key(KeyCode::Enter));
    assert!(!a.list_icons_visible);
    update(&mut a, Msg::SettingsSaved(Err("x".into())));
    assert_eq!(a.settings.list_icons, "visible");
    assert!(a.list_icons_visible);
}

// ---- Task 94: mouse ----

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::time::Instant;
use wingetrs::tui::regions::{item_at, regions};

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Msg {
    Msg::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn click_at(a: &mut App, at: Instant, column: u16, row: u16) -> Vec<Cmd> {
    a.now = at;
    update(
        a,
        mouse(MouseEventKind::Down(MouseButton::Left), column, row),
    )
}

fn ten_hits() -> App {
    let es = (0..10)
        .map(|i| entry(SourceKind::Scoop, &format!("pkg{i}"), &format!("Pkg{i}")))
        .collect();
    let mut a = App::new(es, GlyphSet::Ascii);
    a.size = (100, 30);
    a
}

fn screen(a: &App) -> Rect {
    Rect::new(0, 0, a.size.0, a.size.1)
}

#[test]
fn click_on_tab_switches_tab() {
    let mut a = ten_hits();
    let r = regions(&a, screen(&a));
    let (rect, tab) = r.tabs[1];
    assert_eq!(tab, Tab::Installed);
    click_at(&mut a, Instant::now(), rect.x + rect.width / 2, rect.y + 1);
    assert_eq!(a.tab, Tab::Installed);
}

#[test]
fn click_on_item_selects_it() {
    let mut a = ten_hits();
    a.selected = 3;
    a.scroll = 3;
    let list = regions(&a, screen(&a)).list;
    let t0 = Instant::now();
    // Second visible item (index 4), first line.
    click_at(&mut a, t0, list.x + 2, list.y + 2);
    assert_eq!(a.selected, 4);
    assert_eq!(item_at(&a, list, list.x, list.y), Some(3));
}

#[test]
fn double_click_runs_enter() {
    let t0 = Instant::now();
    let item_cell = |a: &App, n: u16| {
        let list = regions(a, screen(a)).list;
        (list.x + 2, list.y + 2 * n)
    };
    // Reference: one click then Enter.
    let mut b = ten_hits();
    let (x, y) = item_cell(&b, 1);
    click_at(&mut b, t0, x, y);
    let enter = update(&mut b, key(KeyCode::Enter));
    assert!(!enter.is_empty(), "Enter on a search row plans an install");

    let mut a = ten_hits();
    click_at(&mut a, t0, x, y);
    let double = click_at(&mut a, t0 + Duration::from_millis(100), x, y);
    assert_eq!(double, enter);

    let mut slow = ten_hits();
    click_at(&mut slow, t0, x, y);
    let late = click_at(&mut slow, t0 + Duration::from_millis(500), x, y);
    assert!(late.is_empty());
    assert_eq!(slow.selected, 1);

    let mut other = ten_hits();
    click_at(&mut other, t0, x, y);
    let (x2, y2) = item_cell(&other, 2);
    let second = click_at(&mut other, t0 + Duration::from_millis(100), x2, y2);
    assert_ne!(second, enter);
    assert_eq!(other.selected, 2);
}

#[test]
fn wheel_scrolls_the_list() {
    let mut a = ten_hits();
    let list = regions(&a, screen(&a)).list;
    let (x, y) = (list.x + 2, list.y + 1);
    update(&mut a, mouse(MouseEventKind::ScrollDown, x, y));
    assert_eq!(a.selected, 3);
    for _ in 0..5 {
        update(&mut a, mouse(MouseEventKind::ScrollDown, x, y));
    }
    assert_eq!(a.selected, 9);
    update(&mut a, mouse(MouseEventKind::ScrollUp, x, y));
    assert_eq!(a.selected, 6);
}

#[test]
fn mouse_ignored_under_modal_and_when_disabled() {
    let mut a = ten_hits();
    let list = regions(&a, screen(&a)).list;
    let (x, y) = (list.x + 2, list.y + 3);
    a.mouse_enabled = false;
    click_at(&mut a, Instant::now(), x, y);
    update(&mut a, mouse(MouseEventKind::ScrollDown, x, y));
    assert_eq!(a.selected, 0);
    a.mouse_enabled = true;
    update(&mut a, ready_confirm());
    assert!(a.modal.is_some());
    click_at(&mut a, Instant::now(), x, y);
    update(&mut a, mouse(MouseEventKind::ScrollDown, x, y));
    assert_eq!(a.selected, 0);
    assert!(a.modal.is_some());
}

fn ready_confirm() -> Msg {
    Msg::Planned(Ok(wingetrs::tui::app::Planned::Ready {
        text: "x".into(),
        action: wingetrs::tui::app::Action::Apply(vec![]),
    }))
}

#[test]
fn settings_click_selects_row() {
    let mut a = ten_hits();
    a.tab = Tab::Settings;
    let id = wingetrs::tui::settings::ORDER[2];
    let before = a.settings.value_index(id);
    let rows = regions(&a, screen(&a)).setting_rows;
    let rect = rows[2];
    click_at(&mut a, Instant::now(), rect.x + 3, rect.y);
    assert_eq!(a.settings.selected, 2);
    assert_eq!(a.settings.value_index(id), before);
}

#[test]
fn click_below_the_last_item_hits_nothing() {
    let mut a = app();
    a.size = (100, 30);
    let list = regions(&a, screen(&a)).list;
    // Three items take six lines; the line right after them is empty.
    assert_eq!(item_at(&a, list, list.x, list.y + 5), Some(2));
    assert_eq!(item_at(&a, list, list.x, list.y + 6), None);
    click_at(&mut a, Instant::now(), list.x + 2, list.y + 6);
    assert_eq!(a.selected, 0);
}

#[test]
fn motion_is_not_a_message() {
    use wingetrs::tui::run::mouse_msg;
    let ev = |kind| MouseEvent {
        kind,
        column: 1,
        row: 1,
        modifiers: KeyModifiers::NONE,
    };
    assert!(mouse_msg(ev(MouseEventKind::Moved)).is_none());
    assert!(mouse_msg(ev(MouseEventKind::Drag(MouseButton::Left))).is_none());
    assert!(mouse_msg(ev(MouseEventKind::Up(MouseButton::Left))).is_none());
    assert!(mouse_msg(ev(MouseEventKind::Down(MouseButton::Right))).is_none());
    assert!(mouse_msg(ev(MouseEventKind::Down(MouseButton::Left))).is_some());
    assert!(mouse_msg(ev(MouseEventKind::ScrollDown)).is_some());
    assert!(mouse_msg(ev(MouseEventKind::ScrollUp)).is_some());
}

// ---- Task 95: mouse setting; every setting joins save coalescing ----

fn select_setting(a: &mut App, id: wingetrs::tui::settings::SettingId) {
    a.settings.selected = wingetrs::tui::settings::ORDER
        .iter()
        .position(|s| *s == id)
        .unwrap();
}

fn saves(cmds: &[Cmd]) -> usize {
    cmds.iter()
        .filter(|c| matches!(c, Cmd::SaveSettings { .. }))
        .count()
}

#[test]
fn mouse_setting_toggles_capture() {
    use wingetrs::tui::settings::SettingId;
    let mut a = ten_hits();
    a.tab = Tab::Settings;
    select_setting(&mut a, SettingId::Mouse);
    assert!(a.mouse_enabled);
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(!a.mouse_enabled);
    assert_eq!(
        cmds.iter()
            .filter(|c| matches!(c, Cmd::SetMouseCapture(false)))
            .count(),
        1,
        "{cmds:?}"
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Cmd::SaveSettings { behavior, .. } if !behavior.mouse
        )),
        "{cmds:?}"
    );
    // A click on a tab no longer switches it.
    let (rect, _) = regions(&a, screen(&a)).tabs[0];
    click_at(&mut a, Instant::now(), rect.x + rect.width / 2, rect.y + 1);
    assert_eq!(a.tab, Tab::Settings);
}

#[test]
fn mouse_setting_reverts_capture_on_save_failure() {
    use wingetrs::tui::settings::SettingId;
    let mut a = ten_hits();
    a.tab = Tab::Settings;
    select_setting(&mut a, SettingId::Mouse);
    update(&mut a, key(KeyCode::Enter));
    let cmds = update(&mut a, Msg::SettingsSaved(Err("x".into())));
    assert!(a.mouse_enabled);
    assert!(cmds.contains(&Cmd::SetMouseCapture(true)), "{cmds:?}");
}

#[test]
fn every_setting_joins_save_coalescing() {
    use wingetrs::tui::settings::ORDER;
    for id in ORDER {
        let fresh = || {
            let mut a = on_settings();
            select_setting(&mut a, id);
            a
        };
        // `Glyphs` forced by a flag cannot change; `fresh` never forces it.
        let original = fresh().settings.value_index(id);

        // Ok: the change made during the save is saved next.
        let mut a = fresh();
        let first = update(&mut a, key(KeyCode::Enter));
        assert_eq!(saves(&first), 1, "{id:?}: {first:?}");
        let during = update(&mut a, key(KeyCode::Enter));
        assert_eq!(saves(&during), 0, "{id:?}: {during:?}");
        let newest = a.settings.value_index(id);
        let next = update(&mut a, Msg::SettingsSaved(Ok(())));
        assert_eq!(saves(&next), 1, "{id:?} lost its change: {next:?}");
        assert_eq!(a.settings.value_index(id), newest, "{id:?}");

        // Err: a newer change retries, and the failure of the retry reverts to the last saved.
        let mut a = fresh();
        update(&mut a, key(KeyCode::Enter));
        update(&mut a, key(KeyCode::Enter));
        let retry = update(&mut a, Msg::SettingsSaved(Err("x".into())));
        assert_eq!(saves(&retry), 1, "{id:?}: {retry:?}");
        update(&mut a, Msg::SettingsSaved(Err("x".into())));
        assert_eq!(
            a.settings.value_index(id),
            original,
            "{id:?} did not revert"
        );
    }
}

// ---- operation queue (Task 96) ----

fn finished(summary: &str) -> Msg {
    Msg::InstallFinished(Ok(Done {
        summary: summary.into(),
        residues: None,
    }))
}

fn queued_labels(a: &App) -> Vec<&str> {
    a.queue.iter().map(|q| q.label.as_str()).collect()
}

fn running_label(a: &App) -> Option<&str> {
    a.running.as_ref().map(|r| r.label.as_str())
}

fn executes(cmds: &[Cmd]) -> Vec<&Cmd> {
    cmds.iter()
        .filter(|c| matches!(c, Cmd::Execute(_)))
        .collect()
}

/// Confirmation off: every ready plan goes straight to the queue.
fn unconfirmed() -> App {
    let mut a = app();
    a.settings.behavior.confirm = ConfirmMode::Never;
    a
}

#[test]
fn second_action_waits_in_queue() {
    let mut a = app();
    update(&mut a, ready("a"));
    assert_eq!(
        update(&mut a, key(KeyCode::Enter)),
        vec![Cmd::Execute(apply_action("a"))]
    );
    assert_eq!(running_label(&a), Some("Instalando a"));
    assert!(a.modal.is_none());

    update(&mut a, ready("b"));
    assert!(confirm_open(&a));
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(executes(&cmds).is_empty(), "{cmds:?}");
    assert_eq!(queued_labels(&a), ["Instalando b"]);
    assert_eq!(a.toasts[0].text, "na fila: Instalando b (posição 1)");
    assert_eq!(running_label(&a), Some("Instalando a"));

    let cmds = update(&mut a, finished("Instalado: a"));
    assert_eq!(executes(&cmds), [&Cmd::Execute(apply_action("b"))]);
    assert!(cmds.contains(&Cmd::ScanInstalled), "{cmds:?}");
    assert!(a.toasts.iter().any(|t| t.text == "Instalado: a"));
    assert_eq!(running_label(&a), Some("Instalando b"));
    assert!(a.queue.is_empty());
}

#[test]
fn starting_at_once_shows_no_queue_toast() {
    let mut a = unconfirmed();
    update(&mut a, ready("a"));
    assert!(a.toasts.iter().all(|t| !t.text.starts_with("na fila")));
}

#[test]
fn navigation_works_while_running() {
    let mut a = unconfirmed();
    a.size = (100, 30);
    update(&mut a, ready("a"));
    assert!(a.running.is_some());
    assert!(a.modal.is_none());
    let before = a.selected;
    update(&mut a, key(KeyCode::Down));
    assert_eq!(a.selected, before + 1);
    update(&mut a, key(KeyCode::Tab));
    assert_eq!(a.tab, Tab::Installed);
    let r = regions(&a, screen(&a));
    let (rect, tab) = r.tabs[0];
    assert_eq!(tab, Tab::Search);
    click_at(&mut a, Instant::now(), rect.x + rect.width / 2, rect.y + 1);
    assert_eq!(a.tab, Tab::Search, "the mouse works while running");
    update(&mut a, key(KeyCode::Char('z')));
    assert_eq!(a.query, "z");
    assert!(a.modal.is_none());
    assert_eq!(running_label(&a), Some("Instalando a"));
    update(
        &mut a,
        Msg::InstallProgress {
            label: "a.zip".into(),
            done: 5,
            total: Some(10),
        },
    );
    let progress = a.running.as_ref().and_then(|r| r.progress.as_ref());
    assert_eq!(progress.map(|p| p.done), Some(5));
}

#[test]
fn residues_dialog_pauses_the_queue() {
    for close in [KeyCode::Esc, KeyCode::Enter] {
        let mut a = unconfirmed();
        a.settings.behavior.residues = ResidueMode::Ask;
        update(
            &mut a,
            Msg::Planned(Ok(Planned::Ready {
                text: "Remover a".into(),
                action: removal("a"),
            })),
        );
        assert_eq!(running_label(&a), Some("Removendo a"));
        update(&mut a, ready("b"));
        assert_eq!(queued_labels(&a), ["Instalando b"]);

        let cmds = update(&mut a, removed_with_residues());
        assert!(matches!(a.modal, Some(Modal::Residues { .. })));
        assert!(executes(&cmds).is_empty(), "{cmds:?}");
        assert!(a.running.is_none());
        // Moving inside the dialog does not let the queue go on.
        let cmds = update(&mut a, key(KeyCode::Down));
        assert!(executes(&cmds).is_empty(), "{cmds:?}");
        assert!(matches!(a.modal, Some(Modal::Residues { .. })));

        let cmds = update(&mut a, key(close));
        if close == KeyCode::Esc {
            assert_eq!(cmds, vec![Cmd::Execute(apply_action("b"))]);
        } else {
            // The chosen cleanup runs first; the queue continues when it ends.
            assert_eq!(
                cmds,
                vec![Cmd::CleanResidues {
                    app: "a".into(),
                    residues: vec![residue("p1", true)]
                }]
            );
            assert_eq!(running_label(&a), Some("Limpando resíduos de a"));
            let cmds = update(&mut a, Msg::ResiduesCleaned(vec![]));
            assert_eq!(cmds, vec![Cmd::Execute(apply_action("b"))]);
        }
        assert!(a.modal.is_none());
        assert_eq!(running_label(&a), Some("Instalando b"));
    }
}

#[test]
fn busy_requeues_at_front_and_retries() {
    let mut a = unconfirmed();
    update(&mut a, ready("a"));
    update(&mut a, ready("b"));
    let cmds = update(&mut a, Msg::InstallFinished(Err(InstallFail::Busy)));
    assert!(cmds.contains(&Cmd::RetryQueue), "{cmds:?}");
    assert!(executes(&cmds).is_empty(), "{cmds:?}");
    assert_eq!(queued_labels(&a), ["Instalando a", "Instalando b"]);
    assert!(a.running.is_none());
    assert_eq!(a.toasts[0].text, "outra instalação em andamento");
    // Waiting for the lock: a new action does not jump ahead.
    let cmds = update(&mut a, ready("c"));
    assert!(executes(&cmds).is_empty(), "{cmds:?}");
    assert_eq!(
        update(&mut a, Msg::RetryQueue),
        vec![Cmd::Execute(apply_action("a"))]
    );
    assert_eq!(queued_labels(&a), ["Instalando b", "Instalando c"]);
}

#[test]
fn queue_modal_removes_pending() {
    use wingetrs::tui::app::queue_rows;
    let mut a = unconfirmed();
    a.size = (100, 30);
    for name in ["a", "b", "c"] {
        update(&mut a, ready(name));
    }
    assert!(update(&mut a, key(KeyCode::Char('f'))).is_empty());
    assert!(matches!(a.modal, Some(Modal::Queue { .. })));
    let rows = queue_rows(&a);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].1.label, "em andamento: Instalando a");
    assert!(!rows[0].1.selectable);
    assert_eq!(rows[1].1.label, "Instalando b");
    // The mouse is ignored under the queue dialog.
    let r = regions(&a, screen(&a));
    let (rect, _) = r.tabs[1];
    click_at(&mut a, Instant::now(), rect.x + rect.width / 2, rect.y + 1);
    assert_eq!(a.tab, Tab::Search);
    // The cursor starts on the running line, which cannot be removed.
    update(&mut a, key(KeyCode::Char('x')));
    assert_eq!(queued_labels(&a), ["Instalando b", "Instalando c"]);
    update(&mut a, key(KeyCode::Down));
    update(&mut a, key(KeyCode::Char('x')));
    assert_eq!(queued_labels(&a), ["Instalando c"]);
    assert_eq!(a.toasts[0].text, "removido da fila: Instalando b");
    assert_eq!(running_label(&a), Some("Instalando a"));
    update(&mut a, key(KeyCode::Delete));
    assert!(a.queue.is_empty());
    update(&mut a, key(KeyCode::Char('f')));
    assert!(a.modal.is_none());
}

#[test]
fn f_with_nothing_queued_says_so() {
    let mut a = app();
    update(&mut a, key(KeyCode::Char('f')));
    assert!(a.modal.is_none());
    assert_eq!(a.toasts[0].text, "fila vazia");
    // With a query, `f` is text.
    let mut t = unconfirmed();
    update(&mut t, ready("a"));
    update(&mut t, key(KeyCode::Char('r')));
    update(&mut t, key(KeyCode::Char('f')));
    assert_eq!(t.query, "rf");
    assert!(t.modal.is_none());
}

#[test]
fn quit_waits_for_running() {
    let mut a = unconfirmed();
    update(&mut a, ready("a"));
    update(&mut a, ready("b"));
    assert!(update(&mut a, key(KeyCode::Char('q'))).is_empty());
    let Some(Modal::Confirm { plan_text, .. }) = &a.modal else {
        panic!("no quit dialog");
    };
    assert_eq!(
        plan_text,
        "Há 1 operações na fila e uma em andamento. Sair cancela as pendentes; a em andamento termina antes de fechar."
    );
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(!cmds.contains(&Cmd::Quit), "{cmds:?}");
    assert!(a.queue.is_empty());
    assert!(a.modal.is_none());
    let cmds = update(&mut a, finished("Instalado: a"));
    assert!(cmds.contains(&Cmd::Quit), "{cmds:?}");
    assert!(executes(&cmds).is_empty(), "{cmds:?}");
}

#[test]
fn a_dialog_arriving_meanwhile_waits_for_the_open_one() {
    let mut a = app();
    a.settings.behavior.residues = ResidueMode::Ask;
    update(&mut a, ready("a"));
    update(&mut a, key(KeyCode::Enter));
    // B's confirmation is on screen when A ends with residues.
    update(&mut a, ready("b"));
    assert!(confirm_open(&a));
    let cmds = update(&mut a, removed_with_residues());
    assert!(confirm_open(&a), "the residues dialog must not replace it");
    assert!(executes(&cmds).is_empty(), "{cmds:?}");
    // Confirming B queues it: the residues dialog still has to be answered first.
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(executes(&cmds).is_empty(), "{cmds:?}");
    assert!(matches!(a.modal, Some(Modal::Residues { .. })));
    assert_eq!(queued_labels(&a), ["Instalando b"]);
    assert_eq!(
        update(&mut a, key(KeyCode::Esc)),
        vec![Cmd::Execute(apply_action("b"))]
    );
}

#[test]
fn esc_and_ctrl_c_also_ask_while_running() {
    for quit in [KeyCode::Esc, KeyCode::Char('c')] {
        let mut a = unconfirmed();
        update(&mut a, ready("a"));
        let msg = if quit == KeyCode::Esc {
            key(quit)
        } else {
            ctrl('c')
        };
        assert!(update(&mut a, msg).is_empty());
        assert!(confirm_open(&a));
        // Esc on the dialog keeps everything running.
        update(&mut a, key(KeyCode::Esc));
        assert!(a.modal.is_none());
        assert_eq!(running_label(&a), Some("Instalando a"));
    }
    let mut idle = app();
    assert_eq!(update(&mut idle, key(KeyCode::Char('q'))), vec![Cmd::Quit]);
}

#[test]
fn nothing_is_queued_once_quitting() {
    let mut a = unconfirmed();
    update(&mut a, ready("a"));
    update(&mut a, key(KeyCode::Char('q')));
    update(&mut a, key(KeyCode::Enter));
    assert!(a.quit_after_running);
    let cmds = update(&mut a, ready("b"));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert!(a.queue.is_empty());
    assert_eq!(a.toasts[0].text, "saindo: operação não enfileirada");
}

fn quit_text(a: &mut App) -> String {
    update(a, key(KeyCode::Char('q')));
    match &a.modal {
        Some(Modal::Confirm { plan_text, .. }) => plan_text.clone(),
        _ => panic!("no quit dialog"),
    }
}

#[test]
fn quit_text_fits_what_is_running() {
    let mut a = unconfirmed();
    update(&mut a, ready("a"));
    assert_eq!(
        quit_text(&mut a),
        "Há uma operação em andamento; ela termina antes de fechar."
    );
    let mut r = unconfirmed();
    r.settings.behavior.residues = ResidueMode::Ask;
    update(
        &mut r,
        Msg::Planned(Ok(Planned::Ready {
            text: "Remover a".into(),
            action: removal("a"),
        })),
    );
    assert_eq!(
        quit_text(&mut r),
        "Há uma operação em andamento; ela termina antes de fechar. Resíduos dela não serão limpos."
    );
    let mut keep = unconfirmed();
    keep.settings.behavior.residues = ResidueMode::Keep;
    update(
        &mut keep,
        Msg::Planned(Ok(Planned::Ready {
            text: "Remover a".into(),
            action: removal("a"),
        })),
    );
    assert!(!quit_text(&mut keep).contains("Resíduos"));
}

#[test]
fn second_ctrl_c_forces_quit() {
    let mut a = unconfirmed();
    update(&mut a, ready("a"));
    assert!(update(&mut a, ctrl('c')).is_empty());
    assert!(confirm_open(&a));
    assert_eq!(update(&mut a, ctrl('c')), vec![Cmd::Quit]);
    // Also after quitting was confirmed and the job seems stuck.
    let mut b = unconfirmed();
    update(&mut b, ready("a"));
    update(&mut b, ctrl('c'));
    assert!(update(&mut b, key(KeyCode::Enter)).is_empty());
    assert_eq!(update(&mut b, ctrl('c')), vec![Cmd::Quit]);
    // A plan's confirmation still ignores Ctrl+C.
    let mut c = app();
    update(&mut c, ready("a"));
    assert!(update(&mut c, ctrl('c')).is_empty());
}
// ---- marks (Task 97) ----

fn space(a: &mut App) {
    step(a, key(KeyCode::Char(' ')));
}

fn mark_all(a: &mut App) {
    for i in 0..a.visible_len() {
        a.selected = i;
        space(a);
    }
}

fn ready_remove(app: &str) -> Result<Planned, String> {
    Ok(Planned::Ready {
        text: format!("Remover {app}"),
        action: removal(app),
    })
}

fn ready_apply(app: &str) -> Result<Planned, String> {
    Ok(Planned::Ready {
        text: format!("Instalar {app}"),
        action: apply_action(app),
    })
}

fn has_toast(a: &App, needle: &str) -> bool {
    a.toasts.iter().any(|t| t.text.contains(needle))
}

fn row_key(origin: &str, name: &str) -> (String, String) {
    (origin.to_string(), name.to_string())
}

fn plan_remove_cmd(app: &str) -> Cmd {
    Cmd::PlanRemove {
        app: app.into(),
        purge: false,
    }
}

#[test]
fn space_toggles_mark_in_state() {
    let mut a = on_tab(
        Tab::Installed,
        vec![irow("alpha", "wingetrs", "scoop:main", false)],
    );
    space(&mut a);
    assert_eq!(a.marked_count(), 1);
    assert!(a.is_marked(&row_key("wingetrs", "alpha")));
    assert_eq!(a.query, "", "space is not text");
    space(&mut a);
    assert_eq!(a.marked_count(), 0);
}

#[test]
fn upgrade_marked_plans_batch_and_one_confirm() {
    let mut a = on_tab(
        Tab::Updates,
        vec![
            irow("alpha", "wingetrs", "scoop:main", true),
            irow("beta", "wingetrs", "scoop:main", true),
            irow("gamma", "arp", "winget", true),
        ],
    );
    mark_all(&mut a);
    assert_eq!(a.marked_count(), 3);
    let cmds = update(&mut a, key(KeyCode::Enter));
    let items = match cmds.as_slice() {
        [Cmd::PlanBatch { items }] => items.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(
        items
            .iter()
            .all(|i| matches!(i, Cmd::PlanInstall { id, .. } if id != "gamma"))
    );
    update(
        &mut a,
        Msg::BatchPlanned(vec![ready_apply("alpha"), ready_apply("beta")]),
    );
    let Some(Modal::Confirm { plan_text, .. }) = &a.modal else {
        panic!("no confirm");
    };
    assert!(
        plan_text.contains("alpha") && plan_text.contains("beta"),
        "{plan_text}"
    );
    assert!(
        a.toasts
            .iter()
            .any(|t| t.text.contains("pulados") && t.text.contains("gamma")),
        "{:?}",
        a.toasts
    );
    assert_eq!(a.marked_count(), 3, "marks stay until queued");
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(
        cmds,
        vec![Cmd::Execute(Action::Apply(vec![
            vec![plan("alpha")],
            vec![plan("beta")]
        ]))]
    );
    assert_eq!(a.marked_count(), 0);
}

#[test]
fn remove_marked_enqueues_one_per_app() {
    let mut a = on_tab(
        Tab::Installed,
        vec![
            irow("alpha", "wingetrs", "scoop:main", false),
            irow("beta", "wingetrs", "scoop:main", false),
            irow("gamma", "scoop", "scoop:main", false),
        ],
    );
    a.selected = 0;
    space(&mut a);
    a.selected = 1;
    space(&mut a);
    let cmds = update(&mut a, key(KeyCode::Char('u')));
    assert_eq!(
        cmds,
        vec![Cmd::PlanBatch {
            items: vec![plan_remove_cmd("alpha"), plan_remove_cmd("beta")]
        }]
    );
    update(
        &mut a,
        Msg::BatchPlanned(vec![ready_remove("alpha"), ready_remove("beta")]),
    );
    assert!(confirm_open(&a));
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(cmds, vec![Cmd::Execute(removal("alpha"))]);
    assert!(a.running.as_ref().unwrap().label.contains("alpha"));
    assert_eq!(a.queue.len(), 1);
    assert_eq!(a.queue[0].action, removal("beta"));
    assert_eq!(a.marked_count(), 0);
}

#[test]
fn marks_survive_rescan_and_sort_by_identity() {
    let mut a = on_tab(
        Tab::Installed,
        vec![
            irow("alpha", "wingetrs", "scoop:main", false),
            irow("beta", "wingetrs", "bucket-a", false),
        ],
    );
    a.selected = 1;
    space(&mut a);
    assert!(a.is_marked(&row_key("wingetrs", "beta")));
    // beta moves to another position in the new scan.
    update(
        &mut a,
        Msg::InstalledLoaded(vec![
            irow("aaa", "wingetrs", "scoop:main", false),
            irow("alpha", "wingetrs", "scoop:main", false),
            irow("beta", "wingetrs", "bucket-a", false),
        ]),
    );
    assert_eq!(a.marked_count(), 1);
    assert!(a.is_marked(&row_key("wingetrs", "beta")));
    // The order by source puts beta first; the mark follows it.
    update(&mut a, key(KeyCode::Char('o')));
    a.selected = 0;
    let cmds = update(&mut a, key(KeyCode::Char('u')));
    assert_eq!(
        cmds,
        vec![Cmd::PlanBatch {
            items: vec![plan_remove_cmd("beta")]
        }]
    );
    // A row that vanished loses its mark.
    update(
        &mut a,
        Msg::InstalledLoaded(vec![irow("alpha", "wingetrs", "scoop:main", false)]),
    );
    assert_eq!(a.marked_count(), 0);
}

#[test]
fn hidden_marks_are_not_acted_on() {
    let scoop = |name: &str| entry(SourceKind::Scoop, name, name);
    let mut a = App::new(
        vec![scoop("alpha"), scoop("beta"), scoop("alpine")],
        GlyphSet::Nerd,
    );
    for name in ["alpha", "beta"] {
        a.selected = a
            .hits
            .iter()
            .position(|h| a.search.entries()[h.index].name == name)
            .unwrap();
        space(&mut a);
    }
    assert_eq!(a.marked_count(), 2);
    for c in "alp".chars() {
        step(&mut a, key(KeyCode::Char(c)));
    }
    assert_eq!(a.visible_len(), 2, "alpha and alpine");
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert_eq!(
        cmds,
        vec![Cmd::PlanBatch {
            items: vec![Cmd::PlanInstall {
                source_name: "scoop:main".into(),
                id: "alpha".into(),
                keep: false
            }]
        }]
    );
    update(&mut a, Msg::BatchPlanned(vec![ready_apply("alpha")]));
    assert!(has_toast(&a, "1 marcados fora da vista"), "{:?}", a.toasts);
}

#[test]
fn marked_winget_hits_are_skipped_and_named() {
    let mut a = app();
    mark_all(&mut a);
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(
        matches!(cmds.as_slice(), [Cmd::PlanBatch { items }] if items.len() == 1),
        "{cmds:?}"
    );
    update(&mut a, Msg::BatchPlanned(vec![ready_apply("ripgrep")]));
    assert!(has_toast(&a, "pulados: "), "{:?}", a.toasts);
    assert!(has_toast(&a, "PowerToys"), "{:?}", a.toasts);
}

#[test]
fn ambiguous_and_failed_batch_items_are_reported() {
    let mut a = on_tab(
        Tab::Updates,
        vec![
            irow("alpha", "wingetrs", "scoop:main", true),
            irow("beta", "wingetrs", "scoop:main", true),
            irow("gamma", "wingetrs", "scoop:main", true),
        ],
    );
    mark_all(&mut a);
    update(&mut a, key(KeyCode::Char('U')));
    update(
        &mut a,
        Msg::BatchPlanned(vec![
            Ok(Planned::Choose(vec![])),
            Err("falhou".into()),
            ready_apply("gamma"),
        ]),
    );
    assert!(has_toast(&a, "ambíguo, escolha individualmente: alpha"));
    assert!(has_toast(&a, "beta: falhou"));
    assert!(confirm_open(&a));
}

#[test]
fn esc_clears_marks_before_query() {
    let mut a = app();
    space(&mut a);
    step(&mut a, key(KeyCode::Char('p')));
    assert_eq!(a.marked_count(), 1);
    // An error toast is closed first, the marks stay.
    a.push_toast(wingetrs::tui::app::ToastKind::Error, "x");
    step(&mut a, key(KeyCode::Esc));
    assert_eq!(a.marked_count(), 1);
    assert!(a.toasts.is_empty());
    step(&mut a, key(KeyCode::Esc));
    assert_eq!(a.marked_count(), 0);
    assert_eq!(a.query, "p");
    step(&mut a, key(KeyCode::Esc));
    assert_eq!(a.query, "");
    assert_eq!(update(&mut a, key(KeyCode::Esc)), vec![Cmd::Quit]);
}

#[test]
fn space_types_into_nonempty_query() {
    let mut a = app();
    for c in "visual".chars() {
        step(&mut a, key(KeyCode::Char(c)));
    }
    step(&mut a, key(KeyCode::Char(' ')));
    for c in "studio".chars() {
        step(&mut a, key(KeyCode::Char(c)));
    }
    assert_eq!(a.query, "visual studio");
    assert_eq!(a.marked_count(), 0);
}

#[test]
fn insert_marks_with_query() {
    let mut a = app();
    step(&mut a, key(KeyCode::Char('z')));
    assert_eq!(a.visible_len(), 1);
    step(&mut a, key(KeyCode::Insert));
    assert_eq!(a.marked_count(), 1);
    assert_eq!(a.query, "z");
    step(&mut a, key(KeyCode::Insert));
    assert_eq!(a.marked_count(), 0);
}

#[test]
fn ctrl_space_marks_with_query() {
    let mut a = app();
    step(&mut a, key(KeyCode::Char('z')));
    step(&mut a, ctrl(' '));
    assert_eq!(a.marked_count(), 1);
    assert_eq!(a.query, "z");
}

#[test]
fn pinned_app_is_not_upgraded() {
    let mut a = on_tab(
        Tab::Updates,
        vec![irow("beta", "wingetrs", "scoop:main", true)],
    );
    a.pins.insert(wingetrs::pins::key("wingetrs", "beta"));
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
    assert!(
        a.toasts
            .iter()
            .any(|t| t.text == "versão fixada (desafixe no menu)"),
        "{:?}",
        a.toasts
    );
    // In the menu, "Atualizar" is disabled with the reason.
    let mut m = on_tab(
        Tab::Installed,
        vec![irow("beta", "wingetrs", "scoop:main", true)],
    );
    m.pins.insert(wingetrs::pins::key("wingetrs", "beta"));
    update(&mut m, key(KeyCode::Enter));
    let items = menu_items(&m);
    assert_eq!(items[0].0, "Atualizar");
    assert!(!items[0].1);
    assert_eq!(items[0].2.as_deref(), Some("versão fixada"));
    assert_eq!(items[2].0, "Desafixar");
}

#[test]
fn pin_from_menu_applies_and_reverts_on_error() {
    let mut a = on_tab(
        Tab::Installed,
        vec![irow("beta", "wingetrs", "scoop:main", false)],
    );
    update(&mut a, key(KeyCode::Enter));
    assert_eq!(menu_items(&a)[2].0, "Fixar versão");
    update(&mut a, key(KeyCode::Down));
    update(&mut a, key(KeyCode::Down));
    let cmds = update(&mut a, key(KeyCode::Enter));
    let k = wingetrs::pins::key("wingetrs", "beta");
    assert_eq!(
        cmds,
        vec![Cmd::SetPin {
            key: k.clone(),
            pinned: true
        }]
    );
    assert!(a.pins.contains(&k));
    update(
        &mut a,
        Msg::PinSaved {
            key: k.clone(),
            pinned: true,
            result: Err("disco cheio".into()),
        },
    );
    assert!(!a.pins.contains(&k));
    assert!(
        a.toasts
            .iter()
            .any(|t| t.kind == ToastKind::Error && t.text.contains("disco cheio")),
        "{:?}",
        a.toasts
    );
    update(
        &mut a,
        Msg::PinSaved {
            key: k,
            pinned: true,
            result: Ok(()),
        },
    );
    assert!(a.toasts.iter().any(|t| t.text == "versão fixada: beta"));
}

#[test]
fn app_created_with_pins_blocks_the_first_upgrade() {
    let mut a = on_tab(
        Tab::Updates,
        vec![irow("beta", "wingetrs", "scoop:main", true)],
    );
    a.pins = [wingetrs::pins::key("wingetrs", "beta")].into();
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
}

fn override_row(
    correlation: wingetrs::installed::correlate::Correlation,
) -> wingetrs::installed::correlate::ListRow {
    use wingetrs::installed::correlate::UpdateState;
    let mut r = irow("Tool", "arp", "winget", false);
    r.app_key = "arp/{TOOL-GUID}".into();
    r.correlation = correlation;
    r.state = UpdateState::Unknown;
    r
}

#[test]
fn ambiguous_menu_offers_choice_and_saves() {
    use wingetrs::installed::correlate::{Correlation, Rule};
    use wingetrs::installed::overrides::Choice;
    let mut a = on_tab(
        Tab::Installed,
        vec![override_row(Correlation::Ambiguous {
            source: "winget".into(),
            candidates: vec!["A.A".into(), "B.B".into()],
            rule: Rule::ProductCode,
        })],
    );
    update(&mut a, key(KeyCode::Enter));
    assert_eq!(menu_items(&a)[0].0, "Escolher correspondência");
    update(&mut a, key(KeyCode::Enter));
    match &a.modal {
        Some(Modal::Choosing { picker, .. }) => {
            let labels: Vec<&str> = picker.items().iter().map(|i| i.label.as_str()).collect();
            assert_eq!(
                labels,
                ["A.A", "B.B", "Nenhum destes (não está no catálogo)"]
            );
        }
        _ => panic!("picker not open"),
    }
    assert_eq!(
        update(&mut a, key(KeyCode::Enter)),
        vec![Cmd::SetOverride {
            app_key: "arp/{TOOL-GUID}".into(),
            choice: Some(Choice::Package {
                source: "winget".into(),
                package_id: "A.A".into()
            }),
        }]
    );
}

#[test]
fn choosing_none_of_these_saves_not_in_catalog() {
    use wingetrs::installed::correlate::{Correlation, Rule};
    use wingetrs::installed::overrides::Choice;
    let mut a = on_tab(
        Tab::Installed,
        vec![override_row(Correlation::Ambiguous {
            source: "winget".into(),
            candidates: vec!["A.A".into(), "B.B".into()],
            rule: Rule::ProductCode,
        })],
    );
    update(&mut a, key(KeyCode::Enter));
    update(&mut a, key(KeyCode::Enter));
    update(&mut a, key(KeyCode::Down));
    update(&mut a, key(KeyCode::Down));
    assert_eq!(
        update(&mut a, key(KeyCode::Enter)),
        vec![Cmd::SetOverride {
            app_key: "arp/{TOOL-GUID}".into(),
            choice: Some(Choice::NotInCatalog),
        }]
    );
}

#[test]
fn undo_choice_in_menu() {
    use wingetrs::installed::correlate::{Correlation, Rule};
    let mut a = on_tab(
        Tab::Installed,
        vec![override_row(Correlation::Matched {
            source: "winget".into(),
            package_id: "A.A".into(),
            rule: Rule::User,
        })],
    );
    update(&mut a, key(KeyCode::Enter));
    let items = menu_items(&a);
    assert_eq!(items.len(), 4);
    assert_eq!(items[3].0, "Desfazer escolha de correspondência");
    for _ in 0..3 {
        update(&mut a, key(KeyCode::Down));
    }
    assert_eq!(
        update(&mut a, key(KeyCode::Enter)),
        vec![Cmd::SetOverride {
            app_key: "arp/{TOOL-GUID}".into(),
            choice: None,
        }]
    );
}

#[test]
fn override_saved_toasts_and_rescans() {
    use wingetrs::installed::overrides::Choice;
    let mut a = app();
    let cmds = update(
        &mut a,
        Msg::OverrideSaved {
            choice: Some(Choice::Package {
                source: "winget".into(),
                package_id: "A.A".into(),
            }),
            result: Ok(()),
        },
    );
    assert_eq!(cmds, vec![Cmd::ScanInstalled]);
    assert!(
        a.toasts
            .iter()
            .any(|t| t.text == "correspondência salva: A.A")
    );
}

#[test]
fn scoop_rows_are_prefetched() {
    let mut a = ten_hits();
    a.icon_protocol = wingetrs::tui::icons::IconProtocol::Sixel;
    let got = prefetches(&update(&mut a, Msg::Resize(100, 40)));
    assert_eq!(got.len(), 1, "{got:?}");
    let scoop = a.list_keys(0..10).into_iter().flatten().collect::<Vec<_>>();
    assert_eq!(scoop.len(), 10);
    assert!(
        scoop.iter().all(|(s, _)| s.starts_with("scoop")),
        "{scoop:?}"
    );
    assert_eq!(got[0].1, scoop);
}
