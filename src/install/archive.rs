//! The single source of truth for download formats (manifest, plan and extractor).

/// Download format, decided by the final file name (after `#/rename`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarGz,
    SevenZip,
    Msi,
    Nupkg,
    TarXz,
    /// Not an archive: copied as is (`.exe`, `.ps1`, `.jar`, ...).
    Plain,
}

impl ArchiveKind {
    /// `Err(ext)` for formats that no phase plans to read (`.rar`, `.tar.bz2`, `.tbz2`, `.zst`).
    /// Case-insensitive; longest suffix wins (`.tar.gz` before `.gz`).
    pub fn of(file_name: &str) -> Result<ArchiveKind, &'static str> {
        let lower = file_name.to_ascii_lowercase();
        let has = |e: &str| lower.ends_with(e);
        if has(".zip") {
            Ok(ArchiveKind::Zip)
        } else if has(".tar.gz") || has(".tgz") {
            Ok(ArchiveKind::TarGz)
        } else if has(".7z") {
            Ok(ArchiveKind::SevenZip)
        } else if has(".msi") {
            Ok(ArchiveKind::Msi)
        } else if has(".nupkg") {
            Ok(ArchiveKind::Nupkg)
        } else if has(".tar.xz") || has(".txz") {
            Ok(ArchiveKind::TarXz)
        } else if has(".rar") {
            Err(".rar")
        } else if has(".tar.bz2") {
            Err(".tar.bz2")
        } else if has(".tbz2") {
            Err(".tbz2")
        } else if has(".zst") {
            Err(".zst")
        } else {
            Ok(ArchiveKind::Plain)
        }
    }

    /// Whether the extractor can read this kind today. P1: Zip, TarGz, Plain.
    pub fn supported(self) -> bool {
        matches!(
            self,
            ArchiveKind::Zip | ArchiveKind::TarGz | ArchiveKind::Plain
        )
    }

    /// Suffix shown in messages (`".7z"`, `".msi"`, ...); `""` for Plain.
    pub fn ext(self) -> &'static str {
        match self {
            ArchiveKind::Zip => ".zip",
            ArchiveKind::TarGz => ".tar.gz",
            ArchiveKind::SevenZip => ".7z",
            ArchiveKind::Msi => ".msi",
            ArchiveKind::Nupkg => ".nupkg",
            ArchiveKind::TarXz => ".tar.xz",
            ArchiveKind::Plain => "",
        }
    }

    /// Verb for the plan display: `"Extrair"` for archives, `"Copiar"` for Plain.
    pub fn verb(self) -> &'static str {
        match self {
            ArchiveKind::Plain => "Copiar",
            _ => "Extrair",
        }
    }
}
