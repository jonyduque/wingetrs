//! Plan executor: applies an [`InstallPlan`] step by step, journaling an undo action for
//! every change, and rolls everything back in reverse order on any failure. `current`
//! only moves at `LinkCurrent`, so the previous version keeps working until then.
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use tracing::Instrument;

use crate::cache::DataDir;
use crate::fs_retry;
use crate::http::Http;
use crate::install::download::{self, Progress};
use crate::install::installer_args;
use crate::install::layout::Layout;
use crate::install::manifest::{self, Arch, HookKind, Source};
use crate::install::msi_queue::{self, Attempt};
use crate::install::plan::{
    InstallPlan, PersistKind, PersistSpec, ShimSpec, ShortcutSpec, Step, UninstallPlan,
};
use crate::install::prelude::{CONTRACT_VARS, PRELUDE};
use crate::install::progress::{Phase, ProgressEvent};
use crate::install::runner::{
    RunnerInput, ScriptBody, ScriptContext, ScriptOp, VarValue, runner_ps1, script_vars,
};
use crate::install::script_effects::{self, ScriptEffect};
use crate::install::state::{AppRecord, Artifact, ArtifactKind, StateDb};
use crate::install::traits::{EffectError, Effects, EnvStore, EnvValue};
use crate::install::{extract, inno, link, path_edit, planner, safepath, script_scan, shim, state};

pub trait StepHook: Send + Sync {
    fn before(&self, index: usize, step: &Step) -> Result<(), ExecError>;
}

/// Production hook: never interferes.
#[derive(Debug, Clone, Copy)]
pub struct NoHook;
impl StepHook for NoHook {
    fn before(&self, _index: usize, _step: &Step) -> Result<(), ExecError> {
        Ok(())
    }
}

/// Files under a directory that another process holds open.
pub type InUseCheck<'a> = &'a (dyn Fn(&Path) -> Vec<PathBuf> + Send + Sync);

/// Extraction entry point, `extract::extract` in production (file, file name, extract_dir,
/// extract_to, destination). Tests inject one that panics.
pub type ExtractFn =
    fn(&Path, &str, Option<&str>, Option<&str>, &Path) -> Result<(), extract::ExtractError>;

/// The `msiexec /a` fallback for an MSI the own extractor does not support, injected like the
/// other effects. Production: `msiexec_win::{installer_busy, msiexec_admin}`.
#[derive(Debug, Clone, Copy)]
pub struct Msiexec {
    /// Whether another Windows Installer run holds its mutex.
    pub busy: fn() -> bool,
    /// `(msi, target_dir, log)`: runs `msiexec /a` and waits for it.
    pub launch: fn(&Path, &Path, &Path) -> io::Result<Attempt>,
}

fn never_busy() -> bool {
    false
}

fn refuse_launch(_: &Path, _: &Path, _: &Path) -> io::Result<Attempt> {
    Err(io::Error::other("msiexec desativado neste ambiente"))
}

impl Msiexec {
    /// Never runs `msiexec` (non-Windows, tests, `WINGETRS_TEST_ENV_KEY`).
    pub const DISABLED: Msiexec = Msiexec {
        busy: never_busy,
        launch: refuse_launch,
    };
}

/// `(exe, args, dir)` → exit code and decoded output lines.
pub type ToolRun = fn(&Path, &[String], &Path) -> io::Result<(i32, Vec<String>)>;

/// One manifest hook to run: `runner` (inside `workdir`) defines the Scoop variables,
/// dot-sources the prelude and runs the hook; `env` is added to the process environment.
#[derive(Debug, Clone)]
pub struct ScriptJob {
    pub app: String,
    pub hook: HookKind,
    pub workdir: PathBuf,
    pub runner: PathBuf,
    pub env: Vec<(OsString, OsString)>,
}

/// Exit code and decoded output lines of a hook.
#[derive(Debug, Clone)]
pub struct ScriptOutcome {
    pub code: i32,
    pub lines: Vec<String>,
}

/// Runs manifest scripts (third-party code). Tests inject a fake; nothing runs PowerShell
/// without the user's consent.
pub trait ScriptHost: Send + Sync {
    fn run(&self, job: &ScriptJob) -> io::Result<ScriptOutcome>;
}

/// External tools the executor runs, injected like [`Msiexec`]. Production: `inno_win::run`.
#[derive(Clone)]
pub struct Runners {
    pub innounp: ToolRun,
    /// Runs `installer.file`/`uninstaller.file` (third-party code). `None` = the user gave no
    /// consent: a plan with a `RunInstaller` fails before any step.
    pub installer: Option<ToolRun>,
    /// Runs manifest hooks (third-party code). `None` = the user gave no consent: a plan with
    /// a `Step::Hook` fails before any step.
    pub scripts: Option<Arc<dyn ScriptHost>>,
}

impl std::fmt::Debug for Runners {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runners")
            .field("innounp", &self.innounp)
            .field("installer", &self.installer)
            .field("scripts", &self.scripts.as_ref().map(|_| "ScriptHost"))
            .finish()
    }
}

fn refuse_innounp(_: &Path, _: &[String], _: &Path) -> io::Result<(i32, Vec<String>)> {
    Err(io::Error::other("innounp desativado neste ambiente"))
}

impl Runners {
    /// Runs nothing (non-Windows, tests, `WINGETRS_TEST_ENV_KEY`).
    pub fn disabled() -> Runners {
        Runners {
            innounp: refuse_innounp,
            installer: None,
            scripts: None,
        }
    }
}

pub struct ExecCtx<'a> {
    pub layout: &'a Layout,
    pub data: &'a DataDir,
    pub http: &'a Http,
    pub effects: Effects<'a>,
    pub state: &'a mut StateDb,
    pub hook: &'a dyn StepHook,
    pub progress: Progress<'a>,
    /// Production: `fs_win::files_in_use`; tests pass a closure.
    pub in_use: InUseCheck<'a>,
    /// `Some` when this install may fall back to `msiexec /a` (authorized by the user).
    pub msiexec: Option<Msiexec>,
    /// Who came first: the launch of `msiexec` or a cancellation (see [`LaunchGate`]).
    pub launch: LaunchGate,
    pub runners: Runners,
}

/// The one decision between launching `msiexec` and cancelling the install: whichever
/// comes first wins, the other one is refused. Free at the start of each plan; once
/// `msiexec` launched it stays launched (a 1618 relaunch included) until the next plan.
/// Clones share the decision (the CLI keeps one, the executor and its worker others).
#[derive(Debug, Clone, Default)]
pub struct LaunchGate(Arc<AtomicU8>);

const GATE_FREE: u8 = 0;
const GATE_LAUNCHED: u8 = 1;
const GATE_CANCELLED: u8 = 2;

impl LaunchGate {
    /// Taken by the worker right before each launch: `false` = the install was cancelled.
    pub fn try_launch(&self) -> bool {
        match self.cas(GATE_FREE, GATE_LAUNCHED) {
            Ok(_) => true,
            Err(now) => now == GATE_LAUNCHED,
        }
    }

    /// Asked on Ctrl+C (and by a dropped install): `false` = `msiexec` already launched, the
    /// install must run to its end.
    pub fn try_cancel(&self) -> bool {
        match self.cas(GATE_FREE, GATE_CANCELLED) {
            Ok(_) => true,
            Err(now) => now == GATE_CANCELLED,
        }
    }

    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst) == GATE_CANCELLED
    }

    /// Each plan starts free: Ctrl+C cancels again until its own `msiexec` launches. (A
    /// cancelled install is dropped at once, so no plan of it starts afterwards.)
    pub fn next_plan(&self) {
        self.0.store(GATE_FREE, Ordering::SeqCst);
    }

    fn cas(&self, from: u8, to: u8) -> Result<u8, u8> {
        self.0
            .compare_exchange(from, to, Ordering::SeqCst, Ordering::SeqCst)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("falha no passo {index} ({step}): {message}; desfeito: {undone:?}")]
    Failed {
        index: usize,
        step: String,
        message: String,
        undone: Vec<String>,
    },
    #[error("arquivo em uso: {0} — feche o programa e tente de novo")]
    InUse(String),
    #[error("{0}")]
    Hook(String),
    /// The own extractor refused an MSI that only the Windows Installer can unpack; the install
    /// was rolled back and needs the user's authorization for `msiexec /a`.
    #[error(
        "{app}: este MSI precisa do Windows Installer ({reason}); rode de novo com --allow-msiexec. O msiexec /a não pode ser cancelado depois de começar e grava arquivos em C:\\Windows\\Installer."
    )]
    NeedsMsiexec {
        app: String,
        source_index: usize,
        reason: String,
    },
    /// The plan (or the installed app) runs third-party code and no consent was given.
    #[error("{app}: precisa da sua confirmação para rodar {what} (fase 2c, Task 161)")]
    NeedsScriptConsent { app: String, what: String },
}

/// User environment variable holding the PATH.
pub(crate) const PATH_VAR: &str = "Path";

/// One change to revert, in the order it was made.
enum Undo {
    /// A file, directory tree or link this run created.
    RemovePath(PathBuf),
    /// `original` was renamed to `aside` to make room: rename it back.
    RestoreAside {
        aside: PathBuf,
        original: PathBuf,
    },
    /// A directory this run created; removed only if empty again.
    RemoveEmptyDir(PathBuf),
    /// `current` before `LinkCurrent` (`None` = there was none).
    RestoreCurrent {
        link: PathBuf,
        previous: Option<PathBuf>,
    },
    /// Contents of `shims\<name>.*` before the shim step.
    RestoreShim {
        name: String,
        saved: Vec<(PathBuf, Vec<u8>)>,
    },
    RemoveShortcut(PathBuf),
    /// The shortcut belonged to the previous install of this app: put it back.
    RecreateShortcut(ShortcutSpec),
    RestoreEnv {
        name: String,
        previous: Option<EnvValue>,
    },
    /// The PATH entry was added by this run.
    PathRemove {
        entry: String,
        var_existed: bool,
    },
}

fn io_msg(p: &Path, e: io::Error) -> String {
    format!("{}: {e}", p.display())
}

