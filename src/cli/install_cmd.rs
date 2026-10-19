//! `install`, `uninstall`, `upgrade` and `repair`: plan, show, confirm, execute.
//! Platform-neutral: every real effect arrives through `OpsCtx`, every prompt through `Ui`.
use super::picker::{self, StdTerm, Term};
use super::winget_cmd::{self, CleanupArgs, WingetFlows};
use super::{Command, output, style};
use crate::install::executor::LaunchGate;
use crate::install::ops::{self, AllowMsiexec, OpsCtx, Resolution};
use crate::install::progress::ProgressEvent;
use crate::install::residue::{self, Confidence, Residue};
use crate::install::script_scan::CallRoots;
use crate::install::target::Candidate;
use crate::job_log::{JobInfo, JobLog, Outcome};
use crate::tui::picker::PickerItem;
use crate::tui::theme::Glyphs;
use anyhow::bail;
use output::{Phases, ThirdParty};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// Everything the commands ask of the user. Production: [`TtyUi`].
pub trait Ui {
    /// Both stdin and stdout are terminals.
    fn interactive(&self) -> bool;
    fn confirm(&self, question: &str) -> anyhow::Result<bool>;
    /// One of a few options; a shortcut letter picks its row at once.
    fn pick(&self, items: Vec<PickerItem>) -> anyhow::Result<Option<usize>>;
    /// One of the apps that answer to the same name: a letter only moves the cursor and
    /// Enter confirms (the names often share an initial and a slip would act on the wrong one).
    fn pick_target(&self, items: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
        self.pick(items)
    }
    fn pick_many(
        &self,
        items: Vec<PickerItem>,
        initial: &[bool],
    ) -> anyhow::Result<Option<Vec<usize>>>;
    /// Glyph set for the short plans (arrows).
    fn glyphs(&self) -> &'static Glyphs {
        Glyphs::for_set(crate::tui::theme::GlyphSet::Ascii)
    }
    /// Stops and erases the progress line; called before anything else is printed.
    fn end_progress(&self) {}
    /// A line printed once above the progress line (e.g. why Ctrl+C did nothing).
    fn notice(&self, _text: &str) {}
}

/// Said once when Ctrl+C arrives while an uncancellable step runs (`LaunchGate::running`:
/// the Windows Installer, a manifest script, an installer).
pub fn uncancellable(step: Option<&str>) -> String {
    let step = step.unwrap_or("instalador");
    format!("um passo já começou e não pode ser interrompido ({step}); esperando terminar")
}

/// The user stopped the operation with Ctrl+C. `Cancelado.` was already printed and the job
/// log says `cancelado`; `main` only sets the exit code.
#[derive(Debug, thiserror::Error)]
#[error("cancelado pelo usuário")]
pub struct Cancelled;

/// Exit code of a failed command: 130 when the user cancelled with Ctrl+C (even under a
/// `.context()`), 1 otherwise.
pub fn exit_code(e: &anyhow::Error) -> u8 {
    if e.is::<Cancelled>() { 130 } else { 1 }
}

/// Requests to stop the running operation: Ctrl+C in production, a channel in tests.
pub trait Interrupts {
    /// From now until [`Interrupts::release`], a request no longer ends the process: it
    /// waits for [`Interrupts::next`]. No-op by default.
    fn hold(&mut self) {}
    /// Back to the default, where a request ends the process. No-op by default.
    fn release(&mut self) {}
    /// Resolves at the next request; never when none can come any more.
    fn next(&mut self) -> impl std::future::Future<Output = ()>;
}

impl Interrupts for tokio::sync::mpsc::UnboundedReceiver<()> {
    async fn next(&mut self) {
        if self.recv().await.is_none() {
            std::future::pending::<()>().await;
        }
    }
}

/// Runs `op` listening for interrupts. A request that wins `gate` (no `msiexec` launched in
/// the current plan) drops `op` (the executor waits for its worker and removes the staging
/// folder) and gives `None`; once `msiexec` launched, `refused` runs (once) and `op` goes on
/// to its normal end. Interrupts are held from here on and **not** released: the caller
/// releases them once the job log is written, so a second Ctrl+C never cuts that short.
pub async fn until_interrupted<T>(
    op: impl std::future::Future<Output = T>,
    interrupts: &mut impl Interrupts,
    gate: &LaunchGate,
    refused: &dyn Fn(),
) -> Option<T> {
    interrupts.hold();
    tokio::pin!(op);
    let mut told = false;
    loop {
        tokio::select! {
            biased;
            out = &mut op => return Some(out),
            () = interrupts.next() => {
                // Returning drops `op`, here, while interrupts are still held.
                if gate.try_cancel() {
                    return None;
                }
                if !told {
                    told = true;
                    refused();
                }
            }
        }
    }
}

