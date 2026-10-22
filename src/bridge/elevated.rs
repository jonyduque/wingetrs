//! One elevated helper per session for machine-wide winget upgrades, HKLM uninstallers and ARP
//! key deletions: the first such job asks for UAC once and starts `wingetrs bridge-worker`;
//! later ones reuse it. A refusal is final for the session (the jobs run unelevated, as
//! before). It runs native installers only as `InstallPackage` (Task 202): the helper derives
//! the installer from the signed catalog. The Win32 side is `elevate_win`.
use super::helper_folder::is_refusal;
use super::worker::{self, DeleteReason, MachineHive, PackageRequest, Reply, Request};
use super::{BridgeExec, BridgeJob, BridgeOp, Tail, Tool};
use crate::install::progress::{ELEVATION_GRANTED, ELEVATION_NOTICE, ELEVATION_REFUSED};
use crate::installed::correlate::InstallScope;
use crate::winget_install::uninstall_cmd::Rule;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError, TryLockError};
use std::time::Duration;

/// Progress line of a machine-wide job whose elevation was refused (starts with
/// `progress::ELEVATION_REFUSED`, as every refused UAC prompt).
pub const REFUSED_NOTE: &str = "Permissão de administrador recusada. Atualizando sem elevar.";

/// The channel to a started helper.
pub trait Helper: Send {
    fn reader(&mut self) -> &mut dyn BufRead;
    fn writer(&mut self) -> &mut dyn Write;
    /// Waits at most `limit` for the helper to exit; whether it did.
    fn wait_exit(&mut self, limit: Duration) -> bool;
    /// Whether the helper process is still running (it exits on its own after a while idle).
    fn alive(&mut self) -> bool;
}

/// Starts the helper (the one UAC prompt) and connects to it; `Err` carries the reason.
pub trait Transport: Send + Sync {
    fn start(&self) -> Result<Box<dyn Helper>, String>;

    /// `start`, calling `notice` right before the UAC prompt opens (Task 201), never when
    /// starting is refused before any prompt. By default `notice` comes right before `start`.
    fn start_with_notice(&self, notice: &mut dyn FnMut()) -> Result<Box<dyn Helper>, String> {
        notice();
        self.start()
    }
}

enum State {
    NotStarted,
    Running(Box<dyn Helper>),
    /// Why it could not start (a refused UAC, or `helper_folder::refusal_text`).
    Refused(String),
}

/// The session's helper and whether it may still be started.
pub struct ElevatedBridge {
    transport: Box<dyn Transport>,
    state: Mutex<State>,
}

/// An owned `BridgeExec`.
type BoxedExec<'a> = Box<
    dyn FnMut(&str, &[String], &mut dyn FnMut(&str)) -> std::io::Result<(i32, Vec<String>)> + 'a,
>;

/// The exec a job runs with, and whether it goes through the helper.
pub struct ChosenExec<'a> {
    elevated: bool,
    run: BoxedExec<'a>,
}

impl ChosenExec<'_> {
    pub fn elevated(&self) -> bool {
        self.elevated
    }

    pub fn exec(&mut self) -> BridgeExec<'_> {
        &mut *self.run
    }
}

fn lost(message: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(format!("auxiliar elevado: {message}"))
}

impl ElevatedBridge {
    pub fn new(transport: Box<dyn Transport>) -> ElevatedBridge {
        ElevatedBridge {
            transport,
            state: Mutex::new(State::NotStarted),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The exec for `job`: the helper for a winget job installed for the whole machine, unless
    /// elevation was refused this session; `current` otherwise. The first such job starts the
    /// helper: `progress::ELEVATION_NOTICE` on `on_line` right before the UAC prompt, then
    /// `ELEVATION_GRANTED` once accepted; a refusal is told on `on_line` once and never asked
    /// again.
    pub fn exec_for<'a>(
        &'a self,
        job: &'a BridgeJob,
        current: BridgeExec<'a>,
        on_line: &mut dyn FnMut(&str),
    ) -> ChosenExec<'a> {
        let wanted = job.tool == Tool::Winget && job.scope == Some(InstallScope::Machine);
        let elevated = wanted && {
            let mut state = self.lock();
            match self.ensure_started(&mut state, on_line) {
                Ok(started) => {
                    if started {
                        on_line(ELEVATION_GRANTED);
                    }
                    true
                }
                Err((_, false)) => false,
                Err((e, true)) => {
                    tracing::warn!(error = %e, "bridge: elevation refused; running unelevated");
                    // The folder refusal says itself why (never "no permission").
                    on_line(if is_refusal(&e) { &e } else { REFUSED_NOTE });
                    false
                }
            }
        };
        if !elevated {
            return ChosenExec {
                elevated,
                run: Box::new(current),
            };
        }
        ChosenExec {
            elevated,
            run: Box::new(
                move |program: &str, args: &[String], on_line: &mut dyn FnMut(&str)| {
                    self.run_elevated(job, program, args, on_line)
                },
            ),
        }
    }

