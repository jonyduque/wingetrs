#![allow(clippy::unwrap_used, clippy::expect_used)]
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use wingetrs::core::{PackageEntry, SourceKind};
use wingetrs::tui::app::{App, Cmd, Msg, run_search, update};
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
        let cmds = update(
            &mut a,
            Msg::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            )),
        );
        // Run the searches the keystroke asked for, as the event loop does.
        for cmd in cmds {
            if let Cmd::Search {
                generation,
                query,
                kind,
            } = cmd
            {
                let hits = run_search(&a.search, &query, kind);
                update(&mut a, Msg::SearchDone { generation, hits });
            }
        }
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
        !s.contains("versão"),
        "details panel must be hidden below 60 columns:\n{s}"
    );
    let s = render(&a, 100, 12);
    assert!(
        s.contains("versão"),
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
// ---- modals (Task 39) ----

use std::path::PathBuf;
use wingetrs::install::plan::InstallPlan;
use wingetrs::install::residue::{Confidence, Residue, ResidueKind};
use wingetrs::install::target::Candidate;
use wingetrs::tui::app::{Action, Done, Planned};

fn plan() -> InstallPlan {
    InstallPlan {
        app: "powertoys".into(),
        bucket: "main".into(),
        version: "0.101.2".into(),
        arch: None,
        upgrade_from: None,
        root: PathBuf::from("C:/w"),
        manifest_sha256: "x".into(),
        steps: vec![],
    }
}

fn press(a: &mut App, code: crossterm::event::KeyCode) {
    update(
        a,
        Msg::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        )),
    );
}

fn with_confirm() -> App {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    update(
        &mut a,
        Msg::Planned(Ok(Planned::Ready {
            text: "Instalar powertoys 0.101.2 (bucket main)\n  1. Baixar powertoys.zip\n  2. Extrair em apps/powertoys/0.101.2".into(),
            action: Action::Apply(vec![vec![plan()]]),
        })),
    );
    a
}

fn with_picking() -> App {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    let cand = |bucket: &str| {
        let mut entry = entries().remove(2);
        entry.source_name = format!("scoop:{bucket}");
        Candidate {
            entry,
            selectable: true,
            note: None,
        }
    };
    update(
        &mut a,
        Msg::Planned(Ok(Planned::Choose(vec![cand("extras"), cand("main")]))),
    );
    a
}

fn with_running() -> App {
    let mut a = with_confirm();
    press(&mut a, crossterm::event::KeyCode::Enter);
    update(
        &mut a,
        Msg::InstallProgress {
            label: "powertoys.zip".into(),
            done: 50,
            total: Some(100),
        },
    );
    a
}

fn with_residues() -> App {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    let r = |p: &str, pre: bool| Residue {
        kind: ResidueKind::Folder { path: p.into() },
        confidence: if pre {
            Confidence::High
        } else {
            Confidence::Low
        },
        preselected: pre,
        note: (!pre).then(|| "pode ser de outra instalação".to_string()),
    };
    update(
        &mut a,
        Msg::InstallFinished(Ok(Done {
            summary: "powertoys removido".into(),
            residues: Some((
                "powertoys".into(),
                vec![
                    r("C:/Users/x/AppData/Roaming/powertoys", true),
                    r("C:/Users/x/AppData/Local/powertoys", false),
                ],
            )),
        })),
    );
    a
}

#[test]
fn confirm_modal_snapshot() {
    insta::assert_snapshot!(render(&with_confirm(), 100, 16));
}

#[test]
fn picking_modal_snapshot() {
    insta::assert_snapshot!(render(&with_picking(), 100, 16));
}

#[test]
fn running_modal_snapshot() {
    insta::assert_snapshot!(render(&with_running(), 100, 16));
}

#[test]
fn residues_modal_snapshot() {
    insta::assert_snapshot!(render(&with_residues(), 100, 16));
}

#[test]
fn modals_never_panic_on_tiny_terminals() {
    for build in [with_confirm, with_picking, with_running, with_residues] {
        let a = build();
        assert!(a.modal.is_some());
        for (w, h) in [(1, 1), (20, 5), (10, 3), (59, 7), (60, 8), (200, 60)] {
            let _ = render(&a, w, h);
        }
    }
}

