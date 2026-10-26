//! `install`, `uninstall`, `upgrade` and `repair`: plan, show, confirm, execute.
//! Platform-neutral: every real effect arrives through `OpsCtx`, every prompt through `Ui`.
use super::picker::{self, StdTerm, Term};
use super::winget_cmd::{self, CleanupArgs, WingetFlows};
use super::{Command, emit, output};
use crate::github::Asset;
use crate::github::install::{GhEnv, GhTarget};
use crate::install::executor::LaunchGate;
use crate::install::ops::{self, AllowMsiexec, OpsCtx, Resolution};
use crate::install::progress::ProgressEvent;
use crate::install::residue::{self, Confidence, Residue};
use crate::install::script_policy::{AdminBlock, admin_terminal_refusal, elevated_or_unknown};
use crate::install::script_scan::CallRoots;
use crate::install::target::Candidate;
use crate::job_log::{JobInfo, JobLog, Outcome};
use crate::tui::picker::PickerItem;
use crate::tui::theme::Glyphs;
use anyhow::bail;
use output::{Phases, ThirdParty};
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
    /// A line of the command's result on stdout (e.g. the output of winget or scoop through
    /// the bridge), printed above the progress line, which stays on stderr.
    fn output(&self, text: &str) {
        emit::out(text);
    }
    /// A plan block on stdout (`text` carries its own line ends), before any question.
    fn plan(&self, text: &str) {
        emit::out_block(text);
    }
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
    emit::out(&output::cancelled());
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
        TtyUi::build(cfg, Box::new(StdTerm::new()), emit::stderr_is_terminal())
    }

    /// Questions go to `term` (tests inject keys there); the progress line stays plain.
    pub fn with_term(cfg: &crate::config::Config, term: Box<dyn Term + Send>) -> TtyUi {
        TtyUi::build(cfg, term, false)
    }

    fn build(cfg: &crate::config::Config, term: Box<dyn Term + Send>, tty: bool) -> TtyUi {
        TtyUi {
            progress: progress_for(cfg, tty),
            term: Mutex::new(term),
            theme: theme_of(cfg),
            glyphs: glyphs_of(cfg),
        }
    }

    /// The progress line this UI stops through [`Ui::end_progress`]; feed it to `OpsCtx.progress`.
    pub fn progress(&self) -> Arc<LineProgress> {
        Arc::clone(&self.progress)
    }
}

/// The TUI theme the config picks (dark or light, at the terminal's color depth).
fn theme_of(cfg: &crate::config::Config) -> crate::tui::theme::Theme {
    use crate::tui::theme::{Theme, detect_depth};
    Theme::new(
        cfg.theme != "light",
        detect_depth(
            std::env::var("COLORTERM").ok().as_deref(),
            std::env::var("NO_COLOR").ok().as_deref(),
        ),
    )
}

/// The glyph set the config picks.
fn glyphs_of(cfg: &crate::config::Config) -> &'static Glyphs {
    use crate::tui::theme::GlyphSet;
    Glyphs::for_set(match cfg.glyphs.as_str() {
        "unicode" => GlyphSet::Unicode,
        "ascii" => GlyphSet::Ascii,
        _ => GlyphSet::Nerd,
    })
}

/// The progress line on stderr with the config's glyphs and the theme's colors; animated
/// when `tty`, one plain line per phase otherwise.
fn progress_for(cfg: &crate::config::Config, tty: bool) -> Arc<LineProgress> {
    let styles = super::style::ProgressStyles::of(&theme_of(cfg));
    Arc::new(LineProgress::new(tty, glyphs_of(cfg), styles))
}

