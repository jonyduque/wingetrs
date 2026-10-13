#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Frame-cost benchmark for the TUI list (Task 59). Needs the real catalog on disk.
//! Run: cargo run --release --example tui_frame
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::sync::Arc;
use std::time::{Duration, Instant};
use wingetrs::cache::DataDir;
use wingetrs::catalog::Catalog;
use wingetrs::catalog::search::SearchIndex;
use wingetrs::core::PackageEntry;
use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
use wingetrs::tui::app::{App, Cmd, Msg, run_search, update};
use wingetrs::tui::icons::IconProtocol;
use wingetrs::tui::layout::LIST_ICON_COLS;
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

/// A red icon that only reserves its cells: the real terminal is not measured here.
fn reserve_icon() -> ratatui_image::protocol::Protocol {
    let picker = ratatui_image::picker::Picker::halfblocks();
    let fs = picker.font_size();
    let img = image::RgbaImage::from_pixel(
        u32::from(LIST_ICON_COLS * fs.width),
        u32::from(2 * fs.height),
        image::Rgba([255, 0, 0, 255]),
    );
    picker
        .new_protocol(
            image::DynamicImage::ImageRgba8(img),
            ratatui::layout::Size::new(LIST_ICON_COLS, 2),
            ratatui_image::Resize::Fit(None),
        )
        .unwrap()
}

/// Median and p95 microseconds of `FRAMES` frames; with `icons` the protocol is on and
/// every package of the first screens has a cached (pre-encoded) list icon.
fn scenario(entries: &[PackageEntry], tabs: usize, query: &str, icons: bool) -> (u128, u128) {
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
    if icons {
        app.icon_protocol = IconProtocol::Sixel;
        let size = (LIST_ICON_COLS, 2);
        for (source, id) in app.list_keys(0..FRAMES + 40).into_iter().flatten() {
            app.icons
                .insert(format!("{source}/{id}"), size, reserve_icon());
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

const BURST: &str = "code";
const KEY_GAP: Duration = Duration::from_millis(30);
const BURST_RUNS: usize = 15;

/// Before Task 60: every key searched on the UI thread, one after the other, as keys arrived
/// every `KEY_GAP`. Returns last key -> final hits.
fn burst_sync(app: &App) -> Duration {
    let mut end = Duration::ZERO;
    for (i, n) in (1..=BURST.len()).enumerate() {
        let arrival = KEY_GAP * u32::try_from(i).unwrap();
        let start = end.max(arrival);
        let t = Instant::now();
        std::hint::black_box(run_search(&app.search, &BURST[..n], None));
        end = start + t.elapsed();
    }
    end.saturating_sub(KEY_GAP * u32::try_from(BURST.len() - 1).unwrap())
}

/// Now: real keys every `KEY_GAP`, searches on a worker thread, as the event loop runs them.
fn burst_async(entries: &[PackageEntry]) -> Duration {
    let mut app = App::new(entries.to_vec(), GlyphSet::Nerd);
    let (to_worker, jobs) = std::sync::mpsc::channel::<(Arc<SearchIndex>, Cmd)>();
    let (to_ui, done) = std::sync::mpsc::channel::<Msg>();
    let worker = std::thread::spawn(move || {
        for (index, cmd) in jobs {
            if let Cmd::Search {
                generation,
                query,
                kind,
            } = cmd
            {
                let hits = run_search(&index, &query, kind);
                let _ = to_ui.send(Msg::SearchDone { generation, hits });
            }
        }
    });
    let send = |app: &App, cmds: Vec<Cmd>| {
        for c in cmds {
            if matches!(c, Cmd::Search { .. }) {
                to_worker.send((Arc::clone(&app.search), c)).unwrap();
            }
        }
    };
    let t0 = Instant::now();
    let mut sent = 0;
    let mut last_key = t0;
    let latency = loop {
        if sent < BURST.len() {
            let due = t0 + KEY_GAP * u32::try_from(sent).unwrap();
            match done.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(m) => {
                    let cmds = update(&mut app, m);
                    send(&app, cmds);
                }
                Err(_) => {
                    let c = BURST[sent..].chars().next().unwrap();
                    let cmds = update(&mut app, key(KeyCode::Char(c)));
                    send(&app, cmds);
                    sent += 1;
                    last_key = Instant::now();
                }
            }
        } else {
            let cmds = update(&mut app, done.recv().unwrap());
            let more = cmds.iter().any(|c| matches!(c, Cmd::Search { .. }));
            send(&app, cmds);
            if !more && app.query == BURST {
                break last_key.elapsed();
            }
        }
    };
    drop(to_worker);
    worker
        .join()
        .unwrap_or_else(|p| std::panic::resume_unwind(p));
    assert_eq!(app.hits, run_search(&app.search, BURST, None));
    latency
}

fn report_burst(name: &str, mut times: Vec<u128>) {
    times.sort_unstable();
    println!(
        "{name:<14} median {:>6} us   max {:>6} us",
        times[times.len() / 2],
        times[times.len() - 1]
    );
}

fn main() {
    let data = DataDir::from_env().expect("data dir");
    let entries = Catalog::open_read_only(&data.catalog_path())
        .expect("open catalog")
        .load_entries()
        .expect("load entries");
    println!("entries: {}", entries.len());
    if std::env::args().any(|a| a == "frames") {
        // Task 85: 3 series of FRAMES frames on the Search tab, icons off then on.
        for icons in [false, true] {
            for _ in 0..3 {
                let (med, p95) = scenario(&entries, 0, "", icons);
                println!(
                    "pesquisar icones {:<4} median {med:>6} us   p95 {p95:>6} us",
                    if icons { "on" } else { "off" }
                );
            }
        }
        return;
    }
    for (name, tabs, q) in [
        ("search", 0, ""),
        ("search 'code'", 0, "code"),
        ("installed", 1, ""),
        ("updates", 2, ""),
    ] {
        let (med, p95) = scenario(&entries, tabs, q, false);
        println!("{name:<14} median {med:>6} us   p95 {p95:>6} us");
    }
    let mut app = App::new(entries, GlyphSet::Nerd);
    for q in ["c", "co", "cod", "code", "vscode"] {
        let (med, p95) = search_scenario(&app, q);
        println!("busca {q:<8} median {med:>6} us   p95 {p95:>6} us");
    }
    let (med, p95) = keystroke_scenario(&mut app);
    println!("tecla 'c'      median {med:>6} us   p95 {p95:>6} us");
    let sync: Vec<u128> = (0..BURST_RUNS)
        .map(|_| burst_sync(&app).as_micros())
        .collect();
    report_burst("rajada sync", sync);
    let entries = app.search.entries().to_vec();
    let asy: Vec<u128> = (0..BURST_RUNS)
        .map(|_| burst_async(&entries).as_micros())
        .collect();
    report_burst("rajada async", asy);
}
