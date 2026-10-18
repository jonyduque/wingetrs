#![allow(clippy::unwrap_used, clippy::expect_used)]
//! winget install through the bridge (Task 176): command line, elevated-helper protocol,
//! elevation choice, version check, Settings-scope fallback, log, CLI and TUI. Fake execs and
//! transports only: nothing here runs winget or elevates.
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use clap::{CommandFactory, Parser};
use tempfile::TempDir;
use wingetrs::bridge::elevated::{self, ElevatedBridge, Helper, Transport};
use wingetrs::bridge::worker::{self, Reply, Request, ScopeArg};
use wingetrs::bridge::{self, BridgeJob, BridgeOp, Tool};
use wingetrs::cli::winget_cmd;
use wingetrs::cli::{Cli, Command};
use wingetrs::install::layout::Layout;
use wingetrs::installed::correlate::InstallScope;
use wingetrs::sources::winget::manifest::{Agreement, MergedManifest, parse};
use wingetrs::tui::app::{Cmd, InstallFail, Modal, Msg};
use wingetrs::tui::jobs;

type Call = (String, Vec<String>);

fn install(target: &str, scope: Option<InstallScope>) -> BridgeJob {
    BridgeJob {
        tool: Tool::Winget,
        op: BridgeOp::Install {
            scope,
            agreements: Vec::new(),
        },
        target: target.into(),
        name: format!("App {target}"),
        installed: None,
        available: Some("2.0".into()),
        scope,
    }
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

const BASE: [&str; 10] = [
    "install",
    "--id",
    "Git.Git",
    "--exact",
    "--source",
    "winget",
    "--silent",
    "--disable-interactivity",
    "--accept-package-agreements",
    "--accept-source-agreements",
];

fn base_with(extra: &[&str]) -> Vec<String> {
    let mut v = strings(&BASE);
    v.extend(strings(extra));
    v
}

fn cmd(job: &BridgeJob) -> Call {
    bridge::command(job, Path::new("C:/nao-usado"), "pwsh")
}

#[test]
fn install_command_exact() {
    assert_eq!(
        cmd(&install("Git.Git", None)),
        ("winget".to_string(), base_with(&[]))
    );
    assert_eq!(
        cmd(&install("Git.Git", Some(InstallScope::Machine))),
        ("winget".to_string(), base_with(&["--scope", "machine"]))
    );
    assert_eq!(
        cmd(&install("Git.Git", Some(InstallScope::User))),
        ("winget".to_string(), base_with(&["--scope", "user"]))
    );
}

/// Runs `serve` over `input` with an exec that records its calls and exits 0.
fn serve_on(input: &str) -> (std::io::Result<()>, Vec<Call>, Vec<Reply>) {
    let mut calls = Vec::new();
    let mut exec = |p: &str, a: &[String], on_line: &mut dyn FnMut(&str)| {
        calls.push((p.to_string(), a.to_vec()));
        on_line("ok");
        Ok((0, Vec::new()))
    };
    let mut reader = BufReader::new(input.as_bytes());
    let mut out = Vec::new();
    let r = worker::serve(&mut reader, &mut out, &mut exec);
    let replies = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| worker::decode::<Reply>(l).unwrap())
        .collect();
    (r, calls, replies)
}

#[test]
fn serve_runs_install_only_with_valid_target() {
    let (r, calls, replies) =
        serve_on("{\"op\":\"install\",\"target\":\"Git.Git\",\"scope\":\"machine\"}\n");
    r.unwrap();
    assert_eq!(
        calls,
        vec![("winget".to_string(), base_with(&["--scope", "machine"]))]
    );
    assert_eq!(replies.last(), Some(&Reply::End { code: 0 }));
    // Without a scope: no `--scope` at all.
    let (_, calls, _) = serve_on("{\"op\":\"install\",\"target\":\"Git.Git\"}\n");
    assert_eq!(calls, vec![("winget".to_string(), base_with(&[]))]);

    for bad in ["a b", "--override", "x&y"] {
        let line = worker::encode(&Request::Install {
            target: bad.into(),
            scope: Some(ScopeArg::Machine),
        })
        .unwrap();
        let (r, calls, replies) = serve_on(&line);
        r.unwrap();
        assert!(calls.is_empty(), "{bad}: exec ran");
        assert!(
            matches!(
                replies.as_slice(),
                [Reply::Error {
                    not_found: false,
                    ..
                }]
            ),
            "{bad}: {replies:?}"
        );
    }
}

