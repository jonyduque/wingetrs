//! 7z extraction (phase 2b, Task 123).
//!
//! The whole header is validated (names, links, entry count, declared size,
//! methods and dictionary sizes) before any block is decoded. Decoding runs on
//! one thread: the multithreaded LZMA2 reader of `sevenz-rust2` neither joins
//! nor catches its workers.
use std::any::Any;
use std::io::{self, Read, Seek};
use std::path::Path;

use sevenz_rust2::{Archive, ArchiveReader, Coder, EncoderMethod, Password};

use crate::install::extract::{
    Budget, ExtractError, Limits, Mapper, check_entries, safe_components, write_entry,
};

/// Largest dictionary (or PPMd model) accepted: the decoder allocates it up front.
pub(super) const MAX_7Z_DICT: u64 = 1 << 30;

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
/// 7-Zip stores the unix mode in the high 16 bits when this flag is set.
const FILE_ATTRIBUTE_UNIX_EXTENSION: u32 = 0x8000;

/// Extracts the 7z archive read from `src` (`file` is the download, for the byte budget).
pub(super) fn extract_7z<R: Read + Seek>(
    src: R,
    file: &Path,
    root: &Path,
    mapper: &mut Mapper,
    limits: &Limits,
) -> Result<(), ExtractError> {
    let mut budget = Budget::for_file(file, limits)?;
    // D4-1: third-party decoder panic becomes an error (spec 2b)
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut reader = ArchiveReader::new(src, Password::empty()).map_err(from_7z)?;
        reader.set_thread_count(1);
        validate(reader.archive(), &budget, limits)?;
        decode(&mut reader, root, mapper, &mut budget)
    }));
    match run {
        Ok(r) => r,
        Err(payload) => Err(ExtractError::Decoder(panic_message(payload.as_ref()))),
    }
}

/// Refuses the archive from its header alone, before any block is decoded.
fn validate(archive: &Archive, budget: &Budget, limits: &Limits) -> Result<(), ExtractError> {
    check_entries(archive.files.len() as u64, limits)?;
    let mut declared: u64 = 0;
    for f in &archive.files {
        safe_components(&f.name)?;
        if f.is_anti_item || is_link(f.has_windows_attributes, f.windows_attributes) {
            return Err(ExtractError::UnsafePath(f.name.clone()));
        }
        // Counts entries filtered out by extract_dir too: a solid block decodes them anyway.
        declared = declared.saturating_add(f.size);
    }
    for block in &archive.blocks {
        for coder in &block.coders {
            check_coder(coder)?;
        }
    }
    if declared > budget.limit {
        return Err(ExtractError::TooLarge {
            limit: budget.limit,
        });
    }
    Ok(())
}

fn is_link(has_attributes: bool, attributes: u32) -> bool {
    if !has_attributes {
        return false;
    }
    let reparse = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    let unix_symlink = attributes & FILE_ATTRIBUTE_UNIX_EXTENSION != 0
        && (attributes >> 16) & 0o170000 == 0o120000;
    reparse || unix_symlink
}

const READABLE: [&[u8]; 16] = [
    EncoderMethod::ID_COPY,
    EncoderMethod::ID_DELTA,
    EncoderMethod::ID_LZMA,
    EncoderMethod::ID_LZMA2,
    EncoderMethod::ID_PPMD,
    EncoderMethod::ID_BZIP2,
    EncoderMethod::ID_DEFLATE,
    EncoderMethod::ID_BCJ_X86,
    EncoderMethod::ID_BCJ2,
    EncoderMethod::ID_BCJ_PPC,
    EncoderMethod::ID_BCJ_IA64,
    EncoderMethod::ID_BCJ_ARM,
    EncoderMethod::ID_BCJ_ARM64,
    EncoderMethod::ID_BCJ_ARM_THUMB,
    EncoderMethod::ID_BCJ_SPARC,
    EncoderMethod::ID_BCJ_RISCV,
];

fn check_coder(coder: &Coder) -> Result<(), ExtractError> {
    let id = coder.encoder_method_id();
    if id == EncoderMethod::ID_AES256_SHA256 {
        return Err(ExtractError::Encrypted);
    }
    if !READABLE.contains(&id) {
        let name = EncoderMethod::by_id(id)
            .map(|m| m.name().to_string())
            .unwrap_or_else(|| format!("{id:02X?}"));
        return Err(ExtractError::UnsupportedMethod(name));
    }
    let props = coder.properties();
    let dict = if id == EncoderMethod::ID_LZMA2 {
        let p = *props
            .first()
            .ok_or_else(|| ExtractError::Archive("propriedades LZMA2 ausentes".into()))?;
        if p > 40 {
            return Err(ExtractError::Archive(format!(
                "propriedade LZMA2 inválida: {p}"
            )));
        }
        // 7-Zip's LZMA2 dictionary encoding; 40 means 4 GiB - 1.
        if p == 40 {
            0xFFFF_FFFF
        } else {
            u64::from(2 | (p & 1)) << (p / 2 + 11)
        }
    } else if id == EncoderMethod::ID_LZMA || id == EncoderMethod::ID_PPMD {
        // LZMA: lc/lp/pb byte + dictionary; PPMd: order byte + model size (both u32 LE).
        let b = props
            .get(1..5)
            .ok_or_else(|| ExtractError::Archive("propriedades do compressor curtas".into()))?;
        u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    } else {
        return Ok(());
    };
    if dict > MAX_7Z_DICT {
        return Err(ExtractError::DictionaryTooLarge {
            dict,
            limit: MAX_7Z_DICT,
        });
    }
    Ok(())
}

