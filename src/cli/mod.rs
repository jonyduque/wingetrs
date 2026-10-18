//! Command line interface.
pub mod install_cmd;
pub mod output;
pub mod picker;
#[cfg(windows)]
mod run_win;
pub mod style;
pub mod winget_cmd;

use crate::cache::{DataDir, now_unix};
use crate::catalog::Catalog;
use crate::catalog::CatalogError;
use crate::catalog::index_file;
use crate::catalog::search::{Found, search_on_disk};
use crate::config::Config;
use crate::core::SourceKind;
use crate::http::Http;
use crate::installed::correlate::{Correlation, ListRow, UpdateState, correlate};
use crate::tui::theme::GlyphSet;
use crate::update::{
    Endpoints, LockWait, Outcome, SourceReport, UpdateError, claim_background_refresh, run_update,
};
use anyhow::Context;
use std::str::FromStr;

fn parse_source(s: &str) -> Result<SourceKind, String> {
    SourceKind::from_str(s)
}

const HELP_TEMPLATE: &str = "{about-with-newline}
Uso: {usage}

{all-args}{after-help}";

#[derive(Debug, clap::Parser)]
#[command(
    name = "wingetrs",
    version,
    about = "Gerenciador de pacotes rápido sobre winget e scoop",
    override_usage = "wingetrs [OPÇÕES] [COMANDO]",
    disable_help_flag = true,
    disable_version_flag = true,
    disable_help_subcommand = true,
    subcommand_help_heading = "Comandos",
    next_help_heading = "Opções",
    help_template = HELP_TEMPLATE,
    after_help = style::root_after_help(),
    styles = style::CLAP_STYLES
)]
pub struct Cli {
    /// Conjunto de glifos da TUI (nerd, unicode ou ascii)
    #[arg(long, value_enum, hide_possible_values = true, value_name = "GLIFOS")]
    pub glyphs: Option<GlyphArg>,
    /// Cores na saída (auto, always ou never)
    #[arg(
        long,
        value_enum,
        global = true,
        default_value = "auto",
        hide_default_value = true,
        hide_possible_values = true,
        value_name = "QUANDO"
    )]
    pub color: style::ColorArg,
    /// Verbosidade (-v, -vv): também loga no stderr
    #[arg(short, long, action = clap::ArgAction::Count, global = true, display_order = 9000)]
    pub verbose: u8,
    /// Mostra a ajuda
    #[arg(short = 'h', long, action = clap::ArgAction::Help, global = true, display_order = 9001)]
    help: Option<bool>,
    /// Mostra a versão
    #[arg(short = 'V', long, action = clap::ArgAction::Version, display_order = 9002)]
    version: Option<bool>,
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
    #[command(
        override_usage = "wingetrs update [OPÇÕES]",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções",
        after_help = "Exemplos:
  wingetrs update
  wingetrs update --force"
    )]
    Update {
        /// Baixa o catálogo mesmo que esteja recente
        #[arg(long)]
        force: bool,
        /// Usado pelo refresh em segundo plano: sem saída
        #[arg(long, hide = true)]
        background: bool,
    },
    /// Busca pacotes
    #[command(
        override_usage = "wingetrs search [OPÇÕES] <TERMO>",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções",
        after_help = "Exemplos:
  wingetrs search powertoys
  wingetrs search powertoys --source scoop -n 5
  wingetrs search git --json"
    )]
    Search {
        /// Texto a buscar (nome, id ou palavra-chave)
        #[arg(help_heading = "Argumentos", value_name = "TERMO")]
        query: String,
        /// Restringe a uma fonte (winget ou scoop)
        #[arg(long, value_parser = parse_source, value_name = "FONTE")]
        source: Option<SourceKind>,
        /// Saída em JSON
        #[arg(long)]
        json: bool,
        /// Máximo de resultados (padrão: 20)
        #[arg(
            short = 'n',
            long,
            default_value_t = 20,
            hide_default_value = true,
            value_name = "N"
        )]
        limit: usize,
    },
    /// Mostra detalhes de um pacote
    #[command(
        override_usage = "wingetrs show [OPÇÕES] <ID>",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções",
        after_help = "Exemplos:
  wingetrs show Microsoft.PowerToys
  wingetrs show git --source scoop
  wingetrs show Microsoft.PowerToys --version 0.80.0 --json"
    )]
    Show {
        /// Identificador do pacote (ex.: Microsoft.PowerToys)
        #[arg(help_heading = "Argumentos")]
        id: String,
        /// Restringe a uma fonte (winget ou scoop)
        #[arg(long, value_parser = parse_source, value_name = "FONTE")]
        source: Option<SourceKind>,
        /// Versão específica a mostrar
        #[arg(long, value_name = "VERSÃO")]
        version: Option<String>,
        /// Saída em JSON
        #[arg(long)]
        json: bool,
    },
    /// Lista pacotes instalados
    #[command(
        override_usage = "wingetrs list [OPÇÕES]",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções",
        after_help = "Exemplos:
  wingetrs list
  wingetrs list --upgradable
  wingetrs list --source winget --json"
    )]
    List {
        /// Mostra só pacotes com atualização disponível
        #[arg(long)]
        upgradable: bool,
        /// Restringe a uma fonte (winget ou scoop)
        #[arg(long, value_parser = parse_source, value_name = "FONTE")]
        source: Option<SourceKind>,
        /// Saída em JSON
        #[arg(long)]
        json: bool,
    },
    /// Instala um app de um bucket do Scoop
    #[command(
        override_usage = "wingetrs install [OPÇÕES] <ALVO>",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções",
        after_help = "Exemplos:
  wingetrs install ripgrep
  wingetrs install extras/vscode --dry-run
  wingetrs install git --bucket main --yes"
    )]
    Install {
        /// App a instalar (id, ou bucket/id)
        #[arg(help_heading = "Argumentos", value_name = "ALVO")]
        target: String,
        /// Restringe a um bucket
        #[arg(long, value_name = "BUCKET")]
        bucket: Option<String>,
        /// Mostra o plano e sai, sem alterar nada
        #[arg(long)]
        dry_run: bool,
        /// Não pergunta nada (falha se o alvo for ambíguo)
        #[arg(long)]
        yes: bool,
        /// Permite usar o Windows Installer (msiexec /a) num MSI que o extrator não suporta
        #[arg(long)]
        allow_msiexec: bool,
    },
    /// Remove um app instalado pelo wingetrs
    #[command(
        override_usage = "wingetrs uninstall [OPÇÕES] <ALVO>",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções",
        after_help = "Exemplos:
  wingetrs uninstall ripgrep
  wingetrs uninstall ripgrep --purge --yes"
    )]
    Uninstall {
        /// App a remover
        #[arg(help_heading = "Argumentos", value_name = "ALVO")]
        target: String,
        /// Apaga também os dados persistentes do app
        #[arg(long)]
        purge: bool,
        /// Mostra o plano e sai, sem alterar nada
        #[arg(long)]
        dry_run: bool,
        /// Não pergunta nada (limpa só resíduos de confiança alta)
        #[arg(long)]
        yes: bool,
    },
    /// Atualiza apps instalados pelo wingetrs
    #[command(
        override_usage = "wingetrs upgrade [OPÇÕES] [ALVO]",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções",
        after_help = "Exemplos:
  wingetrs upgrade               lista o que pode ser atualizado
  wingetrs upgrade ripgrep
  wingetrs upgrade --all --yes"
    )]
    Upgrade {
        /// App a atualizar
        #[arg(
            help_heading = "Argumentos",
            value_name = "ALVO",
            conflicts_with = "all"
        )]
        target: Option<String>,
        /// Atualiza todos os apps com versão nova
        #[arg(long)]
        all: bool,
        /// Mantém a versão antiga em disco
        #[arg(long)]
        keep: bool,
        /// Mostra o plano e sai, sem alterar nada
        #[arg(long)]
        dry_run: bool,
        /// Não pergunta nada
        #[arg(long)]
        yes: bool,
        /// Permite usar o Windows Installer (msiexec /a) num MSI que o extrator não suporta
        #[arg(long)]
        allow_msiexec: bool,
    },
    /// Gerencia os buckets do Scoop
    #[command(
        override_usage = "wingetrs bucket <COMANDO>",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções",
        subcommand_help_heading = "Comandos",
        subcommand_required = true,
        arg_required_else_help = true,
        after_help = "Exemplos:
  wingetrs bucket list
  wingetrs bucket add extras
  wingetrs bucket add meu-bucket usuario/repositorio --yes
  wingetrs bucket rm meu-bucket"
    )]
    Bucket {
        #[command(subcommand)]
        action: BucketAction,
    },
    /// Reconstrói o estado a partir do disco
    #[command(
        override_usage = "wingetrs repair",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções",
        after_help = "Exemplos:
  wingetrs repair"
    )]
    Repair,
    /// Auxiliar elevado da ponte do winget (iniciado pela TUI via UAC)
    #[command(hide = true)]
    BridgeWorker {
        /// Pipe da TUI
        #[arg(long)]
        pipe: String,
        /// PID da TUI que serve o pipe
        #[arg(long)]
        parent: u32,
    },
}

