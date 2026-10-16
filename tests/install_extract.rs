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

fn tarxz_with(entries: &[(&str, &[u8])], symlink: Option<(&str, &str)>) -> Vec<u8> {
    let enc = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
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
fn extracts_tar_xz() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.tar.xz");
    std::fs::write(
        &f,
        tarxz_with(&[("pkg/rg.exe", b"x"), ("pkg/doc/README", b"r")], None),
    )
    .unwrap();
    let dest = d.path().join("out");
    extract(&f, "a.tar.xz", Some("pkg"), None, &dest).unwrap();
    assert_eq!(std::fs::read(dest.join("rg.exe")).unwrap(), b"x");
    assert_eq!(std::fs::read(dest.join("doc/README")).unwrap(), b"r");
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for b in data {
        c ^= u32::from(*b);
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

#[test]
fn tar_xz_with_a_4_gib_dictionary_is_refused_before_allocating() {
    let mut xz = tarxz_with(&[("a.txt", b"1")], None);
    // Stream header is 12 bytes; the block header starts with its size byte.
    let hdr_len = (usize::from(xz[12]) + 1) * 4;
    assert_eq!(xz[14], 0x21, "LZMA2 filter id");
    xz[16] = 40; // dictionary 4 GiB
    let crc = crc32(&xz[12..12 + hdr_len - 4]);
    xz[12 + hdr_len - 4..12 + hdr_len].copy_from_slice(&crc.to_le_bytes());
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.tar.xz");
    std::fs::write(&f, xz).unwrap();
    let dest = d.path().join("out");
    let r = extract(&f, "a.tar.xz", None, None, &dest);
    let Err(ExtractError::Archive(m)) = &r else {
        panic!("{r:?}")
    };
    assert!(m.contains("1 GiB"), "{m}");
    assert!(!dest.exists());
}

#[test]
fn tar_xz_symlink_refused() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.tar.xz");
    std::fs::write(
        &f,
        tarxz_with(&[("ok.txt", b"1")], Some(("link", "../../etc"))),
    )
    .unwrap();
    let dest = d.path().join("inner").join("out");
    let r = extract(&f, "a.tar.xz", None, None, &dest);
    assert!(matches!(r, Err(ExtractError::UnsafePath(_))), "{r:?}");
    assert!(!dest.exists());
    assert_eq!(
        std::fs::read_dir(dest.parent().unwrap()).unwrap().count(),
        0
    );
}

#[test]
fn extracts_nupkg() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.nupkg");
    std::fs::write(&f, zip_with(&[("tools/x.exe", b"x"), ("a.nuspec", b"n")])).unwrap();
    let dest = d.path().join("out");
    extract(&f, "a.nupkg", None, None, &dest).unwrap();
    assert_eq!(std::fs::read(dest.join("tools/x.exe")).unwrap(), b"x");
    assert!(dest.join("a.nuspec").exists());
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

fn entries_in(d: &std::path::Path) -> usize {
    std::fs::read_dir(d).unwrap().count()
}

#[test]
fn manifest_extract_to_with_traversal_is_refused() {
    for bad in ["../x", "..\\..\\x", "C:\\x"] {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let f = root.join("a.zip");
        std::fs::write(&f, zip_with(&[("ok.txt", b"1")])).unwrap();
        let dest = root.join("out");
        let r = extract(&f, "a.zip", None, Some(bad), &dest);
        assert!(
            matches!(r, Err(ExtractError::UnsafePath(_))),
            "{bad}: {r:?}"
        );
        assert_eq!(entries_in(&root), 1, "{bad}: only a.zip may exist in root");
        assert_eq!(entries_in(d.path()), 1, "{bad}");
    }
}

#[test]
fn manifest_extract_dir_with_traversal_is_unsafe_not_missing() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(&f, zip_with(&[("x/rg.exe", b"x")])).unwrap();
    let r = extract(&f, "a.zip", Some("../x"), None, &d.path().join("o"));
    assert!(matches!(r, Err(ExtractError::UnsafePath(_))), "{r:?}");
}

#[test]
fn windows_reserved_and_trailing_dot_space_names_are_refused() {
    for bad in ["aux.txt", "x/CON", "foo.", "bar ", "dir/Lpt1.log", "nul"] {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.zip");
        std::fs::write(&f, zip_with(&[(bad, b"2")])).unwrap();
        let dest = d.path().join("out");
        let r = extract(&f, "a.zip", None, None, &dest);
        assert!(
            matches!(r, Err(ExtractError::UnsafePath(_))),
            "{bad}: {r:?}"
        );
        assert!(!dest.exists());
    }
}

