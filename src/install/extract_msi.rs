//! MSI extraction from embedded cabinets, without running the installer (phase 2b, Task 125).
//!
//! The layout comes from `msi_layout` (the `msiexec /a` tree under `SourceDir`).
//! Everything that can refuse the package (compression, external or Quantum
//! cabinets, transforms, unsafe names, files without data) is checked before the
//! first byte is written. Each cabinet folder is decoded once, in offset order.
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use cab::{Cabinet, CompressionType, FolderEntry};
use msi::{Package, Row, Select, StreamReader};

use crate::install::extract::{
    Budget, ExtractError, Limits, Mapper, check_entries, io_at, write_entry,
};
use crate::install::extract_7z::panic_message;
use crate::install::msi_layout::{self, CompRow, DirRow, FileRow};

pub(crate) type Pkg = Package<BufReader<File>>;

/// Summary Information word count bit: files live in cabinets.
const WORD_COUNT_COMPRESSED: i32 = 0x2;
/// `File.Attributes` bit: this file sits uncompressed next to the MSI.
const FILE_NONCOMPRESSED: i32 = 0x2000;
/// Largest `cbUncomp` of one CFDATA block (MS-CAB).
const MAX_BLOCK: u64 = 0x8000;
/// Largest `cbUncomp` the `cab` parser accepts (a `u16`): what one block can decode.
const PARSED_BLOCK: u64 = 0xFFFF;
/// CFHEADER flags `cfhdrPREV_CABINET | cfhdrNEXT_CABINET`.
const CAB_SPANNED: u16 = 0x3;
/// CFFILE `iFolder` values from 0xFFFD up: continued from or into another cabinet.
const IFOLD_CONTINUED: u16 = 0xFFFD;

/// Extracts the `msiexec /a` tree of `file` into `root` through `mapper`.
pub(super) fn extract_msi(
    file: &Path,
    root: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
) -> Result<(), ExtractError> {
    let mut budget = Budget::for_file(file, limits)?;
    // `BufReader` has no `Write`: the package cannot be modified.
    let src = BufReader::new(File::open(file).map_err(io_at("abrir", file))?);
    // D4-1: third-party decoder panic becomes an error (spec 2b); the OLE and
    // table parsers of `msi` run inside too.
    let run = crate::install::extract::catch_decoder(|| {
        let mut pkg = Package::open(src).map_err(|e| msi_error("MSI", e))?;
        extract_package(&mut pkg, root, mapper, limits, &mut budget)
    });
    match run {
        Ok(r) => r,
        Err(payload) => Err(ExtractError::Decoder(panic_message(payload.as_ref()))),
    }
}

fn extract_package(
    pkg: &mut Pkg,
    root: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
    budget: &mut Budget,
) -> Result<(), ExtractError> {
    if pkg.summary_info().word_count().unwrap_or(0) & WORD_COUNT_COMPRESSED == 0 {
        return Err(ExtractError::Unsupported(
            "MSI sem compressão (arquivos fora do pacote)".to_string(),
        ));
    }
    if let Some(t) = transforms(pkg)? {
        return Err(ExtractError::Unsupported(format!(
            "MSI exige transform ({t})"
        )));
    }
    let cabinets = cabinets(pkg)?;
    let (dirs, comps, files) = tables(pkg)?;
    if let Some(f) = files.iter().find(|f| f.1 & FILE_NONCOMPRESSED != 0) {
        return Err(ExtractError::Unsupported(format!(
            "arquivo não comprimido no MSI: {}",
            f.0.file
        )));
    }
    let files: Vec<FileRow> = files.into_iter().map(|(f, _)| f).collect();
    let placed = msi_layout::layout(&dirs, &comps, &files)
        .map_err(|e| ExtractError::MsiLayout(e.to_string()))?;

    // Every cabinet's folders and entries, before anything is decoded.
    let mut sizes: Vec<HashMap<String, u64>> = Vec::new();
    let mut entries = 0u64;
    for name in &cabinets {
        if continued(pkg, name)? {
            return Err(ExtractError::Unsupported(format!(
                "gabinete com arquivos continuados: {name}"
            )));
        }
        let cab = open_cabinet(pkg, name)?;
        let mut in_cab = HashMap::new();
        for folder in cab.folder_entries() {
            if let CompressionType::Quantum(..) = folder.compression_type() {
                return Err(ExtractError::Unsupported(format!(
                    "gabinete Quantum no MSI: {name}"
                )));
            }
            // Seeking decodes the bytes between files without handing them out.
            budget.charge(folder_gaps(folder)?)?;
            for e in folder.file_entries() {
                entries += 1;
                in_cab.insert(e.name().to_string(), u64::from(e.uncompressed_size()));
            }
        }
        sizes.push(in_cab);
    }
    check_entries(entries, limits)?;
    if let Some(p) = placed
        .iter()
        .find(|p| !sizes.iter().any(|s| s.contains_key(&p.file)))
    {
        return Err(ExtractError::MsiLayout(format!(
            "arquivo {} sem dados",
            p.file
        )));
    }

    let mut targets: HashMap<String, Option<PathBuf>> = HashMap::new();
    for p in placed {
        let rel = align_case(p.rel, &mapper.dir);
        targets.insert(p.file, mapper.map(rel));
    }
    let mut written = HashSet::new();
    for (name, sizes) in cabinets.iter().zip(&sizes) {
        let mut cab = open_cabinet(pkg, name)?;
        let mut failure = None;
        let res = cab.for_each_file(|key, data| {
            // A key written by an earlier cabinet, a collision loser, a cabinet entry
            // without a `File` row or a file outside `extract_dir`: read and charged only.
            let target = match targets.get(key) {
                Some(Some(rel)) if written.insert(key.to_string()) => Some(rel.as_path()),
                _ => None,
            };
            match place(key, target, data, sizes, root, budget) {
                Ok(()) => Ok(()),
                Err(e) => {
                    failure = Some(e);
                    Err(io::Error::other("extração interrompida"))
                }
            }
        });
        match (failure, res) {
            (Some(e), _) => return Err(e),
            (None, Err(e)) => return Err(ExtractError::Archive(format!("gabinete {name}: {e}"))),
            (None, Ok(())) => {}
        }
    }
    Ok(())
}

