//! Archive extraction (zip, tar.gz, plain file) with path-traversal protection.
//!
//! Everything is written into a sibling temp directory first and only moved into
//! `dest` once the whole archive was validated and extracted; on error the temp
//! directory is removed.
use std::cell::Cell;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::fs_retry;
use crate::install::archive::ArchiveKind;
use crate::install::extract_7z::{MAX_7Z_DICT, extract_7z};
use crate::install::extract_msi::{align_case, extract_msi};
use crate::install::safepath;
use crate::install::sniff::{self, Sniffed};

thread_local! {
    /// Set while a third-party decoder runs under [`catch_decoder`] on this thread.
    static IN_DECODER: Cell<bool> = const { Cell::new(false) };
}

/// Whether this thread is inside a decoder guarded by [`catch_decoder`]. Read by the TUI
/// panic hook: that panic becomes an extraction error, so the terminal must stay as is.
pub fn in_decoder() -> bool {
    IN_DECODER.with(Cell::get)
}

/// D4-1: runs a third-party decoder under `catch_unwind` with [`in_decoder`] set.
pub fn catch_decoder<T>(f: impl FnOnce() -> T) -> std::thread::Result<T> {
    /// Puts the previous value back, also while unwinding.
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            IN_DECODER.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(IN_DECODER.with(|c| c.replace(true)));
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("caminho inseguro no arquivo: {0}")]
    UnsafePath(String),
    #[error("pasta {0} não existe no arquivo")]
    MissingExtractDir(String),
    #[error("arquivo inválido: {0}")]
    Archive(String),
    #[error("formato não suportado pelo extrator: {0}")]
    Unsupported(String),
    #[error("E/S: {0}")]
    Io(String),
    #[error("arquivo descompactado passa de {limit} bytes (possível bomba de descompressão)")]
    TooLarge { limit: u64 },
    #[error("arquivo com mais de {limit} entradas (possível bomba de descompressão)")]
    TooManyEntries { limit: u64 },
    #[error("arquivo 7z protegido por senha")]
    Encrypted,
    #[error("método de compressão não suportado: {0}")]
    UnsupportedMethod(String),
    #[error("dicionário de {dict} bytes passa do limite de {limit}")]
    DictionaryTooLarge { dict: u64, limit: u64 },
    #[error("falha no decodificador: {0}")]
    Decoder(String),
    #[error("layout do MSI inválido: {0}")]
    MsiLayout(String),
}

/// Decompression-bomb limits; the byte budget of one archive is `clamp(size × ratio, floor, ceiling)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub ratio: u64,
    pub floor: u64,
    pub ceiling: u64,
    pub max_entries: u64,
}

// Measured on 90 real archives: ratio 100 = 13x the max (7.5), 1 GiB floor covers any small text archive, 32 GiB = 48x the largest total (668 MB), 200k entries = 12x the max (15 639).
pub const DEFAULT_LIMITS: Limits = Limits {
    ratio: 100,
    floor: 1 << 30,
    ceiling: 32 << 30,
    max_entries: 200_000,
};

/// Bytes actually written for one archive, against its budget.
pub(super) struct Budget {
    pub(super) limit: u64,
    pub(super) used: u64,
}

impl Budget {
    pub(super) fn for_file(file: &Path, limits: &Limits) -> Result<Self, ExtractError> {
        let len = fs::metadata(file)?.len();
        let limit = len
            .saturating_mul(limits.ratio)
            .max(limits.floor)
            .min(limits.ceiling);
        Ok(Budget { limit, used: 0 })
    }

    pub(super) fn charge(&mut self, n: u64) -> Result<(), ExtractError> {
        self.used = self.used.saturating_add(n);
        if self.used > self.limit {
            return Err(ExtractError::TooLarge { limit: self.limit });
        }
        Ok(())
    }
}

pub(super) fn check_entries(count: u64, limits: &Limits) -> Result<(), ExtractError> {
    if count > limits.max_entries {
        return Err(ExtractError::TooManyEntries {
            limit: limits.max_entries,
        });
    }
    Ok(())
}

