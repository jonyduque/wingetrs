#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::Cursor;
use std::path::Path;
use std::time::{Duration, Instant};

use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderMethod, SourceReader};
use sha2::{Digest, Sha256};
use wingetrs::install::extract::{
    DEFAULT_LIMITS, ExtractError, ExtractSource, Limits, extract, extract_all_with,
};

/// Non-solid 7z: one block per file; `None` data = directory or empty file.
fn sz(entries: Vec<(ArchiveEntry, Option<Vec<u8>>)>, method: Option<EncoderMethod>) -> Vec<u8> {
    let mut w = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    if let Some(m) = method {
        w.set_content_methods(vec![m.into()]);
    }
    for (e, data) in entries {
        w.push_archive_entry(e, data.map(Cursor::new)).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn file(name: &str, data: &[u8]) -> (ArchiveEntry, Option<Vec<u8>>) {
    (ArchiveEntry::new_file(name), Some(data.to_vec()))
}

/// Solid 7z: every file in one LZMA2 block.
fn solid(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut w = ArchiveWriter::new(Cursor::new(Vec::new())).unwrap();
    let entries = files
        .iter()
        .map(|(n, _)| ArchiveEntry::new_file(n))
        .collect();
    let readers = files
        .iter()
        .map(|(_, d)| SourceReader::new(Cursor::new(d.clone())))
        .collect();
    w.push_archive_entries(entries, readers).unwrap();
    w.finish().unwrap().into_inner()
}

fn pseudo_random(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// (offset, length) of the next header, from the start header.
fn next_header(a: &[u8]) -> (usize, usize) {
    let off = u64::from_le_bytes(a[12..20].try_into().unwrap()) as usize;
    let len = u64::from_le_bytes(a[20..28].try_into().unwrap()) as usize;
    (32 + off, len)
}

/// Recomputes the next-header CRC and the start-header CRC after editing the header.
fn fix_crcs(a: &mut [u8]) {
    let (at, len) = next_header(a);
    let h = crc32(&a[at..at + len]);
    a[28..32].copy_from_slice(&h.to_le_bytes());
    let s = crc32(&a[12..32]);
    a[8..12].copy_from_slice(&s.to_le_bytes());
}

fn tree_2() -> Vec<u8> {
    let mut empty_dir = ArchiveEntry::new_directory("empty");
    empty_dir.has_stream = false;
    sz(
        vec![
            file("a.txt", b"hello 7z\r\n"),
            file("sub/b.bin", &pseudo_random(100 * 1024)),
            (empty_dir, None),
            (ArchiveEntry::new_file("zero.txt"), None),
        ],
        None,
    )
}

fn list(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn run(
    bytes: &[u8],
    name: &str,
    extract_dir: Option<&str>,
    limits: &Limits,
) -> (tempfile::TempDir, Result<(), ExtractError>) {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join(name);
    std::fs::write(&f, bytes).unwrap();
    let r = extract_all_with(
        &[ExtractSource {
            file: &f,
            file_name: name,
            extract_dir,
            extract_to: None,
        }],
        &d.path().join("out"),
        limits,
    );
    (d, r)
}

#[test]
fn extracts_7z_tree() {
    let (d, r) = run(&tree_2(), "a.7z", None, &DEFAULT_LIMITS);
    r.unwrap();
    let out = d.path().join("out");
    assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"hello 7z\r\n");
    assert_eq!(
        Sha256::digest(std::fs::read(out.join("sub/b.bin")).unwrap()),
        Sha256::digest(pseudo_random(100 * 1024))
    );
    assert!(out.join("empty").is_dir());
    assert_eq!(list(&out.join("empty")), Vec::<String>::new());
    assert_eq!(std::fs::read(out.join("zero.txt")).unwrap(), b"");
    assert_eq!(list(&out), ["a.txt", "empty", "sub", "zero.txt"]);
}

#[test]
fn extract_dir_on_solid_7z_charges_skipped() {
    let a = solid(&[
        ("keep/x", b"0123456789".to_vec()),
        ("skip/y", vec![0u8; 2 << 20]),
    ]);
    let limits = Limits {
        ratio: 1,
        floor: 1 << 20,
        ..DEFAULT_LIMITS
    };
    let (d, r) = run(&a, "s.7z", Some("keep"), &limits);
    assert!(matches!(r, Err(ExtractError::TooLarge { .. })), "{r:?}");
    assert!(!d.path().join("out").exists());
    // Same archive under the default budget: only keep/ lands.
    let (d, r) = run(&a, "s.7z", Some("keep"), &DEFAULT_LIMITS);
    r.unwrap();
    assert_eq!(list(&d.path().join("out")), ["x"]);
}

#[test]
fn unsafe_second_entry_writes_nothing() {
    let entries = || vec![file("ok.txt", b"fine"), file("../x", b"evil")];
    let intact = sz(entries(), Some(EncoderMethod::COPY));
    // Same archive with the packed data overwritten: refusing by name must not need to decode.
    let mut garbled = intact.clone();
    let (hdr, _) = next_header(&garbled);
    garbled[32..hdr].fill(0xEE);
    for a in [intact, garbled] {
        let outer = tempfile::tempdir().unwrap();
        let parent = outer.path().join("p");
        std::fs::create_dir(&parent).unwrap();
        let f = outer.path().join("a.7z");
        std::fs::write(&f, &a).unwrap();
        let before = list(&parent);
        let r = extract(&f, "a.7z", None, None, &parent.join("dest"));
        assert!(
            matches!(r, Err(ExtractError::UnsafePath(ref n)) if n == "../x"),
            "{r:?}"
        );
        assert_eq!(list(&parent), before);
        assert_eq!(list(outer.path()), ["a.7z", "p"]);
    }
}

#[test]
fn reparse_and_unix_symlink_refused() {
    for attrs in [0x400u32, 0x8000 | (0o120777 << 16)] {
        let mut link = ArchiveEntry::new_file("link");
        link.has_windows_attributes = true;
        link.windows_attributes = attrs;
        let a = sz(
            vec![file("a.txt", b"a"), (link, Some(b"target".to_vec()))],
            None,
        );
        let (d, r) = run(&a, "l.7z", None, &DEFAULT_LIMITS);
        assert!(
            matches!(r, Err(ExtractError::UnsafePath(ref n)) if n == "link"),
            "{attrs:#x}: {r:?}"
        );
        assert_eq!(list(d.path()), ["l.7z"]);
    }
    // A plain unix file mode (0o100644) with the extension flag is fine.
    let mut plain = ArchiveEntry::new_file("p.txt");
    plain.has_windows_attributes = true;
    plain.windows_attributes = 0x8000 | (0o100644 << 16);
    let (d, r) = run(
        &sz(vec![(plain, Some(b"p".to_vec()))], None),
        "p.7z",
        None,
        &DEFAULT_LIMITS,
    );
    r.unwrap();
    assert_eq!(std::fs::read(d.path().join("out/p.txt")).unwrap(), b"p");
}

#[test]
fn huge_dictionary_refused_before_decoding() {
    let mut a = sz(vec![file("a.txt", &pseudo_random(1000))], None);
    let (at, len) = next_header(&a);
    assert_eq!(a[at], 0x01, "header must be stored raw for this test");
    // Folder with 1 coder: flags 0x21 (1-byte id + properties), id 0x21 (LZMA2), 1 property byte.
    let pat = [0x01, 0x21, 0x21, 0x01];
    let hits: Vec<usize> = (at..at + len - 4).filter(|&i| a[i..i + 4] == pat).collect();
    assert_eq!(hits.len(), 1, "{hits:?}");
    a[hits[0] + 4] = 40;
    fix_crcs(&mut a);
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("h.7z");
    std::fs::write(&f, &a).unwrap();
    let t = Instant::now();
    let r = extract(&f, "h.7z", None, None, &d.path().join("out"));
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert!(
        matches!(
            r,
            Err(ExtractError::DictionaryTooLarge {
                dict: 0xFFFF_FFFF,
                limit: 0x4000_0000
            })
        ),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["h.7z"]);
}

#[test]
fn zip_named_7z_goes_to_zip() {
    let mut buf = Cursor::new(Vec::new());
    {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(&mut buf);
        z.start_file("in-zip.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(b"z").unwrap();
        z.finish().unwrap();
    }
    let (d, r) = run(&buf.into_inner(), "dl.7z", None, &DEFAULT_LIMITS);
    r.unwrap();
    assert_eq!(
        std::fs::read(d.path().join("out/in-zip.txt")).unwrap(),
        b"z"
    );
}

fn mz(pad: usize) -> Vec<u8> {
    let mut v = b"MZ".to_vec();
    v.resize(2 + pad, 0);
    v
}

#[test]
fn pe_without_7z_is_unsupported() {
    let (d, r) = run(&mz(4096), "setup.7z", None, &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::Unsupported(ref m)) if m.contains("instalador")),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["setup.7z"]);
}

#[test]
fn sfx_7z_extracts() {
    let mut a = mz(4096);
    a.extend_from_slice(&tree_2());
    let (d, r) = run(&a, "sfx.7z", None, &DEFAULT_LIMITS);
    r.unwrap();
    assert_eq!(
        Sha256::digest(std::fs::read(d.path().join("out/sub/b.bin")).unwrap()),
        Sha256::digest(pseudo_random(100 * 1024))
    );
    assert_eq!(
        list(&d.path().join("out")),
        ["a.txt", "empty", "sub", "zero.txt"]
    );

    let mut stray = mz(4096);
    stray.extend_from_slice(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0x00, 0x04]);
    stray.extend_from_slice(&pseudo_random(4096));
    let (d, r) = run(&stray, "sfx.7z", None, &DEFAULT_LIMITS);
    assert!(matches!(r, Err(ExtractError::Unsupported(_))), "{r:?}");
    assert_eq!(list(d.path()), ["sfx.7z"]);
}

#[test]
fn truncated_7z_is_error_not_panic() {
    let a = tree_2();
    let (hdr, _) = next_header(&a);
    for cut in [32, 100, a.len() / 2, hdr + 3, a.len() - 1] {
        let (d, r) = run(&a[..cut], "t.7z", None, &DEFAULT_LIMITS);
        assert!(
            matches!(r, Err(ExtractError::Archive(_))),
            "cut {cut}: {r:?}"
        );
        assert_eq!(list(d.path()), ["t.7z"]);
    }
}

#[test]
fn short_decoded_stream_is_an_error() {
    // COPY archive whose header claims 6 bytes for a 5-byte stream: the library ends quietly.
    let mut a = sz(vec![file("a.txt", b"hello")], Some(EncoderMethod::COPY));
    let (at, len) = next_header(&a);
    assert_eq!(a[at], 0x01, "header must be stored raw for this test");
    // kCodersUnpackSize (0x0C) followed by the size.
    let hits: Vec<usize> = (at..at + len - 1)
        .filter(|&i| a[i] == 0x0C && a[i + 1] == 0x05)
        .collect();
    assert_eq!(hits.len(), 1, "{hits:?}");
    a[hits[0] + 1] = 0x06;
    fix_crcs(&mut a);
    let (d, r) = run(&a, "s.7z", None, &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::Archive(ref m)) if m.contains("5 de 6")),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["s.7z"]);
}

#[test]
fn nsis_entries_skipped() {
    let a = solid(&[
        ("app.exe", b"exe".to_vec()),
        ("$PLUGINSDIR/script.NSIS", b"nsis".to_vec()),
        ("data/[NSIS].nsis", b"nsis2".to_vec()),
        ("readme.txt", b"r".to_vec()),
    ]);
    let (d, r) = run(&a, "n.7z", None, &DEFAULT_LIMITS);
    r.unwrap();
    let out = d.path().join("out");
    assert_eq!(list(&out), ["app.exe", "readme.txt"]);
    assert_eq!(std::fs::read(out.join("readme.txt")).unwrap(), b"r");
}

#[test]
fn corrupt_data_is_an_archive_error() {
    let mut a = sz(vec![file("a.txt", b"hello")], Some(EncoderMethod::COPY));
    a[32] ^= 0xFF;
    let (d, r) = run(&a, "c.7z", None, &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::Archive(ref m)) if m.contains("ChecksumVerificationFailed")),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["c.7z"]);
}
