#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Frame-cost benchmark for the TUI list (Task 59). Needs the real catalog on disk.
//! Run: cargo run --release --example tui_frame
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::time::Instant;
use wingetrs::cache::DataDir;
use wingetrs::catalog::Catalog;
use wingetrs::core::PackageEntry;
use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
use wingetrs::tui::app::{App, Cmd, Msg, run_search, update};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Theme};
use wingetrs::tui::view::view;

const FRAMES: usize = 200;
const SEARCH_RUNS: usize = 50;

fn rows(entries: &[PackageEntry]) -> Vec<ListRow> {
    entries
        .iter()
        .take(400)
        .enumerate()
        .map(|(i, e)| ListRow {
            name: e.name.clone(),
            origin: "arp".into(),
            installed_version: Some("1".into()),
            correlation: Correlation::Matched {
                source: e.source_name.clone(),
                package_id: e.id.clone(),
                rule: Rule::ScoopManifest,
            },
            latest: Some("2".into()),
            state: if i % 4 == 0 {
                UpdateState::Available { latest: "2".into() }
            } else {
                UpdateState::UpToDate
            },
            tiebreak: None,
        })
        .collect()
}

fn key(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

fn scenario(entries: &[PackageEntry], tabs: usize, query: &str) -> (u128, u128) {
    let mut app = App::new(entries.to_vec(), GlyphSet::Nerd);
    update(&mut app, Msg::InstalledLoaded(rows(entries)));
    for _ in 0..tabs {
        update(&mut app, key(KeyCode::Tab));
    }
    for c in query.chars() {
        for cmd in update(&mut app, key(KeyCode::Char(c))) {
            // The event loop runs this off the UI thread; here it is delivered inline.
            if let Cmd::Search {
                generation,
                query,
                kind,
            } = cmd
            {
                let hits = run_search(&app.search, &query, kind);
                update(&mut app, Msg::SearchDone { generation, hits });
            }
        }
    }
    let theme = Theme::new(true, ColorDepth::TrueColor);
    let mut term = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
    let mut times = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let t = Instant::now();
        term.draw(|f| view(&app, &theme, f)).expect("draw");
        times.push(t.elapsed().as_micros());
        update(&mut app, key(KeyCode::Down));
    }
    times.sort_unstable();
    (times[FRAMES / 2], times[FRAMES * 95 / 100])
}

/// Task 60 step 0: cost of one `SearchIndex::search` call as run per keystroke.
fn search_scenario(app: &App, query: &str) -> (u128, u128) {
    let mut times = Vec::with_capacity(SEARCH_RUNS);
    for _ in 0..SEARCH_RUNS {
        let t = Instant::now();
        let hits = app.search.search(query, None, 1000);
        times.push(t.elapsed().as_micros());
        std::hint::black_box(hits);
    }
    times.sort_unstable();
    (times[SEARCH_RUNS / 2], times[SEARCH_RUNS * 95 / 100])
}

/// UI-thread cost of typing `c` and erasing it (Task 60: the search itself runs elsewhere).
fn keystroke_scenario(app: &mut App) -> (u128, u128) {
    let mut times = Vec::with_capacity(SEARCH_RUNS);
    for _ in 0..SEARCH_RUNS {
        let t = Instant::now();
        let cmds = update(app, key(KeyCode::Char('c')));
        times.push(t.elapsed().as_micros());
        std::hint::black_box(cmds);
        update(app, key(KeyCode::Backspace));
    }
    times.sort_unstable();
    (times[SEARCH_RUNS / 2], times[SEARCH_RUNS * 95 / 100])
}

fn main() {
    let data = DataDir::from_env().expect("data dir");
    let entries = Catalog::open_read_only(&data.catalog_path())
        .expect("open catalog")
        .load_entries()
        .expect("load entries");
    println!("entries: {}", entries.len());
    for (name, tabs, q) in [
        ("search", 0, ""),
        ("search 'code'", 0, "code"),
        ("installed", 1, ""),
        ("updates", 2, ""),
    ] {
        let (med, p95) = scenario(&entries, tabs, q);
        println!("{name:<14} median {med:>6} us   p95 {p95:>6} us");
    }
    let mut app = App::new(entries, GlyphSet::Nerd);
    for q in ["c", "co", "cod", "code", "vscode"] {
        let (med, p95) = search_scenario(&app, q);
        println!("busca {q:<8} median {med:>6} us   p95 {p95:>6} us");
    }
    let (med, p95) = keystroke_scenario(&mut app);
    println!("tecla 'c'      median {med:>6} us   p95 {p95:>6} us");
}
