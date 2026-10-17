#![allow(clippy::unwrap_used, clippy::expect_used)]
//! `job.start` / `job.end` log lines of the bridge and the CLI, and the winget version check.
use crate::spans::Log;
use std::path::PathBuf;
use tempfile::TempDir;
use wingetrs::bridge::{self, BridgeJob, Tool};
use wingetrs::install::layout::Layout;
use wingetrs::tui::app::InstallFail;
use wingetrs::tui::jobs;
use wiremock::MockServer;

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn job(available: Option<&str>) -> BridgeJob {
    BridgeJob {
        tool: Tool::Winget,
        target: "Google.Antigravity".into(),
        name: "Antigravity".into(),
        installed: Some("2.20.0".into()),
        available: available.map(str::to_string),
    }
}

/// A fresh `msi.lock` path per call; the directory goes away with the value.
fn msi_lock() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = Layout::new(dir.path()).msi_lock();
    (dir, path)
}

/// Runs the bridge job with a fake winget and returns the result and the log text.
fn run(job: &BridgeJob, code: i32, output: Vec<String>) -> (Result<String, InstallFail>, String) {
    let log = Log::default();
    let _installed = log.install();
    let (_dir, lock) = msi_lock();
    let mut exec = |_: &str, _: &[String], _: &mut dyn FnMut(&str)| Ok((code, output.clone()));
    let r = jobs::run_bridge(job, None, &lock, "pwsh", &mut exec, &mut |_: &str| {});
    (r.map(|d| d.summary), log.text())
}

#[test]
fn chosen_version_pt_and_en() {
    let pt = lines(&["Encontrado Git [Git.Git] Versão 2.55.0.5"]);
    let en = lines(&["Found Git [Git.Git] Version 2.55.0.5"]);
    let color = lines(&["\u{1b}[32mFound Git [Git.Git] Version 2.55.0.5\u{1b}[0m"]);
    for l in [pt, en, color] {
        assert_eq!(
            bridge::winget_chosen_version("Git.Git", &l).as_deref(),
            Some("2.55.0.5")
        );
    }
    let other = lines(&["Found Other [Other.Id] Version 1.0"]);
    assert_eq!(bridge::winget_chosen_version("Git.Git", &other), None);
    let no_version = lines(&["[Git.Git] Successfully installed"]);
    assert_eq!(bridge::winget_chosen_version("Git.Git", &no_version), None);
}

#[test]
fn stale_winget_source_is_not_success() {
    let out = lines(&["Encontrado Antigravity [Google.Antigravity] Versão 2.23.1"]);
    let (r, text) = run(&job(Some("2.24.0")), 0, out);
    assert_eq!(
        r,
        Err(InstallFail::Other(
            "Antigravity: O winget instalou 2.23.1, não 2.24.0: o catálogo local do winget está mais velho que o do wingetrs. Tente de novo mais tarde.".into()
        ))
    );
    assert!(text.contains("outcome=sem_efeito"), "{text}");
    assert!(text.contains("WARN"), "{text}");
    assert!(text.contains("after=2.23.1"), "{text}");
}