#[test]
fn closed_modal_leaves_the_screen_alone() {
    let mut a = with_confirm();
    press(&mut a, crossterm::event::KeyCode::Esc);
    assert!(a.modal.is_none());
    assert!(!render(&a, 100, 16).contains("Confirmar"));
}

#[test]
fn selected_row_moves_on_screen_while_details_load() {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    for _ in 0..2 {
        update(
            &mut a,
            Msg::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Down,
                crossterm::event::KeyModifiers::NONE,
            )),
        );
    }
    let screen = render(&a, 100, 16);
    let selected = screen
        .lines()
        .map(|l| l.trim_start_matches('"'))
        .find(|l| l.starts_with("│ | "))
        .expect("a selected row");
    assert!(selected.contains("powertoys"), "{selected}");
    assert!(screen.contains("carregando"), "{screen}");
}

#[test]
fn list_window_keeps_selection_visible() {
    use wingetrs::tui::view::list_window;
    assert_eq!(list_window(0, 0, 1500, 10), 0..10);
    assert_eq!(list_window(700, 0, 1500, 10), 691..701);
    assert_eq!(list_window(1499, 1490, 1500, 10), 1490..1500);
    assert_eq!(list_window(5, 3, 1500, 10), 3..13);
    assert_eq!(list_window(0, 0, 3, 10), 0..3);
    assert_eq!(list_window(0, 0, 0, 10), 0..0);
}

#[test]
fn long_list_end_is_rendered() {
    let es: Vec<PackageEntry> = (0..1500)
        .map(|i| {
            let n = format!("pkg{i:04}");
            PackageEntry {
                source: SourceKind::Winget,
                source_name: "winget".into(),
                id: n.clone(),
                name: n,
                moniker: None,
                version: "1".into(),
                publisher: None,
                description: None,
                tags: vec![],
                commands: vec![],
            }
        })
        .collect();
    // The search keeps at most MAX_HITS (1000) hits, so the last row is pkg0999.
    let mut a = App::new(es, GlyphSet::Ascii);
    update(
        &mut a,
        Msg::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::End,
            crossterm::event::KeyModifiers::NONE,
        )),
    );
    let out = render(&a, 100, 20);
    let line = out
        .lines()
        .map(|l| l.trim_start_matches('"'))
        .find(|l| l.starts_with("│ | "))
        .unwrap();
    assert!(line.contains("pkg0999"), "{line}");
    assert!(!out.contains("pkg0000"));
}

#[test]
fn installed_details_show_the_catalog_publisher() {
    use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
    use wingetrs::tui::app::Tab;
    let mut a = App::new(vec![], GlyphSet::Ascii);
    let mut e = entries().remove(0);
    e.id = "Acme.Tool".into();
    e.name = "Tool".into();
    e.publisher = Some("Acme Corp".into());
    update(&mut a, Msg::CatalogLoaded(vec![e]));
    update(
        &mut a,
        Msg::InstalledLoaded(vec![ListRow {
            name: "Tool".into(),
            origin: "arp".into(),
            installed_version: Some("1".into()),
            correlation: Correlation::Matched {
                source: "winget".into(),
                package_id: "Acme.Tool".into(),
                rule: Rule::ScoopManifest,
            },
            latest: None,
            state: UpdateState::UpToDate,
            tiebreak: None,
        }]),
    );
    update(
        &mut a,
        Msg::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        )),
    );
    assert_eq!(a.tab, Tab::Installed);
    assert!(render(&a, 120, 30).contains("Acme Corp"));
}