    /// Makes `state` `Running`: a helper that exited (idle) is dropped and a new one started
    /// (a new UAC prompt, announced on `notice` with `progress::ELEVATION_NOTICE` right before
    /// it). `Ok(true)` when a helper was started now. `Err((reason, true))` when starting
    /// failed now (the state becomes `Refused`), `Err((reason, false))` when it was refused
    /// before.
    fn ensure_started(
        &self,
        state: &mut State,
        notice: &mut dyn FnMut(&str),
    ) -> Result<bool, (String, bool)> {
        if let State::Running(helper) = state
            && !helper.alive()
        {
            tracing::info!("bridge: elevated helper exited; starting a new one");
            *state = State::NotStarted;
        }
        match state {
            State::Running(_) => Ok(false),
            State::Refused(reason) => Err((reason.clone(), false)),
            State::NotStarted => {
                match self
                    .transport
                    .start_with_notice(&mut || notice(ELEVATION_NOTICE))
                {
                    Ok(helper) => {
                        tracing::info!("bridge: elevated helper started");
                        *state = State::Running(helper);
                        Ok(true)
                    }
                    Err(e) => {
                        *state = State::Refused(e.clone());
                        Err((e, true))
                    }
                }
            }
        }
    }

    /// Runs one command of `job` through the helper. When the request cannot even be sent
    /// (the helper died before winget started), a new helper is started and the request sent
    /// once more; a helper lost after that is dropped, so the next machine-wide job starts a
    /// new one.
    fn run_elevated(
        &self,
        job: &BridgeJob,
        program: &str,
        args: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<(i32, Vec<String>)> {
        let request = request_for(job, program, args)?;
        let mut state = self.lock();
        match self.ask_running(&mut state, &request, on_line) {
            Ok(answer) => answer,
            Err(Fail::Lost(e)) => Err(e),
            Err(Fail::Refused(reason)) if is_refusal(&reason) => Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                reason,
            )),
            Err(Fail::Refused(reason)) => Err(lost(reason)),
        }
    }

    /// Sends `request` to the running helper in `state` and relays its answer. When the
    /// request cannot even be sent, a new helper is started and it is sent once more; a helper
    /// lost after that is dropped, so the next request starts a new one.
    fn ask_running(
        &self,
        state: &mut State,
        request: &Request,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<std::io::Result<(i32, Vec<String>)>, Fail> {
        let mut retried = false;
        loop {
            let State::Running(helper) = state else {
                return Err(Fail::Lost(lost("não está rodando")));
            };
            let e = match ask(helper.as_mut(), request, on_line) {
                Ok(answer) => return Ok(answer),
                Err(Lost::Unsent(e)) if !retried => e,
                Err(Lost::Unsent(e) | Lost::Midway(e)) => {
                    tracing::warn!(error = %e, "bridge: elevated helper lost");
                    *state = State::NotStarted;
                    return Err(Fail::Lost(e));
                }
            };
            retried = true;
            tracing::warn!(error = %e, "bridge: request not sent; starting a new helper");
            *state = State::NotStarted;
            match self.ensure_started(state, on_line) {
                Ok(_) => on_line(ELEVATION_GRANTED),
                Err((reason, _)) => return Err(Fail::Refused(reason)),
            }
        }
    }

    /// Asks a running helper to quit and waits at most `limit` for it; whether nothing is left
    /// running. A helper busy with a job is left to finish it (it exits with this process).
    pub fn shutdown(&self, limit: Duration) -> bool {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => {
                tracing::warn!("bridge: elevated helper busy at exit; not waited for");
                return false;
            }
        };
        let State::Running(mut helper) = std::mem::replace(&mut *state, State::NotStarted) else {
            return true;
        };
        let quit = worker::encode(&Request::Quit {}).and_then(|line| {
            helper.writer().write_all(line.as_bytes())?;
            helper.writer().flush()
        });
        if let Err(e) = quit {
            tracing::warn!(error = %e, "bridge: could not ask the elevated helper to quit");
        }
        let exited = helper.wait_exit(limit);
        if !exited {
            tracing::warn!(
                ?limit,
                "bridge: elevated helper did not exit in time; left running"
            );
        }
        exited
    }
}

