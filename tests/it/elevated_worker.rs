//! Task 137: the elevated bridge helper (protocol, helper loop, elevate-or-not choice), over
//! in-memory pipes and fake execs. Nothing here elevates or runs winget, except the ignored
//! live test.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tempfile::TempDir;
use wingetrs::bridge::elevated::{ElevatedBridge, Helper, REFUSED_NOTE, Transport};
use wingetrs::bridge::worker::{self, LINE_CAP, Reply, Request};
use wingetrs::bridge::{self, BridgeJob, Tool};
use wingetrs::install::layout::Layout;
use wingetrs::installed::correlate::InstallScope;
use wingetrs::tui::jobs;

type Call = (String, Vec<String>);
type Calls = Arc<Mutex<Vec<Call>>>;
/// `serve`'s result, the exec calls and the replies written.
type Served = (std::io::Result<()>, Vec<Call>, Vec<Reply>);

fn job(tool: Tool, target: &str, scope: Option<InstallScope>) -> BridgeJob {
    BridgeJob {
        tool,
        target: target.into(),
        name: format!("App {target}"),
        installed: None,
        available: None,
        scope,
    }
}

fn winget_command(target: &str) -> (String, Vec<String>) {
    bridge::command(
        &job(Tool::Winget, target, None),
        Path::new("C:/nao-usado"),
        "pwsh",
    )
}

fn line(req: &Request) -> String {
    worker::encode(req).unwrap()
}

/// Runs `serve` over `input` with an exec that records its calls, prints two lines and exits
/// with 7; returns serve's result, the calls and the replies written.
fn serve_on(input: &str) -> Served {
    let mut calls = Vec::new();
    let mut exec = |p: &str, a: &[String], on_line: &mut dyn FnMut(&str)| {
        calls.push((p.to_string(), a.to_vec()));
        on_line("um");
        on_line("dois");
        Ok((7, Vec::new()))
    };
    let mut reader = BufReader::new(input.as_bytes());
    let mut out = Vec::new();
    let r = worker::serve(&mut reader, &mut out, &mut exec);
    let text = String::from_utf8(out).unwrap();
    let replies = text
        .lines()
        .map(|l| worker::decode::<Reply>(l).unwrap())
        .collect();
    (r, calls, replies)
}

fn ran(code: i32) -> Vec<Reply> {
    vec![
        Reply::Line { text: "um".into() },
        Reply::Line {
            text: "dois".into(),
        },
        Reply::End { code },
    ]
}

#[test]
fn serve_runs_only_the_fixed_winget_command() {
    let input = line(&Request::Upgrade {
        target: "Git.Git".into(),
    });
    let (r, calls, replies) = serve_on(&input);
    r.unwrap();
    assert_eq!(calls, vec![winget_command("Git.Git")]);
    assert_eq!(calls[0].1[..3], ["upgrade", "--id", "Git.Git"]);
    assert_eq!(replies, ran(7));
}

#[test]
fn serve_refuses_bad_targets() {
    let bad = ["a b", "\"x\"", "x&y", "--override", r"C:\x", ""];
    let input: String = bad
        .iter()
        .map(|t| {
            line(&Request::Upgrade {
                target: t.to_string(),
            })
        })
        .collect();
    let (r, calls, replies) = serve_on(&input);
    r.unwrap();
    assert!(calls.is_empty(), "{calls:?}");
    assert_eq!(replies.len(), bad.len(), "{replies:?}");
    for reply in &replies {
        assert!(
            matches!(
                reply,
                Reply::Error {
                    not_found: false,
                    ..
                }
            ),
            "{reply:?}"
        );
    }
}

/// `serve` stops at `bad` and never runs the valid request after it.
fn stops_at(bad: &str) {
    let input = format!("{bad}\n{}", line(&Request::SourceUpdate {}));
    let (r, calls, replies) = serve_on(&input);
    assert!(r.is_err(), "{bad:.80}");
    assert!(calls.is_empty(), "{bad:.80}: {calls:?}");
    assert!(replies.is_empty(), "{bad:.80}: {replies:?}");
}