fn not_found_ok(r: io::Result<()>) -> io::Result<()> {
    match r {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Removes a file, a link (never following it) or a whole directory tree.
fn remove_any(p: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(p) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() {
        return link::remove(p).or_else(|_| fs_retry::retry("apagar", p, || fs::remove_file(p)));
    }
    if meta.is_dir() {
        fs_retry::retry("apagar", p, || fs::remove_dir_all(p))
    } else {
        fs_retry::retry("apagar", p, || fs::remove_file(p))
    }
}

/// `previous = None` removes the variable.
fn restore_env(
    env: &dyn EnvStore,
    name: &str,
    previous: Option<&EnvValue>,
) -> Result<(), EffectError> {
    match previous {
        Some(v) => env.set(name, v),
        None => env.remove(name),
    }
}

fn path_remove_entry(
    env: &dyn EnvStore,
    entry: &str,
    var_existed: bool,
) -> Result<(), EffectError> {
    let Some(cur) = env.get(PATH_VAR)? else {
        return Ok(());
    };
    let value = path_edit::path_remove(&cur.value, entry);
    if value.is_empty() && !var_existed {
        return env.remove(PATH_VAR);
    }
    env.set(
        PATH_VAR,
        &EnvValue {
            value,
            expand: cur.expand,
        },
    )
}

/// A value that becomes a single directory name (app, version) must be one safe component.
fn check_component(what: &str, v: &str) -> Result<(), String> {
    match safepath::safe_components(v) {
        Some(c) if c.len() == 1 && c[0] == v => Ok(()),
        _ => Err(format!("nome inseguro de {what}: {v}")),
    }
}

/// Stable short name of a step, for the `install.step` span.
fn step_kind(step: &Step) -> &'static str {
    match step {
        Step::EnsureLauncher => "launcher",
        Step::Download { .. } => "download",
        Step::Extract { .. } => "extract",
        Step::ExtractInno { .. } => "extract_inno",
        Step::Promote => "promote",
        Step::RunInstaller { .. } => "installer",
        Step::Hook { .. } => "hook",
        Step::Persist(_) => "persist",
        Step::LinkCurrent => "link",
        Step::Shim(_) => "shim",
        Step::Shortcut(_) => "shortcut",
        Step::PsModule { .. } => "psmodule",
        Step::EnvSet { .. } => "env",
        Step::EnvAddPath { .. } => "path",
        Step::EnsureShimsOnPath => "shims_path",
        Step::WriteManifest { .. } => "manifest",
        Step::RecordState => "state",
        Step::RemoveOldVersion { .. } => "remove_old",
    }
}

/// Phase entered when `step` starts; `None` keeps the current one (a download reports its
/// own bytes, the old version is removed after the record).
fn step_phase(step: &Step, plan: &InstallPlan) -> Option<Phase> {
    match step {
        Step::EnsureLauncher => Some(Phase::Preparing),
        Step::Download { .. } | Step::RemoveOldVersion { .. } => None,
        Step::Extract { index } => plan.steps.iter().find_map(|s| match s {
            Step::Download { index: i, source } if i == index => Some(Phase::Extracting {
                file: source.file_name.clone(),
                engine: crate::install::progress::engine_of(&source.file_name),
            }),
            _ => None,
        }),
        Step::ExtractInno { index } => plan.steps.iter().find_map(|s| match s {
            Step::Download { index: i, source } if i == index => Some(Phase::Extracting {
                file: source.file_name.clone(),
                engine: "Inno Setup".into(),
            }),
            _ => None,
        }),
        Step::Promote | Step::RunInstaller { .. } | Step::Persist(_) => Some(Phase::Placing),
        Step::Hook { kind, .. } => match kind {
            HookKind::PostInstall => Some(Phase::Linking),
            _ => Some(Phase::Placing),
        },
        Step::LinkCurrent
        | Step::Shim(_)
        | Step::Shortcut(_)
        | Step::PsModule { .. }
        | Step::EnvSet { .. }
        | Step::EnvAddPath { .. }
        | Step::EnsureShimsOnPath => Some(Phase::Linking),
        Step::WriteManifest { .. } | Step::RecordState => Some(Phase::Recording),
    }
}

fn step_label(step: &Step) -> String {
    match step {
        Step::EnsureLauncher => "lançador".into(),
        Step::Download { index, .. } => format!("baixar [{}]", index + 1),
        Step::Extract { index } => format!("extrair [{}]", index + 1),
        Step::ExtractInno { index } => format!("extrair [{}] (Inno Setup)", index + 1),
        Step::Promote => "promover versão".into(),
        Step::RunInstaller { file, .. } => format!("rodar instalador {file}"),
        Step::Hook { kind, .. } => format!("script {}", kind.key()),
        Step::Persist(p) => format!("persistir {}", p.in_version.display()),
        Step::LinkCurrent => "apontar current".into(),
        Step::Shim(s) => format!("comando {}", s.name),
        Step::Shortcut(s) => format!("atalho {}", s.link.display()),
        Step::PsModule { name } => format!("módulo PowerShell {name}"),
        Step::EnvSet { name, .. } => format!("variável {name}"),
        Step::EnvAddPath { entry } => format!("PATH {}", entry.display()),
        Step::EnsureShimsOnPath => "shims no PATH".into(),
        Step::WriteManifest { .. } => "gravar manifesto".into(),
        Step::RecordState => "registro".into(),
        Step::RemoveOldVersion { version } => format!("remover versão {version}"),
    }
}

struct Run<'p> {
    plan: &'p InstallPlan,
    previous: Option<(AppRecord, Vec<Artifact>)>,
    downloads: BTreeMap<usize, PathBuf>,
    artifacts: Vec<Artifact>,
    journal: Vec<(String, Undo)>,
    /// Label of the step being run (journal entries are tagged with it).
    label: String,
    extract: ExtractFn,
    /// Panic of the extraction worker: re-raised only after the rollback.
    panic: Option<Box<dyn std::any::Any + Send>>,
    /// `(source index, reason)` of an MSI that needs `msiexec /a`, not authorized.
    needs_msiexec: Option<(usize, String)>,
    /// Last phase reported: a step of the same phase reports nothing.
    phase: Option<Phase>,
}

impl Run<'_> {
    /// Reports `phase` unless it is the one already reported.
    fn enter(&mut self, ctx: &ExecCtx<'_>, phase: Phase) {
        if self.phase.as_ref() != Some(&phase) {
            (ctx.progress)(&ProgressEvent::Phase(phase.clone()));
            self.phase = Some(phase);
        }
    }

    fn push(&mut self, u: Undo) {
        self.journal.push((self.label.clone(), u));
    }

    /// Creates every missing directory down to `dir`, journaling each one.
    fn ensure_dirs(&mut self, dir: &Path) -> Result<(), String> {
        let mut missing = Vec::new();
        let mut cur = Some(dir);
        while let Some(d) = cur {
            if fs::symlink_metadata(d).is_ok() {
                break;
            }
            missing.push(d.to_path_buf());
            cur = d.parent();
        }
        for d in missing.into_iter().rev() {
            match fs::create_dir(&d) {
                Ok(()) => self.push(Undo::RemoveEmptyDir(d)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io_msg(&d, e)),
            }
        }
        Ok(())
    }

    fn previous_artifact(&self, kind: ArtifactKind, target: &str) -> Option<&Artifact> {
        self.previous
            .as_ref()?
            .1
            .iter()
            .find(|a| a.kind == kind && a.target.eq_ignore_ascii_case(target))
    }
}

fn apply_undo(u: Undo, ctx: &mut ExecCtx<'_>) -> Result<(), String> {
    let env = ctx.effects.env;
    match u {
        Undo::RemovePath(p) => remove_any(&p).map_err(|e| io_msg(&p, e)),
        Undo::RemoveEmptyDir(p) => not_found_ok(fs::remove_dir(&p)).map_err(|e| io_msg(&p, e)),
        Undo::RestoreAside { aside, original } => {
            fs_retry::retry("renomear", &aside, || fs::rename(&aside, &original))
                .map_err(|e| io_msg(&original, e))
        }
        Undo::RestoreCurrent { link: l, previous } => {
            not_found_ok(remove_any(&l)).map_err(|e| io_msg(&l, e))?;
            match previous {
                Some(target) => link::create(&target, &l).map_err(|e| io_msg(&l, e)),
                None => Ok(()),
            }
        }
        Undo::RestoreShim { name, saved } => {
            // Keep going after an error: the saved files still have to come back.
            let mut errors = Vec::new();
            if let Err(e) = shim::remove_shim(ctx.layout, &name) {
                errors.push(format!("{e}"));
            }
            for (p, bytes) in saved {
                if let Err(e) = fs::write(&p, bytes) {
                    errors.push(io_msg(&p, e));
                }
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("; "))
            }
        }
        Undo::RemoveShortcut(p) => ctx.effects.shortcuts.remove(&p).map_err(|e| e.to_string()),
        Undo::RecreateShortcut(s) => ctx
            .effects
            .shortcuts
            .create(&s.link, &s.target, s.args.as_deref(), s.icon.as_deref())
            .map_err(|e| e.to_string()),
        Undo::RestoreEnv { name, previous } => {
            restore_env(env, &name, previous.as_ref()).map_err(|e| e.to_string())
        }
        Undo::PathRemove { entry, var_existed } => {
            path_remove_entry(env, &entry, var_existed).map_err(|e| e.to_string())
        }
    }
}

/// Reverts the journal in reverse order; returns the labels of the steps undone.
fn rollback(journal: Vec<(String, Undo)>, ctx: &mut ExecCtx<'_>) -> Vec<String> {
    let mut undone: Vec<String> = Vec::new();
    for (label, u) in journal.into_iter().rev() {
        let entry = match apply_undo(u, ctx) {
            Ok(()) => label,
            Err(e) => {
                tracing::warn!("falha ao desfazer {label}: {e}");
                format!("{label} (falhou: {e})")
            }
        };
        if undone.last() != Some(&entry) {
            undone.push(entry);
        }
    }
    undone
}

/// The record is the last step that can be rolled back: only `RemoveOldVersion` may follow it.
fn check_order(steps: &[Step]) -> Result<(), String> {
    let is_old = |s: &Step| matches!(s, Step::RemoveOldVersion { .. });
    let records: Vec<usize> = steps
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s, Step::RecordState))
        .map(|(i, _)| i)
        .collect();
    match records.as_slice() {
        [r] if steps[r + 1..].iter().all(is_old) && !steps[..*r].iter().any(is_old) => Ok(()),
        _ => Err(
            "plano inválido: o registro deve ser o último passo (só a remoção da versão antiga vem depois)"
                .into(),
        ),
    }
}

/// Applies `plan`; on failure everything done so far is undone in reverse order.
///
/// Precondition: never cancel (drop) this future on an async worker thread. Dropped during an
/// extraction it blocks until the extraction or `msiexec` worker ends; the TUI drives it with
/// `block_on` on a blocking thread.
///
/// Never broadcasts the environment change: the caller calls `effects.env.broadcast()` once
/// after a successful install (and after a failed one whose rollback touched the environment).
pub async fn execute_install(plan: &InstallPlan, ctx: &mut ExecCtx<'_>) -> Result<(), ExecError> {
    execute_install_with(plan, ctx, extract::extract).await
}

/// [`execute_install`] with an injected extraction (tests). Same precondition on dropping it.
#[doc(hidden)]
pub async fn execute_install_with(
    plan: &InstallPlan,
    ctx: &mut ExecCtx<'_>,
    extract: ExtractFn,
) -> Result<(), ExecError> {
    let fail_early = |message: String| ExecError::Failed {
        index: 0,
        step: "validar plano".into(),
        message,
        undone: Vec::new(),
    };
    check_component("app", &plan.app).map_err(fail_early)?;
    check_component("versão", &plan.version).map_err(fail_early)?;
    check_order(&plan.steps).map_err(fail_early)?;
    let needs_consent = plan.third_party_uninstaller
        || plan
            .steps
            .iter()
            .any(|s| matches!(s, Step::RunInstaller { .. }));
    if needs_consent && ctx.runners.installer.is_none() {
        return Err(ExecError::NeedsScriptConsent {
            app: plan.app.clone(),
            what: "o instalador do manifesto".into(),
        });
    }
    let has_hooks = plan.steps.iter().any(|s| matches!(s, Step::Hook { .. }));
    if has_hooks && ctx.runners.scripts.is_none() {
        return Err(ExecError::NeedsScriptConsent {
            app: plan.app.clone(),
            what: "os scripts do manifesto".into(),
        });
    }
    for s in &plan.steps {
        if let Step::RemoveOldVersion { version } = s {
            check_component("versão", version).map_err(fail_early)?;
        }
    }
    let previous = ctx
        .state
        .get(&plan.app)
        .map_err(|e| fail_early(e.to_string()))?;
    ctx.launch.next_plan();
    let mut run = Run {
        plan,
        previous,
        downloads: BTreeMap::new(),
        artifacts: Vec::new(),
        journal: Vec::new(),
        label: String::new(),
        extract,
        panic: None,
        needs_msiexec: None,
        phase: None,
    };
    // Removing the old version runs only after every other step succeeded.
    let (main, cleanup): (Vec<_>, Vec<_>) = plan
        .steps
        .iter()
        .enumerate()
        .partition(|(_, s)| !matches!(s, Step::RemoveOldVersion { .. }));
    for (i, step) in main {
        run.label = step_label(step);
        if let Some(phase) = step_phase(step, plan) {
            run.enter(ctx, phase);
        }
        let result = match ctx.hook.before(i, step) {
            Ok(()) => {
                run_step(&mut run, step, ctx)
                    .instrument(tracing::debug_span!(
                        "install.step",
                        kind = step_kind(step),
                        index = i
                    ))
                    .await
            }
            Err(e) => Err(e.to_string()),
        };
        if let Err(message) = result {
            let journal = std::mem::take(&mut run.journal);
            let undone = rollback(journal, ctx);
            if let Some(p) = run.panic.take() {
                std::panic::resume_unwind(p);
            }
            if let Some((source_index, reason)) = run.needs_msiexec.take() {
                return Err(ExecError::NeedsMsiexec {
                    app: plan.app.clone(),
                    source_index,
                    reason,
                });
            }
            return Err(ExecError::Failed {
                index: i,
                step: run.label,
                message,
                undone,
            });
        }
    }
    // Same policy as removing the old version: the install stands, failures are warnings.
    remove_upgrade_leftovers(&run, ctx);
    remove_replaced_folders(&run, ctx.layout);
    for (i, step) in cleanup {
        let Step::RemoveOldVersion { version } = step else {
            continue;
        };
        let result = ctx
            .hook
            .before(i, step)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                tracing::debug_span!("install.step", kind = step_kind(step), index = i)
                    .in_scope(|| remove_old_version(ctx.layout, &plan.app, &plan.version, version))
            });
        if let Err(e) = result {
            tracing::warn!(
                "{} {} instalado, mas a versão antiga {version} ficou: {e}",
                plan.app,
                plan.version
            );
        }
    }
    Ok(())
}