#[test]
fn zip_symlink_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("ok.txt", o).unwrap();
        z.write_all(b"1").unwrap();
        z.add_symlink("link", "../../etc", o).unwrap();
        z.finish().unwrap();
    }
    std::fs::write(&f, buf.into_inner()).unwrap();
    let dest = d.path().join("out");
    let r = extract(&f, "a.zip", None, None, &dest);
    assert!(matches!(r, Err(ExtractError::UnsafePath(_))), "{r:?}");
    assert!(!dest.exists());
}

#[test]
fn tar_gz_hardlink_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.tar.gz");
    let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut b = tar::Builder::new(enc);
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Link);
    h.set_size(0);
    h.set_mode(0o644);
    h.set_cksum();
    b.append_link(&mut h, "hl", "ok.txt").unwrap();
    std::fs::write(&f, b.into_inner().unwrap().finish().unwrap()).unwrap();
    let dest = d.path().join("out");
    let r = extract(&f, "a.tar.gz", None, None, &dest);
    assert!(matches!(r, Err(ExtractError::UnsafePath(_))), "{r:?}");
    assert!(!dest.exists());
}

#[test]
fn many_sources_land_in_dest_with_a_single_move_and_failure_leaves_nothing() {
    use wingetrs::install::extract::{ExtractSource, extract_all};
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(&f, zip_with(&[("one.txt", b"1")])).unwrap();
    let g = d.path().join("blob");
    std::fs::write(&g, b"2").unwrap();
    let src = |file, file_name| ExtractSource {
        file,
        file_name,
        extract_dir: None,
        extract_to: None,
    };
    let dest = d.path().join("out");
    extract_all(&[src(&f, "a.zip"), src(&g, "two.txt")], &dest).unwrap();
    assert!(dest.join("one.txt").exists() && dest.join("two.txt").exists());
    let bad = d.path().join("bad.zip");
    std::fs::write(&bad, zip_with(&[("../e", b"x")])).unwrap();
    let dest2 = d.path().join("out2");
    let r = extract_all(&[src(&f, "a.zip"), src(&bad, "bad.zip")], &dest2);
    assert!(matches!(r, Err(ExtractError::UnsafePath(_))));
    assert!(!dest2.exists());
}

mod limits {
    use super::{targz_with, zip_with};
    use std::path::Path;
    use wingetrs::install::extract::{
        DEFAULT_LIMITS, ExtractError, ExtractSource, Limits, extract_all_with,
    };

    fn lim(ratio: u64, floor: u64, max_entries: u64) -> Limits {
        Limits {
            ratio,
            floor,
            ceiling: 1 << 40,
            max_entries,
        }
    }

    fn run(file: &Path, name: &str, dest: &Path, l: &Limits) -> Result<(), ExtractError> {
        extract_all_with(
            &[ExtractSource {
                file,
                file_name: name,
                extract_dir: None,
                extract_to: None,
            }],
            dest,
            l,
        )
    }

