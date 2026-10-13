#![allow(clippy::unwrap_used, clippy::expect_used)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use wingetrs::core::{PackageEntry, SourceKind};
use wingetrs::tui::app::{App, Cmd, Focus, Msg, SourceFilter, Tab, update};
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
    update(&mut a, key(KeyCode::Enter));
    assert_eq!(a.focus, Focus::Details);
    assert!(update(&mut a, key(KeyCode::Esc)).is_empty());
    assert_eq!(a.focus, Focus::List);
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
    assert!(matches!(a.modal, Some(Modal::Running { .. })));
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
fn running_swallows_keys_and_shows_progress() {
    let mut a = app();
    update(&mut a, ready("ripgrep"));
    update(&mut a, key(KeyCode::Enter));
    assert!(update(&mut a, key(KeyCode::Esc)).is_empty());
    assert!(update(&mut a, key(KeyCode::Char('q'))).is_empty());
    assert!(update(&mut a, ctrl('c')).is_empty(), "no quit mid-install");
    assert!(matches!(a.modal, Some(Modal::Running { .. })));
    update(
        &mut a,
        Msg::InstallProgress {
            label: "ripgrep.zip".into(),
            done: 5,
            total: Some(10),
        },
    );
    let Some(Modal::Running { progress, .. }) = &a.modal else {
        panic!("not running");
    };
    assert_eq!(progress.as_ref().unwrap().done, 5);
}

#[test]
fn busy_shows_status_and_rescans() {
    let mut a = app();
    update(&mut a, ready("ripgrep"));
    update(&mut a, key(KeyCode::Enter));
    let cmds = update(&mut a, Msg::InstallFinished(Err(InstallFail::Busy)));
    assert_eq!(cmds, vec![Cmd::ScanInstalled]);
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
    assert!(matches!(a.modal, Some(Modal::Running { .. })));
    update(&mut a, Msg::ResiduesCleaned(vec![]));
    assert!(a.modal.is_none());
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
fn arrows_switch_tabs_with_text_and_from_details() {
    let mut a = app();
    update(&mut a, key(KeyCode::Char('p')));
    update(&mut a, key(KeyCode::Char('o')));
    update(&mut a, key(KeyCode::Right));
    assert_eq!(a.tab, Tab::Installed);
    assert_eq!(a.query, "po");
    let mut b = app();
    update(&mut b, key(KeyCode::Enter));
    assert_eq!(b.focus, Focus::Details);
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
                let expected: Vec<&str> = rows
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
    for _ in 0..4 {
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
    for _ in 0..5 {
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
    for _ in 0..4 {
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
    assert!(matches!(a.modal, Some(Modal::Running { .. })));
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
    assert!(matches!(a.modal, Some(Modal::Running { .. })));
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
fn second_upgrade_while_running_does_nothing() {
    let mut a = on_tab(
        Tab::Updates,
        vec![irow("a", "wingetrs", "scoop:main", true)],
    );
    a.settings.behavior.confirm = ConfirmMode::Never;
    assert_eq!(update(&mut a, key(KeyCode::Char('U'))).len(), 1);
    update(&mut a, ready("a"));
    assert!(matches!(a.modal, Some(Modal::Running { .. })));
    assert!(update(&mut a, key(KeyCode::Char('U'))).is_empty());
    assert!(matches!(a.modal, Some(Modal::Running { .. })));
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
            key: k.clone(),
            size: (8, 4),
            proto: Ok(tiny_icon()),
        },
    );
    for i in 0..ICON_CACHE_MAX {
        update(
            &mut a,
            Msg::IconReady {
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
fn list_never_fetches_icons() {
    // Visible rows without a cached icon produce no request of any kind.
    let mut a = icon_app(50, 10);
    let mut cmds = update(&mut a, Msg::IconMapLoaded(Default::default()));
    cmds.extend(
        update(&mut a, key(KeyCode::Down))
            .into_iter()
            .filter(|c| !matches!(c, Cmd::LoadDetails { .. })),
    );
    cmds.extend(update(&mut a, Msg::Resize(100, 60)));
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