#[derive(Debug, PartialEq, clap::Subcommand)]
pub enum BucketAction {
    /// Adiciona um bucket (nome conhecido, ou nome e repositório do GitHub)
    #[command(
        override_usage = "wingetrs bucket add [OPÇÕES] <NOME> [REPO]",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções"
    )]
    Add {
        /// Nome do bucket
        #[arg(help_heading = "Argumentos", value_name = "NOME")]
        name: String,
        /// owner/repo ou URL do GitHub (dispensável para buckets conhecidos)
        #[arg(help_heading = "Argumentos", value_name = "REPO")]
        url: Option<String>,
        /// Confirma um repositório desconhecido sem perguntar
        #[arg(long)]
        yes: bool,
    },
    /// Lista os buckets configurados
    #[command(
        override_usage = "wingetrs bucket list",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções"
    )]
    List,
    /// Remove um bucket (recusa se houver apps instalados dele)
    #[command(
        override_usage = "wingetrs bucket rm <NOME>",
        help_template = HELP_TEMPLATE,
        next_help_heading = "Opções"
    )]
    Rm {
        /// Nome do bucket
        #[arg(help_heading = "Argumentos", value_name = "NOME")]
        name: String,
    },
}

/// Whether the stderr log layer is wanted: never for the TUI (it owns the terminal).
pub fn stderr_logging(cli: &Cli) -> bool {
    cli.command.is_some() && cli.verbose > 0
}