/// The progress line of a command (`update`, `list`, an install): animated when stderr is a
/// terminal.
pub fn line_progress(cfg: &crate::config::Config) -> Arc<LineProgress> {
    progress_for(cfg, emit::stderr_is_terminal())
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

    fn output(&self, text: &str) {
        self.progress.stdout_line(text);
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
    styles: super::style::ProgressStyles,
    phases: Mutex<Phases>,
    frame: AtomicUsize,
    /// Chars of the line on screen (0 = none): the next draw pads to it, the erase clears it.
    drawn: AtomicUsize,
    /// Set by `finish`: nothing is drawn afterwards.
    closed: AtomicBool,
    /// A line to print once above the progress line ([`LineProgress::notice`]).
    notice: Mutex<Option<String>>,
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
        let mut err = emit::progress();
        // `finish` sets `closed` before erasing under this same lock, so no late redraw.
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        if let Some(notice) = notice {
            let width = self.drawn.swap(0, Ordering::Relaxed);
            err.line_above(width, &notice);
        }
        let Some(parts) = parts else {
            err.flush();
            return;
        };
        let (line, len) = output::progress_frame(&parts, cols, &self.styles);
        let pad = self.drawn.load(Ordering::Relaxed).saturating_sub(len);
        err.redraw(&line, pad);
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
    /// Phase lines for the transcript while the terminal shows the animated line.
    transcribed: Mutex<output::PhaseLog>,
    animation: Mutex<Option<Animation>>,
}

impl LineProgress {
    pub fn new(
        tty: bool,
        glyphs: &'static Glyphs,
        styles: super::style::ProgressStyles,
    ) -> LineProgress {
        LineProgress {
            tty,
            state: Arc::new(ProgressState {
                glyphs,
                styles,
                phases: Mutex::new(Phases::new()),
                frame: AtomicUsize::new(0),
                drawn: AtomicUsize::new(0),
                closed: AtomicBool::new(false),
                notice: Mutex::new(None),
            }),
            plain: Mutex::new(output::PhaseLog::new()),
            transcribed: Mutex::new(output::PhaseLog::new()),
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
                emit::err(&line);
            }
            return;
        }
        if crate::transcript::enabled() {
            let line = self
                .transcribed
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .line(ev);
            if let Some(line) = line {
                crate::transcript::line(&line);
            }
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
            emit::err(text);
            return;
        }
        *self.state.notice.lock().unwrap_or_else(|p| p.into_inner()) = Some(text.to_string());
        self.state.draw();
    }

    /// Prints `text` on stdout: the progress line (stderr, same terminal) is erased first and
    /// drawn again on the next tick, so the two never share a row.
    pub fn stdout_line(&self, text: &str) {
        if !self.tty || self.state.closed.load(Ordering::Relaxed) {
            emit::out(text);
            return;
        }
        let mut err = emit::progress();
        err.erase(self.state.drawn.swap(0, Ordering::Relaxed));
        emit::out_flushed(text);
        drop(err);
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
            emit::progress().erase(width);
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
    /// `--allow-unreviewed`: an install from a GitHub release (no bucket reviewed it). Never
    /// implied by `--yes`.
    pub allow_unreviewed: bool,
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
                emit::out(&format!("{}:", output::name(app)));
            }
            emit::out_block(&output::third_party_block(items));
        }
    }
    if f.dry_run {
        return Ok(());
    }
    if needs_scripts && !f.allow_scripts {
        let apps: Vec<&str> = code.iter().map(|(app, _)| app.as_str()).collect();
        let items = code.iter().flat_map(|(_, items)| items);
        return Err(output::third_party_refusal(&apps, items).into());
    }
    Ok(())
}

/// The administrator-terminal refusal of third-party `code` (and innounp when `inno`): `None`
/// when not elevated or there is none (Task 218). The one text of the CLI and the TUI.
pub fn admin_code_refusal(elevated: bool, code: &[ThirdParty], inno: bool) -> Option<String> {
    if !elevated {
        return None;
    }
    let mut what: Vec<String> = Vec::new();
    let labels = code.iter().map(ThirdParty::label);
    for label in labels.chain(inno.then(|| "innounp".to_string())) {
        if !what.contains(&label) {
            what.push(label);
        }
    }
    if what.is_empty() {
        return None;
    }
    let what = what.join(", ");
    admin_terminal_refusal(elevated, AdminBlock::ThirdPartyCode { what })
}

/// [`admin_code_refusal`] of the first plan of `plans` that cannot run as administrator.
pub fn plans_admin_refusal(
    elevated: bool,
    plans: &[crate::install::plan::InstallPlan],
) -> Option<String> {
    if !elevated {
        return None;
    }
    plans.iter().find_map(|p| {
        let inno = p
            .steps
            .iter()
            .any(|s| matches!(s, crate::install::plan::Step::ExtractInno { .. }));
        // Only what runs now: the code the manifest runs at removal time is refused then.
        let now = crate::install::plan::InstallPlan {
            third_party_uninstaller: false,
            has_uninstall_scripts: false,
            ..p.clone()
        };
        // Only the labels of the code count here (no report): no parser is asked.
        let code = output::plan_third_party(&now, &crate::install::script_ast::Unavailable);
        admin_code_refusal(elevated, &code, inno)
    })
}

/// An administrator-terminal refusal ends the command before the consent block and
/// "Continuar?"; a dry run keeps it to print as its last warning.
fn admin_gate(refusal: Option<String>, f: Flags) -> anyhow::Result<Option<String>> {
    match refusal {
        Some(r) if !f.dry_run => Err(anyhow::Error::msg(r)),
        other => Ok(other),
    }
}

type Group = Vec<crate::install::plan::InstallPlan>;

/// The upgrade groups after the administrator-terminal check (Task 218).
struct AdminSplit {
    /// The groups to show (and, unless `fail`, to apply).
    groups: Vec<Group>,
    /// One warning per refused group: printed at the end of a dry run, else before the
    /// consent block.
    warnings: Vec<String>,
    /// Every group refused (not a dry run): the command's error, after the plans are shown.
    fail: Option<String>,
}

/// Splits the upgrade groups an administrator terminal refuses. A dry run keeps every group;
/// otherwise each refused group is skipped and the others go on, and refusing every group
/// fails the command (its plans still shown, as `install` does).
fn admin_split(groups: Vec<Group>, elevated: bool, f: Flags) -> AdminSplit {
    let total = groups.len();
    let refusals: Vec<Option<String>> = groups
        .iter()
        .map(|g| plans_admin_refusal(elevated, g))
        .collect();
    let lines: Vec<String> = groups
        .iter()
        .zip(&refusals)
        .filter_map(|(g, r)| {
            let r = r.as_deref()?;
            if total == 1 {
                return Some(r.to_string());
            }
            let apps: Vec<&str> = g.iter().map(|p| p.app.as_str()).collect();
            Some(format!("{} pulado: {r}", apps.join(", ")))
        })
        .collect();
    if f.dry_run || lines.is_empty() {
        return AdminSplit {
            groups,
            warnings: lines,
            fail: None,
        };
    }
    if lines.len() == total {
        let first = refusals.into_iter().flatten().next();
        let warnings = if total > 1 { lines } else { Vec::new() };
        return AdminSplit {
            groups,
            warnings,
            fail: first,
        };
    }
    let kept = groups
        .into_iter()
        .zip(refusals)
        .filter_map(|(g, r)| r.is_none().then_some(g))
        .collect();
    AdminSplit {
        groups: kept,
        warnings: lines,
        fail: None,
    }
}