/// Ends a job stopped by Ctrl+C.
fn cancelled(log: JobLog) -> anyhow::Result<()> {
    log.finish(Outcome::Cancelled, "Ctrl+C");
    anstream::println!("{}", style::paint(style::WARN, "Cancelado."));
    Err(Cancelled.into())
}

pub struct TtyUi {
    theme: crate::tui::theme::Theme,
    glyphs: &'static Glyphs,
    progress: Arc<LineProgress>,
    /// Where every question is drawn and answered (the process's terminal in production).
    term: Mutex<Box<dyn Term + Send>>,
}

impl TtyUi {
    pub fn new(cfg: &crate::config::Config) -> TtyUi {
        use std::io::IsTerminal;
        TtyUi::build(
            cfg,
            Box::new(StdTerm::new()),
            std::io::stderr().is_terminal(),
        )
    }

    /// Questions go to `term` (tests inject keys there); the progress line stays plain.
    pub fn with_term(cfg: &crate::config::Config, term: Box<dyn Term + Send>) -> TtyUi {
        TtyUi::build(cfg, term, false)
    }

    fn build(cfg: &crate::config::Config, term: Box<dyn Term + Send>, tty: bool) -> TtyUi {
        use crate::tui::theme::{GlyphSet, Theme, detect_depth};
        let set = match cfg.glyphs.as_str() {
            "unicode" => GlyphSet::Unicode,
            "ascii" => GlyphSet::Ascii,
            _ => GlyphSet::Nerd,
        };
        let glyphs = Glyphs::for_set(set);
        TtyUi {
            progress: Arc::new(LineProgress::new(tty, glyphs)),
            term: Mutex::new(term),
            theme: Theme::new(
                cfg.theme != "light",
                detect_depth(
                    std::env::var("COLORTERM").ok().as_deref(),
                    std::env::var("NO_COLOR").ok().as_deref(),
                ),
            ),
            glyphs,
        }
    }

    /// The progress line this UI stops through [`Ui::end_progress`]; feed it to `OpsCtx.progress`.
    pub fn progress(&self) -> Arc<LineProgress> {
        Arc::clone(&self.progress)
    }
}

impl TtyUi {
    fn select_one(
        &self,
        items: Vec<PickerItem>,
        letters: picker::Letters,
    ) -> anyhow::Result<Option<usize>> {
        let mut term = self.term()?;
        Ok(picker::select_one(
            &mut **term,
            None,
            items,
            None,
            letters,
            &self.theme,
            self.glyphs,
        )?)
    }

    fn term(&self) -> anyhow::Result<std::sync::MutexGuard<'_, Box<dyn Term + Send>>> {
        self.term
            .lock()
            .map_err(|_| anyhow::anyhow!("o terminal ficou indisponível"))
    }
}

impl Ui for TtyUi {
    fn glyphs(&self) -> &'static Glyphs {
        self.glyphs
    }

    fn end_progress(&self) {
        self.progress.finish();
    }

    fn notice(&self, text: &str) {
        self.progress.notice(text);
    }

    fn interactive(&self) -> bool {
        match self.term.lock() {
            Ok(t) => t.interactive(),
            Err(_) => false,
        }
    }

    fn confirm(&self, question: &str) -> anyhow::Result<bool> {
        let mut term = self.term()?;
        Ok(picker::yes_no(
            &mut **term,
            question,
            &self.theme,
            self.glyphs,
        )?)
    }

    fn pick(&self, items: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
        self.select_one(items, picker::Letters::Pick)
    }

    fn pick_target(&self, items: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
        self.select_one(items, picker::Letters::Move)
    }

    fn pick_many(
        &self,
        items: Vec<PickerItem>,
        initial: &[bool],
    ) -> anyhow::Result<Option<Vec<usize>>> {
        let mut term = self.term()?;
        Ok(picker::select_many(
            &mut **term,
            None,
            items,
            initial,
            &self.theme,
            self.glyphs,
        )?)
    }
}

const TICK: Duration = Duration::from_millis(120);

/// How often the animation looks at the end of the Windows Installer log.
const LOG_POLL: Duration = Duration::from_secs(1);

struct ProgressState {
    glyphs: &'static Glyphs,
    phases: Mutex<Phases>,
    frame: AtomicUsize,
    /// Chars of the line on screen (0 = none): the next draw pads to it, the erase clears it.
    drawn: AtomicUsize,
    /// Set by `finish`: nothing is drawn afterwards.
    closed: AtomicBool,
    /// A line to print once above the progress line ([`LineProgress::notice`]).
    notice: Mutex<Option<String>>,
}