/// File log filter: our crate at DEBUG, dependencies only from WARN.
pub fn file_log_filter() -> tracing_subscriber::filter::Targets {
    use tracing::Level;
    tracing_subscriber::filter::Targets::new()
        .with_default(Level::WARN)
        .with_target("wingetrs", Level::DEBUG)
}

/// File log layer: wingetrs at DEBUG, with the duration of every closed span.
fn file_layer<S, W>(writer: W) -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    use tracing_subscriber::Layer;
    use tracing_subscriber::fmt::format::FmtSpan;
    tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_span_events(FmtSpan::CLOSE)
        .with_filter(file_log_filter())
}

/// Span durations on stderr only with `-vv` (`-v` stays quiet).
fn stderr_span_events(verbose: u8) -> tracing_subscriber::fmt::format::FmtSpan {
    use tracing_subscriber::fmt::format::FmtSpan;
    if verbose > 1 {
        FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    }
}

/// Stderr log filter for `-v`/`-vv`: same principle, dependencies stay at WARN.
fn stderr_log_filter(verbose: u8) -> tracing_subscriber::filter::Targets {
    use tracing::Level;
    let level = if verbose > 1 {
        Level::DEBUG
    } else {
        Level::INFO
    };
    tracing_subscriber::filter::Targets::new()
        .with_default(Level::WARN)
        .with_target("wingetrs", level)
}

/// Number of daily log files kept.
const LOG_KEEP: usize = 14;

/// Delete the oldest `wingetrs.log.YYYY-MM-DD` files beyond the `keep` newest.
/// I/O errors are only logged; other files are ignored.
pub fn prune_logs(dir: &std::path::Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            n.strip_prefix("wingetrs.log.").is_some_and(|d| {
                d.len() == 10
                    && d.bytes().enumerate().all(|(i, b)| {
                        if i == 4 || i == 7 {
                            b == b'-'
                        } else {
                            b.is_ascii_digit()
                        }
                    })
            })
        })
        .collect();
    names.sort();
    let excess = names.len().saturating_sub(keep);
    for n in &names[..excess] {
        if let Err(e) = std::fs::remove_file(dir.join(n)) {
            tracing::warn!("could not remove old log {n}: {e}");
        }
    }
}

fn init_logging(data: &DataDir, verbose: u8) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    prune_logs(&data.log_dir(), LOG_KEEP);
    let appender = tracing_appender::rolling::daily(data.log_dir(), "wingetrs.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let file = file_layer(writer);
    let stderr = (verbose > 0).then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_span_events(stderr_span_events(verbose))
            .with_filter(stderr_log_filter(verbose))
    });
    let _ = tracing_subscriber::registry()
        .with(file)
        .with(stderr)
        .try_init();
    guard
}

