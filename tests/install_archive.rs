#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::install::archive::ArchiveKind::{self, *};
use wingetrs::install::manifest::{Arch, resolve};

const TABLE: &[(&str, Result<ArchiveKind, &str>)] = &[
    ("a.zip", Ok(Zip)),
    ("A.ZIP", Ok(Zip)),
    ("a.tar.gz", Ok(TarGz)),
    ("a.tgz", Ok(TarGz)),
    ("a.7z", Ok(SevenZip)),
    ("a.msi", Ok(Msi)),
    ("a.nupkg", Ok(Nupkg)),
    ("a.tar.xz", Ok(TarXz)),
    ("a.txz", Ok(TarXz)),
    ("a.rar", Err(".rar")),
    ("a.tar.bz2", Err(".tar.bz2")),
    ("a.tbz2", Err(".tbz2")),
    ("a.zst", Err(".zst")),
    ("a.exe", Ok(Plain)),
    ("a.gz", Ok(Plain)),
    ("a.xz", Ok(Plain)),
    ("setup.7z.exe", Ok(Plain)),
    ("a.zip.ps1", Ok(Plain)),
];

#[test]
fn archive_kind_table() {
    for (name, want) in TABLE {
        assert_eq!(ArchiveKind::of(name), *want, "{name}");
    }
}

#[test]
fn supported_kinds() {
    for k in [Zip, TarGz, Plain, Nupkg, TarXz, SevenZip, Msi] {
        assert!(k.supported(), "{k:?}");
    }
}

#[test]
fn engine_named_only_where_the_suffix_does_not_say_it() {
    assert_eq!(SevenZip.engine(), Some("7z"));
    assert_eq!(Msi.engine(), Some("MSI, sem executar o instalador"));
    assert_eq!(TarXz.engine(), Some("tar.xz"));
    assert_eq!(Nupkg.engine(), Some("zip"));
    for k in [Zip, TarGz, Plain] {
        assert_eq!(k.engine(), None, "{k:?}");
    }
}

#[test]
fn verb_matches_kind() {
    assert_eq!(Plain.verb(), "Copiar");
    for k in [Zip, TarGz, SevenZip, Msi, Nupkg, TarXz] {
        assert_eq!(k.verb(), "Extrair", "{k:?}");
    }
}

fn resolve_single_url(url: &str) -> Result<(), String> {
    let json = format!(
        r#"{{"version":"1","url":"{url}","hash":"{}"}}"#,
        "0".repeat(64)
    );
    resolve(&json, Arch::X64)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[test]
fn manifest_and_extractor_agree() {
    for (name, kind) in TABLE {
        let manifest_ok = resolve_single_url(&format!("https://x.test/{name}")).is_ok();
        let accepted = matches!(kind, Ok(k) if k.supported());
        assert_eq!(manifest_ok, accepted, "manifesto: {name}");
        if !accepted {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join(name);
            std::fs::write(&file, b"junk").unwrap();
            let dest = dir.path().join("out");
            let err = wingetrs::install::extract::extract(&file, name, None, None, &dest)
                .expect_err(name);
            assert!(err.to_string().contains("não suportado"), "{name}: {err}");
            assert!(!dest.exists(), "{name}: dest criado");
            let left: Vec<_> = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(left, [std::ffi::OsString::from(name)], "{name}: sobras");
        }
    }
}