/// Why a request did not reach a helper's answer.
enum Fail {
    /// The channel broke (see `Lost`).
    Lost(std::io::Error),
    /// A new helper was needed and elevation was refused.
    Refused(String),
}

/// Why an operation did not run through the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevatedError {
    /// Elevation was refused (now or earlier this session): nothing ran.
    Refused,
    /// The helper was lost; the installer may have run.
    Lost(String),
    /// The helper refused the request or could not run it.
    Remote(String),
    /// No UAC prompt: wingetrs is not in an administrator-only folder (Task 197). Shown as
    /// `helper_folder::refusal_text` alone, never as a refused UAC.
    NotAdminFolder(String),
}

impl ElevatedError {
    /// From why the helper could not start: the folder refusal keeps its own text.
    fn refused(reason: String) -> ElevatedError {
        if is_refusal(&reason) {
            ElevatedError::NotAdminFolder(reason)
        } else {
            ElevatedError::Refused
        }
    }

    /// `"<prefix>: <self>"`, or the folder refusal alone (it already says everything).
    pub fn context(&self, prefix: impl std::fmt::Display) -> String {
        match self {
            ElevatedError::NotAdminFolder(text) => text.clone(),
            other => format!("{prefix}: {other}"),
        }
    }
}

impl std::fmt::Display for ElevatedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElevatedError::NotAdminFolder(text) => f.write_str(text),
            ElevatedError::Refused => f.write_str(ELEVATION_REFUSED),
            ElevatedError::Lost(e) => write!(f, "auxiliar elevado perdido: {e}"),
            ElevatedError::Remote(e) => write!(f, "auxiliar elevado: {e}"),
        }
    }
}

impl std::error::Error for ElevatedError {}

/// What the native flows ask the elevated helper (Task 182): uninstallers and ARP keys of
/// HKLM, and winget packages it installs from the signed catalog (Task 202).
pub trait ElevatedRunner: Send + Sync {
    /// Runs the uninstaller of an HKLM ARP entry (Task 182): the helper recomputes the command
    /// from the key and refuses when its rule is not `rule`. The uninstaller's exit code.
    fn run_uninstaller(
        &self,
        _hive: MachineHive,
        _key: &str,
        _rule: Rule,
        _nsis_hint: bool,
    ) -> Result<i32, ElevatedError> {
        Err(ElevatedError::Remote(
            "desinstalação elevada não suportada".to_string(),
        ))
    }

    /// `run_uninstaller`, with the helper's progress lines (`worker::WAITING_UNINSTALLER` once
    /// the uninstaller exited and the helper waits for what it left running) sent to
    /// `on_line`. By default, no line.
    fn run_uninstaller_lines(
        &self,
        hive: MachineHive,
        key: &str,
        rule: Rule,
        nsis_hint: bool,
        _on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        self.run_uninstaller(hive, key, rule, nsis_hint)
    }

    /// Deletes an HKLM ARP key once the helper confirms `reason` again (Task 182).
    fn delete_arp_key(
        &self,
        _hive: MachineHive,
        _key: &str,
        _reason: DeleteReason,
    ) -> Result<(), ElevatedError> {
        Err(ElevatedError::Remote(
            "exclusão elevada de chave não suportada".to_string(),
        ))
    }

