#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 162): the TUI confirmation of third-party scripts.
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use sha2::{Digest, Sha256};
use wingetrs::cache::DataDir;
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::cli::output::ThirdParty;
use wingetrs::config::ConfirmMode;
use wingetrs::core::SourceKind;
use wingetrs::http::Http;
use wingetrs::install::executor::{Runners, ScriptHost, ScriptJob, ScriptOutcome};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, HookKind};
use wingetrs::install::ops::{OpsCtx, ResidueScan};
use wingetrs::install::plan::{InstallPlan, Step, UninstallPlan};
use wingetrs::install::progress::ProgressEvent;
use wingetrs::install::script_scan::{ScriptReport, Verdict};
use wingetrs::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
use wingetrs::install::traits::{Effects, EnvStore, EnvValue};
use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
use wingetrs::tui::app::{
    Action, App, Cmd, InstallFail, Modal, Msg, Planned, Tab, ToastKind, update,
};
use wingetrs::tui::jobs;
use wingetrs::tui::regions::confirm_geom;
use wingetrs::tui::theme::{ColorDepth, GlyphSet, Theme};
use wingetrs::tui::view::view;
use wingetrs::update::LockWait;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn key(c: KeyCode) -> Msg {
    Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
}

fn app_with(mode: ConfirmMode) -> App {
    let mut a = App::new(Vec::new(), GlyphSet::Ascii);
    a.size = (80, 24);
    a.settings.behavior.confirm = mode;
    a
}

/// A plan whose only step is a `post_install` hook running `script`.
fn plan_with(name: &str, script: Option<&str>) -> InstallPlan {
    InstallPlan {
        app: name.into(),
        bucket: "main".into(),
        version: "1.0".into(),
        arch: Some(Arch::X64),
        upgrade_from: None,
        root: PathBuf::from("C:\\wingetrs-test"),
        manifest_sha256: String::new(),
        third_party_uninstaller: false,
        has_uninstall_scripts: false,
        notes: Vec::new(),
        steps: script
            .map(|s| Step::Hook {
                kind: HookKind::PostInstall,
                script: s.to_string(),
            })
            .into_iter()
            .collect(),
    }
}

fn apply_planned(a: &mut App, plan: InstallPlan) {
    let action = Action::Apply(vec![vec![plan]]);
    update(
        a,
        Msg::Planned(Ok(Planned::Ready {
            text: String::new(),
            action,
        })),
    );
}

fn uninstall_plan(name: &str) -> UninstallPlan {
    UninstallPlan {
        app: name.into(),
        version: "1.0".into(),
        purge: false,
        artifacts: Vec::new(),
        path_users: Vec::new(),
        remove_path_entry: false,
    }
}

fn report(verdict: Verdict) -> ScriptReport {
    ScriptReport {
        commands: Vec::new(),
        variables: BTreeSet::new(),
        flags: Vec::new(),
        verdict,
    }
}

fn removal_planned(a: &mut App, name: &str, script: &str, verdict: Verdict) {
    update(
        a,
        Msg::Planned(Ok(Planned::ReadyScripts {
            text: String::new(),
            action: Action::Remove(uninstall_plan(name)),
            manifest_sha256: "sha-a".into(),
            code: vec![ThirdParty::Script {
                hook: "pre_uninstall".into(),
                script: script.into(),
                report: report(verdict),
            }],
        })),
    );
}

/// The action the last Enter started.
fn started(a: &App) -> Option<Action> {
    a.running.as_ref().and_then(|r| r.action.clone())
}

