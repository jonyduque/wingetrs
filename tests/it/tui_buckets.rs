#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Task 207: scoop bucket management in the TUI.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use flate2::{Compression, write::GzEncoder};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::io::Write;
use wingetrs::buckets::BucketRow;
use wingetrs::cache::{DataDir, UpdateLock};
use wingetrs::config::Config;
use wingetrs::http::Http;
use wingetrs::tui::app::{
    Action, App, BucketInfo, BucketStage, Cmd, Done, InstallFail, Modal, Msg, Tab, ToastKind,
    update,
};
use wingetrs::tui::jobs;
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Theme};
use wingetrs::tui::view::view;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn key(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

fn typed(a: &mut App, text: &str) {
    for c in text.chars() {
        update(a, key(KeyCode::Char(c)));
    }
}

fn render(app: &App, w: u16, h: u16) -> String {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| view(app, &theme, f)).unwrap();
    format!("{}", t.backend())
}

fn info(name: &str, repo: &str, installed: Result<Vec<String>, String>) -> BucketInfo {
    BucketInfo {
        row: BucketRow {
            name: name.into(),
            repo: repo.into(),
            apps: Some(1234),
            updated: Some(1_700_000_000),
        },
        age: Some(3 * 86_400),
        installed,
    }
}

fn cfg_with(extra: &[(&str, &str)]) -> Config {
    let mut cfg = Config::default();
    for (name, repo) in extra {
        wingetrs::buckets::add(&mut cfg, name, repo, true).unwrap();
    }
    cfg
}

/// Settings tab, `b` pressed and the list loaded with `main`, `extras` and `rows`.
fn dialog(extra: &[(&str, &str)], rows: Vec<BucketInfo>) -> App {
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.size = (110, 30);
    a.tab = Tab::Settings;
    let cmds = update(&mut a, key(KeyCode::Char('b')));
    assert_eq!(cmds, vec![Cmd::LoadBuckets]);
    assert!(matches!(a.modal, Some(Modal::Buckets(_))));
    let mut all = vec![
        info("main", "ScoopInstaller/Main", Ok(vec![])),
        info("extras", "ScoopInstaller/Extras", Ok(vec![])),
    ];
    all.extend(rows);
    update(&mut a, Msg::BucketsLoaded(Ok((cfg_with(extra), all))));
    a
}

fn stage(a: &App) -> &BucketStage {
    match &a.modal {
        Some(Modal::Buckets(d)) => &d.stage,
        _ => panic!("buckets dialog not open"),
    }
}

fn execute(cmds: &[Cmd]) -> Vec<&Action> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Execute(a) => Some(a),
            _ => None,
        })
        .collect()
}

#[test]
fn buckets_modal_lists_known_and_third_party() {
    let a = dialog(
        &[("mine", "someone/scoop-mine")],
        vec![info("mine", "someone/scoop-mine", Ok(vec![]))],
    );
    let screen = render(&a, 110, 30);
    let line = |name: &str| {
        screen
            .lines()
            .find(|l| l.contains(&format!(" {name} ")))
            .unwrap_or_else(|| panic!("no line for {name}:\n{screen}"))
            .to_string()
    };
    let main = line("main");
    assert!(main.contains("ScoopInstaller/Main"), "{main}");
    assert!(main.contains("1234 apps"), "{main}");
    assert!(main.contains("conhecido"), "{main}");
    assert!(!main.contains("de terceiro"), "{main}");
    let mine = line("mine");
    assert!(mine.contains("someone/scoop-mine"), "{mine}");
    assert!(mine.contains("de terceiro"), "{mine}");
    assert!(main.contains("há 3d"), "{main}");
    // The list is the dialog's first stage.
    assert!(matches!(stage(&a), BucketStage::List));
}

#[test]
fn add_known_runs_as_queued_operation() {
    let mut a = dialog(&[], vec![]);
    assert!(update(&mut a, key(KeyCode::Char('a'))).is_empty());
    let BucketStage::Add { picker, names } = stage(&a) else {
        panic!("add picker not open");
    };
    let labels: Vec<&str> = picker.items().iter().map(|i| i.label.as_str()).collect();
    // Known buckets not configured yet, then the free-form item.
    assert_eq!(labels.first(), Some(&"versions"));
    assert!(!labels.contains(&"main") && !labels.contains(&"extras"));
    assert_eq!(labels.last(), Some(&"Outro repositório do GitHub…"));
    assert_eq!(names.len() + 1, labels.len());
    let cmds = update(&mut a, key(KeyCode::Enter));
    let add = Action::BucketAdd {
        name: "versions".into(),
        spec: "ScoopInstaller/Versions".into(),
        confirmed: false,
    };
    assert_eq!(execute(&cmds), vec![&add]);
    let running = a.running.as_ref().expect("the add runs as the operation");
    assert_eq!(running.label, "Adicionando bucket versions");
    // Spinner, phase and clock from the first instant.
    assert!(running.progress.as_ref().is_some_and(|p| p.phase.is_some()));
    assert!(matches!(stage(&a), BucketStage::List));

    // With an operation in progress the next add waits in line.
    update(&mut a, key(KeyCode::Char('a')));
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(execute(&cmds).is_empty(), "{cmds:?}");
    assert_eq!(a.queue.len(), 1);
    assert_eq!(a.queue[0].label, "Adicionando bucket versions");
}

