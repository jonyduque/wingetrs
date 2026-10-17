#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::Cursor;
use std::path::Path;

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
    // No wall-clock bound (flaky under load); `DictionaryTooLarge` itself proves the refusal
    // came from the pre-decode check.
    let r = extract(&f, "h.7z", None, None, &d.path().join("out"));
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

/// Replaces `old_len` header bytes at `at` with `new` and fixes the next-header size and both CRCs.
fn splice_header(a: &mut Vec<u8>, at: usize, old_len: usize, new: &[u8]) {
    let (_, len) = next_header(a);
    a.splice(at..at + old_len, new.iter().copied());
    let len = (len + new.len() - old_len) as u64;
    a[20..28].copy_from_slice(&len.to_le_bytes());
    fix_crcs(a);
}

/// The single position of `pat` inside the raw (unencoded) next header.
fn find_in_header(a: &[u8], pat: &[u8]) -> usize {
    let (at, len) = next_header(a);
    assert_eq!(a[at], 0x01, "header must be stored raw for this test");
    let hits: Vec<usize> = (at..at + len - pat.len())
        .filter(|&i| a[i..i + pat.len()] == *pat)
        .collect();
    assert_eq!(hits.len(), 1, "{pat:02X?}: {hits:?}");
    hits[0]
}

#[test]
fn anti_item_refused() {
    let mut anti = ArchiveEntry::new_file("gone.txt");
    anti.is_anti_item = true;
    let a = sz(vec![file("a.txt", b"a"), (anti, None)], None);
    let (d, r) = run(&a, "x.7z", None, &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::UnsafePath(ref n)) if n == "gone.txt"),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["x.7z"]);
}

#[test]
fn too_many_entries_refused() {
    let a = sz(vec![file("a.txt", b"a"), file("b.txt", b"b")], None);
    let one = Limits {
        max_entries: 1,
        ..DEFAULT_LIMITS
    };
    let (d, r) = run(&a, "x.7z", None, &one);
    assert!(
        matches!(r, Err(ExtractError::TooManyEntries { limit: 1 })),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["x.7z"]);
    let two = Limits {
        max_entries: 2,
        ..DEFAULT_LIMITS
    };
    run(&a, "x.7z", None, &two).1.unwrap();
}

#[test]
fn lzma_and_ppmd_dictionaries_refused() {
    // Coder flags 0x23 (3-byte id + properties), id, 5 property bytes: [byte, u32 LE size].
    for (method, id) in [
        (EncoderMethod::LZMA, [0x03, 0x01, 0x01]),
        (EncoderMethod::PPMD, [0x03, 0x04, 0x01]),
    ] {
        let mut a = sz(vec![file("a.txt", &pseudo_random(1000))], Some(method));
        let at = find_in_header(&a, &[0x23, id[0], id[1], id[2], 0x05]);
        a[at + 6..at + 10].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        fix_crcs(&mut a);
        let (d, r) = run(&a, "h.7z", None, &DEFAULT_LIMITS);
        assert!(
            matches!(
                r,
                Err(ExtractError::DictionaryTooLarge {
                    dict: 0x8000_0000,
                    limit: 0x4000_0000
                })
            ),
            "{id:02X?}: {r:?}"
        );
        assert_eq!(list(d.path()), ["h.7z"]);
    }
}

/// LZMA2 coder in the raw header: flags 0x21, id 0x21, 1 property byte.
fn lzma2_coder(a: &[u8]) -> usize {
    find_in_header(a, &[0x01, 0x21, 0x21, 0x01]) + 1
}

#[test]
fn encrypted_unknown_method_and_bad_lzma2_property_refused() {
    let base = sz(vec![file("a.txt", &pseudo_random(1000))], None);

    let mut aes = base.clone();
    let at = lzma2_coder(&aes);
    let p = aes[at + 3];
    splice_header(&mut aes, at, 4, &[0x24, 0x06, 0xF1, 0x07, 0x01, 0x01, p]);
    let (_, r) = run(&aes, "e.7z", None, &DEFAULT_LIMITS);
    assert!(matches!(r, Err(ExtractError::Encrypted)), "{r:?}");

    let mut unknown = base.clone();
    let at = lzma2_coder(&unknown);
    unknown[at + 1] = 0x7F;
    fix_crcs(&mut unknown);
    let (_, r) = run(&unknown, "u.7z", None, &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::UnsupportedMethod(ref m)) if m == "[7F]"),
        "{r:?}"
    );

    let mut bad = base;
    let at = lzma2_coder(&bad);
    bad[at + 3] = 41;
    fix_crcs(&mut bad);
    let (d, r) = run(&bad, "b.7z", None, &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::Archive(ref m)) if m == "propriedade LZMA2 inválida: 41"),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["b.7z"]);
}

