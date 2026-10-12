#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::catalog::index_file::{self, IndexView, MAGIC};
use wingetrs::catalog::search::{SearchIndex, search_on_disk};
use wingetrs::catalog::{Catalog, NewPackage, WingetMaps};
use wingetrs::core::SourceKind;

fn catalog(n: usize) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut c = Catalog::open(&path).unwrap();
    let names = [
        "PowerToys",
        "Power Automate",
        "powertoys",
        "Zeal",
        "360安全卫士",
        "Visual Studio Code",
        "Café Läuft",
        "7-Zip",
    ];
    let mk = |i: usize| NewPackage {
        id: format!("Pub{}.App{i}", i % 37),
        name: format!("{} {i}", names[i % names.len()]),
        moniker: i.is_multiple_of(5).then(|| format!("m{i}")),
        version: format!("1.{i}"),
        publisher: i.is_multiple_of(3).then(|| "Pub".to_string()),
        description: i.is_multiple_of(4).then(|| format!("desc {i} ✓")),
        tags: vec![format!("tag{}", i % 11), "power".into()],
        commands: vec![format!("cmd{}", i % 13)],
        ..Default::default()
    };
    let w: Vec<NewPackage> = (0..n).map(mk).collect();
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v",
        1,
        &w,
        &WingetMaps::default(),
    )
    .unwrap();
    let s: Vec<NewPackage> = (n..n + n / 3).map(mk).collect();
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "e",
        1,
        &s,
        &WingetMaps::default(),
    )
    .unwrap();
    drop(c);
    (dir, path)
}

#[test]
fn round_trip_every_entry() {
    let (d, db) = catalog(500);
    let idx = d.path().join("search.idx");
    let cat = Catalog::open(&db).unwrap();
    assert_eq!(index_file::write(&cat, &idx).unwrap(), 666);
    let bytes = std::fs::read(&idx).unwrap();
    let v = IndexView::parse(&bytes).unwrap();
    let mem = cat.load_entries().unwrap();
    assert_eq!(v.len(), mem.len());
    for (i, e) in mem.iter().enumerate() {
        assert_eq!(&v.entry(i).unwrap(), e, "record {i}");
    }
    assert!(!idx.with_extension("idx.tmp").exists());
}

#[test]
fn index_search_matches_in_memory_and_sqlite_exactly() {
    let (d, db) = catalog(6000);
    let idx = d.path().join("search.idx");
    let cat = Catalog::open(&db).unwrap();
    index_file::write(&cat, &idx).unwrap();
    let mem = SearchIndex::new(cat.load_entries().unwrap());
    for (q, f, limit) in [
        ("powertoys", None, 20),
        ("power", None, 50),
        ("power", Some(SourceKind::Scoop), 50),
        ("安全", None, 10),
        ("cafe", None, 10),
        ("m10", None, 5),
        ("tag3", None, 30),
        ("", None, 25),
        ("zzzz-no-match", None, 10),
    ] {
        let a: Vec<(String, u32, Vec<u32>)> = mem
            .search(q, f, limit)
            .into_iter()
            .map(|h| (mem.entries()[h.index].id.clone(), h.score, h.name_indices))
            .collect();
        let b: Vec<(String, u32, Vec<u32>)> = index_file::search(&idx, q, f, limit)
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|x| (x.entry.id, x.score, x.name_indices))
            .collect();
        let c: Vec<(String, u32, Vec<u32>)> = search_on_disk(&db, q, f, limit)
            .unwrap()
            .into_iter()
            .map(|x| (x.entry.id, x.score, x.name_indices))
            .collect();
        assert_eq!(b, a, "index vs memory: {q:?} {f:?}");
        assert_eq!(c, a, "sqlite vs memory: {q:?} {f:?}");
    }
}

#[test]
fn missing_file_is_none_and_damaged_files_are_errors_never_panics() {
    let (d, db) = catalog(300);
    let idx = d.path().join("search.idx");
    assert!(index_file::search(&idx, "x", None, 10).unwrap().is_none());
    index_file::write(&Catalog::open(&db).unwrap(), &idx).unwrap();
    let good = std::fs::read(&idx).unwrap();
    // truncations
    for cut in [
        0,
        7,
        8,
        23,
        24,
        good.len() / 2,
        good.len() - 8,
        good.len() - 1,
    ] {
        assert!(IndexView::parse(&good[..cut]).is_err(), "cut {cut}");
    }
    // right size, damaged trailer
    let mut b = good.clone();
    let n = b.len();
    b[n - 1] ^= 0xFF;
    assert!(IndexView::parse(&b).is_err());
    // wrong version byte
    let mut bad = good.clone();
    bad[7] = 2;
    assert!(IndexView::parse(&bad).is_err());
    // count claims more entries than exist
    let mut bad = good.clone();
    bad[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(IndexView::parse(&bad).is_err());
    // deterministic byte flips anywhere: parse/search may fail, but must never panic
    let mut x: u32 = 0x9E37_79B9;
    for _ in 0..500 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let mut b = good.clone();
        let pos = (x as usize) % b.len();
        b[pos] ^= (x >> 24) as u8 | 1;
        if let Ok(v) = IndexView::parse(&b) {
            for i in 0..v.len() {
                let _ = v.view(i);
                let _ = v.entry(i);
            }
        }
    }
    assert_eq!(&good[..8], MAGIC);
}