#[test]
fn install_request_refuses_bad_scope_and_extra_fields() {
    for bad in [
        "{\"op\":\"install\",\"target\":\"Git.Git\",\"scope\":\"admin\"}\n",
        "{\"op\":\"install\",\"target\":\"Git.Git\",\"cmd\":\"calc\"}\n",
        "{\"op\":\"install\",\"target\":\"Git.Git\",\"scope\":\"machine\",\"cmd\":\"calc\"}\n",
    ] {
        // A valid request after the bad one must never run: serve stops at the bad line.
        let input = format!("{bad}{{\"op\":\"install\",\"target\":\"Git.Git\"}}\n");
        let (r, calls, replies) = serve_on(&input);
        assert!(r.is_err(), "{bad}");
        assert!(calls.is_empty(), "{bad}: exec ran");
        assert!(replies.is_empty(), "{bad}: {replies:?}");
    }
}

#[test]
fn request_for_maps_install() {
    let job = install("Git.Git", None);
    let (p, a) = cmd(&job);
    assert_eq!(
        elevated::request_for(&job, &p, &a).unwrap(),
        Request::Install {
            target: "Git.Git".into(),
            scope: None
        }
    );
    let machine = install("Git.Git", Some(InstallScope::Machine));
    let (p, a) = cmd(&machine);
    assert_eq!(
        elevated::request_for(&machine, &p, &a).unwrap(),
        Request::Install {
            target: "Git.Git".into(),
            scope: Some(ScopeArg::Machine)
        }
    );
    let mut more = a.clone();
    more.extend(strings(&["--override", "x"]));
    let e = elevated::request_for(&machine, &p, &more).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
    // The install command of a job is not its upgrade, nor the other way round.
    let upgrade = BridgeJob {
        op: BridgeOp::Upgrade,
        ..job.clone()
    };
    let e = elevated::request_for(&upgrade, &p, &a).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
}

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A manifest with one installer per `(architecture, scope)`.
fn manifest(installers: &[(&str, &str)]) -> MergedManifest {
    let mut y = String::from("PackageIdentifier: Git.Git\nPackageVersion: '2.0'\nInstallers:\n");
    for (arch, scope) in installers {
        y.push_str(&format!(
            "- InstallerUrl: https://x/i.exe\n  InstallerSha256: {SHA}\n  Architecture: {arch}\n  Scope: {scope}\n"
        ));
    }
    parse(y.as_bytes()).unwrap()
}

/// A helper that is never asked anything: only whether it was started matters.
struct IdleHelper;

impl Helper for IdleHelper {
    fn reader(&mut self) -> &mut dyn std::io::BufRead {
        unreachable!("no request in this test")
    }
    fn writer(&mut self) -> &mut dyn std::io::Write {
        unreachable!("no request in this test")
    }
    fn wait_exit(&mut self, _limit: Duration) -> bool {
        true
    }
    fn alive(&mut self) -> bool {
        true
    }
}

#[derive(Clone, Default)]
struct CountingTransport {
    starts: Arc<AtomicUsize>,
}

impl Transport for CountingTransport {
    fn start(&self) -> Result<Box<dyn Helper>, String> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(IdleHelper))
    }
}

/// Whether `job` would run through the elevated helper, and how many helpers it started.
fn elevates(job: &BridgeJob) -> (bool, usize) {
    let t = CountingTransport::default();
    let bridge = ElevatedBridge::new(Box::new(t.clone()));
    let mut current = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| Ok((0, Vec::new()));
    let chosen = bridge.exec_for(job, &mut current, &mut |_: &str| {});
    (chosen.elevated(), t.starts.load(Ordering::SeqCst))
}

