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
fn app() -> App {
    App::new(
        vec![
            entry(SourceKind::Winget, "Microsoft.PowerToys", "PowerToys"),
            entry(SourceKind::Scoop, "ripgrep", "ripgrep"),
            entry(SourceKind::Winget, "Zeal.Zeal", "Zeal"),
        ],
        GlyphSet::Nerd,
    )
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
        update(&mut a, key(KeyCode::Char(c)));
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
    update(&mut a, key(KeyCode::Char('x')));
    assert!(update(&mut a, key(KeyCode::Esc)).is_empty());
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
    assert_eq!(a.tab, Tab::Search);
    update(&mut a, key(KeyCode::BackTab));
    assert_eq!(a.tab, Tab::Updates);
    let mut b = app();
    update(&mut b, ctrl('s'));
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
    let status = a.status.unwrap();
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
    assert!(a.status.is_some());
    update(&mut a, Msg::RefreshFinished(failed_report(true)));
    assert!(a.offline);
}

#[test]
fn installed_failure_shows_status() {
    let mut a = app();
    update(&mut a, Msg::InstalledFailed("pânico interno".into()));
    let status = a.status.unwrap();
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
            id: "ripgrep".into()
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
    assert!(a.status.as_deref().unwrap().contains("fase 2d"));
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
    assert_eq!(a.status.as_deref(), Some("outra instalação em andamento"));
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
    assert_eq!(a.status.as_deref(), Some("Instalado: ripgrep"));
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
            id: "tool".into()
        }]
    );
    assert!(a.modal.is_none());
}

#[test]
fn planning_error_becomes_status() {
    let mut a = app();
    update(&mut a, Msg::Planned(Err("não suportado: x".into())));
    assert!(a.modal.is_none());
    assert_eq!(a.status.as_deref(), Some("não suportado: x"));
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
    assert!(a.status.is_some());
    a.selected = 1;
    assert_eq!(
        update(&mut a, key(KeyCode::Char('u'))),
        vec![Cmd::PlanRemove { app: "beta".into() }]
    );
    let removal = UninstallPlan {
        app: "beta".into(),
        version: "1".into(),
        purge: false,
        artifacts: vec![],
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
            id: "beta".into()
        }]
    );
    let mut empty = on_tab(Tab::Updates, vec![]);
    assert_eq!(
        update(&mut empty, key(KeyCode::Char('U'))),
        vec![Cmd::PlanUpgradeAll]
    );
    let mut foreign = on_tab(Tab::Updates, vec![irow("gamma", "arp", "winget", true)]);
    assert!(update(&mut foreign, key(KeyCode::Char('U'))).is_empty());
    assert!(foreign.status.is_some());
}
