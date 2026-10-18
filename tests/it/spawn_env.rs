#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Phase 2c tests (Task 146): `run_spec` passes environment, working dir and raw bytes.
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;
use wingetrs::bridge::spawn_win::{PIPE_GRACE, RunSpec, run_spec, utf8_lossy};

fn script(dir: &Path, body: &str) -> std::path::PathBuf {
    let p = dir.join("t.cmd");
    std::fs::write(&p, body).unwrap();
    p
}

fn spec<'a>(
    program: &'a Path,
    dir: &'a Path,
    env: &'a [(OsString, OsString)],
    env_remove: &'a [OsString],
    decode: fn(&[u8]) -> String,
) -> RunSpec<'a> {
    RunSpec {
        program,
        args: &[],
        dir,
        env,
        env_remove,
        decode,
        grace: PIPE_GRACE,
    }
}

fn run(s: &RunSpec<'_>) -> (i32, Vec<String>) {
    let mut seen = Vec::new();
    let (code, tail) = run_spec(s, |l| seen.push(l.to_string())).unwrap();
    assert_eq!(seen, tail, "on_line got the same lines the tail kept");
    (code, tail)
}

#[test]
fn env_is_passed_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let p = script(dir.path(), "@echo off\r\necho A=%WR_A%\r\necho B=%OS%\r\n");
    let env = [(OsString::from("WR_A"), OsString::from("1"))];
    let remove = [OsString::from("OS")];
    let (code, lines) = run(&spec(&p, dir.path(), &env, &remove, utf8_lossy));
    assert_eq!(code, 0);
    // In a batch file an undefined `%OS%` expands to nothing (only the interactive prompt
    // leaves it literal); with `OS` inherited the line would be `B=Windows_NT`.
    assert_eq!(lines, ["A=1", "B="]);
}

#[test]
fn cwd_is_dir() {
    let dir = tempfile::tempdir().unwrap();
    let p = script(dir.path(), "@echo off\r\ncd\r\n");
    let (_, lines) = run(&spec(&p, dir.path(), &[], &[], utf8_lossy));
    let got = std::fs::canonicalize(&lines[0]).unwrap();
    let want = std::fs::canonicalize(dir.path()).unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(got, want);
}

/// Test decoder: CP850-like, only `0xC6` (`ã`) differs from ASCII.
fn cp850_ish(b: &[u8]) -> String {
    b.iter()
        .map(|&c| if c == 0xC6 { 'ã' } else { char::from(c) })
        .collect()
}

#[test]
fn decoder_gets_raw_bytes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("data.bin"), [0x6E, 0xC6, 0x6F, 0x0D, 0x0A]).unwrap();
    let p = script(dir.path(), "@type data.bin\r\n");
    let (_, lines) = run(&spec(&p, dir.path(), &[], &[], cp850_ish));
    assert_eq!(lines, ["não"]);
    let (_, lines) = run(&spec(&p, dir.path(), &[], &[], utf8_lossy));
    assert_eq!(lines, ["n\u{FFFD}o"]);
}

#[test]
fn relative_program_refused() {
    let dir = tempfile::tempdir().unwrap();
    script(dir.path(), "@echo off\r\necho oi\r\n");
    let s = RunSpec {
        program: Path::new("t.cmd"),
        args: &[],
        dir: dir.path(),
        env: &[],
        env_remove: &[],
        decode: utf8_lossy,
        grace: Duration::from_secs(2),
    };
    let err = run_spec(&s, |_| {}).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn exit_code_propagates() {
    let dir = tempfile::tempdir().unwrap();
    let p = script(dir.path(), "@echo off\r\nexit /b 3\r\n");
    let (code, _) = run(&spec(&p, dir.path(), &[], &[], utf8_lossy));
    assert_eq!(code, 3);
}
