//! Typed progress of an install: download bytes and the phase each slow step enters. Every
//! consumer matches [`ProgressEvent`] and [`Phase`] exhaustively, so a new phase cannot be
//! silently dropped by one of them (the `INSTALLER_RUNNING` label was, see ARMADILHAS).
use crate::tui::theme::Glyphs;
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Shown, the same in the CLI and the TUI, before every UAC prompt wingetrs opens or a
/// self-elevating installer opens (Task 201), with the clock running until it is answered.
pub const ELEVATION_NOTICE: &str = "Aguardando autorização para executar como administrador";
/// A refused UAC prompt (the CLI and the TUI say it the same way).
pub const ELEVATION_REFUSED: &str = "Permissão de administrador recusada.";
/// Progress line of the elevated helper once its UAC prompt was accepted (or the session's
/// helper already runs): the phase leaves [`ELEVATION_NOTICE`].
pub const ELEVATION_GRANTED: &str = "permissão de administrador concedida";
/// How long a self-elevating installer's phase keeps [`ELEVATION_NOTICE`]: its own UAC prompt
/// cannot be observed, so after this the line says it may still ask (Task 201).
pub const SELF_ELEVATION_WAIT: Duration = Duration::from_secs(120);
/// [`Phase::SelfElevating`] past [`SELF_ELEVATION_WAIT`].
pub const SELF_ELEVATION_LATER: &str =
    "rodando o instalador (pode pedir autorização de administrador)";

/// One report through `download::Progress`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressEvent {
    /// Bytes of one download so far.
    Download {
        url: String,
        done: u64,
        total: Option<u64>,
    },
    /// A step that may take a while has just started.
    Phase(Phase),
}

/// What the install is doing between downloads; sent when each step starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Lock, revalidation, cache checks, launcher.
    Preparing,
    /// `msi.lock` or the Windows Installer mutex is held by another install (or a 1618 retry).
    WaitingInstaller,
    /// Own extraction of `file` (`engine`: the format, e.g. `MSI`).
    Extracting { file: String, engine: String },
    /// `msiexec /a` launched, writing its verbose log at `log` (cannot be cancelled).
    Installer { log: PathBuf },
    /// Moving the extracted version (or the `msiexec /a` tree) into place and its persisted data.
    Placing,
    /// `current`, commands, shortcuts and environment.
    Linking,
    /// Writing the install record.
    Recording,
    /// Uninstall: removing `app`'s artifacts and folder.
    Removing { app: String },
    /// Uninstall: looking for leftovers in the registry and on disk.
    Scanning,
    /// Native winget install: the installer's own UAC prompt is open (Task 196); native
    /// removal: the elevated helper's UAC, then the uninstaller. Already launched, cannot be
    /// cancelled.
    WaitingElevation,
    /// Native winget install: the installer of `name` is running (cannot be cancelled).
    RunningInstaller { name: String },
    /// Native winget install: looking for the app among the installed ones.
    Verifying,
    /// Native removal: copying the data folders another installation shares to the backups.
    CopyingFolders,
    /// Native removal: the uninstaller of `name` is running (cannot be cancelled).
    Uninstalling { name: String },
    /// Native removal: waiting (up to two minutes) for the record of `name` to go away.
    CheckingRemoval { name: String },
    /// Native winget install: the installer of `name` runs unelevated and asks for its own UAC
    /// (`ElevationRequirement` `elevatesSelf`/`elevationRequired`); cannot be cancelled.
    SelfElevating { name: String },
    /// Bridge: `tool` (winget or scoop) is running the job; cannot be cancelled.
    Bridge { tool: String },
    /// Scoop: the manifest's `hook` script (`pre_install`, `uninstaller.script`…) is running
    /// in PowerShell; cannot be cancelled.
    RunningScript { hook: String },
    /// Scoop: the package's own installer (`installer.file`) or old uninstaller `file` is
    /// running; cannot be cancelled.
    RunningPackageInstaller { file: String },
    /// `wingetrs self update` (Task 213): one of its steps.
    SelfUpdate(SelfUpdateStep),
    /// `update`: another process holds the update lock and this one waits for it.
    WaitingUpdate,
    /// `update`: downloading the `sources` catalogs that are not up to date.
    FetchingCatalog { sources: usize },
    /// `update`: writing the downloaded catalogs into the local catalog.
    WritingCatalog,
    /// `list`: reading the installed apps (registry, winget, Scoop).
    ReadingInstalled,
    /// `list`: matching the installed apps against the catalog.
    Correlating,
}

