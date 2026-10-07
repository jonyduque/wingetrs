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
fn glyphs_is_global() {
    let cli = Cli::try_parse_from(["wingetrs", "list", "--glyphs", "ascii"]).unwrap();
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
