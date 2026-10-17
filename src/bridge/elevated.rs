//! One elevated helper per session for machine-wide winget upgrades: the first such job asks
//! for UAC once and starts `wingetrs bridge-worker`; later ones reuse it. A refusal is final
//! for the session (the jobs run unelevated, as before). The Win32 side is `elevate_win`.
use super::worker::{self, Reply, Request};
use super::{BridgeExec, BridgeJob, Tail, Tool};
use crate::installed::correlate::InstallScope;
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
            match &*state {
                State::Running(_) => true,
                State::Refused => false,
                State::NotStarted => match self.transport.start() {
                    Ok(helper) => {
                        tracing::info!("bridge: elevated helper started");
                        *state = State::Running(helper);
                        true
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "bridge: elevation refused; running unelevated");
                        *state = State::Refused;
                        on_line(REFUSED_NOTE);
                        false
                    }
                },
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

    /// Runs one command of `job` through the helper. A broken helper is dropped, so the next
    /// machine-wide job starts a new one.
    fn run_elevated(
        &self,
        job: &BridgeJob,
        program: &str,
        args: &[String],
        on_line: &mut dyn FnMut(&str),
    ) -> std::io::Result<(i32, Vec<String>)> {
        let request = request_for(job, program, args)?;
        let mut state = self.lock();
        let State::Running(helper) = &mut *state else {
            return Err(lost("não está rodando"));
        };
        match ask(helper.as_mut(), &request, on_line) {
            Ok(answer) => answer,
            Err(e) => {
                tracing::warn!(error = %e, "bridge: elevated helper lost");
                *state = State::NotStarted;
                Err(e)
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

/// The request for `program args`, which must be one of the two commands of `job`.
fn request_for(job: &BridgeJob, program: &str, args: &[String]) -> std::io::Result<Request> {
    let is = |(p, a): (String, Vec<String>)| p == program && a == args;
    if is(super::command(job, Path::new(""), "")) {
        return Ok(Request::Upgrade {
            target: job.target.clone(),
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

/// Sends `request` and relays the helper's lines until its answer. The outer `Err` is a broken
/// channel; the inner result is the helper's own answer.
fn ask(
    helper: &mut dyn Helper,
    request: &Request,
    on_line: &mut dyn FnMut(&str),
) -> std::io::Result<std::io::Result<(i32, Vec<String>)>> {
    let line = worker::encode(request)?;
    let sent = helper
        .writer()
        .write_all(line.as_bytes())
        .and_then(|()| helper.writer().flush());
    sent.map_err(lost)?;
    let mut tail = Tail::new();
    loop {
        let Some(line) = worker::read_line(helper.reader()).map_err(lost)? else {
            return Err(lost("fechou o canal"));
        };
        match worker::decode::<Reply>(&line).map_err(lost)? {
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