fn script_of(n: usize) -> String {
    (1..=n)
        .map(|i| format!("$x{i} = {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render(a: &App, w: u16, h: u16) -> String {
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| view(a, &theme, f)).unwrap();
    format!("{}", t.backend())
}

#[test]
fn dialog_forced_even_when_confirm_setting_skips() {
    let mut a = app_with(ConfirmMode::Never);
    apply_planned(&mut a, plan_with("tool", Some("$x = 1")));
    assert!(matches!(a.modal, Some(Modal::Confirm { .. })));
    assert!(a.running.is_none(), "nothing runs before the confirmation");
    // The same setting runs a plan without third-party code at once.
    let mut b = app_with(ConfirmMode::Never);
    apply_planned(&mut b, plan_with("plain", None));
    assert!(b.modal.is_none());
    assert!(matches!(started(&b), Some(Action::Apply(_))));
}

#[test]
fn confirm_yields_apply_scripts() {
    let mut a = app_with(ConfirmMode::Never);
    apply_planned(&mut a, plan_with("tool", Some("$x = 1")));
    update(&mut a, key(KeyCode::Enter));
    assert!(a.modal.is_none());
    assert!(
        matches!(started(&a), Some(Action::ApplyScripts { .. })),
        "{:?}",
        started(&a)
    );
}

#[test]
fn cancel_yields_nothing() {
    let mut a = app_with(ConfirmMode::Never);
    apply_planned(&mut a, plan_with("tool", Some("$x = 1")));
    update(&mut a, key(KeyCode::Esc));
    assert!(a.modal.is_none());
    assert!(a.running.is_none() && a.queue.is_empty());
}

#[test]
fn plan_without_scripts_keeps_apply() {
    let mut a = app_with(ConfirmMode::Always);
    apply_planned(&mut a, plan_with("plain", None));
    assert!(matches!(a.modal, Some(Modal::Confirm { .. })));
    update(&mut a, key(KeyCode::Enter));
    assert!(matches!(started(&a), Some(Action::Apply(_))));
}

#[test]
fn script_lines_numbered_and_header_rendered() {
    let mut a = app_with(ConfirmMode::Never);
    apply_planned(&mut a, plan_with("tool", Some(&script_of(3))));
    let text = render(&a, 200, 30);
    assert!(
        text.contains(
            "Script de terceiro (post_install), roda com os seus privilégios e pode alterar qualquer arquivo seu"
        ),
        "{text}"
    );
    assert!(text.contains("1 | $x1 = 1"), "{text}");
    assert!(text.contains("3 | $x3 = 3"), "{text}");
    assert!(text.contains("Análise:"), "{text}");
    assert!(!text.to_lowercase().contains("segur"), "{text}");
    // On a narrow screen the header wraps instead of losing its end.
    let narrow = render(&a, 80, 30);
    assert!(narrow.contains("qualquer arquivo seu"), "{narrow}");
}

#[test]
fn scroll_moves_script_window() {
    let mut a = app_with(ConfirmMode::Never);
    apply_planned(&mut a, plan_with("tool", Some(&script_of(40))));
    let before = render(&a, 80, 24);
    assert!(before.contains(" 1 | $x1 = 1"), "{before}");
    update(&mut a, key(KeyCode::PageDown));
    let after = render(&a, 80, 24);
    assert!(!after.contains(" 1 | $x1 = 1"), "{after}");
    assert_ne!(before, after);
    update(&mut a, key(KeyCode::PageUp));
    assert_eq!(render(&a, 80, 24), before);
    // The wheel over the section scrolls it too.
    let Some(Modal::Confirm { .. }) = &a.modal else {
        panic!("dialog closed");
    };
    let r = wingetrs::tui::regions::regions(&a, Rect::new(0, 0, 80, 24));
    let ev = crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::ScrollDown,
        column: r.confirm_scripts.x + 1,
        row: r.confirm_scripts.y + 1,
        modifiers: KeyModifiers::NONE,
    };
    update(&mut a, Msg::Mouse(ev));
    assert_ne!(render(&a, 80, 24), before);
}

#[test]
fn remove_with_scripts_yields_remove_scripts() {
    let mut a = app_with(ConfirmMode::Never);
    removal_planned(&mut a, "tool", "$y = 1", Verdict::Confirm);
    assert!(matches!(a.modal, Some(Modal::Confirm { .. })));
    let text = render(&a, 100, 30);
    assert!(
        text.contains("Script de terceiro (pre_uninstall)"),
        "{text}"
    );
    update(&mut a, key(KeyCode::Enter));
    assert!(
        matches!(started(&a), Some(Action::RemoveScripts { plan, manifest_sha256 }) if plan.app == "tool" && manifest_sha256 == "sha-a"),
        "{:?}",
        started(&a)
    );
    // A removal without code, planned afterwards, keeps `Remove` and no dialog.
    let mut b = app_with(ConfirmMode::Never);
    update(
        &mut b,
        Msg::Planned(Ok(Planned::Ready {
            text: String::new(),
            action: Action::Remove(uninstall_plan("plain")),
        })),
    );
    assert!(b.modal.is_none());
    assert!(matches!(started(&b), Some(Action::Remove(_))));
}