#[test]
fn add_third_party_asks_confirmation() {
    let mut a = dialog(&[], vec![]);
    update(&mut a, key(KeyCode::Char('a')));
    update(&mut a, key(KeyCode::End));
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
    assert!(matches!(stage(&a), BucketStage::Typing { .. }));
    // Invalid text: the error shows while typing and Enter does nothing.
    typed(&mut a, "gitlab.com:x/y");
    assert!(render(&a, 110, 30).contains("só buckets do GitHub"));
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
    assert!(matches!(stage(&a), BucketStage::Typing { .. }));
    for _ in 0.."gitlab.com:x/y".len() {
        update(&mut a, key(KeyCode::Backspace));
    }
    typed(&mut a, "https://github.com/Someone/My-Bucket.git");
    assert!(update(&mut a, key(KeyCode::Enter)).is_empty());
    let BucketStage::ThirdParty { name, repo } = stage(&a) else {
        panic!("third-party confirmation not shown");
    };
    assert_eq!(
        (name.as_str(), repo.as_str()),
        ("my-bucket", "Someone/My-Bucket")
    );
    assert!(a.running.is_none() && a.queue.is_empty());
    let screen = render(&a, 110, 30);
    assert!(
        screen.contains("bucket de terceiro: código e binários de terceiros"),
        "{screen}"
    );
    // Esc goes back without running anything.
    assert!(update(&mut a, key(KeyCode::Esc)).is_empty());
    assert!(matches!(stage(&a), BucketStage::List));
    assert!(a.running.is_none() && a.queue.is_empty());

    update(&mut a, key(KeyCode::Char('a')));
    update(&mut a, key(KeyCode::End));
    update(&mut a, key(KeyCode::Enter));
    typed(&mut a, "Someone/My-Bucket");
    update(&mut a, key(KeyCode::Enter));
    let cmds = update(&mut a, key(KeyCode::Enter));
    let add = Action::BucketAdd {
        name: "my-bucket".into(),
        spec: "Someone/My-Bucket".into(),
        confirmed: true,
    };
    assert_eq!(execute(&cmds), vec![&add]);
}

#[test]
fn remove_with_installed_apps_refused_with_names() {
    let mut a = dialog(
        &[("mine", "someone/scoop-mine"), ("odd", "someone/odd")],
        vec![
            info(
                "mine",
                "someone/scoop-mine",
                Ok(vec!["7zip".into(), "git".into()]),
            ),
            info("odd", "someone/odd", Err("state.db ilegível".into())),
        ],
    );
    // mine: third row.
    update(&mut a, key(KeyCode::Down));
    update(&mut a, key(KeyCode::Down));
    assert!(update(&mut a, key(KeyCode::Char('d'))).is_empty());
    let BucketStage::InUse { name, apps } = stage(&a) else {
        panic!("refusal not shown");
    };
    assert_eq!(name, "mine");
    assert_eq!(apps, &vec!["7zip".to_string(), "git".to_string()]);
    let screen = render(&a, 110, 30);
    assert!(screen.contains("7zip, git"), "{screen}");
    // Nothing goes on from the refusal.
    let cmds = update(&mut a, key(KeyCode::Enter));
    assert!(execute(&cmds).is_empty(), "{cmds:?}");
    assert!(a.running.is_none() && a.queue.is_empty());
    assert!(matches!(stage(&a), BucketStage::List));

    // An unreadable state.db refuses too (fail closed).
    update(&mut a, key(KeyCode::Down));
    assert!(update(&mut a, key(KeyCode::Delete)).is_empty());
    assert!(matches!(stage(&a), BucketStage::InUse { name, .. } if name == "odd"));
    assert!(render(&a, 110, 30).contains("state.db ilegível"));
    update(&mut a, key(KeyCode::Esc));

    // Without installed apps: one confirmation, then the queue.
    let mut b = dialog(
        &[("free", "someone/free")],
        vec![info("free", "someone/free", Ok(vec![]))],
    );
    update(&mut b, key(KeyCode::End));
    update(&mut b, key(KeyCode::Char('d')));
    assert!(matches!(stage(&b), BucketStage::Remove { name } if name == "free"));
    let cmds = update(&mut b, key(KeyCode::Enter));
    let rm = Action::BucketRemove {
        name: "free".into(),
        confirmed: false,
    };
    assert_eq!(execute(&cmds), vec![&rm]);

    // A default bucket asks twice.
    let mut c = dialog(&[], vec![]);
    update(&mut c, key(KeyCode::Char('d')));
    assert!(matches!(stage(&c), BucketStage::Remove { name } if name == "main"));
    assert!(update(&mut c, key(KeyCode::Enter)).is_empty());
    assert!(matches!(stage(&c), BucketStage::RemoveDefault { name } if name == "main"));
    assert!(render(&c, 110, 30).contains("bucket padrão"));
    let cmds = update(&mut c, key(KeyCode::Enter));
    let rm = Action::BucketRemove {
        name: "main".into(),
        confirmed: true,
    };
    assert_eq!(execute(&cmds), vec![&rm]);
}

