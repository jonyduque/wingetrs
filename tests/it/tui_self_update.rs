//! Task 214: the TUI's notice of a new wingetrs version and Ctrl+U.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::path::Path;
use wingetrs::http::Http;
use wingetrs::selfupdate::{self, CHECK_EVERY};
use wingetrs::tui::app::{
    Action, App, Cmd, Done, InstallFail, Modal, Msg, Running, ToastKind, update,
};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Theme};
use wingetrs::tui::view::view;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const NOW: i64 = 1_800_000_000;
const NEW: &str = "v99.0.0";

fn ctrl_u() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL))
}

fn enter() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
}

fn esc() -> Msg {
    Msg::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
}

/// A server whose `releases/latest` redirects to the tag page of `tag`.
async fn github(tag: &str) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/jonyduque/wingetrs/releases/latest"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "Location",
            format!("/jonyduque/wingetrs/releases/tag/{tag}"),
        ))
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/jonyduque/wingetrs/releases/tag/{tag}")))
        .respond_with(ResponseTemplate::new(200))
        .mount(&s)
        .await;
    s
}

fn write_cache(dir: &Path, checked_at: i64, latest: &str) {
    std::fs::write(
        dir.join("self-update.json"),
        format!("{{\"checked_at\":{checked_at},\"latest\":\"{latest}\"}}"),
    )
    .unwrap();
}

fn with_update() -> App {
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    update(&mut a, Msg::SelfUpdateAvailable { tag: NEW.into() });
    a
}

fn render(app: &App) -> String {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut t = Terminal::new(TestBackend::new(100, 20)).unwrap();
    t.draw(|f| view(app, &theme, f)).unwrap();
    format!("{}", t.backend())
}