    fn noise(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    fn leftovers(parent: &Path) -> usize {
        std::fs::read_dir(parent)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".wingetrs-extract-")
            })
            .count()
    }

    #[test]
    fn zip_over_the_byte_budget_is_refused_before_writing() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.zip");
        let big = vec![0u8; 2 << 20];
        std::fs::write(&f, zip_with(&[("big.bin", &big)])).unwrap();
        let dest = d.path().join("out");
        let r = run(&f, "a.zip", &dest, &lim(100, 1 << 20, 10));
        assert!(
            matches!(r, Err(ExtractError::TooLarge { limit }) if limit == 1 << 20),
            "{r:?}"
        );
        assert!(!dest.exists());
        run(&f, "a.zip", &dest, &lim(100, 4 << 20, 10)).unwrap();
        assert_eq!(
            std::fs::metadata(dest.join("big.bin")).unwrap().len(),
            2 << 20
        );
    }

    #[test]
    fn ratio_raises_the_budget_above_the_floor() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.zip");
        let (a, b, c) = (noise(1 << 20, 1), noise(1 << 20, 2), noise(1 << 20, 3));
        std::fs::write(&f, zip_with(&[("a", &a), ("b", &b), ("c", &c)])).unwrap();
        assert!(std::fs::metadata(&f).unwrap().len() > 3 << 20);
        run(&f, "a.zip", &d.path().join("ok"), &lim(2, 1 << 20, 10)).unwrap();
        let dest = d.path().join("out");
        let r = run(&f, "a.zip", &dest, &lim(0, 1 << 20, 10));
        assert!(matches!(r, Err(ExtractError::TooLarge { .. })), "{r:?}");
        assert!(!dest.exists());
    }

    #[test]
    fn zip_with_too_many_entries_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.zip");
        let names: Vec<String> = (0..11).map(|i| format!("f{i}")).collect();
        let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
        std::fs::write(&f, zip_with(&entries)).unwrap();
        let dest = d.path().join("out");
        let r = run(&f, "a.zip", &dest, &lim(100, 1 << 20, 10));
        assert!(
            matches!(r, Err(ExtractError::TooManyEntries { limit: 10 })),
            "{r:?}"
        );
        assert!(!dest.exists());
        run(&f, "a.zip", &dest, &lim(100, 1 << 20, 11)).unwrap();
        assert!(dest.join("f10").exists());
    }

    #[test]
    fn tar_gz_over_the_byte_budget_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.tar.gz");
        let big = vec![0u8; 2 << 20];
        std::fs::write(&f, targz_with(&[("big.bin", &big)], None)).unwrap();
        let dest = d.path().join("out");
        let r = run(&f, "a.tar.gz", &dest, &lim(100, 1 << 20, 10));
        assert!(
            matches!(r, Err(ExtractError::TooLarge { limit }) if limit == 1 << 20),
            "{r:?}"
        );
        assert!(!dest.exists());
        run(&f, "a.tar.gz", &dest, &lim(100, 4 << 20, 10)).unwrap();
        assert_eq!(
            std::fs::metadata(dest.join("big.bin")).unwrap().len(),
            2 << 20
        );
    }

    #[test]
    fn tar_gz_with_too_many_entries_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.tar.gz");
        let names: Vec<String> = (0..11).map(|i| format!("f{i}")).collect();
        let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
        std::fs::write(&f, targz_with(&entries, None)).unwrap();
        let parent = d.path().join("apps");
        std::fs::create_dir(&parent).unwrap();
        let dest = parent.join("out");
        let r = run(&f, "a.tar.gz", &dest, &lim(100, 1 << 20, 10));
        assert!(
            matches!(r, Err(ExtractError::TooManyEntries { limit: 10 })),
            "{r:?}"
        );
        assert!(!dest.exists());
        assert_eq!(leftovers(&parent), 0);
        run(&f, "a.tar.gz", &dest, &lim(100, 1 << 20, 11)).unwrap();
        assert!(dest.join("f10").exists());
    }

    #[test]
    fn budget_is_for_the_whole_archive_not_per_entry() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.tar.gz");
        let part = vec![0u8; 600 << 10];
        std::fs::write(&f, targz_with(&[("a.bin", &part), ("b.bin", &part)], None)).unwrap();
        let dest = d.path().join("out");
        let r = run(&f, "a.tar.gz", &dest, &lim(100, 1 << 20, 10));
        assert!(matches!(r, Err(ExtractError::TooLarge { .. })), "{r:?}");
        assert!(!dest.exists());
        run(&f, "a.tar.gz", &dest, &lim(100, 2 << 20, 10)).unwrap();
        assert!(dest.join("b.bin").exists());
    }

    #[test]
    fn default_limits_are_the_measured_ones() {
        assert_eq!(DEFAULT_LIMITS.ratio, 100);
        assert_eq!(DEFAULT_LIMITS.floor, 1 << 30);
        assert_eq!(DEFAULT_LIMITS.ceiling, 32 << 30);
        assert_eq!(DEFAULT_LIMITS.max_entries, 200_000);
    }

    /// Rewrites the declared uncompressed size of every entry (local + central headers).
    fn lie_about_size(bytes: &mut [u8], size: u32) {
        let mut i = 0;
        while i + 4 <= bytes.len() {
            let off = match &bytes[i..i + 4] {
                b"PK\x03\x04" => Some(22),
                b"PK\x01\x02" => Some(24),
                _ => None,
            };
            if let Some(off) = off {
                bytes[i + off..i + off + 4].copy_from_slice(&size.to_le_bytes());
            }
            i += 1;
        }
    }

    #[test]
    fn lying_zip_header_does_not_bypass_the_budget() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.zip");
        let big = vec![0u8; 2 << 20];
        let mut bytes = zip_with(&[("big.bin", &big)]);
        lie_about_size(&mut bytes, 1024);
        std::fs::write(&f, bytes).unwrap();
        let dest = d.path().join("out");
        let r = run(&f, "a.zip", &dest, &lim(100, 1 << 20, 10));
        // Measured: zip 8.6 streams past the declared 1 KiB; only the byte counter stops it.
        assert!(
            matches!(r, Err(ExtractError::TooLarge { limit }) if limit == 1 << 20),
            "{r:?}"
        );
        assert!(!dest.exists());
    }

    #[test]
    fn zip_declared_size_over_budget_is_refused_even_if_content_is_small() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.zip");
        let small = vec![7u8; 1024];
        let mut bytes = zip_with(&[("small.bin", &small)]);
        lie_about_size(&mut bytes, 4 << 20);
        std::fs::write(&f, bytes).unwrap();
        let dest = d.path().join("out");
        let r = run(&f, "a.zip", &dest, &lim(100, 1 << 20, 10));
        assert!(
            matches!(r, Err(ExtractError::TooLarge { limit }) if limit == 1 << 20),
            "{r:?}"
        );
        assert!(!dest.exists());
        run(&f, "a.zip", &dest, &lim(100, 8 << 20, 10)).unwrap();
        assert_eq!(std::fs::read(dest.join("small.bin")).unwrap(), small);
    }

    #[test]
    fn tar_gz_skipped_entries_count_against_the_budget() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.tar.gz");
        let big = vec![0u8; 2 << 20];
        std::fs::write(
            &f,
            targz_with(&[("other/big.bin", &big), ("pkg/a.txt", b"a")], None),
        )
        .unwrap();
        let with_dir = |dest: &Path, floor: u64| {
            extract_all_with(
                &[ExtractSource {
                    file: &f,
                    file_name: "a.tar.gz",
                    extract_dir: Some("pkg"),
                    extract_to: None,
                }],
                dest,
                &lim(100, floor, 10),
            )
        };
        let dest = d.path().join("out");
        let r = with_dir(&dest, 1 << 20);
        assert!(
            matches!(r, Err(ExtractError::TooLarge { limit }) if limit == 1 << 20),
            "{r:?}"
        );
        assert!(!dest.exists());
        with_dir(&dest, 4 << 20).unwrap();
        assert!(dest.join("a.txt").exists());
        assert!(!dest.join("big.bin").exists());
    }
}

