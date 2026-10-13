#![allow(clippy::unwrap_used, clippy::expect_used)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures_util::StreamExt;
use tokio::time::MissedTickBehavior;
use wingetrs::tui::app::{Cmd, Msg};
use wingetrs::tui::run::{
    clock_turn, drain_ready, join_details, join_refresh, join_search, last_details_only,
    needs_draw, spinner_ticker,
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

#[tokio::test]
async fn drain_ready_takes_what_is_queued_up_to_the_cap() {
    let mut s = futures_util::stream::iter((0..4).map(|_| key(KeyCode::Down)))
        .chain(futures_util::stream::pending());
    assert_eq!(drain_ready(key(KeyCode::Down), &mut s, 256).await.len(), 5);
    let mut s = futures_util::stream::iter((0..4).map(|_| key(KeyCode::Down)))
        .chain(futures_util::stream::pending());
    assert_eq!(drain_ready(key(KeyCode::Down), &mut s, 3).await.len(), 3);
    let mut s = futures_util::stream::pending();
    assert_eq!(drain_ready(key(KeyCode::Down), &mut s, 256).await.len(), 1);
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
    assert!(matches!(&msg, Msg::DetailsLoaded { key, result: Err(_), .. } if key == "k"));
}

/// Always pending; remembers the waker of its last poll, like crossterm's `EventStream`.
struct RecordsWaker(Option<std::task::Waker>);

impl futures_util::Stream for RecordsWaker {
    type Item = Msg;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Msg>> {
        self.0 = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}

#[tokio::test]
async fn drain_ready_leaves_the_task_waker_with_a_pending_stream() {
    let task = std::future::poll_fn(|cx| std::task::Poll::Ready(cx.waker().clone())).await;
    let mut s = RecordsWaker(None);
    assert_eq!(drain_ready(key(KeyCode::Down), &mut s, 256).await.len(), 1);
    let recorded = s.0.expect("the stream was polled");
    assert!(recorded.will_wake(&task));
}

#[tokio::test]
async fn refresh_worker_panic_reaches_the_loop() {
    let msg = join_refresh(tokio::spawn(async { panic!("x") })).await;
    assert!(matches!(msg, Msg::Panicked(_)));
    let h = tokio::spawn(std::future::pending::<Msg>());
    h.abort();
    let msg = join_refresh(h).await;
    assert!(matches!(msg, Msg::RefreshFinished(r) if r.len() == 1));
}

#[tokio::test]
async fn search_worker_panic_reaches_the_loop() {
    let msg = join_search(7, tokio::spawn(async { panic!("x") })).await;
    assert!(matches!(msg, Msg::Panicked(_)));
    let h = tokio::spawn(std::future::pending::<Msg>());
    h.abort();
    assert!(matches!(
        join_search(7, h).await,
        Msg::SearchCancelled { generation: 7 }
    ));
    let ok = tokio::spawn(async {
        Msg::SearchDone {
            generation: 7,
            hits: vec![],
        }
    });
    assert!(matches!(
        join_search(7, ok).await,
        Msg::SearchDone { generation: 7, .. }
    ));
}

#[test]
fn clock_message_draws() {
    assert!(needs_draw(&[Msg::Clock], false));
}

#[test]
fn clock_turn_sets_the_clock_so_toasts_expire() {
    use std::time::Duration;
    use wingetrs::tui::app::{App, ToastKind};
    use wingetrs::tui::theme::GlyphSet;
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    let t0 = a.now;
    a.push_toast(ToastKind::Success, "ok");
    clock_turn(&mut a, t0 + Duration::from_millis(2999), vec![Msg::Clock]);
    assert_eq!(a.toasts.len(), 1);
    clock_turn(&mut a, t0 + Duration::from_millis(3001), vec![Msg::Clock]);
    assert!(a.toasts.is_empty());
}
