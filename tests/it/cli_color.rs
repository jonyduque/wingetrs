#![allow(clippy::unwrap_used, clippy::expect_used)]
use clap::CommandFactory;
use std::process::{Command, Output};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Env {
    _server: MockServer,
    dir: tempfile::TempDir,
    uri: String,
}

impl Env {
    async fn new() -> Env {
        let s = MockServer::start().await;
        for (p, body) in [
            (
                "/source2.msix",
                include_bytes!("../fixtures/winget/source2-mini.msix").to_vec(),
            ),
            (
                "/main.tgz",
                include_bytes!("../fixtures/scoop/Main-mini.tar.gz").to_vec(),
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
                .mount(&s)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/extras.tgz"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&s)
            .await;
        let env = Env {
            uri: s.uri(),
            _server: s,
            dir: tempfile::tempdir().unwrap(),
        };
        let out = tokio::task::block_in_place(|| env.run(&["update", "--force"], &[]));
        assert!(out.status.success());
        env
    }

    fn run(&self, args: &[&str], vars: &[(&str, &str)]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_wingetrs"));
        c.args(args)
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env("LOCALAPPDATA", self.dir.path())
            .env(
                "WINGETRS_WINGET_INDEX_URL",
                format!("{}/source2.msix", self.uri),
            )
            .env("WINGETRS_SCOOP_BASE_URL", &self.uri);
        for (k, v) in vars {
            c.env(k, v);
        }
        c.output().unwrap()
    }

    fn search(&self, extra: &[&str], vars: &[(&str, &str)]) -> String {
        let mut args = vec!["search", "", "-n", "100"];
        args.extend_from_slice(extra);
        let out = tokio::task::block_in_place(|| self.run(&args, vars));
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn piped_output_has_no_escape() {
    let e = Env::new().await;
    let plain = e.search(&[], &[]);
    assert!(plain.contains("Nome"), "{plain}");
    assert!(!plain.contains('\x1b'), "{plain:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn color_always_paints() {
    let e = Env::new().await;
    let plain = e.search(&[], &[]);
    let colored = e.search(&["--color", "always"], &[]);
    assert!(colored.contains('\x1b'), "{colored:?}");
    assert_eq!(anstream::adapter::strip_str(&colored).to_string(), plain);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_color_env_wins_over_always_absent() {
    let e = Env::new().await;
    let out = e.search(&[], &[("NO_COLOR", "1")]);
    assert!(!out.contains('\x1b'), "{out:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn json_never_colored() {
    let e = Env::new().await;
    let out = e.search(&["--json", "--color", "always"], &[]);
    assert!(!out.contains('\x1b'), "{out:?}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v.is_array());
}

#[test]
fn table_alignment_ignores_ansi() {
    use wingetrs::cli::{output::search_table, style::with_enabled};
    use wingetrs::core::PackageEntry;
    let rows: Vec<PackageEntry> = ["a", "longer name", "ç"]
        .iter()
        .enumerate()
        .map(|(i, n)| PackageEntry {
            name: (*n).into(),
            id: format!("Pub.Id{i}"),
            version: format!("1.{i}"),
            source: wingetrs::core::SourceKind::Winget,
            source_name: "winget".into(),
            moniker: None,
            publisher: None,
            description: None,
            tags: vec![],
            commands: vec![],
        })
        .collect();
    let plain = with_enabled(false, || search_table(&rows));
    let colored = with_enabled(true, || search_table(&rows));
    assert!(colored.contains('\x1b'));
    let stripped = anstream::adapter::strip_str(&colored).to_string();
    assert_eq!(stripped.lines().count(), plain.lines().count());
    for (a, b) in stripped.lines().zip(plain.lines()) {
        assert_eq!(a, b);
    }
}

#[test]
fn help_is_styled() {
    let help = wingetrs::cli::Cli::command().render_help();
    assert!(help.ansi().to_string().contains('\x1b'));
    let text = help.to_string();
    assert!(text.contains("wingetrs upgrade --all"), "{text}");
    assert!(text.contains("wingetrs update"), "{text}");
}

#[test]
fn help_color_flag_overrides_pipe() {
    let help = |flag: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_wingetrs"))
            .arg("--help")
            .args(flag)
            .env_remove("CLICOLOR_FORCE")
            .env_remove("NO_COLOR")
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    assert!(help(&["--color", "always"]).contains(''));
    assert!(help(&["--color=always"]).contains(''));
    assert!(!help(&["--color", "never"]).contains(''));
}

#[test]
fn list_text_alignment_ignores_ansi() {
    use wingetrs::cli::{output::list_text, style::with_enabled};
    use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
    let row = |n: &str, id: &str, v: &str, latest: Option<&str>| ListRow {
        name: n.into(),
        origin: "arp".into(),
        installed_version: Some(v.into()),
        correlation: Correlation::Matched {
            source: "winget".into(),
            package_id: id.into(),
            rule: Rule::ProductCode,
        },
        state: latest.map_or(UpdateState::UpToDate, |l| UpdateState::Available {
            latest: l.into(),
        }),
        ..Default::default()
    };
    let rows = vec![
        row("Zip", "7zip.7zip", "1.0", Some("2.0")),
        row("Ação longa", "A.B", "10.20.30", None),
    ];
    let plain = with_enabled(false, || list_text(&rows, false));
    let colored = with_enabled(true, || list_text(&rows, false));
    assert!(colored.contains(''));
    let stripped = anstream::adapter::strip_str(&colored).to_string();
    assert_eq!(stripped, plain);
}

#[test]
fn help_plain_when_piped() {
    let out = Command::new(env!("CARGO_BIN_EXE_wingetrs"))
        .arg("--help")
        .env_remove("CLICOLOR_FORCE")
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("Exemplos"), "{text}");
    assert!(!text.contains('\x1b'), "{text:?}");
}
/// Every byte the binary wrote, stdout then stderr.
fn both(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn error_has_action_line() {
    use wingetrs::cli::output::{Advice, error_report};
    use wingetrs::cli::style::with_enabled;
    let e: anyhow::Error =
        Advice::new("a confirmação exige um terminal interativo", "use --yes").into();
    // The one-line form stays for logs and the job log.
    assert_eq!(
        e.to_string(),
        "a confirmação exige um terminal interativo; use --yes"
    );
    assert_eq!(
        with_enabled(false, || error_report(&e, false)),
        "✗ a confirmação exige um terminal interativo\n→ use --yes\n"
    );
    // No action: one line.
    let e = anyhow::anyhow!("falhou");
    assert_eq!(
        with_enabled(false, || error_report(&e, false)),
        "✗ falhou\n"
    );
    // The causes only with -v; without it, how to see them.
    let e = anyhow::anyhow!("raiz").context("topo");
    assert_eq!(
        with_enabled(false, || error_report(&e, false)),
        "✗ topo\n→ rode de novo com -v para ver a causa\n"
    );
    assert_eq!(
        with_enabled(false, || error_report(&e, true)),
        "✗ topo\n  causa: raiz\n"
    );
    // An `Advice` under a context is a cause, never the headline.
    let e = anyhow::Error::from(Advice::new("a", "b")).context("c");
    assert!(with_enabled(false, || error_report(&e, false)).starts_with("✗ c\n"));
    // The binary: what failed, then what to do, on stderr; nothing on stdout.
    let env = Env::new().await;
    let out = tokio::task::block_in_place(|| env.run(&["show", "nao.existe"], &[]));
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "✗ pacote não encontrado: nao.existe\n→ procure o id com `wingetrs search <termo>`\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn no_ansi_when_not_tty() {
    let env = Env::new().await;
    let cmds: [&[&str]; 4] = [
        &["search", "", "-n", "100"],
        &["show", "7zip", "--source", "scoop"],
        &["show", "nao.existe"],
        &["bucket", "list"],
    ];
    // Piped; NO_COLOR over a forced color; `--color never` over CLICOLOR_FORCE.
    type Mode<'a> = (&'a [&'a str], &'a [(&'a str, &'a str)]);
    let modes: [Mode; 3] = [
        (&[], &[]),
        (&[], &[("NO_COLOR", "1"), ("CLICOLOR_FORCE", "1")]),
        (&["--color", "never"], &[("CLICOLOR_FORCE", "1")]),
    ];
    for cmd in cmds {
        for (flags, vars) in modes {
            let args: Vec<&str> = cmd.iter().chain(flags).copied().collect();
            let out = tokio::task::block_in_place(|| env.run(&args, vars));
            let text = both(&out);
            assert!(!text.is_empty(), "{args:?}");
            assert!(!text.contains('\x1b'), "{args:?} {vars:?}: {text:?}");
        }
    }
    // The same command with color forced does paint (the check above can see ANSI).
    let out = tokio::task::block_in_place(|| {
        env.run(&["show", "nao.existe"], &[("CLICOLOR_FORCE", "1")])
    });
    assert!(both(&out).contains('\x1b'));
}

#[test]
fn wide_chars_align_table() {
    use wingetrs::cli::{output::search_table, style::with_enabled};
    use wingetrs::core::PackageEntry;
    // 10 columns in 5 chars; 4 columns in 6 chars (combining marks); 2 columns.
    let names = ["漢字アプリ", "Ac\u{327}a\u{303}o", "ab"];
    let rows: Vec<PackageEntry> = names
        .iter()
        .enumerate()
        .map(|(i, n)| PackageEntry {
            name: (*n).into(),
            id: format!("Pub.Id{i}"),
            version: "1.0".into(),
            source: wingetrs::core::SourceKind::Winget,
            source_name: "winget".into(),
            moniker: None,
            publisher: None,
            description: None,
            tags: vec![],
            commands: vec![],
        })
        .collect();
    let width = |s: &str| ratatui::text::Span::raw(s).width();
    for on in [false, true] {
        let table = with_enabled(on, || search_table(&rows));
        let plain = anstream::adapter::strip_str(&table).to_string();
        let lines: Vec<&str> = plain.lines().collect();
        assert_eq!(lines.len(), 4, "{plain}");
        // The Id column starts at column 12 (widest name 10 + 2) on every line.
        for (line, id) in lines.iter().zip(["Id", "Pub.Id0", "Pub.Id1", "Pub.Id2"]) {
            let at = line.find(id).unwrap();
            assert_eq!(width(&line[..at]), 12, "{line:?}");
        }
    }
}

/// An ARP removal that runs elevated and shares its data folders (Kindle).
pub(crate) fn kindle_uninstall() -> wingetrs::winget_install::uninstall::UninstallPlan {
    use wingetrs::winget_install::launch::Elevation;
    use wingetrs::winget_install::uninstall::{SharedData, UninstallPlan, UninstallTarget};
    use wingetrs::winget_install::uninstall_cmd::{Rule, UninstallCommand};
    UninstallPlan {
        target: UninstallTarget::Arp {
            hive: wingetrs::installed::Hive::MachineX64,
            key: "Kindle".into(),
        },
        name: "Kindle".into(),
        version: Some("2.0".into()),
        command: Some(UninstallCommand {
            rule: Rule::Nsis,
            program: r"C:\K\uninst.exe".into(),
            args: "/S".into(),
            interactive: false,
        }),
        elevation: Elevation::Helper,
        shared: Some(SharedData {
            folders: vec![r"C:\Users\u\Documents\My Kindle Content".into()],
            size: None,
            registry_only_allowed: true,
            registry_only_refused: None,
        }),
    }
}

/// `\x1b` made visible, so a colored snapshot reads as text.
fn visible(s: &str) -> String {
    s.replace('\x1b', "␛")
}

/// The formatted output of each command, in the ways it reaches a terminal: colored (TTY) and
/// plain on a TTY (`NO_COLOR`); `snapshots_not_tty` has the binary, piped.
#[test]
fn snapshots_tty_color_and_plain() {
    use wingetrs::cli::output;
    use wingetrs::cli::style::with_enabled;
    use wingetrs::core::{PackageDetails, PackageEntry, SourceKind};
    use wingetrs::installed::correlate::{Correlation, ListRow, Rule, UpdateState};
    use wingetrs::update::{Outcome, SourceReport};
    let reports = [
        Outcome::Fresh,
        Outcome::Unchanged,
        Outcome::Updated {
            packages: 28,
            skipped: 1,
        },
        Outcome::Failed {
            message: "HTTP 500".into(),
            offline: false,
        },
    ];
    let entry = PackageEntry {
        name: "7-Zip".into(),
        id: "7zip.7zip".into(),
        version: "24.08".into(),
        source: SourceKind::Winget,
        source_name: "winget".into(),
        moniker: None,
        publisher: None,
        description: None,
        tags: vec![],
        commands: vec![],
    };
    let details = PackageDetails {
        source: SourceKind::Scoop,
        id: "7zip".into(),
        name: "7zip".into(),
        version: "24.08".into(),
        publisher: None,
        description: Some("A multi-format file archiver".into()),
        homepage: Some("https://www.7-zip.org/".into()),
        license: Some("LGPL-2.1-or-later".into()),
        tags: vec![],
        installers: vec![],
        available_versions: vec![],
        icon: None,
    };
    let row = ListRow {
        name: "Ação".into(),
        origin: "arp".into(),
        installed_version: Some("1.0".into()),
        correlation: Correlation::Matched {
            source: "winget".into(),
            package_id: "A.B".into(),
            rule: Rule::ProductCode,
        },
        state: UpdateState::Available {
            latest: "2.0".into(),
        },
        ..Default::default()
    };
    let upgrades = [("solo".to_string(), "1.0".to_string(), "2.0".to_string())];
    use crate::cli_plan_summary::{plan, shim, source};
    use wingetrs::install::plan::Step;
    let mut dry_plan = plan(
        [
            vec![
                Step::Download {
                    index: 0,
                    source: source("app.7z"),
                },
                Step::Extract { index: 0 },
                Step::Promote,
                Step::LinkCurrent,
                shim("app"),
                Step::EnsureShimsOnPath,
                Step::RecordState,
            ],
            // Ten steps or more: the numbers stay right-aligned.
            vec![Step::Promote; 3],
        ]
        .concat(),
    );
    dry_plan
        .notes
        .push("o manifesto pede uma reinicialização".into());
    let winget_plan =
        crate::wi_tui::native("msi", wingetrs::winget_install::select::Scope::Machine);
    let glyphs = wingetrs::tui::theme::Glyphs::for_set(wingetrs::tui::theme::GlyphSet::Unicode);
    let render = || {
        let mut s = String::new();
        for o in &reports {
            let r = SourceReport {
                source: "winget".into(),
                outcome: o.clone(),
            };
            s.push_str(&output::report_line(&r));
            s.push('\n');
        }
        s.push_str("-- search\n");
        s.push_str(&output::search_table(std::slice::from_ref(&entry)));
        s.push_str("-- show\n");
        s.push_str(&output::details_text(&details));
        s.push_str("-- list\n");
        s.push_str(&output::list_text(std::slice::from_ref(&row), false));
        s.push_str("-- upgrade\n");
        s.push_str(&output::upgrades_listing(&upgrades, &["fixo".to_string()]));
        s.push_str("-- install / uninstall / cleanup\n");
        for line in [
            output::success("Instalado:", &output::app_version("solo", "1.0")),
            output::warning("o desinstalador do app não rodou"),
            output::cancelled(),
            output::DRY_RUN_DONE.to_string(),
            output::failed_item("App A: negado"),
        ] {
            s.push_str(&line);
            s.push('\n');
        }
        s.push_str("-- install --dry-run (scoop)\n");
        s.push_str(&output::install_plan_text(&dry_plan, glyphs));
        s.push_str("-- install (winget, confirmação)\n");
        s.push_str(&output::winget_plan_text(&winget_plan, false));
        s.push_str("-- install --dry-run (winget)\n");
        s.push_str(&output::winget_plan_text(&winget_plan, true));
        s.push_str(&output::through_winget(false, "tipo zip"));
        s.push('\n');
        s.push_str(&output::license(
            "Licença aceita: EULA — https://x.invalid/eula",
        ));
        s.push('\n');
        s.push_str(&output::reboot(false));
        s.push('\n');
        s.push_str("-- uninstall (winget, confirmação)\n");
        s.push_str(&output::winget_uninstall_text(&kindle_uninstall(), false));
        s.push_str("-- uninstall --dry-run (winget)\n");
        s.push_str(&output::winget_uninstall_text(&kindle_uninstall(), true));
        for line in [
            output::folders_copied(std::path::Path::new(r"C:\B\kindle")),
            output::record_removed("App A", std::path::Path::new(r"C:\B\a.reg")),
        ] {
            s.push_str(&line);
            s.push('\n');
        }
        s.push_str("-- cleanup --orphans\n");
        for line in [
            output::orphan_line("App A", "HKCU", r"C:\Gone\A\uninst.exe"),
            output::orphans_may_have_moved(),
            output::orphans_backup(std::path::Path::new(r"C:\B\orphans-1")),
            output::orphans_not_removed(),
            output::NO_ORPHANS.to_string(),
            output::NONE_REMOVED.to_string(),
            output::orphan_removed("App A", std::path::Path::new(r"C:\B\a.reg")),
            output::orphan_failed("App B", "acesso negado"),
        ] {
            s.push_str(&line);
            s.push('\n');
        }
        s.push_str("-- erro\n");
        let failure: anyhow::Error = output::not_found("x", &["xz".into()]).into();
        s.push_str(&output::error_report(&failure, false));
        s
    };
    let color = with_enabled(true, render);
    let plain = with_enabled(false, render);
    // The winget plans keep their text: only styled.
    assert!(color.contains('\x1b'));
    assert!(!plain.contains('\x1b'));
    // Color never changes the text.
    assert_eq!(anstream::adapter::strip_str(&color).to_string(), plain);
    insta::assert_snapshot!("tty_color", visible(&color));
    insta::assert_snapshot!("tty_plain", plain);
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshots_not_tty() {
    let env = Env::new().await;
    let mut s = String::new();
    for args in [
        &["search", "7zip"][..],
        &["show", "7zip", "--source", "scoop"],
        &["show", "nao.existe"],
        &["bucket", "list"],
    ] {
        let out = tokio::task::block_in_place(|| env.run(args, &[]));
        s.push_str(&format!("$ wingetrs {}\n", args.join(" ")));
        s.push_str(&both(&out));
    }
    insta::assert_snapshot!("not_tty", s);
}

/// `install --dry-run` of a Scoop app from the binary, piped (root and port redacted).
#[tokio::test(flavor = "multi_thread")]
async fn snapshots_not_tty_dry_run() {
    let server = MockServer::start().await;
    let lab = crate::cli_install::Lab::new(&server, "t199-dry").await;
    let out = tokio::task::block_in_place(|| lab.run(&["install", "solo", "--dry-run"]));
    assert!(out.status.success(), "{}", both(&out));
    let text = both(&out)
        .replace(&lab.root.display().to_string(), "<RAIZ>")
        .replace(&server.uri(), "<SERVIDOR>");
    assert!(!text.contains('\x1b'), "{text:?}");
    insta::assert_snapshot!("not_tty_dry_run", text);
}

/// On a terminal, long names are cut (display width, ending in `…`) so no table line wraps;
/// off a terminal the cells stay whole.
#[test]
fn long_names_fit_the_terminal() {
    use wingetrs::cli::output::{columns, search_table};
    use wingetrs::cli::style::{with_columns, with_enabled};
    use wingetrs::core::PackageEntry;
    let names = [
        "Advanced Archive Password Recovery Professional Edition",
        "漢字アプリケーション漢字アプリケーション漢字",
        "ab",
    ];
    let rows: Vec<PackageEntry> = names
        .iter()
        .enumerate()
        .map(|(i, n)| PackageEntry {
            name: (*n).into(),
            id: format!("Pub.Id{i}"),
            version: "1.0".into(),
            source: wingetrs::core::SourceKind::Winget,
            source_name: "winget".into(),
            moniker: None,
            publisher: None,
            description: None,
            tags: vec![],
            commands: vec![],
        })
        .collect();
    for on in [false, true] {
        let narrow = with_enabled(on, || with_columns(40, || search_table(&rows)));
        let plain = anstream::adapter::strip_str(&narrow).to_string();
        for line in plain.lines() {
            assert!(columns(line) <= 39, "{} {line:?}", columns(line));
        }
        assert!(plain.contains("Advanced"), "{plain}");
        assert_eq!(plain.matches('…').count(), 2, "{plain}");
        // The Id column still starts at the same place on every line.
        let starts: Vec<usize> = plain
            .lines()
            .map(|l| columns(&l[..l.find(" Id").or_else(|| l.find(" Pub.")).unwrap()]))
            .collect();
        assert!(starts.windows(2).all(|w| w[0] == w[1]), "{plain}");
    }
    let full = with_enabled(false, || with_columns(0, || search_table(&rows)));
    for n in names {
        assert!(full.contains(n), "{full}");
    }
    assert!(!full.contains('…'));
}
