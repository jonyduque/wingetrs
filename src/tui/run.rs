//! TUI event loop: terminal setup, input stream, command execution.
use crate::cache::{BlobStore, DataDir, HttpState, now_unix};
use crate::catalog::Catalog;
use crate::catalog::search::SearchIndex;
use crate::config::Config;
use crate::core::SourceKind;
use crate::http::Http;
use crate::install::ops::OpsCtx;
use crate::install::residue::Residue;
use crate::installed::correlate::correlate;
use crate::installed::normalize::NameNormalizer;
use crate::installed::scan_all;
use crate::tui::app::{Action, App, Cmd, InstallFail, Msg, Planned, run_search, update};
use crate::tui::jobs;
use crate::tui::settings::SettingsState;
use crate::tui::theme::{ColorDepth, GlyphSet, Theme, detect_depth};
use crate::tui::view::view;
use crate::update::{Endpoints, LockWait, UpdateError, needs_refresh, run_update};
use anyhow::Context;
use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::Poll;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

const TICK: Duration = Duration::from_millis(100);
const DETAILS_DEBOUNCE: Duration = Duration::from_millis(150);
/// Wait before fetching the list's icons; a newer screen meanwhile drops the request.
const LIST_FETCH_DEBOUNCE: Duration = Duration::from_millis(300);
/// Most list packages whose icon is being fetched at once.
pub const MAX_LIST_FETCH: usize = 4;
/// Most messages folded into one frame.
const BATCH_CAP: usize = 256;
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
    /// Generation of the newest list icon prefetch; older ones stop starting packages.
    list_fetch_gen: AtomicU64,
    /// Slots for list packages being fetched, shared by every generation.
    list_fetch_slots: Arc<tokio::sync::Semaphore>,
    /// `<source>/<id>` of the list packages being fetched, by any generation.
    list_fetch_running: Arc<std::sync::Mutex<HashSet<String>>>,
    /// Encoder for package icons; `None` when icons are off.
    icon_picker: std::sync::Mutex<Option<ratatui_image::picker::Picker>>,
}

/// Runs the TUI; builds its own tokio runtime. The terminal is restored on return and on panic.
/// The first frame is drawn before the catalog is read; loading happens in the background.
pub fn run(glyphs: GlyphSet, glyphs_forced: bool) -> anyhow::Result<()> {
    let start = Instant::now();
    let data = DataDir::from_env()?;
    data.ensure()?;
    let cfg = Config::load(&data.config_path())?;
    let depth = detect_depth(
        std::env::var("COLORTERM").ok().as_deref(),
        std::env::var("NO_COLOR").ok().as_deref(),
    );
    let dark = cfg.theme != "light";
    let theme = Theme::new(dark, depth);
    let mut app = App::new(Vec::new(), glyphs);
    app.dark = dark;
    app.settings = SettingsState::new(cfg.behavior, glyphs, dark, glyphs_forced);
    app.settings.icons.clone_from(&cfg.icons);
    app.settings.saved_icons.clone_from(&cfg.icons);
    app.settings.list_icons.clone_from(&cfg.list_icons);
    app.list_icons_visible = cfg.list_icons == crate::config::LIST_ICONS[0];
    app.buckets_summary = cfg
        .scoop_buckets
        .iter()
        .chain(cfg.buckets.iter().map(|b| &b.name))
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    app.icon_protocol = crate::tui::icons::choose_from_env(&cfg.icons);
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
            (Ok(()), Ok(rt)) => {
                rt.block_on(event_loop(&mut terminal, app, theme, depth, data, cfg))
            }
            (Err(e), _) => Err(e.into()),
            (_, Err(e)) => Err(e.into()),
        }
    }));
    ratatui::restore();
    result.unwrap_or_else(|p| std::panic::resume_unwind(p))
}

