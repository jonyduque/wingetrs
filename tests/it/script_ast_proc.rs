//! Task 227: the parser process of the script gate. Each test runs a fake `.cmd` written into a
//! temp folder, never PowerShell.
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use wingetrs::install::script_ast::{self, AstError, AstGate};
use wingetrs::install::script_ast_win::{PwshAst, host_args};
use wingetrs::install::script_policy::GpoValues;

fn not_elevated() -> io::Result<bool> {
    Ok(false)
}

fn no_gpo(_: &Path) -> GpoValues {
    GpoValues::default()
}

/// `body` as `<tmp>\fake.cmd` (CRLF lines, `@echo off` first).
fn fake(body: &str) -> (TempDir, PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("fake.cmd");
    let text = format!("@echo off\r\n{}\r\n", body.replace('\n', "\r\n"));
    std::fs::write(&p, text).unwrap();
    (t, p)
}

fn ast(program: PathBuf, first: Duration, each: Duration) -> PwshAst {
    PwshAst::with_checks(program, not_elevated, no_gpo, first, each)
}

#[test]
fn fake_parser_reply_is_used() {
    let (t, p) = fake(
        "set /p L=\necho x>>\"%~dp0marks.txt\"\necho CMD\t1\tnone\tStringConstant\t.\tiex\necho END\t1",
    );
    let gate = ast(p, Duration::from_secs(20), Duration::from_secs(10));
    let facts = gate.facts("q").unwrap();
    assert_eq!(facts.commands.len(), 1);
    assert_eq!(facts.commands[0].name.as_deref(), Some("iex"));
    // Cached: the second call sends nothing (the fake has exited by now anyway).
    assert_eq!(gate.facts("q").unwrap(), facts);
    let marks = std::fs::read_to_string(t.path().join("marks.txt")).unwrap();
    assert_eq!(marks.lines().count(), 1, "{marks:?}");
}

#[test]
fn fake_parser_timeout_refuses() {
    let (t, p) = fake("ping -n 30 127.0.0.1 > \"%~dp0ping.txt\"");
    let gate = ast(p, Duration::from_secs(1), Duration::from_secs(1));
    let start = Instant::now();
    let got = gate.facts("q");
    assert!(matches!(got, Err(AstError::Timeout(_))), "{got:?}");
    // A job left alive would hold the pipe (and this call) until `ping` ends, 30 s later.
    let took = start.elapsed();
    let fast = took < Duration::from_secs(5); // clock-ok: the regression is a 30 s wait
    assert!(fast, "{took:?}");
    // `ping` holds its output file open while it runs: deleting it proves it is gone.
    let out = t.path().join("ping.txt");
    let mut removed = std::fs::remove_file(&out);
    for _ in 0..20 {
        if removed.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        removed = std::fs::remove_file(&out);
    }
    removed.unwrap();
}

#[test]
fn fake_parser_dead_refuses() {
    let (_t, p) = fake("exit /b 0");
    let gate = ast(p, Duration::from_secs(20), Duration::from_secs(10));
    let got = gate.facts("q");
    assert!(matches!(got, Err(AstError::Protocol(_))), "{got:?}");
}

/// An answer for another request id is a broken answer, and the next text starts a new process.
#[test]
fn fake_parser_wrong_id_refuses_and_restarts() {
    let (t, p) = fake("echo x>>\"%~dp0starts.txt\"\nset /p L=\necho END\t7");
    let gate = ast(p, Duration::from_secs(20), Duration::from_secs(10));
    assert!(matches!(gate.facts("a"), Err(AstError::Protocol(_))));
    assert!(matches!(gate.facts("b"), Err(AstError::Protocol(_))));
    let starts = std::fs::read_to_string(t.path().join("starts.txt")).unwrap();
    assert_eq!(starts.lines().count(), 2, "{starts:?}");
}

#[test]
fn fake_parser_bad_record_refuses() {
    let (_t, p) = fake("set /p L=\necho BOGUS\t1\necho END\t1");
    let gate = ast(p, Duration::from_secs(20), Duration::from_secs(10));
    assert!(matches!(gate.facts("q"), Err(AstError::Protocol(_))));
}

