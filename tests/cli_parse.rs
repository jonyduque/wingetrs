#![allow(clippy::unwrap_used, clippy::expect_used)]
use clap::{CommandFactory, Parser};
use wingetrs::cli::{Cli, Command, GlyphArg};
use wingetrs::core::SourceKind;

#[test]
fn clap_definition_is_valid() {
    Cli::command().debug_assert();
}

#[test]
fn no_subcommand_means_tui() {
    let cli = Cli::try_parse_from(["wingetrs"]).unwrap();
    assert_eq!(cli.command, None);
}

#[test]
fn search_with_source_and_limit() {
    let cli = Cli::try_parse_from([
        "wingetrs",
        "search",
        "power toys",
        "--source",
        "SCOOP",
        "-n",
        "5",
    ])
    .unwrap();
    assert_eq!(
        cli.command,
        Some(Command::Search {
            query: "power toys".into(),
            source: Some(SourceKind::Scoop),
            json: false,
            limit: 5
        })
    );
}

#[test]
fn unknown_source_is_rejected_with_portuguese_message() {
    let err = Cli::try_parse_from(["wingetrs", "list", "--source", "choco"]).unwrap_err();
    assert!(
        err.to_string().contains("fonte desconhecida: choco"),
        "{err}"
    );
}

#[test]
fn glyphs_is_root_only() {
    let cli = Cli::try_parse_from(["wingetrs", "--glyphs", "ascii", "list"]).unwrap();
    assert_eq!(cli.glyphs, Some(GlyphArg::Ascii));
}

#[test]
fn update_background_flag_is_hidden_but_accepted() {
    let cli = Cli::try_parse_from(["wingetrs", "update", "--background"]).unwrap();
    assert_eq!(
        cli.command,
        Some(Command::Update {
            force: false,
            background: true
        })
    );
    // The root help never lists subcommand flags; inspect the subcommand's own help.
    let mut root = Cli::command();
    let help = root
        .find_subcommand_mut("update")
        .unwrap()
        .render_long_help()
        .to_string();
    assert!(
        help.contains("--force"),
        "sanity: update help rendered:\n{help}"
    );
    assert!(!help.contains("--background"));
}

#[test]
fn source_kind_display_and_parse_round_trip() {
    for k in [SourceKind::Winget, SourceKind::Scoop] {
        assert_eq!(k.to_string().parse::<SourceKind>().unwrap(), k);
    }
}

#[test]
fn stderr_logging_is_off_without_subcommand() {
    let cli = Cli::try_parse_from(["wingetrs", "-v"]).unwrap();
    assert!(!wingetrs::cli::stderr_logging(&cli));
}

#[test]
fn stderr_logging_is_on_with_subcommand_and_v() {
    let cli = Cli::try_parse_from(["wingetrs", "list", "-v"]).unwrap();
    assert!(wingetrs::cli::stderr_logging(&cli));
    let cli = Cli::try_parse_from(["wingetrs", "list"]).unwrap();
    assert!(!wingetrs::cli::stderr_logging(&cli));
}

#[test]
fn glyphs_is_rejected_after_a_subcommand() {
    assert!(Cli::try_parse_from(["wingetrs", "list", "--glyphs", "ascii"]).is_err());
}

#[test]
fn every_argument_has_a_portuguese_description() {
    use clap::CommandFactory;
    let mut root = wingetrs::cli::Cli::command();
    root.build();
    let mut cmds = vec![root.clone()];
    cmds.extend(root.get_subcommands().cloned());
    for c in cmds {
        for a in c.get_arguments() {
            if a.is_hide_set() {
                continue;
            }
            let help = a.get_help().map(|h| h.to_string()).unwrap_or_default();
            assert!(
                !help.trim().is_empty(),
                "{} --{:?} sem descrição",
                c.get_name(),
                a.get_id()
            );
        }
    }
    let text = wingetrs::cli::Cli::command().render_long_help().to_string();
    for en in ["Print help", "Usage:", "Options:", "Commands:"] {
        assert!(!text.contains(en), "texto em inglês: {en}");
    }
    let mut all = text;
    for n in ["search", "show", "list", "update"] {
        let mut c = wingetrs::cli::Cli::command();
        c.build();
        all += &c
            .find_subcommand_mut(n)
            .unwrap()
            .render_long_help()
            .to_string();
    }
    for en in ["<QUERY>", "<SOURCE>", "<LIMIT>", "<GLYPHS>", "<VERSION>"] {
        assert!(!all.contains(en), "nome de valor em inglês: {en}");
    }
}

#[test]
fn help_snapshots() {
    let mut root = Cli::command();
    root.build();
    insta::assert_snapshot!("help_root", root.render_long_help().to_string());
    for name in ["update", "search", "show", "list"] {
        let mut c = Cli::command();
        c.build();
        let sub = c.find_subcommand_mut(name).unwrap();
        insta::assert_snapshot!(format!("help_{name}"), sub.render_long_help().to_string());
    }
}