/// The dry run's end: the administrator-terminal warning, if any, then "nothing changed".
fn dry_run_done(ui: &dyn Ui, refusal: Option<&str>) {
    if let Some(r) = refusal {
        ui.output(&output::warning(r));
    }
    emit::out(output::DRY_RUN_DONE);
}

/// The plans that run third-party code, with that code.
fn plans_code<'a>(
    plans: impl Iterator<Item = &'a crate::install::plan::InstallPlan>,
    ast: &dyn crate::install::script_ast::AstGate,
) -> Vec<(String, Vec<ThirdParty>)> {
    plans
        .map(|p| (p.app.clone(), output::plan_third_party(p, ast)))
        .filter(|(_, items)| !items.is_empty())
        .collect()
}

/// Asks to proceed: `--yes` skips, no terminal without `--yes` is an error.
pub(super) fn proceed(ui: &dyn Ui, f: Flags) -> anyhow::Result<bool> {
    if f.yes {
        return Ok(true);
    }
    if !ui.interactive() {
        return Err(
            output::Advice::new("a confirmação exige um terminal interativo", "use --yes").into(),
        );
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
            allow_unreviewed,
            asset,
        } => {
            let f = Flags {
                yes,
                dry_run,
                allow_msiexec,
                allow_scripts,
                allow_unreviewed,
            };
            let t = Target {
                name: &target,
                bucket: bucket.as_deref(),
                asset: asset.as_deref(),
            };
            install(ctx, ui, t, scope, f, stop, winget).await
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
                allow_unreviewed: false,
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
                allow_unreviewed: false,
            };
            upgrade(ctx, ui, target.as_deref(), all, keep, f, stop, winget).await
        }
        Command::Repair => {
            emit::out_block(&output::repair_text(&ops::repair(ctx)?));
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
    emit::out(line);
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
        Resolution::NotFound { suggestions } => Err(output::not_found(target, &suggestions).into()),
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
            ui.plan(&output::install_plan_text(p, ui.glyphs()));
            if let Some(hint) = output::msi_hint(p, f.allow_msiexec) {
                emit::out(&output::dim(hint));
            }
        } else {
            let layout = crate::install::layout::Layout::new(p.root.clone());
            ui.plan(&output::plan_summary(
                p,
                &layout,
                ui.glyphs(),
                user_path.as_deref(),
                f.allow_msiexec,
            ));
        }
    }
}

/// `Abra um terminal novo…` when the plan put a folder on the user PATH this process lacks.
fn print_terminal_hint(plan: &crate::install::plan::InstallPlan) {
    let layout = crate::install::layout::Layout::new(plan.root.clone());
    let path = std::env::var_os("PATH");
    if let Some(hint) = output::new_terminal_hint(plan, &layout, path.as_deref()) {
        emit::out(&hint);
    }
}

/// What `install` was asked for.
pub struct Target<'a> {
    pub name: &'a str,
    pub bucket: Option<&'a str>,
    /// `--asset`: the release file of a GitHub target (Task 210).
    pub asset: Option<&'a str>,
}

async fn install(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    t: Target<'_>,
    scope: Option<crate::bridge::worker::ScopeArg>,
    f: Flags,
    stop: Stop<'_, impl Interrupts>,
    winget: &impl WingetFlows,
) -> anyhow::Result<()> {
    // Before `choose`: `gh:owner` would read as a bucket name.
    if let Some(gh) = crate::github::install::parse_target(t.name) {
        let gh = gh.map_err(|e| {
            output::Advice::new(
                e.to_string(),
                "use gh:<dono>/<repo>[@<tag>], https://github.com/<dono>/<repo> ou …/releases/tag/<tag>",
            )
        })?;
        return install_github(ctx, ui, &gh, t.asset, scope, f, stop, winget).await;
    }
    if t.asset.is_some() {
        return Err(output::Advice::new(
            "--asset só vale para releases do GitHub",
            "use gh:<dono>/<repo> ou rode de novo sem --asset",
        )
        .into());
    }
    let Some(cand) = choose(ctx, ui, t.name, t.bucket, f.yes)? else {
        emit::out(&output::cancelled());
        return Ok(());
    };
    if cand.entry.source == crate::core::SourceKind::Winget {
        return winget_cmd::install(winget, ui, &cand, scope, f, stop, &mut stdout_line).await;
    }
    if scope.is_some() {
        return Err(output::Advice::new(
            "--scope só vale para pacotes do winget",
            "rode de novo sem --scope",
        )
        .into());
    }
    let plans = ops::plan_for(ctx, &cand, false)?;
    let elevated = elevated_or_unknown(winget.elevated());
    apply_install(ctx, ui, plans, f, elevated, stop).await
}