fn decode<R: Read + Seek>(
    reader: &mut ArchiveReader<R>,
    root: &Path,
    mapper: &mut Mapper,
    budget: &mut Budget,
) -> Result<(), ExtractError> {
    let mut failure = None;
    let res = reader.for_each_entries(|entry, data| {
        let mut counted = Counted {
            inner: data,
            n: 0,
            failed: false,
        };
        let placed = place(
            &entry.name,
            entry.is_directory,
            &mut counted,
            root,
            mapper,
            budget,
        )
        .map_err(|e| match e {
            // A failed read is the archive's fault (CRC, corrupt stream), not the disk's.
            ExtractError::Io(m) if counted.failed => ExtractError::Archive(m),
            e => e,
        })
        .and_then(|()| {
            // The CRC reader of `sevenz-rust2` ends quietly on a short stream.
            if counted.n == entry.size {
                Ok(())
            } else {
                Err(ExtractError::Archive(format!(
                    "{} terminou com {} de {} bytes",
                    entry.name, counted.n, entry.size
                )))
            }
        });
        match placed {
            Ok(()) => Ok(true),
            Err(e) => {
                failure = Some(e);
                Err(sevenz_rust2::Error::Other("extração interrompida".into()))
            }
        }
    });
    match (failure, res) {
        (Some(e), _) => Err(e),
        (None, Err(e)) => Err(from_7z(e)),
        (None, Ok(())) => Ok(()),
    }
}

/// Writes one decoded entry, or drains and charges it when it is not extracted.
fn place(
    name: &str,
    is_dir: bool,
    data: &mut dyn Read,
    root: &Path,
    mapper: &mut Mapper,
    budget: &mut Budget,
) -> Result<(), ExtractError> {
    let comps = safe_components(name)?;
    // Scoop extracts 7z with `-xr!*.nsis`.
    let nsis = !is_dir && name.to_ascii_lowercase().ends_with(".nsis");
    match mapper.map(comps) {
        Some(rel) if !nsis => write_entry(root, &rel, is_dir, data, budget),
        // A solid block decodes skipped entries anyway: read them to the end and charge them.
        _ => drain(data, budget),
    }
}

/// Counts the bytes read through it.
struct Counted<'a> {
    inner: &'a mut dyn Read,
    n: u64,
    failed: bool,
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

fn from_7z(e: sevenz_rust2::Error) -> ExtractError {
    use sevenz_rust2::Error as E;
    match e {
        E::PasswordRequired | E::MaybeBadPassword(_) => ExtractError::Encrypted,
        // Built without AES: an encrypted header fails to find a decoder for it.
        E::UnsupportedCompressionMethod(m) if m == EncoderMethod::AES256_SHA256.name() => {
            ExtractError::Encrypted
        }
        E::UnsupportedCompressionMethod(m) => ExtractError::UnsupportedMethod(m),
        E::ExternalUnsupported => ExtractError::UnsupportedMethod("externo".into()),
        other => ExtractError::Archive(other.to_string()),
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "pânico sem mensagem".to_string()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::io::{Cursor, SeekFrom};

    fn mapper(dir: &[&str]) -> Mapper {
        Mapper {
            dir: dir.iter().map(|s| s.to_string()).collect(),
            to: Vec::new(),
            dir_seen: false,
        }
    }

    fn solid(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut w = sevenz_rust2::ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
        let entries = files
            .iter()
            .map(|(n, _)| sevenz_rust2::ArchiveEntry::new_file(n))
            .collect();
        let readers = files
            .iter()
            .map(|(_, d)| sevenz_rust2::SourceReader::new(Cursor::new(d.clone())))
            .collect();
        w.push_archive_entries(entries, readers).unwrap();
        w.finish().unwrap().into_inner()
    }

    /// The decode loop on its own (the header check already refuses honest archives):
    /// entries outside extract_dir are still charged.
    #[test]
    fn decode_charges_entries_outside_extract_dir() {
        let a = solid(&[
            ("keep/x", b"0123456789".to_vec()),
            ("skip/y", vec![0u8; 4096]),
        ]);
        let d = tempfile::tempdir().unwrap();
        let mut reader = ArchiveReader::new(Cursor::new(a), Password::empty()).unwrap();
        reader.set_thread_count(1);
        let mut budget = Budget {
            limit: 1000,
            used: 0,
        };
        let r = decode(&mut reader, d.path(), &mut mapper(&["keep"]), &mut budget);
        assert!(
            matches!(r, Err(ExtractError::TooLarge { limit: 1000 })),
            "{r:?}"
        );
    }

    /// Panics once a read reaches the packed streams (the header reads stay intact).
    struct PanicsInData {
        inner: Cursor<Vec<u8>>,
    }

    impl Read for PanicsInData {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let pos = self.inner.position();
            if (32..64).contains(&pos) {
                panic!("decoder blew up at {pos}");
            }
            self.inner.read(buf)
        }
    }

    impl Seek for PanicsInData {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    #[test]
    fn decoder_panic_becomes_an_error() {
        let a = solid(&[("x", vec![7u8; 4096])]);
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.7z");
        std::fs::write(&f, &a).unwrap();
        let root = d.path().join("root");
        let r = extract_7z(
            PanicsInData {
                inner: Cursor::new(a),
            },
            &f,
            &root,
            &mut mapper(&[]),
            &crate::install::extract::DEFAULT_LIMITS,
        );
        assert!(
            matches!(r, Err(ExtractError::Decoder(ref m)) if m == "decoder blew up at 32"),
            "{r:?}"
        );
    }
}