#[test]
fn machine_only_manifest_goes_elevated_user_never() {
    let all_machine = manifest(&[("x64", "machine"), ("x86", "machine")]);
    let job = bridge::install_job("Git.Git", "Git", "2.0", &all_machine, None, "x64");
    assert_eq!(job.scope, Some(InstallScope::Machine));
    assert_eq!(
        job.op,
        BridgeOp::Install {
            scope: None,
            agreements: Vec::new()
        },
        "no --scope added"
    );
    assert_eq!(elevates(&job), (true, 1));

    let mixed = manifest(&[("x64", "machine"), ("x64", "user")]);
    let job = bridge::install_job("Git.Git", "Git", "2.0", &mixed, None, "x64");
    assert_eq!(job.scope, None);
    assert_eq!(elevates(&job), (false, 0));
    // The order of the installers does not matter.
    let mixed = manifest(&[("x64", "user"), ("x64", "machine")]);
    let job = bridge::install_job("Git.Git", "Git", "2.0", &mixed, None, "x64");
    assert_eq!(job.scope, None);

    // No installer for this machine at all: nothing says machine-wide, so no elevation.
    let foreign = manifest(&[("arm64", "machine")]);
    let job = bridge::install_job("Git.Git", "Git", "2.0", &foreign, None, "x64");
    assert_eq!(job.scope, None);
    assert_eq!(elevates(&job), (false, 0));

    // Only installers for this machine count: an arm64 user installer does not run on x64.
    let other_arch = manifest(&[("x64", "machine"), ("arm64", "user")]);
    let job = bridge::install_job("Git.Git", "Git", "2.0", &other_arch, None, "x64");
    assert_eq!(job.scope, Some(InstallScope::Machine));

    let user = bridge::install_job(
        "Git.Git",
        "Git",
        "2.0",
        &all_machine,
        Some(InstallScope::User),
        "x64",
    );
    assert_eq!(user.scope, Some(InstallScope::User));
    assert_eq!(elevates(&user), (false, 0));

    let explicit = bridge::install_job(
        "Git.Git",
        "Git",
        "2.0",
        &mixed,
        Some(InstallScope::Machine),
        "x64",
    );
    assert_eq!(explicit.scope, Some(InstallScope::Machine));
    assert_eq!(elevates(&explicit), (true, 1));
}

/// A fresh `msi.lock` path per call; the directory goes away with the value.
fn msi_lock() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = Layout::new(dir.path()).msi_lock();
    (dir, path)
}

/// Runs an install of `Git.Git` offering `available` with a fake winget that picks `chosen`.
fn run_install(available: &str, chosen: &str) -> Result<String, InstallFail> {
    let (_dir, lock) = msi_lock();
    let job = BridgeJob {
        available: Some(available.into()),
        ..install("Git.Git", None)
    };
    let line = format!("Found Git [Git.Git] Version {chosen}");
    let mut exec = |p: &str, _: &[String], on_line: &mut dyn FnMut(&str)| {
        if p == "winget" {
            on_line(&line);
        }
        Ok((0, vec![line.clone()]))
    };
    jobs::run_bridge(&job, None, &lock, "pwsh", &mut exec, &mut |_: &str| {}).map(|d| d.summary)
}

#[test]
fn install_success_and_older_version() {
    assert_eq!(
        run_install("2.0", "2.0").unwrap(),
        "App Git.Git: instalado via winget"
    );
    run_install("2.0.0", "2.0").unwrap();
    run_install("2.0", "2.1").unwrap();
    match run_install("2.0", "1.9") {
        Err(InstallFail::Other(m)) => assert!(m.contains("instalou 1.9, não 2.0"), "{m}"),
        other => panic!("older version accepted: {other:?}"),
    }
    // Text order would call 10.0 older than 9.0.
    run_install("9.0", "10.0").unwrap();
}