#[test]
fn rewriting_while_a_reader_loops_never_fails_or_tears() {
    let (d, db) = catalog(300);
    let idx = d.path().join("search.idx");
    let cat = Catalog::open(&db).unwrap();
    index_file::write(&cat, &idx).unwrap();
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    let stop = AtomicBool::new(false);
    let reads = AtomicU32::new(0);
    std::thread::scope(|s| {
        let reader = s.spawn(|| {
            while !stop.load(Ordering::Relaxed) {
                // Ok(None) ("not found" for an instant during the rename, measured 1 in 200
                // stressed runs) is fine: the caller falls back to the catalog. Err or a torn
                // file is not.
                if let Some(r) = index_file::search(&idx, "power", None, 5).unwrap() {
                    assert!(!r.is_empty());
                    reads.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        // Under load the writer could finish before the reader's first read; keep rewriting
        // until reads and writes have overlapped for a while.
        let mut writes = 0;
        while writes < 10 || (reads.load(Ordering::Relaxed) < 5 && !reader.is_finished()) {
            assert_eq!(index_file::write(&cat, &idx).unwrap(), 400);
            writes += 1;
        }
        stop.store(true, Ordering::Relaxed);
        if let Err(p) = reader.join() {
            std::panic::resume_unwind(p);
        }
        assert!(reads.load(Ordering::Relaxed) >= 5);
    });
}
/// Frozen copy of the pre-Task-54 encoder (entries -> file bytes): the file-format pin.
fn reference_bytes(entries: &[wingetrs::core::PackageEntry]) -> Vec<u8> {
    fn put(out: &mut Vec<u8>, s: Option<&str>) {
        match s {
            Some(s) => {
                out.extend_from_slice(&(s.len() as u32).to_le_bytes());
                out.extend_from_slice(s.as_bytes());
            }
            None => out.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()),
        }
    }
    let mut offsets = Vec::new();
    let mut body = Vec::new();
    for e in entries {
        offsets.extend_from_slice(&(body.len() as u32).to_le_bytes());
        body.push(match e.source {
            SourceKind::Winget => 0,
            SourceKind::Scoop => 1,
        });
        put(&mut body, Some(&e.source_name));
        put(&mut body, Some(&e.id));
        put(&mut body, Some(&e.name));
        put(&mut body, e.moniker.as_deref());
        put(&mut body, Some(&e.version));
        put(&mut body, e.publisher.as_deref());
        put(&mut body, e.description.as_deref());
        put(&mut body, Some(&e.tags.join(" ")));
        put(&mut body, Some(&e.commands.join(" ")));
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    out.extend_from_slice(&offsets);
    out.extend_from_slice(&body);
    out.extend_from_slice(MAGIC);
    out
}

#[test]
fn streamed_bytes_equal_the_frozen_entry_encoder_including_odd_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.db");
    let mut c = Catalog::open(&path).unwrap();
    let mk = |i: usize| NewPackage {
        id: format!("P.{i}"),
        name: format!("Name {i} ✓"),
        moniker: i.is_multiple_of(2).then(|| format!("m{i}")),
        version: format!("1.{i}"),
        publisher: i.is_multiple_of(3).then(|| "Pub".into()),
        description: i.is_multiple_of(5).then(String::new),
        // join(" ") of these stores doubled/leading/tab whitespace in the column;
        // load_entries splits it, so the stored text differs from the file text.
        tags: vec![
            " lead".into(),
            "a\tb".into(),
            String::new(),
            format!("t{i} "),
        ],
        commands: if i.is_multiple_of(4) {
            vec![]
        } else {
            vec!["c  d".into()]
        },
        ..Default::default()
    };
    let w: Vec<NewPackage> = (0..3000).map(mk).collect();
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v",
        1,
        &w,
        &WingetMaps::default(),
    )
    .unwrap();
    let s: Vec<NewPackage> = (3000..3400).map(mk).collect();
    c.replace_source(
        "scoop:main",
        SourceKind::Scoop,
        "e",
        1,
        &s,
        &WingetMaps::default(),
    )
    .unwrap();
    // Re-import winget so rowids are not contiguous by source (winget lands after scoop).
    c.replace_source(
        "winget",
        SourceKind::Winget,
        "v2",
        2,
        &w,
        &WingetMaps::default(),
    )
    .unwrap();
    let (bytes, n) = index_file::encode_catalog(&c).unwrap();
    assert_eq!(n, 3400);
    // Not assert_eq!: a 3400-record diff would flood the log.
    assert!(
        bytes == reference_bytes(&c.load_entries().unwrap()),
        "streamed bytes differ"
    );
}

#[test]
fn write_writes_the_reference_bytes() {
    let (d, db) = catalog(500);
    let idx = d.path().join("search.idx");
    let cat = Catalog::open(&db).unwrap();
    assert_eq!(index_file::write(&cat, &idx).unwrap(), 666);
    assert!(
        std::fs::read(&idx).unwrap() == reference_bytes(&cat.load_entries().unwrap()),
        "written bytes differ"
    );
    assert!(!idx.with_extension("idx.tmp").exists());
}