/// The steps of `wingetrs self update` (the download itself is `ProgressEvent::Download`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfUpdateStep {
    /// Asking GitHub for the latest tag.
    CheckingVersion,
    /// Hashing the downloaded executable.
    CheckingHash,
    /// Renaming the running executable to `.old` and moving the new one in.
    Swapping,
    /// The elevated copy downloads, checks and swaps (cannot be cancelled).
    Elevated,
}

/// Format name shown next to the file being extracted.
pub fn engine_of(file_name: &str) -> String {
    use crate::install::archive::ArchiveKind as K;
    match K::of(file_name) {
        Ok(K::Zip) => "ZIP",
        Ok(K::TarGz) => "tar.gz",
        Ok(K::SevenZip) => "7z",
        Ok(K::Msi) => "MSI",
        Ok(K::Nupkg) => "nupkg",
        Ok(K::TarXz) => "tar.xz",
        Ok(K::Cab) => "CAB",
        Ok(K::Plain) => "cópia",
        Err(ext) => ext,
    }
    .to_string()
}

/// Short title of `phase` (no clock): `Extraindo x.msi (MSI)`.
pub fn phase_title(phase: &Phase) -> String {
    match phase {
        Phase::Preparing => "Preparando".into(),
        Phase::WaitingInstaller => {
            "Aguardando o Windows Installer (outro instalador em uso)".into()
        }
        Phase::Extracting { file, engine } => format!("Extraindo {file} ({engine})"),
        Phase::Installer { .. } => "Windows Installer (msiexec /a)".into(),
        Phase::Placing => "Colocando os arquivos".into(),
        Phase::Linking => "Criando comandos".into(),
        Phase::Recording => "Registrando".into(),
        Phase::Removing { app } => format!("Removendo {app}"),
        Phase::Scanning => "Procurando resíduos".into(),
        Phase::WaitingElevation => format!(
            "{ELEVATION_NOTICE} (se o UAC não aparecer, clique no escudo piscando na barra de tarefas)"
        ),
        Phase::RunningInstaller { name } => {
            format!("Instalador de {name} rodando (pode abrir janelas)")
        }
        Phase::Verifying => "Conferindo a instalação".into(),
        Phase::CopyingFolders => "Copiando as pastas de dados para backups".into(),
        Phase::Uninstalling { name } => {
            format!("Desinstalador de {name} rodando (pode abrir janelas)")
        }
        Phase::CheckingRemoval { name } => {
            format!("Conferindo a remoção de {name} (até 2 min)")
        }
        Phase::SelfElevating { name } => {
            format!("{ELEVATION_NOTICE} · instalador de {name} rodando (pode abrir janelas)")
        }
        Phase::Bridge { tool } => format!("{tool} rodando (pode abrir janelas)"),
        Phase::RunningScript { hook } => format!("Rodando o script {hook} do pacote"),
        Phase::RunningPackageInstaller { file } => {
            format!("Rodando o instalador do pacote ({file}; pode abrir janelas)")
        }
        Phase::SelfUpdate(step) => match step {
            SelfUpdateStep::CheckingVersion => "Conferindo versão".into(),
            SelfUpdateStep::CheckingHash => "Conferindo SHA-256".into(),
            SelfUpdateStep::Swapping => "Trocando o executável".into(),
            SelfUpdateStep::Elevated => "Atualizando como administrador".into(),
        },
        Phase::WaitingUpdate => "Aguardando outra atualização do catálogo".into(),
        Phase::FetchingCatalog { sources: 1 } => "Baixando o catálogo".into(),
        Phase::FetchingCatalog { sources } => format!("Baixando os catálogos ({sources} fontes)"),
        Phase::WritingCatalog => "Gravando o catálogo".into(),
        Phase::ReadingInstalled => "Lendo os apps instalados".into(),
        Phase::Correlating => "Comparando com o catálogo".into(),
    }
}

