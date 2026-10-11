//! Archive extraction (zip, tar.gz, plain file) with path-traversal protection.
//!
//! Everything is written into a sibling temp directory first and only moved into
//! `dest` once the whole archive was validated and extracted; on error the temp
//! directory is removed.
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::install::safepath;

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("caminho inseguro no arquivo: {0}")]
    UnsafePath(String),
    #[error("pasta {0} não existe no arquivo")]
    MissingExtractDir(String),
    #[error("arquivo inválido: {0}")]
    Archive(String),
    #[error("E/S: {0}")]
    Io(String),
    #[error("arquivo descompactado passa de {limit} bytes (possível bomba de descompressão)")]
    TooLarge { limit: u64 },
    #[error("arquivo com mais de {limit} entradas (possível bomba de descompressão)")]
    TooManyEntries { limit: u64 },
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
struct Budget {
    limit: u64,
    used: u64,
}

impl Budget {
    fn for_file(file: &Path, limits: &Limits) -> Result<Self, ExtractError> {
        let len = fs::metadata(file)?.len();
        let limit = len
            .saturating_mul(limits.ratio)
            .max(limits.floor)
            .min(limits.ceiling);
        Ok(Budget { limit, used: 0 })
    }

    fn charge(&mut self, n: u64) -> Result<(), ExtractError> {
        self.used = self.used.saturating_add(n);
        if self.used > self.limit {
            return Err(ExtractError::TooLarge { limit: self.limit });
        }
        Ok(())
    }
}

fn check_entries(count: u64, limits: &Limits) -> Result<(), ExtractError> {
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

/// Splits an archive path on `/` and `\` into safe components; refuses `..`,
/// absolute paths, drive prefixes, UNC and `\\?\` forms.
fn safe_components(raw: &str) -> Result<Vec<String>, ExtractError> {
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
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Maps archive entries to paths relative to the temp root (applies extract_dir / extract_to).
struct Mapper {
    dir: Vec<String>,
    to: Vec<String>,
    dir_seen: bool,
}

impl Mapper {
    /// `None` when the entry is outside `extract_dir`.
    fn map(&mut self, comps: Vec<String>) -> Option<PathBuf> {
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
        mappers.push(Mapper {
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
        });
    }
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        ".wingetrs-extract-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&tmp)?;
    let mut guard = TempGuard {
        path: tmp.clone(),
        armed: true,
    };
    for (s, mapper) in sources.iter().zip(mappers.iter_mut()) {
        extract_one(s, &tmp, mapper, limits)?;
    }
    merge_into(&tmp, dest)?;
    guard.armed = false;
    let _ = fs::remove_dir_all(&tmp);
    Ok(())
}

fn extract_one(
    s: &ExtractSource<'_>,
    tmp: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
) -> Result<(), ExtractError> {
    let lower = s.file_name.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        extract_zip(s.file, tmp, mapper, limits)?;
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        extract_tar_gz(s.file, tmp, mapper, limits)?;
    } else {
        let comps = safe_components(s.file_name)?;
        if comps.is_empty() {
            return Err(ExtractError::UnsafePath(s.file_name.to_string()));
        }
        let mut target = tmp.to_path_buf();
        for c in mapper.to.iter().chain(comps.iter()) {
            target.push(c);
        }
        if let Some(p) = target.parent() {
            fs::create_dir_all(p)?;
        }
        fs::copy(s.file, &target)?;
        mapper.dir_seen = true;
    }
    if !mapper.dir.is_empty() && !mapper.dir_seen {
        return Err(ExtractError::MissingExtractDir(
            s.extract_dir.unwrap_or_default().to_string(),
        ));
    }
    Ok(())
}

fn write_entry(
    root: &Path,
    rel: &Path,
    is_dir: bool,
    src: &mut dyn Read,
    budget: &mut Budget,
) -> Result<(), ExtractError> {
    let target = root.join(rel);
    if is_dir {
        fs::create_dir_all(&target)?;
    } else {
        if let Some(p) = target.parent() {
            fs::create_dir_all(p)?;
        }
        let mut out = fs::File::create(&target)?;
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
            out.write_all(&buf[..n])?;
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

fn extract_tar_gz(
    file: &Path,
    root: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
) -> Result<(), ExtractError> {
    let mut budget = Budget::for_file(file, limits)?;
    let mut count: u64 = 0;
    let f = fs::File::open(file)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    let entries = ar
        .entries()
        .map_err(|e| ExtractError::Archive(e.to_string()))?;
    for entry in entries {
        count += 1;
        check_entries(count, limits)?;
        let mut entry = entry.map_err(|e| ExtractError::Archive(e.to_string()))?;
        let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let comps = safe_components(&name)?;
        let ty = entry.header().entry_type();
        if ty.is_symlink() || ty.is_hard_link() {
            return Err(ExtractError::UnsafePath(name));
        }
        let is_dir = ty.is_dir();
        if !is_dir && !ty.is_file() {
            continue;
        }
        if let Some(rel) = mapper.map(comps) {
            write_entry(root, &rel, is_dir, &mut entry, &mut budget)?;
        }
    }
    Ok(())
}

/// Moves the contents of `src` into `dest`, merging with existing directories.
fn merge_into(src: &Path, dest: &Path) -> Result<(), ExtractError> {
    if !dest.exists() {
        fs::rename(src, dest)?;
        return Ok(());
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            if to.exists() {
                merge_into(&entry.path(), &to)?;
            } else {
                fs::rename(entry.path(), &to)?;
            }
        } else {
            if to.exists() {
                fs::remove_file(&to)?;
            }
            fs::rename(entry.path(), &to)?;
        }
    }
    Ok(())
}
