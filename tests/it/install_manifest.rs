#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::install::manifest::{Arch, HashAlgo, ManifestError, resolve};
use wingetrs::install::planner::refuse_2c;

/// Reason of a refusal: by `resolve` (download formats, hashes) or by the planner (phase 2c keys).
fn refusal(json: &str) -> Option<String> {
    match resolve(json, Arch::X64) {
        Err(ManifestError::Unsupported { reason }) => Some(reason),
        Err(e) => panic!("{e:?}"),
        Ok(m) => refuse_2c(&m).err().map(|e| e.to_string()),
    }
}
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
    let reason = |f: &str| refusal(&fx(f)).unwrap_or_else(|| panic!("{f}: not refused"));
    // `post_install` runs through the script host since Task 156; its `uninstaller.script` does not yet.
    assert!(reason("unsup-script__main__7zip.json").contains("uninstaller"));
    // `innosetup` (Task 147) and `pre_install` (Task 156) are both planned now.
    assert_eq!(refusal(&fx("hooks-innosetup__main__boost.json")), None);
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
        r#"{"version":"1","bin":[["a.exe","a","x",5]]}"#,
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
            assert!(refusal(&fx(&name)).is_some(), "{name}: {r:?}");
        } else {
            assert!(r.is_ok(), "{name}: {r:?}");
        }
        seen += 1;
    }
    assert!(seen >= 19, "fixtures missing: {seen}");
}

fn unsup(json: &str) -> bool {
    refusal(json).is_some()
}

#[test]
fn hash_count_mismatch() {
    let more = r#"{"version":"1","url":"https://x/a.zip","hash":["aa","bb"]}"#;
    assert!(
        matches!(resolve(more, Arch::X64), Err(ManifestError::Invalid(m)) if m.contains("2 hashes para 1 URLs"))
    );
    let less = r#"{"version":"1","url":["https://x/a.zip","https://x/b.zip"],"hash":["aa"]}"#;
    assert!(
        matches!(resolve(less, Arch::X64), Err(ManifestError::Unsupported { reason }) if reason.contains("sem hash"))
    );
}

