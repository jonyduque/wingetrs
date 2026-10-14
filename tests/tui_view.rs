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

/// powertoys running with one more operation waiting behind it.
fn with_queue() -> App {
    let mut a = with_running();
    update(
        &mut a,
        Msg::Planned(Ok(Planned::Ready {
            text: "Instalar ripgrep".into(),
            action: Action::Apply(vec![vec![InstallPlan {
                app: "ripgrep".into(),
                ..plan()
            }]]),
        })),
    );
    press(&mut a, crossterm::event::KeyCode::Enter);
    a
}

fn with_queue_modal() -> App {
    let mut a = with_queue();
    press(&mut a, crossterm::event::KeyCode::Char('f'));
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
fn actions_modal_snapshot() {
    use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
    use wingetrs::tui::app::Tab;
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.installed = vec![ListRow {
        name: "ripgrep".into(),
        app_key: String::new(),
        not_in_catalog: false,
        origin: "arp".into(),
        installed_version: Some("1".into()),
        correlation: Correlation::Matched {
            source: "scoop:main".into(),
            package_id: "ripgrep".into(),
            rule: Rule::ScoopManifest,
        },
        latest: Some("2".into()),
        state: UpdateState::Available { latest: "2".into() },
        tiebreak: None,
        ..Default::default()
    }];
    a.tab = Tab::Installed;
    press(&mut a, crossterm::event::KeyCode::Enter);
    insta::assert_snapshot!(render(&a, 100, 16));
}

#[test]
fn queue_modal_snapshot() {
    insta::assert_snapshot!(render(&with_queue_modal(), 100, 16));
}

#[test]
fn status_bar_shows_running_and_queue() {
    let a = with_queue();
    assert_eq!(a.queue.len(), 1);
    let screen = render(&a, 100, 16);
    let bar = screen.lines().last().unwrap();
    assert!(bar.contains("Instalando powertoys"), "{bar}");
    assert!(bar.contains("powertoys.zip"), "{bar}");
    assert!(bar.contains("50%"), "{bar}");
    assert!(bar.contains("fila: 1"), "{bar}");
    // The running operation wins over a status message.
    let mut busy = with_queue();
    busy.status = Some("planejando…".into());
    let screen = render(&busy, 100, 16);
    assert!(screen.lines().last().unwrap().contains("fila: 1"));
    // Nothing queued: no queue count.
    let screen = render(&with_running(), 100, 16);
    let bar = screen.lines().last().unwrap();
    assert!(bar.contains("Instalando powertoys"), "{bar}");
    assert!(!bar.contains("fila"), "{bar}");
}

#[test]
fn status_bar_says_when_waiting_for_the_lock() {
    use wingetrs::tui::app::InstallFail;
    let mut a = with_queue();
    update(&mut a, Msg::InstallFinished(Err(InstallFail::Busy)));
    let screen = render(&a, 100, 16);
    let bar = screen.lines().last().unwrap();
    assert!(bar.contains("aguardando outra instalação"), "{bar}");
}

#[test]
fn queue_hint_shows_only_with_work() {
    use wingetrs::tui::help::hints;
    let idle = App::new(entries(), GlyphSet::Ascii);
    assert!(!hints(&idle).contains(&("f", "fila")));
    assert!(hints(&with_running()).contains(&("f", "fila")));
}

#[test]
fn residues_modal_snapshot() {
    insta::assert_snapshot!(render(&with_residues(), 100, 16));
}

#[test]
fn modals_never_panic_on_tiny_terminals() {
    for build in [with_confirm, with_picking, with_queue_modal, with_residues] {
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
            app_key: String::new(),
            not_in_catalog: false,
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
            ..Default::default()
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
fn divider_uses_the_border_color() {
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let paint = |a: &App| {
        let mut t = Terminal::new(TestBackend::new(100, 16)).unwrap();
        t.draw(|f| view(a, &theme, f)).unwrap();
        t.backend().buffer().clone()
    };
    let a = typed("powert");
    // Divider at column 55, first list row is screen row 5.
    let b = paint(&a);
    assert_eq!(b[(55, 5)].symbol(), "│");
    assert_eq!(b[(55, 5)].fg, theme.border().fg.unwrap());
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
    // Page in items = (height - chrome_rows(height)) / 2.
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

fn one_entry_with_description(d: &str) -> App {
    let mut e = entries().remove(0);
    e.description = Some(d.into());
    App::new(vec![e], GlyphSet::Ascii)
}

#[test]
fn cjk_description_is_clipped_by_cell_width() {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let a = one_entry_with_description("日本語のとても長い説明文日本語のと");
    let buf = screen_cells(&a, 100, 20, &theme);
    let y = (0..20)
        .find(|&y| row_text(&buf, y).contains("Microsoft.PowerToys ·"))
        .expect("second line");
    let div = (3..100u16)
        .find(|&x| buf[(x, y)].symbol() == "│")
        .expect("divider");
    assert_eq!(buf[(div - 1, y)].symbol(), " ", "gutter");
    let before: String = (0..div - 1)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect();
    assert!(before.trim_end().ends_with('…'), "{before}");
}

#[test]
fn newlines_in_description_become_spaces() {
    let a = one_entry_with_description("linha1\nlinha2");
    assert!(render(&a, 120, 20).contains("linha1 linha2"));
}

#[test]
fn installed_second_line_explains_missing_correlation() {
    use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
    let row = |name: &str, c: Correlation| ListRow {
        name: name.into(),
        app_key: String::new(),
        not_in_catalog: false,
        origin: "arp".into(),
        installed_version: Some("1".into()),
        correlation: c,
        latest: None,
        state: UpdateState::UpToDate,
        tiebreak: None,
        ..Default::default()
    };
    let _ = Rule::ScoopManifest;
    let mut a = App::new(vec![], GlyphSet::Ascii);
    update(
        &mut a,
        Msg::InstalledLoaded(vec![
            row("Orfao", Correlation::Unmatched),
            row(
                "Duplo",
                Correlation::Ambiguous {
                    source: "winget".into(),
                    candidates: vec!["a".into(), "b".into()],
                    rule: Rule::ScoopManifest,
                },
            ),
        ]),
    );
    key(&mut a, crossterm::event::KeyCode::Tab);
    let out = render(&a, 120, 20);
    assert!(out.contains("sem correspondência no catálogo"), "{out}");
    assert!(out.contains("correspondência ambígua"), "{out}");
}

fn keys_of(h: &[(&str, &str)]) -> Vec<String> {
    h.iter().map(|(k, _)| (*k).to_string()).collect()
}

#[test]
fn hints_change_with_the_tab() {
    use wingetrs::tui::app::Tab;
    use wingetrs::tui::help::hints;
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.tab = Tab::Search;
    assert_eq!(
        keys_of(&hints(&a)),
        ["↑/↓", "←/→", "enter", "espaço", "ctrl+s", "?"]
    );
    a.tab = Tab::Installed;
    assert_eq!(
        keys_of(&hints(&a)),
        ["↑/↓", "←/→", "enter", "u", "o", "espaço", "ctrl+s", "?"]
    );
    a.tab = Tab::Updates;
    assert_eq!(
        keys_of(&hints(&a)),
        ["↑/↓", "←/→", "enter", "o", "espaço", "ctrl+s", "?"]
    );
    assert!(!keys_of(&hints(&a)).contains(&"esc".to_string()));
}

fn line_text(l: &ratatui::text::Line) -> String {
    l.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn help_line_truncates_whole_hints_with_ellipsis() {
    use wingetrs::tui::help::{help_line, hints};
    let theme = Theme::new(true, ColorDepth::NoColor);
    let a = App::new(entries(), GlyphSet::Ascii);
    let h = hints(&a);
    let line = help_line(&h, 40, &theme);
    let text = line_text(&line);
    assert!(text.ends_with(" …"), "{text}");
    assert!(line.width() <= 40, "{text}");
    let body = text.trim_end_matches(" …");
    let shown: Vec<&str> = body.split(" • ").collect();
    assert!(shown.len() < h.len());
    for (s, (k, d)) in shown.iter().zip(h.iter()) {
        assert_eq!(*s, format!("{k} {d}"));
    }
    for w in 0..=100usize {
        let l = help_line(&h, w, &theme);
        let t = line_text(&l);
        assert!(l.width() <= w, "w={w}: {t}");
        let whole =
            t.matches(" • ").count() + usize::from(!t.is_empty()) == h.len() && !t.contains('…');
        assert!(whole || t.ends_with(" …") || t.is_empty(), "w={w}: {t}");
        for piece in t
            .trim_end_matches(" …")
            .split(" • ")
            .filter(|p| !p.is_empty())
        {
            assert!(
                h.iter().any(|(k, d)| piece == format!("{k} {d}")),
                "w={w}: {piece}"
            );
        }
    }
    let wide = line_text(&help_line(&h, 200, &theme));
    assert!(!wide.contains('…'), "{wide}");
    assert!(wide.contains("? ajuda"), "{wide}");
}

fn col_of(text: &str, byte: usize) -> u16 {
    u16::try_from(text[..byte].chars().count()).unwrap()
}

#[test]
fn status_message_is_a_pink_pill() {
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.status = Some("Instalado: rg".into());
    let buf = screen_cells(&a, 100, 24, &theme);
    let (x, y) = (0..24)
        .find_map(|y| {
            row_text(&buf, y)
                .find("Instalado:")
                .map(|b| (col_of(&row_text(&buf, y), b), y))
        })
        .expect("status text");
    assert_eq!(buf[(x, y)].bg, theme.pill_msg().bg.unwrap());
}

#[test]
fn short_terminal_hides_the_help_line() {
    let a = App::new(entries(), GlyphSet::Ascii);
    assert!(!render(&a, 100, 18).contains("? ajuda"));
    assert!(render(&a, 100, 24).contains("? ajuda"));
}

#[test]
fn help_line_colors_keys_and_descriptions() {
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let a = App::new(entries(), GlyphSet::Ascii);
    let buf = screen_cells(&a, 100, 24, &theme);
    let y = (0..24)
        .find(|&y| row_text(&buf, y).contains("enter instalar"))
        .expect("help row");
    let text = row_text(&buf, y);
    let x = col_of(&text, text.find("enter instalar").unwrap());
    assert_eq!(buf[(x, y)].fg, theme.help_key().fg.unwrap());
    assert_eq!(buf[(x + 6, y)].fg, theme.help_desc().fg.unwrap());
}

#[test]
fn end_keeps_selection_visible_when_the_help_bar_hides() {
    for h in [19u16, 20] {
        let mut a = App::new(long_entries(300), GlyphSet::Ascii);
        update(&mut a, Msg::Resize(100, h));
        key(&mut a, crossterm::event::KeyCode::End);
        let name = format!("pkg{:04}", a.selected);
        let out = render(&a, 100, h);
        assert!(out.contains(&name), "h={h}");
    }
}

#[test]
fn full_help_has_key_column_and_accent_keys() {
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let mut a = App::new(entries(), GlyphSet::Ascii);
    key(&mut a, crossterm::event::KeyCode::Char('?'));
    let buf = screen_cells(&a, 100, 30, &theme);
    let rows: Vec<(u16, String)> = (0..30).map(|y| (y, row_text(&buf, y))).collect();
    let find = |needle: &str| {
        rows.iter()
            .find_map(|(y, t)| t.find(needle).map(|b| (col_of(t, b), *y)))
            .expect(needle)
    };
    let (kx, ky) = find("←→ / Tab");
    let (dx, dy) = find("trocar aba");
    assert_eq!((dy, dx), (ky, kx + 12));
    for (key_text, desc) in [("PgUp PgDn", "mover de 10"), ("Ctrl+S", "filtrar fonte")] {
        let (k, ky) = find(key_text);
        let (d, dy) = find(desc);
        assert_eq!((dy, d), (ky, kx + 12), "{desc}");
        assert_eq!(k, kx, "{key_text}");
    }
    let cell = &buf[(kx, ky)];
    assert_eq!(cell.fg, theme.accent().fg.unwrap());
    assert!(cell.modifier.contains(Modifier::BOLD));
    assert_eq!(buf[(dx, dy)].fg, theme.fg().fg.unwrap());
    // Dialog border: the corner cell above-left of the padded content.
    let (_, title_y) = find(" Ajuda ");
    let (tx, _) = find(" Ajuda ");
    assert_eq!(buf[(tx - 1, title_y)].fg, theme.accent().fg.unwrap());
}

#[test]
fn long_status_keeps_right_pills_visible() {
    let theme = Theme::new(true, ColorDepth::NoColor);
    for status in ["x".repeat(300), "日本語のメッセージ".repeat(30)] {
        let mut a = App::new(entries(), GlyphSet::Ascii);
        a.offline = true;
        a.status = Some(status);
        let buf = screen_cells(&a, 80, 24, &theme);
        let y = (0..24)
            .find(|&y| row_text(&buf, y).contains("offline"))
            .expect("offline pill");
        let text = row_text(&buf, y);
        assert!(text.contains('…'), "{text}");
        assert!(text.contains("… ") || text.contains("…"), "{text}");
        let left = text.split("offline").next().unwrap().trim_end();
        assert!(left.ends_with('…') || left.ends_with("… "), "{text}");
        assert!(
            text.trim_end().ends_with("offline") || text.ends_with(' '),
            "{text}"
        );
    }
}

#[test]
fn toast_box_sits_top_right_with_margins() {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.push_toast(wingetrs::tui::app::ToastKind::Success, "Instalado: rg");
    let buf = screen_cells(&a, 100, 24, &theme);
    assert_eq!(buf[(97, 4)].symbol(), "╮");
    assert_eq!(buf[(98, 4)].symbol(), " ");
    assert_eq!(buf[(99, 4)].symbol(), "│");
    assert!(
        row_text(&buf, 5).contains("+ Sucesso"),
        "{}",
        row_text(&buf, 5)
    );
    assert!(
        row_text(&buf, 6).contains("│ Instalado: rg"),
        "{}",
        row_text(&buf, 6)
    );
}

#[test]
fn small_terminal_puts_the_toast_in_the_status_bar() {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.push_toast(wingetrs::tui::app::ToastKind::Success, "Instalado: rg");
    let buf = screen_cells(&a, 50, 10, &theme);
    for y in 3..=6 {
        for x in 40..50 {
            assert_ne!(buf[(x, y)].symbol(), "╮", "({x},{y})");
        }
    }
    assert!(
        row_text(&buf, 9).contains("Instalado: rg"),
        "{}",
        row_text(&buf, 9)
    );
}

#[test]
fn toasts_never_cover_a_dialog() {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut a = with_residues();
    assert!(a.modal.is_some());
    assert_eq!(a.toasts.len(), 1);
    let buf = screen_cells(&a, 100, 24, &theme);
    // Dialog rectangle: from its top-left corner (the title row) to its bottom-left corner.
    let top = (0..24)
        .find(|&y| row_text(&buf, y).contains("Resíduos de powertoys"))
        .expect("dialog title");
    let text = row_text(&buf, top);
    let left = col_of(&text, text.find("╭ Resíduos").unwrap());
    let right = (left + 1..100)
        .find(|&x| buf[(x, top)].symbol() == "╮")
        .expect("dialog top-right corner");
    let bottom = (top + 1..24)
        .find(|&y| buf[(left, y)].symbol() == "╰")
        .expect("dialog bottom-left corner");
    for y in top..=bottom {
        for x in left..=right {
            let on_edge = y == top || y == bottom || x == left || x == right;
            let sym = buf[(x, y)].symbol();
            if !on_edge {
                assert!(
                    !["╭", "╮", "╰", "╯", "│", "─"].contains(&sym),
                    "toast border inside the dialog at ({x},{y}): {}",
                    row_text(&buf, y)
                );
            } else if y == top && x > left && x < right {
                assert!(sym != "╭" && sym != "╮", "({x},{y}): {}", row_text(&buf, y));
            }
        }
    }
    assert!((0..24).all(|y| !row_text(&buf, y).contains("Sucesso")));
    assert!(
        row_text(&buf, 22).contains("powertoys removido"),
        "{}",
        row_text(&buf, 22)
    );
    // Closing the dialog brings the toast back as a box while it is still due.
    press(&mut a, crossterm::event::KeyCode::Esc);
    assert!(a.modal.is_none());
    let out = render(&a, 100, 24);
    assert!(out.contains("+ Sucesso"), "{out}");
}

#[test]
fn repeated_toast_shows_its_count_in_the_title() {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.push_toast(wingetrs::tui::app::ToastKind::Success, "Instalado: rg");
    a.push_toast(wingetrs::tui::app::ToastKind::Success, "Instalado: rg");
    assert_eq!(a.toasts[0].count, 2);
    let out = render(&a, 100, 24);
    assert!(out.contains("+ Sucesso ×2"), "{out}");
}

#[test]
fn settings_screen_snapshot() {
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.tab = wingetrs::tui::app::Tab::Settings;
    a.buckets_summary = "main, extras".into();
    insta::assert_snapshot!(render(&a, 100, 24));
}

#[test]
fn split_icon_leaves_a_gap() {
    use ratatui::layout::Rect;
    use wingetrs::tui::layout::split_icon;
    let (icon, text) = split_icon(Rect::new(0, 0, 40, 10), 4, 8, 2);
    assert_eq!(icon, Rect::new(0, 0, 8, 4));
    assert_eq!(text, Rect::new(10, 0, 30, 4));
    let (icon, text) = split_icon(Rect::new(3, 2, 15, 10), 4, 8, 2);
    assert_eq!(icon, Rect::new(3, 2, 8, 4));
    assert_eq!(text, Rect::new(13, 2, 5, 4));
    // Narrower than icon + gap: saturates instead of panicking.
    let (_, text) = split_icon(Rect::new(0, 0, 9, 2), 4, 8, 2);
    assert_eq!((text.x, text.width, text.height), (9, 0, 2));
}

/// An opaque red 8x4-cell halfblocks icon: its cells get a red background in the buffer.
fn test_icon() -> ratatui_image::protocol::Protocol {
    use wingetrs::tui::layout::{DETAIL_ICON_COLS, DETAIL_ICON_ROWS};
    let picker = ratatui_image::picker::Picker::halfblocks();
    let fs = picker.font_size();
    let img = image::RgbaImage::from_pixel(
        u32::from(DETAIL_ICON_COLS * fs.width),
        u32::from(DETAIL_ICON_ROWS * fs.height),
        image::Rgba([255, 0, 0, 255]),
    );
    picker
        .new_protocol(
            image::DynamicImage::ImageRgba8(img),
            ratatui::layout::Size::new(DETAIL_ICON_COLS, DETAIL_ICON_ROWS),
            ratatui_image::Resize::Fit(None),
        )
        .unwrap()
}

fn selected_key(a: &App) -> String {
    let (s, id) = a.selected_key().unwrap();
    format!("{s}/{id}")
}

/// `typed("powert")` with the selected package's icon encoded and the protocol on.
fn with_icon() -> App {
    use wingetrs::tui::layout::{DETAIL_ICON_COLS, DETAIL_ICON_ROWS};
    let mut a = typed("powert");
    a.icon_protocol = wingetrs::tui::icons::IconProtocol::Sixel;
    let key = selected_key(&a);
    a.icons
        .insert(key, (DETAIL_ICON_COLS, DETAIL_ICON_ROWS), test_icon());
    a
}

fn is_icon(c: &ratatui::buffer::Cell) -> bool {
    c.bg == ratatui::style::Color::Rgb(255, 0, 0)
}

fn icon_cells(buf: &ratatui::buffer::Buffer) -> usize {
    buf.content().iter().filter(|c| is_icon(c)).count()
}

#[test]
fn details_text_starts_after_the_icon_gap() {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let buf = screen_cells(&with_icon(), 100, 16, &theme);
    // Details pane: text column 57, first line 5 (see list_and_details_text_start_one_column_after_the_border).
    let (x, y) = (57u16, 5u16);
    let mut drawn = 0;
    for row in y..y + 4 {
        for col in x..x + 8 {
            let s = buf[(col, row)].symbol();
            assert!(
                !s.chars().any(char::is_alphanumeric),
                "text inside the icon at ({col},{row}): {s:?}"
            );
            drawn += usize::from(is_icon(&buf[(col, row)]));
        }
        assert_eq!(buf[(x + 8, row)].symbol(), " ", "row {row}");
        assert_eq!(buf[(x + 9, row)].symbol(), " ", "row {row}");
    }
    assert_eq!(drawn, 32, "the icon fills its 8x4 cells");
    let line = row_text(&buf, y);
    let name_at: String = line.chars().skip(usize::from(x) + 10).take(9).collect();
    assert_eq!(name_at, "PowerToys", "{line}");
    // Only the identity lines (name, ID, version) sit beside the icon; the description
    // starts below it, at the full width.
    let below: String = row_text(&buf, y + 4).chars().skip(usize::from(x)).collect();
    assert!(below.starts_with("Utilitários para power users"), "{below}");
}

#[test]
fn details_without_icon_keep_the_old_layout() {
    use wingetrs::tui::icons::IconProtocol;
    let base = render(&typed("powert"), 100, 16);
    // Bytes but the protocol off.
    let mut a = typed("powert");
    let key = selected_key(&a);
    a.icon_bytes.insert(key.clone(), vec![1, 2, 3]);
    assert_eq!(render(&a, 100, 16), base);
    // Protocol on, still encoding (nothing in the cache).
    // (The list indents for the icon column, so only the details pane is compared.)
    a.icon_protocol = IconProtocol::Sixel;
    let theme = Theme::new(true, ColorDepth::NoColor);
    let pane = |a: &App| pane_cells(&screen_cells(a, 100, 16, &theme), PANE_X, 5..14);
    assert_eq!(pane(&a), pane(&typed("powert")));
    // An encoded icon with the protocol off is not drawn either.
    let mut b = with_icon();
    b.icon_protocol = IconProtocol::Off;
    assert_eq!(render(&b, 100, 16), base);
    // Control: with the icon the screen does change.
    assert_ne!(render(&with_icon(), 100, 16), base);
}

#[test]
fn no_icon_over_a_dialog() {
    use wingetrs::tui::view::icon_visible;
    let theme = Theme::new(true, ColorDepth::NoColor);
    let a = with_icon();
    assert!(icon_visible(&a));
    assert!(icon_cells(&screen_cells(&a, 100, 16, &theme)) > 0);
    let mut m = with_confirm();
    m.icon_protocol = wingetrs::tui::icons::IconProtocol::Sixel;
    let key = selected_key(&m);
    m.icons.insert(key, (8, 4), test_icon());
    assert!(!icon_visible(&m));
    assert_eq!(icon_cells(&screen_cells(&m, 100, 16, &theme)), 0);
    let mut h = with_icon();
    h.show_help = true;
    assert!(!icon_visible(&h));
    assert_eq!(icon_cells(&screen_cells(&h, 100, 16, &theme)), 0);
}

#[test]
fn icon_crossing_a_toast_is_not_drawn() {
    use wingetrs::tui::app::ToastKind;
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut a = with_icon();
    a.push_toast(ToastKind::Error, "falhou");
    let buf = screen_cells(&a, 100, 16, &theme);
    assert_eq!(icon_cells(&buf), 0);
}

fn add_icon(a: &mut App) {
    use wingetrs::tui::layout::{DETAIL_ICON_COLS, DETAIL_ICON_ROWS};
    a.icon_protocol = wingetrs::tui::icons::IconProtocol::Sixel;
    let key = selected_key(a);
    a.icons
        .insert(key, (DETAIL_ICON_COLS, DETAIL_ICON_ROWS), test_icon());
}

/// Details text column at 100 columns: pane starts at 57 and ends before the border at 98.
const PANE_X: u16 = 57;
const PANE_END: u16 = 98;

fn pane_cells(buf: &ratatui::buffer::Buffer, x0: u16, rows: std::ops::Range<u16>) -> String {
    rows.map(|y| {
        (x0..PANE_END)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect::<String>()
    })
    .collect::<Vec<_>>()
    .join("\n")
}

fn pane_words(a: &App, h: u16) -> Vec<String> {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let buf = screen_cells(a, 100, h, &theme);
    let mut w: Vec<String> = pane_cells(&buf, PANE_X, 5..h - 2)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    w.sort();
    w
}

fn long_id_app() -> App {
    let mut a = App::new(vec![], GlyphSet::Ascii);
    let mut e = entries().remove(0);
    e.id = "Contoso.AnExtremelyLongPackageIdentifierThatWraps".into();
    e.name = "Tool".into();
    update(&mut a, Msg::CatalogLoaded(vec![e]));
    a
}

/// Installed tab, catalog match without publisher and an installed row without version.
fn bare_installed_app() -> App {
    use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
    let mut a = App::new(vec![], GlyphSet::Ascii);
    let mut e = entries().remove(0);
    e.id = "Acme.Tool".into();
    e.name = "Tool".into();
    e.publisher = None;
    e.description = Some("Descricao completa do pacote".into());
    update(&mut a, Msg::CatalogLoaded(vec![e]));
    update(
        &mut a,
        Msg::InstalledLoaded(vec![ListRow {
            name: "Tool".into(),
            app_key: String::new(),
            not_in_catalog: false,
            origin: "arp".into(),
            installed_version: None,
            correlation: Correlation::Matched {
                source: "winget".into(),
                package_id: "Acme.Tool".into(),
                rule: Rule::ScoopManifest,
            },
            latest: None,
            state: UpdateState::UpToDate,
            tiebreak: None,
            ..Default::default()
        }]),
    );
    press(&mut a, crossterm::event::KeyCode::Tab);
    a
}

#[test]
fn long_id_beside_the_icon_wraps_in_full() {
    let mut a = long_id_app();
    add_icon(&mut a);
    let theme = Theme::new(true, ColorDepth::NoColor);
    let buf = screen_cells(&a, 100, 16, &theme);
    assert!(icon_cells(&buf) > 0);
    let beside: String = pane_cells(&buf, PANE_X + 10, 5..9)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        beside.contains("Contoso.AnExtremelyLongPackageIdentifierThatWraps"),
        "{beside}"
    );
}

#[test]
fn description_without_publisher_and_version_goes_below_at_full_width() {
    let mut a = bare_installed_app();
    add_icon(&mut a);
    let theme = Theme::new(true, ColorDepth::NoColor);
    let buf = screen_cells(&a, 100, 16, &theme);
    assert!(icon_cells(&buf) > 0);
    let rows: Vec<String> = pane_cells(&buf, PANE_X, 5..14)
        .lines()
        .map(str::to_string)
        .collect();
    let at = rows
        .iter()
        .position(|r| r.starts_with("Descricao completa do pacote"))
        .unwrap_or_else(|| panic!("{rows:#?}"));
    assert!(at >= 4, "description beside the icon: {rows:#?}");
    assert!(
        !rows[..4].iter().any(|r| r.contains("instalado")),
        "install line beside the icon: {rows:#?}"
    );
}

#[test]
fn icon_layout_draws_the_same_words() {
    for (name, plain) in [
        ("search", typed("powert")),
        ("installed", bare_installed_app()),
    ] {
        let mut with = if name == "search" {
            typed("powert")
        } else {
            bare_installed_app()
        };
        add_icon(&mut with);
        let theme = Theme::new(true, ColorDepth::NoColor);
        assert!(
            icon_cells(&screen_cells(&with, 100, 20, &theme)) > 0,
            "{name}"
        );
        assert_eq!(pane_words(&with, 20), pane_words(&plain, 20), "{name}");
    }
}

/// An opaque red halfblocks icon of `cols` x `rows` cells.
fn red_icon(cols: u16, rows: u16) -> ratatui_image::protocol::Protocol {
    let picker = ratatui_image::picker::Picker::halfblocks();
    let fs = picker.font_size();
    let img = image::RgbaImage::from_pixel(
        u32::from(cols * fs.width),
        u32::from(rows * fs.height),
        image::Rgba([255, 0, 0, 255]),
    );
    picker
        .new_protocol(
            image::DynamicImage::ImageRgba8(img),
            ratatui::layout::Size::new(cols, rows),
            ratatui_image::Resize::Fit(None),
        )
        .unwrap()
}

/// `(column, row)` of the first occurrence of `needle` on screen (one cell per char).
fn find_text(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
    (0..buf.area.height).find_map(|y| {
        let row = row_text(buf, y);
        let at = row.find(needle)?;
        Some((u16::try_from(row[..at].chars().count()).unwrap(), y))
    })
}

#[test]
fn list_indents_every_item_when_icons_are_on() {
    use wingetrs::tui::icons::IconProtocol;
    use wingetrs::tui::layout::{LIST_ICON_COLS, LIST_ICON_GAP};
    let theme = Theme::new(true, ColorDepth::NoColor);
    let ids = [
        "Microsoft.PowerToys ·",
        "lin-ycv.EverythingPowerToys ·",
        "powertoys · U",
    ];
    let off = typed("powert");
    let off_buf = screen_cells(&off, 100, 16, &theme);
    let off_pos: Vec<(u16, u16)> = ids
        .iter()
        .map(|i| find_text(&off_buf, i).unwrap())
        .collect();

    // One of the three has an icon in the cache; the protocol is on.
    let mut on = typed("powert");
    on.icon_protocol = IconProtocol::Sixel;
    let (src, id) = on.list_keys(0..1).remove(0).unwrap();
    on.icons.insert(
        format!("{src}/{id}"),
        (LIST_ICON_COLS, 2),
        red_icon(LIST_ICON_COLS, 2),
    );
    let on_buf = screen_cells(&on, 100, 16, &theme);
    let indent = LIST_ICON_COLS + LIST_ICON_GAP;
    let list_x = off_pos[0].0 - 4;
    let mut icon_rows = 0;
    for (id, (x, y)) in ids.iter().zip(&off_pos) {
        let (nx, ny) = find_text(&on_buf, id).unwrap();
        assert_eq!((nx, ny), (x + indent, *y), "{id} text column");
        // The 4 icon columns of both lines of the item hold no text.
        for row in [y - 1, *y] {
            for col in list_x..list_x + LIST_ICON_COLS {
                let c = &on_buf[(col, row)];
                if is_icon(c) {
                    icon_rows += 1;
                } else {
                    assert_eq!(c.symbol(), " ", "text in the icon area at ({col},{row})");
                }
            }
            assert_eq!(on_buf[(list_x + LIST_ICON_COLS, row)].symbol(), " ");
        }
    }
    assert_eq!(icon_rows, 8, "the one cached icon fills its 4x2 cells");
    assert_eq!(icon_cells(&on_buf), 8);

    // Off: no indent and no icon even with one in the cache.
    let mut still_off = typed("powert");
    still_off.icons.insert(
        format!("{src}/{id}"),
        (LIST_ICON_COLS, 2),
        red_icon(LIST_ICON_COLS, 2),
    );
    assert_eq!(
        render(&still_off, 100, 16),
        render(&typed("powert"), 100, 16)
    );
}

#[test]
fn list_page_rows_matches_the_items_the_view_draws() {
    let many: Vec<PackageEntry> = (0..80)
        .map(|i| PackageEntry {
            source: SourceKind::Winget,
            source_name: "winget".into(),
            id: format!("Pkg.N{i:02}"),
            name: format!("pkg{i:02}"),
            moniker: None,
            version: "1".into(),
            publisher: None,
            description: None,
            tags: vec![],
            commands: vec![],
        })
        .collect();
    let theme = Theme::new(true, ColorDepth::NoColor);
    // Heights below and above the help-bar threshold (20) and the chrome threshold (8).
    for h in 1..=60u16 {
        let mut a = App::new(many.clone(), GlyphSet::Ascii);
        update(&mut a, Msg::Resize(100, h));
        let buf = screen_cells(&a, 100, h, &theme);
        let drawn = (0..h)
            .filter(|y| {
                row_text(&buf, *y)
                    .chars()
                    .take(50)
                    .collect::<String>()
                    .contains("pkg")
            })
            .count();
        assert_eq!(wingetrs::tui::app::list_page_rows(h), drawn, "height {h}");
    }
}

#[test]
fn turning_icons_off_clears_the_cache_and_the_indent() {
    use crossterm::event::KeyCode;
    use wingetrs::tui::app::Tab;
    use wingetrs::tui::icons::IconProtocol;
    use wingetrs::tui::layout::LIST_ICON_COLS;
    let mut a = typed("powert");
    a.icon_protocol = IconProtocol::Sixel;
    a.settings.icons = "sixel".into();
    let (src, id) = a.list_keys(0..1).remove(0).unwrap();
    a.icons.insert(
        format!("{src}/{id}"),
        (LIST_ICON_COLS, 2),
        red_icon(LIST_ICON_COLS, 2),
    );
    // The details pane has its icon too, so the switch must also undo that layout.
    a.icons
        .insert(format!("{src}/{id}"), (8, 4), red_icon(8, 4));
    a.status = None;
    assert_ne!(
        render(&a, 100, 16),
        render(&typed("powert"), 100, 16),
        "icons change the frame while on"
    );
    assert!(!a.icons.is_empty());
    a.tab = Tab::Settings;
    for _ in 0..6 {
        key(&mut a, KeyCode::Down);
    }
    key(&mut a, KeyCode::Enter);
    assert_eq!(a.settings.icons, "off");
    assert_eq!(a.icon_protocol, IconProtocol::Off);
    assert!(a.icons.is_empty(), "cache cleared");
    a.tab = Tab::Search;
    a.status = None; // "salvando…" is the only other difference
    assert_eq!(
        render(&a, 100, 16),
        render(&typed("powert"), 100, 16),
        "no icon, no indent"
    );
}

fn licensed_row(set: GlyphSet) -> String {
    let mut a = App::new(entries(), set);
    let (source, id) = a.selected_key().unwrap();
    let details = wingetrs::core::PackageDetails {
        source: wingetrs::core::SourceKind::Winget,
        id: id.clone(),
        name: id.clone(),
        version: "1".into(),
        publisher: None,
        description: None,
        homepage: None,
        license: Some("Zlicense-text".into()),
        tags: Vec::new(),
        installers: Vec::new(),
        available_versions: Vec::new(),
        icon: None,
    };
    a.details.insert(format!("{source}/{id}"), Ok(details));
    render(&a, 120, 24)
        .lines()
        .find(|l| l.contains("Zlicense-text"))
        .expect("license row")
        .to_string()
}

#[test]
fn nerd_detail_glyphs_have_two_spaces() {
    assert!(licensed_row(GlyphSet::Nerd).contains("\u{f24e}  Zlicense-text"));
    let u = licensed_row(GlyphSet::Unicode);
    assert!(u.contains("§ Zlicense-text"));
    assert!(!u.contains("§  Zlicense-text"));
}

// ---- Task 94: drawing and mouse share one geometry ----

#[test]
fn tab_titles_match_regions() {
    use ratatui::layout::Rect;
    use wingetrs::tui::app::Tab;
    use wingetrs::tui::regions::regions;
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut narrow_seen = false;
    for glyphs in [GlyphSet::Ascii, GlyphSet::Unicode, GlyphSet::Nerd] {
        for (w, h) in [(80, 24), (100, 30), (60, 15)] {
            for tab in [Tab::Search, Tab::Installed, Tab::Updates, Tab::Settings] {
                let mut a = App::new(entries(), glyphs);
                a.tab = tab;
                a.size = (w, h);
                let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                t.draw(|f| view(&a, &theme, f)).unwrap();
                let buf = t.backend().buffer().clone();
                let r = regions(&a, Rect::new(0, 0, w, h));
                assert_eq!(r.tabs.len(), 4);
                narrow_seen |= r.tabs[0].0.height == 1;
                for ((rect, t), name) in
                    r.tabs
                        .iter()
                        .zip(["Pesquisar", "Instalados", "Atualizações", "Ajustes"])
                {
                    // Some row of the rectangle holds the whole title text, cell by cell.
                    let found = (rect.y..rect.bottom()).any(|y| {
                        let row: String = (rect.x..rect.right())
                            .map(|x| buf[(x, y)].symbol().to_string())
                            .collect();
                        row.contains(name)
                    });
                    assert!(
                        found,
                        "{name} ({t:?}) not inside {rect:?} at {w}x{h}, {glyphs:?}, tab {tab:?}"
                    );
                }
            }
        }
    }
    assert!(narrow_seen, "the narrow form must be covered");
}

#[test]
fn setting_rows_match_the_drawing() {
    use ratatui::layout::Rect;
    use wingetrs::tui::app::Tab;
    use wingetrs::tui::regions::regions;
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.tab = Tab::Settings;
    a.size = (100, 30);
    let out = render(&a, 100, 30);
    let lines: Vec<&str> = out.lines().collect();
    let r = regions(&a, Rect::new(0, 0, 100, 30));
    assert_eq!(r.setting_rows.len(), wingetrs::tui::settings::ORDER.len());
    for (rect, id) in r.setting_rows.iter().zip(wingetrs::tui::settings::ORDER) {
        assert!(
            lines[usize::from(rect.y)].contains(id.label()),
            "{} not on row {}",
            id.label(),
            rect.y
        );
    }
}
#[test]
fn space_toggles_mark_and_glyph() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut a = App::new(entries(), GlyphSet::Unicode);
    let space = || Msg::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    let glyph = wingetrs::tui::theme::Glyphs::for_set(GlyphSet::Unicode).mark;
    assert!(!render(&a, 100, 24).contains(glyph));
    update(&mut a, space());
    let marked = render(&a, 100, 24);
    assert!(marked.contains(glyph), "{marked}");
    assert!(marked.contains("1 marcados"), "{marked}");
    update(&mut a, space());
    let cleared = render(&a, 100, 24);
    assert!(!cleared.contains(glyph), "{cleared}");
    assert!(!cleared.contains("marcados"), "{cleared}");
}

#[test]
fn mark_column_aligns_both_lines() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut a = App::new(entries(), GlyphSet::Unicode);
    let col = |s: &str, needle: &str| {
        s.lines()
            .find(|l| l.contains(needle))
            .and_then(|l| l.find(needle))
            .map(|i| l_chars(s, needle, i))
    };
    fn l_chars(s: &str, needle: &str, byte: usize) -> usize {
        let line = s.lines().find(|l| l.contains(needle)).unwrap();
        line[..byte].chars().count()
    }
    let before = render(&a, 100, 24);
    let (n0, d0) = (
        col(&before, "PowerToys").unwrap(),
        col(&before, "Utilit").unwrap(),
    );
    update(
        &mut a,
        Msg::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
    );
    let after = render(&a, 100, 24);
    let (n1, d1) = (
        col(&after, "PowerToys").unwrap(),
        col(&after, "Utilit").unwrap(),
    );
    assert_eq!(
        n1 - n0,
        d1 - d0,
        "both lines shift together:
{after}"
    );
    assert!(n1 > n0, "{after}");
    // A mark hidden by the query leaves no column.
    for c in "zzz".chars() {
        let cmds = update(
            &mut a,
            Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)),
        );
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
    assert_eq!(a.marked_count(), 1);
    assert!(!a.has_visible_mark());
}

#[test]
fn pinned_row_shows_the_pin_glyph() {
    use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
    use wingetrs::tui::app::Tab;
    let mut a = App::new(entries(), GlyphSet::Unicode);
    a.tab = Tab::Installed;
    a.size = (100, 24);
    a.installed = vec![ListRow {
        name: "beta".into(),
        app_key: String::new(),
        not_in_catalog: false,
        origin: "wingetrs".into(),
        installed_version: Some("1".into()),
        correlation: Correlation::Matched {
            source: "scoop:main".into(),
            package_id: "beta".into(),
            rule: Rule::ScoopManifest,
        },
        latest: None,
        state: UpdateState::UpToDate,
        tiebreak: None,
        ..Default::default()
    }];
    let pin = wingetrs::tui::theme::Glyphs::for_set(GlyphSet::Unicode).pin;
    assert!(!render(&a, 100, 24).contains(pin));
    a.pins.insert(wingetrs::pins::key("wingetrs", "beta"));
    let out = render(&a, 100, 24);
    assert!(out.contains(pin), "{out}");
}

#[test]
fn details_list_records_and_warning() {
    use wingetrs::installed::correlate::{ListRow, OtherRecord, UpdateState};
    use wingetrs::tui::app::Tab;
    let mut a = App::new(entries(), GlyphSet::Ascii);
    a.installed = vec![ListRow {
        name: "Kindle".into(),
        origin: "arp".into(),
        location: "usuário".into(),
        installed_version: Some("2.9.1".into()),
        others: vec![OtherRecord {
            app_key: "arp/hklm32/K".into(),
            origin: "arp".into(),
            location: "máquina 32-bit".into(),
            installed_version: Some("2.8.3".into()),
            state: UpdateState::Unknown,
        }],
        ..Default::default()
    }];
    a.tab = Tab::Installed;
    let screen = render(&a, 140, 24);
    assert!(screen.contains("Registros"), "{screen}");
    assert!(screen.contains("usuário · 2.9.1 · arp"), "{screen}");
    assert!(screen.contains("máquina 32-bit · 2.8.3 · arp"), "{screen}");
    assert!(screen.contains("mais de um registro"), "{screen}");
    assert!(screen.contains("+1"), "{screen}");
    a.installed[0].others.clear();
    let screen = render(&a, 140, 24);
    assert!(!screen.contains("Registros"), "{screen}");
}
