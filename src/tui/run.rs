//! TUI event loop: terminal setup, input stream, command execution.
use crate::cache::{BlobStore, DataDir, HttpState, now_unix};
use crate::catalog::Catalog;
use crate::config::Config;
use crate::core::SourceKind;
use crate::http::Http;
use crate::install::ops::OpsCtx;
use crate::install::residue::Residue;
use crate::installed::correlate::correlate;
use crate::installed::normalize::NameNormalizer;
use crate::installed::scan_all;
use crate::tui::app::{Action, App, Cmd, InstallFail, Msg, Planned, update};
use crate::tui::jobs;
use crate::tui::theme::{GlyphSet, Theme, detect_depth};
use crate::tui::view::view;
use crate::update::{Endpoints, LockWait, UpdateError, needs_refresh, run_update};
use anyhow::Context;
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

const TICK: Duration = Duration::from_millis(100);
const DETAILS_DEBOUNCE: Duration = Duration::from_millis(150);
/// A `draw` slower than this (one 60 Hz frame) is logged at debug level.
const SLOW_DRAW_US: u64 = 16_000;

/// Loop counters for measuring responsiveness; logged once when the loop ends, on every path.
struct LoopStats {
    start: Instant,
    frames: u64,
    keys: u64,
    ticks: u64,
    max_draw_us: u64,
}

impl LoopStats {
    fn new() -> LoopStats {
        LoopStats {
            start: Instant::now(),
            frames: 0,
            keys: 0,
            ticks: 0,
            max_draw_us: 0,
        }
    }

    fn count(&mut self, msg: &Msg) {
        match msg {
            Msg::Key(_) => self.keys += 1,
            Msg::Tick => self.ticks += 1,
            _ => {}
        }
    }

    /// Times one `draw`; `batch` is how many messages it covers.
    fn draw(
        &mut self,
        batch: usize,
        f: impl FnOnce() -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let t = Instant::now();
        let r = f();
        let draw_us = u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.frames += 1;
        self.max_draw_us = self.max_draw_us.max(draw_us);
        if draw_us > SLOW_DRAW_US {
            tracing::debug!(draw_us, batch, "frame lento");
        }
        r
    }
}

impl Drop for LoopStats {
    fn drop(&mut self) {
        tracing::info!(
            frames = self.frames,
            keys = self.keys,
            ticks = self.ticks,
            max_draw_us = self.max_draw_us,
            secs = self.start.elapsed().as_secs_f64(),
            "tui stats"
        );
    }
}

struct Ctx {
    data: DataDir,
    cfg: Config,
    http: Http,
    tx: UnboundedSender<Msg>,
    /// Generation of the newest detail request; older ones are dropped after the debounce.
    details_gen: AtomicU64,
}

/// Runs the TUI; builds its own tokio runtime. The terminal is restored on return and on panic.
/// The first frame is drawn before the catalog is read; loading happens in the background.
pub fn run(glyphs: GlyphSet) -> anyhow::Result<()> {
    let start = Instant::now();
    let data = DataDir::from_env()?;
    data.ensure()?;
    let cfg = Config::load(&data.config_path())?;
    let theme = Theme::new(
        cfg.theme != "light",
        detect_depth(
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("NO_COLOR").ok().as_deref(),
        ),
    );
    let mut app = App::new(Vec::new(), glyphs);
    app.status = Some("carregando catálogo…".into());
    // `ratatui::init` also installs a panic hook that restores the terminal before the message.
    let mut terminal = ratatui::init();
    app.size = terminal
        .size()
        .map(|s| (s.width, s.height))
        .unwrap_or(app.size);
    let drawn = terminal.draw(|f| view(&app, &theme, f)).map(|_| ());
    tracing::info!(
        first_frame_ms = start.elapsed().as_millis() as u64,
        "first frame"
    );
    // A worker panic is re-raised by the loop; restore the terminal on that path too.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match (drawn, tokio::runtime::Runtime::new()) {
            (Ok(()), Ok(rt)) => rt.block_on(event_loop(&mut terminal, app, &theme, data, cfg)),
            (Err(e), _) => Err(e.into()),
            (_, Err(e)) => Err(e.into()),
        }
    }));
    ratatui::restore();
    result.unwrap_or_else(|p| std::panic::resume_unwind(p))
}

