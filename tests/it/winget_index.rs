#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::cache::DataDir;
use wingetrs::http::Http;
use wingetrs::sources::winget::index::{IndexError, read_index, validate_msix};
use wingetrs::sources::winget::{WingetFetch, fetch_index};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MSIX: &[u8] = include_bytes!("../fixtures/winget/source2-mini.msix");
const CORRUPT: &[u8] = include_bytes!("../fixtures/winget/source2-mini-corrupt.msix");
const MISSING_BLOCK: &[u8] = include_bytes!("../fixtures/winget/source2-mini-missing-block.msix");
const BAD_FILEHASH: &[u8] = include_bytes!("../fixtures/winget/source2-mini-bad-filehash.msix");
const DB: &[u8] = include_bytes!("../fixtures/winget/index-mini.db");

#[test]
fn valid_msix_yields_exact_index_bytes() {
    assert_eq!(validate_msix(MSIX).unwrap(), DB);
}

#[test]
fn corrupt_block_is_rejected() {
    match validate_msix(CORRUPT) {
        Err(IndexError::Integrity { block }) => assert_eq!(block, (DB.len() / 2) / 65536),
        other => panic!("expected Integrity, got {other:?}"),
    }
}

#[test]
fn missing_block_is_rejected() {
    assert!(matches!(
        validate_msix(MISSING_BLOCK),
        Err(IndexError::BlockMap(_))
    ));
}

#[test]
fn bad_file_hash_is_rejected() {
    assert!(matches!(
        validate_msix(BAD_FILEHASH),
        Err(IndexError::FileHash)
    ));
}

#[test]
fn not_a_zip_is_an_error() {
    assert!(matches!(validate_msix(b"hello"), Err(IndexError::Zip(_))));
}

#[test]
fn reads_packages_and_all_maps() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("i.db");
    std::fs::write(&p, DB).unwrap();
    let imp = read_index(&p).unwrap();
    assert_eq!(imp.packages.len(), 20);
    assert_eq!(imp.maps.productcodes.len(), 6025);
    assert_eq!(imp.maps.upgradecodes.len(), 13);
    assert_eq!(imp.maps.pfns.len(), 5);
    assert_eq!(imp.maps.norm_names.len(), 29);
    assert_eq!(imp.maps.norm_publishers.len(), 22);
    let tag_total: usize = imp.packages.iter().map(|p| p.tags.len()).sum();
    let cmd_total: usize = imp.packages.iter().map(|p| p.commands.len()).sum();
    assert_eq!((tag_total, cmd_total), (289, 12));
    let py = imp
        .packages
        .iter()
        .find(|p| p.id == "Python.Python.3.13")
        .unwrap();
    assert_eq!(py.version, "3.13.15");
    assert_eq!(py.arp_max_version.as_deref(), Some("3.13.15150.0"));
    let pt = imp
        .packages
        .iter()
        .find(|p| p.id == "Microsoft.PowerToys")
        .unwrap();
    assert_eq!(
        hex::encode(&pt.winget_hash.as_ref().unwrap()[..4]),
        "d193b67a"
    );
    let mut seven: Vec<&str> = imp
        .maps
        .norm_names
        .iter()
        .filter(|(_, id)| id == "7zip.7zip")
        .map(|(k, _)| k.as_str())
        .collect();
    seven.sort();
    assert_eq!(seven, ["7zip", "7zip(X64)"]);
}

#[test]
fn unsupported_major_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("i.db");
    std::fs::write(&p, DB).unwrap();
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute(
        "update metadata set value='3' where name='majorVersion'",
        [],
    )
    .unwrap();
    drop(c);
    assert!(matches!(read_index(&p), Err(IndexError::UnsupportedVersion(v)) if v == "3"));
}

#[tokio::test]
async fn fetch_index_end_to_end_and_304() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/source2.msix"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(MSIX.to_vec())
                .insert_header("ETag", "\"e1\""),
        )
        .up_to_n_times(1)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/source2.msix"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let url = format!("{}/source2.msix", s.uri());
    let http = Http::new().unwrap();
    let entry = match fetch_index(&http, &data, &url, None).await.unwrap() {
        WingetFetch::Modified { import, entry } => {
            assert_eq!(import.packages.len(), 20);
            entry
        }
        WingetFetch::NotModified => panic!("expected Modified"),
    };
    assert!(matches!(
        fetch_index(&http, &data, &url, Some(&entry)).await.unwrap(),
        WingetFetch::NotModified
    ));
    let leftovers = std::fs::read_dir(data.tmp_dir()).unwrap().count();
    assert_eq!(leftovers, 0, "temporary index.db must be removed");
}

