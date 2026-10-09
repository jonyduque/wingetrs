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