/// Artifacts of the previous install that this version no longer declares, removed as
/// uninstall would.
fn remove_upgrade_leftovers(run: &Run<'_>, ctx: &mut ExecCtx<'_>) {
    let Some((_, old)) = &run.previous else {
        return;
    };
    for a in old {
        let kept = run
            .artifacts
            .iter()
            .any(|n| n.kind == a.kind && n.target.eq_ignore_ascii_case(&a.target));
        if kept {
            continue;
        }
        // A renamed module leaves its old junction, but the app still needs `PSModulePath`.
        let keeps_module = run
            .artifacts
            .iter()
            .any(|n| n.kind == ArtifactKind::PsModule);
        if let Some((label, Err(e))) = remove_artifact(ctx, &run.plan.app, a, true, keeps_module) {
            tracing::warn!(
                "{}: sobra da versão anterior ({label}) ficou: {e}",
                run.plan.app
            );
        }
    }
}

/// Version folders moved aside by `Promote` (a kept copy of the version being installed),
/// then any `.replaced-*` a crashed run left in this app's folder.
fn remove_replaced_folders(run: &Run<'_>, layout: &Layout) {
    let app = run.plan.app.as_str();
    let warn = |p: &Path, e: &dyn std::fmt::Display| {
        tracing::warn!("{app}: a cópia antiga {} ficou: {e}", p.display());
    };
    for (_, u) in &run.journal {
        if let Undo::RestoreAside { aside, .. } = u
            && let Err(e) = remove_any(aside)
        {
            warn(aside, &e);
        }
    }
    let entries = match fs::read_dir(layout.app_dir(app)) {
        Ok(entries) => entries,
        Err(e) => return warn(&layout.app_dir(app), &e),
    };
    let current = layout.current(app);
    for entry in entries.flatten() {
        let p = entry.path();
        let stale = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(REPLACED_PREFIX));
        if !stale || current_points_to(&current, &p) {
            continue;
        }
        if let Err(e) = remove_any(&p) {
            warn(&p, &e);
        }
    }
}

const REPLACED_PREFIX: &str = ".replaced-";
static ASIDE_SEQ: AtomicU64 = AtomicU64::new(0);
/// Names tried before giving up on a free `.replaced-*` (a crashed run may have left some).
const ASIDE_TRIES: usize = 64;

/// A free `apps\<app>\.replaced-<version>-<pid>-<seq>` path.
fn free_aside(layout: &Layout, app: &str, version: &str) -> Result<PathBuf, String> {
    for _ in 0..ASIDE_TRIES {
        let aside = layout.app_dir(app).join(format!(
            "{REPLACED_PREFIX}{version}-{}-{}",
            std::process::id(),
            ASIDE_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        if fs::symlink_metadata(&aside).is_err() {
            return Ok(aside);
        }
    }
    Err(format!(
        "nenhum nome livre para afastar {}",
        layout.version_dir(app, version).display()
    ))
}

/// True when `current` resolves to `dir` (both normalized; a broken link never matches).
fn current_points_to(current: &Path, dir: &Path) -> bool {
    let Ok(target) = link::target(current) else {
        return false;
    };
    match (fs::canonicalize(&target), fs::canonicalize(dir)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn remove_old_version(layout: &Layout, app: &str, new: &str, old: &str) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    let dir = layout.version_dir(app, old);
    not_found_ok(remove_any(&dir)).map_err(|e| io_msg(&dir, e))
}

/// The source of extraction step `index` and its downloaded file.
fn extract_source<'p>(run: &Run<'p>, index: usize) -> Result<(&'p Source, PathBuf), String> {
    let source = run
        .plan
        .steps
        .iter()
        .find_map(|s| match s {
            Step::Download { index: i, source } if *i == index => Some(source),
            _ => None,
        })
        .ok_or_else(|| format!("fonte [{}] sem download no plano", index + 1))?;
    let file = run
        .downloads
        .get(&index)
        .cloned()
        .ok_or_else(|| format!("fonte [{}] não baixada", index + 1))?;
    Ok((source, file))
}

/// The staging folder, made fresh and journaled by the first extraction step of the run.
fn own_staging(
    run: &mut Run<'_>,
    layout: &Layout,
    app: &str,
    version: &str,
) -> Result<PathBuf, String> {
    let staging = layout.staging_dir(app, version);
    let owned = run
        .journal
        .iter()
        .any(|(_, u)| matches!(u, Undo::RemovePath(p) if *p == staging));
    if !owned {
        // A staging folder left by a crashed run is garbage of ours.
        not_found_ok(remove_any(&staging)).map_err(|e| io_msg(&staging, e))?;
        // So is what a killed run left beside it (`msiexec` work, extraction temps).
        remove_dead_run_leftovers(&layout.app_dir(app));
        run.ensure_dirs(&layout.app_dir(app))?;
        run.push(Undo::RemovePath(staging.clone()));
    }
    Ok(staging)
}

/// Appended to a failed installer: it ran outside our journal.
const INSTALLER_SIDE_EFFECTS: &str =
    "; o instalador pode ter alterado arquivos fora da pasta do app, e isso não foi desfeito";

/// The installer's exit as a step result: a spawn error or a code other than 0 fails.
fn check_installer_exit(r: io::Result<(i32, Vec<String>)>) -> Result<(), String> {
    match r {
        Ok((0, _)) => Ok(()),
        Ok((code, lines)) => {
            let tail = lines.last().map(|l| format!(" ({l})")).unwrap_or_default();
            Err(format!(
                "o instalador saiu com código {code}{tail}{INSTALLER_SIDE_EFFECTS}"
            ))
        }
        Err(e) => Err(format!("não consegui rodar o instalador: {e}")),
    }
}

/// Deleting the installer afterwards is tidiness: a failure is only logged.
fn remove_installer(program: &Path) {
    if let Err(e) = fs::remove_file(program) {
        tracing::warn!("{}: {e}", program.display());
    }
}

/// `<vdir>/<file>` for an existing regular file that stays inside `vdir` once normalized
/// (links followed). `file` came from a manifest: joined only through safepath.
fn installer_program(vdir: &Path, file: &str) -> Result<PathBuf, String> {
    let comps = safepath::safe_components(file)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| format!("caminho inseguro do instalador: {file}"))?;
    let mut program = vdir.to_path_buf();
    program.extend(comps);
    if !program.is_file() {
        return Err(format!("instalador {file} não existe"));
    }
    let real = fs::canonicalize(&program).map_err(|e| io_msg(&program, e))?;
    let root = fs::canonicalize(vdir).map_err(|e| io_msg(vdir, e))?;
    if !real.starts_with(&root) {
        return Err(format!("instalador {file} fica fora da pasta da versão"));
    }
    Ok(program)
}

static SCRIPT_SEQ: AtomicU64 = AtomicU64::new(0);

/// Helpers a hook may call through the prelude (`$__wr_helpers`): Scoop app and its exe.
const SCRIPT_HELPERS: &[(&str, &str)] = &[
    ("7zip", "7z.exe"),
    ("lessmsi", "lessmsi.exe"),
    ("innounp", "innounp.exe"),
    ("dark", "dark.exe"),
];

/// Output lines of a failed hook quoted in its message.
const SCRIPT_TAIL_LINES: usize = 5;

/// Names seeded into `env.json`: `Path`, `PSModulePath` and those the script names literally,
/// deduplicated ignoring case (PowerShell 5.1 refuses JSON keys that differ only in case).
fn script_env_names(script: &str) -> Vec<String> {
    let mut names = vec![PATH_VAR.to_string(), "PSModulePath".to_string()];
    for n in script_scan::env_var_names(script) {
        if !names.iter().any(|m| m.eq_ignore_ascii_case(&n)) {
            names.push(n);
        }
    }
    names
}

type EnvSnapshot = Vec<(String, Option<String>)>;

fn env_values(env: &dyn EnvStore, names: &[String]) -> Result<EnvSnapshot, String> {
    names
        .iter()
        .map(|n| {
            let v = env.get(n).map_err(|e| format!("{n}: {e}"))?;
            Ok((n.clone(), v.map(|v| v.value)))
        })
        .collect()
}

fn env_seed_json(values: &EnvSnapshot) -> String {
    let map: serde_json::Map<String, serde_json::Value> = values
        .iter()
        .map(|(k, v)| {
            let v = v
                .clone()
                .map_or(serde_json::Value::Null, serde_json::Value::String);
            (k.clone(), v)
        })
        .collect();
    serde_json::Value::Object(map).to_string()
}

/// Seeded names whose value differs now (D5: what a failed script left behind).
fn changed_env(env: &dyn EnvStore, before: &EnvSnapshot) -> Vec<String> {
    before
        .iter()
        .filter(|(name, old)| match env.get(name) {
            Ok(now) => now.map(|v| v.value) != *old,
            Err(e) => {
                tracing::warn!("{name}: {e}");
                false
            }
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// `$__wr_helpers`: the helpers installed at `<root>\apps\<helper>\current\<exe>`.
fn script_helpers(layout: &Layout) -> Vec<(String, String)> {
    SCRIPT_HELPERS
        .iter()
        .filter_map(|(helper, exe)| {
            let p = layout.current(helper).join(exe);
            p.is_file()
                .then(|| (helper.to_string(), p.to_string_lossy().into_owned()))
        })
        .collect()
}

/// The prelude's contract variables, in [`CONTRACT_VARS`] order; one without a value is a bug.
fn contract_vars(values: Vec<(&str, VarValue)>) -> Result<Vec<(String, VarValue)>, String> {
    CONTRACT_VARS
        .iter()
        .map(|name| {
            values
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| (name.to_string(), v.clone()))
                .ok_or_else(|| format!("erro interno: variável {name} do prelúdio sem valor"))
        })
        .collect()
}

/// A fresh `<root>\tmp\script-<app>-<hook>-<n>` (`app` already checked as one component).
fn fresh_script_dir(
    run: &mut Run<'_>,
    layout: &Layout,
    app: &str,
    kind: HookKind,
) -> Result<PathBuf, String> {
    let tmp = layout.root().join("tmp");
    run.ensure_dirs(&tmp)?;
    let hook = kind.key().replace('.', "-");
    for _ in 0..ASIDE_TRIES {
        let n = SCRIPT_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = tmp.join(format!("script-{app}-{hook}-{n}"));
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_msg(&dir, e)),
        }
    }
    Err(format!("nenhuma pasta livre para o script {}", kind.key()))
}

/// `Step::Hook`: writes the hook, the prelude, the manifest, the environment seed and the
/// runner into a fresh folder (removed on every path out), then runs it through the host.
async fn run_hook(
    run: &mut Run<'_>,
    ctx: &mut ExecCtx<'_>,
    kind: HookKind,
    script: &str,
) -> Result<(), String> {
    let Some(host) = ctx.runners.scripts.clone() else {
        return Err("sem consentimento para rodar os scripts do manifesto".into());
    };
    let plan = run.plan;
    let layout = ctx.layout;
    let (app, version) = (plan.app.as_str(), plan.version.as_str());
    let hook = kind.key();
    let manifest_json = plan
        .steps
        .iter()
        .find_map(|s| match s {
            Step::WriteManifest { json } => Some(json.as_str()),
            _ => None,
        })
        .ok_or_else(|| "erro interno: plano com script e sem manifesto".to_string())?;
    let work = fresh_script_dir(run, layout, app, kind)?;
    let _remove = RemoveWork(work.clone());
    let write = |name: &str, bytes: &[u8]| -> Result<PathBuf, String> {
        let p = work.join(name);
        fs::write(&p, bytes).map_err(|e| io_msg(&p, e))?;
        Ok(p)
    };
    let prelude = write("prelude.ps1", PRELUDE.as_bytes())?;
    let hook_file = write("hook.ps1", script.as_bytes())?;
    let manifest_path = write("manifest.json", manifest_json.as_bytes())?;
    let before = env_values(ctx.effects.env, &script_env_names(script))?;
    let seed = env_seed_json(&before);
    write("env.json", seed.as_bytes())?;
    let effects = write("effects.jsonl", b"")?;
    let root = layout.root().to_path_buf();
    let cx = ScriptContext {
        op: match plan.upgrade_from {
            Some(_) => ScriptOp::Update,
            None => ScriptOp::Install,
        },
        app: app.to_string(),
        version: version.to_string(),
        old_version: plan.upgrade_from.clone(),
        bucket: Some(plan.bucket.clone()),
        arch: plan.arch,
        root: root.clone(),
        version_dir: layout.version_dir(app, version),
        current: layout.current(app),
        persist_dir: layout.persist_dir(app),
        fnames: plan
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::Download { source, .. } => Some(source.file_name.clone()),
                _ => None,
            })
            .collect(),
        purge: false,
        manifest_path,
        bucketsdir: root.join("buckets"),
    };
    let path_str = |p: &Path| VarValue::Str(p.to_string_lossy().into_owned());
    let contract = contract_vars(vec![
        ("__wr_root", path_str(&root)),
        ("__wr_effects", path_str(&effects)),
        ("__wr_env_seed", VarValue::Str(seed)),
        ("__wr_helpers", VarValue::Table(script_helpers(layout))),
        (
            "__wr_shortcut_root",
            path_str(&ctx.effects.shortcuts.root()),
        ),
    ])?;
    let vars = script_vars(kind, &cx);
    let text = runner_ps1(&RunnerInput {
        contract: &contract,
        vars: &vars,
        prelude_path: &prelude,
        body: ScriptBody::Hook {
            script_path: &hook_file,
        },
    })
    .map_err(|e| format!("erro interno: {e}"))?;
    // powershell 5.1 reads a BOM-less file as ANSI.
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend_from_slice(text.as_bytes());
    let runner = write("runner.ps1", &bytes)?;
    let job = ScriptJob {
        app: app.to_string(),
        hook: kind,
        workdir: work.clone(),
        runner,
        env: vec![
            ("SCOOP".into(), root.clone().into_os_string()),
            (
                "SCOOP_GLOBAL".into(),
                root.join(".global-disabled").into_os_string(),
            ),
            ("SCOOP_CACHE".into(), root.join("cache").into_os_string()),
        ],
    };
    tracing::info!("rodando o script {hook} de {app}");
    let gate = ctx.launch.clone();
    let launch = move || {
        // Right before PowerShell starts: a Ctrl+C until here cancels; from here it is refused
        // and the hook runs to its end (it is never killed halfway through its effects).
        if !gate.try_launch() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "instalação cancelada antes de o script começar",
            ));
        }
        host.run(&job)
    };
    let worker = AwaitWorker::spawn(launch, work.clone(), ctx.launch.clone());
    let result = match worker.join().await {
        Ok(r) => r,
        Err(e) => match e.try_into_panic() {
            // Re-raised by `execute_install` once the install is rolled back.
            Ok(p) => {
                run.panic = Some(p);
                return Err(format!("pânico ao rodar o script {hook}"));
            }
            Err(e) => return Err(e.to_string()),
        },
    };
    let failure = match result {
        Ok(ScriptOutcome { code: 0, .. }) => None,
        Ok(ScriptOutcome { code, lines }) => {
            let tail = &lines[lines.len().saturating_sub(SCRIPT_TAIL_LINES)..];
            Some(format!(
                "o script {hook} saiu com código {code}: {}",
                tail.join(" | ")
            ))
        }
        Err(e) => Some(format!("não consegui rodar o script {hook}: {e}")),
    };
    if let Some(mut message) = failure {
        let changed = changed_env(ctx.effects.env, &before);
        if !changed.is_empty() {
            message.push_str(&format!(
                "; variáveis alteradas pelo script e não desfeitas: {}",
                changed.join(", ")
            ));
        }
        return Err(message);
    }
    // Every effect is parsed before any is applied: an unreadable file changes nothing.
    let parsed = read_effects(&effects).map_err(|e| format!("efeitos do script ilegíveis: {e}"))?;
    for effect in parsed {
        apply_effect(run, ctx, effect)?;
    }
    Ok(())
}