/// Detached `wingetrs update --background`; the spawn itself is not tested.
fn spawn_background_update() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["update", "--background"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NO_WINDOW
        cmd.creation_flags(0x0000_0008 | 0x0800_0000);
    }
    if let Err(e) = cmd.spawn() {
        tracing::warn!("falha ao iniciar atualização em segundo plano: {e}");
    }
}

fn all_urls(ep: &Endpoints) -> Vec<&str> {
    std::iter::once(ep.winget_index.as_str())
        .chain(ep.scoop.iter().map(|(_, u)| u.as_str()))
        .collect()
}

/// Triggers a detached refresh when any source was not checked recently.
fn refresh_if_stale(data: &DataDir, ep: &Endpoints) {
    if claim_background_refresh(data, &all_urls(ep), now_unix()) {
        spawn_background_update();
    }
}

/// Ensures a catalog exists (blocking update when empty) and triggers a
/// detached refresh when it is stale.
async fn ensure_catalog(data: &DataDir, cfg: &Config, http: &Http) -> anyhow::Result<Catalog> {
    Ok(ensure_catalog_ran(data, cfg, http, None).await?.0)
}

/// Like `ensure_catalog`; the flag is true when a blocking update had to run.
async fn ensure_catalog_ran(
    data: &DataDir,
    cfg: &Config,
    http: &Http,
    early: Option<&mut EarlySearch>,
) -> anyhow::Result<(Catalog, bool)> {
    let ep = Endpoints::production(cfg)?;
    let path = data.catalog_path();
    // Cheap path: an existing, populated catalog opens read-only (no DDL, no journal write).
    let existing = if path.exists() {
        Catalog::open_read_only(&path)
            .and_then(|c| c.is_empty().map(|empty| (c, empty)))
            .ok()
    } else {
        None
    };
    let (catalog, empty) = match existing {
        Some(found) => found,
        None => {
            let c = Catalog::open(&path)?;
            let empty = c.is_empty()?;
            (c, empty)
        }
    };
    if empty {
        drop(catalog);
        // Never let the early reader run while the update rewrites the catalog.
        if let Some(e) = early {
            let _ = e.join();
        }
        let reports = run_update(data, &ep, http, true, LockWait::from_env()).await?;
        let catalog = Catalog::open(&data.catalog_path())?;
        if catalog.is_empty()? {
            for r in &reports {
                if let Outcome::Failed { message: m, .. } = &r.outcome {
                    tracing::warn!("{}: {m}", r.source);
                }
            }
            anyhow::bail!("sem catálogo local e sem rede");
        }
        return Ok((catalog, true));
    }
    refresh_if_stale(data, &ep);
    Ok((catalog, false))
}

/// Search through `search.idx` when it is present and valid; otherwise through the catalog file.
fn search_best(
    data: &DataDir,
    query: &str,
    source: Option<SourceKind>,
    limit: usize,
) -> Result<Vec<Found>, CatalogError> {
    let out = search_early(
        &data.search_index_path(),
        &data.catalog_path(),
        query,
        source,
        limit,
    );
    out.log_warning();
    out.result
}

fn print_reports(reports: &[SourceReport]) {
    for r in reports {
        anstream::println!("{}", output::report_line(r));
    }
}

/// Result of the early search plus the index problem seen on the way: the thread runs before the
/// log is initialised, so the caller records the warning after the join.
struct EarlyOutcome {
    result: Result<Vec<Found>, CatalogError>,
    index_warning: Option<String>,
    /// Which path answered: `"idx"` (search.idx) or `"catalog"` (catalog file).
    via: &'static str,
    elapsed_ms: f64,
}

impl EarlyOutcome {
    fn log_warning(&self) {
        if let Some(w) = &self.index_warning {
            tracing::warn!("índice de busca inutilizável, usando o catálogo: {w}");
        }
        tracing::debug!(
            stage = "search",
            via = self.via,
            ms = self.elapsed_ms,
            "stage"
        );
    }
}

/// The search thread started before setup. Joined on every path: explicitly before a blocking
/// update, or by `Drop` (e.g. after an error), so it never outlives the work that follows.
struct EarlySearch(Option<std::thread::JoinHandle<EarlyOutcome>>);

impl EarlySearch {
    /// Waits for the thread; `None` when already taken. A worker panic is propagated.
    fn join(&mut self) -> Option<EarlyOutcome> {
        match self.0.take()?.join() {
            Ok(out) => Some(out),
            Err(p) => std::panic::resume_unwind(p),
        }
    }
}