#[test]
fn scrolled_selection_highlights_the_selected_row() {
    let es: Vec<PackageEntry> = (0..1500)
        .map(|i| {
            let n = format!("pkg{i:04}");
            PackageEntry {
                source: SourceKind::Winget,
                source_name: "winget".into(),
                id: n.clone(),
                name: n,
                moniker: None,
                version: "1".into(),
                publisher: None,
                description: None,
                tags: vec![],
                commands: vec![],
            }
        })
        .collect();
    let mut a = App::new(es, GlyphSet::Ascii);
    update(
        &mut a,
        Msg::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::End,
            crossterm::event::KeyModifiers::NONE,
        )),
    );
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let sel_bar = theme.sel_bar().fg.expect("bar color");
    let sel_title = theme.sel_title().fg.expect("title color");
    let mut t = Terminal::new(TestBackend::new(100, 20)).unwrap();
    t.draw(|f| view(&a, &theme, f)).unwrap();
    let buf = t.backend().buffer().clone();
    let mut barred = Vec::new();
    for y in 0..buf.area.height {
        let text: String = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect();
        if text.contains("pkg") && buf[(2, y)].symbol() == "|" {
            assert_eq!(buf[(2, y)].fg, sel_bar, "{text}");
            barred.push((y, text));
        }
    }
    // Title line plus description line of the one selected item, nothing else.
    assert_eq!(barred.len(), 2, "{barred:?}");
    assert!(barred[0].1.contains("pkg0999"), "{barred:?}");
    assert_eq!(barred[1].0, barred[0].0 + 1, "{barred:?}");
    let name_x = (0..buf.area.width)
        .find(|&x| buf[(x, barred[0].0)].symbol() == "p")
        .expect("name cell");
    assert_eq!(buf[(name_x, barred[0].0)].fg, sel_title);
    assert!(buf[(name_x, barred[0].0)].modifier.contains(Modifier::BOLD));
}

fn cell(out: &str, x: usize, y: usize) -> String {
    out.lines()
        .nth(y)
        .map(|l| {
            l.trim_matches('"')
                .chars()
                .nth(x)
                .unwrap_or('?')
                .to_string()
        })
        .unwrap_or_default()
}

#[test]
fn inset_saturates_on_tiny_rects() {
    use ratatui::layout::Rect;
    use wingetrs::tui::layout::inset;
    let tiny = inset(Rect::new(0, 0, 1, 1), 1, 1);
    assert_eq!((tiny.width, tiny.height), (0, 0));
    assert_eq!(inset(Rect::new(2, 3, 10, 5), 1, 1), Rect::new(3, 4, 8, 3));
}

#[test]
fn split_with_gutter_leaves_blank_columns() {
    use ratatui::layout::Rect;
    use wingetrs::tui::layout::split_with_gutter;
    let (left, divider, right) = split_with_gutter(Rect::new(0, 0, 100, 10), 55);
    assert_eq!(left, Rect::new(0, 0, 55, 10));
    assert_eq!(divider, 56);
    assert_eq!(right.x, 58);
    assert_eq!(right.right() - 1, 99);
    assert_eq!(right.width, 42);
}

#[test]
fn list_and_details_text_start_one_column_after_the_border() {
    let out = render(&typed("powert"), 100, 16);
    for y in 5..=7 {
        assert_eq!(cell(&out, 1, y), " ", "{out}");
        let c2 = cell(&out, 2, y);
        assert!(c2 == "|" || c2 == " ", "{out}");
        if y % 2 == 1 {
            // Title lines (rows 5 and 7) carry the source glyph; description lines do not.
            let glyph = cell(&out, 4, y);
            assert!(glyph == "W" || glyph == "S", "{out}");
        }
    }
    assert_eq!(cell(&out, 55, 5), "│", "{out}");
    assert_eq!(cell(&out, 56, 5), " ", "{out}");
    assert_eq!(cell(&out, 57, 5), "P", "{out}");
}

#[test]
fn confirm_dialog_has_inner_padding() {
    let out = render(&with_confirm(), 100, 24);
    let rows: Vec<Vec<char>> = out
        .lines()
        .map(|l| l.trim_matches('"').chars().collect())
        .collect();
    let top = rows
        .iter()
        .position(|r| r.iter().collect::<String>().contains("Confirmar"))
        .expect("dialog title");
    let left = rows[top].iter().position(|c| *c == '╭').expect("corner");
    let right = rows[top].iter().rposition(|c| *c == '╮').expect("corner");
    let inner = |y: usize| -> String { rows[y][left + 1..right].iter().collect() };
    assert!(inner(top + 1).chars().all(|c| c == ' '), "{out}");
    let first_text = inner(top + 2);
    assert!(first_text.starts_with("  Instalar"), "{first_text:?}");
    assert!(inner(top + 3).starts_with("  "), "{out}");
}

#[test]
fn layout_constants_are_the_documented_scale() {
    use wingetrs::tui::layout::*;
    assert_eq!(
        [
            PAD_X,
            FILTER_GAP,
            GUTTER,
            ITEM_GAP,
            DIALOG_PAD_X,
            DIALOG_PAD_Y,
            TOAST_PAD_X,
            TOAST_MARGIN_X,
            TOAST_MARGIN_Y,
            TAB_PAD_X,
            BAR_PAD_X
        ],
        [1, 1, 1, 0, 2, 1, 1, 2, 1, 1, 1]
    );
}