#[test]
fn install_older_version_is_logged_as_no_effect() {
    let log = crate::spans::Log::default();
    let _installed = log.install();
    run_install("2.0", "1.9").unwrap_err();
    let text = log.text();
    let end = text.lines().find(|l| l.contains("job.end")).unwrap();
    assert!(end.contains("outcome=sem_efeito"), "{text}");
    let start = text.lines().find(|l| l.contains("job.start")).unwrap();
    assert!(start.contains("kind=install"), "{text}");
}

#[test]
fn job_start_lists_agreements() {
    let log = crate::spans::Log::default();
    let _installed = log.install();
    let info = |target: &str| wingetrs::job_log::JobInfo {
        kind: "install",
        tool: "winget",
        target: target.into(),
        name: target.into(),
        from: None,
        to: Some("2.0".into()),
        cmd: None,
    };
    wingetrs::job_log::JobLog::start_accepting(
        info("Accepting.App"),
        &strings(&["EULA https://x/eula", "Privacy https://x/p"]),
    )
    .finish(wingetrs::job_log::Outcome::Ok, "ok");
    wingetrs::job_log::JobLog::start_accepting(info("None.App"), &[])
        .finish(wingetrs::job_log::Outcome::Ok, "ok");
    wingetrs::job_log::JobLog::start(info("Plain.App"))
        .finish(wingetrs::job_log::Outcome::Ok, "ok");
    let text = log.text();
    let start = |target: &str| {
        text.lines()
            .find(|l| l.contains("job.start") && l.contains(&format!("target={target} ")))
            .unwrap_or_else(|| panic!("no job.start of {target}: {text}"))
            .to_string()
    };
    assert!(
        start("Accepting.App").contains("agreements=EULA https://x/eula; Privacy https://x/p"),
        "{text}"
    );
    assert!(start("None.App").contains("agreements=-"), "{text}");
    assert!(!start("Plain.App").contains("agreements"), "{text}");
}

fn agreement(label: Option<&str>, text: Option<&str>, url: Option<&str>) -> Agreement {
    Agreement {
        agreement_label: label.map(str::to_string),
        agreement: text.map(str::to_string),
        agreement_url: url.map(str::to_string),
    }
}

#[test]
fn cli_yes_prints_each_agreement() {
    let a1 = agreement(Some("EULA"), None, Some("https://x/eula"));
    let a2 = agreement(Some("Privacy"), None, Some("https://x/p"));
    assert_eq!(
        winget_cmd::agreement_lines(&[a1.clone(), a2.clone()], true),
        [
            "Licença aceita: EULA — https://x/eula",
            "Licença aceita: Privacy — https://x/p"
        ]
    );
    assert_eq!(
        winget_cmd::agreement_lines(&[a1, a2], false),
        [
            "Licença: EULA — https://x/eula",
            "Licença: Privacy — https://x/p"
        ]
    );
    let long = "x".repeat(300);
    let bare = agreement(None, Some(&long), None);
    assert_eq!(
        winget_cmd::agreement_lines(&[bare], false),
        [format!("Licença: Licença — {}", "x".repeat(200))]
    );
    // Multi-byte text is cut at 200 characters, not bytes.
    let accented = agreement(Some("Termos"), Some(&"é".repeat(250)), None);
    assert_eq!(
        winget_cmd::agreement_lines(std::slice::from_ref(&accented), false),
        [format!("Licença: Termos — {}", "é".repeat(200))]
    );
    assert_eq!(
        bridge::agreement_log(&accented),
        format!("Termos {}", "é".repeat(200))
    );
}

#[test]
fn scope_flag_parses_and_rejects() {
    let cli =
        Cli::try_parse_from(["wingetrs", "install", "Git.Git", "--scope", "machine"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Install {
            scope: Some(ScopeArg::Machine),
            ..
        })
    ));
    let cli = Cli::try_parse_from(["wingetrs", "install", "Git.Git", "--scope", "user"]).unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Install {
            scope: Some(ScopeArg::User),
            ..
        })
    ));
    let e =
        Cli::try_parse_from(["wingetrs", "install", "Git.Git", "--scope", "admin"]).unwrap_err();
    let text = e.to_string();
    assert!(text.contains("user") && text.contains("machine"), "{text}");
    let mut cmd = Cli::command();
    let help = cmd
        .find_subcommand_mut("install")
        .unwrap()
        .render_help()
        .to_string();
    assert!(help.contains("--scope"), "{help}");
}