/// Shows `plans`, asks, and applies them (the scoop install; also a winget portable package,
/// Task 189). `elevated`: this process runs as administrator (third-party code is refused).
pub(crate) async fn apply_install(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    plans: Vec<crate::install::plan::InstallPlan>,
    f: Flags,
    elevated: bool,
    stop: Stop<'_, impl Interrupts>,
) -> anyhow::Result<()> {
    apply_install_from(ctx, ui, plans, f, elevated, stop, None).await
}

/// A plan from a GitHub release: its trust lines (shown after the plan) and its repository
/// (the `--allow-unreviewed` gate).
struct Unreviewed {
    note: String,
    slug: String,
}

/// Refuses an install from a GitHub release without `--allow-unreviewed` (a dry run shows).
fn unreviewed_gate(slug: &str, f: Flags) -> anyhow::Result<()> {
    if f.dry_run || f.allow_unreviewed {
        return Ok(());
    }
    Err(output::gh_unreviewed_refusal(slug).into())
}

/// [`apply_install`], with the trust lines and gate of a GitHub release when `release` is set.
async fn apply_install_from(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    plans: Vec<crate::install::plan::InstallPlan>,
    f: Flags,
    elevated: bool,
    stop: Stop<'_, impl Interrupts>,
    release: Option<&Unreviewed>,
) -> anyhow::Result<()> {
    print_plans(ctx, ui, &plans, f);
    if let Some(r) = release {
        ui.plan(&r.note);
    }
    let refusal = admin_gate(plans_admin_refusal(elevated, &plans), f)?;
    third_party_gate(&plans_code(plans.iter(), &*ctx.runners.ast), f)?;
    if let Some(r) = release {
        unreviewed_gate(&r.slug, f)?;
    }
    if f.dry_run {
        dry_run_done(ui, refusal.as_deref());
        return Ok(());
    }
    if !proceed(ui, f)? {
        emit::out(&output::cancelled());
        return Ok(());
    }
    run_plans(ctx, ui, &plans, f, stop).await
}

/// Applies confirmed `plans` and says what was installed.
async fn run_plans(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    plans: &[crate::install::plan::InstallPlan],
    f: Flags,
    mut stop: Stop<'_, impl Interrupts>,
) -> anyhow::Result<()> {
    // Held from before `job.start` until after `job.end`: the log always gets its end.
    stop.interrupts.hold();
    let log = JobLog::start(JobInfo::for_plans("install", plans));
    let applied = stop.run(ui, ops::apply_with(ctx, plans, &f.allow())).await;
    ui.end_progress();
    let Some(applied) = applied else {
        let r = cancelled(log);
        stop.interrupts.release();
        return r;
    };
    finish_log(log, &applied, "instalado");
    stop.interrupts.release();
    applied?;
    for p in plans {
        emit::out(&output::success(
            "Instalado:",
            &output::app_version(&p.app, &p.version),
        ));
        print_terminal_hint(p);
    }
    Ok(())
}

// ---- GitHub releases (Task 210) ----

/// The release being installed and where it goes.
struct Rel<'a> {
    t: &'a GhTarget,
    env: &'a GhEnv<'a>,
    release: &'a crate::github::Release,
    app: String,
    bucket: String,
    version: String,
}

/// An installer asset, downloaded and verified.
struct InstallerAsset {
    asset: Asset,
    generator: &'static str,
    blob: std::path::PathBuf,
    asks_admin: bool,
}

/// The assets an install uses: the installer (the default) and the program (alone, or the
/// installer's fallback).
struct Selection {
    installer: Option<InstallerAsset>,
    program: Option<Asset>,
}

/// What one candidate asset is.
enum AssetKind {
    Installer(InstallerAsset),
    Program(Asset),
    /// MSI/MSIX/AppX: the reason it is out of scope.
    Package(Asset, String),
}

impl AssetKind {
    fn asset(&self) -> &Asset {
        match self {
            AssetKind::Installer(i) => &i.asset,
            AssetKind::Program(a) | AssetKind::Package(a, _) => a,
        }
    }

    fn detail(&self) -> String {
        match self {
            AssetKind::Installer(i) => format!("instalador {}", i.generator),
            AssetKind::Program(_) => "programa".into(),
            AssetKind::Package(_, why) => why.clone(),
        }
    }

    fn into_selection(self) -> anyhow::Result<Selection> {
        match self {
            AssetKind::Installer(i) => Ok(Selection {
                installer: Some(i),
                program: None,
            }),
            AssetKind::Program(a) => Ok(Selection {
                installer: None,
                program: Some(a),
            }),
            AssetKind::Package(_, why) => bail!("{why}"),
        }
    }
}

