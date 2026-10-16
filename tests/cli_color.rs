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
                include_bytes!("fixtures/winget/source2-mini.msix").to_vec(),
            ),
            (
                "/main.tgz",
                include_bytes!("fixtures/scoop/Main-mini.tar.gz").to_vec(),
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