    /// `delete_arp_key`, with the helper's lines (`progress::ELEVATION_NOTICE` before its UAC
    /// prompt, `ELEVATION_GRANTED` once it runs) sent to `on_line`. By default, no line.
    fn delete_arp_key_lines(
        &self,
        hive: MachineHive,
        key: &str,
        reason: DeleteReason,
        _on_line: &mut dyn FnMut(&str),
    ) -> Result<(), ElevatedError> {
        self.delete_arp_key(hive, key, reason)
    }

    /// Installs a winget package through the helper (Task 202), which derives the installer
    /// from the signed catalog itself; its lines (`progress::ELEVATION_NOTICE` before its UAC
    /// prompt, `ELEVATION_GRANTED`, `signed_install`'s progress) go to `on_line`. The
    /// installer's exit code. `NotAdminFolder` means the helper cannot exist this session.
    fn install_package_lines(
        &self,
        _req: &PackageRequest,
        _on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        Err(ElevatedError::Remote(
            "instalação elevada não suportada".to_string(),
        ))
    }
}

impl ElevatedRunner for ElevatedBridge {
    fn run_uninstaller(
        &self,
        hive: MachineHive,
        key: &str,
        rule: Rule,
        nsis_hint: bool,
    ) -> Result<i32, ElevatedError> {
        self.run_uninstaller_lines(hive, key, rule, nsis_hint, &mut |_| {})
    }

    /// The helper's lines (waiting for processes the uninstaller left, a folder kept) go to
    /// the log and to `on_line`.
    fn run_uninstaller_lines(
        &self,
        hive: MachineHive,
        key: &str,
        rule: Rule,
        nsis_hint: bool,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        let request = Request::RunUninstaller {
            hive,
            key: key.to_string(),
            rule,
            nsis_hint,
        };
        self.ask_elevated(&request, &mut |l: &str| {
            tracing::info!(line = l, "elevated uninstaller");
            on_line(l);
        })
    }

    fn delete_arp_key(
        &self,
        hive: MachineHive,
        key: &str,
        reason: DeleteReason,
    ) -> Result<(), ElevatedError> {
        self.delete_arp_key_lines(hive, key, reason, &mut |_| {})
    }

    fn delete_arp_key_lines(
        &self,
        hive: MachineHive,
        key: &str,
        reason: DeleteReason,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), ElevatedError> {
        let request = Request::DeleteArpKey {
            hive,
            key: key.to_string(),
            reason,
        };
        match self.ask_elevated(&request, on_line)? {
            0 => Ok(()),
            code => Err(ElevatedError::Remote(format!(
                "resposta inesperada {code} ao apagar a chave"
            ))),
        }
    }

    fn install_package_lines(
        &self,
        req: &PackageRequest,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        let request = Request::InstallPackage {
            id: req.id.clone(),
            version: req.version.clone(),
            installer: req.installer,
            scope: req.scope,
            mode: req.mode,
            upgrade: req.upgrade,
            plan_sha256: req.plan_sha256.clone(),
        };
        self.ask_elevated(&request, &mut |l: &str| {
            tracing::info!(line = l, "elevated install");
            on_line(l);
        })
    }
}

impl ElevatedBridge {
    /// Sends one request that needs the helper, starting it when needed (the one UAC prompt,
    /// `progress::ELEVATION_NOTICE` on `on_line` right before it); a refusal this session is
    /// final. `ELEVATION_GRANTED` goes to `on_line` once the helper runs (started now or
    /// already), so the caller's phase leaves the notice. The helper's exit code.
    fn ask_elevated(
        &self,
        request: &Request,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        let mut state = self.lock();
        if let Err((e, now)) = self.ensure_started(&mut state, on_line) {
            if now {
                tracing::warn!(error = %e, "bridge: elevation refused");
            }
            return Err(ElevatedError::refused(e));
        }
        on_line(ELEVATION_GRANTED);
        match self.ask_running(&mut state, request, on_line) {
            Ok(Ok((code, _))) => Ok(code),
            Ok(Err(e)) => Err(ElevatedError::Remote(e.to_string())),
            Err(Fail::Lost(e)) => Err(ElevatedError::Lost(e.to_string())),
            Err(Fail::Refused(e)) => {
                tracing::warn!(error = %e, "bridge: elevation refused");
                Err(ElevatedError::refused(e))
            }
        }
    }
}