impl From<io::Error> for ExtractError {
    fn from(e: io::Error) -> Self {
        ExtractError::Io(e.to_string())
    }
}

/// Maps a filesystem error to `ExtractError::Io` naming the operation (Portuguese) and the path.
pub(super) fn io_at<'a>(
    op: &'static str,
    path: &'a Path,
) -> impl FnOnce(io::Error) -> ExtractError + 'a {
    move |e| ExtractError::Io(format!("{op} {}: {e}", path.display()))
}

/// Splits an archive path on `/` and `\` into safe components; refuses `..`,
/// absolute paths, drive prefixes, UNC and `\\?\` forms.
pub(super) fn safe_components(raw: &str) -> Result<Vec<String>, ExtractError> {
    safepath::safe_components(raw).ok_or_else(|| ExtractError::UnsafePath(raw.to_string()))
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Removes the directory on drop unless disarmed.
struct TempGuard {
    path: PathBuf,
    armed: bool,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs_retry::retry("apagar", &self.path, || fs::remove_dir_all(&self.path));
        }
    }
}

/// Maps archive entries to paths relative to the temp root (applies extract_dir / extract_to).
pub(super) struct Mapper {
    pub(super) dir: Vec<String>,
    pub(super) to: Vec<String>,
    pub(super) dir_seen: bool,
}

impl Mapper {
    /// `None` when the entry is outside `extract_dir`.
    pub(super) fn map(&mut self, comps: Vec<String>) -> Option<PathBuf> {
        if comps.len() < self.dir.len() || comps[..self.dir.len()] != self.dir[..] {
            return None;
        }
        self.dir_seen = true;
        let mut p = PathBuf::new();
        for c in self.to.iter().chain(comps[self.dir.len()..].iter()) {
            p.push(c);
        }
        Some(p)
    }
}

/// One downloaded file to extract.
pub struct ExtractSource<'a> {
    pub file: &'a Path,
    pub file_name: &'a str,
    pub extract_dir: Option<&'a str>,
    pub extract_to: Option<&'a str>,
}

/// Extracts (or copies) `file` into `dest` honoring extract_dir/extract_to. `dest` may already have content.
pub fn extract(
    file: &Path,
    file_name: &str,
    extract_dir: Option<&str>,
    extract_to: Option<&str>,
    dest: &Path,
) -> Result<(), ExtractError> {
    extract_all_with(
        &[ExtractSource {
            file,
            file_name,
            extract_dir,
            extract_to,
        }],
        dest,
        &DEFAULT_LIMITS,
    )
}