/// A fake winget answering each call with the next `(code, chosen version)`; records calls.
struct Script {
    answers: Vec<(i32, &'static str)>,
    calls: Vec<Call>,
}

impl Script {
    fn run(
        &mut self,
        fallback: bool,
        job: &BridgeJob,
    ) -> (Result<String, InstallFail>, Vec<String>) {
        let (_dir, lock) = msi_lock();
        // Refuses elevation: every run goes through the fake exec below.
        let bridge = ElevatedBridge::new(Box::new(Refusing));
        let mut seen = Vec::new();
        let answers = &mut self.answers;
        let calls = &mut self.calls;
        let mut exec = |p: &str, a: &[String], on_line: &mut dyn FnMut(&str)| {
            calls.push((p.to_string(), a.to_vec()));
            let (code, chosen) = answers.remove(0);
            let line = format!("Found Git [Git.Git] Version {chosen}");
            on_line(&line);
            Ok((code, vec![line]))
        };
        let r =
            jobs::run_bridge_install(&bridge, job, fallback, &lock, &mut exec, &mut |l: &str| {
                seen.push(l.to_string())
            });
        (r.map(|d| d.summary), seen)
    }
}

struct Refusing;

impl Transport for Refusing {
    fn start(&self) -> Result<Box<dyn Helper>, String> {
        Err("recusado".into())
    }
}

const NO_INSTALLER: i32 = bridge::NO_APPLICABLE_INSTALLER as i32;
const INSTALLER_FAILED: i32 = 0x8A15_0006_u32 as i32;

#[test]
fn settings_machine_scope_falls_back_to_no_scope_once() {
    let machine = install("Git.Git", Some(InstallScope::Machine));
    let note = bridge::SCOPE_FALLBACK_NOTE.to_string();

    // No installer for machine, then the unscoped install works.
    let mut s = Script {
        answers: vec![(NO_INSTALLER, "2.0"), (0, "2.0")],
        calls: Vec::new(),
    };
    let (r, seen) = s.run(true, &machine);
    assert_eq!(r.unwrap(), "App Git.Git: instalado via winget");
    assert_eq!(
        s.calls,
        vec![
            ("winget".to_string(), base_with(&["--scope", "machine"])),
            ("winget".to_string(), base_with(&[])),
        ]
    );
    assert!(seen.contains(&note), "{seen:?}");

    // Works at once: no second run, no note.
    let mut s = Script {
        answers: vec![(0, "2.0")],
        calls: Vec::new(),
    };
    let (r, seen) = s.run(true, &machine);
    r.unwrap();
    assert_eq!(s.calls.len(), 1);
    assert!(!seen.contains(&note), "{seen:?}");

    // The second run fails for another reason: that reason is reported, no third run.
    let mut s = Script {
        answers: vec![(NO_INSTALLER, "2.0"), (INSTALLER_FAILED, "2.0")],
        calls: Vec::new(),
    };
    match s.run(true, &machine) {
        (Err(InstallFail::Other(m)), _) => assert!(m.contains("O instalador falhou."), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.calls.len(), 2);

    // Another failure first: never retried without the scope.
    let mut s = Script {
        answers: vec![(INSTALLER_FAILED, "2.0")],
        calls: Vec::new(),
    };
    s.run(true, &machine).0.unwrap_err();
    assert_eq!(s.calls.len(), 1);

    // An explicit `--scope machine` (no fallback) is never dropped.
    let mut s = Script {
        answers: vec![(NO_INSTALLER, "2.0")],
        calls: Vec::new(),
    };
    let (r, seen) = s.run(false, &machine);
    match r {
        Err(InstallFail::Other(m)) => assert!(m.contains("Nenhum instalador"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.calls.len(), 1);
    assert!(!seen.contains(&note));

    // A user scope is never dropped either.
    let mut s = Script {
        answers: vec![(NO_INSTALLER, "2.0")],
        calls: Vec::new(),
    };
    s.run(true, &install("Git.Git", Some(InstallScope::User)))
        .0
        .unwrap_err();
    assert_eq!(s.calls.len(), 1);
}

fn tui_app() -> wingetrs::tui::app::App {
    use wingetrs::core::{PackageEntry, SourceKind};
    let entry = |source: SourceKind, id: &str| PackageEntry {
        source,
        source_name: if source == SourceKind::Winget {
            "winget".into()
        } else {
            "scoop:main".into()
        },
        id: id.into(),
        name: id.into(),
        moniker: None,
        version: "1".into(),
        publisher: None,
        description: None,
        tags: vec![],
        commands: vec![],
    };
    wingetrs::tui::app::App::new(
        vec![entry(SourceKind::Winget, "Git.Git")],
        wingetrs::tui::theme::GlyphSet::Nerd,
    )
}

#[test]
fn tui_search_install_winget_enabled() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use wingetrs::tui::app::{ActionId, buttons, update};
    let mut a = tui_app();
    a.size = (100, 30);
    assert_eq!(
        buttons(&a),
        vec![("Instalar (enter)".to_string(), ActionId::Install)]
    );
    let cmds = update(
        &mut a,
        Msg::Key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE)),
    );
    assert_eq!(
        cmds,
        vec![Cmd::PlanWingetInstall {
            id: "Git.Git".into()
        }]
    );
    let job = install("Git.Git", None);
    let cmds = update(
        &mut a,
        Msg::WingetPlanned(Ok((
            job.clone(),
            vec!["Licença: EULA — https://x/eula".to_string()],
        ))),
    );
    assert!(cmds.is_empty(), "{cmds:?}");
    match &a.modal {
        Some(Modal::Confirm {
            items,
            notes,
            actions,
            ..
        }) => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].manager, "winget");
            assert_eq!(items[0].to.as_deref(), Some("2.0"));
            assert!(
                notes.contains(&"Licença: EULA — https://x/eula".to_string()),
                "{notes:?}"
            );
            assert!(
                notes.iter().any(|n| n.contains("pedido do UAC")),
                "{notes:?}"
            );
            assert_eq!(
                actions,
                &vec![wingetrs::tui::app::Action::Bridge(job.clone())]
            );
        }
        _ => panic!("no confirmation"),
    }
    // Confirming queues the install as a bridge job.
    let cmds = update(
        &mut a,
        Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Cmd::Execute(wingetrs::tui::app::Action::Bridge(j)) if j.op == job.op
        )),
        "{cmds:?}"
    );
    let running = a.running.as_ref().unwrap();
    assert_eq!(running.label, "Instalando App Git.Git via winget");
}