/// `main` then `warn` cut to `cols` chars (the warning goes first).
fn fit_cols(main: String, warn: String, cols: usize) -> (String, String) {
    let n = main.chars().count();
    if n >= cols {
        return (main.chars().take(cols).collect(), String::new());
    }
    (main, warn.chars().take(cols - n).collect())
}

impl ProgressState {
    fn phases(&self) -> std::sync::MutexGuard<'_, Phases> {
        self.phases.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Draws the current phase line, if any, on stderr (with the one-time notice above it).
    fn draw(&self) {
        let (parts, notice) = {
            let mut phases = self.phases();
            let now = std::time::Instant::now();
            let extra = self.notice.lock().unwrap_or_else(|p| p.into_inner()).take();
            let notice = extra.or_else(|| phases.take_notice_at(now).map(str::to_string));
            (
                phases.parts_at(self.frame.load(Ordering::Relaxed), self.glyphs, now),
                notice,
            )
        };
        // One column less than the terminal: a full line would wrap and break the `\r`.
        let cols = crossterm::terminal::size()
            .map(|(c, _)| usize::from(c))
            .unwrap_or(output::LINE_COLUMNS + 1)
            .saturating_sub(1)
            .max(20);
        let mut err = std::io::stderr().lock();
        // `finish` sets `closed` before erasing under this same lock, so no late redraw.
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        // Progress is cosmetic: a broken stderr must not fail the install.
        if let Some(notice) = notice {
            let width = self.drawn.swap(0, Ordering::Relaxed);
            let _ = writeln!(err, "\r{:<width$}\r{notice}", "");
        }
        let Some((main, warn)) = parts else {
            let _ = err.flush();
            return;
        };
        let (main, warn) = fit_cols(main, warn, cols);
        let len = main.chars().count() + warn.chars().count();
        let pad = " ".repeat(self.drawn.load(Ordering::Relaxed).saturating_sub(len));
        let _ = write!(err, "\r{main}{}{pad}", style::paint(style::WARN, &warn));
        let _ = err.flush();
        self.drawn.store(len, Ordering::Relaxed);
    }

    /// Looks at the end of the Windows Installer log, when that is the phase.
    fn poll_log(&self) {
        let Some((log, since)) = self.phases().log_to_read() else {
            return;
        };
        // Read-only; std opens with read/write/delete sharing, so msiexec keeps writing.
        let tail = std::fs::File::open(&log).and_then(|mut f| {
            crate::install::msiexec_log::read_tail(
                &mut f,
                crate::install::msiexec_log::TAIL_WINDOW,
                since,
            )
        });
        if let Err(e) = &tail {
            tracing::debug!("{}: {e}", log.display());
        }
        self.phases()
            .log_read_at(&log, tail.ok(), std::time::Instant::now());
    }
}

/// The animation thread: ticks the spinner and the phase clock until its sender drops.
struct Animation {
    stop: mpsc::Sender<()>,
    handle: std::thread::JoinHandle<()>,
}

/// Progress on stderr. On a terminal: one animated line (phase and clock, download bar,
/// Windows Installer) redrawn by a thread. Otherwise: one plain line per phase change.
pub struct LineProgress {
    tty: bool,
    state: Arc<ProgressState>,
    plain: Mutex<output::PhaseLog>,
    animation: Mutex<Option<Animation>>,
}

impl LineProgress {
    pub fn new(tty: bool, glyphs: &'static Glyphs) -> LineProgress {
        LineProgress {
            tty,
            state: Arc::new(ProgressState {
                glyphs,
                phases: Mutex::new(Phases::new()),
                frame: AtomicUsize::new(0),
                drawn: AtomicUsize::new(0),
                closed: AtomicBool::new(false),
                notice: Mutex::new(None),
            }),
            plain: Mutex::new(output::PhaseLog::new()),
            animation: Mutex::new(None),
        }
    }

    pub fn update(&self, ev: &ProgressEvent) {
        if self.state.closed.load(Ordering::Relaxed) {
            return;
        }
        if !self.tty {
            let line = self
                .plain
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .line(ev);
            if let Some(line) = line {
                let _ = writeln!(std::io::stderr().lock(), "{line}");
            }
            return;
        }
        self.state.phases().update(ev);
        self.state.draw();
        self.start_animation();
    }

    /// Prints `text` on its own line above the progress line (a plain line off a terminal).
    pub fn notice(&self, text: &str) {
        if self.state.closed.load(Ordering::Relaxed) {
            return;
        }
        if !self.tty {
            let _ = writeln!(std::io::stderr().lock(), "{text}");
            return;
        }
        *self.state.notice.lock().unwrap_or_else(|p| p.into_inner()) = Some(text.to_string());
        self.state.draw();
    }