fn index_age(data: &DataDir) -> Option<i64> {
    let catalog = jobs::open_catalog_for_reading(&data.catalog_path()).ok()?;
    let newest = catalog.sources().ok()?.iter().map(|s| s.fetched_at).max()?;
    Some((now_unix() - newest).max(0))
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    mut app: App,
    mut theme: Theme,
    depth: ColorDepth,
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
        list_fetch_gen: AtomicU64::new(0),
        list_fetch_slots: Arc::new(tokio::sync::Semaphore::new(MAX_LIST_FETCH)),
        list_fetch_running: Arc::default(),
        // After `ratatui::init`, so the pixel size is that of the terminal in use.
        icon_picker: std::sync::Mutex::new(crate::tui::icons::picker(app.icon_protocol)),
    });
    ctx.http
        .set_allow_http_redirect(ctx.cfg.behavior.allow_http_redirect);
    let mut events = EventStream::new();
    let mut ticker = spinner_ticker();
    spawn_boot(&ctx);
    if app.icon_protocol != crate::tui::icons::IconProtocol::Off {
        spawn_job(&ctx, |c, _| {
            Msg::IconMapLoaded(crate::icons::load_map(&c.data))
        });
    }
    let mut stats = LoopStats::new();
    let mut dark_now = app.dark;
    let mut protocol_now = app.icon_protocol;

    loop {
        let wake = app.next_toast_deadline();
        let first = tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(ev)) => match input_msg(ev) {
                    Some(m) => m,
                    None => continue,
                },
                Some(Err(e)) => return Err(e.into()),
                None => return Ok(()),
            },
            Some(m) = rx.recv() => m,
            _ = ticker.tick(), if app.animating() => Msg::Tick,
            // One deadline, recomputed every turn: nothing to pile up while asleep.
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(wake.unwrap_or(app.now))),
                if wake.is_some() => Msg::Clock,
        };
        // Take every key and job message already queued, so one frame covers them all.
        let mut input_err = None;
        let batch = {
            let keys = (&mut events).filter_map(|ev| {
                futures_util::future::ready(match ev {
                    Ok(ev) => input_msg(ev),
                    Err(e) => {
                        input_err.get_or_insert(e);
                        None
                    }
                })
            });
            let jobs = futures_util::stream::poll_fn(|cx| rx.poll_recv(cx));
            let mut queued = futures_util::stream::select(keys, jobs);
            drain_ready(first, &mut queued, BATCH_CAP).await
        };
        if let Some(e) = input_err {
            return Err(e.into());
        }
        let draw = needs_draw(&batch, app.animating());
        let batch_len = batch.len();
        let mut refresh_msg = false;
        let batch: Vec<Msg> = batch
            .into_iter()
            .map(jobs::propagate)
            .inspect(|msg| {
                stats.count(msg);
                refresh_msg |= matches!(msg, Msg::RefreshFinished(_));
            })
            .collect();
        let cmds = clock_turn(&mut app, Instant::now(), batch);
        if app.icon_protocol != protocol_now {
            // The Ícones setting changed: encode with the new protocol from here on.
            protocol_now = app.icon_protocol;
            let new_picker = crate::tui::icons::picker(protocol_now);
            match ctx.icon_picker.lock() {
                Ok(mut g) => *g = new_picker,
                Err(poisoned) => *poisoned.into_inner() = new_picker,
            }
            if protocol_now != crate::tui::icons::IconProtocol::Off && app.icon_map.is_empty() {
                spawn_job(&ctx, |c, _| {
                    Msg::IconMapLoaded(crate::icons::load_map(&c.data))
                });
            }
        }
        for cmd in last_details_only(cmds) {
            match cmd {
                Cmd::Quit => return Ok(()),
                Cmd::Refresh { force } => spawn_refresh(&ctx, force),
                Cmd::ScanInstalled => spawn_scan(&ctx),
                Cmd::LoadDetails { source, id } => {
                    spawn_details(&ctx, source, id);
                }
                Cmd::PlanInstall {
                    source_name,
                    id,
                    keep,
                } => {
                    spawn_plan(&ctx, move |o| {
                        jobs::plan_install(o, &source_name, &id, keep)
                    });
                }
                Cmd::PlanRemove { app, purge } => {
                    spawn_plan(&ctx, move |o| jobs::plan_remove(o, &app, purge));
                }
                Cmd::PlanUpgradeAll { keep } => {
                    spawn_plan(&ctx, move |o| jobs::plan_upgrade_all(o, keep));
                }
                Cmd::SaveSettings {
                    behavior,
                    glyphs,
                    icons,
                    list_icons,
                    theme,
                } => {
                    ctx.http
                        .set_allow_http_redirect(behavior.allow_http_redirect);
                    spawn_job(&ctx, move |c, _| {
                        Msg::SettingsSaved(jobs::save_settings(
                            &c.data.config_path(),
                            behavior,
                            glyphs.as_deref(),
                            icons.as_deref(),
                            list_icons.as_deref(),
                            &theme,
                        ))
                    });
                }
                Cmd::EncodeIcon {
                    generation,
                    key,
                    bytes,
                    size,
                } => spawn_icon(&ctx, generation, key, bytes, size),
                Cmd::EncodeCachedIcon {
                    generation,
                    source,
                    id,
                    sha,
                    size,
                } => spawn_cached_icon(&ctx, generation, source, id, sha, size),
                Cmd::PrefetchIcons { generation, keys } => spawn_prefetch(&ctx, generation, keys),
                Cmd::Execute(action) => spawn_execute(&ctx, action),
                Cmd::CleanResidues { app, residues } => spawn_clean(&ctx, app, residues),
                Cmd::Search {
                    generation,
                    query,
                    kind,
                } => spawn_search(&ctx, Arc::clone(&app.search), generation, query, kind),
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
        if draw {
            if app.dark != dark_now {
                dark_now = app.dark;
                theme = Theme::new(dark_now, depth);
            }
            stats.draw(batch_len, || {
                terminal.draw(|f| view(&app, &theme, f)).map(|_| ())
            })?;
        }
    }
}

