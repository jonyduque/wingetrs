//! Typed progress of an install: download bytes and the phase each slow step enters. Every
//! consumer matches [`ProgressEvent`] and [`Phase`] exhaustively, so a new phase cannot be
//! silently dropped by one of them (the `INSTALLER_RUNNING` label was, see ARMADILHAS).
use std::path::PathBuf;
use std::time::Duration;

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
    /// Native winget install: the elevated helper was asked to run the installer (UAC, then
    /// the installer itself; already launched, cannot be cancelled).
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
        Phase::WaitingElevation => {
            "Aguardando o UAC ou o instalador elevado; se o UAC não aparecer, clique no escudo piscando na barra de tarefas".into()
        }
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
    format!("{} · {}", phase_title(phase), clock(elapsed))
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
        | Phase::CheckingRemoval { .. } => format!("{text} · não pode ser cancelado"),
        Phase::Preparing
        | Phase::WaitingInstaller
        | Phase::Extracting { .. }
        | Phase::Placing
        | Phase::Linking
        | Phase::Recording
        | Phase::Removing { .. }
        | Phase::Scanning
        | Phase::Verifying
        | Phase::CopyingFolders => text,
    }
}