#[test]
fn residue_conflict_warning_survives_narrow_terminals() {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    let long = "C:/Users/someone/AppData/Local/vendor/product/very/long/path/powertoys";
    assert!(long.len() >= 70);
    update(
        &mut a,
        Msg::InstallFinished(Ok(Done {
            summary: "powertoys removido".into(),
            residues: Some((
                "powertoys".into(),
                vec![Residue {
                    kind: ResidueKind::Folder { path: long.into() },
                    confidence: Confidence::Low,
                    preselected: false,
                    note: Some("pode ser de outra instalação".to_string()),
                }],
            )),
        })),
    );
    for w in [100, 80, 60] {
        let out = render(&a, w, 16);
        let line = out
            .lines()
            .find(|l| l.contains("[ ]"))
            .unwrap_or_else(|| panic!("no residue row at {w}: {out}"));
        assert!(line.contains("(outra instalação?)"), "{w}: {line}");
    }
}

// ---- Task 63: tabs joined to the content window ----

fn tab_labels() -> Vec<wingetrs::tui::tabs::TabLabel> {
    ["Pesquisar", "Instalados", "Atualizações 2"]
        .iter()
        .map(|s| wingetrs::tui::tabs::TabLabel {
            text: s.to_string(),
            warn: false,
        })
        .collect()
}

fn tab_buffer(w: u16, active: usize, theme: &Theme) -> ratatui::buffer::Buffer {
    let labels = tab_labels();
    let mut t = Terminal::new(TestBackend::new(w, 3)).unwrap();
    t.draw(|f| {
        wingetrs::tui::tabs::draw_tabs(f, f.area(), &labels, active, theme);
    })
    .unwrap();
    t.backend().buffer().clone()
}

fn buf_row(b: &ratatui::buffer::Buffer, y: u16) -> String {
    (0..b.area.width).map(|x| b[(x, y)].symbol()).collect()
}

#[test]
fn active_first_tab_opens_into_the_window() {
    let b = tab_buffer(60, 0, &Theme::new(true, ColorDepth::NoColor));
    assert_eq!(
        buf_row(&b, 0),
        format!(
            "╭{}╮╭{}╮╭{}╮{}",
            "─".repeat(11),
            "─".repeat(12),
            "─".repeat(16),
            " ".repeat(15)
        )
    );
    assert_eq!(
        buf_row(&b, 1),
        format!(
            "│ Pesquisar ││ Instalados ││ Atualizações 2 │{} wingetrs  ",
            " ".repeat(4)
        )
    );
    assert_eq!(
        buf_row(&b, 2),
        format!(
            "│{}└┴{}┴┴{}┴{}╮",
            " ".repeat(11),
            "─".repeat(12),
            "─".repeat(16),
            "─".repeat(14)
        )
    );
}

#[test]
fn active_middle_tab_tees_the_first_corner() {
    let b = tab_buffer(60, 1, &Theme::new(true, ColorDepth::NoColor));
    assert_eq!(
        buf_row(&b, 2),
        format!(
            "├{}┴┘{}└┴{}┴{}╮",
            "─".repeat(11),
            " ".repeat(12),
            "─".repeat(16),
            "─".repeat(14)
        )
    );
}

#[test]
fn tabs_fall_back_to_one_line_when_narrow() {
    let b = tab_buffer(30, 0, &Theme::new(true, ColorDepth::NoColor));
    assert_eq!(buf_row(&b, 0).trim(), "");
    assert!(buf_row(&b, 1).contains("Pesquisar · Instalados"));
    assert_eq!(buf_row(&b, 2), format!("╭{}╮", "─".repeat(28)));
}

#[test]
fn active_label_is_bold_accent_and_others_dim() {
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let b = tab_buffer(60, 0, &theme);
    let p = &b[(2, 1)];
    assert_eq!(p.symbol(), "P");
    assert_eq!(p.fg, theme.accent().fg.unwrap());
    assert!(p.modifier.contains(Modifier::BOLD));
    let i = &b[(15, 1)];
    assert_eq!(i.symbol(), "I");
    assert_eq!(i.fg, theme.fg_dim().fg.unwrap());
}