/// One loop turn: sets the toast clock to `now`, then applies the batch in order.
pub fn clock_turn(app: &mut App, now: Instant, msgs: Vec<Msg>) -> Vec<Cmd> {
    app.now = now;
    let mut cmds = Vec::new();
    for msg in msgs {
        cmds.extend(update(app, msg));
    }
    cmds
}

/// Terminal events the app reacts to; the rest are dropped.
fn input_msg(ev: Event) -> Option<Msg> {
    match ev {
        Event::Key(k) => Some(Msg::Key(k)),
        Event::Resize(w, h) => Some(Msg::Resize(w, h)),
        _ => None,
    }
}

/// Spinner clock; ticks missed while nothing animated are skipped, never fired in a burst.
pub fn spinner_ticker() -> tokio::time::Interval {
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticker
}

/// `first` followed by whatever `more` already has ready, without waiting; at most `cap` items.
/// `more` is polled with the calling task's waker: crossterm's `EventStream` keeps the first
/// waker it sees while pending, so a throwaway waker here would leave the loop asleep.
pub async fn drain_ready<S: futures_util::Stream<Item = Msg> + Unpin>(
    first: Msg,
    more: &mut S,
    cap: usize,
) -> Vec<Msg> {
    let mut batch = vec![first];
    while batch.len() < cap {
        match std::future::poll_fn(|cx| Poll::Ready(more.poll_next_unpin(cx))).await {
            Poll::Ready(Some(m)) => batch.push(m),
            // Nothing ready, or the stream ended.
            Poll::Ready(None) | Poll::Pending => break,
        }
    }
    batch
}

/// Drops every `Cmd::LoadDetails` but the last; other commands keep their order.
pub fn last_details_only(cmds: Vec<Cmd>) -> Vec<Cmd> {
    let last = cmds
        .iter()
        .rposition(|c| matches!(c, Cmd::LoadDetails { .. }));
    cmds.into_iter()
        .enumerate()
        .filter(|(i, c)| !matches!(c, Cmd::LoadDetails { .. }) || Some(*i) == last)
        .map(|(_, c)| c)
        .collect()
}

/// Whether a batch changes the screen: anything but a tick does (`Msg::Clock` included),
/// a tick only while animating.
pub fn needs_draw(batch: &[Msg], animating: bool) -> bool {
    batch.iter().any(|m| !matches!(m, Msg::Tick) || animating)
}

/// Awaits a detail worker: a panic travels to the loop as `Msg::Panicked`, a cancellation
/// becomes a failed load for `key`.
pub async fn join_details(key: String, h: tokio::task::JoinHandle<Msg>) -> Msg {
    match h.await {
        Ok(msg) => msg,
        Err(e) if e.is_panic() => Msg::Panicked(e.into_panic()),
        Err(e) => Msg::DetailsLoaded {
            key,
            result: Err(format!("tarefa cancelada: {e}")),
        },
    }
}