/// Spells the leading components as `dir` when they match it ignoring case: on
/// Windows `extract_dir = "pfiles/App"` finds `PFiles\app` (Scoop runs there).
pub(super) fn align_case(mut rel: Vec<String>, dir: &[String]) -> Vec<String> {
    if rel.len() >= dir.len()
        && rel
            .iter()
            .zip(dir)
            .all(|(a, b)| a.to_lowercase() == b.to_lowercase())
    {
        rel[..dir.len()].clone_from_slice(dir);
    }
    rel
}

/// Writes one cabinet entry to `target`, or reads it to the end and charges it.
pub(super) fn place(
    key: &str,
    target: Option<&Path>,
    data: &mut dyn Read,
    sizes: &HashMap<String, u64>,
    root: &Path,
    budget: &mut Budget,
) -> Result<(), ExtractError> {
    let mut counted = Counted {
        inner: data,
        n: 0,
        failed: false,
    };
    let res = match target {
        Some(rel) => write_entry(root, rel, false, &mut counted, budget),
        None => drain(&mut counted, budget),
    };
    res.map_err(|e| match e {
        // A failed read is the cabinet's fault (checksum, truncation), not the disk's.
        ExtractError::Io(m) if counted.failed => ExtractError::Archive(m),
        e => e,
    })?;
    // The folder reader ends quietly when the data runs out before the declared size.
    let size = sizes.get(key).copied().unwrap_or(0);
    if counted.n != size {
        return Err(ExtractError::Archive(format!(
            "{key} terminou com {} de {size} bytes",
            counted.n
        )));
    }
    Ok(())
}

/// Counts the bytes read through it.
pub(super) struct Counted<'a> {
    pub(super) inner: &'a mut dyn Read,
    pub(super) n: u64,
    pub(super) failed: bool,
}

impl Read for Counted<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf).inspect_err(|_| self.failed = true)?;
        self.n += n as u64;
        Ok(n)
    }
}

fn drain(data: &mut dyn Read, budget: &mut Budget) -> Result<(), ExtractError> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = match data.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(ExtractError::Archive(e.to_string())),
        };
        budget.charge(n as u64)?;
    }
}