fn index_age(data: &DataDir) -> Option<i64> {
    let catalog = Catalog::open(&data.catalog_path()).ok()?;
    let newest = catalog.sources().ok()?.iter().map(|s| s.fetched_at).max()?;
    Some((now_unix() - newest).max(0))
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    mut app: App,
    theme: &Theme,
    data: DataDir,
    cfg: Config,
) -> anyhow::Result<()> {
    let (tx, mut rx) = unbounded_channel::<Msg>();
    let ctx = Arc::new(Ctx {
        data,
        cfg,
        http: Http::new()?,
        tx,
        details_gen: AtomicU64::new(0),
    });
    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(TICK);
    spawn_boot(&ctx);
    let mut stats = LoopStats::new();

    loop {
        let msg = tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(Event::Key(k))) => Msg::Key(k),
                Some(Ok(Event::Resize(w, h))) => Msg::Resize(w, h),
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()),
            },
            Some(m) = rx.recv() => m,
            _ = ticker.tick(), if app.refreshing || details_loading(&app) => Msg::Tick,
        };
        let msg = jobs::propagate(msg);
        stats.count(&msg);
        let is_tick = matches!(msg, Msg::Tick);
        let refresh_msg = matches!(msg, Msg::RefreshFinished(_));
        let cmds = update(&mut app, msg);
        for cmd in cmds {
            match cmd {
                Cmd::Quit => return Ok(()),
                Cmd::Refresh { force } => spawn_refresh(&ctx, force),
                Cmd::ScanInstalled => spawn_scan(&ctx),
                Cmd::LoadDetails { source, id } => {
                    spawn_details(&ctx, source, id);
                }
                Cmd::PlanInstall { source_name, id } => {
                    spawn_plan(&ctx, move |o| jobs::plan_install(o, &source_name, &id));
                }
                Cmd::PlanRemove { app } => spawn_plan(&ctx, move |o| jobs::plan_remove(o, &app)),
                Cmd::PlanUpgradeAll => spawn_plan(&ctx, jobs::plan_upgrade_all),
                Cmd::Execute(action) => spawn_execute(&ctx, action),
                Cmd::CleanResidues { app, residues } => spawn_clean(&ctx, app, residues),
            }
        }
        if refresh_msg {
            // New data on disk: reload the catalog into the search index.
            spawn_reload(&ctx);
            if !app.offline
                && let Some(age) = index_age(&ctx.data)
            {
                app.index_age_secs = Some(age);
            }
        }
        // Ticks only matter while something animates.
        if !is_tick || app.refreshing || details_loading(&app) {
            stats.draw(1, || terminal.draw(|f| view(&app, theme, f)).map(|_| ()))?;
        }
    }
}

/// True while the selected package's details have been requested but have not arrived.
fn details_loading(app: &App) -> bool {
    app.selected_key()
        .is_some_and(|(s, id)| !app.details.contains_key(&format!("{s}/{id}")))
}

/// Loads the catalog off the UI path, then reports what the startup refresh should do.
fn spawn_boot(ctx: &Arc<Ctx>) {
    spawn_job(ctx, |c, _| boot_msg(c));
}

fn boot_msg(c: &Ctx) -> Msg {
    Msg::Booted {
        catalog: jobs::read_catalog(&c.data.catalog_path()),
        age: index_age(&c.data),
        needs_refresh: startup_needs_refresh(c),
    }
}

fn startup_needs_refresh(ctx: &Ctx) -> bool {
    let Ok(ep) = Endpoints::production(&ctx.cfg) else {
        return false;
    };
    let urls: Vec<&str> = std::iter::once(ep.winget_index.as_str())
        .chain(ep.scoop.iter().map(|(_, u)| u.as_str()))
        .collect();
    let state = HttpState::load(&ctx.data.http_state_path());
    needs_refresh(&state, &urls, now_unix())
}

