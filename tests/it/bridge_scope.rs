#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Task 205: retry without --scope when winget 1.29 crashes on it.

use std::path::PathBuf;

use tempfile::TempDir;
use wingetrs::bridge::elevated::{ElevatedBridge, Helper, Transport};
use wingetrs::bridge::{self, BridgeJob, BridgeOp, Tool};
use wingetrs::install::layout::Layout;
use wingetrs::installed::correlate::InstallScope;
use wingetrs::tui::app::InstallFail;
use wingetrs::tui::jobs;

type Call = (String, Vec<String>);

const CRASH: i32 = bridge::WINGET_SCOPE_CRASH as i32;
const NO_INSTALLER: i32 = bridge::NO_APPLICABLE_INSTALLER as i32;
const INSTALLER_FAILED: i32 = 0x8A15_0006_u32 as i32;

fn install(scope: Option<InstallScope>) -> BridgeJob {
    BridgeJob {
        tool: Tool::Winget,
        op: BridgeOp::Install {
            scope,
            agreements: Vec::new(),
        },
        target: "Git.Git".into(),
        name: "App Git.Git".into(),
        installed: None,
        available: Some("2.0".into()),
        scope,
    }
}

struct Refusing;

impl Transport for Refusing {
    fn start(&self) -> Result<Box<dyn Helper>, String> {
        Err("recusado".into())
    }
}

fn msi_lock() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = Layout::new(dir.path()).msi_lock();
    (dir, path)
}

/// Runs `job` against a fake winget answering each call with the next exit code.
fn run(
    codes: &[i32],
    fallback: bool,
    job: &BridgeJob,
) -> (Result<String, InstallFail>, Vec<Call>, Vec<String>) {
    let (_dir, lock) = msi_lock();
    // Refuses elevation: every run goes through the fake exec below.
    let bridge = ElevatedBridge::new(Box::new(Refusing));
    let mut answers = codes.to_vec();
    let mut calls = Vec::new();
    let mut seen = Vec::new();
    let mut exec = |p: &str, a: &[String], on_line: &mut dyn FnMut(&str)| {
        calls.push((p.to_string(), a.to_vec()));
        let line = "Found Git [Git.Git] Version 2.0".to_string();
        on_line(&line);
        Ok((answers.remove(0), vec![line]))
    };
    let r = jobs::run_bridge_install(&bridge, job, fallback, &lock, &mut exec, &mut |l: &str| {
        seen.push(l.to_string())
    });
    (r.map(|d| d.summary), calls, seen)
}

fn has_scope(call: &Call) -> bool {
    call.1.iter().any(|a| a == "--scope")
}

#[test]
fn scope_crash_retries_without_scope_from_settings() {
    let (r, calls, seen) = run(&[CRASH, 0], true, &install(Some(InstallScope::Machine)));
    assert_eq!(r.unwrap(), "App Git.Git: instalado via winget");
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(has_scope(&calls[0]) && !has_scope(&calls[1]), "{calls:?}");
    assert!(
        seen.contains(&bridge::SCOPE_CRASH_NOTE.to_string()),
        "{seen:?}"
    );
}

#[test]
fn scope_crash_retries_without_scope_when_explicit() {
    for scope in [InstallScope::Machine, InstallScope::User] {
        let (r, calls, seen) = run(&[CRASH, 0], false, &install(Some(scope)));
        r.unwrap();
        assert_eq!(calls.len(), 2, "{calls:?}");
        assert!(has_scope(&calls[0]) && !has_scope(&calls[1]), "{calls:?}");
        assert!(
            seen.contains(&bridge::SCOPE_CRASH_NOTE.to_string()),
            "{seen:?}"
        );
    }
}

#[test]
fn scope_crash_without_scope_does_not_retry() {
    let (r, calls, seen) = run(&[CRASH], false, &install(None));
    match r {
        Err(InstallFail::Other(m)) => {
            assert!(m.contains("falhou internamente (0x8000FFFF)"), "{m}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(!seen.contains(&bridge::SCOPE_CRASH_NOTE.to_string()));
}

#[test]
fn second_attempt_fails_for_another_reason_reports_that_reason() {
    let (r, calls, _) = run(
        &[CRASH, INSTALLER_FAILED],
        false,
        &install(Some(InstallScope::User)),
    );
    match r {
        Err(InstallFail::Other(m)) => {
            assert!(m.contains("O instalador falhou."), "{m}");
            assert!(!m.contains("0x8000FFFF"), "{m}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(calls.len(), 2, "{calls:?}");
}

#[test]
fn no_applicable_installer_rule_unchanged() {
    let machine = install(Some(InstallScope::Machine));
    // From the Settings default: retried once, with the 176 note.
    let (r, calls, seen) = run(&[NO_INSTALLER, 0], true, &machine);
    r.unwrap();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(seen.contains(&bridge::SCOPE_FALLBACK_NOTE.to_string()));
    assert!(!seen.contains(&bridge::SCOPE_CRASH_NOTE.to_string()));
    // Explicit scope: never dropped.
    let (r, calls, _) = run(&[NO_INSTALLER], false, &machine);
    r.unwrap_err();
    assert_eq!(calls.len(), 1, "{calls:?}");
}