#[test]
fn tui_winget_plan_error_toasts() {
    use wingetrs::tui::app::update;
    let mut a = tui_app();
    update(&mut a, Msg::WingetPlanned(Err("sem rede".into())));
    assert!(a.modal.is_none());
    assert!(a.toasts.iter().any(|t| t.text.contains("sem rede")));
}
/// A helper process played by a thread running `serve`; its winget answers `code`.
struct ServingHelper {
    reader: BufReader<std::io::PipeReader>,
    writer: Option<std::io::PipeWriter>,
    thread: Option<std::thread::JoinHandle<()>>,
    sink: std::io::Sink,
}

impl ServingHelper {
    fn spawn(code: i32, calls: Arc<std::sync::Mutex<Vec<Call>>>) -> ServingHelper {
        let (req_r, req_w) = std::io::pipe().unwrap();
        let (resp_r, mut resp_w) = std::io::pipe().unwrap();
        let thread = std::thread::spawn(move || {
            let mut req_r = BufReader::new(req_r);
            let mut exec = |p: &str, a: &[String], _: &mut dyn FnMut(&str)| {
                calls.lock().unwrap().push((p.to_string(), a.to_vec()));
                Ok((code, Vec::new()))
            };
            let _ = worker::serve(&mut req_r, &mut resp_w, &mut exec);
        });
        ServingHelper {
            reader: BufReader::new(resp_r),
            writer: Some(req_w),
            thread: Some(thread),
            sink: std::io::sink(),
        }
    }
}