#[test]
fn summary_line_updates_after_add() {
    let mut a = dialog(&[], vec![]);
    a.buckets_summary = "main, extras".into();
    update(&mut a, key(KeyCode::Char('a')));
    update(&mut a, key(KeyCode::Enter));
    let cmds = update(
        &mut a,
        Msg::InstallFinished(Ok(Done {
            summary: "Bucket adicionado: versions".into(),
            residues: None,
        })),
    );
    // The dialog, the summary line and the search index are read again.
    assert!(cmds.contains(&Cmd::LoadBuckets), "{cmds:?}");
    assert!(cmds.contains(&Cmd::ReloadCatalog), "{cmds:?}");
    update(
        &mut a,
        Msg::BucketsLoaded(Ok((
            cfg_with(&[("versions", "ScoopInstaller/Versions")]),
            vec![
                info("main", "ScoopInstaller/Main", Ok(vec![])),
                info("extras", "ScoopInstaller/Extras", Ok(vec![])),
                info("versions", "ScoopInstaller/Versions", Ok(vec![])),
            ],
        ))),
    );
    assert_eq!(a.buckets_summary, "main, extras, versions");
    update(&mut a, key(KeyCode::Esc));
    assert!(a.modal.is_none());
    let screen = render(&a, 110, 40);
    assert!(
        screen.contains("Buckets do scoop  main, extras, versions"),
        "{screen}"
    );

    // A failed add reloads nothing.
    update(&mut a, key(KeyCode::Char('b')));
    let cmds = update(
        &mut a,
        Msg::InstallFinished(Err(InstallFail::Other("x".into()))),
    );
    assert!(!cmds.contains(&Cmd::ReloadCatalog), "{cmds:?}");
}

#[test]
fn busy_lock_requeues() {
    let mut a = dialog(&[], vec![]);
    update(&mut a, key(KeyCode::Char('a')));
    let cmds = update(&mut a, key(KeyCode::Enter));
    let add = Action::BucketAdd {
        name: "versions".into(),
        spec: "ScoopInstaller/Versions".into(),
        confirmed: false,
    };
    assert_eq!(execute(&cmds), vec![&add]);
    let cmds = update(&mut a, Msg::InstallFinished(Err(InstallFail::Busy)));
    assert!(cmds.contains(&Cmd::RetryQueue), "{cmds:?}");
    assert!(!cmds.contains(&Cmd::ReloadCatalog), "{cmds:?}");
    assert!(a.running.is_none());
    assert!(a.waiting_lock);
    assert_eq!(a.queue.front().map(|q| &q.action), Some(&add));
    assert!(
        a.toasts
            .iter()
            .any(|t| t.kind == ToastKind::Warning
                && t.text.contains("atualização está em andamento")),
        "{:?}",
        a.toasts
    );
    let cmds = update(&mut a, Msg::RetryQueue);
    assert_eq!(execute(&cmds), vec![&add]);
}

#[tokio::test]
async fn busy_lock_job_reports_busy() {
    let d = tempfile::tempdir().unwrap();
    let data = DataDir::new(d.path());
    Config::default().save(&data.config_path()).unwrap();
    let _held = UpdateLock::try_acquire(&data.lock_path()).unwrap().unwrap();
    let http = Http::new().unwrap();
    let r = jobs::bucket_add(
        &data,
        &http,
        "mine",
        "someone/scoop-mine",
        true,
        Some("http://127.0.0.1:9/never.tgz"),
        &|_| {},
    )
    .await;
    assert_eq!(r.unwrap_err(), InstallFail::Busy);
    assert_eq!(
        jobs::bucket_remove(&data, "extras", true).unwrap_err(),
        InstallFail::Busy
    );
    // Nothing was saved while the lock was held.
    assert_eq!(
        Config::load(&data.config_path()).unwrap(),
        Config::default()
    );
}

