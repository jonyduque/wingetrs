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
