//! Standalone cabinet (`.cab`) extraction, also for a cabinet renamed `.7z`
//! (Task 204; the Scoop yori manifests download `x.cab#/x.7z`).
//!
//! Same protections as the MSI cabinets: entry and byte caps before decoding, every
//! name through `safepath`, third-party decoder inside `catch_decoder`, and no
//! multi-volume or continued entries.
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

use cab::{Cabinet, CompressionType};

use crate::install::extract::{
    Budget, ExtractError, Limits, Mapper, check_entries, io_at, safe_components,
};
use crate::install::extract_7z::panic_message;
use crate::install::extract_msi::{folder_gaps, place, raw_continued};

/// Extracts the cabinet `file` into `root` through `mapper`.
pub(super) fn extract_cab(
    file: &Path,
    root: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
) -> Result<(), ExtractError> {
    let mut budget = Budget::for_file(file, limits)?;
    let run = crate::install::extract::catch_decoder(|| {
        extract_inner(file, root, mapper, limits, &mut budget)
    });
    match run {
        Ok(r) => r,
        Err(payload) => Err(ExtractError::Decoder(panic_message(payload.as_ref()))),
    }
}

fn open(file: &Path) -> Result<BufReader<File>, ExtractError> {
    Ok(BufReader::new(
        File::open(file).map_err(io_at("abrir", file))?,
    ))
}

fn extract_inner(
    file: &Path,
    root: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
    budget: &mut Budget,
) -> Result<(), ExtractError> {
    // The raw header first: `cab` refuses continued entries as out of bounds.
    let spanned =
        raw_continued(open(file)?).map_err(|e| ExtractError::Archive(format!("cabinet: {e}")))?;
    if spanned {
        return Err(ExtractError::Unsupported(
            "CAB de vários volumes ou com arquivos continuados".to_string(),
        ));
    }
    let mut cab =
        Cabinet::new(open(file)?).map_err(|e| ExtractError::Archive(format!("cabinet: {e}")))?;
    let mut sizes: HashMap<String, u64> = HashMap::new();
    let mut names: Vec<String> = Vec::new();
    let mut entries = 0u64;
    for folder in cab.folder_entries() {
        if let CompressionType::Quantum(..) = folder.compression_type() {
            return Err(ExtractError::Unsupported(
                "CAB com compressão Quantum".to_string(),
            ));
        }
        // Seeking decodes the bytes between files without handing them out.
        budget.charge(folder_gaps(folder)?)?;
        for e in folder.file_entries() {
            entries += 1;
            check_entries(entries, limits)?;
            names.push(e.name().to_string());
            sizes.insert(e.name().to_string(), u64::from(e.uncompressed_size()));
        }
    }
    // Every name is validated before the first byte is written.
    let mut targets: HashMap<String, Option<PathBuf>> = HashMap::new();
    for name in names {
        let comps = safe_components(&name)?;
        let target = mapper.map(comps);
        targets.insert(name, target);
    }
    let mut written = HashSet::new();
    let mut failure = None;
    let res = cab.for_each_file(|key, data| {
        let target = match targets.get(key) {
            Some(Some(rel)) if written.insert(key.to_string()) => Some(rel.as_path()),
            _ => None,
        };
        match place(key, target, data, &sizes, root, budget) {
            Ok(()) => Ok(()),
            Err(e) => {
                failure = Some(e);
                Err(io::Error::other("extração interrompida"))
            }
        }
    });
    match (failure, res) {
        (Some(e), _) => Err(e),
        (None, Err(e)) => Err(ExtractError::Archive(format!("cabinet: {e}"))),
        (None, Ok(())) => Ok(()),
    }
}