/// Awaits a search worker: a panic travels to the loop as `Msg::Panicked`; a cancelled search
/// becomes `Msg::SearchCancelled`, which frees the in-flight slot for the next search.
pub async fn join_search(generation: u64, h: tokio::task::JoinHandle<Msg>) -> Msg {
    match h.await {
        Ok(msg) => msg,
        Err(e) if e.is_panic() => Msg::Panicked(e.into_panic()),
        Err(_) => Msg::SearchCancelled { generation },
    }
}

/// Awaits an icon encoder: a panic travels to the loop as `Msg::Panicked`, a cancellation
/// becomes a failed encoding for `key`.
pub async fn join_icon(
    generation: u64,
    key: String,
    size: (u16, u16),
    h: tokio::task::JoinHandle<Msg>,
) -> Msg {
    match h.await {
        Ok(msg) => msg,
        Err(e) if e.is_panic() => Msg::Panicked(e.into_panic()),
        Err(e) => Msg::IconReady {
            generation,
            key,
            size,
            proto: Err(format!("tarefa cancelada: {e}")),
        },
    }
}

/// The encoder for the protocol in force (changes when the Ícones setting does).
fn current_picker(ctx: &Ctx) -> Option<ratatui_image::picker::Picker> {
    match ctx.icon_picker.lock() {
        Ok(g) => g.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Decodes and encodes one icon on a blocking thread; the handle is always awaited.
fn spawn_icon(ctx: &Arc<Ctx>, generation: u64, key: String, bytes: Vec<u8>, size: (u16, u16)) {
    let picker = current_picker(ctx);
    let tx = ctx.tx.clone();
    tokio::spawn(async move {
        let work = tokio::task::spawn_blocking({
            let key = key.clone();
            move || {
                let proto = match &picker {
                    Some(p) => crate::tui::icons::encode(p, &bytes, size),
                    None => Err("ícones desligados".into()),
                };
                if let Err(e) = &proto {
                    tracing::debug!(%key, error = %e, "icon not decoded");
                }
                Msg::IconReady {
                    generation,
                    key,
                    size,
                    proto,
                }
            }
        });
        let _ = tx.send(join_icon(generation, key, size, work).await);
    });
}

/// Reads a cached icon from the blob store and encodes it on a blocking thread (no network);
/// the handle is always awaited.
fn spawn_cached_icon(
    ctx: &Arc<Ctx>,
    generation: u64,
    source: String,
    id: String,
    sha: String,
    size: (u16, u16),
) {
    let picker = current_picker(ctx);
    let ctx = Arc::clone(ctx);
    let key = format!("{source}/{id}");
    tokio::spawn(async move {
        let work = tokio::task::spawn_blocking({
            let (ctx, key) = (Arc::clone(&ctx), key.clone());
            move || {
                let map = std::collections::BTreeMap::from([(key.clone(), sha)]);
                let proto = match (
                    &picker,
                    crate::icons::cached_icon(&ctx.data, &map, &source, &id),
                ) {
                    (None, _) => Err("ícones desligados".to_string()),
                    (_, None) => Err("ícone fora do cache".to_string()),
                    (Some(p), Some(bytes)) => crate::tui::icons::encode(p, &bytes, size),
                };
                if let Err(e) = &proto {
                    tracing::debug!(%key, error = %e, "cached icon not encoded");
                }
                Msg::IconReady {
                    generation,
                    key,
                    size,
                    proto,
                }
            }
        });
        let _ = ctx.tx.send(join_icon(generation, key, size, work).await);
    });
}

/// Runs one keystroke's search on a blocking thread; the handle is always awaited.
fn spawn_search(
    ctx: &Arc<Ctx>,
    index: Arc<SearchIndex>,
    generation: u64,
    query: String,
    kind: Option<SourceKind>,
) {
    let tx = ctx.tx.clone();
    tokio::spawn(async move {
        let work = tokio::task::spawn_blocking(move || Msg::SearchDone {
            generation,
            hits: run_search(&index, &query, kind),
        });
        let _ = tx.send(join_search(generation, work).await);
    });
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
    let tx = ctx.tx.clone();
    let _ = ctx.tx.send(Msg::RefreshStarted);
    let work = tokio::spawn(async move {
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
                Err(UpdateError::Busy) => return Msg::RefreshBusy,
                Err(e) => failed(e.to_string()),
            },
            Err(e) => failed(e.to_string()),
        };
        Msg::RefreshFinished(reports)
    });
    tokio::spawn(async move {
        let _ = tx.send(join_refresh(work).await);
    });
}

