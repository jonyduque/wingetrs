#![allow(clippy::unwrap_used, clippy::expect_used)]
//! MSI extraction (Task 125). The MSI is built here with the `msi` crate; the
//! cabinets in `tests/fixtures/msi/` come from `makecab` (command in the Task 125 report):
//! `three.cab` (LZX:21) = f_a, f_b, f_c; `one.cab` = f_one; `two.cab` = f_two;
//! `many.cab` (MSZIP, one folder) = m000..m199, each `"file NNN\n"` × 50.
use std::cell::Cell;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::rc::Rc;

use msi::{Column, Insert, Package, PackageType, Value};
use sha2::{Digest, Sha256};
use wingetrs::install::extract::{
    DEFAULT_LIMITS, ExtractError, ExtractSource, Limits, extract_all_with, needs_installer,
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/msi");

fn cab(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(FIXTURES).join(name)).unwrap()
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

const F_A: &[u8] = b"alpha\r\n";
const F_C: &[u8] = b"gamma file\r\n";

fn f_b() -> Vec<u8> {
    pseudo_random(40 * 1024)
}

struct FileSpec {
    key: &'static str,
    comp: &'static str,
    name: &'static str,
    version: Option<&'static str>,
    attributes: i32,
    sequence: i32,
}

fn file(key: &'static str, comp: &'static str, name: &'static str, sequence: i32) -> FileSpec {
    FileSpec {
        key,
        comp,
        name,
        version: None,
        attributes: 0,
        sequence,
    }
}

struct MsiSpec {
    word_count: i32,
    dirs: Vec<(&'static str, Option<&'static str>, &'static str)>,
    comps: Vec<(String, &'static str)>,
    files: Vec<FileSpec>,
    media: Vec<(i32, i32, &'static str)>,
    streams: Vec<(&'static str, Vec<u8>)>,
    properties: Vec<(&'static str, &'static str)>,
}

/// TARGETDIR + `dirs`; one component per directory, named `C_<dir>`.
fn spec(
    dirs: Vec<(&'static str, Option<&'static str>, &'static str)>,
    files: Vec<FileSpec>,
) -> MsiSpec {
    let mut all = vec![("TARGETDIR", None, "SourceDir")];
    all.extend(dirs);
    let comps = all.iter().map(|d| (format!("C_{}", d.0), d.0)).collect();
    MsiSpec {
        word_count: 2,
        dirs: all,
        comps,
        files,
        media: vec![(1, 100, "#cab1")],
        streams: vec![("cab1", cab("three.cab"))],
        properties: vec![("ProductName", "Teste")],
    }
}

fn build(s: &MsiSpec) -> Vec<u8> {
    let mut p = Package::create(PackageType::Installer, Cursor::new(Vec::new())).unwrap();
    p.summary_info_mut().set_word_count(s.word_count);
    p.create_table(
        "Directory",
        vec![
            Column::build("Directory").primary_key().id_string(72),
            Column::build("Directory_Parent").nullable().id_string(72),
            Column::build("DefaultDir").text_string(255),
        ],
    )
    .unwrap();
    p.create_table(
        "Component",
        vec![
            Column::build("Component").primary_key().id_string(72),
            Column::build("Directory_").id_string(72),
        ],
    )
    .unwrap();
    p.create_table(
        "File",
        vec![
            Column::build("File").primary_key().id_string(72),
            Column::build("Component_").id_string(72),
            Column::build("FileName").text_string(255),
            Column::build("FileSize").int32(),
            Column::build("Version").nullable().string(72),
            Column::build("Attributes").nullable().int16(),
            Column::build("Sequence").int32(),
        ],
    )
    .unwrap();
    p.create_table(
        "Media",
        vec![
            Column::build("DiskId").primary_key().int16(),
            Column::build("LastSequence").int32(),
            Column::build("Cabinet").nullable().string(255),
        ],
    )
    .unwrap();
    p.create_table(
        "Property",
        vec![
            Column::build("Property").primary_key().id_string(72),
            Column::build("Value").text_string(0),
        ],
    )
    .unwrap();
    for (d, parent, default) in &s.dirs {
        let parent = parent.map_or(Value::Null, Value::from);
        p.insert_rows(Insert::into("Directory").row(vec![(*d).into(), parent, (*default).into()]))
            .unwrap();
    }
    for (c, d) in &s.comps {
        p.insert_rows(Insert::into("Component").row(vec![c.as_str().into(), (*d).into()]))
            .unwrap();
    }
    for f in &s.files {
        p.insert_rows(Insert::into("File").row(vec![
            f.key.into(),
            f.comp.into(),
            f.name.into(),
            0.into(),
            f.version.map_or(Value::Null, Value::from),
            Value::Int(f.attributes),
            f.sequence.into(),
        ]))
        .unwrap();
    }
    for (disk, last, cabinet) in &s.media {
        p.insert_rows(Insert::into("Media").row(vec![
            Value::Int(*disk),
            (*last).into(),
            (*cabinet).into(),
        ]))
        .unwrap();
    }
    for (k, v) in &s.properties {
        p.insert_rows(Insert::into("Property").row(vec![(*k).into(), (*v).into()]))
            .unwrap();
    }
    for (name, data) in &s.streams {
        p.write_stream(name).unwrap().write_all(data).unwrap();
    }
    p.into_inner().unwrap().into_inner()
}

fn list(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn run(bytes: &[u8], extract_dir: Option<&str>) -> (tempfile::TempDir, Result<(), ExtractError>) {
    run_with(bytes, extract_dir, &DEFAULT_LIMITS)
}

fn run_with(
    bytes: &[u8],
    extract_dir: Option<&str>,
    limits: &Limits,
) -> (tempfile::TempDir, Result<(), ExtractError>) {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("x.msi");
    std::fs::write(&f, bytes).unwrap();
    let r = extract_all_with(
        &[ExtractSource {
            file: &f,
            file_name: "x.msi",
            extract_dir,
            extract_to: None,
        }],
        &d.path().join("out"),
        limits,
    );
    (d, r)
}

fn sha(p: &Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(p).unwrap()).to_vec()
}

fn unsupported(r: Result<(), ExtractError>) -> String {
    match r {
        Err(ExtractError::Unsupported(m)) => m,
        other => panic!("esperava Unsupported: {other:?}"),
    }
}

fn tree_spec() -> MsiSpec {
    spec(
        vec![
            ("D1", Some("TARGETDIR"), "tgt1|Target One:src1|Source One"),
            ("D2", Some("D1"), "tgt2:Bin"),
        ],
        vec![
            file("f_a", "C_D1", "a.txt", 1),
            file("f_b", "C_D2", "B~1.BIN|b.bin", 2),
            file("f_c", "C_D2", "c.txt", 3),
        ],
    )
}

#[test]
fn extracts_msi_tree() {
    let (d, r) = run(&build(&tree_spec()), None);
    r.unwrap();
    let out = d.path().join("out");
    assert_eq!(list(&out), ["Source One"]);
    assert_eq!(list(&out.join("Source One")), ["Bin", "a.txt"]);
    assert_eq!(list(&out.join("Source One/Bin")), ["b.bin", "c.txt"]);
    assert_eq!(
        sha(&out.join("Source One/a.txt")),
        Sha256::digest(F_A).to_vec()
    );
    assert_eq!(
        sha(&out.join("Source One/Bin/b.bin")),
        Sha256::digest(f_b()).to_vec()
    );
    assert_eq!(
        sha(&out.join("Source One/Bin/c.txt")),
        Sha256::digest(F_C).to_vec()
    );
}

#[test]
fn two_cabinets() {
    let mut s = spec(
        vec![],
        vec![
            file("f_one", "C_TARGETDIR", "one.txt", 1),
            file("f_two", "C_TARGETDIR", "two.txt", 2),
        ],
    );
    s.media = vec![(1, 1, "#cab1"), (2, 2, "#cab2")];
    s.streams = vec![("cab1", cab("one.cab")), ("cab2", cab("two.cab"))];
    let (d, r) = run(&build(&s), None);
    r.unwrap();
    let out = d.path().join("out");
    assert_eq!(list(&out), ["one.txt", "two.txt"]);
    assert_eq!(std::fs::read(out.join("one.txt")).unwrap(), b"one\r\n");
    assert_eq!(std::fs::read(out.join("two.txt")).unwrap(), b"two\r\n");
}

#[test]
fn external_cabinet_unsupported() {
    let mut s = tree_spec();
    s.media = vec![(1, 100, "data1.cab")];
    let (d, r) = run(&build(&s), None);
    let m = unsupported(r);
    assert!(m.contains("gabinete externo"), "{m}");
    assert_eq!(list(d.path()), ["x.msi"]);
}

#[test]
fn uncompressed_unsupported() {
    let mut s = tree_spec();
    s.word_count = 0;
    let (d, r) = run(&build(&s), None);
    let m = unsupported(r);
    assert!(m.contains("sem compressão"), "{m}");
    assert_eq!(list(d.path()), ["x.msi"]);
    let mut ext = tree_spec();
    ext.media = vec![(1, 100, "data1.cab")];
    assert_ne!(m, unsupported(run(&build(&ext), None).1));
}

#[test]
fn noncompressed_file_unsupported() {
    let mut s = tree_spec();
    s.files[1].attributes = 0x2000;
    let m = unsupported(run(&build(&s), None).1);
    assert!(m.contains("não comprimido"), "{m}");
}

#[test]
fn quantum_cabinet_unsupported() {
    let mut c = cab("three.cab");
    // No reserve area: CFFOLDER starts at 36, typeCompress at 42. Quantum level 7, memory 20.
    assert_eq!(c[30] & 0x4, 0);
    c[42..44].copy_from_slice(&0x1472u16.to_le_bytes());
    let mut s = tree_spec();
    s.streams = vec![("cab1", c)];
    let (d, r) = run(&build(&s), None);
    let m = unsupported(r);
    assert!(m.contains("Quantum"), "{m}");
    assert_eq!(list(d.path()), ["x.msi"]);
}

#[test]
fn transform_required_unsupported() {
    let mut s = tree_spec();
    s.properties.push(("TRANSFORMS", ":lang.mst"));
    let m = unsupported(run(&build(&s), None).1);
    assert!(m.contains("transform"), "{m}");
}

#[test]
fn not_an_msi_is_unsupported() {
    let m = unsupported(run(b"PK\x03\x04 not an msi", None).1);
    assert_eq!(m, "conteúdo não é MSI");
}

#[test]
fn unsafe_layout_writes_nothing() {
    let mut s = tree_spec();
    s.files[2].name = "..";
    let (d, r) = run(&build(&s), None);
    assert!(matches!(r, Err(ExtractError::MsiLayout(_))), "{r:?}");
    assert_eq!(list(d.path()), ["x.msi"]);
}

#[test]
fn file_without_data_is_layout_error() {
    let mut s = tree_spec();
    s.files.push(file("f_zz", "C_D1", "zz.txt", 4));
    let (d, r) = run(&build(&s), None);
    match r {
        Err(ExtractError::MsiLayout(m)) => {
            assert!(m.contains("f_zz") && m.contains("sem dados"), "{m}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(list(d.path()), ["x.msi"]);
}

#[test]
fn extract_dir_case_and_slash() {
    let s = spec(
        vec![
            ("PF", Some("TARGETDIR"), "PFiles"),
            ("APP", Some("PF"), "app"),
            ("OTHER", Some("TARGETDIR"), "Other"),
        ],
        vec![
            file("f_a", "C_APP", "a.txt", 1),
            file("f_b", "C_OTHER", "b.bin", 2),
            file("f_c", "C_APP", "c.txt", 3),
        ],
    );
    let (d, r) = run(&build(&s), Some("pfiles/App"));
    r.unwrap();
    let out = d.path().join("out");
    assert_eq!(list(&out), ["a.txt", "c.txt"]);
    assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), F_A);
}

#[test]
fn collision_loser_not_written() {
    let mut s = spec(
        vec![("D", Some("TARGETDIR"), "App")],
        vec![
            file("f_a", "C_D", "a.txt", 1),
            file("f_b", "C_D", "same.txt", 2),
            file("f_c", "C_D", "SAME.txt", 3),
        ],
    );
    s.files[1].version = Some("1.0");
    s.files[2].version = Some("2.0");
    let (d, r) = run(&build(&s), None);
    r.unwrap();
    let out = d.path().join("out/App");
    assert_eq!(list(&out), ["SAME.txt", "a.txt"]);
    assert_eq!(std::fs::read(out.join("SAME.txt")).unwrap(), F_C);
}

/// Counts the bytes read from the compressed stream.
struct Counting {
    inner: Cursor<Vec<u8>>,
    n: Rc<Cell<u64>>,
}

impl Read for Counting {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.n.set(self.n.get() + n as u64);
        Ok(n)
    }
}

impl Seek for Counting {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[test]
fn sequential_reader_is_linear() {
    let bytes = cab("many.cab");
    let len = bytes.len() as u64;
    let counter = Rc::new(Cell::new(0u64));
    let reader = Counting {
        inner: Cursor::new(bytes),
        n: counter.clone(),
    };
    let mut cabinet = cab::Cabinet::new(reader).unwrap();
    let folders: Vec<usize> = cabinet
        .folder_entries()
        .map(|f| f.file_entries().count())
        .collect();
    assert_eq!(folders, [200]);
    let mut seen = Vec::new();
    cabinet
        .for_each_file(|name, data| {
            let mut v = Vec::new();
            data.read_to_end(&mut v)?;
            let i: usize = name[1..].parse().unwrap();
            assert_eq!(
                v,
                format!("file {i:03}\n").repeat(50).into_bytes(),
                "{name}"
            );
            seen.push(i);
            Ok(())
        })
        .unwrap();
    seen.sort();
    assert_eq!(seen, (0..200).collect::<Vec<_>>());
    assert!(
        counter.get() <= 2 * len,
        "leu {} de um gabinete de {len}",
        counter.get()
    );
}

#[test]
fn truncated_cab_is_error() {
    let c = cab("three.cab");
    for cut in [10, 60, c.len() / 2, c.len() - 1] {
        let mut s = tree_spec();
        s.streams = vec![("cab1", c[..cut].to_vec())];
        let (d, r) = run(&build(&s), None);
        assert!(
            matches!(r, Err(ExtractError::Archive(_))),
            "cut {cut}: {r:?}"
        );
        assert_eq!(list(d.path()), ["x.msi"], "cut {cut}");
    }
}

/// Byte offset of CFFILE entry `i` (16 fixed bytes, then the NUL-terminated name).
fn cffile(c: &[u8], i: usize) -> usize {
    let mut at = u32::from_le_bytes(c[16..20].try_into().unwrap()) as usize;
    for _ in 0..i {
        at += 16;
        at += c[at..].iter().position(|&b| b == 0).unwrap() + 1;
    }
    at
}

fn with_cab(s: &mut MsiSpec, c: Vec<u8>) -> Vec<u8> {
    s.streams = vec![("cab1", c)];
    build(s)
}

fn one_file_spec() -> MsiSpec {
    let mut s = spec(vec![], vec![file("f_one", "C_TARGETDIR", "one.txt", 1)]);
    s.streams = vec![("cab1", cab("one.cab"))];
    s
}

#[test]
fn overlapping_files_refused() {
    let mut c = cab("three.cab");
    // f_c starts where f_b starts (f_a is 7 bytes at 0, f_b at 7).
    let (b, cc) = (cffile(&c, 1), cffile(&c, 2));
    assert_eq!(&c[b + 16..b + 20], b"f_b\0");
    let off: [u8; 4] = c[b + 4..b + 8].try_into().unwrap();
    c[cc + 4..cc + 8].copy_from_slice(&off);
    let (d, r) = run(&with_cab(&mut tree_spec(), c), None);
    match r {
        Err(ExtractError::Archive(m)) => assert!(m.contains("sobrepostos"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(list(d.path()), ["x.msi"]);
}

#[test]
fn gap_before_file_is_charged() {
    let mut c = cab("one.cab");
    let f = cffile(&c, 0);
    c[f + 4..f + 8].copy_from_slice(&30_000u32.to_le_bytes());
    let limits = Limits {
        ratio: 1,
        floor: 1000,
        ceiling: 1000,
        ..DEFAULT_LIMITS
    };
    let (d, r) = run_with(&with_cab(&mut one_file_spec(), c), None, &limits);
    assert!(
        matches!(r, Err(ExtractError::TooLarge { limit: 1000 })),
        "{r:?}"
    );
    assert_eq!(list(d.path()), ["x.msi"]);
}

#[test]
fn short_folder_data_is_error() {
    let mut c = cab("one.cab");
    // f_one declares 100 bytes; the folder only decodes 5.
    let f = cffile(&c, 0);
    assert_eq!(&c[f..f + 4], 5u32.to_le_bytes());
    c[f..f + 4].copy_from_slice(&100u32.to_le_bytes());
    let (d, r) = run(&with_cab(&mut one_file_spec(), c), None);
    match r {
        Err(ExtractError::Archive(m)) => assert!(m.contains("terminou com"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(list(d.path()), ["x.msi"]);
}

#[test]
fn continued_cabinet_unsupported() {
    // Header flag cfhdrNEXT_CABINET.
    let mut c = cab("three.cab");
    c[30] |= 0x2;
    let m = unsupported(run(&with_cab(&mut tree_spec(), c), None).1);
    assert!(m.contains("continuados"), "{m}");
    // A CFFILE continued from the previous cabinet (iFolder 0xFFFD), flags clear.
    let mut c = cab("three.cab");
    let f = cffile(&c, 0);
    c[f + 8..f + 10].copy_from_slice(&0xFFFDu16.to_le_bytes());
    let m = unsupported(run(&with_cab(&mut tree_spec(), c), None).1);
    assert!(m.contains("continuados"), "{m}");
}

#[test]
fn shared_span_extracts_both_files() {
    // f_c points at f_b's bytes: WiX stores identical files once (pkgconf, yubico-piv-tool).
    let mut c = cab("three.cab");
    let (b, cc) = (cffile(&c, 1), cffile(&c, 2));
    let span: [u8; 8] = c[b..b + 8].try_into().unwrap();
    c[cc..cc + 8].copy_from_slice(&span);
    let (d, r) = run(&with_cab(&mut tree_spec(), c), None);
    r.unwrap();
    let out = d.path().join("out/Source One");
    assert_eq!(list(&out.join("Bin")), ["b.bin", "c.txt"]);
    assert_eq!(sha(&out.join("a.txt")), Sha256::digest(F_A).to_vec());
    assert_eq!(sha(&out.join("Bin/b.bin")), Sha256::digest(f_b()).to_vec());
    assert_eq!(sha(&out.join("Bin/c.txt")), Sha256::digest(f_b()).to_vec());
}

#[test]
fn partial_and_contained_overlaps_refused() {
    // f_b spans 7..40967: f_c inside it, then running past its end.
    for off in [8u32, 40_963] {
        let mut c = cab("three.cab");
        let cc = cffile(&c, 2);
        c[cc + 4..cc + 8].copy_from_slice(&off.to_le_bytes());
        let (d, r) = run(&with_cab(&mut tree_spec(), c), None);
        match r {
            Err(ExtractError::Archive(m)) => assert!(m.contains("sobrepostos"), "{off}: {m}"),
            other => panic!("{off}: {other:?}"),
        }
        assert_eq!(list(d.path()), ["x.msi"], "{off}");
    }
}

/// One uncompressed folder holding `data`, with `n` entries all pointed at `span`
/// (offset, size); each entry is a `File` row under `App`.
fn shared_span_msi(data: &[u8], n: usize, span: (u32, u32)) -> Vec<u8> {
    let mut b = cab::CabinetBuilder::new();
    let folder = b.add_folder(cab::CompressionType::None);
    for i in 0..n {
        folder.add_file(format!("k{i:04}"));
    }
    let mut w = b.build(Cursor::new(Vec::new())).unwrap();
    let mut first = true;
    while let Some(mut f) = w.next_file().unwrap() {
        if first {
            f.write_all(data).unwrap();
            first = false;
        }
    }
    let mut c = w.finish().unwrap().into_inner();
    for i in 0..n {
        let at = cffile(&c, i);
        c[at..at + 4].copy_from_slice(&span.1.to_le_bytes());
        c[at + 4..at + 8].copy_from_slice(&span.0.to_le_bytes());
    }
    let files = (0..n)
        .map(|i| {
            let key: &'static str = Box::leak(format!("k{i:04}").into_boxed_str());
            let name: &'static str = Box::leak(format!("f{i:04}.bin").into_boxed_str());
            file(key, "C_D", name, i as i32 + 1)
        })
        .collect();
    let mut s = spec(vec![("D", Some("TARGETDIR"), "App")], files);
    s.media = vec![(1, 10_000, "#cab1")];
    with_cab(&mut s, c)
}

const MIB: u32 = 1 << 20;

fn limits_64_mib() -> Limits {
    Limits {
        ratio: 1,
        floor: 64 << 20,
        ceiling: 64 << 20,
        ..DEFAULT_LIMITS
    }
}

#[test]
fn many_entries_on_one_span_hit_the_budget() {
    let data = pseudo_random(MIB as usize);
    // Control: two entries on the span extract, so `App` would exist.
    let (d, r) = run_with(&shared_span_msi(&data, 2, (0, MIB)), None, &limits_64_mib());
    r.unwrap();
    let app = d.path().join("out/App");
    assert_eq!(list(&app), ["f0000.bin", "f0001.bin"]);
    assert_eq!(std::fs::read(app.join("f0001.bin")).unwrap(), data);
    let (d, r) = run_with(
        &shared_span_msi(&data, 1000, (0, MIB)),
        None,
        &limits_64_mib(),
    );
    assert!(
        matches!(r, Err(ExtractError::TooLarge { limit }) if limit == 64 << 20),
        "{r:?}"
    );
    assert!(!d.path().join("out/App").exists());
    assert_eq!(list(d.path()), ["x.msi"]);
}

#[test]
fn rereading_a_span_behind_the_reader_is_charged() {
    // 16 bytes across the boundary of the last two blocks: each entry after the first
    // seeks back into an earlier block and decodes the folder again from the start.
    let data = pseudo_random(MIB as usize);
    let span = (MIB - 0x8000 - 8, 16);
    let (d, r) = run_with(&shared_span_msi(&data, 2, span), None, &limits_64_mib());
    r.unwrap();
    let at = span.0 as usize;
    assert_eq!(
        std::fs::read(d.path().join("out/App/f0001.bin")).unwrap(),
        &data[at..at + 16]
    );
    let (d, r) = run_with(&shared_span_msi(&data, 1000, span), None, &limits_64_mib());
    assert!(
        matches!(r, Err(ExtractError::TooLarge { limit }) if limit == 64 << 20),
        "{r:?}"
    );
    assert!(!d.path().join("out/App").exists());
    assert_eq!(list(d.path()), ["x.msi"]);
}

#[test]
fn file_past_folder_data_refused() {
    // one.cab has one block: at most 32 KiB of data.
    let mut c = cab("one.cab");
    let f = cffile(&c, 0);
    c[f + 4..f + 8].copy_from_slice(&40_000u32.to_le_bytes());
    let (d, r) = run(&with_cab(&mut one_file_spec(), c), None);
    match r {
        Err(ExtractError::Archive(m)) => assert!(m.contains("além dos dados"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(list(d.path()), ["x.msi"]);
}

#[test]
fn msi_row_limit_needs_installer() {
    // `msi` reads at most 65536 rows per table; the Windows Installer reads more (rust's
    // File table). Here `Media`, with an int32 key so no string is needed per row.
    let mut p = Package::open(Cursor::new(build(&tree_spec()))).unwrap();
    p.drop_table("Media").unwrap();
    p.create_table(
        "Media",
        vec![
            Column::build("DiskId").primary_key().int32(),
            Column::build("LastSequence").int32(),
            Column::build("Cabinet").nullable().string(255),
        ],
    )
    .unwrap();
    let mut rows = Insert::into("Media").row(vec![1.into(), 100.into(), "#cab1".into()]);
    for i in 2..=65_537 {
        rows = rows.row(vec![i.into(), 0.into(), Value::Null]);
    }
    p.insert_rows(rows).unwrap();
    let (d, r) = run(&p.into_inner().unwrap().into_inner(), None);
    let e = r.unwrap_err();
    match &e {
        ExtractError::Unsupported(m) => {
            assert_eq!(m, "MSI com mais de 65536 linhas numa tabela");
        }
        other => panic!("{other:?}"),
    }
    assert!(needs_installer(&d.path().join("x.msi"), "x.msi", &e));
    // Any other `msi` error stays an integrity error: a cut package.
    let whole = build(&tree_spec());
    let (d, r) = run(&whole[..whole.len() / 2], None);
    let e = r.unwrap_err();
    assert!(matches!(e, ExtractError::Archive(_)), "{e:?}");
    assert!(!needs_installer(&d.path().join("x.msi"), "x.msi", &e));
}