impl Drop for EarlySearch {
    fn drop(&mut self) {
        // While already unwinding, a second panic would abort: just reap the thread.
        if std::thread::panicking() {
            if let Some(h) = self.0.take() {
                let _ = h.join();
            }
        } else {
            let _ = self.join();
        }
    }
}

/// Search body: `search.idx` first, the catalog file when the index is absent or invalid.
fn search_early(
    idx: &std::path::Path,
    catalog: &std::path::Path,
    query: &str,
    source: Option<SourceKind>,
    limit: usize,
) -> EarlyOutcome {
    let started = std::time::Instant::now();
    let mut index_warning = None;
    match index_file::search(idx, query, source, limit) {
        Ok(Some(found)) => {
            return EarlyOutcome {
                result: Ok(found),
                index_warning,
                via: "idx",
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            };
        }
        Ok(None) => {}
        Err(e) => index_warning = Some(e.to_string()),
    }
    let result = search_on_disk(catalog, query, source, limit);
    EarlyOutcome {
        result,
        index_warning,
        via: "catalog",
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    }
}

async fn run_async(
    cli: Cli,
    data: DataDir,
    cfg: Config,
    mut early: Option<EarlySearch>,
    after_log: std::time::Instant,
) -> anyhow::Result<()> {
    let http = tracing::debug_span!("startup.http").in_scope(Http::new)?;
    http.set_allow_http_redirect(cfg.behavior.allow_http_redirect);
    let Some(cmd) = cli.command else {
        anyhow::bail!("TUI ainda não implementada");
    };
    match cmd {
        Command::Update { force, background } => {
            let ep = Endpoints::production(&cfg)?;
            let wait = if background {
                LockWait::NoWait
            } else {
                LockWait::from_env()
            };
            let reports = match run_update(&data, &ep, &http, force, wait).await {
                Err(UpdateError::Busy) if background => {
                    tracing::debug!("another update holds the lock; background refresh skipped");
                    return Ok(());
                }
                other => other?,
            };
            if !background {
                print_reports(&reports);
            }
            Ok(())
        }
        Command::Search {
            query,
            source,
            json,
            limit,
        } => {
            // A valid non-empty search.idx proves a catalog exists: skip opening SQLite.
            let (catalog, updated) = if data.catalog_path().exists()
                && tracing::debug_span!("search.probe")
                    .in_scope(|| index_file::probe(&data.search_index_path()))
                    .is_some_and(|n| n > 0)
            {
                let ep = Endpoints::production(&cfg)?;
                tracing::debug_span!("search.refresh_check")
                    .in_scope(|| refresh_if_stale(&data, &ep));
                (None, false)
            } else {
                let (c, updated) = ensure_catalog_ran(&data, &cfg, &http, early.as_mut()).await?;
                (Some(c), updated)
            };
            let early = tracing::debug_span!("search.join_wait")
                .in_scope(|| early.as_mut().and_then(EarlySearch::join));
            if let Some(out) = &early {
                out.log_warning();
            }
            // An early search error (not a panic) falls back to a fresh search.
            let found = match early.map(|o| o.result) {
                Some(Ok(found)) if !updated => found,
                _ => tracing::debug_span!("search.fallback")
                    .in_scope(|| search_best(&data, &query, source, limit))?,
            };
            let rows: Vec<_> = found.into_iter().map(|f| f.entry).collect();
            {
                let _print = tracing::debug_span!("search.print").entered();
                if json {
                    output::print_json(&rows)?;
                } else {
                    anstream::print!("{}", output::search_table(&rows));
                }
            }
            // The process exits next: skip closing the db.
            std::mem::forget(catalog);
            Ok(())
        }
        Command::Show {
            id,
            source,
            version,
            json,
        } => {
            let catalog = ensure_catalog(&data, &cfg, &http).await?;
            let mut found =
                tracing::debug_span!("show.find_by_id").in_scope(|| catalog.find_by_id(&id))?;
            if let Some(k) = source {
                found.retain(|p| p.entry.source == k);
            }
            let pkg = match found.len() {
                0 => anyhow::bail!("pacote não encontrado: {id}"),
                1 => found.remove(0),
                _ => {
                    let names: Vec<_> =
                        found.iter().map(|p| p.entry.source_name.as_str()).collect();
                    anyhow::bail!(
                        "pacote ambíguo: {id} existe em {}; use --source",
                        names.join(", ")
                    )
                }
            };
            let details = match pkg.entry.source {
                SourceKind::Winget => {
                    let hash = pkg
                        .winget_hash
                        .as_deref()
                        .context("pacote winget sem hash de manifesto")?;
                    let blobs = crate::cache::BlobStore::new(data.blobs_dir());
                    crate::sources::winget::details::details(
                        &http,
                        &blobs,
                        crate::sources::winget::CDN_BASE,
                        &pkg.entry.id,
                        hash,
                        version.as_deref(),
                    )
                    .await?
                }
                SourceKind::Scoop => {
                    let raw = pkg.raw.as_deref().context("pacote scoop sem manifesto")?;
                    crate::sources::scoop::manifest::details_from_raw(&pkg.entry.id, raw)?
                }
            };
            if json {
                output::print_json(&details)?;
            } else {
                anstream::print!("{}", output::details_text(&details));
            }
            Ok(())
        }
        Command::List {
            upgradable,
            source,
            json,
        } => {
            tracing::debug!(
                ms = after_log.elapsed().as_secs_f64() * 1000.0,
                "list.setup"
            );
            // Scan, tracking and normalizer start now so they overlap with the catalog open.
            let scan_job = std::thread::spawn(|| {
                crate::installed::timed("scan_all", crate::installed::scan_all)
            });
            let tracking_job = std::thread::spawn(|| {
                crate::installed::timed(
                    "winget_tracking",
                    crate::installed::winget_tracking::load_default,
                )
            });
            let norm_job = std::thread::spawn(|| {
                crate::installed::timed(
                    "normalizer_new",
                    crate::installed::normalize::NameNormalizer::new,
                )
            });
            let t0 = std::time::Instant::now();
            let catalog = ensure_catalog(&data, &cfg, &http).await;
            // Join every early thread on every path before propagating a catalog error.
            let scan_res = scan_job.join();
            let tracking_res = tracking_job.join();
            let norm_res = norm_job.join();
            let catalog = catalog?;
            tracing::info!(
                step = "open_catalog",
                ms = t0.elapsed().as_secs_f64() * 1000.0,
                "span"
            );
            // A worker panic is a bug: re-raise it instead of returning partial results.
            let scan = scan_res.unwrap_or_else(|p| std::panic::resume_unwind(p));
            let tracking = tracking_res.unwrap_or_else(|p| std::panic::resume_unwind(p));
            let normalizer = norm_res.unwrap_or_else(|p| std::panic::resume_unwind(p));
            for e in &scan.errors {
                tracing::warn!("coleta de instalados: {e}");
            }
            let t = std::time::Instant::now();
            let overrides = crate::installed::overrides::load(&data);
            let mut rows = crate::installed::timed("correlate", || {
                correlate(&scan.apps, &catalog, &normalizer, &tracking, &overrides)
            })?;
            tracing::info!(
                scan_ms = scan.elapsed_ms,
                correlate_ms = t.elapsed().as_millis(),
                "list"
            );
            if let Some(kind) = source {
                rows.retain(|r| row_kind(r) == Some(kind));
            }
            if upgradable {
                rows.retain(|r| matches!(r.state, UpdateState::Available { .. }));
            }
            crate::installed::timed("output", || {
                if json {
                    output::print_json(&rows)?;
                } else {
                    anstream::print!("{}", output::list_text(&rows, upgradable));
                }
                Ok::<_, anyhow::Error>(())
            })?;
            Ok(())
        }
        cmd @ (Command::Install { .. }
        | Command::Uninstall { .. }
        | Command::Upgrade { .. }
        | Command::Repair) => {
            #[cfg(windows)]
            {
                let catalog = ensure_catalog(&data, &cfg, &http).await?;
                run_win::run(cmd, &data, &cfg, &http, &catalog).await
            }
            #[cfg(not(windows))]
            {
                let _ = cmd;
                anyhow::bail!("instalar e remover apps só funciona no Windows")
            }
        }
        Command::Bucket { action } => run_bucket(action, &data, cfg),
        // Handled in `run`, before the runtime exists.
        Command::BridgeWorker { .. } => anyhow::bail!("bridge-worker fora do lugar"),
    }
}