    fn start_animation(&self) {
        let mut slot = self.animation.lock().unwrap_or_else(|p| p.into_inner());
        if slot.is_some() || self.state.closed.load(Ordering::Relaxed) {
            return;
        }
        let (stop, rx) = mpsc::channel::<()>();
        let state = Arc::clone(&self.state);
        let handle = std::thread::spawn(move || {
            let mut polled = std::time::Instant::now();
            // `Timeout` is a tick; a message or the sender dropping ends the thread.
            while let Err(mpsc::RecvTimeoutError::Timeout) = rx.recv_timeout(TICK) {
                if polled.elapsed() >= LOG_POLL {
                    polled = std::time::Instant::now();
                    state.poll_log();
                }
                state.frame.fetch_add(1, Ordering::Relaxed);
                state.draw();
            }
        });
        *slot = Some(Animation { stop, handle });
    }

    /// Stops and joins the animation thread, then erases the line.
    pub fn finish(&self) {
        // Closed for good: `finish` runs once per command (`end_progress`), not between plans.
        self.state.closed.store(true, Ordering::Relaxed);
        let animation = self
            .animation
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(Animation { stop, handle }) = animation {
            drop(stop);
            if let Err(panic) = handle.join() {
                // A panic while already unwinding (Drop) cannot be re-raised.
                if !std::thread::panicking() {
                    std::panic::resume_unwind(panic);
                }
            }
        }
        let width = self.state.drawn.swap(0, Ordering::Relaxed);
        if width > 0 {
            let mut err = std::io::stderr().lock();
            let _ = write!(err, "\r{:<width$}\r", "");
            let _ = err.flush();
        }
    }
}

impl Drop for LineProgress {
    fn drop(&mut self) {
        self.finish();
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Flags {
    pub yes: bool,
    pub dry_run: bool,
    /// `--allow-msiexec`: an MSI the own extractor refuses may use `msiexec /a`. Never
    /// implied by `--yes`.
    pub allow_msiexec: bool,
    /// `--allow-scripts`: the manifest's third-party code (hooks, installer and uninstaller
    /// files) may run, after being shown. Never implied by `--yes`.
    pub allow_scripts: bool,
}

impl Flags {
    fn allow(self) -> ops::Allow {
        ops::Allow {
            msiexec: if self.allow_msiexec {
                AllowMsiexec::All
            } else {
                AllowMsiexec::No
            },
            scripts: self.allow_scripts,
        }
    }
}

/// Shows the operation's third-party code (with `--dry-run` or `--allow-scripts`) and,
/// unless it is a dry run, refuses it without `--allow-scripts`, even with `--yes`.
fn third_party_gate(code: &[(String, Vec<ThirdParty>)], f: Flags) -> anyhow::Result<()> {
    let needs_scripts = !code.is_empty();
    if needs_scripts && (f.dry_run || f.allow_scripts) {
        for (app, items) in code {
            if code.len() > 1 {
                anstream::println!("{}:", style::paint(style::NAME, app));
            }
            anstream::print!("{}", output::third_party_block(items));
        }
    }
    if f.dry_run {
        return Ok(());
    }
    if needs_scripts && !f.allow_scripts {
        let apps: Vec<&str> = code.iter().map(|(app, _)| app.as_str()).collect();
        let items = code.iter().flat_map(|(_, items)| items);
        bail!("{}", output::third_party_refusal(&apps, items));
    }
    Ok(())
}

/// The plans that run third-party code, with that code.
fn plans_code<'a>(
    plans: impl Iterator<Item = &'a crate::install::plan::InstallPlan>,
) -> Vec<(String, Vec<ThirdParty>)> {
    plans
        .map(|p| (p.app.clone(), output::plan_third_party(p)))
        .filter(|(_, items)| !items.is_empty())
        .collect()
}

/// Asks to proceed: `--yes` skips, no terminal without `--yes` is an error.
pub(super) fn proceed(ui: &dyn Ui, f: Flags) -> anyhow::Result<bool> {
    if f.yes {
        return Ok(true);
    }
    if !ui.interactive() {
        bail!("a confirmação exige um terminal interativo; use --yes");
    }
    ui.confirm("Continuar?")
}

/// Writes the `job.end` line of a CLI job from its result.
fn finish_log<T, E: std::fmt::Display>(log: JobLog, result: &Result<T, E>, ok: &str) {
    match result {
        Ok(_) => log.finish(Outcome::Ok, ok),
        Err(e) => log.finish(Outcome::Failed, &e.to_string()),
    }
}

/// Runs one of the operation commands. `interrupts` (Ctrl+C) are listened to only while the
/// disk changes, deciding through `ctx.launch`. Apps wingetrs did not install go to `winget`.
pub async fn execute(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    cmd: Command,
    interrupts: &mut impl Interrupts,
    winget: &impl WingetFlows,
) -> anyhow::Result<()> {
    let stop = Stop {
        interrupts,
        gate: &ctx.launch,
    };
    match cmd {
        Command::Install {
            target,
            bucket,
            dry_run,
            yes,
            allow_msiexec,
            allow_scripts,
            scope,
        } => {
            let f = Flags {
                yes,
                dry_run,
                allow_msiexec,
                allow_scripts,
            };
            install(ctx, ui, &target, bucket.as_deref(), scope, f, stop, winget).await
        }
        Command::Uninstall {
            target,
            purge,
            dry_run,
            yes,
            allow_scripts,
        } => {
            let f = Flags {
                yes,
                dry_run,
                allow_msiexec: false,
                allow_scripts,
            };
            uninstall(ctx, ui, &target, purge, f, stop, winget).await
        }
        Command::Upgrade {
            target,
            all,
            keep,
            dry_run,
            yes,
            allow_msiexec,
            allow_scripts,
        } => {
            let f = Flags {
                yes,
                dry_run,
                allow_msiexec,
                allow_scripts,
            };
            upgrade(ctx, ui, target.as_deref(), all, keep, f, stop, winget).await
        }
        Command::Repair => {
            anstream::print!("{}", output::repair_text(&ops::repair(ctx)?));
            Ok(())
        }
        Command::Cleanup {
            orphans,
            dry_run,
            yes,
        } => {
            let a = CleanupArgs {
                orphans,
                dry_run,
                yes,
            };
            winget_cmd::cleanup(winget, ui, a, &mut stdout_line)
        }
        _ => bail!("comando não é uma operação de instalação"),
    }
}

/// One line on stdout (the `Say` of the winget commands).
fn stdout_line(line: &str) {
    anstream::println!("{line}");
}

/// What [`execute`] listens to while the disk changes.
pub struct Stop<'a, I: Interrupts> {
    pub interrupts: &'a mut I,
    gate: &'a LaunchGate,
}