/// `install gh:<owner>/<repo>[@<tag>]`: a bucket manifest of the same repository first; else
/// the release, with its installer by default (the program as its fallback) or its portable
/// program through the Scoop planner.
#[allow(clippy::too_many_arguments)]
async fn install_github(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    t: &GhTarget,
    asset: Option<&str>,
    scope: Option<crate::bridge::worker::ScopeArg>,
    f: Flags,
    stop: Stop<'_, impl Interrupts>,
    winget: &impl WingetFlows,
) -> anyhow::Result<()> {
    use crate::github::install as gh;
    if scope == Some(crate::bridge::worker::ScopeArg::Machine) {
        return Err(output::Advice::new(
            "releases do GitHub instalam só para o usuário; o instalador pode pedir administrador por conta própria",
            "rode de novo sem --scope machine",
        )
        .into());
    }
    let elevated = elevated_or_unknown(winget.elevated());
    let mut found = gh::bucket_manifests(ctx.catalog, &t.repo)?;
    // An explicit tag or asset asks for the release itself.
    if let (Some((bucket, app)), true) = (found.first(), t.tag.is_some() || asset.is_some()) {
        ui.output(&output::gh_bucket_skipped(&format!("{bucket}/{app}")));
        found.clear();
    }
    if !found.is_empty() {
        match choose_bucket(ui, &found, f)? {
            Some(i) if i < found.len() => {
                let (bucket, app) = &found[i];
                let Resolution::One(cand) = ops::resolve_installed(ctx, app, bucket)? else {
                    bail!("{bucket}/{app} não está mais no catálogo");
                };
                let plans = ops::plan_for(ctx, &cand, false)?;
                return apply_install(ctx, ui, plans, f, elevated, stop).await;
            }
            Some(_) => {}
            None => {
                emit::out(&output::cancelled());
                return Ok(());
            }
        }
    }
    let Some(env) = winget.github() else {
        bail!("instalação por release do GitHub indisponível neste fluxo");
    };
    let app = gh::app_name(&t.repo)?;
    let bucket = t.bucket();
    let state = ops::read_state(&ctx.data.install_layout())?;
    if let Some((r, _)) = state.get(&app)? {
        if r.bucket != bucket {
            bail!("já existe um app {app} de {}", r.bucket);
        }
        bail!("{app} {} já está instalado de {bucket}", r.version);
    }
    drop(state);
    let release = match &t.tag {
        Some(tag) => crate::github::by_tag(ctx.http, &env.api, &t.repo, tag).await?,
        None => crate::github::latest(ctx.http, &env.api, &t.repo).await?,
    };
    let version = gh::version_of(&release.tag)?;
    let rel = Rel {
        t,
        env: &env,
        release: &release,
        app,
        bucket,
        version,
    };
    let selection = select_assets(ctx, ui, &rel, asset, f).await;
    ui.end_progress();
    let Some(selection) = selection? else {
        emit::out(&output::cancelled());
        return Ok(());
    };
    match (selection.installer, selection.program) {
        (Some(inst), program) => {
            install_release_installer(ctx, ui, &rel, inst, program, f, elevated, stop).await
        }
        (None, Some(program)) => {
            let plan = release_plan(ctx, &rel, &program).await;
            ui.end_progress();
            let plan = plan?;
            let mut note = output::gh_trust(&rel.t.slug(), rel.release.immutable);
            if f.dry_run {
                note.push_str(&release_details(&rel, &[&program]));
            }
            let unreviewed = Unreviewed {
                note,
                slug: rel.t.slug(),
            };
            apply_install_from(ctx, ui, vec![plan], f, elevated, stop, Some(&unreviewed)).await
        }
        (None, None) => bail!("nenhum arquivo do release para instalar"),
    }
}

/// The bucket manifests that download from the repository, or the release itself: `Some(i)`
/// with `i == found.len()` for the release. `--yes` or no terminal take the first bucket.
fn choose_bucket(
    ui: &dyn Ui,
    found: &[(String, String)],
    f: Flags,
) -> anyhow::Result<Option<usize>> {
    if f.yes || !ui.interactive() {
        return Ok(Some(0));
    }
    let mut items: Vec<PickerItem> = found
        .iter()
        .map(|(b, app)| PickerItem {
            label: format!("manifesto do bucket {b}/{app} (revisado)"),
            detail: String::new(),
            selectable: true,
            note: None,
        })
        .collect();
    items.push(PickerItem {
        label: "release direto (origem não revisada)".into(),
        detail: String::new(),
        selectable: true,
        note: None,
    });
    ui.pick(items)
}

/// `--dry-run` details of `assets`: file, URL and digest.
fn release_details(rel: &Rel<'_>, assets: &[&Asset]) -> String {
    use crate::github::install as gh;
    let rows: Vec<(String, String, String)> = assets
        .iter()
        .map(|a| {
            (
                a.name.clone(),
                gh::download_url(&rel.env.web, a),
                gh::digest(a),
            )
        })
        .collect();
    let refs: Vec<(&str, &str, &str)> = rows
        .iter()
        .map(|(n, u, d)| (n.as_str(), u.as_str(), d.as_str()))
        .collect();
    output::gh_details(&refs)
}

/// Downloads `asset` into the cache and checks it against the API's digest.
async fn fetch_asset(
    ctx: &OpsCtx<'_>,
    rel: &Rel<'_>,
    asset: &Asset,
) -> anyhow::Result<std::path::PathBuf> {
    use crate::github::install as gh;
    use crate::install::manifest::{HashAlgo, HashSpec, Source};
    let source = Source {
        url: gh::download_url(&rel.env.web, asset),
        file_name: gh::safe_name(asset)?.to_string(),
        hash: HashSpec {
            algo: HashAlgo::Sha256,
            hex: hex::encode(asset.sha256),
        },
        extract_dir: None,
        extract_to: None,
    };
    let mut paths =
        crate::install::download::fetch_all(ctx.http, ctx.data, &[source], ctx.progress).await?;
    paths
        .pop()
        .ok_or_else(|| anyhow::anyhow!("{}: download sem arquivo", asset.name))
}

