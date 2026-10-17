//! Upgrade bridge: who goes through winget/scoop, the exact command lines, exit codes and output.
use std::collections::BTreeSet;
use std::path::Path;
use wingetrs::bridge::{
    self, BridgeJob, NOT_OURS_NOTE, PINNED_NOTE, REBOOT_MARKERS, Tail, Tool, bridge_target,
    reboot_requested, scoop_message, valid_target, winget_message,
};
use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};

fn row(name: &str, origin: &str, source: &str, state: UpdateState) -> ListRow {
    ListRow {
        name: name.into(),
        origin: origin.into(),
        installed_version: Some("1".into()),
        correlation: Correlation::Matched {
            source: source.into(),
            package_id: format!("Pub.{name}"),
            rule: Rule::ProductCode,
        },
        latest: Some("2".into()),
        state,
        ..Default::default()
    }
}

fn available() -> UpdateState {
    UpdateState::Available { latest: "2".into() }
}

#[test]
fn bridge_target_rules() {
    let none = BTreeSet::new();
    let arp = bridge_target(&row("Git", "arp", "winget", available()), &none).unwrap();
    assert_eq!(arp.tool, Tool::Winget);
    assert_eq!(arp.target, "Pub.Git");
    assert_eq!(arp.name, "Git");
    assert_eq!(arp.installed.as_deref(), Some("1"));
    assert_eq!(arp.available.as_deref(), Some("2"));
    let msix = bridge_target(&row("Term", "msix", "winget", available()), &none).unwrap();
    assert_eq!(msix.tool, Tool::Winget);
    assert_eq!(msix.target, "Pub.Term");
    let scoop = bridge_target(&row("7zip", "scoop", "scoop:main", available()), &none).unwrap();
    assert_eq!(scoop.tool, Tool::Scoop);
    assert_eq!(scoop.target, "7zip");
    assert!(bridge_target(&row("beta", "wingetrs", "scoop:main", available()), &none).is_err());
    let pins: BTreeSet<String> = ["arp/Git".to_string()].into();
    assert_eq!(
        bridge_target(&row("Git", "arp", "winget", available()), &pins),
        Err(PINNED_NOTE)
    );
    assert_eq!(
        bridge_target(&row("Git", "arp", "winget", UpdateState::UpToDate), &none),
        Err("sem atualização")
    );
    assert_eq!(
        bridge_target(&row("7zip", "arp", "scoop:main", available()), &none),
        Err(NOT_OURS_NOTE)
    );
}

fn job(tool: Tool, target: &str) -> BridgeJob {
    BridgeJob {
        tool,
        target: target.into(),
        name: target.into(),
        installed: None,
        available: None,
    }
}

#[test]
fn command_lines_match_the_probe() {
    let (program, args) = bridge::command(&job(Tool::Winget, "Git.Git"), Path::new("C:/s"), "pwsh");
    assert_eq!(program, "winget");
    assert_eq!(
        args,
        [
            "upgrade",
            "--id",
            "Git.Git",
            "--exact",
            "--source",
            "winget",
            "--silent",
            "--disable-interactivity",
            "--accept-package-agreements",
            "--accept-source-agreements",
        ]
    );
    let root = Path::new(r"C:\Users\o'neil\scoop");
    let (program, args) = bridge::command(&job(Tool::Scoop, "7zip"), root, "powershell");
    assert_eq!(program, "powershell");
    assert_eq!(
        args[..4],
        ["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"]
    );
    assert_eq!(args.len(), 5);
    let script = &args[4];
    assert!(script.starts_with("[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); "));
    assert!(
        script
            .contains(r"& 'C:\Users\o''neil\scoop\apps\scoop\current\bin\scoop.ps1' update 7zip;"),
        "{script}"
    );
    assert!(script.ends_with("; exit $LASTEXITCODE"));
    let typographic = Path::new("C:\\Users\\D\u{2019}Angelo \u{2018}x\u{201A}\u{201B}\\scoop");
    let (_, args) = bridge::command(&job(Tool::Scoop, "7zip"), typographic, "pwsh");
    assert!(
        args[4].contains(
            "& 'C:\\Users\\D\u{2019}\u{2019}Angelo \u{2018}\u{2018}x\u{201A}\u{201A}\u{201B}\u{201B}\\scoop\\apps"
        ),
        "{}",
        args[4]
    );
}

#[test]
fn invalid_targets_refused() {
    for bad in [
        "-x",
        "a&b",
        "x'y",
        r"..\x",
        "",
        &"a".repeat(129),
        "a b",
        "a/b",
    ] {
        assert!(!valid_target(Tool::Winget, bad), "{bad:?}");
        assert!(!valid_target(Tool::Scoop, bad), "{bad:?}");
    }
    for good in [
        "Foo.Bar-2",
        "7zip",
        "vcredist2015",
        &"a".repeat(128),
        "a+b_c",
    ] {
        assert!(valid_target(Tool::Winget, good), "{good:?}");
        assert!(valid_target(Tool::Scoop, good), "{good:?}");
    }
}

