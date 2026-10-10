#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::install::manifest::{Arch, HashAlgo, ManifestError, resolve};
fn fx(n: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/scoop/manifests/{n}")).unwrap()
}

#[test]
fn ripgrep_x64() {
    let r = resolve(&fx("ripgrep__main__ripgrep.json"), Arch::X64).unwrap();
    assert_eq!(r.version, "15.2.0");
    assert_eq!(r.arch, Some(Arch::X64));
    assert_eq!(r.sources.len(), 1);
    let s = &r.sources[0];
    assert!(s.url.ends_with("ripgrep-15.2.0-x86_64-pc-windows-msvc.zip"));
    assert_eq!(s.file_name, "ripgrep-15.2.0-x86_64-pc-windows-msvc.zip");
    assert_eq!(s.hash.algo, HashAlgo::Sha256);
    assert_eq!(
        s.hash.hex,
        "71b2fef860abe467217a538ff31de02f5258807c0129f771846f87bd029aafc5"
    );
    assert_eq!(
        s.extract_dir.as_deref(),
        Some("ripgrep-15.2.0-x86_64-pc-windows-msvc")
    );
    assert_eq!(r.bin.len(), 1);
    assert_eq!(r.bin[0].path, "rg.exe");
}

#[test]
fn arch_fallback_and_rename_fragment() {
    let r = resolve(&fx("ripgrep__main__ripgrep.json"), Arch::Arm64).unwrap();
    assert_eq!(r.arch, Some(Arch::Arm64));
    let r = resolve(&fx("url-rename__main__agent-browser.json"), Arch::X64).unwrap();
    assert_eq!(r.sources[0].file_name, "agent-browser.exe");
    assert!(
        !r.sources[0].url.contains('#'),
        "fragment is not part of the request URL"
    );
    // only 64bit exists: an x86 host has no variant
    assert_eq!(
        resolve(&fx("url-rename__main__agent-browser.json"), Arch::X86),
        Err(ManifestError::NoArch)
    );
}

#[test]
fn bin_alias_args_hash_prefixes_and_persist() {
    let r = resolve(&fx("bin-alias__main__alist.json"), Arch::X64).unwrap();
    assert_eq!(r.bin[0].path, "alist.exe");
    assert_eq!(r.bin[0].alias.as_deref(), Some("alist"));
    assert_eq!(r.bin[0].args.as_deref(), Some("--force-bin-dir"));
    assert_eq!(r.sources[0].hash.algo, HashAlgo::Md5);
    assert_eq!(
        r.persist
            .iter()
            .map(|p| (p.source.as_str(), p.target.as_str()))
            .collect::<Vec<_>>(),
        [("data", "data"), ("log", "log")]
    );
    assert_eq!(
        resolve(&fx("sha1__main__astyle.json"), Arch::X64)
            .unwrap()
            .sources[0]
            .hash
            .algo,
        HashAlgo::Sha1
    );
    assert_eq!(
        resolve(&fx("sha512__main__antigravity-cli.json"), Arch::X64)
            .unwrap()
            .sources[0]
            .hash
            .algo,
        HashAlgo::Sha512
    );
}

#[test]
fn env_and_depends() {
    let r = resolve(&fx("env__main__android-cli.json"), Arch::X64).unwrap();
    assert!(
        r.env_set
            .contains(&("ANDROID_HOME".to_string(), r"$dir\Sdk".to_string()))
    );
    assert_eq!(
        r.env_add_path,
        [r"Sdk\cmdline-tools\latest\bin", r"Sdk\platform-tools"]
    );
    assert_eq!(
        resolve(&fx("depends__main__apimtemplate.json"), Arch::X64)
            .unwrap()
            .depends,
        ["azure-cli"]
    );
}

#[test]
fn unsupported_manifests_say_why() {
    let reason = |f: &str| match resolve(&fx(f), Arch::X64) {
        Err(ManifestError::Unsupported { reason }) => reason,
        o => panic!("{f}: {o:?}"),
    };
    assert!(reason("unsup-script__main__7zip.json").contains("post_install"));
    assert!(reason("unsup-7z__main__cdrtools.json").contains(".7z"));
    assert!(reason("unsup-msi__main__7zip19.00-helper.json").contains(".msi"));
    let r = reason("unsup-innosetup__main__boost.json");
    assert!(r.contains("innosetup") || r.contains("pre_install"), "{r}");
    let nohash = r#"{"version":"1","url":"https://x/a.zip"}"#;
    assert!(
        matches!(resolve(nohash, Arch::X64), Err(ManifestError::Unsupported { reason }) if reason.contains("sem hash"))
    );
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    for s in [
        "",
        "[]",
        "{",
        r#"{"version":1}"#,
        r#"{"version":"1","url":5,"hash":"x"}"#,
        r#"{"version":"1","bin":[[]]}"#,
    ] {
        assert!(resolve(s, Arch::X64).is_err(), "{s}");
    }
}

#[test]
fn every_fixture_manifest_is_classified_by_its_prefix() {
    let mut seen = 0;
    for e in std::fs::read_dir("tests/fixtures/scoop/manifests").unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().into_owned();
        let r = resolve(&fx(&name), Arch::X64);
        if name.starts_with("unsup-") {
            assert!(
                matches!(r, Err(ManifestError::Unsupported { .. })),
                "{name}: {r:?}"
            );
        } else {
            assert!(r.is_ok(), "{name}: {r:?}");
        }
        seen += 1;
    }
    assert!(seen >= 19, "fixtures missing: {seen}");
}
