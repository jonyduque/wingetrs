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
    Other,
}

const SEVEN_Z_MAGIC: [u8; 6] = [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C];
/// Magic plus the format version (0, 4) written by every 7-Zip since 9.x.
const SEVEN_Z_SIGNATURE: [u8; 8] = [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0x00, 0x04];
const SIGNATURE_HEADER_SIZE: usize = 32;

/// `head` = first bytes of the file (up to 64 KiB).
pub fn sniff(head: &[u8]) -> Sniffed {
    if head.starts_with(&SEVEN_Z_MAGIC) {
        Sniffed::SevenZip
    } else if head.starts_with(&[0x50, 0x4B, 0x03, 0x04])
        || head.starts_with(&[0x50, 0x4B, 0x05, 0x06])
    {
        Sniffed::Zip
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
    let mut hh = [0u8; SIGNATURE_HEADER_SIZE];
    r.seek(SeekFrom::Start(at))?;
    if read_full(r, &mut hh)? < SIGNATURE_HEADER_SIZE {
        return Ok(false);
    }
    // A zero start-header CRC sends `sevenz-rust2` into an unverified header guess: refuse it.
    if hh[8..12] == [0, 0, 0, 0] {
        return Ok(false);
    }
    let mut sub = OffsetReader::new(&mut *r, at)?;
    // D4-1: third-party decoder panic becomes an error (spec 2b)
    let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sevenz_rust2::Archive::read(&mut sub, &sevenz_rust2::Password::empty())
    }));
    Ok(matches!(opened, Ok(Ok(_))))
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
    fn sniff_table() {
        let cases: [(&[u8], Sniffed); 7] = [
            (
                &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0x00, 0x04],
                Sniffed::SevenZip,
            ),
            (&[0x50, 0x4B, 0x03, 0x04, 0x14, 0x00], Sniffed::Zip),
            (&[0x50, 0x4B, 0x05, 0x06, 0x00, 0x00], Sniffed::Zip),
            (&[0x4D, 0x5A, 0x90, 0x00], Sniffed::Pe),
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