/// Awaits the refresh task: a panic travels to the loop as `Msg::Panicked`, a cancellation
/// finishes the refresh as failed, so `refreshing` never stays on.
pub async fn join_refresh(h: tokio::task::JoinHandle<Msg>) -> Msg {
    match h.await {
        Ok(msg) => msg,
        Err(e) if e.is_panic() => Msg::Panicked(e.into_panic()),
        Err(e) => Msg::RefreshFinished(vec![crate::update::SourceReport {
            source: "update".into(),
            outcome: crate::update::Outcome::Failed {
                message: format!("tarefa cancelada: {e}"),
                offline: false,
            },
        }]),
    }
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
        match jobs::open_catalog_for_reading(&c.data.catalog_path())
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
        let work = tokio::spawn({
            let (ctx, key) = (Arc::clone(&ctx), key.clone());
            let (source, id) = (source.clone(), id.clone());
            async move {
                let result = load_details(&ctx, &source, &id)
                    .await
                    .map_err(|e| format!("{e:#}"));
                Msg::DetailsLoaded { key, result }
            }
        });
        let msg = join_details(key.clone(), work).await;
        let icon = match &msg {
            Msg::DetailsLoaded { result: Ok(d), .. } => d.icon.clone(),
            _ => None,
        };
        let icon_sha = icon.as_ref().map(|i| i.sha256.clone()).unwrap_or_default();
        // Details first; the icon is fetched afterwards so a slow host never delays them.
        let _ = ctx.tx.send(msg);
        if let Some(bytes) = load_icon(&ctx, &source, &id, icon.as_ref()).await {
            let _ = ctx.tx.send(Msg::IconMapped {
                key: key.clone(),
                sha: icon_sha,
            });
            let _ = ctx.tx.send(Msg::IconLoaded { key, bytes });
        }
    });
}

/// Debounced fetch of the visible list rows' icons through the details path; dropped when
/// a newer screen supersedes it. Sends `Msg::IconMapped` and `Msg::PrefetchDone`, never the
/// details.
fn spawn_prefetch(ctx: &Arc<Ctx>, generation: u64, keys: Vec<(String, String)>) {
    let ctx = Arc::clone(ctx);
    tokio::spawn(async move {
        let work = tokio::spawn({
            let ctx = Arc::clone(&ctx);
            async move {
                let fetch = |source: String, id: String| {
                    let ctx = Arc::clone(&ctx);
                    async move {
                        prefetch_one(&ctx, &source, &id).await;
                        let key = format!("{source}/{id}");
                        let _ = ctx.tx.send(Msg::PrefetchDone { key });
                    }
                };
                prefetch_list(
                    &ctx.list_fetch_gen,
                    generation,
                    LIST_FETCH_DEBOUNCE,
                    &ctx.list_fetch_slots,
                    &ctx.list_fetch_running,
                    keys,
                    fetch,
                )
                .await;
            }
        });
        match work.await {
            Ok(()) => {}
            Err(e) if e.is_panic() => {
                let _ = ctx.tx.send(Msg::Panicked(e.into_panic()));
            }
            Err(e) => tracing::debug!(error = %e, "list icon prefetch cancelled"),
        }
    });
}

/// Details, then icon, of one list package; announces the icon when it was obtained.
async fn prefetch_one(ctx: &Ctx, source: &str, id: &str) {
    let icon = match load_details(ctx, source, id).await {
        Ok(d) => d.icon,
        Err(e) => {
            tracing::debug!(%source, %id, error = %e, "list details not loaded");
            return;
        }
    };
    if load_icon(ctx, source, id, icon.as_ref()).await.is_some()
        && let Some(icon) = icon
    {
        let _ = ctx.tx.send(Msg::IconMapped {
            key: format!("{source}/{id}"),
            sha: icon.sha256,
        });
    }
}

/// True while `mine` is still the newest list prefetch.
pub fn still_current(generation: &AtomicU64, mine: u64) -> bool {
    generation.load(Ordering::SeqCst) == mine
}