impl<'a, I: Interrupts> Stop<'a, I> {
    /// Listens to `interrupts`; a request cancels through `gate` until something launched.
    pub fn new(interrupts: &'a mut I, gate: &'a LaunchGate) -> Stop<'a, I> {
        Stop { interrupts, gate }
    }

    /// [`until_interrupted`], telling the user through `ui` when Ctrl+C cannot stop it.
    pub(super) async fn run<T>(
        &mut self,
        ui: &dyn Ui,
        op: impl std::future::Future<Output = T>,
    ) -> Option<T> {
        let gate = self.gate;
        let refused = || ui.notice(&uncancellable(gate.running().as_deref()));
        until_interrupted(op, self.interrupts, gate, &refused).await
    }
}

/// The candidate to act on: the only match, or the user's pick when a terminal allows it.
/// `Ok(None)` = the user cancelled.
fn choose(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    target: &str,
    bucket: Option<&str>,
    yes: bool,
) -> anyhow::Result<Option<Candidate>> {
    match ops::resolve_target(ctx, target, bucket)? {
        Resolution::One(c) => Ok(Some(c)),
        Resolution::NotFound { suggestions } => {
            bail!("{}", output::not_found_text(target, &suggestions))
        }
        Resolution::Many(mut found) => {
            // Never pick on the user's behalf: `--yes` and a missing terminal both fail.
            if yes || !ui.interactive() {
                let names: Vec<String> = found
                    .iter()
                    .map(|c| output::bucket_id(&c.entry.source_name, &c.entry.id))
                    .collect();
                bail!("{}", output::ambiguous_text(target, &names));
            }
            let items = found
                .iter()
                .map(|c| PickerItem {
                    label: output::bucket_id(&c.entry.source_name, &c.entry.id),
                    detail: c.entry.version.clone(),
                    selectable: c.selectable,
                    note: c.note.clone(),
                })
                .collect();
            Ok(ui
                .pick_target(items)?
                .and_then(|i| (i < found.len()).then(|| found.swap_remove(i))))
        }
    }
}

/// `--dry-run` prints every step; otherwise the short summary.
fn print_plans(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    plans: &[crate::install::plan::InstallPlan],
    f: Flags,
) {
    // Read once: the user PATH decides whether a PATH line is news.
    let user_path = match ctx.effects.env.get(crate::install::executor::PATH_VAR) {
        Ok(Some(v)) => Some(v.value),
        _ => None,
    };
    for p in plans {
        if f.dry_run {
            anstream::println!("{p}");
            if let Some(hint) = output::msi_hint(p, f.allow_msiexec) {
                anstream::println!("{}", style::paint(style::DIM, hint));
            }
        } else {
            let layout = crate::install::layout::Layout::new(p.root.clone());
            anstream::print!(
                "{}",
                output::plan_summary(
                    p,
                    &layout,
                    ui.glyphs(),
                    user_path.as_deref(),
                    f.allow_msiexec
                )
            );
        }
    }
}