/// Cap of `effects.jsonl`, checked before reading it.
const MAX_EFFECTS_BYTES: u64 = 64 * 1024 * 1024;

/// Reads and parses `effects.jsonl` (written by third-party code: size checked first, the
/// read itself bounded by the same cap).
fn read_effects(path: &Path) -> Result<Vec<ScriptEffect>, String> {
    use std::io::Read;
    let too_big = || "effects.jsonl passa de 64 MiB".to_string();
    let f = fs::File::open(path).map_err(|e| io_msg(path, e))?;
    let len = f.metadata().map_err(|e| io_msg(path, e))?.len();
    if len > MAX_EFFECTS_BYTES {
        return Err(too_big());
    }
    let mut bytes = Vec::new();
    f.take(MAX_EFFECTS_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_msg(path, e))?;
    if u64::try_from(bytes.len()).map_or(true, |n| n > MAX_EFFECTS_BYTES) {
        return Err(too_big());
    }
    let text = String::from_utf8(bytes).map_err(|_| "effects.jsonl não é UTF-8".to_string())?;
    script_effects::parse_effects(&text).map_err(|e| e.to_string())
}

/// Variables a script may never set with `Set-EnvVar`/`env` (exact name, any case): the
/// PATH and `PSModulePath` only change entry by entry (`Add-Path`), and the rest are read by
/// Windows, the shells or a language runtime at startup.
const DENIED_ENV: &[&str] = &[
    "PATHEXT",
    "ComSpec",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "HOME",
    "HOMEDRIVE",
    "HOMEPATH",
    "SystemRoot",
    "windir",
    "PSExecutionPolicyPreference",
    "__PSLockdownPolicy",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "BASH_ENV",
    "ENV",
];

/// Families refused by prefix (any case): loader, profiler and runtime hooks (.NET, Node,
/// Python, Perl, Ruby, git, the dynamic linker), `PSModule*` and the processor variables.
const DENIED_ENV_PREFIXES: &[&str] = &[
    "COR_",
    "CORECLR_",
    "COMPLUS_",
    "DOTNET_",
    "NODE_",
    "PYTHON",
    "PERL",
    "RUBY",
    "GIT_",
    "LD_",
    "PSMODULE",
    "PROCESSOR_",
];

/// Harmless names real manifests set that a prefix family would catch; `<APP>_HOME` is
/// accepted the same way (never an exact name of [`DENIED_ENV`]).
const ALLOWED_ENV: &[&str] = &[
    "JAVA_HOME",
    "GOPATH",
    "GOROOT",
    "ANDROID_HOME",
    "ANDROID_SDK_ROOT",
];

fn env_denied(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    if DENIED_ENV.iter().any(|d| d.eq_ignore_ascii_case(name)) {
        return true;
    }
    if ALLOWED_ENV.iter().any(|a| a.eq_ignore_ascii_case(name)) || upper.ends_with("_HOME") {
        return false;
    }
    DENIED_ENV_PREFIXES.iter().any(|p| upper.starts_with(p))
}

/// Longest `Remove-Path` pattern accepted (the match is quadratic in the worst case).
const MAX_PATTERN_CHARS: usize = 1024;

/// Name a script may set: `[A-Za-z_][A-Za-z0-9_]*`, at most 255 characters, not the PATH and
/// not refused by [`env_denied`].
fn check_env_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let valid = name.len() <= 255
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err(format!("nome de variável inválido: {name:?}"));
    }
    if name.eq_ignore_ascii_case(PATH_VAR) {
        return Err("o script tentou gravar o PATH inteiro; use Add-Path".into());
    }
    if env_denied(name) {
        return Err(format!("a variável {name} não pode ser gravada por script"));
    }
    Ok(())
}

/// `entry` is `base` itself or a path under it made only of safe components (no `..`,
/// drive, ADS, reserved name), compared through `path_edit::norm`. `;`, `%`, `"` and control
/// characters are refused: they would split or expand a PATH entry.
fn is_inside(entry: &str, base: &Path) -> bool {
    if entry.contains([';', '%', '"']) || entry.chars().any(char::is_control) {
        return false;
    }
    let base = base.to_string_lossy();
    let Some((head, rest)) = entry.split_at_checked(base.len()) else {
        return false;
    };
    if path_edit::norm(head) != path_edit::norm(&base) {
        return false;
    }
    if rest.is_empty() {
        return true;
    }
    match rest.strip_prefix(['\\', '/']) {
        Some(rel) => safepath::safe_components(rel).is_some(),
        None => false,
    }
}