fn spawn_refresh(ctx: &Arc<Ctx>, force: bool) {
    let ctx = Arc::clone(ctx);
    let _ = ctx.tx.send(Msg::RefreshStarted);
    tokio::spawn(async move {
        let failed = |message: String| {
            vec![crate::update::SourceReport {
                source: "update".into(),
                outcome: crate::update::Outcome::Failed {
                    message,
                    offline: false,
                },
            }]
        };
        let reports = match Endpoints::production(&ctx.cfg) {
            Ok(ep) => match run_update(&ctx.data, &ep, &ctx.http, force, LockWait::NoWait).await {
                Ok(r) => r,
                Err(UpdateError::Busy) => {
                    let _ = ctx.tx.send(Msg::RefreshBusy);
                    return;
                }
                Err(e) => failed(e.to_string()),
            },
            Err(e) => failed(e.to_string()),
        };
        let _ = ctx.tx.send(Msg::RefreshFinished(reports));
    });
}

fn spawn_reload(ctx: &Arc<Ctx>) {
    spawn_job(ctx, |c, _| {
        match jobs::read_catalog(&c.data.catalog_path()) {
            Ok(e) => Msg::CatalogLoaded(e),
            Err(e) => Msg::CatalogFailed(e),
        }
    });
}

fn spawn_scan(ctx: &Arc<Ctx>) {
    spawn_job(ctx, |c, _| {
        // Tracking loads while the collectors run; joined on every path, even if the scan panics.
        let tracking_job = std::thread::spawn(crate::installed::winget_tracking::load_default);
        let scan = std::panic::catch_unwind(scan_all);
        let tracking = tracking_job.join();
        let scan = scan.unwrap_or_else(|p| std::panic::resume_unwind(p));
        let tracking = tracking.unwrap_or_else(|p| std::panic::resume_unwind(p));
        for e in &scan.errors {
            tracing::warn!("scan: {e}");
        }
        match Catalog::open(&c.data.catalog_path())
            .and_then(|cat| correlate(&scan.apps, &cat, &NameNormalizer::new(), &tracking))
        {
            Ok(rows) => Msg::InstalledLoaded(rows),
            Err(e) => {
                tracing::warn!("correlação falhou: {e}");
                Msg::InstalledFailed(e.to_string())
            }
        }
    });
}

/// Debounced detail load; dropped when a newer request supersedes it.
fn spawn_details(ctx: &Arc<Ctx>, source: String, id: String) {
    let ctx = Arc::clone(ctx);
    let generation = ctx.details_gen.fetch_add(1, Ordering::SeqCst) + 1;
    tokio::spawn(async move {
        tokio::time::sleep(DETAILS_DEBOUNCE).await;
        if ctx.details_gen.load(Ordering::SeqCst) != generation {
            return;
        }
        let key = format!("{source}/{id}");
        let result = load_details(&ctx, &source, &id)
            .await
            .map_err(|e| format!("{e:#}"));
        let _ = ctx.tx.send(Msg::DetailsLoaded { key, result });
    });
}

async fn load_details(
    ctx: &Ctx,
    source: &str,
    id: &str,
) -> anyhow::Result<crate::core::PackageDetails> {
    let (data_path, source, id) = (ctx.data.catalog_path(), source.to_owned(), id.to_owned());
    let pkg = tokio::task::spawn_blocking(move || Catalog::open(&data_path)?.get(&source, &id))
        .await??
        .context("pacote não encontrado no catálogo")?;
    match pkg.entry.source {
        SourceKind::Winget => {
            let hash = pkg
                .winget_hash
                .as_deref()
                .context("pacote winget sem hash de manifesto")?;
            let blobs = BlobStore::new(ctx.data.blobs_dir());
            Ok(crate::sources::winget::details::details(
                &ctx.http,
                &blobs,
                crate::sources::winget::CDN_BASE,
                &pkg.entry.id,
                hash,
                None,
            )
            .await?)
        }
        SourceKind::Scoop => {
            let raw = pkg.raw.as_deref().context("pacote scoop sem manifesto")?;
            Ok(crate::sources::scoop::manifest::details_from_raw(
                &pkg.entry.id,
                raw,
            )?)
        }
    }
}