#[test]
fn winget_codes_to_messages() {
    let table: &[(u32, &str)] = &[
        (0, "Atualizado."),
        (
            0x8A15002B,
            "Nenhuma atualização aplicável para este pacote.",
        ),
        (0x8A150014, "O winget não encontrou este pacote instalado."),
        (0x8A150016, "Mais de um pacote corresponde a este id."),
        (
            0x8A150068,
            "Pacote fixado no winget (pin); remova o pin para atualizar.",
        ),
        (
            0x8A15008E,
            "A nova versão usa outro tipo de instalador; atualize pelo winget manualmente.",
        ),
        (0x8A150006, "O instalador falhou."),
        (0x8A150049, "O instalador falhou."),
        (0x8A150052, "O instalador falhou."),
        (
            0x8A150101,
            "O aplicativo está em uso; feche-o e tente de novo.",
        ),
        (
            0x8A150103,
            "O aplicativo está em uso; feche-o e tente de novo.",
        ),
        (
            0x8A150102,
            "Outra instalação está em andamento; tente mais tarde.",
        ),
        (
            0x8A150109,
            "Atualizado; reinicie o computador para concluir.",
        ),
        (0x8A15010A, "É preciso reiniciar o computador."),
        (0x8A15010B, "É preciso reiniciar o computador."),
        (0x8A15010C, "Atualização cancelada."),
        (0x80004004, "Atualização cancelada."),
        (0x800704C7, "Permissão de administrador recusada."),
        (0x8A150008, "Falha no download do instalador."),
        (
            0x8A150011,
            "O hash do instalador não confere com o manifesto.",
        ),
        (0x8A150041, "Contrato não aceito."),
        (0x8A150046, "Contrato não aceito."),
    ];
    for &(code, text) in table {
        assert_eq!(winget_message(code as i32), text, "0x{code:08X}");
    }
    assert_eq!(
        winget_message(-1978335212),
        "O winget não encontrou este pacote instalado."
    );
    assert_eq!(
        winget_message(0x8A15FFFFu32 as i32),
        "O winget terminou com erro 0x8A15FFFF."
    );
    assert_eq!(winget_message(1), "O winget terminou com erro 0x00000001.");
    assert!(bridge::winget_updated(0));
    assert!(bridge::winget_updated(0x8A150109u32 as i32));
    assert!(!bridge::winget_updated(-1978335212));
}

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn scoop_exit_zero_needs_new_version() {
    assert_eq!(
        scoop_message(
            0,
            Some("1"),
            Some("1"),
            &lines(&["7zip: 1 (latest version)"])
        ),
        (false, "Já está na versão mais recente.".to_string())
    );
    assert_eq!(
        scoop_message(
            0,
            Some("1"),
            Some("1"),
            &lines(&["WARN  The latest version of '7zip' (1) is already installed."])
        ),
        (false, "Já está na versão mais recente.".to_string())
    );
    let (updated, text) = scoop_message(0, Some("1"), Some("2"), &[]);
    assert!(updated, "{text}");
    assert_eq!(
        scoop_message(0, None, None, &lines(&["ERROR '7zip' isn't installed."])),
        (false, "O scoop não encontrou este app.".to_string())
    );
    assert_eq!(
        scoop_message(
            0,
            Some("1"),
            Some("1"),
            &lines(&["'7zip' is held to version 1"])
        ),
        (false, "App segurado no scoop (hold).".to_string())
    );
    assert_eq!(
        scoop_message(
            0,
            Some("1"),
            Some("1"),
            &lines(&["Running process detected, skip updating."])
        ),
        (
            false,
            "O app está aberto; feche-o e tente de novo.".to_string()
        )
    );
    assert_eq!(
        scoop_message(0, Some("1"), Some("1"), &[]),
        (false, "O scoop não atualizou o app.".to_string())
    );
    assert_eq!(
        scoop_message(0, Some("1"), None, &[]),
        (false, "O scoop não atualizou o app.".to_string())
    );
    assert_eq!(
        scoop_message(1, Some("1"), Some("2"), &[]),
        (false, "O scoop falhou (código 1).".to_string())
    );
}