fn bucket_tgz() -> Vec<u8> {
    let files: [(&str, &[u8]); 2] = [
        (
            "mine-master/bucket/zebra.json",
            br#"{"version":"1.0","description":"d","homepage":"https://x.io"}"#,
        ),
        (
            "mine-master/bucket/okapi.json",
            br#"{"version":"2.0","description":"d","homepage":"https://x.io"}"#,
        ),
    ];
    let mut b = tar::Builder::new(Vec::new());
    for (name, data) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, name, data).unwrap();
    }
    let mut gz = GzEncoder::new(Vec::new(), Compression::fast());
    gz.write_all(&b.into_inner().unwrap()).unwrap();
    gz.finish().unwrap()
}

struct NoTty;

impl wingetrs::cli::install_cmd::Ui for NoTty {
    fn interactive(&self) -> bool {
        false
    }
    fn confirm(&self, _: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    fn pick(&self, _: Vec<wingetrs::tui::picker::PickerItem>) -> anyhow::Result<Option<usize>> {
        Ok(None)
    }
    fn pick_many(
        &self,
        _: Vec<wingetrs::tui::picker::PickerItem>,
        _: &[bool],
    ) -> anyhow::Result<Option<Vec<usize>>> {
        Ok(None)
    }
}

/// What a bucket operation leaves on disk, without the download times.
fn outcome(data: &DataDir) -> (String, Vec<BucketRow>, Vec<String>) {
    let cfg = std::fs::read_to_string(data.config_path()).unwrap();
    let rows = wingetrs::buckets::rows(data, &Config::load(&data.config_path()).unwrap())
        .unwrap()
        .into_iter()
        .map(|r| BucketRow { updated: None, ..r })
        .collect();
    let hits = wingetrs::catalog::index_file::search(&data.search_index_path(), "zebra", None, 10)
        .unwrap()
        .unwrap_or_default()
        .into_iter()
        .map(|h| format!("{h:?}"))
        .collect();
    (cfg, rows, hits)
}

/// The TUI job and the CLI's steps (`cli::bucket_add` + `add_downloaded`, then
/// `remove_and_cleanup_confirmed`) leave the same config, catalog and index.
#[tokio::test]
async fn tui_and_cli_bucket_ops_are_equivalent() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/mine.tgz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bucket_tgz()))
        .mount(&s)
        .await;
    let url = format!("{}/mine.tgz", s.uri());
    let http = Http::new().unwrap();
    let (cli_dir, tui_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (cli, tui) = (DataDir::new(cli_dir.path()), DataDir::new(tui_dir.path()));
    for d in [&cli, &tui] {
        Config::default().save(&d.config_path()).unwrap();
    }

    let mut cfg = Config::load(&cli.config_path()).unwrap();
    let spec = "https://github.com/someone/scoop-mine".to_string();
    assert!(
        wingetrs::cli::bucket_add(&NoTty, &mut cfg, "mine", Some(spec), true, &mut |_| {}).unwrap()
    );
    wingetrs::buckets::add_downloaded(&cli, &http, &cfg, "mine", Some(&url))
        .await
        .unwrap();
    let done = jobs::bucket_add(
        &tui,
        &http,
        "mine",
        "https://github.com/someone/scoop-mine",
        true,
        Some(&url),
        &|_| {},
    )
    .await
    .unwrap();
    assert!(done.summary.contains("mine"), "{done:?}");
    let added = outcome(&cli);
    assert_eq!(added, outcome(&tui));
    assert_eq!(added.1.len(), 3, "{added:?}");
    assert_eq!(added.2.len(), 1, "{added:?}");

    let mut cfg = Config::load(&cli.config_path()).unwrap();
    let state_db = cli.install_layout().state_db();
    wingetrs::buckets::remove_and_cleanup_confirmed(&cli, &mut cfg, "mine", &state_db, false)
        .unwrap();
    jobs::bucket_remove(&tui, "mine", false).unwrap();
    let removed = outcome(&cli);
    assert_eq!(removed, outcome(&tui));
    assert_eq!(removed.1.len(), 2, "{removed:?}");
    assert!(removed.2.is_empty(), "{removed:?}");
}