/// `bucket add|list|rm`: edits `config.toml` (and the catalog on `rm`).
fn run_bucket(action: BucketAction, data: &DataDir, mut cfg: Config) -> anyhow::Result<()> {
    use crate::buckets::{self, BucketError};
    match action {
        BucketAction::List => {
            for (name, repo) in buckets::resolved(&cfg)? {
                anstream::println!("{name}  {repo}");
            }
            Ok(())
        }
        BucketAction::Add { name, url, yes } => {
            let spec = match (url, buckets::known_repo(&name)) {
                (Some(u), _) => u,
                (None, Some(known)) => known.to_string(),
                (None, None) => anyhow::bail!(
                    "o bucket {name} não é conhecido; informe o repositório: wingetrs bucket add {name} <owner/repo>"
                ),
            };
            match buckets::add(&mut cfg, &name, &spec, yes) {
                Err(BucketError::NeedsConfirmation { name, repo }) if picker::is_interactive() => {
                    let ui = install_cmd::TtyUi::new(&cfg);
                    let q = format!("O bucket {name} ({repo}) não é conhecido. Adicionar?");
                    if !install_cmd::Ui::confirm(&ui, &q)? {
                        anstream::println!("Cancelado.");
                        return Ok(());
                    }
                    buckets::add(&mut cfg, &name, &spec, true)?;
                }
                other => other?,
            }
            cfg.save(&data.config_path())?;
            anstream::println!(
                "Bucket {name} adicionado. Rode `wingetrs update` para baixar o catálogo."
            );
            Ok(())
        }
        BucketAction::Rm { name } => {
            let state_db = data.install_layout().state_db();
            buckets::remove_and_cleanup(data, &mut cfg, &name, &state_db)?;
            cfg.save(&data.config_path())?;
            anstream::println!("Bucket {name} removido.");
            Ok(())
        }
    }
}

