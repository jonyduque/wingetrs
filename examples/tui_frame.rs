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
use wingetrs::tui::app::{App, Msg, update};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Theme};
use wingetrs::tui::view::view;

const FRAMES: usize = 200;

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
        update(&mut app, key(KeyCode::Char(c)));
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
}
