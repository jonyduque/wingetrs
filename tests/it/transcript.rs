#![allow(clippy::unwrap_used, clippy::expect_used)]
use clap::Parser;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;
use wingetrs::cli::picker::{self, fake};
use wingetrs::cli::{Cli, emit, output, style, transcript_path};
use wingetrs::install::progress::Phase;
use wingetrs::transcript;
use wingetrs::tui::app::{
    App, ConfirmItem, Modal, Msg, RunProgress, Running, ScriptSection, ToastKind, update,
};
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Glyphs, Theme};
use wingetrs::tui::transcript::Observer;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Env {
    _server: MockServer,
    dir: tempfile::TempDir,
    uri: String,
}

impl Env {
    async fn new() -> Env {
        let s = MockServer::start().await;
        for (p, body) in [
            (
                "/source2.msix",
                include_bytes!("../fixtures/winget/source2-mini.msix").to_vec(),
            ),
            (
                "/main.tgz",
                include_bytes!("../fixtures/scoop/Main-mini.tar.gz").to_vec(),
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
                .mount(&s)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/extras.tgz"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&s)
            .await;
        let env = Env {
            uri: s.uri(),
            _server: s,
            dir: tempfile::tempdir().unwrap(),
        };
        let out = tokio::task::block_in_place(|| env.run(&["update", "--force"], &[]));
        assert!(out.status.success());
        env
    }

    fn run(&self, args: &[&str], vars: &[(&str, &Path)]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_wingetrs"));
        c.args(args)
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove(transcript::VAR)
            .env("LOCALAPPDATA", self.dir.path())
            .env(
                "WINGETRS_WINGET_INDEX_URL",
                format!("{}/source2.msix", self.uri),
            )
            .env("WINGETRS_SCOOP_BASE_URL", &self.uri);
        for (k, v) in vars {
            c.env(k, v);
        }
        c.output().unwrap()
    }
}

#[test]
fn emit_lines_are_plain() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.txt");
    transcript::with_file(&file, || {
        style::with_enabled(true, || {
            emit::out(&output::success("x", "y"));
            emit::out_block("a\nb\n");
            emit::err("e");
        });
        // Read before leaving `with_file`: each line is flushed as it is written.
        let text = std::fs::read_to_string(&file).unwrap();
        let ok = Glyphs::for_set(GlyphSet::Unicode).toast_ok;
        assert_eq!(text, format!("{ok} x y\na\nb\ne\n"));
        assert!(!text.contains('\x1b') && !text.contains('\r'), "{text:?}");
    });
}

#[test]
fn strip_ansi_drops_csi_and_osc() {
    assert_eq!(
        transcript::strip_ansi("\x1b]8;;http://x\x1b\\texto\x1b]8;;\x07\x1b[1;32mok\x1b[0m\r"),
        "textook"
    );
}

#[test]
fn appends_to_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.txt");
    std::fs::write(&file, "antes\n").unwrap();
    transcript::with_file(&file, || transcript::line("novo"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "antes\nnovo\n");
}

#[test]
fn invalid_path_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("nao/existe/t.txt");
    assert!(transcript::open(&bad).is_err());
    transcript::line("x");
    assert!(!transcript::enabled());
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_warns_once_and_runs() {
    let e = Env::new().await;
    let args = ["search", "", "-n", "100"];
    let base = tokio::task::block_in_place(|| e.run(&args, &[]));
    assert!(base.status.success());
    let bad = e.dir.path().join("pasta-inexistente").join("t.txt");
    let out = tokio::task::block_in_place(|| e.run(&args, &[(transcript::VAR, &bad)]));
    assert!(out.status.success());
    assert_eq!(out.stdout, base.stdout);
    let err = String::from_utf8(out.stderr).unwrap();
    assert_eq!(
        err.matches("não foi possível abrir o transcript ").count(),
        1,
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_transcript_has_stdout_stderr_and_phases() {
    let e = Env::new().await;
    let file = e.dir.path().join("t.txt");
    let out = tokio::task::block_in_place(|| {
        e.run(
            &["update", "--force", "--color", "always"],
            &[(transcript::VAR, &file)],
        )
    });
    assert!(out.status.success());
    let text = std::fs::read(&file).unwrap();
    assert!(
        !text.contains(&0x1b),
        "{:?}",
        String::from_utf8_lossy(&text)
    );
    let text = String::from_utf8(text).unwrap();
    assert!(text.contains("winget: "), "{text}");
    assert!(text.contains("Preparando…"), "{text}");
    println!("{}", text.lines().take(10).collect::<Vec<_>>().join("\n"));
}

#[test]
fn tui_toast_goes_to_transcript() {
    let mut app = App::new(Vec::new(), GlyphSet::Unicode);
    let mut obs = Observer::default();
    app.push_toast(ToastKind::Success, "instalado");
    assert_eq!(obs.observe(&app), ["[tui] toast ok: instalado"]);
    assert!(obs.observe(&app).is_empty());
    app.push_toast(ToastKind::Success, "instalado");
    assert_eq!(obs.observe(&app), ["[tui] toast ok: instalado"]);
}

fn item(name: &str, from: Option<&str>, to: Option<&str>) -> ConfirmItem {
    ConfirmItem {
        name: name.into(),
        key: None,
        from: from.map(Into::into),
        to: to.map(Into::into),
        manager: "scoop",
        checked: true,
    }
}

#[test]
fn tui_confirm_modal_lines() {
    let mut app = App::new(Vec::new(), GlyphSet::Unicode);
    let mut obs = Observer::default();
    app.modal = Some(Modal::Confirm {
        items: vec![
            item("rust", Some("1.0"), Some("1.1")),
            item("git", None, None),
        ],
        notes: vec!["um aviso".into()],
        scroll: 0,
        cursor: 0,
        actions: Vec::new(),
        clear_marks: None,
        scripts: ScriptSection::default(),
    });
    assert_eq!(
        obs.observe(&app),
        [
            "[tui] modal: Confirmar",
            "[tui]   rust 1.0 → 1.1 (scoop)",
            "[tui]   git (scoop)",
            "[tui]   nota: um aviso",
        ]
    );
    assert!(obs.observe(&app).is_empty());
    app.modal = None;
    assert_eq!(obs.observe(&app), ["[tui] modal fechado"]);
}

#[test]
fn tui_job_lines_ignore_the_clock() {
    let mut app = App::new(Vec::new(), GlyphSet::Unicode);
    let mut obs = Observer::default();
    app.running = Some(Running {
        id: 1,
        label: "Instalar rust".into(),
        progress: Some(RunProgress {
            label: String::new(),
            phase: Some((Phase::Preparing, app.now)),
            meter: Default::default(),
        }),
        action: None,
    });
    assert_eq!(obs.observe(&app), ["[tui] job: Instalar rust · Preparando"]);
    app.now += Duration::from_secs(5);
    assert!(obs.observe(&app).is_empty());
    update(&mut app, Msg::InstallPhase(Phase::Placing));
    assert_eq!(
        obs.observe(&app),
        ["[tui] job: Instalar rust · Colocando os arquivos"]
    );
    app.running = None;
    assert_eq!(obs.observe(&app), ["[tui] job terminou: Instalar rust"]);
}

#[test]
fn elevated_worker_and_background_never_transcribe() {
    let var = || Some(OsString::from("t.txt"));
    let parse = |args: &[&str]| Cli::try_parse_from(args).unwrap();
    let worker = parse(&["wingetrs", "bridge-worker", "--pipe", "p", "--parent", "1"]);
    assert_eq!(transcript_path(&worker, var()), None);
    let background = parse(&["wingetrs", "update", "--background"]);
    assert_eq!(transcript_path(&background, var()), None);
    let apply = parse(&["wingetrs", "self", "apply-elevated"]);
    assert_eq!(transcript_path(&apply, var()), None);
    let list = parse(&["wingetrs", "list"]);
    assert_eq!(transcript_path(&list, var()), Some("t.txt".into()));
    assert_eq!(transcript_path(&list, Some(OsString::new())), None);
}

#[test]
fn picker_summary_in_transcript() {
    use crossterm::event::KeyCode::Enter;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.txt");
    let mut term = fake::ScriptedTerm::new(vec![fake::key(Enter)]);
    let theme = Theme::new(true, ColorDepth::NoColor);
    let glyphs = Glyphs::for_set(GlyphSet::Ascii);
    transcript::with_file(&file, || {
        let yes = picker::yes_no(&mut term, "Continuar?", &theme, glyphs).unwrap();
        assert!(!yes);
    });
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Continuar? Não\n");
}