#[tokio::test]
async fn check_skipped_within_24h() {
    let tmp = tempfile::tempdir().unwrap();
    let s = github("v7.0.0").await;
    write_cache(tmp.path(), NOW - CHECK_EVERY + 1, NEW);
    let h = Http::new().unwrap();
    let tag = selfupdate::check_latest(&h, &s.uri(), tmp.path(), NOW).await;
    assert_eq!(
        tag.as_deref(),
        Some(NEW),
        "the cached tag, not the server's"
    );
    assert!(s.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn check_refreshes_after_24h() {
    let tmp = tempfile::tempdir().unwrap();
    let s = github("v7.0.0").await;
    write_cache(tmp.path(), NOW - CHECK_EVERY, NEW);
    let h = Http::new().unwrap();
    let tag = selfupdate::check_latest(&h, &s.uri(), tmp.path(), NOW).await;
    assert_eq!(tag.as_deref(), Some("v7.0.0"));
    assert!(!s.received_requests().await.unwrap().is_empty());
    let cache = std::fs::read_to_string(tmp.path().join("self-update.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&cache).unwrap();
    assert_eq!(v["latest"], "v7.0.0");
    assert_eq!(v["checked_at"], NOW);
    // A cache from the future, or with a bad tag, is stale too.
    for (at, tag) in [(NOW + 5, NEW), (NOW - 1, "latest")] {
        write_cache(tmp.path(), at, tag);
        let got = selfupdate::check_latest(&h, &s.uri(), tmp.path(), NOW).await;
        assert_eq!(got.as_deref(), Some("v7.0.0"), "{at} {tag}");
    }
}

#[tokio::test]
async fn offline_check_is_silent() {
    let tmp = tempfile::tempdir().unwrap();
    let h = Http::new().unwrap();
    // Nothing listens there: offline.
    let tag = selfupdate::check_latest(&h, "http://127.0.0.1:1", tmp.path(), NOW).await;
    assert_eq!(tag, None);
    assert!(!tmp.path().join("self-update.json").exists());
    // The limit of GitHub and a server error are silent as well.
    let s = MockServer::start().await;
    for code in [403, 500] {
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(code))
            .mount(&s)
            .await;
        assert_eq!(
            selfupdate::check_latest(&h, &s.uri(), tmp.path(), NOW).await,
            None,
            "{code}"
        );
    }
    // Nothing was sent to the app: no pill, no toast, no dialog.
    let a = App::new(Vec::new(), GlyphSet::Ascii);
    assert!(a.self_update.is_none() && a.toasts.is_empty() && a.modal.is_none());
    assert!(!render(&a).contains("ctrl+u"));
}

#[test]
fn pill_shows_new_version() {
    let a = with_update();
    let out = render(&a);
    assert!(out.contains("wingetrs 99.0.0 · ctrl+u"), "{out}");
    // Not newer than this build: no pill.
    let mut b = App::new(Vec::new(), GlyphSet::Ascii);
    let same = format!("v{}", b.version);
    for tag in ["v0.0.1", same.as_str(), "latest"] {
        update(&mut b, Msg::SelfUpdateAvailable { tag: tag.into() });
        assert!(b.self_update.is_none(), "{tag}");
    }
    assert!(!render(&b).contains("ctrl+u"));
}

#[test]
fn ctrl_u_without_update_does_nothing() {
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    let cmds = update(&mut a, ctrl_u());
    assert!(cmds.is_empty());
    assert!(a.modal.is_none() && a.toasts.is_empty());
}

#[test]
fn ctrl_u_opens_confirm_and_runs_update() {
    let mut a = with_update();
    let cmds = update(&mut a, ctrl_u());
    assert!(cmds.is_empty(), "nothing runs before Continuar: {cmds:?}");
    let Some(Modal::Confirm { notes, .. }) = &a.modal else {
        panic!("expected the confirmation");
    };
    assert_eq!(
        notes[0],
        format!("Atualizar o wingetrs de {} para 99.0.0?", a.version)
    );
    assert!(render(&a).contains("Continuar (enter)"));
    // Cancelar: nothing runs, the pill stays.
    assert!(update(&mut a, esc()).is_empty());
    assert!(a.modal.is_none() && a.self_update.is_some());
    // Continuar: the update, once.
    update(&mut a, ctrl_u());
    let cmds = update(&mut a, enter());
    let updates: Vec<_> = cmds
        .iter()
        .filter(|c| matches!(c, Cmd::Execute(Action::SelfUpdate { tag }) if tag == NEW))
        .collect();
    assert_eq!(updates.len(), 1, "{cmds:?}");
    assert!(a.running.is_some());
    // Success: the toast and no pill.
    let done = Done {
        summary: "wingetrs atualizado para 99.0.0; feche e abra de novo para usar a versão nova"
            .into(),
        residues: None,
    };
    update(&mut a, Msg::InstallFinished(Ok(done)));
    assert!(a.self_update.is_none() && a.running.is_none());
    assert!(
        a.toasts.iter().any(|t| t.kind == ToastKind::Success
            && t.text.starts_with("wingetrs atualizado para 99.0.0"))
    );
}

#[test]
fn failed_update_keeps_pill_and_shows_error() {
    let mut a = with_update();
    update(&mut a, ctrl_u());
    update(&mut a, enter());
    update(
        &mut a,
        Msg::InstallFinished(Err(InstallFail::Other("sem conexão com o GitHub".into()))),
    );
    assert!(a.self_update.is_some());
    assert!(
        a.toasts
            .iter()
            .any(|t| t.kind == ToastKind::Error && t.text == "sem conexão com o GitHub")
    );
}

#[test]
fn ctrl_u_while_busy_is_refused() {
    let mut a = with_update();
    a.running = Some(Running {
        id: 1,
        label: "Instalando x".into(),
        progress: None,
        action: None,
    });
    let cmds = update(&mut a, ctrl_u());
    assert!(cmds.is_empty() && a.modal.is_none());
    assert!(a.toasts.iter().any(|t| t.text == "espere a operação atual"));
}