/// PowerShell `-like` (case-insensitive): `*`, `?`, `[set]`/`[a-z]` and the backtick escape.
fn like(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    // One token of the pattern at `pi`: (matches `c`?, length in chars); `None` for `*`.
    let token = |pi: usize, c: char| -> (bool, usize) {
        match p[pi] {
            '?' => (true, 1),
            '`' if pi + 1 < p.len() => (p[pi + 1] == c, 2),
            '[' => match p[pi + 1..].iter().position(|&x| x == ']') {
                Some(end) if end > 0 => {
                    let set = &p[pi + 1..pi + 1 + end];
                    let mut hit = false;
                    let mut i = 0;
                    while i < set.len() {
                        if i + 2 < set.len() && set[i + 1] == '-' {
                            hit |= set[i] <= c && c <= set[i + 2];
                            i += 3;
                        } else {
                            hit |= set[i] == c;
                            i += 1;
                        }
                    }
                    (hit, end + 2)
                }
                _ => (c == '[', 1),
            },
            x => (x == c, 1),
        }
    };
    // Greedy match with backtracking to the last `*`.
    let (mut ti, mut pi) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            pi += 1;
            star = Some((pi, ti));
            continue;
        }
        if pi < p.len() {
            let (hit, step) = token(pi, t[ti]);
            if hit {
                pi += step;
                ti += 1;
                continue;
            }
        }
        match star {
            Some((sp, st)) => {
                pi = sp;
                ti = st + 1;
                star = Some((sp, st + 1));
            }
            None => return false,
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// A PATH entry this app recorded, in this run or in its previous record.
fn own_path_entry(run: &Run<'_>, entry: &str) -> bool {
    let want = path_edit::norm(entry);
    let same = |a: &Artifact| a.kind == ArtifactKind::Path && path_edit::norm(&a.target) == want;
    run.artifacts.iter().any(same)
        || run
            .previous
            .as_ref()
            .is_some_and(|(_, arts)| arts.iter().any(same))
}

/// A shim this app recorded, in this run or in its previous record.
fn own_shim(run: &Run<'_>, name: &str) -> bool {
    run.artifacts
        .iter()
        .any(|a| a.kind == ArtifactKind::Shim && a.target.eq_ignore_ascii_case(name))
        || run.previous_artifact(ArtifactKind::Shim, name).is_some()
}

/// Applies one effect of `effects.jsonl` (untrusted: written by the manifest script)
/// through the same paths as the phase 2a steps, each with its `Undo` and `Artifact`.
fn apply_effect(
    run: &mut Run<'_>,
    ctx: &mut ExecCtx<'_>,
    effect: ScriptEffect,
) -> Result<(), String> {
    let layout = ctx.layout;
    let app = run.plan.app.clone();
    let app_dir = layout.app_dir(&app);
    let persist_dir = layout.persist_dir(&app);
    let env = ctx.effects.env;
    match effect {
        ScriptEffect::SetEnv { name, value } => {
            check_env_name(&name)?;
            set_env(run, env, &name, value.as_deref())
        }
        ScriptEffect::AddPath { var, entries, .. } => {
            let psmodule = var.eq_ignore_ascii_case(PSMODULE_VAR);
            if !psmodule && !var.eq_ignore_ascii_case(PATH_VAR) {
                return Err(format!("Add-Path em {var} não suportado"));
            }
            let label = if psmodule { PSMODULE_VAR } else { "PATH" };
            for entry in entries {
                if !is_inside(&entry, &app_dir) && !is_inside(&entry, &persist_dir) {
                    return Err(format!(
                        "o script tentou pôr {entry} no {label} (fora da pasta do app)"
                    ));
                }
                if psmodule {
                    psmodulepath_prepend(run, env, &entry)?;
                } else {
                    add_path(run, env, &entry)?;
                }
            }
            Ok(())
        }
        ScriptEffect::RemovePath { var, entries } => {
            if !var.eq_ignore_ascii_case(PATH_VAR) {
                tracing::warn!("{app}: Remove-Path em {var} ignorado");
                return Ok(());
            }
            let patterns: Vec<&str> = entries
                .iter()
                .flat_map(|e| e.split(';'))
                .filter(|p| !p.is_empty())
                .collect();
            if patterns
                .iter()
                .any(|p| p.chars().count() > MAX_PATTERN_CHARS)
            {
                return Err("padrão de Remove-Path longo demais".into());
            }
            let Some(cur) = env.get(PATH_VAR).map_err(|e| e.to_string())? else {
                return Ok(());
            };
            let mut value = cur.value.clone();
            for seg in cur.value.split(';') {
                if seg.is_empty() || !patterns.iter().any(|p| like(seg, p)) {
                    continue;
                }
                if !is_inside(seg, layout.root()) || !own_path_entry(run, seg) {
                    tracing::warn!("{app}: Remove-Path de {seg} ignorado (não é deste app)");
                    continue;
                }
                let want = path_edit::norm(seg);
                run.artifacts.retain(|a| {
                    !(a.kind == ArtifactKind::Path && path_edit::norm(&a.target) == want)
                });
                let users = ctx.state.path_users(&app, seg).map_err(|e| e.to_string())?;
                if users.is_empty() {
                    value = path_edit::path_remove(&value, seg);
                }
            }
            if value == cur.value {
                return Ok(());
            }
            run.push(Undo::RestoreEnv {
                name: PATH_VAR.to_string(),
                previous: Some(cur.clone()),
            });
            env.set(
                PATH_VAR,
                &EnvValue {
                    value,
                    expand: cur.expand,
                },
            )
            .map_err(|e| e.to_string())
        }
        ScriptEffect::Shim { path, name, args } => {
            check_component("comando", &name)?;
            if !is_inside(&path, &app_dir) {
                return Err(format!(
                    "o script tentou criar o comando {name} para {path} (fora da pasta do app)"
                ));
            }
            let spec = ShimSpec {
                kind: planner::shim_kind(&path),
                name,
                target: PathBuf::from(path),
                args,
            };
            make_shim(run, &spec, ctx)
        }
        ScriptEffect::RmShim { name } => {
            check_component("comando", &name)?;
            if !own_shim(run, &name) {
                tracing::warn!("{app}: rm_shim de {name} ignorado (não é deste app)");
                return Ok(());
            }
            let saved = saved_shim_files(layout, &name)?;
            run.push(Undo::RestoreShim {
                name: name.clone(),
                saved,
            });
            shim::remove_shim(layout, &name).map_err(|e| e.to_string())?;
            run.artifacts.retain(|a| {
                !(a.kind == ArtifactKind::Shim && a.target.eq_ignore_ascii_case(&name))
            });
            Ok(())
        }
        ScriptEffect::Shortcut {
            target,
            name,
            args,
            icon,
        } => {
            for p in std::iter::once(&target).chain(icon.as_ref()) {
                if !is_inside(p, &app_dir) {
                    return Err(format!(
                        "o script tentou criar o atalho {name} para {p} (fora da pasta do app)"
                    ));
                }
            }
            // Refused rather than sanitized: `shortcut_link` would turn `..` into `_`.
            if safepath::safe_components(&name).is_none_or(|c| c.is_empty()) {
                return Err(format!("nome inseguro de atalho: {name}"));
            }
            let link = planner::shortcut_link(&ctx.effects.shortcuts.root(), &name);
            let spec = ShortcutSpec {
                link,
                target: PathBuf::from(target),
                args,
                icon: icon.map(PathBuf::from),
            };
            make_shortcut(run, &spec, ctx)
        }
    }
}

/// Runs innounp on `installer` into the fresh folder `tmp` (inside the staging folder) and
/// moves what it wrote into `<staging>\<extract_to>`. The caller removes `tmp` on error.
fn inno_extract(
    runner: ToolRun,
    exe: &Path,
    installer: &Path,
    extract_dir: Option<&str>,
    extract_to: Option<&str>,
    tmp: &Path,
) -> Result<(), extract::ExtractError> {
    use extract::ExtractError;
    let io_err = |p: &Path, e: io::Error| ExtractError::Io(io_msg(p, e));
    let staging = tmp
        .parent()
        .ok_or_else(|| ExtractError::Io(format!("{} sem pasta-mãe", tmp.display())))?;
    let to = extract_to
        .map(extract::safe_components)
        .transpose()?
        .unwrap_or_default();
    // `extract_to` inside the temp folder itself would merge it into itself.
    let tmp_name = tmp.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if to.first().is_some_and(|c| c.eq_ignore_ascii_case(tmp_name)) {
        return Err(ExtractError::UnsafePath(
            extract_to.unwrap_or_default().to_string(),
        ));
    }
    let args = inno::inno_args(installer, tmp, extract_dir)?;
    not_found_ok(remove_any(tmp)).map_err(|e| io_err(tmp, e))?;
    fs::create_dir_all(tmp).map_err(|e| io_err(tmp, e))?;
    let (code, lines) = match extract::catch_decoder(|| runner(exe, &args, tmp)) {
        Ok(r) => r.map_err(|e| ExtractError::Io(format!("innounp: {e}")))?,
        Err(p) => {
            return Err(ExtractError::Decoder(
                crate::install::extract_7z::panic_message(p.as_ref()),
            ));
        }
    };
    let files = inno::count_files(tmp).map_err(|e| io_err(tmp, e))?;
    inno::check_exit(code, files, extract_dir, &lines)?;
    // A link written by the installer would be moved into the app as is: refused.
    if let Some(link) = inno::find_link(tmp).map_err(|e| io_err(tmp, e))? {
        return Err(ExtractError::UnsafePath(link.display().to_string()));
    }
    let mut dest = staging.to_path_buf();
    dest.extend(&to);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    }
    extract::merge_into(tmp, &dest)?;
    not_found_ok(remove_any(tmp)).map_err(|e| io_err(tmp, e))
}