#[test]
fn tabs_never_panic_on_tiny_widths() {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let labels = tab_labels();
    for w in 0..=12u16 {
        for h in 0..=3u16 {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| wingetrs::tui::tabs::draw_tabs(f, f.area(), &labels, 1, &theme))
                .unwrap();
        }
    }
}

#[test]
fn window_spacing_matches_the_scale() {
    let screen = render(&typed("powert"), 100, 16);
    let rows: Vec<Vec<char>> = screen
        .lines()
        .map(|l| {
            l.trim_start_matches('"')
                .trim_end_matches('"')
                .chars()
                .collect()
        })
        .collect();
    for (y, r) in rows.iter().enumerate().take(14).skip(3) {
        assert_eq!(r[1], ' ', "PAD_X on row {y}: {screen}");
    }
    let gap = &rows[4];
    assert_eq!(gap[0], '│', "{screen}");
    assert_eq!(gap[gap.len() - 1], '│', "{screen}");
    assert!(
        gap[1..gap.len() - 1].iter().all(|c| *c == ' '),
        "FILTER_GAP row not blank: {screen}"
    );
    let d = rows[5]
        .iter()
        .enumerate()
        .skip(2)
        .find(|(_, c)| **c == '│')
        .map(|(i, _)| i)
        .expect("divider");
    for (y, r) in rows.iter().enumerate().take(14).skip(5) {
        assert_eq!(r[d], '│', "divider row {y}: {screen}");
        assert_eq!(r[d - 1], ' ', "GUTTER left row {y}");
        assert_eq!(r[d + 1], ' ', "GUTTER right row {y}");
    }
}

fn long_entries(n: usize) -> Vec<PackageEntry> {
    (0..n)
        .map(|i| {
            let name = format!("pkg{i:04}");
            PackageEntry {
                source: SourceKind::Winget,
                source_name: "winget".into(),
                id: name.clone(),
                name,
                moniker: None,
                version: "1".into(),
                publisher: None,
                description: None,
                tags: vec![],
                commands: vec![],
            }
        })
        .collect()
}

fn key(a: &mut App, code: crossterm::event::KeyCode) {
    update(
        a,
        Msg::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        )),
    );
}

#[test]
fn end_and_page_down_keep_the_selection_on_screen_at_100x24() {
    let mut a = App::new(long_entries(300), GlyphSet::Ascii);
    update(&mut a, Msg::Resize(100, 24));
    key(&mut a, crossterm::event::KeyCode::PageDown);
    let sel = a.selected;
    let name = format!("pkg{sel:04}");
    let out = render(&a, 100, 24);
    let line = out
        .lines()
        .map(|l| l.trim_start_matches('"'))
        .find(|l| l.starts_with("│ | "))
        .expect("selected row");
    assert!(line.contains(&name), "{line}");
    key(&mut a, crossterm::event::KeyCode::End);
    let name = format!("pkg{:04}", a.selected);
    let out = render(&a, 100, 24);
    let line = out
        .lines()
        .map(|l| l.trim_start_matches('"'))
        .find(|l| l.starts_with("│ | "))
        .expect("selected row");
    assert!(line.contains(&name), "{line}");
}

#[test]
fn tabs_threshold_is_exact() {
    let theme = Theme::new(true, ColorDepth::NoColor);
    // The three boxes need 45 columns plus one closing column.
    let wide = tab_buffer(46, 0, &theme);
    assert!(buf_row(&wide, 0).starts_with('╭'));
    let narrow = tab_buffer(45, 0, &theme);
    assert_eq!(buf_row(&narrow, 0).trim(), "");
}

#[test]
fn warn_label_is_warn_when_inactive_and_accent_when_active() {
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let mut labels = tab_labels();
    labels[2].warn = true;
    let draw = |active: usize| {
        let mut t = Terminal::new(TestBackend::new(60, 3)).unwrap();
        t.draw(|f| wingetrs::tui::tabs::draw_tabs(f, f.area(), &labels, active, &theme))
            .unwrap();
        t.backend().buffer().clone()
    };
    // 'A' of "Atualizações 2" sits at column 29.
    let inactive = draw(0);
    assert_eq!(inactive[(29, 1)].symbol(), "A");
    assert_eq!(inactive[(29, 1)].fg, theme.warn().fg.unwrap());
    let active = draw(2);
    assert_eq!(active[(29, 1)].fg, theme.accent().fg.unwrap());
}