#[test]
fn tail_splits_lines() {
    let mut t = Tail::new();
    assert_eq!(t.push(b"um\r\ndo"), vec!["um".to_string()]);
    assert_eq!(t.push(b"is\nx"), vec!["dois".to_string()]);
    assert_eq!(t.push(b"\ra\rb\n"), vec!["b".to_string()]);
    assert_eq!(t.last(), Some("b"));
    assert!(
        t.push(b"\n\r\n  \r\n").is_empty(),
        "empty lines are dropped"
    );
    assert_eq!(t.lines(), ["um", "dois", "b"]);
    assert_eq!(
        t.push(b"x\xff\xfey\n"),
        vec!["x\u{FFFD}\u{FFFD}y".to_string()]
    );
    let long = t.push(&[b'z'; 5000]);
    assert!(long.is_empty());
    let long = t.push(b"\n");
    assert_eq!(long.len(), 1);
    assert!(long[0].len() <= 1024 && long[0].starts_with("zzz"));
    let chunk = vec![b'q'; 64 * 1024];
    for _ in 0..16 {
        assert!(t.push(&chunk).is_empty());
        assert!(t.buffered() <= 64 * 1024 + chunk.len(), "{}", t.buffered());
    }
    assert_eq!(
        t.push(b"\nfinal\nresto"),
        vec!["q".repeat(1024), "final".to_string()]
    );
    assert_eq!(t.finish(), Some("resto".to_string()));
    assert_eq!(t.finish(), None);
    for i in 0..60 {
        t.push(format!("l{i}\n").as_bytes());
    }
    assert_eq!(t.lines().len(), 40);
    assert_eq!(t.lines()[0], "l20");
    assert_eq!(t.last(), Some("l59"));
}

#[cfg(windows)]
#[test]
fn spawn_runs_fake_tool() {
    use wingetrs::bridge::spawn_win::run;
    let cmd = |script: &str| vec!["/d".to_string(), "/c".to_string(), script.to_string()];
    let mut seen = Vec::new();
    let (code, tail) = run(
        "cmd.exe",
        &cmd("echo um& echo erro 1>&2& echo dois& (set /p x=fim<nul) 1>&2& exit /b -1978335212"),
        |l| seen.push(l.to_string()),
    )
    .unwrap();
    assert_eq!(code, -1978335212);
    let out: Vec<&String> = seen.iter().filter(|l| *l == "um" || *l == "dois").collect();
    assert_eq!(out, ["um", "dois"], "stdout lines arrive in order");
    assert!(seen.iter().any(|l| l == "erro"), "{seen:?}");
    assert_eq!(seen.last().map(String::as_str), Some("fim"), "{seen:?}");
    assert_eq!(tail, seen);

    // Stdin is closed: `set /p` returns at once instead of waiting for input.
    let (code, tail) = run("cmd.exe", &cmd("set /p x=& echo depois& exit /b 3"), |_| {}).unwrap();
    assert_eq!(code, 3);
    assert_eq!(tail.last().map(String::as_str), Some("depois"));

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("out.bin");
    std::fs::write(&file, b"ok\xff\xfe\r\nmeio\rfim\n\nsem-fim").unwrap();
    let (code, tail) = run("cmd.exe", &cmd(&format!("type {}", file.display())), |_| {}).unwrap();
    assert_eq!(code, 0);
    assert_eq!(tail, ["ok\u{FFFD}\u{FFFD}", "fim", "sem-fim"]);

    let missing = run("wingetrs-ferramenta-inexistente", &[], |_| {}).unwrap_err();
    assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
}

#[cfg(windows)]
type Bounded = Option<std::thread::Result<std::io::Result<(i32, Vec<String>)>>>;

/// Runs `run_with_grace` on a test thread and waits at most `limit` for it. The thread is
/// always joined: if the bridge hangs, it ends on its own when the grandchild dies.
#[cfg(windows)]
fn run_bounded(
    script: &str,
    grace: std::time::Duration,
    limit: std::time::Duration,
    mut on_line: impl FnMut(&str) + Send + 'static,
) -> (Bounded, std::time::Duration) {
    use wingetrs::bridge::spawn_win::run_with_grace;
    let args = vec!["/d".to_string(), "/c".to_string(), script.to_string()];
    let (tx, rx) = std::sync::mpsc::channel();
    let start = std::time::Instant::now();
    let h = std::thread::spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_with_grace("cmd.exe", &args, grace, |l| on_line(l))
        }));
        let _ = tx.send(r);
    });
    let got = rx.recv_timeout(limit).ok();
    let elapsed = start.elapsed();
    if let Err(p) = h.join() {
        std::panic::resume_unwind(p);
    }
    (got, elapsed)
}

#[cfg(windows)]
#[test]
fn grandchild_holding_pipe_does_not_hang() {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let s = Arc::clone(&seen);
    let (got, elapsed) = run_bounded(
        "start /b ping -n 9 127.0.0.1 & echo pronto & exit /b 3",
        Duration::from_millis(500),
        Duration::from_secs(5),
        move |l| s.lock().unwrap().push(l.to_string()),
    );
    let (code, tail) = got
        .unwrap_or_else(|| panic!("bridge still running after {elapsed:?}"))
        .unwrap()
        .unwrap();
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    assert_eq!(code, 3);
    let seen = seen.lock().unwrap();
    assert!(seen.iter().any(|l| l == "pronto"), "{seen:?}");
    assert!(tail.iter().any(|l| l == "pronto"), "{tail:?}");
}