/// Extracts every source, in order, into one temp folder and moves it into `dest` once at the end.
pub fn extract_all(sources: &[ExtractSource<'_>], dest: &Path) -> Result<(), ExtractError> {
    extract_all_with(sources, dest, &DEFAULT_LIMITS)
}

/// [`extract_all`] with explicit decompression-bomb limits (each archive has its own byte budget).
pub fn extract_all_with(
    sources: &[ExtractSource<'_>],
    dest: &Path,
    limits: &Limits,
) -> Result<(), ExtractError> {
    let mut mappers = Vec::new();
    for s in sources {
        mappers.push(mapper_for(s)?);
    }
    through_temp(dest, |tmp| {
        for (s, mapper) in sources.iter().zip(mappers.iter_mut()) {
            extract_one(s, tmp, mapper, limits)?;
        }
        Ok(())
    })
}

fn mapper_for(s: &ExtractSource<'_>) -> Result<Mapper, ExtractError> {
    Ok(Mapper {
        dir: s
            .extract_dir
            .map(safe_components)
            .transpose()?
            .unwrap_or_default(),
        to: s
            .extract_to
            .map(safe_components)
            .transpose()?
            .unwrap_or_default(),
        dir_seen: false,
    })
}

/// Name prefix of the temp folders [`through_temp`] creates beside the destination.
pub const TEMP_PREFIX: &str = ".wingetrs-extract-";

/// Runs `fill` on a fresh sibling temp folder of `dest` and moves the result into `dest`
/// only when it succeeds; the temp folder is removed on every path.
fn through_temp(
    dest: &Path,
    fill: impl FnOnce(&Path) -> Result<(), ExtractError>,
) -> Result<(), ExtractError> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(io_at("criar pasta", parent))?;
    let tmp = parent.join(format!(
        "{TEMP_PREFIX}{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&tmp).map_err(io_at("criar pasta", &tmp))?;
    let mut guard = TempGuard {
        path: tmp.clone(),
        armed: true,
    };
    fill(&tmp)?;
    merge_into(&tmp, dest)?;
    guard.armed = false;
    let _ = fs_retry::retry("apagar", &tmp, || fs::remove_dir_all(&tmp));
    Ok(())
}

/// Whether `msiexec /a` may extract what the own extractor refused: an MSI (by name and by
/// its OLE signature) that it does not support. Integrity and safety errors never qualify.
pub fn needs_installer(file: &Path, file_name: &str, e: &ExtractError) -> bool {
    matches!(e, ExtractError::Unsupported(_))
        && matches!(ArchiveKind::of(file_name), Ok(ArchiveKind::Msi))
        && is_ole(file).unwrap_or(false)
}

/// Moves the tree `msiexec /a` left in `source_dir` (its `TARGETDIR`) into `dest` through
/// the same mapper (`extract_dir` without case, `/` or `\`), `safepath` checks and byte
/// budget as the own MSI extraction (`source.file` sizes the budget). `admin_msi` is the
/// administrative copy of the package that `msiexec` leaves at the root: never moved.
pub fn import_admin_tree(
    source: &ExtractSource<'_>,
    source_dir: &Path,
    admin_msi: &str,
    dest: &Path,
    limits: &Limits,
) -> Result<(), ExtractError> {
    let mut mapper = mapper_for(source)?;
    let mut budget = Budget::for_file(source.file, limits)?;
    through_temp(dest, |tmp| {
        let mut entries = 0u64;
        let mut stack = vec![(source_dir.to_path_buf(), Vec::<String>::new())];
        while let Some((dir, comps)) = stack.pop() {
            for entry in fs::read_dir(&dir).map_err(io_at("listar", &dir))? {
                let entry = entry.map_err(io_at("listar", &dir))?;
                let path = entry.path();
                let name = entry.file_name().into_string().map_err(|n| {
                    let mut rel = comps.clone();
                    rel.push(n.to_string_lossy().into_owned());
                    ExtractError::UnsafePath(rel.join("/"))
                })?;
                if comps.is_empty() && name.eq_ignore_ascii_case(admin_msi) {
                    continue;
                }
                entries += 1;
                check_entries(entries, limits)?;
                let mut rel = comps.clone();
                rel.push(name);
                let raw = rel.join("/");
                // Names from disk have no separators: only the refusal matters.
                safe_components(&raw)?;
                let meta = fs::symlink_metadata(&path).map_err(io_at("ler tipo", &path))?;
                if meta.file_type().is_symlink() {
                    return Err(ExtractError::UnsafePath(raw));
                }
                let target = mapper.map(align_case(rel.clone(), &mapper.dir));
                if meta.is_dir() {
                    if let Some(t) = &target {
                        fs::create_dir_all(tmp.join(t))
                            .map_err(io_at("criar pasta", &tmp.join(t)))?;
                    }
                    stack.push((path, rel));
                    continue;
                }
                budget.charge(meta.len())?;
                if let Some(t) = target {
                    let to = tmp.join(t);
                    if let Some(p) = to.parent() {
                        fs::create_dir_all(p).map_err(io_at("criar pasta", p))?;
                    }
                    rename_retrying(&path, &to)?;
                }
            }
        }
        if !mapper.dir.is_empty() && !mapper.dir_seen {
            return Err(ExtractError::MissingExtractDir(
                source.extract_dir.unwrap_or_default().to_string(),
            ));
        }
        Ok(())
    })
}

fn extract_one(
    s: &ExtractSource<'_>,
    tmp: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
) -> Result<(), ExtractError> {
    match ArchiveKind::of(s.file_name) {
        Ok(ArchiveKind::Zip | ArchiveKind::Nupkg) => extract_zip(s.file, tmp, mapper, limits)?,
        Ok(ArchiveKind::TarGz) => {
            let f = fs::File::open(s.file).map_err(io_at("abrir", s.file))?;
            extract_tar(flate2::read::GzDecoder::new(f), s.file, tmp, mapper, limits)?;
        }
        Ok(ArchiveKind::TarXz) => {
            extract_tar_guarded(
                || {
                    let f = fs::File::open(s.file).map_err(io_at("abrir", s.file))?;
                    Ok(lzma_rust2::XzReader::new_mem_limit(
                        f,
                        true,
                        XZ_MEM_LIMIT_KB,
                    ))
                },
                s.file,
                tmp,
                mapper,
                limits,
            )?;
        }
        Ok(ArchiveKind::Plain) => {
            let comps = safe_components(s.file_name)?;
            if comps.is_empty() {
                return Err(ExtractError::UnsafePath(s.file_name.to_string()));
            }
            let mut target = tmp.to_path_buf();
            for c in mapper.to.iter().chain(comps.iter()) {
                target.push(c);
            }
            if let Some(p) = target.parent() {
                fs::create_dir_all(p).map_err(io_at("criar pasta", p))?;
            }
            fs::copy(s.file, &target).map_err(io_at("copiar", &target))?;
            mapper.dir_seen = true;
        }
        Ok(ArchiveKind::SevenZip) => extract_sniffed(s, tmp, mapper, limits)?,
        Ok(ArchiveKind::Msi) => {
            if !is_ole(s.file)? {
                return Err(ExtractError::Unsupported("conteúdo não é MSI".to_string()));
            }
            extract_msi(s.file, tmp, mapper, limits)?;
        }
        Err(ext) => {
            return Err(ExtractError::Unsupported(format!(
                "{} ({ext})",
                s.file_name
            )));
        }
    }
    if !mapper.dir.is_empty() && !mapper.dir_seen {
        return Err(ExtractError::MissingExtractDir(
            s.extract_dir.unwrap_or_default().to_string(),
        ));
    }
    Ok(())
}

/// OLE compound file signature, the container of every MSI.
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

fn is_ole(file: &Path) -> Result<bool, ExtractError> {
    let mut head = Vec::with_capacity(OLE_MAGIC.len());
    fs::File::open(file)
        .map_err(io_at("abrir", file))?
        .take(OLE_MAGIC.len() as u64)
        .read_to_end(&mut head)
        .map_err(io_at("ler", file))?;
    Ok(head == OLE_MAGIC)
}

/// How far past the PE sections a 7z SFX archive is searched for.
const MAX_SFX_SCAN: u64 = 32 << 20;

/// A `.7z` download is read by its content: 7z, zip in disguise, or 7z SFX.
fn extract_sniffed(
    s: &ExtractSource<'_>,
    tmp: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
) -> Result<(), ExtractError> {
    let mut f = fs::File::open(s.file).map_err(io_at("abrir", s.file))?;
    let mut head = Vec::with_capacity(64 * 1024);
    (&mut f)
        .take(64 * 1024)
        .read_to_end(&mut head)
        .map_err(io_at("ler", s.file))?;
    match sniff::sniff(&head) {
        Sniffed::SevenZip => extract_7z(f, s.file, tmp, mapper, limits),
        Sniffed::Zip => {
            tracing::info!(file = s.file_name, format = "zip", "arquivo .7z é um zip");
            extract_zip(s.file, tmp, mapper, limits)
        }
        Sniffed::Pe => {
            let start = sniff::pe_overlay_start(&head);
            match sniff::find_7z_in_pe(&mut f, start, MAX_SFX_SCAN).map_err(io_at("ler", s.file))? {
                Some(at) => {
                    tracing::info!(
                        file = s.file_name,
                        format = "7z sfx",
                        offset = at,
                        "arquivo .7z é um 7z SFX"
                    );
                    let sub = sniff::OffsetReader::new(f, at).map_err(io_at("ler", s.file))?;
                    extract_7z(sub, s.file, tmp, mapper, limits)
                }
                None => Err(ExtractError::Unsupported(
                    "instalador .exe, não um arquivo 7z".to_string(),
                )),
            }
        }
        Sniffed::Other => Err(ExtractError::Unsupported(
            "conteúdo não reconhecido como 7z".to_string(),
        )),
    }
}

pub(super) fn write_entry(
    root: &Path,
    rel: &Path,
    is_dir: bool,
    src: &mut dyn Read,
    budget: &mut Budget,
) -> Result<(), ExtractError> {
    let target = root.join(rel);
    if is_dir {
        fs::create_dir_all(&target).map_err(io_at("criar pasta", &target))?;
    } else {
        if let Some(p) = target.parent() {
            fs::create_dir_all(p).map_err(io_at("criar pasta", p))?;
        }
        let mut out = fs::File::create(&target).map_err(io_at("criar arquivo", &target))?;
        // Count bytes actually produced: declared sizes can lie and tar has none up front.
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match src.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            budget.charge(n as u64)?;
            out.write_all(&buf[..n]).map_err(io_at("gravar", &target))?;
        }
    }
    Ok(())
}

fn extract_zip(
    file: &Path,
    root: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
) -> Result<(), ExtractError> {
    let mut budget = Budget::for_file(file, limits)?;
    let f = fs::File::open(file)?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| ExtractError::Archive(e.to_string()))?;
    check_entries(zip.len() as u64, limits)?;
    // Validate every name (and the declared total size) first so nothing is written for a bad archive.
    let mut declared: u64 = 0;
    for i in 0..zip.len() {
        let e = zip
            .by_index_raw(i)
            .map_err(|e| ExtractError::Archive(e.to_string()))?;
        safe_components(e.name())?;
        if e.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Err(ExtractError::UnsafePath(e.name().to_string()));
        }
        // Counts entries filtered out by extract_dir too: conservative on purpose.
        declared = declared.saturating_add(e.size());
    }
    if declared > budget.limit {
        return Err(ExtractError::TooLarge {
            limit: budget.limit,
        });
    }
    for i in 0..zip.len() {
        let mut e = zip
            .by_index(i)
            .map_err(|e| ExtractError::Archive(e.to_string()))?;
        let name = e.name().to_string();
        let comps = safe_components(&name)?;
        let is_dir = name.ends_with('/') || name.ends_with('\\');
        if let Some(rel) = mapper.map(comps) {
            write_entry(root, &rel, is_dir, &mut e, &mut budget)?;
        }
    }
    Ok(())
}