#[cfg(windows)]
fn with_ctx<R>(
    data: &DataDir,
    http: &Http,
    catalog: &Catalog,
    wait: LockWait,
    progress: crate::install::download::Progress<'_>,
    f: impl FnOnce(&OpsCtx<'_>) -> R,
) -> Result<R, String> {
    use crate::install::prod_win::{ProdEffects, build_ctx};
    let prod = ProdEffects::new().map_err(|e| e.0)?;
    let ctx = build_ctx(&prod, data, http, catalog, wait, progress);
    Ok(f(&ctx))
}

#[cfg(not(windows))]
fn with_ctx<R>(
    _data: &DataDir,
    _http: &Http,
    _catalog: &Catalog,
    _wait: LockWait,
    _progress: crate::install::download::Progress<'_>,
    _f: impl FnOnce(&OpsCtx<'_>) -> R,
) -> Result<R, String> {
    Err("instalar e remover só funciona no Windows".to_string())
}

fn no_progress(_label: &str, _done: u64, _total: Option<u64>) {}

/// Runs `f` with the production `OpsCtx`; the TUI never waits for `install.lock`.
fn with_ops<R>(
    ctx: &Ctx,
    progress: crate::install::download::Progress<'_>,
    f: impl FnOnce(&OpsCtx<'_>) -> R,
) -> Result<R, String> {
    let catalog = Catalog::open(&ctx.data.catalog_path()).map_err(|e| e.to_string())?;
    with_ctx(
        &ctx.data,
        &ctx.http,
        &catalog,
        LockWait::NoWait,
        progress,
        f,
    )
}

/// Runs `work` on a blocking thread (it may block on the async `apply`) and sends the
/// message it returns. The handle is always awaited; a worker panic travels to the main
/// loop as `Msg::Panicked` and is re-raised there.
fn spawn_job(
    ctx: &Arc<Ctx>,
    work: impl FnOnce(&Ctx, &tokio::runtime::Handle) -> Msg + Send + 'static,
) {
    let ctx = Arc::clone(ctx);
    let handle = tokio::runtime::Handle::current();
    tokio::spawn(async move {
        let job = tokio::task::spawn_blocking({
            let ctx = Arc::clone(&ctx);
            move || work(&ctx, &handle)
        });
        let _ = ctx.tx.send(jobs::join_job(job).await);
    });
}

fn spawn_plan(
    ctx: &Arc<Ctx>,
    plan: impl FnOnce(&OpsCtx<'_>) -> Result<Planned, String> + Send + 'static,
) {
    spawn_job(ctx, move |c, _| {
        Msg::Planned(with_ops(c, &no_progress, plan).and_then(|r| r))
    });
}

fn spawn_execute(ctx: &Arc<Ctx>, action: Action) {
    spawn_job(ctx, move |c, handle| {
        let tx = c.tx.clone();
        let throttle = jobs::ProgressThrottle::new(Instant::now());
        let progress = move |label: &str, done: u64, total: Option<u64>| {
            if throttle.admit(Instant::now(), done, total) {
                let _ = tx.send(Msg::InstallProgress {
                    label: label.to_string(),
                    done,
                    total,
                });
            }
        };
        let result = with_ops(c, &progress, |o| {
            handle.block_on(jobs::run_action(o, action))
        })
        .map_err(InstallFail::Other)
        .and_then(|r| r);
        Msg::InstallFinished(result)
    });
}

fn spawn_clean(ctx: &Arc<Ctx>, app: String, residues: Vec<Residue>) {
    spawn_job(ctx, move |c, _| {
        Msg::ResiduesCleaned(
            with_ops(c, &no_progress, |o| jobs::clean(o, &app, &residues))
                .unwrap_or_else(|e| vec![e]),
        )
    });
}