/// Bytes the folder decodes without handing them out. Entries with the same offset
/// and size share their bytes (WiX stores identical files once): the span counts
/// once, and each extra entry is charged the rewind it costs. Any other overlap
/// and files past the most data the folder's blocks can hold are refused.
pub(super) fn folder_gaps(folder: &FolderEntry) -> Result<u64, ExtractError> {
    let mut spans: Vec<(u64, u64)> = folder
        .file_entries()
        .map(|e| {
            (
                u64::from(e.uncompressed_offset()),
                u64::from(e.uncompressed_size()),
            )
        })
        .collect();
    spans.sort_unstable();
    let (mut end, mut in_files, mut rewinds) = (0u64, 0u64, 0u64);
    let mut last = None;
    for (offset, size) in spans {
        if last == Some((offset, size)) {
            // Reading the span again seeks back: the folder decodes again from its
            // start up to `offset`, plus at most the block the span ends in.
            if size > 0 {
                rewinds = rewinds.saturating_add(offset + PARSED_BLOCK);
            }
            continue;
        }
        if offset < end {
            return Err(ExtractError::Archive(
                "gabinete com arquivos sobrepostos".to_string(),
            ));
        }
        end = offset + size;
        in_files += size;
        last = Some((offset, size));
    }
    if end > u64::from(folder.num_data_blocks()) * MAX_BLOCK {
        return Err(ExtractError::Archive(
            "gabinete com arquivos além dos dados da pasta".to_string(),
        ));
    }
    let gaps = end.checked_sub(in_files).ok_or_else(|| {
        ExtractError::Archive("gabinete com tamanhos de arquivos incoerentes".to_string())
    })?;
    Ok(gaps.saturating_add(rewinds))
}

/// Whether the cabinet spans cabinets (header flags or a continued CFFILE),
/// read from the raw header: `cab` refuses those entries as out of bounds.
fn continued(pkg: &mut Pkg, name: &str) -> Result<bool, ExtractError> {
    let stream = pkg
        .read_stream(name)
        .map_err(|e| ExtractError::Archive(format!("gabinete {name}: {e}")))?;
    raw_continued(BufReader::new(stream))
        .map_err(|e| ExtractError::Archive(format!("gabinete {name}: {e}")))
}

pub(super) fn raw_continued<R: BufRead + Seek>(mut r: R) -> io::Result<bool> {
    let mut h = [0u8; 36];
    r.read_exact(&mut h)?;
    if u16::from_le_bytes([h[30], h[31]]) & CAB_SPANNED != 0 {
        return Ok(true);
    }
    let first_file = u32::from_le_bytes([h[16], h[17], h[18], h[19]]);
    let files = u16::from_le_bytes([h[28], h[29]]);
    r.seek(SeekFrom::Start(u64::from(first_file)))?;
    let mut name = Vec::new();
    for _ in 0..files {
        let mut e = [0u8; 16];
        r.read_exact(&mut e)?;
        if u16::from_le_bytes([e[8], e[9]]) >= IFOLD_CONTINUED {
            return Ok(true);
        }
        name.clear();
        // szName is NUL-terminated, at most 256 bytes (MS-CAB).
        (&mut r).take(257).read_until(0, &mut name)?;
        if name.last() != Some(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "nome de arquivo sem fim no gabinete",
            ));
        }
    }
    Ok(false)
}

fn open_cabinet(
    pkg: &mut Pkg,
    name: &str,
) -> Result<Cabinet<StreamReader<BufReader<File>>>, ExtractError> {
    let stream = pkg
        .read_stream(name)
        .map_err(|e| ExtractError::Archive(format!("gabinete {name}: {e}")))?;
    Cabinet::new(stream).map_err(|e| ExtractError::Archive(format!("gabinete {name}: {e}")))
}

pub(crate) fn rows(pkg: &mut Pkg, table: &str) -> Result<Vec<Row>, ExtractError> {
    if !pkg.has_table(table) {
        return Ok(Vec::new());
    }
    pkg.select_rows(Select::table(table))
        .map(Iterator::collect)
        .map_err(|e| msi_error(&format!("tabela {table} do MSI"), e))
}

/// An `msi` error: integrity, except its own 65536-row limit per table, which the
/// Windows Installer does not have (rust's `File` table has 66064 rows).
fn msi_error(context: &str, e: io::Error) -> ExtractError {
    if e.kind() == io::ErrorKind::InvalidData
        && e.to_string().starts_with("Number of rows is too large")
    {
        return ExtractError::Unsupported("MSI com mais de 65536 linhas numa tabela".to_string());
    }
    ExtractError::Archive(format!("{context}: {e}"))
}

pub(crate) fn text(row: &Row, column: &str) -> Option<String> {
    if !row.has_column(column) {
        return None;
    }
    row[column].as_str().map(str::to_string)
}

pub(crate) fn int(row: &Row, column: &str) -> Option<i32> {
    if !row.has_column(column) {
        return None;
    }
    row[column].as_int()
}

fn required(row: &Row, table: &str, column: &str) -> Result<String, ExtractError> {
    text(row, column).ok_or_else(|| ExtractError::MsiLayout(format!("{table}.{column} ausente")))
}

/// The `TRANSFORMS` property: the package only installs with those applied.
fn transforms(pkg: &mut Pkg) -> Result<Option<String>, ExtractError> {
    Ok(rows(pkg, "Property")?
        .iter()
        .find(|r| text(r, "Property").as_deref() == Some("TRANSFORMS"))
        .and_then(|r| text(r, "Value"))
        .filter(|v| !v.trim().is_empty()))
}