/// `mm:ss` (minutes keep growing past 59).
pub fn clock(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// Plain text of `phase` with its clock, shared by the CLI line and the TUI status bar:
/// `Extraindo x.msi (MSI) · 00:04`.
pub fn phase_text(phase: &Phase, elapsed: Duration) -> String {
    let title = match phase {
        Phase::SelfElevating { name } if elapsed >= SELF_ELEVATION_WAIT => {
            format!("{name}: {SELF_ELEVATION_LATER}")
        }
        _ => phase_title(phase),
    };
    format!("{title} · {}", clock(elapsed))
}

/// [`phase_text`] plus what the phase means for the user: the Windows Installer cannot be
/// cancelled. The TUI status bar, and the CLI line before the installer log says more.
pub fn phase_status(phase: &Phase, elapsed: Duration) -> String {
    let text = phase_text(phase, elapsed);
    match phase {
        Phase::Installer { .. }
        | Phase::RunningInstaller { .. }
        | Phase::WaitingElevation
        | Phase::Uninstalling { .. }
        | Phase::CheckingRemoval { .. }
        | Phase::SelfElevating { .. }
        | Phase::Bridge { .. }
        | Phase::RunningScript { .. }
        | Phase::RunningPackageInstaller { .. }
        | Phase::SelfUpdate(SelfUpdateStep::Elevated) => format!("{text} · não pode ser cancelado"),
        Phase::Preparing
        | Phase::WaitingInstaller
        | Phase::Extracting { .. }
        | Phase::Placing
        | Phase::Linking
        | Phase::Recording
        | Phase::Removing { .. }
        | Phase::Scanning
        | Phase::Verifying
        | Phase::CopyingFolders
        | Phase::SelfUpdate(
            SelfUpdateStep::CheckingVersion
            | SelfUpdateStep::CheckingHash
            | SelfUpdateStep::Swapping,
        )
        | Phase::WaitingUpdate
        | Phase::FetchingCatalog { .. }
        | Phase::WritingCatalog
        | Phase::ReadingInstalled
        | Phase::Correlating => text,
    }
}

/// A bridge output line as progress, the same in the CLI and the TUI: [`ELEVATION_NOTICE`]
/// enters [`Phase::WaitingElevation`] (and is not shown as a line: the phase says it); the
/// first other line after it returns to `Bridge { tool }`. The phase to enter (`None` keeps
/// `current`) and whether the line is shown.
pub fn bridge_line(line: &str, current: Option<&Phase>, tool: &str) -> (Option<Phase>, bool) {
    if line == ELEVATION_NOTICE {
        return (Some(Phase::WaitingElevation), false);
    }
    let back = matches!(current, Some(Phase::WaitingElevation)).then(|| Phase::Bridge {
        tool: tool.to_string(),
    });
    (back, true)
}

/// The phase a line of the elevated helper moves to (Task 201): [`ELEVATION_NOTICE`] right
/// before its UAC prompt, `granted` once it runs; `None` for any other line.
pub fn helper_line_phase(line: &str, granted: &Phase) -> Option<Phase> {
    match line {
        ELEVATION_NOTICE => Some(Phase::WaitingElevation),
        ELEVATION_GRANTED => Some(granted.clone()),
        _ => None,
    }
}

/// Window of the average download speed.
const SPEED_WINDOW: Duration = Duration::from_secs(3);
/// Download silence after which the line says so.
pub const STALL_AFTER: Duration = Duration::from_secs(10);
/// Cells of the download bar.
const BAR_CELLS: usize = 20;

/// Bytes of the downloads in progress (summed over their keys) with the recent samples the
/// speed comes from. The CLI and the TUI feed it and draw it through [`download_parts`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DownloadMeter {
    per_url: BTreeMap<String, (u64, Option<u64>)>,
    /// `(when, summed bytes)` of recent reports, plus one anchor before the window.
    samples: VecDeque<(Instant, u64)>,
    /// When the summed bytes last grew (or the first report).
    last_data: Option<Instant>,
}

impl DownloadMeter {
    /// `url` has `done` of `total` bytes at `now`.
    pub fn record_at(&mut self, url: &str, done: u64, total: Option<u64>, now: Instant) {
        self.per_url.insert(url.to_string(), (done, total));
        let sum = self.done();
        if self.samples.back().is_none_or(|(_, last)| sum > *last) {
            self.last_data = Some(now);
        }
        self.samples.push_back((now, sum));
        while self.samples.len() >= 2 && self.samples[1].0 + SPEED_WINDOW <= now {
            self.samples.pop_front();
        }
    }

    /// No download reported yet.
    pub fn is_empty(&self) -> bool {
        self.per_url.is_empty()
    }