#[test]
fn divider_is_focus_colored_only_when_details_has_focus() {
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let paint = |a: &App| {
        let mut t = Terminal::new(TestBackend::new(100, 16)).unwrap();
        t.draw(|f| view(a, &theme, f)).unwrap();
        t.backend().buffer().clone()
    };
    let mut a = typed("powert");
    // Divider at column 55, first list row is screen row 5.
    let b = paint(&a);
    assert_eq!(b[(55, 5)].symbol(), "│");
    assert_eq!(b[(55, 5)].fg, theme.border().fg.unwrap());
    key(&mut a, crossterm::event::KeyCode::Enter);
    let b = paint(&a);
    assert_eq!(b[(55, 5)].fg, theme.border_focus().fg.unwrap());
}

fn screen_cells(a: &App, w: u16, h: u16, theme: &Theme) -> ratatui::buffer::Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| view(a, theme, f)).unwrap();
    t.backend().buffer().clone()
}

fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect()
}

#[test]
fn selected_item_has_bar_and_pink_title() {
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let a = App::new(entries(), GlyphSet::Nerd);
    let buf = screen_cells(&a, 100, 20, &theme);
    let y1 = (0..20)
        .find(|&y| row_text(&buf, y).contains("EverythingPowerToys"))
        .expect("first item");
    assert_eq!(buf[(2, y1)].symbol(), "┃");
    assert_eq!(buf[(2, y1)].fg, theme.sel_bar().fg.unwrap());
    let nx = (0..100u16)
        .find(|&x| buf[(x, y1)].symbol() == "E")
        .expect("name");
    assert_eq!(buf[(nx, y1)].fg, theme.sel_title().fg.unwrap());
    assert!(buf[(nx, y1)].modifier.contains(Modifier::BOLD));
    // Second item: its name is not selected-colored.
    let y2 = y1 + 2;
    assert!(row_text(&buf, y2).contains("PowerToys"));
    let nx2 = (0..100u16)
        .find(|&x| buf[(x, y2)].symbol() == "P")
        .expect("name 2");
    assert_eq!(buf[(nx2, y2)].fg, theme.fg().fg.unwrap());
    assert_ne!(buf[(2, y2)].symbol(), "┃");
}

#[test]
fn second_line_shows_id_and_description() {
    let a = App::new(entries(), GlyphSet::Ascii);
    let out = render(&a, 120, 20);
    let lines: Vec<&str> = out.lines().collect();
    let i = lines
        .iter()
        .position(|l| l.contains("PowerToys") && !l.contains("Everything"))
        .expect("title line");
    assert!(
        lines[i + 1].contains("Microsoft.PowerToys · Utilitários para power users"),
        "{}",
        lines[i + 1]
    );
}

#[test]
fn end_of_list_is_visible_with_two_row_items() {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let a0 = App::new(long_entries(50), GlyphSet::Ascii);
    let mut a = a0;
    update(&mut a, Msg::Resize(100, 20));
    key(&mut a, crossterm::event::KeyCode::End);
    let buf = screen_cells(&a, 100, 20, &theme);
    let y = (0..20)
        .find(|&y| buf[(2, y)].symbol() == "|" && row_text(&buf, y).contains("pkg0049"))
        .expect("last item visible");
    assert_eq!(buf[(2, y)].symbol(), "|", "{}", row_text(&buf, y));
}

#[test]
fn end_and_page_keep_selection_on_screen_at_other_heights() {
    // Page in items = (height - CHROME_ROWS) / 2.
    for h in [12u16, 40] {
        let mut a = App::new(long_entries(300), GlyphSet::Ascii);
        update(&mut a, Msg::Resize(100, h));
        key(&mut a, crossterm::event::KeyCode::PageDown);
        key(&mut a, crossterm::event::KeyCode::End);
        let name = format!("pkg{:04}", a.selected);
        let out = render(&a, 100, h);
        let line = out
            .lines()
            .map(|l| l.trim_start_matches('"'))
            .find(|l| l.starts_with("│ | "))
            .expect("selected row");
        assert!(line.contains(&name), "h={h}: {line}");
    }
}