/// What `asset` is: a package out of scope, an installer or a program (`.exe` assets are
/// downloaded and classified).
async fn classify_asset(
    ctx: &OpsCtx<'_>,
    rel: &Rel<'_>,
    asset: Asset,
) -> anyhow::Result<AssetKind> {
    use crate::github::install as gh;
    use crate::install::sniff::PeKind;
    if let Some(why) = crate::github::pick::unsupported(&asset.name) {
        return Ok(AssetKind::Package(asset, why));
    }
    if !asset.name.to_ascii_lowercase().ends_with(".exe") {
        return Ok(AssetKind::Program(asset));
    }
    let blob = fetch_asset(ctx, rel, &asset).await?;
    let bytes = std::fs::read(&blob)?;
    Ok(match gh::exe_kind(&bytes)? {
        PeKind::Program => AssetKind::Program(asset),
        PeKind::Installer(generator) => AssetKind::Installer(InstallerAsset {
            asks_admin: gh::asks_admin(&bytes),
            asset,
            generator,
            blob,
        }),
    })
}

/// Words that name an installer asset (only to order the candidates).
const INSTALLER_NAMES: [&str; 3] = ["instalador", "installer", "setup"];

/// The assets to install: `--asset` by exact name, else the pick for this host. One installer
/// with at most one program is taken as is (installer first); anything else ambiguous goes to
/// the user, or fails listing the names without a terminal or with `--yes`. `None` = the user
/// cancelled.
async fn select_assets(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    rel: &Rel<'_>,
    wanted: Option<&str>,
    f: Flags,
) -> anyhow::Result<Option<Selection>> {
    use crate::github::pick::{Pick, pick};
    let tag = &rel.release.tag;
    let names = || -> Vec<&str> { rel.release.assets.iter().map(|a| a.name.as_str()).collect() };
    let candidates = match wanted {
        Some(name) => match rel.release.assets.iter().find(|a| a.name == name) {
            Some(a) => vec![a.clone()],
            None if rel.release.no_digest.iter().any(|n| n == name) => {
                bail!("{}", output::gh_no_digest(name));
            }
            None => return Err(output::gh_missing_asset(tag, name, &names()).into()),
        },
        None => match pick(&rel.release.assets, crate::install::manifest::Arch::host()) {
            Pick::One(a) => vec![a],
            Pick::Choose(list) => list,
            Pick::None(why) => bail!("{why}"),
        },
    };
    // Two candidates, one `.exe` named as an installer: the name only orders; that one is
    // downloaded and confirmed by `classify_pe`, and the other is its fallback (looked into by
    // `inspect` if it ever runs). A name that lied falls through to classifying both.
    if let [x, y] = candidates.as_slice() {
        let named = |a: &Asset| {
            let n = a.name.to_ascii_lowercase();
            n.ends_with(".exe") && INSTALLER_NAMES.iter().any(|w| n.contains(w))
        };
        let pair = match (named(x), named(y)) {
            (true, false) => Some((x, y)),
            (false, true) => Some((y, x)),
            _ => None,
        };
        if let Some((setup, other)) = pair
            && crate::github::pick::unsupported(&other.name).is_none()
            && let AssetKind::Installer(i) = classify_asset(ctx, rel, setup.clone()).await?
        {
            return Ok(Some(Selection {
                installer: Some(i),
                program: Some(other.clone()),
            }));
        }
    }
    let mut kinds = Vec::with_capacity(candidates.len());
    for a in candidates {
        kinds.push(classify_asset(ctx, rel, a).await?);
    }
    if kinds.len() == 1 {
        return kinds.pop().map(AssetKind::into_selection).transpose();
    }
    let installers = kinds
        .iter()
        .filter(|k| matches!(k, AssetKind::Installer(_)))
        .count();
    let programs = kinds
        .iter()
        .filter(|k| matches!(k, AssetKind::Program(_)))
        .count();
    if installers == 1 && programs <= 1 {
        let mut sel = Selection {
            installer: None,
            program: None,
        };
        for k in kinds {
            match k {
                AssetKind::Installer(i) => sel.installer = Some(i),
                AssetKind::Program(a) => sel.program = Some(a),
                AssetKind::Package(..) => {}
            }
        }
        return Ok(Some(sel));
    }
    if f.yes || !ui.interactive() {
        let listed: Vec<&str> = kinds.iter().map(|k| k.asset().name.as_str()).collect();
        return Err(output::gh_ambiguous(tag, &listed).into());
    }
    ui.end_progress();
    let items = kinds
        .iter()
        .map(|k| PickerItem {
            label: k.asset().name.clone(),
            detail: k.detail(),
            selectable: true,
            note: None,
        })
        .collect();
    let Some(i) = ui.pick(items)? else {
        return Ok(None);
    };
    if i >= kinds.len() {
        return Ok(None);
    }
    kinds.swap_remove(i).into_selection().map(Some)
}