/// `Abra um terminal novo…` when the plan put a folder on the user PATH this process lacks.
fn print_terminal_hint(plan: &crate::install::plan::InstallPlan) {
    let layout = crate::install::layout::Layout::new(plan.root.clone());
    let path = std::env::var_os("PATH");
    if let Some(hint) = output::new_terminal_hint(plan, &layout, path.as_deref()) {
        anstream::println!("{hint}");
    }
}

#[allow(clippy::too_many_arguments)]
async fn install(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    target: &str,
    bucket: Option<&str>,
    scope: Option<crate::bridge::worker::ScopeArg>,
    f: Flags,
    mut stop: Stop<'_, impl Interrupts>,
    winget: &impl WingetFlows,
) -> anyhow::Result<()> {
    let Some(cand) = choose(ctx, ui, target, bucket, f.yes)? else {
        anstream::println!("{}", style::paint(style::WARN, "Cancelado."));
        return Ok(());
    };
    if cand.entry.source == crate::core::SourceKind::Winget {
        return winget_cmd::install(winget, ui, &cand, scope, f, stop, &mut stdout_line).await;
    }
    if scope.is_some() {
        bail!("--scope só vale para pacotes do winget");
    }
    let plans = ops::plan_for(ctx, &cand, false)?;
    print_plans(ctx, ui, &plans, f);
    third_party_gate(&plans_code(plans.iter()), f)?;
    if f.dry_run {
        anstream::println!("Simulação: nada foi alterado.");
        return Ok(());
    }
    if !proceed(ui, f)? {
        anstream::println!("{}", style::paint(style::WARN, "Cancelado."));
        return Ok(());
    }
    // Held from before `job.start` until after `job.end`: the log always gets its end.
    stop.interrupts.hold();
    let log = JobLog::start(JobInfo::for_plans("install", &plans));
    let applied = stop.run(ui, ops::apply_with(ctx, &plans, &f.allow())).await;
    ui.end_progress();
    let Some(applied) = applied else {
        let r = cancelled(log);
        stop.interrupts.release();
        return r;
    };
    finish_log(log, &applied, "instalado");
    stop.interrupts.release();
    applied?;
    for p in &plans {
        anstream::println!(
            "{} {} {}",
            style::paint(style::OK, "Instalado:"),
            p.app,
            p.version
        );
        print_terminal_hint(p);
    }
    Ok(())
}

async fn uninstall(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    target: &str,
    purge: bool,
    f: Flags,
    stop: Stop<'_, impl Interrupts>,
    winget: &impl WingetFlows,
) -> anyhow::Result<()> {
    // Installed apps are named by id only; accept `bucket/id` too.
    let app = target.rsplit('/').next().unwrap_or(target);
    let plan = match ops::plan_uninstall(ctx, app, purge) {
        // Not an app of wingetrs: one of the Windows list (ARP, MSIX).
        Err(ops::OpsError::NotInstalled(_)) => {
            return winget_cmd::uninstall(winget, ui, target, purge, f, stop, &mut stdout_line)
                .await;
        }
        other => other?,
    };
    anstream::print!(
        "{}",
        output::uninstall_plan_text(&plan, &ctx.data.install_layout(), ui.glyphs())
    );
    let (manifest, shown_sha) = ops::uninstall_manifest(ctx, &plan)?;
    let layout = ctx.data.install_layout();
    let vdir = layout.version_dir(&plan.app, &plan.version);
    let roots = CallRoots::new(layout.root(), Some(&vdir));
    let code: Vec<_> = manifest
        .map(|m| output::removal_third_party(&m, &vdir, "", &roots))
        .filter(|items| !items.is_empty())
        .map(|items| (plan.app.clone(), items))
        .into_iter()
        .collect();
    third_party_gate(&code, f)?;
    if f.dry_run {
        anstream::println!("Simulação: nada foi alterado.");
        return Ok(());
    }
    if !proceed(ui, f)? {
        anstream::println!("{}", style::paint(style::WARN, "Cancelado."));
        return Ok(());
    }
    // Ctrl+C before this point ends the process with nothing removed. From here the removal
    // (synchronous, seconds) is not cut in the middle: Ctrl+C is held, then let go.
    stop.interrupts.hold();
    let log = JobLog::start(JobInfo {
        kind: "remove",
        tool: "wingetrs",
        target: plan.app.clone(),
        name: plan.app.clone(),
        from: Some(plan.version.clone()),
        to: None,
        cmd: None,
    });
    // The consent covers the code shown above: checked again under `install.lock`.
    let scripts = f.allow_scripts.then_some(shown_sha.as_str());
    let removed = ops::uninstall_with(ctx, &plan, scripts);
    ui.end_progress();
    finish_log(log, &removed, "removido");
    stop.interrupts.release();
    let removed = removed?;
    anstream::println!(
        "{} {} {}",
        style::paint(style::OK, "Desinstalado:"),
        plan.app,
        plan.version
    );
    for note in &removed.notes {
        anstream::eprintln!("{} {note}", style::paint(style::WARN, "aviso:"));
    }
    clean_residues(ctx, ui, &plan.app, &removed.residues, f.yes)
}