/// Per-block xz decoder memory cap in KiB, the same 1 GiB as the 7z dictionary cap.
const XZ_MEM_LIMIT_KB: u32 = (MAX_7Z_DICT >> 10) as u32;

/// Maps a tar/decompressor error; the xz memory-limit refusal gets its own message.
fn tar_err(e: io::Error) -> ExtractError {
    if e.kind() == io::ErrorKind::OutOfMemory {
        return ExtractError::Archive(
            "dicionário xz grande demais (limite de memória de 1 GiB)".to_string(),
        );
    }
    ExtractError::Archive(e.to_string())
}

/// `extract_tar` over a third-party decoder: opening and every read run under `catch_unwind`.
fn extract_tar_guarded<R: Read>(
    open: impl FnOnce() -> Result<R, ExtractError>,
    file: &Path,
    root: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
) -> Result<(), ExtractError> {
    // D4-1: third-party decoder panic becomes an error (spec 2b)
    let run = catch_decoder(|| extract_tar(open()?, file, root, mapper, limits));
    match run {
        Ok(r) => r,
        Err(payload) => Err(ExtractError::Decoder(
            crate::install::extract_7z::panic_message(payload.as_ref()),
        )),
    }
}

/// Walks a tar stream already wrapped in its decompressor; `file` only sizes the budget.
fn extract_tar(
    reader: impl Read,
    file: &Path,
    root: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
) -> Result<(), ExtractError> {
    let mut budget = Budget::for_file(file, limits)?;
    let mut count: u64 = 0;
    let mut ar = tar::Archive::new(reader);
    let entries = ar.entries().map_err(tar_err)?;
    for entry in entries {
        count += 1;
        check_entries(count, limits)?;
        let mut entry = entry.map_err(tar_err)?;
        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let comps = safe_components(&name)?;
        let ty = entry.header().entry_type();
        if ty.is_symlink() || ty.is_hard_link() {
            return Err(ExtractError::UnsafePath(name));
        }
        let is_dir = ty.is_dir();
        let rel = if is_dir || ty.is_file() {
            mapper.map(comps)
        } else {
            None
        };
        match rel {
            Some(rel) => write_entry(root, &rel, is_dir, &mut entry, &mut budget)?,
            // Skipped entries are still decompressed by the tar reader: charge them too.
            None => budget.charge(entry.size())?,
        }
    }
    Ok(())
}