#[test]
fn same_version_ok_when_available_unknown() {
    let out = lines(&["Encontrado Antigravity [Google.Antigravity] Versão 2.23.1"]);
    let (r, text) = run(&job(None), 0, out.clone());
    assert!(r.is_ok(), "{r:?}");
    assert!(text.contains("outcome=ok"), "{text}");
    let (r, _) = run(&job(Some("2.23.1")), 0, out);
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn start_and_end_logged_on_success() {
    let out = lines(&["Encontrado Antigravity [Google.Antigravity] Versão 2.24.0"]);
    let (r, text) = run(&job(Some("2.24.0")), 0, out);
    assert!(r.is_ok(), "{r:?}");
    let start = text.lines().find(|l| l.contains("job.start")).unwrap();
    assert!(start.contains("kind=bridge"), "{start}");
    assert!(start.contains("tool=winget"), "{start}");
    assert!(start.contains("from=2.20.0"), "{start}");
    assert!(start.contains("to=2.24.0"), "{start}");
    assert!(
        start.contains("cmd=winget upgrade --id Google.Antigravity --exact"),
        "{start}"
    );
    let end = text.lines().find(|l| l.contains("job.end")).unwrap();
    assert!(end.contains("INFO"), "{end}");
    assert!(end.contains("outcome=ok"), "{end}");
    assert!(end.contains("ms="), "{end}");
    assert!(end.contains("code=0"), "{end}");
}

#[test]
fn failure_keeps_last_40_lines_and_installer_log() {
    let mut out: Vec<String> = (1..=60).map(|n| format!("linha {n}")).collect();
    out[10] = r"O registro do instalador está disponível em: C:\x\Git.log".into();
    let (r, text) = run(&job(Some("2.24.0")), 0x8A15_0006_u32 as i32, out);
    assert!(r.is_err());
    let end = text.lines().find(|l| l.contains("job.end")).unwrap();
    assert!(end.contains("WARN"), "{end}");
    assert!(end.contains("outcome=falhou"), "{end}");
    assert!(end.contains(r"installer_log=C:\x\Git.log"), "{end}");
    assert!(text.contains("linha 21\n"), "{text}");
    assert!(text.contains("linha 60"), "{text}");
    assert!(!text.contains("linha 20\n"), "{text}");
    assert!(!text.contains("bridge upgrade failed"), "{text}");
}

#[test]
fn upgrade_not_allowed_message() {
    assert_eq!(
        bridge::winget_message(0x8A15_0114_u32 as i32),
        "O editor não permite atualizar este pacote pelo winget; use o atualizador do próprio app."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cli_job_logged() {
    let server = MockServer::start().await;
    let lab = crate::cli_install::Lab::new(&server, "joblog").await;
    // The MSI the own extractor refuses: the install fails before touching anything real.
    let out = tokio::task::block_in_place(|| lab.run(&["install", "m", "--yes"]));
    assert!(!out.status.success());
    let log_dir = lab.root.join("log");
    let text: String = std::fs::read_dir(&log_dir)
        .unwrap()
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect();
    let start = text.lines().find(|l| l.contains("job.start")).unwrap();
    assert!(start.contains("kind=install"), "{start}");
    assert!(start.contains("target=main/m"), "{start}");
    let end = text.lines().find(|l| l.contains("job.end")).unwrap();
    assert!(end.contains("kind=install"), "{end}");
    assert!(end.contains("outcome=falhou"), "{end}");
}

#[test]
fn dropped_job_log_ends_as_cancelled() {
    let log = Log::default();
    let _installed = log.install();
    let started = wingetrs::job_log::JobLog::start(wingetrs::job_log::JobInfo {
        kind: "install",
        tool: "wingetrs",
        target: "main/x".into(),
        name: "x".into(),
        from: None,
        to: None,
        cmd: None,
    });
    drop(started);
    assert!(log.text().contains("outcome=cancelado"), "{}", log.text());
}

#[test]
fn newer_or_equal_winget_version_is_a_success() {
    let newer = lines(&["Encontrado Antigravity [Google.Antigravity] Versão 2.25.0"]);
    let (r, text) = run(&job(Some("2.24.0")), 0, newer);
    assert!(r.is_ok(), "{r:?}");
    assert!(text.contains("outcome=ok"), "{text}");
    let same = lines(&["Encontrado Antigravity [Google.Antigravity] Versão 2.24"]);
    let (r, _) = run(&job(Some("2.24.0")), 0, same);
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn chosen_version_ignores_id_case() {
    let l = lines(&["Found Git [git.git] Version 2.55.0.5"]);
    assert_eq!(
        bridge::winget_chosen_version("Git.Git", &l).as_deref(),
        Some("2.55.0.5")
    );
}

#[test]
fn chosen_version_is_taken_from_the_whole_output_not_only_the_tail() {
    let log = Log::default();
    let _installed = log.install();
    let (_dir, lock) = msi_lock();
    let mut exec = |_: &str, _: &[String], on_line: &mut dyn FnMut(&str)| {
        on_line("Encontrado Antigravity [Google.Antigravity] Versão 2.23.1");
        Ok((0, lines(&["so o fim"])))
    };
    let r = jobs::run_bridge(
        &job(Some("2.24.0")),
        None,
        &lock,
        "pwsh",
        &mut exec,
        &mut |_: &str| {},
    );
    assert!(r.is_err());
    assert!(log.text().contains("outcome=sem_efeito"), "{}", log.text());
}

#[test]
fn panicking_job_ends_as_failed() {
    let log = Log::default();
    let _installed = log.install();
    let r = std::panic::catch_unwind(|| {
        let _job = wingetrs::job_log::JobLog::start(wingetrs::job_log::JobInfo {
            kind: "install",
            tool: "wingetrs",
            target: "main/x".into(),
            name: "x".into(),
            from: None,
            to: None,
            cmd: None,
        });
        panic!("boom");
    });
    assert!(r.is_err());
    let text = log.text();
    let end = text.lines().find(|l| l.contains("job.end")).unwrap();
    assert!(end.contains("outcome=falhou"), "{end}");
    assert!(end.contains("terminou em pânico"), "{end}");
}