#[test]
fn refused_script_never_runs() {
    let mut a = app_with(ConfirmMode::Never);
    removal_planned(&mut a, "bad", "$y = 1", Verdict::Refuse);
    let text = render(&a, 100, 30);
    assert!(text.contains("recusado"), "{text}");
    update(&mut a, key(KeyCode::Enter));
    assert!(a.running.is_none() && a.queue.is_empty());
    assert!(matches!(a.modal, Some(Modal::Confirm { .. })), "stays open");
    assert!(a.toasts.iter().any(|t| t.kind == ToastKind::Error));
}

#[test]
fn bidi_and_control_characters_are_shown_escaped() {
    let mut a = app_with(ConfirmMode::Never);
    let script = "$a = 1 \u{202E}evil\u{200B}\n$b = \u{1b}[31m";
    apply_planned(&mut a, plan_with("tool", Some(script)));
    let Some(Modal::Confirm { scripts, .. }) = &a.modal else {
        panic!("no dialog");
    };
    let all: String = scripts
        .lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !all.contains('\u{202E}') && !all.contains('\u{200B}') && !all.contains('\u{1b}'),
        "{all:?}"
    );
    assert!(all.contains("\\u{202e}"), "{all:?}");
    let text = render(&a, 100, 30);
    assert!(!text.contains('\u{202E}'), "{text:?}");
}

#[test]
fn regions_has_script_area() {
    let area = Rect::new(0, 0, 80, 24);
    let g = confirm_geom(1, &[], &code_lines(40), area);
    assert!(g.scripts.height > 0, "{g:?}");
    let inside = g.scripts.x >= g.dialog.x
        && g.scripts.y >= g.dialog.y
        && g.scripts.right() <= g.dialog.right()
        && g.scripts.bottom() <= g.dialog.bottom();
    assert!(inside, "{g:?}");
    // The list keeps its item, and the two do not overlap.
    assert!(g.list.height >= 2);
    assert!(g.list.bottom() <= g.scripts.y);
    // No scripts: no section.
    assert_eq!(confirm_geom(1, &[], &[], area).scripts.height, 0);
}

#[test]
#[ignore = "bench"]
fn bench_confirm_300_lines() {
    let mut a = app_with(ConfirmMode::Never);
    apply_planned(&mut a, plan_with("tool", Some(&script_of(300))));
    let theme = Theme::new(true, ColorDepth::NoColor);
    let mut t = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let start = std::time::Instant::now();
    for _ in 0..100 {
        t.draw(|f| view(&a, &theme, f)).unwrap();
    }
    eprintln!(
        "draw with 300 script lines: {:?}/frame",
        start.elapsed() / 100
    );
}

// ---- jobs: the action reaches the script host ----

fn fake_pe() -> Vec<u8> {
    let mut b = vec![0u8; 0x80 + 24 + 96];
    b[0] = b'M';
    b[1] = b'Z';
    b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    let off = 0x80 + 24 + 68;
    b[off..off + 2].copy_from_slice(&3u16.to_le_bytes());
    b
}

