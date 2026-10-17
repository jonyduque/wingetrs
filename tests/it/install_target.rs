#![allow(clippy::unwrap_used, clippy::expect_used)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::core::SourceKind;
use wingetrs::install::target::{Candidate, TargetError, resolve};
use wingetrs::tui::picker::{PickOutcome, Picker, PickerItem, window};

fn item(id: &str, selectable: bool) -> PickerItem {
    PickerItem {
        label: id.into(),
        detail: String::new(),
        selectable,
        note: None,
    }
}
fn k(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

#[test]
fn arrows_skip_unselectable_and_enter_picks() {
    let mut p = Picker::single(vec![
        item("a", true),
        item("winget", false),
        item("b", true),
    ]);
    assert_eq!(p.selected(), 0);
    assert_eq!(p.key(k(KeyCode::Down)), PickOutcome::Pending);
    assert_eq!(p.selected(), 2, "skips the winget row");
    assert_eq!(p.key(k(KeyCode::Down)), PickOutcome::Pending);
    assert_eq!(p.selected(), 2, "stays at the end");
    assert_eq!(p.key(k(KeyCode::Up)), PickOutcome::Pending);
    assert_eq!(p.selected(), 0);
    assert_eq!(p.key(k(KeyCode::Enter)), PickOutcome::Picked(vec![0]));
}

#[test]
fn esc_cancels_and_multi_mode_toggles_with_space() {
    let mut p = Picker::single(vec![item("a", true)]);
    assert_eq!(p.key(k(KeyCode::Esc)), PickOutcome::Cancelled);
    let mut m = Picker::multi(
        vec![item("a", true), item("b", true), item("c", true)],
        &[true, false, false],
    );
    m.key(k(KeyCode::Down));
    m.key(k(KeyCode::Char(' ')));
    m.key(k(KeyCode::Up));
    m.key(k(KeyCode::Char(' ')));
    assert_eq!(m.key(k(KeyCode::Enter)), PickOutcome::Picked(vec![1]));
}

#[test]
fn home_end_and_leading_unselectable() {
    let mut p = Picker::single(vec![
        item("w", false),
        item("a", true),
        item("b", true),
        item("w2", false),
    ]);
    assert_eq!(p.selected(), 1, "starts on the first selectable row");
    p.key(k(KeyCode::End));
    assert_eq!(p.selected(), 2);
    p.key(k(KeyCode::Home));
    assert_eq!(p.selected(), 1);
}

#[test]
fn only_unselectable_items_never_pick() {
    let mut p = Picker::single(vec![item("w", false)]);
    assert_eq!(p.key(k(KeyCode::Enter)), PickOutcome::Pending);
}

#[test]
fn window_keeps_selected_visible() {
    assert_eq!(window(0, 5, 10), 0..5);
    assert_eq!(window(0, 30, 10), 0..10);
    assert_eq!(window(29, 30, 10), 20..30);
    let w = window(15, 30, 10);
    assert!(w.contains(&15) && w.len() == 10);
}

fn pkg(id: &str, name: &str, moniker: Option<&str>) -> NewPackage {
    NewPackage {
        id: id.into(),
        name: name.into(),
        moniker: moniker.map(Into::into),
        version: "1".into(),
        ..Default::default()
    }
}

fn catalog() -> Catalog {
    let mut c = Catalog::open_in_memory().unwrap();
    let m = WingetMaps::default();
    let add = |c: &mut Catalog, src: &str, kind, pk: &[NewPackage]| {
        c.replace_source(src, kind, "v", 1, pk, &m).unwrap();
    };
    add(
        &mut c,
        "scoop:main",
        SourceKind::Scoop,
        &[
            pkg("ripgrep", "ripgrep", None),
            pkg("vscode", "vscode", None),
            pkg("ripgrip", "ripgrip", None),
            pkg("ripgrap", "ripgrap", None),
        ],
    );
    add(
        &mut c,
        "scoop:extras",
        SourceKind::Scoop,
        &[
            pkg("vscode", "vscode", None),
            pkg("zed", "Zed Editor", Some("zed-ed")),
        ],
    );
    add(
        &mut c,
        "winget",
        SourceKind::Winget,
        &[pkg("Microsoft.Only", "Only In Winget", None)],
    );
    c
}

fn sources(v: &[Candidate]) -> Vec<String> {
    v.iter().map(|c| c.entry.source_name.clone()).collect()
}

#[test]
fn exact_id_and_case_insensitive_and_bucket_prefix() {
    let c = catalog();
    let r = resolve(&c, "ripgrep", None).unwrap();
    assert_eq!(sources(&r), ["scoop:main"]);
    assert!(r[0].selectable);
    let r = resolve(&c, "RIPGREP", None).unwrap();
    assert_eq!(r[0].entry.id, "ripgrep");
    let r = resolve(&c, "main/ripgrep", None).unwrap();
    assert_eq!(sources(&r), ["scoop:main"]);
}

#[test]
fn bucket_flag_restricts_and_missing_is_not_found() {
    let c = catalog();
    let e = resolve(&c, "ripgrep", Some("extras")).unwrap_err();
    assert!(matches!(e, TargetError::NotFound { .. }));
    let r = resolve(&c, "vscode", Some("extras")).unwrap();
    assert_eq!(sources(&r), ["scoop:extras"]);
}

#[test]
fn same_name_in_two_buckets_gives_two_candidates() {
    let c = catalog();
    let r = resolve(&c, "vscode", None).unwrap();
    assert_eq!(r.len(), 2);
    assert!(r.iter().all(|x| x.selectable));
}

#[test]
fn moniker_and_name_match_exactly() {
    let c = catalog();
    assert_eq!(resolve(&c, "zed-ed", None).unwrap()[0].entry.id, "zed");
    assert_eq!(resolve(&c, "zed editor", None).unwrap()[0].entry.id, "zed");
}

#[test]
fn winget_only_candidate_is_not_selectable() {
    let c = catalog();
    let r = resolve(&c, "microsoft.only", None).unwrap();
    assert_eq!(r.len(), 1);
    assert!(!r[0].selectable);
    assert_eq!(r[0].note.as_deref(), Some("instalação na fase 2d"));
}

#[test]
fn unknown_term_gives_suggestions_never_a_candidate() {
    let c = catalog();
    let e = resolve(&c, "ripgrop", None).unwrap_err();
    match e {
        TargetError::NotFound { suggestions, .. } => {
            assert!(!suggestions.is_empty() && suggestions.len() <= 5);
            assert!(suggestions.iter().any(|s| s.contains("ripgrep")));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn ctrl_c_cancels_but_plain_c_does_not() {
    let mut p = Picker::single(vec![item("a", true)]);
    assert_eq!(p.key(k(KeyCode::Char('c'))), PickOutcome::Pending);
    let ev = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(p.key(ev), PickOutcome::Cancelled);
}

#[test]
fn blank_query_is_not_found_without_suggestions() {
    let c = catalog();
    match resolve(&c, "   ", None).unwrap_err() {
        TargetError::NotFound { suggestions, .. } => assert!(suggestions.is_empty()),
        other => panic!("unexpected: {other:?}"),
    }
}
