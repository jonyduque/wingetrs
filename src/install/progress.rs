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
    /// Own extraction of `file` (`engine`: the format, e.g. `MSI`).
    Extracting { file: String, engine: String },
    /// `msiexec /a` launched, writing its verbose log at `log` (cannot be cancelled).
    Installer { log: PathBuf },
    /// Moving the extracted version into place and its persisted data.
    Placing,
    /// `current`, commands, shortcuts and environment.
    Linking,
    /// Writing the install record.
    Recording,
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
        Phase::Extracting { file, engine } => format!("Extraindo {file} ({engine})"),
        Phase::Installer { .. } => "Windows Installer (msiexec /a)".into(),
        Phase::Placing => "Colocando os arquivos".into(),
        Phase::Linking => "Criando comandos".into(),
        Phase::Recording => "Registrando".into(),
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
