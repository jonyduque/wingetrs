#![allow(clippy::unwrap_used, clippy::expect_used)]
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::cell::RefCell;
use wingetrs::bridge::winget_out::{Kind, TABLE, WingetLine, classify};
use wingetrs::bridge::{BridgeJob, Tool};
use wingetrs::cli::output;
use wingetrs::install::progress::ProgressEvent;
use wingetrs::tui::app::{Action, App, Cmd, Msg, Tab, update};
use wingetrs::tui::theme::GlyphSet;

const PROBE: [&str; 6] = [
    "Encontrado jq [jqlang.jq] Versão 1.8.2",
    "Este aplicativo é licenciado para você pelo proprietário.",
    "A Microsoft não é responsável por, nem concede licenças a pacotes de terceiros.",
    "Baixando https://github.com/jqlang/jq/releases/download/jq-1.8.2/jq-windows-amd64.exe",
    "Hash do instalador verificado com êxito",
    "Instalador baixado: C:\\temp\\dl\\jq_1.8.2_X64_portable_en-US.exe",
];

/// What the CLI prints (stdout) for `lines`.
fn cli_out(lines: &[&str]) -> Vec<String> {
    let out = RefCell::new(Vec::new());
    output::bridge_progress(
        &|_: &ProgressEvent| {},
        &|l: &str| out.borrow_mut().push(l.to_string()),
        "winget",
        |sink| {
            for l in lines {
                sink(l);
            }
        },
    );
    out.into_inner()
}

#[test]
fn probe_lines_classified() {
    let got: Vec<WingetLine> = PROBE.iter().map(|l| classify(l)).collect();
    assert_eq!(
        got[0],
        WingetLine::Found {
            name: "jq".into(),
            id: "jqlang.jq".into(),
            version: "1.8.2".into()
        }
    );
    assert_eq!(got[1], WingetLine::License);
    assert_eq!(got[2], WingetLine::License);
    assert_eq!(
        got[3],
        WingetLine::Downloading {
            url: "https://github.com/jqlang/jq/releases/download/jq-1.8.2/jq-windows-amd64.exe"
                .into()
        }
    );
    assert_eq!(got[4], WingetLine::HashOk);
    assert!(matches!(got[5], WingetLine::Other(_)), "{:?}", got[5]);
}

#[test]
fn english_lines_classified() {
    assert_eq!(
        classify("Found Git [Git.Git] Version 2.50.0"),
        WingetLine::Found {
            name: "Git".into(),
            id: "Git.Git".into(),
            version: "2.50.0".into()
        }
    );
    assert_eq!(
        classify("This application is licensed to you by its owner."),
        WingetLine::License
    );
    assert_eq!(
        classify(
            "Microsoft is not responsible for, nor does it grant any licenses to, third-party packages."
        ),
        WingetLine::License
    );
    assert_eq!(
        classify("Downloading https://x.test/a.exe"),
        WingetLine::Downloading {
            url: "https://x.test/a.exe".into()
        }
    );
    assert_eq!(
        classify("Successfully verified installer hash"),
        WingetLine::HashOk
    );
    assert_eq!(
        classify("Starting package install..."),
        WingetLine::Starting { removing: false }
    );
    assert_eq!(
        classify("Starting package uninstall..."),
        WingetLine::Starting { removing: true }
    );
    assert_eq!(classify("Successfully installed"), WingetLine::Done);
    assert_eq!(classify("Successfully uninstalled"), WingetLine::Done);
    assert_eq!(
        classify("No available upgrade found."),
        WingetLine::NoUpgrade
    );
    assert_eq!(
        classify("No newer package versions are available from the configured sources."),
        WingetLine::NoUpgrade
    );
}

#[test]
fn portuguese_table_entries_classified() {
    assert_eq!(
        classify("Iniciando a instalação do pacote..."),
        WingetLine::Starting { removing: false }
    );
    assert_eq!(
        classify("Iniciando a desinstalação do pacote..."),
        WingetLine::Starting { removing: true }
    );
    assert_eq!(classify("Instalado com êxito"), WingetLine::Done);
    assert_eq!(classify("Desinstalado com êxito"), WingetLine::Done);
    assert_eq!(classify("Baixando"), WingetLine::Other("Baixando".into()));
    // every table entry classifies to its own kind
    for (text, kind) in TABLE {
        let got = classify(text);
        let want = match kind {
            Kind::License => WingetLine::License,
            Kind::HashOk => WingetLine::HashOk,
            Kind::StartInstall => WingetLine::Starting { removing: false },
            Kind::StartRemove => WingetLine::Starting { removing: true },
            Kind::Done => WingetLine::Done,
            Kind::NoUpgrade => WingetLine::NoUpgrade,
        };
        assert_eq!(got, want, "{text}");
    }
}