#[tokio::test]
async fn fetch_index_corrupt_download_is_integrity_error() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(CORRUPT.to_vec()))
        .mount(&s)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let data = DataDir::new(dir.path());
    data.ensure().unwrap();
    let r = fetch_index(&Http::new().unwrap(), &data, &s.uri(), None).await;
    assert!(r.is_err());
}

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};

/// Pre-Task-56 validate_msix, literal (serial: inflate, block hashes, file hash).
fn validate_msix_serial(msix: &[u8]) -> Result<Vec<u8>, IndexError> {
    fn read_entry<R: std::io::Read + std::io::Seek>(
        zip: &mut zip::ZipArchive<R>,
        name: &'static str,
    ) -> Result<Vec<u8>, IndexError> {
        let mut f = zip.by_name(name).map_err(|e| match e {
            zip::result::ZipError::FileNotFound => IndexError::MissingEntry(name),
            other => IndexError::Zip(other.to_string()),
        })?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf).map_err(|e| IndexError::Zip(e.to_string()))?;
        Ok(buf)
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(msix))
        .map_err(|e| IndexError::Zip(e.to_string()))?;
    let db = read_entry(&mut zip, "Public/index.db")?;
    let map = read_entry(&mut zip, "AppxBlockMap.xml")?;
    let xml = std::str::from_utf8(&map).map_err(|e| IndexError::BlockMap(e.to_string()))?;
    let doc = roxmltree::Document::parse(xml).map_err(|e| IndexError::BlockMap(e.to_string()))?;
    let file = doc
        .descendants()
        .find(|n| n.has_tag_name("File") && n.attribute("Name") == Some("Public\\index.db"))
        .ok_or_else(|| IndexError::BlockMap("entrada Public\\index.db ausente".into()))?;
    let hashes: Vec<&str> = file
        .children()
        .filter(|n| n.has_tag_name("Block"))
        .filter_map(|n| n.attribute("Hash"))
        .collect();
    if hashes.len() != db.len().div_ceil(65536) {
        return Err(IndexError::BlockMap(format!(
            "esperados {} blocos, encontrados {}",
            db.len().div_ceil(65536),
            hashes.len()
        )));
    }
    for (i, (chunk, expected)) in db.chunks(65536).zip(&hashes).enumerate() {
        if STANDARD.encode(Sha256::digest(chunk)) != *expected {
            return Err(IndexError::Integrity { block: i });
        }
    }
    let file_hash = file
        .children()
        .find(|n| n.has_tag_name("FileHash"))
        .and_then(|n| n.attribute("Hash"));
    if let Some(expected) = file_hash
        && STANDARD.encode(Sha256::digest(&db)) != expected
    {
        return Err(IndexError::FileHash);
    }
    Ok(db)
}

/// (name, bytes, stored) entries in order; stored = no compression (lets a test corrupt them).
fn zip_of(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        for (name, bytes, stored) in files {
            let o = zip::write::SimpleFileOptions::default();
            let o = if *stored {
                o.compression_method(zip::CompressionMethod::Stored)
            } else {
                o
            };
            z.start_file(*name, o).unwrap();
            z.write_all(bytes).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn fixture_blockmap() -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(MSIX)).unwrap();
    let mut v = Vec::new();
    std::io::Read::read_to_end(&mut z.by_name("AppxBlockMap.xml").unwrap(), &mut v).unwrap();
    v
}