/// Source kind a row was correlated against; `None` when unmatched.
fn row_kind(r: &ListRow) -> Option<SourceKind> {
    match &r.correlation {
        Correlation::Matched { source, .. } if source.starts_with("scoop:") => {
            Some(SourceKind::Scoop)
        }
        Correlation::Matched { .. } | Correlation::Ambiguous { .. } => Some(SourceKind::Winget),
        Correlation::Unmatched => None,
    }
}

pub fn run(cli: Cli) -> anyhow::Result<()> {
    // The elevated helper never touches the disk: no data dir, no log (it runs as admin, and
    // %LOCALAPPDATA% is the user's to plant junctions in).
    if let Some(Command::BridgeWorker { pipe, parent }) = &cli.command {
        return crate::bridge::elevated::run_worker(pipe, *parent)
            .map_err(|e| anyhow::anyhow!("auxiliar elevado: {e}"));
    }
    style::init(cli.color);
    let data = DataDir::from_env()?;
    // Start the search before logging, config and runtime setup: it only needs the file.
    let early: Option<EarlySearch> = match &cli.command {
        Some(Command::Search {
            query,
            source,
            limit,
            ..
        }) if data.catalog_path().exists() => {
            let (path, query, source, limit) =
                (data.search_index_path(), query.clone(), *source, *limit);
            let catalog = data.catalog_path();
            Some(EarlySearch(Some(std::thread::spawn(move || {
                search_early(&path, &catalog, &query, source, limit)
            }))))
        }
        _ => None,
    };
    // Before the subscriber exists: time with Instant and emit as events after init_logging.
    let t0 = std::time::Instant::now();
    data.ensure()?;
    let ensure_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let t1 = std::time::Instant::now();
    let _guard = init_logging(&data, if stderr_logging(&cli) { cli.verbose } else { 0 });
    let log_ms = t1.elapsed().as_secs_f64() * 1000.0;
    tracing::debug!(target: "wingetrs::startup", stage = "startup.ensure", ms = ensure_ms);
    tracing::debug!(target: "wingetrs::startup", stage = "startup.log", ms = log_ms);
    let after_log = std::time::Instant::now();
    let cfg =
        tracing::debug_span!("startup.config").in_scope(|| Config::load(&data.config_path()))?;
    if cli.command.is_none() {
        // The TUI builds its own runtime; glyphs: flag > config.toml > nerd.
        let glyphs = match cli.glyphs {
            Some(GlyphArg::Nerd) => GlyphSet::Nerd,
            Some(GlyphArg::Unicode) => GlyphSet::Unicode,
            Some(GlyphArg::Ascii) => GlyphSet::Ascii,
            None => match cfg.glyphs.as_str() {
                "unicode" => GlyphSet::Unicode,
                "ascii" => GlyphSet::Ascii,
                _ => GlyphSet::Nerd,
            },
        };
        return crate::tui::run::run(glyphs, cli.glyphs.is_some());
    }
    let rt = tracing::debug_span!("startup.runtime").in_scope(tokio::runtime::Runtime::new)?;
    let result = rt.block_on(run_async(cli, data, cfg, early, after_log));
    tracing::debug_span!("startup.runtime_drop").in_scope(|| drop(rt));
    result
}