#[test]
fn decode_refuses_unknown_op_extra_fields_and_long_lines() {
    stops_at(r#"{"op":"run","cmd":"calc"}"#);
    stops_at(r#"{"op":"upgrade","target":"Git.Git","cmd":"calc"}"#);
    stops_at(r#"{"op":"source_update","cmd":"calc"}"#);
    stops_at(r#"{"op":"quit","cmd":"calc"}"#);
    stops_at("not json");
    // A line of exactly LINE_CAP bytes is read; one more byte and it is refused unparsed.
    let base = r#"{"op":"source_update"}"#;
    let exact = format!("{base}{}", " ".repeat(LINE_CAP - base.len()));
    assert_eq!(exact.len(), LINE_CAP);
    let (r, calls, _) = serve_on(&format!("{exact}\n"));
    r.unwrap();
    assert_eq!(calls, vec![bridge::winget_source_update()]);
    stops_at(&format!("{exact} "));
    // Too long without a newline at all (the end of the stream).
    let input = format!("{exact} ");
    let (r, calls, _) = serve_on(&input);
    assert!(r.is_err());
    assert!(calls.is_empty());
}

#[test]
fn encode_refuses_a_message_over_the_cap() {
    let long = Request::Upgrade {
        target: "a".repeat(LINE_CAP),
    };
    assert!(worker::encode(&long).is_err());
}

#[test]
fn source_update_is_the_136_command() {
    let (r, calls, replies) = serve_on(&line(&Request::SourceUpdate {}));
    r.unwrap();
    assert_eq!(calls, vec![bridge::winget_source_update()]);
    assert_eq!(replies, ran(7));
}

#[test]
fn serve_stops_at_quit() {
    let input = format!(
        "{}{}",
        line(&Request::Quit {}),
        line(&Request::SourceUpdate {})
    );
    let (r, calls, replies) = serve_on(&input);
    r.unwrap();
    assert!(calls.is_empty());
    assert!(replies.is_empty());
}

#[test]
fn serve_reports_exec_errors_with_their_kind() {
    let mut exec = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "sem winget",
        ))
    };
    let input = line(&Request::SourceUpdate {});
    let mut out = Vec::new();
    worker::serve(&mut BufReader::new(input.as_bytes()), &mut out, &mut exec).unwrap();
    let reply: Reply = worker::decode(String::from_utf8(out).unwrap().trim_end()).unwrap();
    assert!(
        matches!(
            reply,
            Reply::Error {
                not_found: true,
                ..
            }
        ),
        "{reply:?}"
    );
}

/// A helper process played by a thread running `serve` over two in-memory pipes.
struct FakeHelper {
    reader: BufReader<std::io::PipeReader>,
    writer: Box<dyn Write + Send>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FakeHelper {
    /// `lost`: the helper dies before answering anything.
    fn spawn(calls: Calls, lost: bool) -> FakeHelper {
        let (req_r, req_w) = std::io::pipe().unwrap();
        let (resp_r, resp_w) = std::io::pipe().unwrap();
        let thread = std::thread::spawn(move || {
            if lost {
                return;
            }
            let mut resp_w = resp_w;
            let mut exec = |p: &str, a: &[String], on_line: &mut dyn FnMut(&str)| {
                calls.lock().unwrap().push((p.to_string(), a.to_vec()));
                on_line("elevado");
                Ok((0, Vec::new()))
            };
            let _ = worker::serve(&mut BufReader::new(req_r), &mut resp_w, &mut exec);
        });
        FakeHelper {
            reader: BufReader::new(resp_r),
            writer: Box::new(req_w),
            thread: Some(thread),
        }
    }

    fn join(&mut self) {
        self.writer = Box::new(std::io::sink());
        if let Some(t) = self.thread.take()
            && let Err(p) = t.join()
        {
            std::panic::resume_unwind(p);
        }
    }
}

impl Helper for FakeHelper {
    fn reader(&mut self) -> &mut dyn std::io::BufRead {
        &mut self.reader
    }
    fn writer(&mut self) -> &mut dyn Write {
        &mut self.writer
    }
    fn wait_exit(&mut self, _limit: Duration) -> bool {
        self.join();
        true
    }
}

impl Drop for FakeHelper {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.join();
        }
    }
}

#[derive(Clone, Default)]
struct FakeTransport {
    starts: Arc<AtomicUsize>,
    refuse: bool,
    /// The first helper started dies at once.
    first_lost: bool,
    calls: Calls,
}

impl Transport for FakeTransport {
    fn start(&self) -> Result<Box<dyn Helper>, String> {
        let n = self.starts.fetch_add(1, Ordering::SeqCst);
        if self.refuse {
            return Err("recusado".into());
        }
        Ok(Box::new(FakeHelper::spawn(
            Arc::clone(&self.calls),
            self.first_lost && n == 0,
        )))
    }
}

struct World {
    _dir: TempDir,
    msi_lock: PathBuf,
    scoop_root: PathBuf,
    current: Calls,
}

impl World {
    fn new() -> World {
        let dir = TempDir::new().unwrap();
        World {
            msi_lock: Layout::new(dir.path()).msi_lock(),
            scoop_root: dir.path().join("scoop"),
            _dir: dir,
            current: Calls::default(),
        }
    }