/// Renames `from` to `to`, waiting out a transient external handle (a scanner on a fresh file).
fn rename_retrying(from: &Path, to: &Path) -> Result<(), ExtractError> {
    fs_retry::retry("renomear", from, || fs::rename(from, to)).map_err(io_at("renomear", from))
}

/// Moves the contents of `src` into `dest`, merging with existing directories.
fn merge_into(src: &Path, dest: &Path) -> Result<(), ExtractError> {
    if !dest.exists() {
        return rename_retrying(src, dest);
    }
    for entry in fs::read_dir(src).map_err(io_at("listar", src))? {
        let entry = entry.map_err(io_at("listar", src))?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if entry
            .file_type()
            .map_err(io_at("ler tipo", &from))?
            .is_dir()
        {
            if to.exists() {
                merge_into(&from, &to)?;
            } else {
                rename_retrying(&from, &to)?;
            }
        } else {
            if to.exists() {
                fs_retry::retry("apagar", &to, || fs::remove_file(&to))
                    .map_err(io_at("apagar", &to))?;
            }
            rename_retrying(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::fs_retry::release_on_retry;

    #[test]
    fn a_panicking_tar_decoder_becomes_a_decoder_error() {
        struct Boom;
        impl Read for Boom {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                panic!("boom");
            }
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.tar.xz");
        fs::write(&f, b"x").unwrap();
        let mut mapper = Mapper {
            dir: Vec::new(),
            to: Vec::new(),
            dir_seen: false,
        };
        let r = extract_tar_guarded(|| Ok(Boom), &f, d.path(), &mut mapper, &DEFAULT_LIMITS);
        assert!(
            matches!(&r, Err(ExtractError::Decoder(m)) if m.contains("boom")),
            "{r:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn merge_into_a_missing_dest_waits_for_a_held_child_to_be_released() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = tempfile::tempdir().unwrap();
        let (src, dest) = (d.path().join("src"), d.path().join("dest"));
        fs::create_dir(&src).unwrap();
        fs::write(src.join("f.txt"), b"x").unwrap();
        // Full sharing, like a scanner: renaming the folder still fails until it is closed.
        let held: release_on_retry::Held = std::sync::Arc::new(std::sync::Mutex::new(Some(
            fs::OpenOptions::new()
                .read(true)
                .share_mode(1 | 2 | 4)
                .open(src.join("f.txt"))
                .unwrap(),
        )));
        let _routing = release_on_retry::install(&held);
        merge_into(&src, &dest).unwrap();
        assert_eq!(fs::read(dest.join("f.txt")).unwrap(), b"x");
        assert!(!src.exists());
        assert!(held.lock().unwrap().is_none(), "the retry path never ran");
    }
}