fn clean_residues(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    app: &str,
    residues: &[Residue],
    yes: bool,
) -> anyhow::Result<()> {
    if residues.is_empty() {
        return Ok(());
    }
    let selected: Vec<Residue> = if yes {
        crate::install::residue::auto_clean(residues)
    } else if ui.interactive() {
        let items = residues
            .iter()
            .map(|r| PickerItem {
                label: output::residue_label(r),
                detail: match r.confidence {
                    Confidence::High => "confiança alta".into(),
                    Confidence::Low => "confiança baixa".into(),
                },
                selectable: true,
                note: r.note.clone(),
            })
            .collect();
        let initial: Vec<bool> = residues.iter().map(|r| r.preselected).collect();
        match ui.pick_many(items, &initial)? {
            Some(idx) => idx
                .into_iter()
                .filter_map(|i| residues.get(i).cloned())
                .collect(),
            None => Vec::new(),
        }
    } else {
        anstream::println!(
            "Resíduos encontrados (nada foi removido; use --yes para os de confiança alta):"
        );
        for r in residues {
            anstream::println!("  {}", output::residue_label(r));
        }
        return Ok(());
    };
    if selected.is_empty() {
        anstream::println!("Nenhum resíduo removido.");
        return Ok(());
    }
    let backups = ctx.data.install_layout().backups();
    let results = residue::clean(&selected, app, &ctx.effects, &backups);
    for (r, res) in selected.iter().zip(results) {
        match res {
            Ok(()) => anstream::println!(
                "{} {}",
                style::paint(style::OK, "Removido:"),
                output::residue_label(r)
            ),
            Err(e) => anstream::eprintln!("{} {e}", style::paint(style::WARN, "aviso:")),
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn upgrade(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    target: Option<&str>,
    all: bool,
    keep: bool,
    f: Flags,
    mut stop: Stop<'_, impl Interrupts>,
    winget: &impl WingetFlows,
) -> anyhow::Result<()> {
    let list = ops::upgradable(ctx)?;
    let pins = crate::pins::load(ctx.data);
    let (list, pinned) = list.without_pinned(&pins);
    let state = ops::read_state(&ctx.data.install_layout())?;
    let chosen: Vec<(String, String, String)> = match target {
        Some(t) => {
            let t = t.rsplit('/').next().unwrap_or(t);
            if let Some((app, reason)) =
                list.skipped.iter().find(|(a, _)| a.eq_ignore_ascii_case(t))
            {
                bail!("{app}: {reason}");
            }
            if crate::pins::is_pinned(&pins, "wingetrs", t) {
                bail!("{t} está com a versão fixada; desafixe na TUI (menu de ações)");
            }
            let found: Vec<_> = list
                .upgradable
                .into_iter()
                .filter(|(a, _, _)| a.eq_ignore_ascii_case(t))
                .collect();
            if found.is_empty() {
                if state.get(t)?.is_none() {
                    // Not an app of wingetrs: one of the Windows list, matched to winget.
                    let whole = target.unwrap_or(t);
                    return winget_cmd::upgrade(winget, ui, whole, f, stop, &mut stdout_line).await;
                }
                anstream::println!("{t} já está na versão mais recente.");
                return Ok(());
            }
            found
        }
        None => {
            for (app, reason) in &list.skipped {
                anstream::eprintln!("{} {app}: {reason}", style::paint(style::WARN, "aviso:"));
            }
            if !all {
                anstream::print!("{}", output::upgrades_text(&list.upgradable));
                for app in &pinned {
                    anstream::println!("{app} (fixado)");
                }
                if list.upgradable.is_empty() {
                    anstream::println!("Nada para atualizar.");
                } else {
                    anstream::println!("Use `wingetrs upgrade --all` ou `wingetrs upgrade <app>`.");
                }
                return Ok(());
            }
            for app in &pinned {
                anstream::println!("fixado, pulado: {app}");
            }
            list.upgradable
        }
    };
    if chosen.is_empty() {
        anstream::println!("Nada para atualizar.");
        return Ok(());
    }
    let mut groups = Vec::new();
    for (app, _, _) in &chosen {
        let Some((rec, _)) = state.get(app)? else {
            bail!("{app} não está instalado pelo wingetrs");
        };
        let cand = match ops::resolve_installed(ctx, app, &rec.bucket)? {
            Resolution::One(c) => c,
            _ => bail!("{app} não foi encontrado no bucket {}", rec.bucket),
        };
        groups.push(ops::plan_for(ctx, &cand, keep)?);
    }
    for g in &groups {
        print_plans(ctx, ui, g, f);
    }
    third_party_gate(&plans_code(groups.iter().flatten()), f)?;
    if f.dry_run {
        anstream::println!("Simulação: nada foi alterado.");
        return Ok(());
    }
    if !proceed(ui, f)? {
        anstream::println!("{}", style::paint(style::WARN, "Cancelado."));
        return Ok(());
    }
    stop.interrupts.hold();
    let log = JobLog::start(JobInfo::for_plans("upgrade", groups.iter().flatten()));
    let applied = stop
        .run(ui, ops::apply_groups_with(ctx, groups, &f.allow()))
        .await;
    ui.end_progress();
    let Some(applied) = applied else {
        let r = cancelled(log);
        stop.interrupts.release();
        return r;
    };
    finish_log(log, &applied, "atualizado");
    stop.interrupts.release();
    for p in applied? {
        anstream::println!(
            "{} {} {}",
            style::paint(style::OK, "Atualizado:"),
            p.app,
            p.version
        );
        print_terminal_hint(&p);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoUi;
    impl Ui for NoUi {
        fn interactive(&self) -> bool {
            false
        }
        fn confirm(&self, _: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        fn pick(&self, _: Vec<PickerItem>) -> anyhow::Result<Option<usize>> {
            Ok(None)
        }
        fn pick_many(&self, _: Vec<PickerItem>, _: &[bool]) -> anyhow::Result<Option<Vec<usize>>> {
            Ok(None)
        }
    }

    fn no_progress(_: &crate::install::progress::ProgressEvent) {}
    fn nothing_in_use(_: &std::path::Path) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    #[test]
    fn yes_cleans_only_high_confidence_preselected_residues() {
        use crate::install::ops::ResidueScan;
        use crate::install::residue::ResidueKind;
        use crate::install::traits::Effects;
        use crate::install::traits::fake::{FakeEnv, FakeRegistry, FakeShortcuts, FakeTrash};
        use std::path::PathBuf;
        let t = tempfile::tempdir().unwrap();
        let data = crate::cache::DataDir::new(t.path());
        let http = crate::http::Http::new().unwrap();
        let catalog = crate::catalog::Catalog::open_in_memory().unwrap();
        let (env, sc, trash, reg) = (
            FakeEnv::new(),
            FakeShortcuts::new(t.path().join("sm")),
            FakeTrash::new(),
            FakeRegistry::new(),
        );
        let ctx = OpsCtx {
            data: &data,
            http: &http,
            catalog: &catalog,
            effects: Effects {
                env: &env,
                shortcuts: &sc,
                trash: &trash,
                registry: &reg,
            },
            wait: crate::update::LockWait::NoWait,
            progress: &no_progress,
            in_use: &nothing_in_use,
            residue: ResidueScan::none(),
            msiexec: crate::install::executor::Msiexec::DISABLED,
            launch: Default::default(),
            runners: crate::install::executor::Runners::disabled(),
        };
        let res = |name: &str, confidence, preselected| Residue {
            kind: ResidueKind::Folder {
                path: PathBuf::from(name),
            },
            confidence,
            preselected,
            note: None,
        };
        let residues = [
            res("high-pre", Confidence::High, true),
            res("low-pre", Confidence::Low, true),
            res("high-not", Confidence::High, false),
        ];
        clean_residues(&ctx, &NoUi, "app", &residues, true).unwrap();
        assert_eq!(trash.dump(), vec![PathBuf::from("high-pre")]);
        // Without --yes and without a terminal nothing is removed.
        clean_residues(&ctx, &NoUi, "app", &residues, false).unwrap();
        assert_eq!(trash.dump().len(), 1);
        // In a terminal the question is the selector: pre-marked rows, Space marks one more.
        use crate::cli::picker::fake::{self, ScriptedTerm, key};
        use crossterm::event::KeyCode::{Char, Down, Enter};
        let term = ScriptedTerm::new(vec![key(Down), key(Down), key(Char(' ')), key(Enter)]);
        let screen = term.screen();
        let ui = TtyUi::with_term(&ascii_config(), Box::new(term));
        clean_residues(&ctx, &ui, "app", &residues, false).unwrap();
        assert_eq!(trash.dump().len(), 4);
        let text = fake::plain(&screen.lock().unwrap().out);
        assert!(text.starts_with("> [x] "), "{text:?}");
        assert!(text.contains("  [ ] "), "{text:?}");
    }

    fn ascii_config() -> crate::config::Config {
        crate::config::Config {
            glyphs: "ascii".into(),
            ..Default::default()
        }
    }
}