#[cfg(windows)]
#[test]
fn grace_lets_late_output_arrive() {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let s = Arc::clone(&seen);
    let (got, elapsed) = run_bounded(
        // Carets, not quotes: Rust escapes an inner `"` as `\"`, which cmd does not read.
        "start /b cmd /d /c ping -n 2 127.0.0.1 ^>nul ^& echo tarde& exit /b 0",
        Duration::from_secs(3),
        Duration::from_secs(10),
        move |l| s.lock().unwrap().push(l.to_string()),
    );
    let (code, _) = got
        .unwrap_or_else(|| panic!("bridge still running after {elapsed:?}"))
        .unwrap()
        .unwrap();
    assert_eq!(code, 0);
    let seen = seen.lock().unwrap();
    assert!(seen.iter().any(|l| l == "tarde"), "{seen:?}");
}

#[cfg(windows)]
#[test]
fn panic_in_on_line_with_grandchild_does_not_hang() {
    use std::time::Duration;
    let (got, elapsed) = run_bounded(
        "start /b ping -n 9 127.0.0.1 & echo pronto & exit /b 3",
        Duration::from_millis(500),
        Duration::from_secs(5),
        |_| panic!("on_line falhou"),
    );
    let got = got.unwrap_or_else(|| panic!("bridge still running after {elapsed:?}"));
    assert!(got.is_err(), "the on_line panic propagates");
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
}

#[cfg(windows)]
#[test]
fn chatty_grandchild_does_not_delay_the_cut() {
    use std::time::Duration;
    // The grandchild writes a line every fraction of a millisecond for about 11 s, so the
    // bridge never sees a quiet 100 ms; the tool's exit must still be noticed.
    let (got, elapsed) = run_bounded(
        "start /b cmd /d /c for /l %i in (1,1,150000) do @echo x& exit /b 4",
        Duration::from_millis(500),
        Duration::from_secs(5),
        |_| {},
    );
    let (code, _) = got
        .unwrap_or_else(|| panic!("bridge still running after {elapsed:?}"))
        .unwrap()
        .unwrap();
    assert_eq!(code, 4);
}

#[test]
fn reboot_marker_after_success() {
    let pt = vec![
        "Instalado com êxito".to_string(),
        "Reiniciar seu PC para terminar a instalação.".to_string(),
    ];
    let m = &REBOOT_MARKERS;
    assert!(reboot_requested(0, &pt, m));
    assert!(!reboot_requested(0, &pt[..1], m));
    assert!(!reboot_requested(1, &pt, m));
    let colored = vec!["\x1b[0mReiniciar seu PC para terminar a instalação.\x1b[0m".to_string()];
    assert!(reboot_requested(0, &colored, m));
    let ascii = vec!["Reiniciar seu PC para terminar a instalacao.".to_string()];
    assert!(reboot_requested(0, &ascii, m));
    let it = vec!["Riavvia il PC per completare l'installazione..".to_string()];
    assert!(reboot_requested(0, &it, m));
}

#[test]
fn reboot_code_0x8a150109() {
    assert!(reboot_requested(
        0x8A15_0109u32 as i32,
        &[],
        &REBOOT_MARKERS
    ));
    assert!(!reboot_requested(
        0x8A15_002Bu32 as i32,
        &[],
        &REBOOT_MARKERS
    ));
}

#[test]
fn every_reboot_marker_matches() {
    // Independent copy of the .resw texts: a typo in the const must not pass.
    let expected = [
        "Restart your PC to finish installation.",
        "Starten Sie den PC neu, um die Installation abzuschließen.",
        "Reinicie el equipo para finalizar la instalación.",
        "Redémarrez votre PC pour terminer l’installation.",
        "Riavvia il PC per completare l'installazione..",
        "PC を再起動してインストールを完了します。",
        "PC를 다시 시작하여 설치를 완료합니다.",
        "Reiniciar seu PC para terminar a instalação.",
        "Перезапустите компьютер, чтобы завершить установку.",
        "重启电脑以完成安装。",
        "重新啟動您的電腦以完成安裝。",
    ];
    assert_eq!(REBOOT_MARKERS, expected);
    for m in expected {
        assert!(
            reboot_requested(0, &[m.to_string()], &REBOOT_MARKERS),
            "{m}"
        );
        let wrapped = format!("\u{1b}[0m{m}\u{1b}[0m");
        assert!(reboot_requested(0, &[wrapped], &REBOOT_MARKERS), "{m}");
    }
}
