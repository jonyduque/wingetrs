//! One elevated helper per session for machine-wide winget upgrades: the first such job asks
//! for UAC once and starts `wingetrs bridge-worker`; later ones reuse it. A refusal is final
//! for the session (the jobs run unelevated, as before). The Win32 side is `elevate_win`.
use super::worker::{self, DeleteReason, InstallerRequest, MachineHive, Reply, Request};
use super::{BridgeExec, BridgeJob, BridgeOp, Tail, Tool};
use crate::installed::correlate::InstallScope;
use crate::winget_install::uninstall_cmd::Rule;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError, TryLockError};
use std::time::Duration;

/// Progress line of a machine-wide job whose elevation was refused.
pub const REFUSED_NOTE: &str = "sem permissão de administrador: atualizando sem elevar";

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
}

enum State {
    NotStarted,
    Running(Box<dyn Helper>),
    Refused,
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
    /// helper; a refusal is told on `on_line` once and never asked again.
    pub fn exec_for<'a>(
        &'a self,
        job: &'a BridgeJob,
        current: BridgeExec<'a>,
        on_line: &mut dyn FnMut(&str),
    ) -> ChosenExec<'a> {
        let wanted = job.tool == Tool::Winget && job.scope == Some(InstallScope::Machine);
        let elevated = wanted && {
            let mut state = self.lock();
            match self.ensure_started(&mut state) {
                Ok(()) => true,
                Err(None) => false,
                Err(Some(e)) => {
                    tracing::warn!(error = %e, "bridge: elevation refused; running unelevated");
                    on_line(REFUSED_NOTE);
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
    /// (a new UAC prompt). `Err(Some(reason))` when starting failed now (the state becomes
    /// `Refused`), `Err(None)` when it was refused before.
    fn ensure_started(&self, state: &mut State) -> Result<(), Option<String>> {
        if let State::Running(helper) = state
            && !helper.alive()
        {
            tracing::info!("bridge: elevated helper exited; starting a new one");
            *state = State::NotStarted;
        }
        match state {
            State::Running(_) => Ok(()),
            State::Refused => Err(None),
            State::NotStarted => match self.transport.start() {
                Ok(helper) => {
                    tracing::info!("bridge: elevated helper started");
                    *state = State::Running(helper);
                    Ok(())
                }
                Err(e) => {
                    *state = State::Refused;
                    Err(Some(e))
                }
            },
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
            if let Err(reason) = self.ensure_started(state) {
                return Err(Fail::Refused(
                    reason.unwrap_or_else(|| "recusado".to_string()),
                ));
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

/// Why an installer did not run through the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevatedError {
    /// Elevation was refused (now or earlier this session): nothing ran.
    Refused,
    /// The helper was lost; the installer may have run.
    Lost(String),
    /// The helper refused the request or could not run it.
    Remote(String),
}

impl std::fmt::Display for ElevatedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElevatedError::Refused => f.write_str("sem permissão de administrador"),
            ElevatedError::Lost(e) => write!(f, "auxiliar elevado perdido: {e}"),
            ElevatedError::Remote(e) => write!(f, "auxiliar elevado: {e}"),
        }
    }
}

impl std::error::Error for ElevatedError {}

/// Runs a hash-verified installer elevated (Task 180).
pub trait ElevatedRunner: Send + Sync {
    /// The installer's exit code; `on_line` gets the end of its log.
    fn run_installer(
        &self,
        req: &InstallerRequest,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError>;

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
}

impl ElevatedRunner for ElevatedBridge {
    /// Starts the helper when needed (the one UAC prompt); a refusal this session is final.
    fn run_installer(
        &self,
        req: &InstallerRequest,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        let request = Request::RunInstaller {
            source: req.source.clone(),
            sha256: req.sha256.clone(),
            file_name: req.file_name.clone(),
            kind: req.kind,
            params: req.params.clone(),
        };
        self.ask_elevated(&request, on_line)
    }

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
        let request = Request::DeleteArpKey {
            hive,
            key: key.to_string(),
            reason,
        };
        match self.ask_elevated(&request, &mut |_| {})? {
            0 => Ok(()),
            code => Err(ElevatedError::Remote(format!(
                "resposta inesperada {code} ao apagar a chave"
            ))),
        }
    }
}

impl ElevatedBridge {
    /// Sends one request that needs the helper, starting it when needed (the one UAC prompt);
    /// a refusal this session is final. The helper's exit code.
    fn ask_elevated(
        &self,
        request: &Request,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<i32, ElevatedError> {
        let mut state = self.lock();
        match self.ensure_started(&mut state) {
            Ok(()) => {}
            Err(None) => return Err(ElevatedError::Refused),
            Err(Some(e)) => {
                tracing::warn!(error = %e, "bridge: elevation refused");
                return Err(ElevatedError::Refused);
            }
        }
        match self.ask_running(&mut state, request, on_line) {
            Ok(Ok((code, _))) => Ok(code),
            Ok(Err(e)) => Err(ElevatedError::Remote(e.to_string())),
            Err(Fail::Lost(e)) => Err(ElevatedError::Lost(e.to_string())),
            Err(Fail::Refused(e)) => {
                tracing::warn!(error = %e, "bridge: elevation refused");
                Err(ElevatedError::Refused)
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
        super::elevate_win::start(&self.exe)
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