/// The request for `program args`, which must be one of the two commands of `job` (its
/// upgrade or install, or the source update); anything else is `PermissionDenied`.
pub fn request_for(job: &BridgeJob, program: &str, args: &[String]) -> std::io::Result<Request> {
    let is = |(p, a): (String, Vec<String>)| p == program && a == args;
    if job.tool == Tool::Winget && is(super::command(job, Path::new(""), "")) {
        let target = job.target.clone();
        return Ok(match &job.op {
            BridgeOp::Upgrade => Request::Upgrade { target },
            BridgeOp::Install { scope, .. } => Request::Install {
                target,
                scope: scope.map(Into::into),
            },
        });
    }
    if is(super::winget_source_update()) {
        return Ok(Request::SourceUpdate {});
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!("comando fora do protocolo do auxiliar elevado: {program}"),
    ))
}

/// A broken channel to the helper.
enum Lost {
    /// The request was not sent: nothing ran.
    Unsent(std::io::Error),
    /// Broke after the request went out: the tool may have run.
    Midway(std::io::Error),
}

/// Sends `request` and relays the helper's lines until its answer. The outer `Err` is a broken
/// channel; the inner result is the helper's own answer.
fn ask(
    helper: &mut dyn Helper,
    request: &Request,
    on_line: &mut dyn FnMut(&str),
) -> Result<std::io::Result<(i32, Vec<String>)>, Lost> {
    let line = match worker::encode(request) {
        Ok(line) => line,
        Err(e) => return Ok(Err(e)),
    };
    let sent = helper
        .writer()
        .write_all(line.as_bytes())
        .and_then(|()| helper.writer().flush());
    sent.map_err(|e| Lost::Unsent(lost(e)))?;
    let midway = |e: std::io::Error| Lost::Midway(lost(e));
    let mut tail = Tail::new();
    loop {
        let Some(line) = worker::read_line(helper.reader()).map_err(midway)? else {
            return Err(midway(std::io::Error::other("fechou o canal")));
        };
        match worker::decode::<Reply>(&line).map_err(midway)? {
            Reply::Line { text } => {
                on_line(&text);
                tail.add(text);
            }
            Reply::End { code } => return Ok(Ok((code, tail.lines()))),
            Reply::Error { message, not_found } => {
                let kind = if not_found {
                    std::io::ErrorKind::NotFound
                } else {
                    std::io::ErrorKind::Other
                };
                return Ok(Err(std::io::Error::new(kind, message)));
            }
        }
    }
}

/// The helper of this system: `wingetrs bridge-worker` started elevated through UAC.
pub struct SystemTransport {
    #[cfg_attr(not(windows), allow(dead_code))]
    exe: PathBuf,
}

impl SystemTransport {
    /// `exe` is the wingetrs executable the helper runs.
    pub fn new(exe: PathBuf) -> SystemTransport {
        SystemTransport { exe }
    }
}

#[cfg(windows)]
impl Transport for SystemTransport {
    fn start(&self) -> Result<Box<dyn Helper>, String> {
        self.start_with_notice(&mut || {})
    }

    /// The notice comes after the folder check, right before `ShellExecuteEx` opens the
    /// prompt: a folder refusal shows no notice.
    fn start_with_notice(&self, notice: &mut dyn FnMut()) -> Result<Box<dyn Helper>, String> {
        super::elevate_win::start_noticed(&self.exe, notice)
            .map(|h| Box::new(h) as Box<dyn Helper>)
            .map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
impl Transport for SystemTransport {
    fn start(&self) -> Result<Box<dyn Helper>, String> {
        Err("a elevação só existe no Windows".to_string())
    }
}

/// `wingetrs bridge-worker`: connects to the TUI's pipe and serves it until it quits.
#[cfg(windows)]
pub fn run_worker(pipe: &str, parent: u32) -> std::io::Result<()> {
    super::elevate_win::worker_main(pipe, parent)
}

#[cfg(not(windows))]
pub fn run_worker(_pipe: &str, _parent: u32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "o auxiliar elevado só existe no Windows",
    ))
}
