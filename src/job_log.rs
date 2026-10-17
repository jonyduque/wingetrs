//! The `job.start` / `job.end` log lines every job writes (target `wingetrs::job`), shared by
//! the TUI jobs and the CLI so the format lives in one place.
use crate::install::plan::InstallPlan;
use std::time::Instant;

/// Output lines kept in the `job.end` line of a job that did not succeed.
pub const END_TAIL: usize = 40;

/// How a job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    /// The tool ran and said success, but the app did not change as expected.
    NoEffect,
    Failed,
    Cancelled,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::NoEffect => "sem_efeito",
            Outcome::Failed => "falhou",
            Outcome::Cancelled => "cancelado",
        }
    }
}

/// What the `job.start` line says.
#[derive(Debug, Clone)]
pub struct JobInfo {
    /// `bridge`, `install`, `remove` or `upgrade`.
    pub kind: &'static str,
    /// `winget`, `scoop` or `wingetrs`.
    pub tool: &'static str,
    pub target: String,
    pub name: String,
    pub from: Option<String>,
    pub to: Option<String>,
    /// The exact command line of a bridge job.
    pub cmd: Option<String>,
}

impl JobInfo {
    /// A job of the own installer over `plans` (the last one is the app the user asked for).
    pub fn for_plans<'a>(
        kind: &'static str,
        plans: impl IntoIterator<Item = &'a InstallPlan>,
    ) -> JobInfo {
        let plans: Vec<&InstallPlan> = plans.into_iter().collect();
        let last = plans.last();
        JobInfo {
            kind,
            tool: "wingetrs",
            target: last
                .map(|p| format!("{}/{}", p.bucket, p.app))
                .unwrap_or_default(),
            name: plans
                .iter()
                .map(|p| p.app.as_str())
                .collect::<Vec<_>>()
                .join(","),
            from: last.and_then(|p| p.upgrade_from.clone()),
            to: last.map(|p| p.version.clone()),
            cmd: None,
        }
    }

    /// Whether any plan replaces an installed version.
    pub fn plans_kind<'a>(plans: impl IntoIterator<Item = &'a InstallPlan>) -> &'static str {
        if plans.into_iter().any(|p| p.upgrade_from.is_some()) {
            "upgrade"
        } else {
            "install"
        }
    }
}

/// A started job. `finish` writes `job.end`; a guard dropped without it (the job was dropped
/// or panicked) writes `cancelado`/`falhou` instead, so every `job.start` has its end.
pub struct JobLog {
    info: JobInfo,
    started: Instant,
    done: bool,
    /// Exit code of the tool, for bridge jobs.
    pub code: Option<i32>,
    /// Version found after the run, when known.
    pub after: Option<String>,
    /// Output of the tool; the last `END_TAIL` lines go to `job.end` unless the job succeeded.
    pub lines: Vec<String>,
    /// Path of the installer's own log, when the output named one.
    pub installer_log: Option<String>,
    /// Outcome to use when the job returns an error (default `Failed`).
    pub failure: Outcome,
}

impl JobLog {
    pub fn start(info: JobInfo) -> JobLog {
        tracing::info!(
            target: "wingetrs::job",
            kind = %info.kind,
            tool = %info.tool,
            target = %info.target,
            name = %info.name,
            from = %dash(&info.from),
            to = %dash(&info.to),
            cmd = %dash(&info.cmd),
            "job.start"
        );
        JobLog {
            info,
            started: Instant::now(),
            done: false,
            code: None,
            after: None,
            lines: Vec::new(),
            installer_log: None,
            failure: Outcome::Failed,
        }
    }

    /// Writes `job.end` (INFO when `outcome` is `Ok`, WARN otherwise).
    pub fn finish(mut self, outcome: Outcome, detail: &str) {
        self.write_end(outcome, detail);
    }

    fn write_end(&mut self, outcome: Outcome, detail: &str) {
        self.done = true;
        let i = &self.info;
        let ms = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let code = self.code.map_or_else(|| "-".to_string(), |c| c.to_string());
        let after = dash(&self.after);
        let installer_log = dash(&self.installer_log);
        let outcome_text = outcome.as_str();
        if outcome == Outcome::Ok {
            tracing::info!(
                target: "wingetrs::job",
                kind = %i.kind, tool = %i.tool, target = %i.target, name = %i.name,
                outcome = %outcome_text, code = %code, ms, after = %after,
                installer_log = %installer_log, detail = %detail,
                "job.end"
            );
        } else {
            let tail = self.lines[self.lines.len().saturating_sub(END_TAIL)..].join("\n");
            tracing::warn!(
                target: "wingetrs::job",
                kind = %i.kind, tool = %i.tool, target = %i.target, name = %i.name,
                outcome = %outcome_text, code = %code, ms, after = %after,
                installer_log = %installer_log, detail = %detail, output = %tail,
                "job.end"
            );
        }
    }
}

impl Drop for JobLog {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        if std::thread::panicking() {
            self.write_end(Outcome::Failed, "o job terminou em pânico");
        } else {
            self.write_end(Outcome::Cancelled, "o job foi interrompido");
        }
    }
}

fn dash(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("-")
}