#[test]
fn support_keys_inside_architecture_variant() {
    for (k, v) in [
        ("pre_uninstall", r#""x""#),
        ("post_uninstall", r#""x""#),
        ("uninstaller", r#"{"file":"a.ps1"}"#),
    ] {
        let j = format!(
            r#"{{"version":"1","architecture":{{"64bit":{{"url":"https://x/a.zip","hash":"aa","{k}":{v}}}}}}}"#
        );
        assert!(unsup(&j), "{k}");
    }
    // `installer.file` `.ps1` is planned since Task 158.
    let ok_ps1 = r#"{"version":"1","architecture":{"64bit":{"url":"https://x/a.zip","hash":"aa","installer":{"file":"a.ps1"}}}}"#;
    assert!(!unsup(ok_ps1));
    // `psmodule` and `innosetup` are read from the root only (as scoop does).
    for (k, v) in [("psmodule", r#"{"name":"X"}"#), ("innosetup", "true")] {
        let j = format!(
            r#"{{"version":"1","architecture":{{"64bit":{{"url":"https://x/a.zip","hash":"aa","{k}":{v}}}}}}}"#
        );
        assert!(!unsup(&j), "{k}");
    }
    let ok = r#"{"version":"1","architecture":{"64bit":{"url":"https://x/a.zip","hash":"aa"}}}"#;
    assert!(!unsup(ok));
}

#[test]
fn extension_is_decided_by_the_renamed_file() {
    let a = r#"{"version":"1","url":"https://x/x.7z#/a.zip","hash":"aa"}"#;
    assert!(resolve(a, Arch::X64).is_ok());
    let b = r#"{"version":"1","url":"https://x/x.zip#/a.rar","hash":"aa"}"#;
    assert!(unsup(b));
}

#[test]
fn variant_replaces_root_fields() {
    let j = r#"{"version":"1","url":"https://x/root.zip","hash":"aa","extract_dir":"rd","bin":"rootbin.exe",
      "shortcuts":[["r.exe","Root"]],"env_set":{"R":"1"},"env_add_path":["rootp"],
      "architecture":{"64bit":{"url":"https://x/v.zip","hash":"bb","extract_dir":"vd","bin":"varbin.exe",
      "shortcuts":[["v.exe","Var"]],"env_set":{"V":"2"},"env_add_path":["varp"]}}}"#;
    let r = resolve(j, Arch::X64).unwrap();
    assert_eq!(r.sources[0].file_name, "v.zip");
    assert_eq!(r.sources[0].extract_dir.as_deref(), Some("vd"));
    assert_eq!(r.bin[0].path, "varbin.exe");
    assert_eq!(r.shortcuts[0].name, "Var");
    assert_eq!(r.env_set, [("V".to_string(), "2".to_string())]);
    assert_eq!(r.env_add_path, ["varp"]);
    let two = r#"{"version":"1","url":["https://x/a.zip","https://x/b.zip"],"hash":["aa","bb"],"extract_dir":["da","db"]}"#;
    let r = resolve(two, Arch::X64).unwrap();
    assert_eq!(r.sources.len(), 2);
    assert_eq!(r.sources[1].file_name, "b.zip");
    assert_eq!(r.sources[1].hash.hex, "bb");
    assert_eq!(r.sources[0].extract_dir.as_deref(), Some("da"));
    assert_eq!(r.sources[1].extract_dir.as_deref(), Some("db"));
}

#[test]
fn host_arch_preference_and_hash_parse() {
    use wingetrs::install::manifest::HashSpec;
    let only32 =
        r#"{"version":"1","architecture":{"32bit":{"url":"https://x/a.zip","hash":"aa"}}}"#;
    assert_eq!(resolve(only32, Arch::X64).unwrap().arch, Some(Arch::X86));
    assert_eq!(resolve(only32, Arch::X86).unwrap().arch, Some(Arch::X86));
    let only64 =
        r#"{"version":"1","architecture":{"64bit":{"url":"https://x/a.zip","hash":"aa"}}}"#;
    assert_eq!(resolve(only64, Arch::X86), Err(ManifestError::NoArch));
    let h = HashSpec::parse("SHA1:ABCD").unwrap();
    assert_eq!((h.algo, h.hex.as_str()), (HashAlgo::Sha1, "abcd"));
    assert!(HashSpec::parse("zzz:ab").is_none());
    let h = HashSpec::parse("ABC").unwrap();
    assert_eq!((h.algo, h.hex.as_str()), (HashAlgo::Sha256, "abc"));
}

fn bin_json(bin: &str) -> String {
    format!(
        r#"{{"version":"1","url":"https://x/z.zip","hash":"{}","bin":{bin}}}"#,
        "a".repeat(64)
    )
}

#[test]
fn bin_with_four_elements_joins_the_extra_args() {
    let j = bin_json(r#"[["zigup.exe","zigup","--path-link","\"$persist_dir\\zig\\zig.exe\""]]"#);
    let r = resolve(&j, Arch::X64).unwrap();
    assert_eq!(r.bin[0].alias.as_deref(), Some("zigup"));
    assert_eq!(
        r.bin[0].args.as_deref(),
        Some(r#"--path-link "$persist_dir\zig\zig.exe""#)
    );
}

#[test]
fn bin_with_five_elements_joins_all_three() {
    let r = resolve(
        &bin_json(r#"[["s.exe","s","-a 1","-b 2","-c 3"]]"#),
        Arch::X64,
    )
    .unwrap();
    assert_eq!(r.bin[0].args.as_deref(), Some("-a 1 -b 2 -c 3"));
}

#[test]
fn bin_with_three_and_two_elements_is_unchanged() {
    let r = resolve(
        &bin_json(r#"[["a.exe","a","--x"],["b.exe","b"],"c.exe"]"#),
        Arch::X64,
    )
    .unwrap();
    assert_eq!(r.bin[0].args.as_deref(), Some("--x"));
    assert_eq!(r.bin[1].args, None);
    assert_eq!(r.bin[2].alias, None);
    assert_eq!(r.bin[2].args, None);
}

#[test]
fn innosetup_is_no_longer_refused() {
    // Task 147: `innosetup` installs through innounp.
    let json = r#"{"version":"1","url":"https://x/a.exe","hash":"0000000000000000000000000000000000000000000000000000000000000000","innosetup":true}"#;
    assert_eq!(refusal(json), None);
}
