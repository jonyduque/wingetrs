//! Content sniffing of downloads (phase 2b, Task 123).
//!
//! A file named `.7z` is often something else (installer, renamed zip, 7z SFX):
//! the first bytes decide how it is read.
use std::io::{self, Read, Seek, SeekFrom};

/// What the first bytes of a download look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    SevenZip,
    Zip,
    Pe,
    /// Microsoft cabinet (`MSCF`).
    Cab,
    Other,
}

const SEVEN_Z_MAGIC: [u8; 6] = [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
/// Magic plus the format version (0, 4) written by every 7-Zip since 9.x.
const SEVEN_Z_SIGNATURE: [u8; 8] = [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0x00, 0x04];

/// `MSCF` plus the 4 reserved zero bytes of CFHEADER.
const CAB_SIGNATURE: [u8; 8] = [0x4D, 0x53, 0x43, 0x46, 0, 0, 0, 0];

/// `head` = first bytes of the file (up to 64 KiB).
pub fn sniff(head: &[u8]) -> Sniffed {
    if head.starts_with(&SEVEN_Z_MAGIC) {
        Sniffed::SevenZip
    } else if head.starts_with(&[0x50, 0x4B, 0x03, 0x04])
        || head.starts_with(&[0x50, 0x4B, 0x05, 0x06])
    {
        Sniffed::Zip
    } else if head.starts_with(&CAB_SIGNATURE) {
        Sniffed::Cab
    } else if head.starts_with(&[0x4D, 0x5A]) {
        Sniffed::Pe
    } else {
        Sniffed::Other
    }
}

/// End of the last PE section on disk (start of the overlay); 0 when the headers do not parse.
pub fn pe_overlay_start(head: &[u8]) -> u64 {
    let u16_at = |o: usize| head.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| {
        head.get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let Some(pe) = u32_at(0x3C).map(|v| v as usize) else {
        return 0;
    };
    if head.get(pe..pe + 4) != Some(b"PE\0\0".as_slice()) {
        return 0;
    }
    let (Some(sections), Some(opt_size)) = (u16_at(pe + 6), u16_at(pe + 20)) else {
        return 0;
    };
    let table = pe + 24 + opt_size as usize;
    let mut end = 0u64;
    for i in 0..sections as usize {
        let s = table + i * 40;
        let (Some(size), Some(ptr)) = (u32_at(s + 16), u32_at(s + 20)) else {
            return 0;
        };
        end = end.max(u64::from(ptr) + u64::from(size));
    }
    end
}

/// What a downloaded `.exe` is (Task 209).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeKind {
    Program,
    /// Generator name, or "instalador" when only the overlay size decides.
    Installer(&'static str),
}

#[derive(Debug, thiserror::Error)]
#[error("cabeçalho PE inválido: {0}")]
pub struct PeError(&'static str);

/// Windows loader limit.
const MAX_SECTIONS: usize = 96;
/// PE32+ needs 0xF0; anything far above is not a real header.
const MAX_OPTIONAL_HEADER: usize = 0x1000;
/// Generator marks are looked for only at the start of the overlay, where the
/// installers put their data (a 7-Zip or innounp program carries the same bytes
/// in its own code).
const MARK_SCAN: usize = 1 << 20;
const OVERLAY_MIN: u64 = 1 << 20;

const GENERATOR_MARKS: &[(&[u8], &str)] = &[
    (b"NullsoftInst", "NSIS"),
    (b"Inno Setup Setup Data", "Inno Setup"),
    (b"rDlPtS", "Inno Setup"),
    (b"InstallShield", "InstallShield"),
    (b"SquirrelSetup", "Squirrel"),
    (&SEVEN_Z_SIGNATURE, "7-Zip SFX"),
];

/// Installer or program: generator mark ⇒ installer; overlay (minus the Authenticode
/// certificate table) over 25 % of the file and over 1 MiB ⇒ installer; else program.
pub fn classify_pe(bytes: &[u8]) -> Result<PeKind, PeError> {
    let u16_at = |o: usize| {
        o.checked_add(2)
            .and_then(|e| bytes.get(o..e))
            .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
    };
    let u32_at = |o: usize| {
        o.checked_add(4)
            .and_then(|e| bytes.get(o..e))
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64)
    };
    let len = bytes.len() as u64;
    if !bytes.starts_with(b"MZ") {
        return Err(PeError("sem assinatura MZ"));
    }
    let pe = u32_at(0x3C).ok_or(PeError("arquivo truncado"))?;
    if pe >= len {
        return Err(PeError("cabeçalho PE fora do arquivo"));
    }
    let pe = pe as usize;
    if bytes.get(pe..pe + 4) != Some(b"PE\0\0".as_slice()) {
        return Err(PeError("sem assinatura PE"));
    }
    let sections = u16_at(pe + 6).ok_or(PeError("arquivo truncado"))?;
    if sections == 0 || sections > MAX_SECTIONS {
        return Err(PeError("número de seções absurdo"));
    }
    let opt_size = u16_at(pe + 20).ok_or(PeError("arquivo truncado"))?;
    if opt_size > MAX_OPTIONAL_HEADER {
        return Err(PeError("cabeçalho opcional grande demais"));
    }
    let opt = pe + 24;
    let dirs = match u16_at(opt) {
        Some(0x10B) => opt + 96,
        Some(0x20B) => opt + 112,
        _ => return Err(PeError("cabeçalho opcional desconhecido")),
    };
    let dir_count = u32_at(dirs - 4).ok_or(PeError("arquivo truncado"))?;
    // IMAGE_DIRECTORY_ENTRY_SECURITY: file offset and size of the certificate table.
    let cert_entry = dirs + 4 * 8;
    let cert = if dir_count > 4 && cert_entry + 8 <= opt + opt_size {
        let off = u32_at(cert_entry).ok_or(PeError("arquivo truncado"))?;
        let size = u32_at(cert_entry + 4).ok_or(PeError("arquivo truncado"))?;
        (size > 0).then_some((off, size))
    } else {
        None
    };

    let table = opt + opt_size;
    let mut names = Vec::with_capacity(sections);
    let mut image_end = 0u64;
    for i in 0..sections {
        let s = table + i * 40;
        let name = bytes
            .get(s..s + 8)
            .ok_or(PeError("tabela de seções truncada"))?;
        let size = u32_at(s + 16).ok_or(PeError("tabela de seções truncada"))?;
        let ptr = u32_at(s + 20).ok_or(PeError("tabela de seções truncada"))?;
        if size > 0 && ptr + size > len {
            return Err(PeError("seção além do fim do arquivo"));
        }
        names.push(name);
        if size > 0 {
            image_end = image_end.max(ptr + size);
        }
    }
    if names.iter().any(|n| n.starts_with(b".wixburn")) {
        return Ok(PeKind::Installer("WiX Burn"));
    }

    let mut overlay_end = len;
    let mut signature = 0u64;
    if let Some((off, size)) = cert {
        if off < image_end || off + size > len {
            return Err(PeError("assinatura fora do overlay"));
        }
        overlay_end = off;
        signature = size;
    }
    let scan_end = overlay_end.min(image_end + MARK_SCAN as u64) as usize;
    let head = bytes
        .get(image_end as usize..scan_end)
        .ok_or(PeError("overlay fora do arquivo"))?;
    for (mark, generator) in GENERATOR_MARKS {
        if head.windows(mark.len()).any(|w| w == *mark) {
            return Ok(PeKind::Installer(generator));
        }
    }
    let overlay = len - image_end - signature;
    if overlay * 4 > len && overlay > OVERLAY_MIN {
        return Ok(PeKind::Installer("instalador"));
    }
    Ok(PeKind::Program)
}

/// Scans `r` from `start` for the 7z signature (SFX overlay); `None` if absent.
///
/// Every candidate must open as a 7z archive (start-header CRC, then the whole
/// header through `sevenz-rust2`); a stray signature is skipped.
pub fn find_7z_in_pe<R: Read + Seek>(
    r: &mut R,
    start: u64,
    max_scan: u64,
) -> io::Result<Option<u64>> {
    const CHUNK: usize = 64 * 1024;
    let overlap = SEVEN_Z_SIGNATURE.len() - 1;
    let end = start.saturating_add(max_scan);
    let mut buf = vec![0u8; CHUNK + overlap];
    let mut pos = start;
    let mut carried = 0usize;
    while pos < end {
        r.seek(SeekFrom::Start(pos))?;
        let want = (CHUNK as u64).min(end - pos) as usize;
        let n = read_full(r, &mut buf[carried..carried + want])?;
        if n == 0 {
            break;
        }
        let filled = carried + n;
        let base = pos - carried as u64;
        for i in 0..filled.saturating_sub(overlap) {
            if buf[i..i + SEVEN_Z_SIGNATURE.len()] == SEVEN_Z_SIGNATURE {
                let at = base + i as u64;
                if is_valid_7z_at(r, at)? {
                    return Ok(Some(at));
                }
            }
        }
        pos += n as u64;
        carried = overlap.min(filled);
        buf.copy_within(filled - carried..filled, 0);
        if n < want {
            break;
        }
    }
    Ok(None)
}

fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(got)
}

fn is_valid_7z_at<R: Read + Seek>(r: &mut R, at: u64) -> io::Result<bool> {
    let mut sub = OffsetReader::new(&mut *r, at)?;
    // Same header pre-check as the extractor: no oversized or zeroed header is opened.
    if let Err(e) = crate::install::extract_7z::check_header(&mut sub) {
        tracing::debug!(offset = at, error = %e, "candidato 7z recusado");
        return Ok(false);
    }
    // D4-1: third-party decoder panic becomes an error (spec 2b)
    let opened = crate::install::extract::catch_decoder(|| {
        sevenz_rust2::Archive::read(&mut sub, &sevenz_rust2::Password::empty())
    });
    match opened {
        Ok(r) => Ok(r.is_ok()),
        Err(payload) => {
            let msg = crate::install::extract_7z::panic_message(payload.as_ref());
            tracing::warn!(offset = at, panic = %msg, "pânico ao validar candidato 7z");
            Ok(false)
        }
    }
}

/// `Read + Seek` view of `inner` that starts at `offset` (an archive embedded in a PE).
pub struct OffsetReader<R> {
    inner: R,
    offset: u64,
}

impl<R: Seek> OffsetReader<R> {
    pub fn new(mut inner: R, offset: u64) -> io::Result<Self> {
        inner.seek(SeekFrom::Start(offset))?;
        Ok(OffsetReader { inner, offset })
    }
}

impl<R: Read> Read for OffsetReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for OffsetReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let abs = match pos {
            SeekFrom::Start(p) => SeekFrom::Start(self.offset.checked_add(p).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "posição fora do arquivo")
            })?),
            other => other,
        };
        let p = self.inner.seek(abs)?;
        p.checked_sub(self.offset).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "posição antes do início do 7z")
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn sniff_detects_cab() {
        assert_eq!(sniff(b"MSCF\0\0\0\0"), Sniffed::Cab);
    }

    #[test]
    fn sniff_table() {
        let cases: [(&[u8], Sniffed); 9] = [
            (
                &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0x00, 0x04],
                Sniffed::SevenZip,
            ),
            (&[0x50, 0x4B, 0x03, 0x04, 0x14, 0x00], Sniffed::Zip),
            (&[0x50, 0x4B, 0x05, 0x06, 0x00, 0x00], Sniffed::Zip),
            (&[0x4D, 0x5A, 0x90, 0x00], Sniffed::Pe),
            (&[0x4D, 0x53, 0x43, 0x46, 0, 0, 0, 0, 1], Sniffed::Cab),
            (&[0x4D, 0x53, 0x43, 0x46, 0, 0, 0, 1], Sniffed::Other),
            (&[], Sniffed::Other),
            (&[0x37, 0x7A, 0xBC], Sniffed::Other),
            (&[0x50, 0x4B, 0x07, 0x08], Sniffed::Other),
        ];
        for (head, want) in cases {
            assert_eq!(sniff(head), want, "{head:02X?}");
        }
    }

    #[test]
    fn overlay_start_is_end_of_last_section() {
        let mut h = vec![0u8; 0x400];
        h[0] = b'M';
        h[1] = b'Z';
        h[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        h[0x80..0x84].copy_from_slice(b"PE\0\0");
        h[0x86..0x88].copy_from_slice(&2u16.to_le_bytes());
        h[0x94..0x96].copy_from_slice(&0xE0u16.to_le_bytes());
        let table = 0x80 + 24 + 0xE0;
        // Section 1: 0x400..0x1400; section 2: 0x1400..0x1600.
        h[table + 16..table + 20].copy_from_slice(&0x1000u32.to_le_bytes());
        h[table + 20..table + 24].copy_from_slice(&0x400u32.to_le_bytes());
        h[table + 56..table + 60].copy_from_slice(&0x200u32.to_le_bytes());
        h[table + 60..table + 64].copy_from_slice(&0x1400u32.to_le_bytes());
        assert_eq!(pe_overlay_start(&h), 0x1600);
        assert_eq!(pe_overlay_start(b"MZ"), 0);
    }

    #[test]
    fn stray_signature_without_archive_is_not_found() {
        let mut data = b"MZ".to_vec();
        data.resize(5000, 0);
        data.extend_from_slice(&SEVEN_Z_SIGNATURE);
        data.resize(9000, 0xAA);
        let mut c = io::Cursor::new(data);
        assert_eq!(find_7z_in_pe(&mut c, 0, 32 << 20).unwrap(), None);
    }
}