#[test]
fn elevated_starts_no_parser() {
    fn elevated() -> io::Result<bool> {
        Ok(true)
    }
    fn unknown() -> io::Result<bool> {
        Err(io::Error::other("token"))
    }
    for check in [elevated as fn() -> io::Result<bool>, unknown] {
        let (t, p) = fake("echo x>\"%~dp0started.txt\"\nset /p L=\necho END\t1");
        let gate = PwshAst::with_checks(
            p,
            check,
            no_gpo,
            Duration::from_secs(20),
            Duration::from_secs(10),
        );
        assert_eq!(gate.facts("q"), Err(AstError::Elevated));
        drop(gate);
        assert!(!t.path().join("started.txt").exists());
        // The folder is there: the fake would have written into it.
        assert!(t.path().join("fake.cmd").is_file());
    }
}

#[test]
fn policy_and_missing_program_start_no_parser() {
    fn blocked(_: &Path) -> GpoValues {
        GpoValues {
            machine_enable_scripts: Some(0),
            ..GpoValues::default()
        }
    }
    let (t, p) = fake("echo x>\"%~dp0started.txt\"\nset /p L=\necho END\t1");
    let gate = PwshAst::with_checks(
        p,
        not_elevated,
        blocked,
        Duration::from_secs(20),
        Duration::from_secs(10),
    );
    assert!(matches!(gate.facts("q"), Err(AstError::Policy(_))));
    drop(gate);
    assert!(!t.path().join("started.txt").exists());
    for program in [PathBuf::from("fake.cmd"), t.path().join("missing.cmd")] {
        let gate = ast(program, Duration::from_secs(20), Duration::from_secs(10));
        assert_eq!(gate.facts("q"), Err(AstError::NoPowerShell));
    }
}

/// A `using` the parser would resolve never reaches the process: refused without starting it.
#[test]
fn using_module_starts_no_parser() {
    let (t, p) = fake("echo x>\"%~dp0started.txt\"\nset /p L=\necho END\t1");
    let gate = ast(p, Duration::from_secs(20), Duration::from_secs(10));
    let facts = gate.facts("using assembly C:\\x\\nao_existe.dll").unwrap();
    assert_eq!(facts.usings.len(), 1);
    assert_eq!(facts.usings[0].kind, "module");
    drop(gate);
    assert!(!t.path().join("started.txt").exists());
    assert!(t.path().join("fake.cmd").is_file());
}

#[test]
fn encoded_host_fits_command_line() {
    let args = host_args();
    let at = args.iter().position(|a| a == "-EncodedCommand").unwrap();
    assert_eq!(at + 2, args.len());
    assert!(args[at + 1].len() < 30_000, "{}", args[at + 1].len());
    assert_eq!(&args[..at], ["-NoLogo", "-NoProfile", "-NonInteractive"]);
}

#[test]
fn host_script_runs_nothing() {
    let host = script_ast::HOST_SCRIPT;
    let lower = host.to_lowercase();
    for word in [
        "invoke",
        "getscriptblock",
        "::create",
        "iex",
        "set-content",
        "out-file",
        "new-item",
        "start-process",
    ] {
        assert!(!lower.contains(word), "{word}");
    }
    // No cmdlet (`Verb-Noun`) outside comments.
    for (i, line) in host.lines().enumerate() {
        let code: Vec<char> = line.split('#').next().unwrap_or("").chars().collect();
        for j in 1..code.len().saturating_sub(1) {
            assert!(
                !(code[j] == '-'
                    && code[j - 1].is_ascii_alphabetic()
                    && code[j + 1].is_ascii_alphabetic()),
                "line {}: {line}",
                i + 1
            );
        }
    }
    assert!(host.contains("$PSModuleAutoLoadingPreference = 'None'"));
    // The host's own `using` check comes before the only parse.
    let check = host.find("[regex]::Match($plain, $usingLoad)").unwrap();
    let parse = host.find("::ParseInput(").unwrap();
    assert!(check < parse);
    assert_eq!(host.matches("::ParseInput(").count(), 1);
}