#[cfg(test)]
mod early_tests {
    use super::*;

    #[test]
    fn early_search_propagates_worker_panic() {
        let mut early = EarlySearch(Some(std::thread::spawn(|| -> EarlyOutcome {
            panic!("worker boom")
        })));
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| early.join()));
        assert!(r.is_err(), "worker panic must be resumed, not swallowed");
    }

    #[test]
    fn early_search_reports_which_path_answered() {
        use crate::catalog::{NewPackage, WingetMaps};
        let dir = tempfile::tempdir().unwrap();
        let (idx, cat) = (dir.path().join("search.idx"), dir.path().join("catalog.db"));
        let mut c = Catalog::open(&cat).unwrap();
        c.replace_source(
            "scoop:main",
            SourceKind::Scoop,
            "v",
            1,
            &[NewPackage {
                id: "ripgrep".into(),
                name: "ripgrep".into(),
                version: "1".into(),
                ..Default::default()
            }],
            &WingetMaps::default(),
        )
        .unwrap();
        let out = search_early(&idx, &cat, "ripgrep", None, 10);
        assert_eq!(out.via, "catalog");
        index_file::write(&c, &idx).unwrap();
        let out = search_early(&idx, &cat, "ripgrep", None, 10);
        assert_eq!(out.via, "idx");
        assert_eq!(out.result.unwrap().len(), 1);
    }
}

#[cfg(test)]
mod log_tests {
    use super::*;
    use tracing::Level;

    #[test]
    fn file_filter_keeps_wingetrs_debug_and_drops_dependency_noise() {
        let f = file_log_filter();
        assert!(f.would_enable("wingetrs::install::ops", &Level::DEBUG));
        assert!(!f.would_enable("wingetrs", &Level::TRACE));
        assert!(!f.would_enable("h2::proto::streams", &Level::TRACE));
        assert!(!f.would_enable("hyper_util::client", &Level::DEBUG));
        assert!(f.would_enable("rustls", &Level::WARN));
    }

    #[derive(Clone, Default)]
    struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn file_layer_logs_the_duration_of_a_closed_wingetrs_span() {
        use tracing_subscriber::layer::SubscriberExt;
        let buf = Buf::default();
        let w = buf.clone();
        let sub = tracing_subscriber::registry().with(file_layer(move || w.clone()));
        tracing::subscriber::with_default(sub, || {
            let _a = tracing::debug_span!(target: "wingetrs::install::ops", "install.prefetch")
                .entered();
            let _b = tracing::debug_span!(target: "h2::proto", "noise").entered();
        });
        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let line = out
            .lines()
            .find(|l| l.contains("install.prefetch") && l.contains("close"))
            .unwrap_or_else(|| panic!("sem linha de fechamento: {out}"));
        assert!(line.contains("time.busy"), "{line}");
        assert!(!out.contains("noise"), "{out}");
    }

    #[test]
    fn stderr_shows_span_durations_only_with_vv() {
        use tracing_subscriber::fmt::format::FmtSpan;
        assert_eq!(stderr_span_events(1), FmtSpan::NONE);
        assert_eq!(stderr_span_events(2), FmtSpan::CLOSE);
    }

    #[test]
    fn prune_logs_keeps_the_newest_and_ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        for d in 1..=16 {
            std::fs::write(dir.path().join(format!("wingetrs.log.2026-09-{d:02}")), "x").unwrap();
        }
        std::fs::write(dir.path().join("outro.txt"), "x").unwrap();
        prune_logs(dir.path(), 14);
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut want: Vec<String> = (3..=16)
            .map(|d| format!("wingetrs.log.2026-09-{d:02}"))
            .collect();
        want.push("outro.txt".into());
        want.sort();
        assert_eq!(names, want);
    }

    #[test]
    fn prune_logs_on_a_missing_dir_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        prune_logs(&dir.path().join("nope"), 14);
    }

    #[test]
    fn stderr_filter_follows_verbosity_and_drops_dependency_noise() {
        let f = stderr_log_filter(1);
        assert!(f.would_enable("wingetrs::x", &Level::INFO));
        assert!(!f.would_enable("wingetrs::x", &Level::DEBUG));
        assert!(!f.would_enable("h2", &Level::DEBUG));
        let f = stderr_log_filter(2);
        assert!(f.would_enable("wingetrs::x", &Level::DEBUG));
        assert!(!f.would_enable("hyper", &Level::DEBUG));
    }
}