impl Helper for ServingHelper {
    fn reader(&mut self) -> &mut dyn std::io::BufRead {
        &mut self.reader
    }
    fn writer(&mut self) -> &mut dyn std::io::Write {
        match &mut self.writer {
            Some(w) => w,
            None => &mut self.sink,
        }
    }
    fn wait_exit(&mut self, _limit: Duration) -> bool {
        true
    }
    fn alive(&mut self) -> bool {
        true
    }
}

impl Drop for ServingHelper {
    fn drop(&mut self) {
        // Closing the request pipe ends `serve`.
        self.writer = None;
        if let Some(t) = self.thread.take()
            && let Err(p) = t.join()
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(p);
        }
    }
}

#[derive(Clone, Default)]
struct ServingTransport {
    starts: Arc<AtomicUsize>,
    calls: Arc<std::sync::Mutex<Vec<Call>>>,
}

impl Transport for ServingTransport {
    fn start(&self) -> Result<Box<dyn Helper>, String> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(ServingHelper::spawn(
            NO_INSTALLER,
            Arc::clone(&self.calls),
        )))
    }
}

#[test]
fn settings_fallback_runs_unelevated() {
    let (_dir, lock) = msi_lock();
    let t = ServingTransport::default();
    let bridge = ElevatedBridge::new(Box::new(t.clone()));
    let machine = install("Git.Git", Some(InstallScope::Machine));
    let mut local: Vec<Call> = Vec::new();
    let mut exec = |p: &str, a: &[String], on_line: &mut dyn FnMut(&str)| {
        local.push((p.to_string(), a.to_vec()));
        on_line("Found Git [Git.Git] Version 2.0");
        Ok((0, Vec::new()))
    };
    let r = jobs::run_bridge_install(&bridge, &machine, true, &lock, &mut exec, &mut |_: &str| {});
    assert_eq!(r.unwrap().summary, "App Git.Git: instalado via winget");
    assert_eq!(t.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        *t.calls.lock().unwrap(),
        vec![("winget".to_string(), base_with(&["--scope", "machine"]))],
        "only the first run goes through the helper"
    );
    assert_eq!(
        local,
        vec![("winget".to_string(), base_with(&[]))],
        "the retry runs here, without --scope"
    );
    bridge.shutdown(Duration::from_secs(1));
}

#[test]
fn agreements_travel_with_the_job() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use wingetrs::tui::app::{Action, update};
    let with = |agreements: &[&str]| BridgeJob {
        op: BridgeOp::Install {
            scope: None,
            agreements: strings(agreements),
        },
        ..install("Git.Git", None)
    };
    let key = |c| Msg::Key(KeyEvent::new(c, KeyModifiers::NONE));
    let mut a = tui_app();
    // A first plan of the id, cancelled.
    update(
        &mut a,
        Msg::WingetPlanned(Ok((with(&["A https://a"]), vec!["n".into()]))),
    );
    update(&mut a, key(KeyCode::Esc));
    assert!(a.modal.is_none());
    // A second plan of the same id: only its own agreements run.
    update(
        &mut a,
        Msg::WingetPlanned(Ok((with(&["B https://b"]), vec!["n".into()]))),
    );
    let cmds = update(&mut a, key(KeyCode::Enter));
    let ran: Vec<&BridgeJob> = cmds
        .iter()
        .filter_map(|c| match c {
            Cmd::Execute(Action::Bridge(j)) => Some(j),
            _ => None,
        })
        .collect();
    assert_eq!(ran, vec![&with(&["B https://b"])]);

    // And they reach `job.start` from the job itself.
    let log = crate::spans::Log::default();
    let _installed = log.install();
    let (_dir, lock) = msi_lock();
    let bridge = ElevatedBridge::new(Box::new(Refusing));
    let mut exec = |_: &str, _: &[String], on_line: &mut dyn FnMut(&str)| {
        on_line("Found Git [Git.Git] Version 2.0");
        Ok((0, Vec::new()))
    };
    jobs::run_bridge_install(
        &bridge,
        &with(&["B https://b"]),
        false,
        &lock,
        &mut exec,
        &mut |_: &str| {},
    )
    .unwrap();
    let text = log.text();
    let start = text.lines().find(|l| l.contains("job.start")).unwrap();
    assert!(start.contains("agreements=B https://b"), "{text}");
}