    /// Runs `job` through `bridge`; the current exec records into `self.current`. Returns
    /// the job's result and its progress lines.
    fn run(
        &self,
        bridge: &ElevatedBridge,
        job: &BridgeJob,
    ) -> (
        Result<wingetrs::tui::app::Done, wingetrs::tui::app::InstallFail>,
        Vec<String>,
    ) {
        let current = Arc::clone(&self.current);
        let mut exec = move |p: &str, a: &[String], on_line: &mut dyn FnMut(&str)| {
            current.lock().unwrap().push((p.to_string(), a.to_vec()));
            on_line("atual");
            Ok((0, Vec::new()))
        };
        let mut seen = Vec::new();
        let r = jobs::run_bridge_elevated(
            bridge,
            job,
            Some(&self.scoop_root),
            &self.msi_lock,
            "pwsh",
            &mut exec,
            &mut |l: &str| seen.push(l.to_string()),
        );
        (r, seen)
    }

    fn current(&self) -> Vec<(String, Vec<String>)> {
        self.current.lock().unwrap().clone()
    }
}

#[test]
fn machine_scope_goes_elevated_others_do_not() {
    let w = World::new();
    let t = FakeTransport::default();
    let bridge = ElevatedBridge::new(Box::new(t.clone()));
    let others = [
        job(Tool::Winget, "User.App", Some(InstallScope::User)),
        job(Tool::Winget, "None.App", None),
        job(Tool::Scoop, "scoopapp", Some(InstallScope::Machine)),
    ];
    for j in &others {
        let (_, seen) = w.run(&bridge, j);
        assert_eq!(seen, ["atual"], "{}", j.target);
    }
    assert_eq!(t.starts.load(Ordering::SeqCst), 0);
    assert_eq!(w.current().len(), 3);
    assert_eq!(w.current()[0], winget_command("User.App"));
    assert_eq!(w.current()[2].0, "pwsh");

    let machine = ["A.A", "B.B", "C.C"];
    for target in machine {
        let (r, seen) = w.run(
            &bridge,
            &job(Tool::Winget, target, Some(InstallScope::Machine)),
        );
        r.unwrap();
        assert_eq!(seen, ["elevado"], "{target}");
    }
    assert_eq!(
        t.starts.load(Ordering::SeqCst),
        1,
        "one elevation for three jobs"
    );
    let elevated: Vec<_> = machine.iter().map(|t| winget_command(t)).collect();
    assert_eq!(*t.calls.lock().unwrap(), elevated);
    assert_eq!(w.current().len(), 3, "no machine job ran unelevated");

    // A user job after the helper started still runs unelevated.
    let (_, seen) = w.run(&bridge, &others[0]);
    assert_eq!(seen, ["atual"]);
    assert_eq!(t.calls.lock().unwrap().len(), 3);
}

#[test]
fn refused_elevation_falls_back_once() {
    let w = World::new();
    let t = FakeTransport {
        refuse: true,
        ..FakeTransport::default()
    };
    let bridge = ElevatedBridge::new(Box::new(t.clone()));
    let first = job(Tool::Winget, "A.A", Some(InstallScope::Machine));
    let (r, seen) = w.run(&bridge, &first);
    r.unwrap();
    assert_eq!(seen, [REFUSED_NOTE, "atual"]);
    assert_eq!(w.current(), vec![winget_command("A.A")]);
    for target in ["B.B", "C.C"] {
        let (r, seen) = w.run(
            &bridge,
            &job(Tool::Winget, target, Some(InstallScope::Machine)),
        );
        r.unwrap();
        assert_eq!(seen, ["atual"], "{target}");
    }
    assert_eq!(t.starts.load(Ordering::SeqCst), 1, "never asked again");
    assert_eq!(w.current().len(), 3);
}

#[test]
fn a_lost_helper_fails_its_job_and_is_started_again() {
    let w = World::new();
    let t = FakeTransport {
        first_lost: true,
        ..FakeTransport::default()
    };
    let bridge = ElevatedBridge::new(Box::new(t.clone()));
    let (r, _) = w.run(
        &bridge,
        &job(Tool::Winget, "A.A", Some(InstallScope::Machine)),
    );
    let err = format!("{:?}", r.unwrap_err());
    assert!(err.contains("auxiliar elevado"), "{err}");
    assert!(w.current().is_empty(), "no silent unelevated run");
    let (r, seen) = w.run(
        &bridge,
        &job(Tool::Winget, "B.B", Some(InstallScope::Machine)),
    );
    r.unwrap();
    assert_eq!(seen, ["elevado"]);
    assert_eq!(t.starts.load(Ordering::SeqCst), 2);
}