#[cfg(windows)]
#[test]
fn extract_io_error_names_operation_and_path() {
    use std::os::windows::fs::OpenOptionsExt;
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(&f, zip_with(&[("x.txt", b"new")])).unwrap();
    let dest = d.path().join("out");
    std::fs::create_dir(&dest).unwrap();
    std::fs::write(dest.join("x.txt"), b"old").unwrap();
    // Held exclusively for the whole call: removing it can never succeed.
    let _held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dest.join("x.txt"))
        .unwrap();
    let r = extract(&f, "a.zip", None, None, &dest);
    match r {
        Err(ExtractError::Io(m)) => {
            assert!(m.contains("apagar"), "{m}");
            assert!(m.contains("x.txt"), "{m}");
        }
        other => panic!("expected Io error, got {other:?}"),
    }
}

/// Test layer: on the thread that installed it, the first "retrying" event drops the held file.
#[cfg(windows)]
mod release_on_retry {
    use std::fs::File;
    use std::sync::{Arc, Mutex};

    pub type Held = Arc<Mutex<Option<File>>>;

    thread_local! {
        static CURRENT: std::cell::RefCell<Option<Held>> = const { std::cell::RefCell::new(None) };
    }

    struct Message(bool);

    impl tracing::field::Visit for Message {
        fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
            if f.name() == "message" && format!("{v:?}") == "transient fs error, retrying" {
                self.0 = true;
            }
        }
    }

    struct Releaser;

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Releaser {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut m = Message(false);
            event.record(&mut m);
            if m.0 {
                CURRENT.with(|c| {
                    if let Some(h) = c.borrow().as_ref() {
                        h.lock().unwrap().take();
                    }
                });
            }
        }
    }

    /// Process-global subscriber (installed once) routing to the calling thread's `held`.
    pub fn install(held: &Held) -> Installed {
        use tracing_subscriber::layer::SubscriberExt;
        static GLOBAL: std::sync::Once = std::sync::Once::new();
        GLOBAL.call_once(|| {
            let _ = tracing::subscriber::set_global_default(
                tracing_subscriber::registry().with(Releaser),
            );
        });
        CURRENT.with(|c| *c.borrow_mut() = Some(held.clone()));
        Installed
    }

    pub struct Installed;

    impl Drop for Installed {
        fn drop(&mut self) {
            CURRENT.with(|c| *c.borrow_mut() = None);
        }
    }
}

#[cfg(windows)]
#[test]
fn extraction_retries_a_held_file_until_it_is_released() {
    use std::os::windows::fs::OpenOptionsExt;
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.zip");
    std::fs::write(&f, zip_with(&[("x.txt", b"new")])).unwrap();
    let dest = d.path().join("out");
    std::fs::create_dir(&dest).unwrap();
    std::fs::write(dest.join("x.txt"), b"old").unwrap();
    let held: release_on_retry::Held = std::sync::Arc::new(std::sync::Mutex::new(Some(
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(dest.join("x.txt"))
            .unwrap(),
    )));
    let _routing = release_on_retry::install(&held);
    extract(&f, "a.zip", None, None, &dest).unwrap();
    assert_eq!(std::fs::read(dest.join("x.txt")).unwrap(), b"new");
    assert!(held.lock().unwrap().is_none(), "the retry path never ran");
}