async fn run_step(run: &mut Run<'_>, step: &Step, ctx: &mut ExecCtx<'_>) -> Result<(), String> {
    let plan = run.plan;
    let layout = ctx.layout;
    let (app, version) = (plan.app.as_str(), plan.version.as_str());
    match step {
        Step::EnsureLauncher => {
            let launcher = layout.launcher();
            if launcher.exists() {
                return Ok(());
            }
            if let Some(dir) = launcher.parent() {
                run.ensure_dirs(dir)?;
            }
            run.push(Undo::RemovePath(launcher));
            shim::ensure_launcher(ctx.http, ctx.data, layout)
                .await
                .map_err(|e| e.to_string())
        }
        Step::Download { index, source } => {
            let paths = download::fetch_all(
                ctx.http,
                ctx.data,
                std::slice::from_ref(source),
                ctx.progress,
            )
            .await
            .map_err(|e| e.to_string())?;
            let p = paths
                .into_iter()
                .next()
                .ok_or_else(|| "download sem arquivo".to_string())?;
            run.downloads.insert(*index, p);
            Ok(())
        }
        Step::Extract { index } => {
            let (source, file) = extract_source(run, *index)?;
            let staging = own_staging(run, layout, app, version)?;
            // Phases of the `msiexec /a` fallback (waiting, each launch, importing).
            let (phase_tx, mut phase_rx) = tokio::sync::mpsc::unbounded_channel::<Phase>();
            // The largest 7z takes ~10 s: off the runtime worker, like the download hashing.
            let work = {
                let (extract, name) = (run.extract, source.file_name.clone());
                let (dir, to) = (source.extract_dir.clone(), source.extract_to.clone());
                let (msiexec, layout, app) = (ctx.msiexec, layout.clone(), app.to_string());
                let staging = staging.clone();
                let gate = ctx.launch.clone();
                move || -> Result<(), Refused> {
                    let e = match extract(&file, &name, dir.as_deref(), to.as_deref(), &staging) {
                        Ok(()) => return Ok(()),
                        Err(e) => e,
                    };
                    if !extract::needs_installer(&file, &name, &e) {
                        return Err(Refused::Failed(e.to_string()));
                    }
                    let reason = match e {
                        extract::ExtractError::Unsupported(r) => r,
                        other => other.to_string(),
                    };
                    let Some(m) = msiexec else {
                        return Err(Refused::NeedsMsiexec(reason));
                    };
                    tracing::info!("{name}: {reason}; usando msiexec /a (autorizado)");
                    let source = extract::ExtractSource {
                        file: &file,
                        file_name: &name,
                        extract_dir: dir.as_deref(),
                        extract_to: to.as_deref(),
                    };
                    let phase = |p: Phase| {
                        // The receiver is gone only once the install stopped waiting.
                        let _ = phase_tx.send(p);
                    };
                    extract_with_msiexec_until(m, &layout, &app, &source, &staging, &phase, &gate)
                        .map_err(Refused::Failed)
                }
            };
            tracing::info!("extraindo {}", source.file_name);
            let start = Instant::now();
            let joined = AwaitWorker::spawn(work, staging, ctx.launch.clone()).join();
            tokio::pin!(joined);
            let joined = loop {
                tokio::select! {
                    r = &mut joined => break r,
                    Some(p) = phase_rx.recv() => run.enter(ctx, p),
                }
            };
            let result = match joined {
                Ok(Ok(())) => Ok(()),
                Ok(Err(Refused::Failed(m))) => Err(m),
                Ok(Err(Refused::NeedsMsiexec(reason))) => {
                    let message = format!("o MSI precisa do Windows Installer ({reason})");
                    run.needs_msiexec = Some((*index, reason));
                    Err(message)
                }
                Err(e) => match e.try_into_panic() {
                    // Re-raised by `execute_install` once the staging folder is rolled back.
                    Ok(p) => {
                        run.panic = Some(p);
                        Err("pânico na extração".to_string())
                    }
                    Err(e) => Err(e.to_string()),
                },
            };
            tracing::info!(
                ms = start.elapsed().as_millis() as u64,
                ok = result.is_ok(),
                "extração de {} terminada",
                source.file_name
            );
            result
        }
        Step::ExtractInno { index } => {
            let (source, file) = extract_source(run, *index)?;
            let exe = layout.current("innounp").join("innounp.exe");
            if !exe.is_file() {
                return Err("innounp não está instalado (auxiliar do bucket main)".into());
            }
            let staging = own_staging(run, layout, app, version)?;
            let work = {
                let runner = ctx.runners.innounp;
                let (dir, to) = (source.extract_dir.clone(), source.extract_to.clone());
                let (staging, index) = (staging.clone(), *index);
                move || {
                    let tmp = staging.join(format!(".inno-{index}"));
                    let r = inno_extract(runner, &exe, &file, dir.as_deref(), to.as_deref(), &tmp);
                    if r.is_err()
                        && let Err(e) = not_found_ok(remove_any(&tmp))
                    {
                        tracing::warn!("{}: {e}", tmp.display());
                    }
                    r.map_err(|e| e.to_string())
                }
            };
            tracing::info!("extraindo {} com o innounp", source.file_name);
            let start = Instant::now();
            let result = match AwaitWorker::spawn(work, staging, ctx.launch.clone())
                .join()
                .await
            {
                Ok(r) => r,
                Err(e) => match e.try_into_panic() {
                    // Re-raised by `execute_install` once the staging folder is rolled back.
                    Ok(p) => {
                        run.panic = Some(p);
                        Err("pânico na extração".to_string())
                    }
                    Err(e) => Err(e.to_string()),
                },
            };
            tracing::info!(
                ms = start.elapsed().as_millis() as u64,
                ok = result.is_ok(),
                "extração de {} terminada",
                source.file_name
            );
            result
        }
        Step::Promote => {
            let staging = layout.staging_dir(app, version);
            let vdir = layout.version_dir(app, version);
            if fs::symlink_metadata(&vdir).is_ok() {
                if current_points_to(&layout.current(app), &vdir) {
                    return Err(format!("{} já existe", vdir.display()));
                }
                // A copy of this version left on disk (kept on upgrade, or a failed removal):
                // replaced, and put back if anything below fails.
                let aside = free_aside(layout, app, version)?;
                fs_retry::retry("renomear", &vdir, || fs::rename(&vdir, &aside))
                    .map_err(|e| io_msg(&vdir, e))?;
                run.push(Undo::RestoreAside {
                    aside,
                    original: vdir.clone(),
                });
            }
            fs_retry::retry("renomear", &staging, || fs::rename(&staging, &vdir))
                .map_err(|e| io_msg(&vdir, e))?;
            run.push(Undo::RemovePath(vdir.clone()));
            let arch = plan.arch.map(|a| a.key());
            state::write_install_json(&vdir, &plan.bucket, arch, version)
                .map_err(|e| io_msg(&vdir, e))
        }
        Step::RunInstaller { file, args, keep } => {
            let Some(runner) = ctx.runners.installer else {
                return Err("sem consentimento para rodar o instalador".into());
            };
            let vdir = layout.version_dir(app, version);
            let program = installer_program(&vdir, file)?;
            let work = {
                let (program, args, dir) = (program.clone(), args.clone(), vdir.clone());
                move || runner(&program, &args, &dir)
            };
            tracing::info!("rodando o instalador {file}");
            let staging = layout.staging_dir(app, version);
            let result = match AwaitWorker::spawn(work, staging, ctx.launch.clone())
                .join()
                .await
            {
                Ok(r) => r,
                Err(e) => match e.try_into_panic() {
                    // Re-raised by `execute_install` once the install is rolled back.
                    Ok(p) => {
                        run.panic = Some(p);
                        return Err("pânico ao rodar o instalador".into());
                    }
                    Err(e) => return Err(e.to_string()),
                },
            };
            check_installer_exit(result)?;
            if !keep {
                remove_installer(&program);
            }
            Ok(())
        }
        Step::Hook { kind, script } => run_hook(run, ctx, *kind, script).await,
        Step::Persist(spec) => persist(run, spec, layout),
        Step::LinkCurrent => {
            let current = layout.current(app);
            let previous = match fs::symlink_metadata(&current) {
                Ok(m) if m.file_type().is_symlink() => {
                    Some(link::target(&current).map_err(|e| io_msg(&current, e))?)
                }
                Ok(_) => return Err(format!("{} não é uma junção", current.display())),
                Err(_) => None,
            };
            run.push(Undo::RestoreCurrent {
                link: current.clone(),
                previous: previous.clone(),
            });
            if previous.is_some() {
                link::remove(&current).map_err(|e| io_msg(&current, e))?;
            }
            link::create(&layout.version_dir(app, version), &current)
                .map_err(|e| io_msg(&current, e))
        }
        Step::Shim(spec) => make_shim(run, spec, ctx),
        Step::Shortcut(spec) => make_shortcut(run, spec, ctx),
        Step::PsModule { name } => psmodule(run, ctx, name),
        Step::EnvSet { name, value } => set_env(run, ctx.effects.env, name, Some(value)),
        Step::EnvAddPath { entry } => add_path(run, ctx.effects.env, &entry.display().to_string()),
        Step::EnsureShimsOnPath => {
            add_path(run, ctx.effects.env, &layout.shims().display().to_string())
        }
        Step::WriteManifest { json } => {
            let dir = layout.version_dir(app, version);
            let dest = dir.join("manifest.json");
            let tmp = dir.join("manifest.json.tmp");
            let written = fs::write(&tmp, json.as_bytes())
                .and_then(|()| fs_retry::retry("renomear", &tmp, || fs::rename(&tmp, &dest)));
            if let Err(e) = written {
                let _ = fs::remove_file(&tmp);
                return Err(io_msg(&dest, e));
            }
            run.push(Undo::RemovePath(dest.clone()));
            Ok(())
        }
        Step::RecordState => {
            // Last step that can fail (validated up front); `record` is one transaction, so a
            // failure leaves the previous record untouched and there is nothing to undo.
            let rec = AppRecord {
                app: app.to_string(),
                bucket: plan.bucket.clone(),
                version: version.to_string(),
                arch: plan.arch.map(|a| a.key().to_string()),
                installed_at: now_secs(),
                manifest_sha256: plan.manifest_sha256.clone(),
            };
            let arts = run.artifacts.clone();
            ctx.state.record(&rec, &arts).map_err(|e| e.to_string())
        }
        Step::RemoveOldVersion { .. } => Ok(()),
    }
}

/// Why the extraction worker did not fill the staging folder.
enum Refused {
    Failed(String),
    NeedsMsiexec(String),
}

/// Suffix of the sibling of the staging folder where `msiexec /a` works.
const MSIEXEC_SUFFIX: &str = ".msiexec";

/// Removes a work folder on every path out (the `msiexec` one of [`extract_with_msiexec`], a
/// hook's `tmp\script-*`); whatever wrote into it has always exited by then.
struct RemoveWork(PathBuf);

impl Drop for RemoveWork {
    fn drop(&mut self) {
        if let Err(e) = not_found_ok(remove_any(&self.0)) {
            tracing::warn!("{}: {e}", self.0.display());
        }
    }
}

/// Fallback for an MSI the own extractor refused: `msiexec /a` into
/// `<staging>.msiexec\SourceDir` (a sibling, so it never mixes with the package's own
/// folders), queued behind `msi.lock` (taken after `install.lock`, never before) and the
/// Windows Installer mutex, then its tree moved into `staging` through
/// `extract::import_admin_tree`. Blocks until `msiexec` ends: it is never killed. `phase`
/// gets `WaitingInstaller` while queued or retrying, `Installer` with the log path at each
/// launch (from then on the job cannot be cancelled), then `Placing` before the import.
pub fn extract_with_msiexec(
    m: Msiexec,
    layout: &Layout,
    app: &str,
    source: &extract::ExtractSource<'_>,
    staging: &Path,
    phase: &dyn Fn(Phase),
) -> Result<(), String> {
    extract_with_msiexec_until(
        m,
        layout,
        app,
        source,
        staging,
        phase,
        &LaunchGate::default(),
    )
}

/// [`extract_with_msiexec`] that launches only through `gate`: once the install was
/// cancelled it stops waiting and launches nothing; a launched `msiexec` is always awaited.
fn extract_with_msiexec_until(
    m: Msiexec,
    layout: &Layout,
    app: &str,
    source: &extract::ExtractSource<'_>,
    staging: &Path,
    phase: &dyn Fn(Phase),
    gate: &LaunchGate,
) -> Result<(), String> {
    let mut work = staging.as_os_str().to_owned();
    work.push(MSIEXEC_SUFFIX);
    let work = PathBuf::from(work);
    // A work folder left by a crashed run is garbage of ours.
    not_found_ok(remove_any(&work)).map_err(|e| io_msg(&work, e))?;
    fs::create_dir_all(&work).map_err(|e| io_msg(&work, e))?;
    let _remove = RemoveWork(work.clone());
    // msiexec names its administrative copy after the package file: give it the manifest's name.
    let admin = match safepath::safe_components(source.file_name).as_deref() {
        Some([one]) => one.clone(),
        _ => "package.msi".to_string(),
    };
    let package = work.join(&admin);
    if fs::hard_link(source.file, &package).is_err() {
        fs::copy(source.file, &package).map_err(|e| io_msg(&package, e))?;
    }
    let target = work.join("SourceDir");
    let started = |log: &Path| {
        phase(Phase::Installer {
            log: log.to_path_buf(),
        })
    };
    let mut status = |s: msi_queue::Status| {
        tracing::info!(app, status = ?s, "msiexec /a");
        match s {
            // Same numbering as the launch below: one launch per `Running`.
            msi_queue::Status::Running { attempt } => {
                started(&layout.msiexec_log(app, u32::from(attempt)));
            }
            msi_queue::Status::WaitingLock
            | msi_queue::Status::WaitingInstaller
            | msi_queue::Status::Retrying { .. } => phase(Phase::WaitingInstaller),
        }
    };
    let lock = msi_queue::lock_msi(&layout.msi_lock(), &mut status).map_err(|e| e.to_string())?;
    let mut attempt = 0u32;
    let ran = msi_queue::run_queued(
        &msi_queue::POLICY,
        // A cancelled install stops waiting at once; the launch below then refuses.
        &mut || !gate.cancelled() && (m.busy)(),
        &mut || {
            // Right before the launch: from here a Ctrl+C is refused, or this refuses.
            if !gate.try_launch() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "instalação cancelada antes de o msiexec começar",
                ));
            }
            attempt += 1;
            (m.launch)(&package, &target, &layout.msiexec_log(app, attempt))
        },
        &mut std::thread::sleep,
        &mut status,
    );
    drop(lock);
    if let Err(e) = ran {
        let mut message = format!("msiexec /a: {e}");
        if let msi_queue::MsiQueueError::Failed(code) = e
            && let Some(hint) = failure_hint(code, &layout.msiexec_log(app, attempt))
        {
            message.push_str("; ");
            message.push_str(hint);
        }
        return Err(message);
    }
    // The log stops growing now: a phase of its own, not a silent installer.
    phase(Phase::Placing);
    extract::import_admin_tree(source, &target, &admin, staging, &extract::DEFAULT_LIMITS)
        .map_err(|e| e.to_string())
}