#[test]
fn shutdown_sends_quit_and_waits() {
    let t = FakeTransport::default();
    let bridge = ElevatedBridge::new(Box::new(t.clone()));
    let w = World::new();
    w.run(
        &bridge,
        &job(Tool::Winget, "A.A", Some(InstallScope::Machine)),
    )
    .0
    .unwrap();
    assert!(bridge.shutdown(Duration::from_secs(5)));
    // Nothing started: nothing to wait for.
    let idle = ElevatedBridge::new(Box::new(FakeTransport::default()));
    assert!(idle.shutdown(Duration::from_secs(5)));
}

#[test]
fn elevated_exec_refuses_commands_outside_the_protocol() {
    let t = FakeTransport::default();
    let bridge = ElevatedBridge::new(Box::new(t.clone()));
    let j = job(Tool::Winget, "A.A", Some(InstallScope::Machine));
    let mut fallback = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| Ok((0, Vec::new()));
    let mut chosen = bridge.exec_for(&j, &mut fallback, &mut |_: &str| {});
    assert!(chosen.elevated());
    let exec = chosen.exec();
    let other = winget_command("B.B");
    let e = exec(&other.0, &other.1, &mut |_: &str| {}).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
    let e = exec("cmd", &["/c".into(), "calc".into()], &mut |_: &str| {}).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
    assert!(t.calls.lock().unwrap().is_empty());
}

#[test]
fn job_log_marks_elevated() {
    let log = crate::spans::Log::default();
    let _installed = log.install();
    let w = World::new();
    let bridge = ElevatedBridge::new(Box::new(FakeTransport::default()));
    w.run(
        &bridge,
        &job(Tool::Winget, "Elev.App", Some(InstallScope::Machine)),
    )
    .0
    .unwrap();
    w.run(
        &bridge,
        &job(Tool::Winget, "User.App", Some(InstallScope::User)),
    )
    .0
    .unwrap();
    let text = log.text();
    let end = |target: &str| {
        text.lines()
            .find(|l| l.contains("job.end") && l.contains(&format!("target={target} ")))
            .unwrap_or_else(|| panic!("sem job.end de {target}: {text}"))
            .to_string()
    };
    assert!(end("Elev.App").contains(" elevated=true "), "{text}");
    assert!(end("User.App").contains(" elevated=false "), "{text}");
}

/// Machine-scope winget ids to upgrade, from `WINGETRS_LIVE_MACHINE_IDS` (comma-separated).
#[cfg(windows)]
#[test]
#[ignore = "live: pede UAC"]
fn live_single_uac_for_two_jobs() {
    let ids = std::env::var("WINGETRS_LIVE_MACHINE_IDS")
        .expect("WINGETRS_LIVE_MACHINE_IDS=Id.Um,Id.Dois (apps de escopo máquina com atualização)");
    let ids: Vec<&str> = ids.split(',').map(str::trim).collect();
    assert_eq!(ids.len(), 2, "dois ids");
    struct Counting(
        Arc<AtomicUsize>,
        wingetrs::bridge::elevated::SystemTransport,
    );
    impl Transport for Counting {
        fn start(&self) -> Result<Box<dyn Helper>, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            self.1.start()
        }
    }
    let starts = Arc::new(AtomicUsize::new(0));
    let transport = wingetrs::bridge::elevated::SystemTransport::new(PathBuf::from(env!(
        "CARGO_BIN_EXE_wingetrs"
    )));
    let bridge = ElevatedBridge::new(Box::new(Counting(Arc::clone(&starts), transport)));
    let dir = TempDir::new().unwrap();
    let msi_lock = Layout::new(dir.path()).msi_lock();
    for id in ids {
        let mut current = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| -> std::io::Result<_> {
            panic!("o job de escopo máquina rodou sem elevar")
        };
        let mut lines = Vec::new();
        let r = jobs::run_bridge_elevated(
            &bridge,
            &job(Tool::Winget, id, Some(InstallScope::Machine)),
            None,
            &msi_lock,
            "pwsh",
            &mut current,
            &mut |l: &str| lines.push(l.to_string()),
        );
        println!("{id}: {r:?}\n{}", lines.join("\n"));
        assert!(!lines.iter().any(|l| l == REFUSED_NOTE), "UAC recusado");
    }
    assert_eq!(starts.load(Ordering::SeqCst), 1, "um UAC para dois jobs");
    assert!(bridge.shutdown(Duration::from_secs(5)), "o auxiliar saiu");
}