/// Stream names of the `#cabinet` media, in order; an external cabinet refuses the package.
fn cabinets(pkg: &mut Pkg) -> Result<Vec<String>, ExtractError> {
    let mut out: Vec<String> = Vec::new();
    for r in rows(pkg, "Media")? {
        let Some(c) = text(&r, "Cabinet").filter(|c| !c.is_empty()) else {
            continue;
        };
        match c.strip_prefix('#') {
            Some(stream) => {
                if !out.iter().any(|s| s == stream) {
                    out.push(stream.to_string());
                }
            }
            None => {
                return Err(ExtractError::Unsupported(format!(
                    "gabinete externo ao MSI: {c}"
                )));
            }
        }
    }
    Ok(out)
}

/// `Directory`, `Component` and `File` rows (each file with its `Attributes`).
pub(crate) type Tables = (Vec<DirRow>, Vec<CompRow>, Vec<(FileRow, i32)>);

pub(crate) fn tables(pkg: &mut Pkg) -> Result<Tables, ExtractError> {
    let mut dirs = Vec::new();
    for r in rows(pkg, "Directory")? {
        dirs.push(DirRow {
            directory: required(&r, "Directory", "Directory")?,
            parent: text(&r, "Directory_Parent"),
            default_dir: required(&r, "Directory", "DefaultDir")?,
        });
    }
    let mut comps = Vec::new();
    for r in rows(pkg, "Component")? {
        comps.push(CompRow {
            component: required(&r, "Component", "Component")?,
            directory: required(&r, "Component", "Directory_")?,
        });
    }
    let mut files = Vec::new();
    for r in rows(pkg, "File")? {
        let f = FileRow {
            file: required(&r, "File", "File")?,
            component: required(&r, "File", "Component_")?,
            file_name: required(&r, "File", "FileName")?,
            version: text(&r, "Version"),
            sequence: int(&r, "Sequence").unwrap_or(0),
        };
        files.push((f, int(&r, "Attributes").unwrap_or(0)));
    }
    Ok((dirs, comps, files))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::io::{Cursor, Write};

    use msi::{Column, Insert, PackageType, Value};

    use super::*;
    use crate::install::extract::DEFAULT_LIMITS;

    fn table(p: &mut Package<Cursor<Vec<u8>>>, name: &str, cols: Vec<Column>, row: Vec<Value>) {
        p.create_table(name, cols).unwrap();
        p.insert_rows(Insert::into(name).row(row)).unwrap();
    }

    #[test]
    fn unsafe_layout_leaves_root_empty() {
        let mut p = Package::create(PackageType::Installer, Cursor::new(Vec::new())).unwrap();
        p.summary_info_mut().set_word_count(WORD_COUNT_COMPRESSED);
        let id = |n: &str| Column::build(n).id_string(72);
        table(
            &mut p,
            "Directory",
            vec![
                Column::build("Directory").primary_key().id_string(72),
                Column::build("Directory_Parent").nullable().id_string(72),
                Column::build("DefaultDir").text_string(255),
            ],
            vec!["TARGETDIR".into(), Value::Null, "SourceDir".into()],
        );
        table(
            &mut p,
            "Component",
            vec![
                Column::build("Component").primary_key().id_string(72),
                id("Directory_"),
            ],
            vec!["C".into(), "TARGETDIR".into()],
        );
        table(
            &mut p,
            "File",
            vec![
                Column::build("File").primary_key().id_string(72),
                id("Component_"),
                Column::build("FileName").text_string(255),
                Column::build("Sequence").int32(),
            ],
            vec!["f_a".into(), "C".into(), "..".into(), 1.into()],
        );
        table(
            &mut p,
            "Media",
            vec![
                Column::build("DiskId").primary_key().int16(),
                Column::build("LastSequence").int32(),
                Column::build("Cabinet").nullable().string(255),
            ],
            vec![Value::Int(1), 1.into(), "#cab1".into()],
        );
        let cab = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/msi/three.cab"),
        )
        .unwrap();
        p.write_stream("cab1").unwrap().write_all(&cab).unwrap();
        let bytes = p.into_inner().unwrap().into_inner();

        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("x.msi");
        std::fs::write(&file, bytes).unwrap();
        let root = d.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let mut mapper = Mapper {
            dir: Vec::new(),
            to: Vec::new(),
            dir_seen: false,
        };
        let r = extract_msi(&file, &root, &mut mapper, &DEFAULT_LIMITS);
        assert!(matches!(r, Err(ExtractError::MsiLayout(_))), "{r:?}");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    }
}
