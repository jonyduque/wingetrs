#![allow(clippy::unwrap_used, clippy::expect_used)]
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use wingetrs::core::{PackageEntry, SourceKind};
use wingetrs::tui::app::{App, Msg, update};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Theme};
use wingetrs::tui::view::view;

fn entries() -> Vec<PackageEntry> {
    let e = |s: SourceKind, id: &str, name: &str, v: &str| PackageEntry {
        source: s,
        source_name: if s == SourceKind::Winget {
            "winget".into()
        } else {
            "scoop:main".into()
        },
        id: id.into(),
        name: name.into(),
        moniker: None,
        version: v.into(),
        publisher: None,
        description: Some("Utilitários para power users".into()),
        tags: vec![],
        commands: vec![],
    };
    vec![
        e(
            SourceKind::Winget,
            "Microsoft.PowerToys",
            "PowerToys",
            "0.101.2362.0",
        ),
        e(
            SourceKind::Winget,
            "lin-ycv.EverythingPowerToys",
            "EverythingPowerToys",
            "0.91.1",
        ),
        e(SourceKind::Scoop, "powertoys", "powertoys", "0.101.2"),
    ]
}

fn render(app: &App, w: u16, h: u16) -> String {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| view(app, &theme, f)).unwrap();
    format!("{}", t.backend())
}

fn typed(q: &str) -> App {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    for c in q.chars() {
        update(
            &mut a,
            Msg::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            )),
        );
    }
    a
}

#[test]
fn search_screen_snapshot() {
    insta::assert_snapshot!(render(&typed("powert"), 100, 16));
}

#[test]
fn empty_catalog_snapshot() {
    insta::assert_snapshot!(render(&App::new(vec![], GlyphSet::Ascii), 80, 12));
}

#[test]
fn offline_snapshot() {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.offline = true;
    a.index_age_secs = Some(3 * 3600);
    insta::assert_snapshot!(render(&a, 100, 12));
}

#[test]
fn tiny_terminals_never_panic() {
    let a = App::new(entries(), GlyphSet::Nerd);
    for (w, h) in [(1, 1), (10, 3), (20, 5), (59, 7), (60, 8), (200, 60)] {
        let _ = render(&a, w, h);
    }
}

#[test]
fn narrow_terminal_hides_details_panel() {
    let a = App::new(entries(), GlyphSet::Ascii);
    let s = render(&a, 50, 12);
    assert!(
        !s.contains("Utilitários"),
        "details panel must be hidden below 60 columns:\n{s}"
    );
    let s = render(&a, 100, 12);
    assert!(
        s.contains("Utilitários"),
        "details panel visible at 100 columns:\n{s}"
    );
}

#[test]
fn matched_letters_are_underlined() {
    let a = typed("powert");
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut t = Terminal::new(TestBackend::new(100, 16)).unwrap();
    t.draw(|f| view(&a, &theme, f)).unwrap();
    let buf = t.backend().buffer();
    // First list row containing "PowerToys" (leftmost occurrence on the first such row).
    let mut found = None;
    'rows: for y in 0..buf.area.height {
        let cells: Vec<char> = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect();
        let needle: Vec<char> = "PowerToys".chars().collect();
        if let Some(x) = cells.windows(needle.len()).position(|w| w == needle) {
            found = Some((x as u16, y));
            break 'rows;
        }
    }
    let (x, y) = found.expect("PowerToys row in list");
    let cell = &buf[(x, y)];
    assert_eq!(cell.symbol(), "P");
    assert!(
        cell.modifier.contains(Modifier::UNDERLINED),
        "first matched letter must be underlined"
    );
    // A non-matched letter on the same row ("s" at the end) is not underlined.
    let tail = &buf[(x + 8, y)];
    assert_eq!(tail.symbol(), "s");
    assert!(!tail.modifier.contains(Modifier::UNDERLINED));
}