#[test]
fn ansi_and_case_ignored() {
    assert_eq!(
        classify("\u{1b}[32mSUCCESSFULLY INSTALLED\u{1b}[0m"),
        WingetLine::Done
    );
    assert_eq!(
        classify("hash do instalador verificado com EXITO"),
        WingetLine::HashOk
    );
    assert_eq!(classify("INSTALADO COM ÊXITO"), WingetLine::Done);
}

#[test]
fn cli_hides_download_url() {
    let out = cli_out(&[PROBE[3]]);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(out[0].contains("Baixando o instalador"), "{out:?}");
    assert!(
        !out[0].contains("http") && !out[0].contains("jq-windows"),
        "{out:?}"
    );
}

#[test]
fn cli_license_lines_merged() {
    let out = cli_out(&[PROBE[1], PROBE[2]]);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(
        out[0].contains("licença de terceiros: aceita pelo --accept-package-agreements"),
        "{out:?}"
    );
}

#[test]
fn other_lines_dimmed_with_prefix() {
    let out = cli_out(&["Algo que o winget disse"]);
    assert_eq!(out, vec!["  winget: Algo que o winget disse".to_string()]);
    // dimmed: the style is applied when color is on
    let styled = wingetrs::cli::style::with_enabled(true, || {
        let mut seen = false;
        output::format_winget_line(&mut seen, "Algo que o winget disse").unwrap()
    });
    assert!(
        styled.contains('\u{1b}') && styled.contains("winget: Algo"),
        "{styled:?}"
    );
}

#[test]
fn tui_label_uses_classify() {
    let mut a = App::new(Vec::new(), GlyphSet::Unicode);
    a.settings.behavior.confirm = wingetrs::config::ConfirmMode::Never;
    a.installed = vec![wingetrs::installed::correlate::ListRow {
        name: "Git.Git".into(),
        origin: "arp".into(),
        installed_version: Some("1".into()),
        correlation: wingetrs::installed::correlate::Correlation::Matched {
            source: "winget".into(),
            package_id: "Git.Git".into(),
            rule: wingetrs::installed::correlate::Rule::ScoopManifest,
        },
        latest: Some("2".into()),
        state: wingetrs::installed::correlate::UpdateState::Available { latest: "2".into() },
        ..Default::default()
    }];
    a.tab = Tab::Updates;
    let job = BridgeJob {
        op: wingetrs::bridge::BridgeOp::Upgrade,
        tool: Tool::Winget,
        target: "Git.Git".into(),
        name: "Git.Git".into(),
        installed: Some("1".into()),
        available: Some("2".into()),
        scope: None,
    };
    let cmds = update(
        &mut a,
        Msg::Key(KeyEvent::new(KeyCode::Char('U'), KeyModifiers::NONE)),
    );
    assert_eq!(cmds, vec![Cmd::Execute(Action::Bridge(job))]);
    update(&mut a, Msg::BridgeLine(PROBE[3].into()));
    let label = a.running.as_ref().unwrap().label.clone();
    assert!(label.ends_with("· baixando o instalador"), "{label}");
    update(&mut a, Msg::BridgeLine("linha desconhecida".into()));
    let label = a.running.as_ref().unwrap().label.clone();
    assert!(label.ends_with("· linha desconhecida"), "{label}");
}

#[test]
fn wingetrs_own_lines_have_no_winget_prefix() {
    for l in output::WINGETRS_LINES {
        let out = cli_out(&[l]);
        assert_eq!(out, vec![format!("  {l}")], "{l}");
    }
    for l in [
        wingetrs::bridge::SCOPE_FALLBACK_NOTE,
        wingetrs::tui::jobs::WAITING_INSTALLER,
        wingetrs::tui::jobs::STARTING_WINGET,
        wingetrs::bridge::winget_out::UPDATING_SOURCE,
    ] {
        assert!(output::WINGETRS_LINES.contains(&l), "{l}");
    }
    let out = cli_out(&["linha desconhecida do processo"]);
    assert_eq!(
        out,
        vec!["  winget: linha desconhecida do processo".to_string()]
    );
}

#[test]
fn done_line_has_no_csi() {
    let out = cli_out(&["\u{1b}[32mSuccessfully installed\u{1b}[0m"]);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].contains("Successfully installed") && !out[0].contains('\u{1b}'),
        "{out:?}"
    );
}