/// Marks `mine` as the newest prefetch (unless a newer one already ran), waits `debounce`,
/// then runs `fetch` for each key in order, each on its own task holding one of `slots`.
/// The generation is checked before each package starts; once superseded, the rest is
/// skipped. A key in `running` (an older generation is fetching it and will report it) is
/// skipped too. Every task is awaited, and a panic in one is re-raised here after the
/// others end.
pub async fn prefetch_list<F, Fut>(
    generation: &AtomicU64,
    mine: u64,
    debounce: Duration,
    slots: &Arc<tokio::sync::Semaphore>,
    running: &Arc<std::sync::Mutex<HashSet<String>>>,
    keys: Vec<(String, String)>,
    fetch: F,
) where
    F: Fn(String, String) -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    // Tasks may run out of order: an older generation never takes the place of a newer one.
    generation.fetch_max(mine, Ordering::SeqCst);
    tokio::time::sleep(debounce).await;
    let mut tasks = tokio::task::JoinSet::new();
    for (source, id) in keys {
        let Ok(slot) = Arc::clone(slots).acquire_owned().await else {
            break;
        };
        if !still_current(generation, mine) {
            break;
        }
        let key = format!("{source}/{id}");
        if !lock_running(running).insert(key.clone()) {
            continue;
        }
        let mark = RunningKey {
            running: Arc::clone(running),
            key,
        };
        let job = fetch(source, id);
        tasks.spawn(async move {
            job.await;
            drop((mark, slot));
        });
    }
    let mut panic = None;
    while let Some(done) = tasks.join_next().await {
        match done {
            Ok(()) => {}
            Err(e) if e.is_panic() => {
                panic.get_or_insert(e.into_panic());
            }
            Err(e) => tracing::debug!(error = %e, "list icon task cancelled"),
        }
    }
    if let Some(p) = panic {
        std::panic::resume_unwind(p);
    }
}

fn lock_running(
    running: &std::sync::Mutex<HashSet<String>>,
) -> std::sync::MutexGuard<'_, HashSet<String>> {
    // Poisoned only by a panic while the lock is held, and no code that can panic runs under
    // it (insert/remove of a String): the set is still consistent.
    running.lock().unwrap_or_else(|p| p.into_inner())
}

/// Takes `key` off the running list packages when its task ends, also on panic.
struct RunningKey {
    running: Arc<std::sync::Mutex<HashSet<String>>>,
    key: String,
}

impl Drop for RunningKey {
    fn drop(&mut self) {
        lock_running(&self.running).remove(&self.key);
    }
}

/// Icon bytes for the details; any failure is only logged, never an error of the details.
async fn load_icon(
    ctx: &Ctx,
    source: &str,
    id: &str,
    icon: Option<&crate::core::IconRef>,
) -> Option<Vec<u8>> {
    let icon = icon?;
    let blobs = BlobStore::new(ctx.data.blobs_dir());
    match crate::icons::fetch(&ctx.http, &blobs, icon).await {
        Ok(bytes) => {
            crate::icons::remember(&ctx.data, source, id, &icon.sha256);
            Some(bytes)
        }
        Err(e) => {
            tracing::debug!(%source, %id, error = %e, "icon not loaded");
            None
        }
    }
}

async fn load_details(
    ctx: &Ctx,
    source: &str,
    id: &str,
) -> anyhow::Result<crate::core::PackageDetails> {
    let (data_path, source, id) = (ctx.data.catalog_path(), source.to_owned(), id.to_owned());
    let read = tokio::task::spawn_blocking(move || {
        jobs::open_catalog_for_reading(&data_path)?.get(&source, &id)
    });
    let pkg = match read.await {
        Ok(r) => r?,
        // A panic in the read re-panics this task, so `join_details` reports it.
        Err(e) => match e.try_into_panic() {
            Ok(p) => std::panic::resume_unwind(p),
            Err(e) => return Err(e.into()),
        },
    }
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
    let catalog =
        jobs::open_catalog_for_reading(&ctx.data.catalog_path()).map_err(|e| e.to_string())?;
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
            if let Some((label, done, total)) = throttle.admit(Instant::now(), label, done, total) {
                let _ = tx.send(Msg::InstallProgress { label, done, total });
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
