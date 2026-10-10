//! Command line interface.
pub mod output;
pub mod picker;

use crate::cache::{DataDir, HttpState, now_unix};
use crate::catalog::Catalog;
use crate::catalog::CatalogError;
use crate::catalog::index_file;
use crate::catalog::search::{Found, search_on_disk};
use crate::config::Config;
use crate::core::SourceKind;
use crate::http::Http;
use crate::installed::correlate::{Correlation, ListRow, UpdateState, correlate};
use crate::tui::theme::GlyphSet;
use crate::update::{Endpoints, LockWait, Outcome, SourceReport, needs_refresh, run_update};
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
    after_help = "Exemplos:
  wingetrs                       abre a TUI
  wingetrs update                atualiza o catálogo
  wingetrs search powertoys      busca pacotes
  wingetrs show Microsoft.PowerToys
  wingetrs list --upgradable     lista pacotes com atualização"
)]
pub struct Cli {
    /// Conjunto de glifos da TUI (nerd, unicode ou ascii)
    #[arg(long, value_enum, hide_possible_values = true, value_name = "GLIFOS")]
    pub glyphs: Option<GlyphArg>,
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
}

/// Whether the stderr log layer is wanted: never for the TUI (it owns the terminal).
pub fn stderr_logging(cli: &Cli) -> bool {
    cli.command.is_some() && cli.verbose > 0
}

fn init_logging(data: &DataDir, verbose: u8) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let appender = tracing_appender::rolling::daily(data.log_dir(), "wingetrs.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let file = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false);
    let stderr = (verbose > 0).then(|| {
        let level = if verbose > 1 {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        };
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(tracing_subscriber::filter::LevelFilter::from_level(level))
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
    let state = HttpState::load(&data.http_state_path());
    if needs_refresh(&state, &all_urls(ep), now_unix()) {
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
        println!("{}", output::report_line(r));
    }
}

/// Result of the early search plus the index problem seen on the way: the thread runs before the
/// log is initialised, so the caller records the warning after the join.
struct EarlyOutcome {
    result: Result<Vec<Found>, CatalogError>,
    index_warning: Option<String>,
}

impl EarlyOutcome {
    fn log_warning(&self) {
        if let Some(w) = &self.index_warning {
            tracing::warn!("índice de busca inutilizável, usando o catálogo: {w}");
        }
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
    let mut index_warning = None;
    match index_file::search(idx, query, source, limit) {
        Ok(Some(found)) => {
            return EarlyOutcome {
                result: Ok(found),
                index_warning,
            };
        }
        Ok(None) => {}
        Err(e) => index_warning = Some(e.to_string()),
    }
    EarlyOutcome {
        result: search_on_disk(catalog, query, source, limit),
        index_warning,
    }
}

async fn run_async(
    cli: Cli,
    data: DataDir,
    cfg: Config,
    mut early: Option<EarlySearch>,
) -> anyhow::Result<()> {
    let http = Http::new()?;
    let Some(cmd) = cli.command else {
        anyhow::bail!("TUI ainda não implementada");
    };
    match cmd {
        Command::Update { force, background } => {
            let ep = Endpoints::production(&cfg)?;
            let reports = run_update(&data, &ep, &http, force, LockWait::from_env()).await?;
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
                && index_file::probe(&data.search_index_path()).is_some_and(|n| n > 0)
            {
                let ep = Endpoints::production(&cfg)?;
                refresh_if_stale(&data, &ep);
                (None, false)
            } else {
                let (c, updated) = ensure_catalog_ran(&data, &cfg, &http, early.as_mut()).await?;
                (Some(c), updated)
            };
            let early = early.as_mut().and_then(EarlySearch::join);
            if let Some(out) = &early {
                out.log_warning();
            }
            // An early search error (not a panic) falls back to a fresh search.
            let found = match early.map(|o| o.result) {
                Some(Ok(found)) if !updated => found,
                _ => search_best(&data, &query, source, limit)?,
            };
            let rows: Vec<_> = found.into_iter().map(|f| f.entry).collect();
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                print!("{}", output::search_table(&rows));
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
            let mut found = catalog.find_by_id(&id)?;
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
                println!("{}", serde_json::to_string_pretty(&details)?);
            } else {
                print!("{}", output::details_text(&details));
            }
            Ok(())
        }
        Command::List {
            upgradable,
            source,
            json,
        } => {
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
            let mut rows = crate::installed::timed("correlate", || {
                correlate(&scan.apps, &catalog, &normalizer, &tracking)
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
                    println!("{}", serde_json::to_string_pretty(&rows)?);
                } else {
                    print!("{}", output::list_text(&rows, upgradable));
                }
                Ok::<_, anyhow::Error>(())
            })?;
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
    data.ensure()?;
    let _guard = init_logging(&data, if stderr_logging(&cli) { cli.verbose } else { 0 });
    let cfg = Config::load(&data.config_path())?;
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
        return crate::tui::run::run(glyphs);
    }
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_async(cli, data, cfg, early))
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
}
