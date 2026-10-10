#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::io::Write;
use wingetrs::install::extract::{ExtractError, extract};

fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        for (n, b) in entries {
            z.start_file(*n, o).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn targz_with(entries: &[(&str, &[u8])], symlink: Option<(&str, &str)>) -> Vec<u8> {
    let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut b = tar::Builder::new(enc);
    for (n, d) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_size(d.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, n, *d).unwrap();
    }
    if let Some((n, target)) = symlink {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_size(0);
        h.set_mode(0o777);
        h.set_cksum();
        b.append_link(&mut h, n, target).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap()
}

#[test]
fn extract_dir_becomes_root() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(
        &f,
        zip_with(&[("pkg-1.0/rg.exe", b"x"), ("pkg-1.0/doc/README", b"r")]),
    )
    .unwrap();
    let dest = d.path().join("out");
    extract(&f, "a.zip", Some("pkg-1.0"), None, &dest).unwrap();
    assert_eq!(std::fs::read(dest.join("rg.exe")).unwrap(), b"x");
    assert!(dest.join("doc/README").exists());
    assert!(!dest.join("pkg-1.0").exists());
}

#[test]
fn traversal_and_absolute_entries_are_refused_and_nothing_escapes() {
    for bad in [
        "../evil.txt",
        "a/../../evil.txt",
        "C:/evil.txt",
        "/abs.txt",
        "\\\\?\\C:\\evil.txt",
    ] {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.zip");
        std::fs::write(&f, zip_with(&[("ok.txt", b"1"), (bad, b"2")])).unwrap();
        let dest = d.path().join("inner").join("out");
        let r = extract(&f, "a.zip", None, None, &dest);
        assert!(
            matches!(r, Err(ExtractError::UnsafePath(_))),
            "{bad}: {r:?}"
        );
        assert!(
            !d.path().join("evil.txt").exists()
                && !d.path().join("inner").join("evil.txt").exists()
        );
        assert!(!dest.exists(), "{bad}: partial output must be removed");
    }
}

#[test]
fn plain_file_is_copied_with_its_name_and_extract_to() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("blob");
    std::fs::write(&f, b"MZ").unwrap();
    let dest = d.path().join("out");
    extract(&f, "agent-browser.exe", None, Some("bin"), &dest).unwrap();
    assert_eq!(
        std::fs::read(dest.join("bin/agent-browser.exe")).unwrap(),
        b"MZ"
    );
}

#[test]
fn missing_extract_dir_is_reported() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(&f, zip_with(&[("x/rg.exe", b"x")])).unwrap();
    assert!(matches!(
        extract(&f, "a.zip", Some("nope"), None, &d.path().join("o")),
        Err(ExtractError::MissingExtractDir(_))
    ));
}

#[test]
fn tar_gz_symlink_is_refused_and_nothing_remains() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.tar.gz");
    std::fs::write(
        &f,
        targz_with(&[("ok.txt", b"1")], Some(("link", "../../etc"))),
    )
    .unwrap();
    let dest = d.path().join("inner").join("out");
    let r = extract(&f, "a.tar.gz", None, None, &dest);
    assert!(matches!(r, Err(ExtractError::UnsafePath(_))), "{r:?}");
    assert!(!dest.exists());
    assert_eq!(
        std::fs::read_dir(dest.parent().unwrap()).unwrap().count(),
        0,
        "temp dir must be removed"
    );
}

#[test]
fn tar_gz_extract_dir_accepts_backslash() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.tgz");
    std::fs::write(
        &f,
        targz_with(&[("a/b/rg.exe", b"x"), ("other/z", b"z")], None),
    )
    .unwrap();
    let dest = d.path().join("out");
    extract(&f, "a.tgz", Some("a\\b"), None, &dest).unwrap();
    assert_eq!(std::fs::read(dest.join("rg.exe")).unwrap(), b"x");
    assert!(!dest.join("other").exists());
}

#[test]
fn second_source_merges_into_existing_dest() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(&f, zip_with(&[("one.txt", b"1")])).unwrap();
    let g = d.path().join("blob");
    std::fs::write(&g, b"2").unwrap();
    let dest = d.path().join("out");
    extract(&f, "a.zip", None, None, &dest).unwrap();
    extract(&g, "two.txt", None, None, &dest).unwrap();
    assert!(dest.join("one.txt").exists() && dest.join("two.txt").exists());
}