/// The Scoop plan of a portable `asset`: downloaded, verified, looked into, then the
/// synthetic manifest through `plan_for_raw`.
async fn release_plan(
    ctx: &OpsCtx<'_>,
    rel: &Rel<'_>,
    asset: &Asset,
) -> anyhow::Result<crate::install::plan::InstallPlan> {
    use crate::github::install as gh;
    let blob = fetch_asset(ctx, rel, asset).await?;
    let portable = {
        let scratch = gh::Scratch::new(&ctx.data.tmp_dir(), "inspect")?;
        gh::inspect(&blob, gh::safe_name(asset)?, scratch.path())?
    };
    let url = gh::download_url(&rel.env.web, asset);
    let raw = gh::manifest_json(&url, asset, &portable, &rel.version);
    Ok(ops::plan_for_raw(ctx, &rel.app, &rel.bucket, &raw, false)?)
}

/// Runs `f` on this thread, letting a multi-thread runtime move its other tasks away.
fn blocking<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// The installer of a release: plan, the third-party block of `installer.file` (the same
/// text), gates, confirmation, the run under `install.lock` and its record; when it fails, the
/// program asset through the Scoop plan (no new question). `--allow-unreviewed` is the consent
/// for this third-party code: unlike a bucket's `installer.file`, no `--allow-scripts`.
#[allow(clippy::too_many_arguments)]
async fn install_release_installer(
    ctx: &OpsCtx<'_>,
    ui: &dyn Ui,
    rel: &Rel<'_>,
    inst: InstallerAsset,
    program: Option<Asset>,
    f: Flags,
    elevated: bool,
    stop: Stop<'_, impl Interrupts>,
) -> anyhow::Result<()> {
    use crate::github::install::{self as gh, InstallerFailure, REBOOT_CODE};
    ui.plan(&output::gh_installer_plan(
        &format!("{} {}", rel.app, rel.version),
        &rel.bucket,
        inst.generator,
        &inst.asset.name,
        program.as_ref().map(|p| p.name.as_str()),
        inst.asks_admin,
    ));
    let args: Vec<String> = gh::silent_args(inst.generator)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let code = [ThirdParty::Program {
        uninstaller: false,
        file: inst.asset.name.clone(),
        args,
    }];
    // The same block the Scoop path shows for an `installer.file`.
    ui.plan(&output::third_party_block(&code));
    let mut note = output::gh_trust(&rel.t.slug(), rel.release.immutable);
    if f.dry_run {
        let mut shown = vec![&inst.asset];
        shown.extend(program.as_ref());
        note.push_str(&release_details(rel, &shown));
    }
    ui.plan(&note);
    let refusal = admin_gate(admin_code_refusal(elevated, &code, false), f)?;
    unreviewed_gate(&rel.t.slug(), f)?;
    if f.dry_run {
        dry_run_done(ui, refusal.as_deref());
        return Ok(());
    }
    if !proceed(ui, f)? {
        emit::out(&output::cancelled());
        return Ok(());
    }
    stop.interrupts.hold();
    let ran = run_release_installer(ctx, rel, &inst).await;
    ui.end_progress();
    stop.interrupts.release();
    let failure = match ran? {
        Ok(done) => {
            emit::out(&output::success(
                "Instalado:",
                &output::app_version(&rel.app, &rel.version),
            ));
            if done.arp.is_none() {
                ui.output(&output::gh_not_registered());
            }
            if done.code == REBOOT_CODE {
                emit::out(&output::reboot(false));
            }
            return Ok(());
        }
        Err(InstallerFailure::Cancelled) => {
            emit::out(&output::cancelled());
            return Ok(());
        }
        Err(e) => e,
    };
    let Some(program) = program else {
        bail!("o instalador falhou: {failure}");
    };
    ui.output(&output::gh_fallback(&failure.to_string()));
    let fallback = async {
        let plan = release_plan(ctx, rel, &program).await;
        ui.end_progress();
        let plans = vec![plan?];
        print_plans(ctx, ui, &plans, f);
        run_plans(ctx, ui, &plans, f, stop).await
    };
    anyhow::Context::with_context(fallback.await, || {
        format!("o instalador falhou ({failure}) e a instalação só do executável também falhou")
    })
}

type InstallerRan =
    Result<crate::github::install::Installed, crate::github::install::InstallerFailure>;