/// What the end of the failed attempt's log explains about exit `code`, if anything. Only
/// the tail is read (the log reaches hundreds of MB).
fn failure_hint(code: i32, log: &Path) -> Option<&'static str> {
    use crate::install::msiexec_log;
    let tail = fs::File::open(log)
        .and_then(|mut f| msiexec_log::tail_text(&mut f, msiexec_log::ERROR_WINDOW));
    match tail {
        Ok(text) => msiexec_log::failure_hint(code, &text),
        Err(e) => {
            tracing::debug!("{}: {e}", log.display());
            None
        }
    }
}

/// Set once the worker ends (returned or unwound).
type DoneFlag = Arc<(Mutex<bool>, Condvar)>;

/// Sets the flag when dropped: also while the worker unwinds.
struct SignalDone(DoneFlag);

impl Drop for SignalDone {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.0;
        *lock.lock().unwrap_or_else(|p| p.into_inner()) = true;
        cvar.notify_all();
    }
}

/// A blocking worker writing into `staging` (extraction, `msiexec /a`). If the awaiting
/// future is dropped (the install was cancelled), the drop waits for the worker to end and
/// only then removes `staging`: nothing is rolled back, nor left behind, while it still writes.
struct AwaitWorker<T> {
    handle: Option<tokio::task::JoinHandle<T>>,
    done: DoneFlag,
    staging: PathBuf,
    /// Cancelled by the drop, before waiting: the worker launches nothing afterwards.
    gate: LaunchGate,
}

impl<T: Send + 'static> AwaitWorker<T> {
    fn spawn(
        work: impl FnOnce() -> T + Send + 'static,
        staging: PathBuf,
        gate: LaunchGate,
    ) -> AwaitWorker<T> {
        let done: DoneFlag = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = SignalDone(Arc::clone(&done));
        let handle = tokio::task::spawn_blocking(move || {
            let _signal = signal;
            work()
        });
        AwaitWorker {
            handle: Some(handle),
            done,
            staging,
            gate,
        }
    }

    /// Awaits the worker; once it answered, dropping `self` does nothing.
    async fn join(mut self) -> Result<T, tokio::task::JoinError> {
        let r = match self.handle.as_mut() {
            Some(handle) => handle.await,
            None => unreachable!("an AwaitWorker is joined once"),
        };
        self.handle = None;
        r
    }
}

impl<T> Drop for AwaitWorker<T> {
    fn drop(&mut self) {
        if self.handle.take().is_none() {
            return;
        }
        // Refused when `msiexec` or a hook already runs: then the wait below is for it to end.
        if !self.gate.try_cancel() {
            tracing::warn!("um passo já começou e não pode ser interrompido; esperando terminar");
        }
        let wait = || {
            let (lock, cvar) = &*self.done;
            let mut done = lock.lock().unwrap_or_else(|p| p.into_inner());
            while !*done {
                done = cvar.wait(done).unwrap_or_else(|p| p.into_inner());
            }
        };
        // On a multi-thread runtime, let its other tasks move off this thread meanwhile.
        match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
            Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(wait),
            _ => wait(),
        }
        tracing::warn!(
            "instalação cancelada durante um passo em andamento; removendo {}",
            self.staging.display()
        );
        if let Err(e) = not_found_ok(remove_any(&self.staging)) {
            tracing::warn!("{}: {e}", self.staging.display());
        }
        // The app folder the extraction step created, when nothing else is in it (a fresh
        // install); `remove_dir` refuses a folder that is not empty.
        if let Some(app_dir) = self.staging.parent() {
            let _ = fs::remove_dir(app_dir);
        }
    }
}

/// Whether `name`, in an app folder, is what a killed install leaves behind: a staging
/// folder (`.staging-<v>`), the `msiexec` work beside it (`.staging-<v>.msiexec`) or an
/// extraction temp (`.wingetrs-extract-*`). Only ever touched under `install.lock`.
pub fn is_dead_run_leftover(name: &str) -> bool {
    name.starts_with(".staging-") || name.starts_with(extract::TEMP_PREFIX)
}

