#![allow(clippy::unwrap_used, clippy::expect_used)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures_util::StreamExt;
use tokio::time::MissedTickBehavior;
use wingetrs::tui::app::{Cmd, Msg};
use wingetrs::tui::run::{
    drain_ready, join_details, last_details_only, needs_draw, spinner_ticker,
};

fn key(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

fn details(id: &str) -> Cmd {
    Cmd::LoadDetails {
        source: "scoop:main".into(),
        id: id.into(),
    }
}

#[tokio::test]
async fn spinner_ticker_skips_missed_ticks() {
    assert_eq!(
        spinner_ticker().missed_tick_behavior(),
        MissedTickBehavior::Skip
    );
}

#[test]
fn drain_ready_takes_what_is_queued_up_to_the_cap() {
    let mut s = futures_util::stream::iter((0..4).map(|_| key(KeyCode::Down)))
        .chain(futures_util::stream::pending());
    assert_eq!(drain_ready(key(KeyCode::Down), &mut s, 256).len(), 5);
    let mut s = futures_util::stream::iter((0..4).map(|_| key(KeyCode::Down)))
        .chain(futures_util::stream::pending());
    assert_eq!(drain_ready(key(KeyCode::Down), &mut s, 3).len(), 3);
    let mut s = futures_util::stream::pending();
    assert_eq!(drain_ready(key(KeyCode::Down), &mut s, 256).len(), 1);
}

#[test]
fn last_details_only_keeps_the_newest_request() {
    let cmds = vec![
        details("a"),
        Cmd::Refresh { force: true },
        details("b"),
        Cmd::ScanInstalled,
    ];
    assert_eq!(
        last_details_only(cmds),
        vec![
            Cmd::Refresh { force: true },
            details("b"),
            Cmd::ScanInstalled
        ]
    );
}

#[test]
fn idle_ticks_do_not_draw() {
    assert!(!needs_draw(&[Msg::Tick, Msg::Tick], false));
    assert!(needs_draw(&[Msg::Tick], true));
    assert!(needs_draw(&[key(KeyCode::Down)], false));
}

#[tokio::test]
async fn details_worker_panic_reaches_the_loop() {
    let msg = join_details("k".into(), tokio::spawn(async { panic!("x") })).await;
    assert!(matches!(msg, Msg::Panicked(_)));
    let h = tokio::spawn(std::future::pending::<Msg>());
    h.abort();
    let msg = join_details("k".into(), h).await;
    assert!(matches!(&msg, Msg::DetailsLoaded { key, result: Err(_) } if key == "k"));
}