/// The installer's run under `install.lock`, from a verified copy named as the asset, with its
/// job log; a run that installed is recorded. The inner error is the installer's failure.
async fn run_release_installer(
    ctx: &OpsCtx<'_>,
    rel: &Rel<'_>,
    inst: &InstallerAsset,
) -> anyhow::Result<InstallerRan> {
    use crate::github::install::{self as gh, InstallerFailure, InstallerRun};
    (ctx.progress)(&ProgressEvent::Phase(
        crate::install::progress::Phase::Preparing,
    ));
    let layout = ctx.data.install_layout();
    let _lock = crate::winget_install::ops::lock_install(&layout, ctx.wait)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let scratch = gh::Scratch::new(&ctx.data.tmp_dir(), "run")?;
    let program = gh::launch_copy(&inst.blob, &inst.asset, scratch.path())?;
    let args = gh::silent_args(inst.generator);
    let mut log = JobLog::start(JobInfo {
        kind: "install",
        tool: "wingetrs",
        target: format!("{}/{}", rel.bucket, rel.app),
        name: rel.app.clone(),
        from: None,
        to: Some(rel.version.clone()),
        cmd: Some(format!("{} {args}", inst.asset.name).trim_end().to_string()),
    });
    let run = InstallerRun {
        program: &program,
        generator: inst.generator,
        app: &rel.app,
        version: &rel.version,
        asks_admin: inst.asks_admin,
    };
    let ran = blocking(|| gh::run_installer(rel.env, &ctx.launch, ctx.progress, &run));
    drop(scratch);
    match &ran {
        Ok(done) => {
            log.code = Some(done.code);
            let url = gh::download_url(&rel.env.web, &inst.asset);
            let manifest = gh::installer_manifest_json(&url, &inst.asset, &rel.version);
            let recorded = gh::record_installer(
                &layout,
                &rel.app,
                &rel.bucket,
                &rel.version,
                &manifest,
                done,
            );
            finish_log(log, &recorded, "instalado");
            recorded?;
        }
        Err(InstallerFailure::Cancelled) => log.finish(Outcome::Cancelled, "cancelado"),
        Err(e) => {
            if let InstallerFailure::Code(c) = e {
                log.code = Some(*c);
            }
            log.finish(Outcome::Failed, &e.to_string());
        }
    }
    Ok(ran)
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
    emit::out_block(&output::uninstall_plan_text(
        &plan,
        &ctx.data.install_layout(),
        ui.glyphs(),
    ));
    let (manifest, shown_sha) = ops::uninstall_manifest(ctx, &plan)?;
    let layout = ctx.data.install_layout();
    let vdir = layout.version_dir(&plan.app, &plan.version);
    let roots = CallRoots::new(layout.root(), Some(&vdir));
    let code: Vec<_> = manifest
        .map(|m| output::removal_third_party(&m, &vdir, "", &roots, &*ctx.runners.ast))
        .filter(|items| !items.is_empty())
        .map(|items| (plan.app.clone(), items))
        .into_iter()
        .collect();
    let items: Vec<ThirdParty> = code.iter().flat_map(|(_, i)| i.iter().cloned()).collect();
    let elevated = elevated_or_unknown(winget.elevated());
    let refusal = admin_gate(admin_code_refusal(elevated, &items, false), f)?;
    third_party_gate(&code, f)?;
    if f.dry_run {
        dry_run_done(ui, refusal.as_deref());
        return Ok(());
    }
    if !proceed(ui, f)? {
        emit::out(&output::cancelled());
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
    emit::out(&output::success(
        "Desinstalado:",
        &output::app_version(&plan.app, &plan.version),
    ));
    for note in &removed.notes {
        emit::err_with(|| output::warning(note));
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
        emit::out_block(&output::residues_left(residues));
        return Ok(());
    };
    if selected.is_empty() {
        emit::out("Nenhum resíduo removido.");
        return Ok(());
    }
    let backups = ctx.data.install_layout().backups();
    let results = residue::clean(&selected, app, &ctx.effects, &backups);
    for (r, res) in selected.iter().zip(results) {
        match res {
            Ok(()) => emit::out(&output::success("Removido:", &output::residue_label(r))),
            Err(e) => emit::err_with(|| output::warning(&e.to_string())),
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
                return Err(output::Advice::new(
                    format!("{t} está com a versão fixada"),
                    "desafixe na TUI (menu de ações)",
                )
                .into());
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
                emit::out(&output::up_to_date(t));
                return Ok(());
            }
            found
        }
        None => {
            for (app, reason) in &list.skipped {
                emit::err_with(|| output::warning(&format!("{app}: {reason}")));
            }
            if !all {
                emit::out_block(&output::upgrades_listing(&list.upgradable, &pinned));
                return Ok(());
            }
            for app in &pinned {
                emit::out(&output::pinned_skipped(app));
            }
            list.upgradable
        }
    };
    if chosen.is_empty() {
        emit::out(output::NOTHING_TO_UPGRADE);
        return Ok(());
    }
    let mut groups = Vec::new();
    for (app, _, _) in &chosen {
        let Some((rec, _)) = state.get(app)? else {
            bail!("{app} não está instalado pelo wingetrs");
        };
        // A winget portable package: its newer manifest is the winget catalog's (Task 189).
        if rec.bucket == crate::winget_install::portable::WINGET_BUCKET {
            groups.push(vec![winget.plan_portable_upgrade(app, keep).await?]);
            continue;
        }
        let cand = match ops::resolve_installed(ctx, app, &rec.bucket)? {
            Resolution::One(c) => c,
            _ => bail!("{app} não foi encontrado no bucket {}", rec.bucket),
        };
        groups.push(ops::plan_for(ctx, &cand, keep)?);
    }
    let elevated = elevated_or_unknown(winget.elevated());
    let split = admin_split(groups, elevated, f);
    let groups = split.groups;
    for g in &groups {
        print_plans(ctx, ui, g, f);
    }
    if !f.dry_run {
        for w in &split.warnings {
            emit::err_with(|| output::warning(w));
        }
    }
    if let Some(r) = split.fail {
        return Err(anyhow::Error::msg(r));
    }
    third_party_gate(&plans_code(groups.iter().flatten(), &*ctx.runners.ast), f)?;
    if f.dry_run {
        for w in &split.warnings {
            ui.output(&output::warning(w));
        }
        emit::out(output::DRY_RUN_DONE);
        return Ok(());
    }
    if !proceed(ui, f)? {
        emit::out(&output::cancelled());
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
        emit::out(&output::success(
            "Atualizado:",
            &output::app_version(&p.app, &p.version),
        ));
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
