//! Command line interface.
use crate::core::SourceKind;
use std::str::FromStr;

fn parse_source(s: &str) -> Result<SourceKind, String> {
    SourceKind::from_str(s)
}

#[derive(Debug, clap::Parser)]
#[command(
    name = "wingetrs",
    version,
    about = "Gerenciador de pacotes rápido sobre winget e scoop"
)]
pub struct Cli {
    /// Conjunto de glifos da TUI
    #[arg(long, value_enum, global = true)]
    pub glyphs: Option<GlyphArg>,
    /// Verbosidade (-v, -vv): também loga no stderr
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum GlyphArg {
    Nerd,
    Unicode,
    Ascii,
}

#[derive(Debug, PartialEq, clap::Subcommand)]
pub enum Command {
    /// Atualiza o catálogo local
    Update {
        #[arg(long)]
        force: bool,
        /// Usado pelo refresh em segundo plano: sem saída
        #[arg(long, hide = true)]
        background: bool,
    },
    /// Busca pacotes
    Search {
        query: String,
        #[arg(long, value_parser = parse_source)]
        source: Option<SourceKind>,
        #[arg(long)]
        json: bool,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Mostra detalhes de um pacote
    Show {
        id: String,
        #[arg(long, value_parser = parse_source)]
        source: Option<SourceKind>,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Lista pacotes instalados
    List {
        #[arg(long)]
        upgradable: bool,
        #[arg(long, value_parser = parse_source)]
        source: Option<SourceKind>,
        #[arg(long)]
        json: bool,
    },
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Command::Update { .. } => "update",
            Command::Search { .. } => "search",
            Command::Show { .. } => "show",
            Command::List { .. } => "list",
        }
    }
}

pub fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        None => anyhow::bail!("TUI ainda não implementada"),
        Some(cmd) => anyhow::bail!("comando ainda não implementado: {}", cmd.name()),
    }
}