    /// Bytes so far, summed.
    pub fn done(&self) -> u64 {
        self.per_url
            .values()
            .fold(0u64, |a, (d, _)| a.saturating_add(*d))
    }

    /// Summed size, when every download knows its own and the sum is not zero.
    pub fn total(&self) -> Option<u64> {
        self.per_url
            .values()
            .try_fold(0u64, |a, (_, t)| t.map(|t| a.saturating_add(t)))
            .filter(|t| *t > 0)
    }

    /// Every download reached its known size.
    pub fn finished(&self) -> bool {
        !self.per_url.is_empty()
            && self
                .per_url
                .values()
                .all(|(d, t)| t.is_some_and(|t| *d >= t))
    }

    /// Average bytes per second over the last ~3 s, or `None` without a measurable rise.
    pub fn rate_at(&self, now: Instant) -> Option<f64> {
        let (_, latest) = *self.samples.back()?;
        let anchor = self
            .samples
            .iter()
            .rev()
            .find(|(t, _)| *t + SPEED_WINDOW <= now)
            .or(self.samples.front())?;
        let secs = now.saturating_duration_since(anchor.0).as_secs_f64();
        let rate = latest.saturating_sub(anchor.1) as f64 / secs;
        (secs >= 0.5 && rate > 0.0).then_some(rate)
    }

    /// Time since the bytes last grew, or `None` before the first report.
    pub fn quiet_at(&self, now: Instant) -> Option<Duration> {
        self.last_data.map(|at| now.saturating_duration_since(at))
    }
}

/// The download line in pieces, so the CLI can paint the bar: `filled` and `empty` make
/// [`BAR_CELLS`] cells (both empty when the size is unknown), `tail` is the percentage, sizes,
/// speed and estimate, `warn` the stall notice (may be empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadParts {
    pub open: &'static str,
    pub filled: String,
    pub empty: String,
    pub close: &'static str,
    pub tail: String,
    pub warn: String,
}

fn mib(n: u64) -> String {
    format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0))
}

/// `~6 s` or `~3 min`.
fn eta(secs: f64) -> String {
    let s = secs.round() as u64;
    if s < 60 {
        format!("~{s} s")
    } else {
        format!("~{} min", s.div_ceil(60))
    }
}

/// The one download formatting of the CLI and the TUI: bar, `34%  9.3 / 27.0 MiB`, speed,
/// estimate and, past [`STALL_AFTER`] without data, `sem dados há N s`.
pub fn download_parts(meter: &DownloadMeter, now: Instant, glyphs: &Glyphs) -> DownloadParts {
    let done = meter.done();
    let total = meter.total();
    let (filled, empty, mut tail) = match total {
        Some(t) => {
            let pct = done.min(t).saturating_mul(100) / t;
            let cells = usize::try_from(pct / 5).unwrap_or(BAR_CELLS).min(BAR_CELLS);
            (
                glyphs.bar_full.repeat(cells),
                glyphs.bar_empty.repeat(BAR_CELLS - cells),
                format!(
                    "{pct}%  {:.1} / {}",
                    done as f64 / (1024.0 * 1024.0),
                    mib(t)
                ),
            )
        }
        None => (String::new(), String::new(), mib(done)),
    };
    if let Some(rate) = meter.rate_at(now) {
        tail.push_str(&format!(" · {}/s", mib(rate as u64)));
        if let Some(t) = total {
            tail.push_str(&format!(" · {}", eta(t.saturating_sub(done) as f64 / rate)));
        }
    }
    let warn = match meter.quiet_at(now) {
        Some(quiet) if quiet >= STALL_AFTER => format!(" · sem dados há {} s", quiet.as_secs()),
        _ => String::new(),
    };
    DownloadParts {
        open: glyphs.bar_open,
        filled,
        empty,
        close: glyphs.bar_close,
        tail,
        warn,
    }
}

/// `{label} {bar} {tail}{warn}` without color (no bar when the size is unknown).
pub fn download_text(label: &str, parts: &DownloadParts) -> String {
    if parts.filled.is_empty() && parts.empty.is_empty() {
        return format!("{label} {}{}", parts.tail, parts.warn);
    }
    format!(
        "{label} {}{}{}{} {}{}",
        parts.open, parts.filled, parts.empty, parts.close, parts.tail, parts.warn
    )
}