fn zip_of(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        z.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(bytes).unwrap();
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn nothing_in_use(_: &Path) -> Vec<PathBuf> {
    Vec::new()
}

#[derive(Default)]
struct FakeHost {
    seen: Mutex<Vec<HookKind>>,
}

impl ScriptHost for FakeHost {
    fn run(&self, job: &ScriptJob) -> std::io::Result<ScriptOutcome> {
        self.seen.lock().unwrap().push(job.hook);
        Ok(ScriptOutcome {
            code: 0,
            lines: Vec::new(),
        })
    }
}

struct World {
    _server: MockServer,
    _t: tempfile::TempDir,
    data: DataDir,
    layout: Layout,
    env: FakeEnv,
    shortcuts: FakeShortcuts,
    trash: FakeTrash,
    registry: FakeRegistry,
    http: Http,
    catalog: Catalog,
}

impl World {
    async fn new() -> World {
        let server = MockServer::start().await;
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("root");
        let layout = Layout::new(&root);
        std::fs::create_dir_all(layout.launcher().parent().unwrap()).unwrap();
        std::fs::write(layout.launcher(), fake_pe()).unwrap();
        let env = FakeEnv::new();
        env.set(
            "Path",
            &EnvValue {
                value: r"C:\Existing".into(),
                expand: true,
            },
        )
        .unwrap();
        let zip = zip_of("pkg/hooked.exe", &fake_pe());
        let sha = hex::encode(Sha256::digest(&zip));
        Mock::given(method("GET"))
            .and(path("/hooked-1.0.zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(zip))
            .mount(&server)
            .await;
        let raw = serde_json::json!({
            "version": "1.0",
            "url": format!("{}/hooked-1.0.zip", server.uri()),
            "hash": sha,
            "extract_dir": "pkg",
            "bin": "hooked.exe",
            "post_install": ["$marker = 1"],
            "post_uninstall": ["$marker = 2"],
        })
        .to_string();
        let mut catalog = Catalog::open_in_memory().unwrap();
        catalog
            .replace_source(
                "scoop:main",
                SourceKind::Scoop,
                "v",
                1,
                &[NewPackage {
                    id: "hooked".into(),
                    name: "hooked".into(),
                    version: "1.0".into(),
                    raw: Some(raw),
                    ..Default::default()
                }],
                &WingetMaps::default(),
            )
            .unwrap();
        World {
            _server: server,
            data: DataDir::new(&root),
            layout,
            _t: t,
            env,
            shortcuts: FakeShortcuts::new(root.join("..").join("startmenu")),
            trash: FakeTrash::new(),
            registry: FakeRegistry::new(),
            http: Http::new().unwrap(),
            catalog,
        }
    }

    fn ctx<'a>(
        &'a self,
        host: &Arc<FakeHost>,
        progress: &'a (dyn Fn(&ProgressEvent) + Send + Sync),
    ) -> OpsCtx<'a> {
        let scripts: Arc<dyn ScriptHost> = host.clone();
        OpsCtx {
            data: &self.data,
            http: &self.http,
            catalog: &self.catalog,
            effects: Effects {
                env: &self.env,
                shortcuts: &self.shortcuts,
                trash: &self.trash,
                registry: &self.registry,
            },
            wait: LockWait::NoWait,
            progress,
            in_use: &nothing_in_use,
            residue: ResidueScan::none(),
            msiexec: wingetrs::install::executor::Msiexec::DISABLED,
            launch: Default::default(),
            runners: Runners {
                scripts: Some(scripts),
                ..Runners::disabled()
            },
        }
    }
}

fn quiet(_: &ProgressEvent) {}

async fn plans_of(w: &World, host: &Arc<FakeHost>) -> Vec<Vec<InstallPlan>> {
    let ctx = w.ctx(host, &quiet);
    let Ok(Planned::Ready {
        action: Action::Apply(groups),
        ..
    }) = jobs::plan_install(&ctx, "scoop:main", "hooked", false)
    else {
        panic!("expected a ready plan");
    };
    groups
}

#[tokio::test]
async fn apply_scripts_reaches_host() {
    let w = World::new().await;
    let host = Arc::new(FakeHost::default());
    let groups = plans_of(&w, &host).await;
    let ctx = w.ctx(&host, &quiet);
    // Plain `Apply` has no consent: the hook does not run.
    let refused = jobs::run_action(&ctx, Action::Apply(groups.clone())).await;
    assert!(refused.is_err(), "{refused:?}");
    assert!(host.seen.lock().unwrap().is_empty());
    // The confirmed action runs it.
    let done = jobs::run_action(&ctx, Action::ApplyScripts { groups })
        .await
        .unwrap();
    assert_eq!(done.summary, "Instalado: hooked");
    assert_eq!(*host.seen.lock().unwrap(), vec![HookKind::PostInstall]);
}

#[tokio::test]
async fn removal_notes_reach_the_user() {
    let w = World::new().await;
    let host = Arc::new(FakeHost::default());
    let groups = plans_of(&w, &host).await;
    let ctx = w.ctx(&host, &quiet);
    jobs::run_action(&ctx, Action::ApplyScripts { groups })
        .await
        .unwrap();
    // The written manifest is gone: the uninstaller cannot be looked for, and the user is told.
    let manifest = w.layout.version_dir("hooked", "1.0").join("manifest.json");
    std::fs::remove_file(&manifest).unwrap();
    let Ok(Planned::Ready { action, .. }) = jobs::plan_remove(&ctx, "hooked", false) else {
        panic!("expected a plain removal: no manifest, no code");
    };
    let done = jobs::run_action(&ctx, action).await.unwrap();
    assert!(
        done.summary.starts_with("hooked removido"),
        "{}",
        done.summary
    );
    assert!(
        done.summary.contains("o desinstalador do app não rodou"),
        "{}",
        done.summary
    );
}

#[test]
fn removed_summary_without_notes_is_plain() {
    assert_eq!(jobs::removed_summary("x", &[]), "x removido");
    assert_eq!(
        jobs::removed_summary("x", &["a".into(), "b".into()]),
        "x removido. a. b"
    );
}

#[test]
fn paging_shows_every_line_and_reaches_the_end() {
    let mut a = app_with(ConfirmMode::Never);
    apply_planned(&mut a, plan_with("tool", Some(&script_of(40))));
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..40 {
        let text = render(&a, 80, 24);
        for i in 1..=40 {
            if text.contains(&format!("$x{i} = {i} ")) {
                seen.insert(i);
            }
        }
        update(&mut a, key(KeyCode::PageDown));
    }
    assert_eq!(seen.len(), 40, "lines never shown: {seen:?}");
}
// ---- review: batches, msiexec, small windows, shown manifest ----

fn irow(name: &str) -> ListRow {
    ListRow {
        name: name.into(),
        app_key: String::new(),
        not_in_catalog: false,
        origin: "wingetrs".into(),
        installed_version: Some("1".into()),
        correlation: Correlation::Matched {
            source: "scoop:main".into(),
            package_id: name.into(),
            rule: Rule::ScoopManifest,
        },
        latest: Some("1".into()),
        state: UpdateState::UpToDate,
        tiebreak: None,
        ..Default::default()
    }
}

/// Installed tab with `alpha` and `beta` marked and the removal batch planned.
fn marked_pair(mode: ConfirmMode) -> App {
    let mut a = app_with(mode);
    a.installed = vec![irow("alpha"), irow("beta")];
    a.tab = Tab::Installed;
    for i in 0..2 {
        a.selected = i;
        update(&mut a, key(KeyCode::Char(' ')));
    }
    assert_eq!(a.marked_count(), 2);
    let cmds = update(&mut a, key(KeyCode::Char('u')));
    assert!(
        matches!(cmds.as_slice(), [Cmd::PlanBatch { .. }]),
        "{cmds:?}"
    );
    a
}

fn batch_results() -> Vec<Result<Planned, String>> {
    vec![
        Ok(Planned::ReadyScripts {
            text: String::new(),
            action: Action::Remove(uninstall_plan("alpha")),
            manifest_sha256: "sha-alpha".into(),
            code: vec![ThirdParty::Script {
                hook: "pre_uninstall".into(),
                script: "$gone = 1".into(),
                report: report(Verdict::Confirm),
            }],
        }),
        Ok(Planned::Ready {
            text: String::new(),
            action: Action::Remove(uninstall_plan("beta")),
        }),
    ]
}

#[test]
fn batch_shows_removal_code_and_yields_remove_scripts() {
    let mut a = marked_pair(ConfirmMode::Always);
    update(&mut a, Msg::BatchPlanned(batch_results()));
    a.size = (100, 50);
    let text = render(&a, 100, 50);
    assert!(
        text.contains("Script de terceiro (pre_uninstall)"),
        "{text}"
    );
    assert!(text.contains("1 | $gone = 1"), "{text}");
    update(&mut a, key(KeyCode::Enter));
    assert!(
        matches!(
            started(&a),
            Some(Action::RemoveScripts { plan, manifest_sha256 })
                if plan.app == "alpha" && manifest_sha256 == "sha-alpha"
        ),
        "{:?}",
        started(&a)
    );
    // The one without code stays a plain removal.
    assert!(matches!(&a.queue[0].action, Action::Remove(p) if p.app == "beta"));
}

#[test]
fn batch_dialog_forced_even_when_confirm_setting_skips() {
    let mut a = marked_pair(ConfirmMode::Never);
    update(&mut a, Msg::BatchPlanned(batch_results()));
    assert!(matches!(a.modal, Some(Modal::Confirm { .. })));
    assert!(a.running.is_none() && a.queue.is_empty());
    // Without code the same setting runs everything at once.
    let mut b = marked_pair(ConfirmMode::Never);
    let plain: Vec<_> = ["alpha", "beta"]
        .iter()
        .map(|n| {
            Ok(Planned::Ready {
                text: String::new(),
                action: Action::Remove(uninstall_plan(n)),
            })
        })
        .collect();
    update(&mut b, Msg::BatchPlanned(plain));
    assert!(b.modal.is_none());
    assert!(started(&b).is_some());
}

#[test]
fn msiexec_in_a_scripts_batch_keeps_the_consent_of_the_rest() {
    let mut a = app_with(ConfirmMode::Always);
    let groups: Vec<Vec<InstallPlan>> = ["one", "two", "three"]
        .iter()
        .map(|n| vec![plan_with(n, Some("$x = 1"))])
        .collect();
    update(
        &mut a,
        Msg::Planned(Ok(Planned::Ready {
            text: String::new(),
            action: Action::Apply(groups),
        })),
    );
    update(&mut a, key(KeyCode::Enter));
    assert!(matches!(started(&a), Some(Action::ApplyScripts { .. })));
    let cmds = update(
        &mut a,
        Msg::InstallFinished(Err(InstallFail::NeedsMsiexec {
            app: "two".into(),
            reason: "MSI".into(),
            applied: vec!["one".into()],
        })),
    );
    // `three` goes on at once, still with the consent; it is not dropped or downgraded.
    let again = cmds.iter().find_map(|c| match c {
        Cmd::Execute(act) => Some(act.clone()),
        _ => None,
    });
    assert!(
        matches!(&again, Some(Action::ApplyScripts { groups })
            if groups.len() == 1 && groups[0][0].app == "three"),
        "{cmds:?}"
    );
    // `two` waits for the msiexec confirmation and keeps it too.
    update(&mut a, key(KeyCode::Enter));
    assert!(
        matches!(&a.queue[0].action, Action::ApplyMsiexec { app, scripts: true, .. } if app == "two"),
        "{:?}",
        a.queue
    );
}

#[test]
fn window_too_small_for_the_scripts_blocks_continue() {
    let mut a = app_with(ConfirmMode::Never);
    a.size = (80, 12);
    apply_planned(&mut a, plan_with("tool", Some(&script_of(5))));
    let g = confirm_geom(1, &[], &code_lines(5), Rect::new(0, 0, 80, 12));
    assert!(g.scripts.height < 3, "{g:?}");
    update(&mut a, key(KeyCode::Enter));
    assert!(matches!(a.modal, Some(Modal::Confirm { .. })), "stays open");
    assert!(a.running.is_none() && a.queue.is_empty());
    assert!(
        a.toasts
            .iter()
            .any(|t| t.kind == ToastKind::Warning && t.text.contains("aumente a janela")),
        "{:?}",
        a.toasts
    );
    // Room enough: it goes.
    a.size = (80, 24);
    update(&mut a, key(KeyCode::Enter));
    assert!(matches!(started(&a), Some(Action::ApplyScripts { .. })));
}

#[tokio::test]
async fn remove_scripts_refuses_a_manifest_other_than_the_one_shown() {
    let w = World::new().await;
    let host = Arc::new(FakeHost::default());
    let groups = plans_of(&w, &host).await;
    let ctx = w.ctx(&host, &quiet);
    jobs::run_action(&ctx, Action::ApplyScripts { groups })
        .await
        .unwrap();
    let Ok(Planned::ReadyScripts {
        action: Action::Remove(plan),
        manifest_sha256,
        code,
        ..
    }) = jobs::plan_remove(&ctx, "hooked", false)
    else {
        panic!("expected a removal with code");
    };
    assert!(!code.is_empty() && !manifest_sha256.is_empty());
    // Another manifest than the one the user read: nothing is removed.
    let refused = jobs::run_action(
        &ctx,
        Action::RemoveScripts {
            plan: plan.clone(),
            manifest_sha256: "outro".into(),
        },
    )
    .await;
    match refused {
        Err(InstallFail::Other(m)) => assert!(m.contains("mudou desde que você revisou"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(w.layout.current("hooked").exists());
    assert!(!host.seen.lock().unwrap().contains(&HookKind::PostUninstall));
    // The manifest that was shown: it runs.
    jobs::run_action(
        &ctx,
        Action::RemoveScripts {
            plan,
            manifest_sha256,
        },
    )
    .await
    .unwrap();
    assert!(host.seen.lock().unwrap().contains(&HookKind::PostUninstall));
}

fn code_lines(n: usize) -> Vec<wingetrs::tui::app::ScriptLine> {
    (0..n)
        .map(|i| wingetrs::tui::app::ScriptLine {
            kind: wingetrs::tui::app::ScriptLineKind::Code,
            text: format!("  {i} | x"),
        })
        .collect()
}