/// Removes every [`is_dead_run_leftover`] in `app_dir`, best effort: a leftover that cannot
/// go (say, an orphaned `msiexec` of a closed console still writing) is logged, not fatal.
fn remove_dead_run_leftovers(app_dir: &Path) {
    let Ok(entries) = fs::read_dir(app_dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_str().is_some_and(is_dead_run_leftover) {
            let p = entry.path();
            tracing::info!("removendo sobra de execução interrompida: {}", p.display());
            if let Err(e) = not_found_ok(remove_any(&p)) {
                tracing::warn!("{}: {e}", p.display());
            }
        }
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn add_path(run: &mut Run<'_>, env: &dyn EnvStore, entry: &str) -> Result<(), String> {
    let cur = env.get(PATH_VAR).map_err(|e| e.to_string())?;
    let existing = cur.as_ref().map(|v| v.value.as_str()).unwrap_or("");
    if !path_edit::path_contains(existing, entry) {
        run.push(Undo::PathRemove {
            entry: entry.to_string(),
            var_existed: cur.is_some(),
        });
        env.set(
            PATH_VAR,
            &EnvValue {
                value: path_edit::path_add(existing, entry),
                expand: cur.as_ref().is_none_or(|v| v.expand),
            },
        )
        .map_err(|e| e.to_string())?;
    }
    run.artifacts.push(Artifact {
        kind: ArtifactKind::Path,
        target: entry.to_string(),
        previous: None,
    });
    Ok(())
}

/// Another app whose record lists an artifact of `kind` at `target` (case-insensitive).
fn owner_of(
    state: &StateDb,
    app: &str,
    kind: ArtifactKind,
    target: &str,
) -> Result<Option<String>, String> {
    for rec in state.apps().map_err(|e| e.to_string())? {
        if rec.app == app {
            continue;
        }
        let owns = state
            .get(&rec.app)
            .map_err(|e| e.to_string())?
            .is_some_and(|(_, arts)| {
                arts.iter()
                    .any(|a| a.kind == kind && a.target.eq_ignore_ascii_case(target))
            });
        if owns {
            return Ok(Some(rec.app));
        }
    }
    Ok(None)
}

/// Pushes `a` unless this run already recorded the same kind and target.
fn push_artifact(run: &mut Run<'_>, a: Artifact) {
    let dup = run
        .artifacts
        .iter()
        .any(|o| o.kind == a.kind && o.target.eq_ignore_ascii_case(&a.target));
    if !dup {
        run.artifacts.push(a);
    }
}

/// Current contents of `shims\<name>.*`, to put back on undo.
fn saved_shim_files(layout: &Layout, name: &str) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let dir = layout.shims();
    let mut saved = Vec::new();
    for ext in ["exe", "shim", "cmd", "ps1"] {
        let p = dir.join(format!("{name}.{ext}"));
        match fs::read(&p) {
            Ok(bytes) => saved.push((p, bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_msg(&p, e)),
        }
    }
    Ok(saved)
}

fn make_shim(run: &mut Run<'_>, spec: &ShimSpec, ctx: &mut ExecCtx<'_>) -> Result<(), String> {
    if let Some(owner) = owner_of(ctx.state, &run.plan.app, ArtifactKind::Shim, &spec.name)? {
        return Err(format!("o comando {} já pertence a {owner}", spec.name));
    }
    let dir = ctx.layout.shims();
    run.ensure_dirs(&dir)?;
    let saved = saved_shim_files(ctx.layout, &spec.name)?;
    run.push(Undo::RestoreShim {
        name: spec.name.clone(),
        saved,
    });
    shim::create_shim(ctx.layout, spec).map_err(|e| e.to_string())?;
    push_artifact(
        run,
        Artifact {
            kind: ArtifactKind::Shim,
            target: spec.name.clone(),
            previous: None,
        },
    );
    Ok(())
}

fn make_shortcut(
    run: &mut Run<'_>,
    spec: &ShortcutSpec,
    ctx: &mut ExecCtx<'_>,
) -> Result<(), String> {
    let target = spec.link.display().to_string();
    if let Some(owner) = owner_of(ctx.state, &run.plan.app, ArtifactKind::Shortcut, &target)? {
        return Err(format!("o atalho {target} já pertence a {owner}"));
    }
    if run
        .previous_artifact(ArtifactKind::Shortcut, &target)
        .is_some()
    {
        run.push(Undo::RecreateShortcut(spec.clone()));
    } else {
        run.push(Undo::RemoveShortcut(spec.link.clone()));
    }
    ctx.effects
        .shortcuts
        .create(
            &spec.link,
            &spec.target,
            spec.args.as_deref(),
            spec.icon.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    push_artifact(
        run,
        Artifact {
            kind: ArtifactKind::Shortcut,
            target,
            previous: None,
        },
    );
    Ok(())
}

/// Sets (`Some`) or removes (`None`) user variable `name`. The artifact keeps the value from
/// before the first install (an upgrade, or a script setting what an earlier step set).
fn set_env(
    run: &mut Run<'_>,
    env: &dyn EnvStore,
    name: &str,
    value: Option<&str>,
) -> Result<(), String> {
    let before = env.get(name).map_err(|e| e.to_string())?;
    let recorded_previous = match run.previous_artifact(ArtifactKind::Env, name) {
        Some(a) => a.previous.clone(),
        None => before.as_ref().map(|v| v.value.clone()),
    };
    run.push(Undo::RestoreEnv {
        name: name.to_string(),
        previous: before,
    });
    match value {
        Some(v) => env.set(
            name,
            &EnvValue {
                value: v.to_string(),
                expand: v.contains('%'),
            },
        ),
        None => env.remove(name),
    }
    .map_err(|e| e.to_string())?;
    push_artifact(
        run,
        Artifact {
            kind: ArtifactKind::Env,
            target: name.to_string(),
            previous: recorded_previous,
        },
    );
    Ok(())
}

/// Scoop's `persist_data`: the disk decides the kind; the plan's kind only applies when
/// the item exists neither in the version nor in `persist\`.
fn persist(run: &mut Run<'_>, spec: &PersistSpec, layout: &Layout) -> Result<(), String> {
    let (app, version) = (run.plan.app.as_str(), run.plan.version.as_str());
    let src = layout.version_dir(app, version).join(&spec.in_version);
    let dst = layout.persist_dir(app).join(&spec.in_persist);
    let src_meta = fs::symlink_metadata(&src).ok();
    let dst_meta = fs::symlink_metadata(&dst).ok();
    let is_dir = match (&src_meta, &dst_meta) {
        (Some(m), _) | (None, Some(m)) => m.is_dir(),
        (None, None) => spec.kind == PersistKind::Dir,
    };
    if let Some(parent) = dst.parent() {
        run.ensure_dirs(parent)?;
    }
    // Only what this step creates in persist\ may be deleted by its undo.
    let mut created = false;
    match (src_meta.is_some(), dst_meta.is_some()) {
        (true, false) => {
            fs_retry::retry("renomear", &src, || fs::rename(&src, &dst))
                .map_err(|e| io_msg(&dst, e))?;
            created = true;
        }
        (_, true) => {
            // The user's data wins; the version's copy is part of the version dir.
            remove_any(&src).map_err(|e| io_msg(&src, e))?;
        }
        (false, false) => {
            let r = if is_dir {
                fs::create_dir(&dst)
            } else {
                fs::File::create(&dst).map(drop)
            };
            r.map_err(|e| io_msg(&dst, e))?;
            created = true;
        }
    }
    if created {
        run.push(Undo::RemovePath(dst.clone()));
    }
    if let Some(parent) = src.parent() {
        run.ensure_dirs(parent)?;
    }
    if is_dir {
        link::create(&dst, &src).map_err(|e| io_msg(&src, e))?;
    } else {
        fs::hard_link(&dst, &src).map_err(|e| io_msg(&src, e))?;
    }
    run.push(Undo::RemovePath(src));
    run.artifacts.push(Artifact {
        kind: ArtifactKind::Persist,
        target: spec.in_persist.display().to_string(),
        previous: None,
    });
    Ok(())
}

/// User variable listing PowerShell module folders.
pub(crate) const PSMODULE_VAR: &str = "PSModulePath";

/// Scoop `lib/psmodules.ps1` `install_psmodule` + `ensure_in_psmodulepath`: junction
/// `<modules>\<name>` -> `current`, and `<modules>` first in the user `PSModulePath`.
fn psmodule(run: &mut Run<'_>, ctx: &mut ExecCtx<'_>, name: &str) -> Result<(), String> {
    let layout = ctx.layout;
    let app = run.plan.app.as_str();
    let modules = layout.modules();
    check_component("módulo", name)?;
    let link_path = modules.join(name);
    if link_path.parent() != Some(modules.as_path()) {
        return Err(format!("nome inseguro de módulo: {name}"));
    }
    run.ensure_dirs(&modules)?;
    if let Some(owner) = owner_of(ctx.state, app, ArtifactKind::PsModule, name)? {
        return Err(format!("o módulo {name} pertence a {owner}"));
    }
    match fs::symlink_metadata(&link_path) {
        Ok(m) => {
            let ours = m.file_type().is_symlink()
                && run
                    .previous_artifact(ArtifactKind::PsModule, name)
                    .is_some();
            if !ours {
                return Err(format!(
                    "{} já existe e não é do wingetrs",
                    link_path.display()
                ));
            }
            let previous = link::target(&link_path).map_err(|e| io_msg(&link_path, e))?;
            run.push(Undo::RestoreCurrent {
                link: link_path.clone(),
                previous: Some(previous),
            });
            link::remove(&link_path).map_err(|e| io_msg(&link_path, e))?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_msg(&link_path, e)),
    }
    link::create(&layout.current(app), &link_path).map_err(|e| io_msg(&link_path, e))?;
    run.push(Undo::RemovePath(link_path));
    run.artifacts.push(Artifact {
        kind: ArtifactKind::PsModule,
        target: name.to_string(),
        previous: None,
    });

    psmodulepath_prepend(run, ctx.effects.env, &modules.display().to_string())
}

/// Puts `entry` first in the user `PSModulePath` unless it is there already (a missing
/// variable starts from the user's default module folder).
fn psmodulepath_prepend(run: &mut Run<'_>, env: &dyn EnvStore, entry: &str) -> Result<(), String> {
    let before = env.get(PSMODULE_VAR).map_err(|e| e.to_string())?;
    let base = match &before {
        Some(v) => v.value.clone(),
        None => {
            let profile = std::env::var_os("USERPROFILE").unwrap_or_default();
            Path::new(&profile)
                .join(r"Documents\WindowsPowerShell\Modules")
                .display()
                .to_string()
        }
    };
    if path_edit::path_contains(&base, entry) {
        return Ok(());
    }
    run.push(Undo::RestoreEnv {
        name: PSMODULE_VAR.to_string(),
        previous: before.clone(),
    });
    env.set(
        PSMODULE_VAR,
        &EnvValue {
            value: format!("{entry};{base}"),
            expand: before.is_some_and(|v| v.expand),
        },
    )
    .map_err(|e| e.to_string())
}

/// Removes the junction of module `name`; the last module also leaves `PSModulePath`.
fn remove_psmodule(
    ctx: &mut ExecCtx<'_>,
    app: &str,
    name: &str,
    app_keeps_module: bool,
) -> Result<(), String> {
    let modules = ctx.layout.modules();
    check_component("módulo", name)?;
    let link_path = modules.join(name);
    if link_path.parent() != Some(modules.as_path()) {
        return Err(format!("nome inseguro de módulo: {name}"));
    }
    match fs::symlink_metadata(&link_path) {
        Ok(m) if m.file_type().is_symlink() => {
            link::remove(&link_path).map_err(|e| io_msg(&link_path, e))?
        }
        Ok(_) => return Err(format!("{} não é uma junção", link_path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_msg(&link_path, e)),
    }
    let others: Vec<String> = ctx
        .state
        .apps_with_artifact(ArtifactKind::PsModule)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|a| a != app)
        .collect();
    if others.is_empty() && !app_keeps_module {
        let env = ctx.effects.env;
        if let Some(cur) = env.get(PSMODULE_VAR).map_err(|e| e.to_string())? {
            let value = path_edit::path_remove(&cur.value, &modules.display().to_string());
            if value.is_empty() {
                env.remove(PSMODULE_VAR).map_err(|e| e.to_string())?;
            } else {
                env.set(
                    PSMODULE_VAR,
                    &EnvValue {
                        value,
                        expand: cur.expand,
                    },
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Removes one recorded artifact of `app` (step label and result); `None` for persist,
/// which uninstall handles as a whole. A PATH entry goes only when no other app records it,
/// and the variable itself goes if nothing is left in it.
fn remove_artifact(
    ctx: &mut ExecCtx<'_>,
    app: &str,
    a: &Artifact,
    remove_path: bool,
    keeps_module: bool,
) -> Option<(String, Result<(), String>)> {
    let env = ctx.effects.env;
    Some(match a.kind {
        ArtifactKind::Shim => (
            format!("comando {}", a.target),
            shim::remove_shim(ctx.layout, &a.target).map_err(|e| e.to_string()),
        ),
        ArtifactKind::Shortcut => (
            format!("atalho {}", a.target),
            ctx.effects
                .shortcuts
                .remove(Path::new(&a.target))
                .map_err(|e| e.to_string()),
        ),
        ArtifactKind::Env => {
            let previous = a.previous.as_ref().map(|v| EnvValue {
                value: v.clone(),
                expand: v.contains('%'),
            });
            (
                format!("variável {}", a.target),
                restore_env(env, &a.target, previous.as_ref()).map_err(|e| e.to_string()),
            )
        }
        ArtifactKind::Path => {
            let r = if remove_path {
                ctx.state
                    .path_users(app, &a.target)
                    .map_err(|e| e.to_string())
                    .and_then(|users| {
                        if users.is_empty() {
                            path_remove_entry(env, &a.target, false).map_err(|e| e.to_string())
                        } else {
                            Ok(())
                        }
                    })
            } else {
                Ok(())
            };
            (format!("PATH {}", a.target), r)
        }
        ArtifactKind::PsModule => (
            format!("módulo PowerShell {}", a.target),
            remove_psmodule(ctx, app, &a.target, keeps_module),
        ),
        ArtifactKind::Persist => return None,
    })
}

/// Removes what the app's record lists. Nothing changes when a file under `apps\<app>` is in use.
///
/// Never broadcasts the environment change: the caller calls `effects.env.broadcast()` once
/// after a successful uninstall.
///
/// Returns notes for the user: what did not happen although the app might have wanted it
/// (the uninstaller skipped because the installed manifest could not be trusted).
pub fn execute_uninstall(
    plan: &UninstallPlan,
    ctx: &mut ExecCtx<'_>,
) -> Result<Vec<String>, ExecError> {
    let fail = |index: usize, step: String, message: String| ExecError::Failed {
        index,
        step,
        message,
        undone: Vec::new(),
    };
    check_component("app", &plan.app).map_err(|m| fail(0, "validar plano".into(), m))?;
    let layout = ctx.layout;
    let app_dir = layout.app_dir(&plan.app);
    if let Some(p) = (ctx.in_use)(&app_dir).first() {
        return Err(ExecError::InUse(p.display().to_string()));
    }
    let notes = run_uninstaller(ctx, &plan.app)
        .map_err(|e| match e {
            UninstallerError::Consent(e) => e,
            UninstallerError::Failed(m) => fail(0, "rodar desinstalador".into(), m),
        })?
        .into_iter()
        .collect();
    for (i, a) in plan.artifacts.iter().enumerate() {
        if let Some((label, r)) = remove_artifact(ctx, &plan.app, a, plan.remove_path_entry, false)
        {
            r.map_err(|m| fail(i, label, m))?;
        }
    }
    let n = plan.artifacts.len();
    remove_any(&app_dir).map_err(|e| fail(n, "remover app".into(), io_msg(&app_dir, e)))?;
    if plan.purge {
        let pd = layout.persist_dir(&plan.app);
        remove_any(&pd).map_err(|e| fail(n + 1, "remover persist".into(), io_msg(&pd, e)))?;
    }
    ctx.state
        .remove(&plan.app)
        .map_err(|e| fail(n + 2, "registro".into(), e.to_string()))?;
    Ok(notes)
}

enum UninstallerError {
    Consent(ExecError),
    Failed(String),
}

/// Host architecture recorded at install time (`None` = unknown key).
fn arch_of_key(key: &str) -> Option<Arch> {
    [Arch::X64, Arch::X86, Arch::Arm64]
        .into_iter()
        .find(|a| a.key() == key)
}

/// Runs `uninstaller.file` of the installed manifest, if it has one, before anything is
/// removed. A manifest that is missing, altered (hash differs from the record) or not
/// resolvable runs nothing: the removal goes on as before, with a warning.
fn run_uninstaller(ctx: &mut ExecCtx<'_>, app: &str) -> Result<Option<String>, UninstallerError> {
    let failed = |m: String| UninstallerError::Failed(m);
    let skipped = |why: String| {
        tracing::warn!("{app}: desinstalador não rodou: {why}");
        Ok(Some(format!(
            "o desinstalador do app não rodou: {why}; o que ele instalou fora da pasta pode ter ficado"
        )))
    };
    let Some((rec, _)) = ctx.state.get(app).map_err(|e| failed(e.to_string()))? else {
        return Ok(None);
    };
    check_component("versão", &rec.version).map_err(failed)?;
    let raw = match state::installed_manifest(ctx.layout, app, &rec.version, &rec.manifest_sha256) {
        Ok(Some(raw)) => raw,
        Ok(None) => return skipped("o manifesto instalado não existe".into()),
        Err(e) => {
            return skipped(format!(
                "o manifesto instalado foi alterado ou está ilegível ({e})"
            ));
        }
    };
    let arch = rec
        .arch
        .as_deref()
        .and_then(arch_of_key)
        .unwrap_or_else(Arch::host);
    let manifest = match manifest::resolve(&raw, arch) {
        Ok(m) => m,
        Err(e) => return skipped(format!("o manifesto instalado não pôde ser lido ({e})")),
    };
    let Some(spec) = manifest.uninstaller else {
        return Ok(None);
    };
    let Some(file) = spec.file.as_deref().filter(|_| spec.script.is_none()) else {
        tracing::warn!("{app}: uninstaller sem file (ou com script): nada a rodar");
        return Ok(None);
    };
    let Some(runner) = ctx.runners.installer else {
        return Err(UninstallerError::Consent(ExecError::NeedsScriptConsent {
            app: app.to_string(),
            what: "o desinstalador do manifesto".into(),
        }));
    };
    let vdir = ctx.layout.version_dir(app, &rec.version);
    let args = installer_args::installer_args(&spec.args, &vdir, &rec.version)
        .map_err(|e| failed(e.to_string()))?;
    let program = installer_program(&vdir, file).map_err(failed)?;
    tracing::info!("rodando o desinstalador {file}");
    check_installer_exit(runner(&program, &args, &vdir)).map_err(failed)?;
    Ok(None)
}