const VD: &[u8] = include_bytes!("../fixtures/winget/powertoys.versionData.mszyml");
const PT_MANIFEST: &[u8] = include_bytes!("../fixtures/winget/powertoys-0.101.2362.0.merged.yaml");
const PT_HASH_HEX: &str = "d193b67ac71ce5d456c10f6731447abb6df68ae12ef8691c92af8ed82659e720";

#[tokio::test]
async fn plan_is_one_sequence_for_cli_and_tui() {
    use wingetrs::config::{Behavior, DefaultScope};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(
            "/packages/Microsoft.PowerToys/d193b67a/versionData.mszyml",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(VD.to_vec()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/manifests/m/Microsoft/PowerToys/0.101.2362.0/509b"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(PT_MANIFEST.to_vec()))
        .mount(&server)
        .await;
    let dir = TempDir::new().unwrap();
    let blobs = wingetrs::cache::BlobStore::new(dir.path().join("blobs"));
    let http = wingetrs::http::Http::new().unwrap();
    let pkg = wingetrs::catalog::StoredPackage {
        entry: wingetrs::core::PackageEntry {
            source: wingetrs::core::SourceKind::Winget,
            source_name: "winget".into(),
            id: "Microsoft.PowerToys".into(),
            name: "PowerToys".into(),
            moniker: None,
            version: "0.101.2362.0".into(),
            publisher: None,
            description: None,
            tags: vec![],
            commands: vec![],
        },
        arp_min_version: None,
        arp_max_version: None,
        winget_hash: Some(hex::decode(PT_HASH_HEX).unwrap()),
        raw: None,
    };
    let base = format!("{}/", server.uri());
    let config = dir.path().join("config.toml");
    let user = Behavior::default();
    let machine = Behavior {
        default_scope: DefaultScope::Machine,
        ..user
    };
    let plan = async |explicit: Option<InstallScope>, loaded: Behavior| {
        jobs::plan_winget_install(&http, &blobs, &base, &pkg, &config, loaded, explicit)
            .await
            .unwrap()
    };
    // Default settings: no --scope, no fallback.
    let p = plan(None, user).await;
    assert!(matches!(p.job.op, BridgeOp::Install { scope: None, .. }));
    assert!(!p.scope_fallback);
    let none_cmd = cmd(&p.job);
    assert!(!none_cmd.1.contains(&"--scope".to_string()), "{none_cmd:?}");

    // `machine` in config.toml (what both front ends read): the same command as an explicit
    // `--scope machine`, but only that one may fall back.
    std::fs::write(&config, "[behavior]\ndefault_scope = \"machine\"\n").unwrap();
    let from_settings = plan(None, user).await;
    let explicit = plan(Some(InstallScope::Machine), user).await;
    assert_eq!(cmd(&from_settings.job), cmd(&explicit.job));
    assert_eq!(
        cmd(&from_settings.job).1.last().map(String::as_str),
        Some("machine")
    );
    assert!(from_settings.scope_fallback && !explicit.scope_fallback);
    assert_eq!(from_settings.job.scope, Some(InstallScope::Machine));
    // An explicit scope ignores the settings.
    let explicit_user = plan(Some(InstallScope::User), machine).await;
    assert_eq!(
        cmd(&explicit_user.job).1.last().map(String::as_str),
        Some("user")
    );

    // A config that does not load: the settings already loaded stand in.
    std::fs::write(&config, "[behavior\n").unwrap();
    assert_eq!(cmd(&plan(None, machine).await.job), cmd(&explicit.job));
    assert_eq!(cmd(&plan(None, user).await.job), none_cmd);
}