/// 41 blocks (> the channel capacity of 32) with a hand-written blockmap.
fn synthetic(bad_block: Option<usize>, bad_file_hash: bool) -> Vec<u8> {
    let data: Vec<u8> = (0..40 * 65536 + 123)
        .map(|i: usize| (i % 251) as u8)
        .collect();
    let mut xml = String::from(r#"<BlockMap><File Name="Public\index.db">"#);
    for c in data.chunks(65536) {
        xml.push_str(&format!(
            r#"<Block Hash="{}"/>"#,
            STANDARD.encode(Sha256::digest(c))
        ));
    }
    let fh = if bad_file_hash {
        Sha256::digest(b"x")
    } else {
        Sha256::digest(&data)
    };
    xml.push_str(&format!(
        r#"<FileHash Hash="{}"/></File></BlockMap>"#,
        STANDARD.encode(fh)
    ));
    let mut stored = data;
    if let Some(b) = bad_block {
        stored[b * 65536 + 7] ^= 0xFF;
    }
    zip_of(&[
        ("Public/index.db", &stored, false),
        ("AppxBlockMap.xml", xml.as_bytes(), false),
    ])
}

#[test]
fn pipelined_validate_msix_equals_serial_reference() {
    let bm = fixture_blockmap();
    let flip = |at: &[usize]| {
        let mut d = DB.to_vec();
        for &i in at {
            d[i] ^= 0xFF;
        }
        d
    };
    let last_block = flip(&[DB.len() - 1]);
    let two_bad = flip(&[65536 + 5, 3 * 65536 + 5]);
    let mut bad_and_long = flip(&[5]);
    bad_and_long.extend_from_slice(&[0u8; 65536]);
    // Stored index.db with one byte flipped inside the zip: the CRC check fails on read.
    let crc_broken = |map: &[u8]| {
        let mut z = zip_of(&[
            ("Public/index.db", DB, true),
            ("AppxBlockMap.xml", map, false),
        ]);
        let at = z.windows(16).position(|w| w == &DB[..16]).unwrap();
        z[at + 70_000] ^= 0xFF;
        z
    };
    let cases: Vec<Vec<u8>> = vec![
        MSIX.to_vec(),
        CORRUPT.to_vec(),
        MISSING_BLOCK.to_vec(),
        BAD_FILEHASH.to_vec(),
        b"hello".to_vec(),
        zip_of(&[("AppxManifest.xml", b"<x/>", false)]),
        zip_of(&[("Public/index.db", DB, false)]),
        zip_of(&[
            ("Public/index.db", DB, false),
            ("AppxBlockMap.xml", b"<not xml", false),
        ]),
        zip_of(&[
            ("Public/index.db", DB, false),
            ("AppxBlockMap.xml", b"<BlockMap/>", false),
        ]),
        zip_of(&[
            ("Public/index.db", &last_block, false),
            ("AppxBlockMap.xml", &bm, false),
        ]),
        zip_of(&[
            ("Public/index.db", &two_bad, false),
            ("AppxBlockMap.xml", &bm, false),
        ]),
        zip_of(&[
            ("Public/index.db", &bad_and_long, false),
            ("AppxBlockMap.xml", &bm, false),
        ]),
        crc_broken(&bm),
        crc_broken(b"<not xml"),
        synthetic(None, false),
        synthetic(Some(37), false),
        synthetic(None, true),
        // Valid blockmap but no index.db: MissingEntry must not become a Zip error.
        zip_of(&[("AppxBlockMap.xml", &bm, false)]),
    ];
    let outcome = |r: Result<Vec<u8>, IndexError>| r.map_err(|e| format!("{e:?}"));
    let mut seen = Vec::new();
    for (i, msix) in cases.iter().enumerate() {
        let want = outcome(validate_msix_serial(msix));
        let got = outcome(validate_msix(msix));
        assert!(
            got == want,
            "case {i}: got {:?}, want {:?}",
            got.as_ref().err(),
            want.as_ref().err()
        );
        seen.push(want.err().unwrap_or_else(|| "Ok".into()));
    }
    // Guard against degenerate inputs: every rung of the precedence is exercised.
    for kind in [
        "Ok",
        "Zip(",
        "MissingEntry(\"Public",
        "MissingEntry(\"Appx",
        "BlockMap(",
        "Integrity",
        "FileHash",
    ] {
        assert!(
            seen.iter().any(|s| s.starts_with(kind)),
            "no case yields {kind}: {seen:?}"
        );
    }
    // Two defects at once: the serial order decides.
    assert!(
        seen[11].starts_with("BlockMap("),
        "count before Integrity: {}",
        seen[11]
    );
    assert!(
        seen[12].starts_with("Zip("),
        "read error before Integrity: {}",
        seen[12]
    );
    assert!(
        seen[13].starts_with("Zip("),
        "read error before BlockMap: {}",
        seen[13]
    );
    assert_eq!(seen[10], "Integrity { block: 1 }");
    assert_eq!(seen[15], "Integrity { block: 37 }");
}