/// Many long names, so the writer stores the header LZMA-encoded (kEncodedHeader).
fn with_encoded_header() -> Vec<u8> {
    let entries = (0..80)
        .map(|i| file(&format!("some/long/directory/name/file-{i:03}.txt"), b"x"))
        .collect();
    sz(entries, None)
}

#[test]
fn encoded_header_with_huge_unpack_size_refused() {
    let mut a = with_encoded_header();
    // Untouched, it extracts.
    run(&a, "e.7z", None, &DEFAULT_LIMITS).1.unwrap();
    let (at, len) = next_header(&a);
    assert_eq!(a[at], 0x17, "header must be encoded for this test");
    // Header coder: flags 0x23, LZMA id, 5 property bytes, then kCodersUnpackSize and the size.
    let pat = [0x23, 0x03, 0x01, 0x01, 0x05];
    let hits: Vec<usize> = (at..at + len - pat.len())
        .filter(|&i| a[i..i + pat.len()] == pat)
        .collect();
    assert_eq!(hits.len(), 1, "{hits:?}");
    let size_at = hits[0] + 10;
    assert_eq!(a[size_at], 0x0C);
    // The current size takes 2 bytes (0x80 | high, low) for a header of a few KiB.
    assert_eq!(a[size_at + 1] & 0xC0, 0x80, "{:02X}", a[size_at + 1]);
    // 2 GiB: 4 extra bytes (prefix 0xF0), little endian.
    splice_header(&mut a, size_at + 1, 2, &[0xF0, 0x00, 0x00, 0x00, 0x80]);
    // No wall-clock bound (it failed the gate under load); the exact message proves the
    // raw pre-check refused it before the crate decoded the header.
    let (d, r) = run(&a, "e.7z", None, &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::Archive(ref m)) if m == "cabeçalho 7z grande demais"),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["e.7z"]);

    // The same header inside a PE is refused by the pre-check, before the library decodes it.
    let mut sfx = mz(4096);
    sfx.extend_from_slice(&a);
    let log = LogBuf::default();
    let sub = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(log.clone())
        .finish();
    let (_, r) =
        tracing::subscriber::with_default(sub, || run(&sfx, "sfx.7z", None, &DEFAULT_LIMITS));
    assert!(
        matches!(r, Err(ExtractError::Unsupported(ref m)) if m.contains("instalador")),
        "{r:?}"
    );
    let log = log.text();
    assert!(
        log.contains(
            "candidato 7z recusado offset=4098 error=arquivo inválido: cabeçalho 7z grande demais\n"
        ),
        "{log}"
    );
}

/// Collects formatted tracing output.
#[derive(Clone, Default)]
struct LogBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl LogBuf {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl std::io::Write for LogBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuf {
    type Writer = LogBuf;
    fn make_writer(&'a self) -> LogBuf {
        self.clone()
    }
}

#[test]
fn oversized_raw_next_header_refused() {
    let mut a = tree_2();
    a[20..28].copy_from_slice(&(65u64 << 20).to_le_bytes());
    let s = crc32(&a[12..32]);
    a[8..12].copy_from_slice(&s.to_le_bytes());
    let (_, r) = run(&a, "o.7z", None, &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::Archive(ref m)) if m == "cabeçalho 7z grande demais"),
        "{r:?}"
    );
}

#[test]
fn extract_dir_holding_only_nsis_is_missing() {
    let a = solid(&[("d/x.nsis", b"n".to_vec()), ("other/y", b"y".to_vec())]);
    let (d, r) = run(&a, "n.7z", Some("d"), &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::MissingExtractDir(ref n)) if n == "d"),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["n.7z"]);
}

#[test]
fn zeroed_start_header_refused() {
    // 7-Zip leaves the start header zeroed when interrupted; sevenz-rust2 then guesses the header unchecked.
    let mut a = tree_2();
    a[8..32].fill(0);
    let (d, r) = run(&a, "z.7z", None, &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::Archive(ref m)) if m == "cabeçalho inicial do 7z zerado"),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["z.7z"]);
    // Inside a PE it is not an SFX candidate either.
    let mut sfx = mz(4096);
    sfx.extend_from_slice(&a);
    let (_, r) = run(&sfx, "sfx.7z", None, &DEFAULT_LIMITS);
    assert!(
        matches!(r, Err(ExtractError::Unsupported(ref m)) if m.contains("instalador")),
        "{r:?}"
    );
}
