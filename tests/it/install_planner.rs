#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::path::{Path, PathBuf};
use wingetrs::install::layout::Layout;
use wingetrs::install::manifest::{Arch, resolve};
use wingetrs::install::plan::{PersistKind, ShimKind, Step};
use wingetrs::install::planner::{InstalledInfo, PlanError, PlanInput, plan_install, substitute};

fn fx(n: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/scoop/manifests/{n}")).unwrap()
}
fn root() -> Layout {
    Layout::new(r"C:\W")
}
const SC: &str = r"C:\SM\wingetrs Apps";

fn plan(
    file: &str,
    installed: Option<&str>,
    launcher: bool,
) -> Result<wingetrs::install::plan::InstallPlan, PlanError> {
    let m = resolve(&fx(file), Arch::X64).unwrap();
    let l = root();
    plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &m,
        layout: &l,
        shortcut_root: Path::new(SC),
        installed: installed.map(|v| InstalledInfo {
            version: v.into(),
            old_manifest: None,
        }),
        launcher_present: launcher,
        keep_old: false,
        manifest_sha256: "",
    })
}

#[test]
fn step_order_is_the_executor_contract() {
    let p = plan("shortcuts-persist__main__danser-go.json", None, false).unwrap();
    let kinds: Vec<&str> = p
        .steps
        .iter()
        .map(|s| match s {
            Step::EnsureLauncher => "launcher",
            Step::Download { .. } => "download",
            Step::Extract { .. } => "extract",
            Step::ExtractInno { .. } => "extract_inno",
            Step::Promote => "promote",
            Step::RunInstaller { .. } => "installer",
            Step::RunPs1Installer { .. } => "installer_ps1",
            Step::Hook { .. } => "hook",
            Step::UpgradeUninstallHook { .. } => "upgrade_hook",
            Step::UpgradeRunUninstaller { .. } => "upgrade_uninstaller",
            Step::Persist(_) => "persist",
            Step::LinkCurrent => "current",
            Step::Shim(_) => "shim",
            Step::Shortcut(_) => "shortcut",
            Step::PsModule { .. } => "psmodule",
            Step::EnvSet { .. } => "envset",
            Step::EnvAddPath { .. } => "envpath",
            Step::EnsureShimsOnPath => "path",
            Step::WriteManifest { .. } => "manifest",
            Step::RecordState => "state",
            Step::RemoveOldVersion { .. } => "rmold",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "launcher", "download", "extract", "promote", "persist", "current", "shim", "shim",
            "shortcut", "path", "manifest", "state"
        ]
    );
    assert!(
        !plan("shortcuts-persist__main__danser-go.json", None, true)
            .unwrap()
            .steps
            .contains(&Step::EnsureLauncher)
    );
}

#[test]
fn shim_names_kinds_and_args() {
    let p = plan("bin-alias__main__alist.json", None, true).unwrap();
    let shims: Vec<_> = p
        .steps
        .iter()
        .filter_map(|s| {
            if let Step::Shim(x) = s {
                Some(x.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(shims.len(), 1);
    assert_eq!(shims[0].name, "alist");
    assert_eq!(shims[0].args.as_deref(), Some("--force-bin-dir"));
    assert_eq!(shims[0].kind, ShimKind::Exe);
    assert_eq!(
        shims[0].target,
        PathBuf::from(r"C:\W\apps\app\current\alist.exe")
    );
    let p = plan("persist-file-and-dir__main__i2p-zero.json", None, true).unwrap();
    let s = p
        .steps
        .iter()
        .find_map(|s| {
            if let Step::Shim(x) = s {
                Some(x.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!((s.name.as_str(), s.kind), ("i2p-zero", ShimKind::Bat));
    let p = plan("ps1-bin__main__concfg.json", None, true).unwrap();
    let s = p
        .steps
        .iter()
        .find_map(|s| {
            if let Step::Shim(x) = s {
                Some(x.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!((s.name.as_str(), s.kind), ("concfg", ShimKind::Ps1));
}

#[test]
fn persist_dir_vs_file_by_last_component() {
    let p = plan("persist-file-and-dir__main__i2p-zero.json", None, true).unwrap();
    let ps: Vec<_> = p
        .steps
        .iter()
        .filter_map(|s| {
            if let Step::Persist(x) = s {
                Some((x.in_version.clone(), x.kind))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        ps,
        [
            (PathBuf::from("conf"), PersistKind::Dir),
            (PathBuf::from("i2p.base"), PersistKind::File)
        ]
    );
}

#[test]
fn env_variables_are_substituted() {
    let p = plan("env__main__android-cli.json", None, true).unwrap();
    assert!(p.steps.contains(&Step::EnvSet {
        name: "ANDROID_HOME".into(),
        value: r"C:\W\apps\app\current\Sdk".into()
    }));
    assert!(p.steps.contains(&Step::EnvAddPath {
        entry: PathBuf::from(r"C:\W\apps\app\current\Sdk\platform-tools")
    }));
    assert_eq!(
        substitute(
            r"$persist_dir\x;$version;$app;$architecture",
            "app",
            "9",
            Some(Arch::X64),
            &root()
        ),
        r"C:\W\persist\app\x;9;app;64bit"
    );
}

#[test]
fn shortcut_names_are_sanitized_and_contained() {
    let raw = r#"{"version":"1","url":"https://x/a.zip","hash":"aa","shortcuts":[["a.exe","7-Zip\\7-Zip File Manager"],["a.exe","bad:na?me"],["a.exe","..\\..\\escape"]]}"#;
    let m = resolve(raw, Arch::X64).unwrap();
    let l = root();
    let p = plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &m,
        layout: &l,
        shortcut_root: Path::new(SC),
        installed: None,
        launcher_present: true,
        keep_old: false,
        manifest_sha256: "",
    })
    .unwrap();
    let links: Vec<PathBuf> = p
        .steps
        .iter()
        .filter_map(|s| {
            if let Step::Shortcut(x) = s {
                Some(x.link.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        links[0],
        PathBuf::from(r"C:\SM\wingetrs Apps\7-Zip\7-Zip File Manager.lnk")
    );
    assert_eq!(
        links[1],
        PathBuf::from(r"C:\SM\wingetrs Apps\bad_na_me.lnk")
    );
    assert!(
        links[2].starts_with(SC) && !links[2].to_string_lossy().contains(".."),
        "{:?}",
        links[2]
    );
}

#[test]
fn upgrade_and_same_version() {
    let p = plan("ripgrep__main__ripgrep.json", Some("14.0.0"), true).unwrap();
    assert_eq!(p.upgrade_from.as_deref(), Some("14.0.0"));
    assert_eq!(
        p.steps.last(),
        Some(&Step::RemoveOldVersion {
            version: "14.0.0".into()
        })
    );
    assert!(matches!(
        plan("ripgrep__main__ripgrep.json", Some("15.2.0"), true),
        Err(PlanError::AlreadyInstalled { .. })
    ));
}

#[test]
fn display_snapshots_for_every_supported_fixture() {
    for e in std::fs::read_dir("tests/fixtures/scoop/manifests").unwrap() {
        let name = e.unwrap().file_name().to_string_lossy().to_string();
        // `refused-`: resolves, but the planner refuses its scripts (see below).
        if name.starts_with("unsup-") || name.starts_with("refused-") {
            continue;
        }
        let Ok(m) = resolve(&fx(&name), Arch::X64) else {
            continue;
        };
        let l = root();
        let p = plan_install(&PlanInput {
            app: "app",
            bucket: "main",
            manifest: &m,
            layout: &l,
            shortcut_root: Path::new(SC),
            installed: None,
            launcher_present: false,
            keep_old: false,
            manifest_sha256: "",
        })
        .unwrap();
        insta::assert_snapshot!(name.trim_end_matches(".json"), p.to_string());
    }
}

#[test]
fn scanner_refusal_of_a_real_manifest_script_stops_the_plan() {
    // The 7zip bucket manifest rewrites HKCU to HKLM in `post_install`, only inside
    // `if ($global) { ... }`: informational since Task 164, so it plans.
    plan("refused-script__main__7zip.json", None, false).unwrap();
    // The same script without the guard is refused at planning.
    let guarded = "if ($global) { $content = $content -replace 'HKEY_CURRENT_USER(?=.+Classes)', 'HKEY_LOCAL_MACHINE' }";
    let raw = fx("refused-script__main__7zip.json");
    assert!(raw.contains(guarded));
    let unguarded = raw.replace(
        guarded,
        "$content = $content -replace 'HKEY_CURRENT_USER(?=.+Classes)', 'HKEY_LOCAL_MACHINE'",
    );
    let m = resolve(&unguarded, Arch::X64).unwrap();
    let l = root();
    let e = plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &m,
        layout: &l,
        shortcut_root: Path::new(SC),
        installed: None,
        launcher_present: false,
        keep_old: false,
        manifest_sha256: "",
    })
    .unwrap_err();
    assert!(
        matches!(&e, PlanError::ScriptRefused { hook, .. } if hook == "post_install"),
        "{e:?}"
    );
}

fn plan_raw(
    extra: &str,
    keep_old: bool,
    installed: Option<&str>,
) -> Result<wingetrs::install::plan::InstallPlan, PlanError> {
    let raw = format!(r#"{{"version":"2","url":"https://x/a.zip","hash":"aa"{extra}}}"#);
    let m = resolve(&raw, Arch::X64).unwrap();
    let l = root();
    plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &m,
        layout: &l,
        shortcut_root: Path::new(SC),
        installed: installed.map(|v| InstalledInfo {
            version: v.into(),
            old_manifest: None,
        }),
        launcher_present: true,
        keep_old,
        manifest_sha256: "",
    })
}

#[test]
fn unsafe_manifest_paths_are_refused() {
    for extra in [
        r#","persist":"..\\x""#,
        r#","persist":[["data","..\\..\\x"]]"#,
        r#","persist":"C:\\Windows""#,
        r#","persist":"CON""#,
        r#","bin":"..\\evil.exe""#,
        r#","bin":"\\\\server\\share\\a.exe""#,
        r#","bin":"a.exe:ads""#,
        r#","shortcuts":[["..\\a.exe","n"]]"#,
        r#","shortcuts":[["a.exe","n","",".\\..\\i.ico"]]"#,
        r#","env_add_path":"C:\\Windows""#,
        r#","env_add_path":"..\\x""#,
        r#","env_add_path":"$dir\\..\\x""#,
        r#","env_add_path":"$dir-evil\\x""#,
    ] {
        assert!(
            matches!(plan_raw(extra, false, None), Err(PlanError::UnsafePath(_))),
            "{extra}"
        );
    }
}

#[test]
fn empty_and_dot_env_path_entries_are_dropped() {
    let p = plan_raw(r#","env_add_path":["", ".", "bin"]"#, false, None).unwrap();
    let entries: Vec<_> = p
        .steps
        .iter()
        .filter_map(|s| {
            if let Step::EnvAddPath { entry } = s {
                Some(entry.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        entries,
        [
            PathBuf::from(r"C:\W\apps\app\current"),
            PathBuf::from(r"C:\W\apps\app\current\bin")
        ]
    );
}

#[test]
fn shim_conflict_is_case_insensitive() {
    assert_eq!(
        plan_raw(r#","bin":["a\\x.exe","b\\X.bat"]"#, false, None),
        Err(PlanError::ShimConflict("X".into()))
    );
}

#[test]
fn keep_old_skips_removal_of_the_old_version() {
    let p = plan_raw("", true, Some("1")).unwrap();
    assert_eq!(p.upgrade_from.as_deref(), Some("1"));
    assert!(
        !p.steps
            .iter()
            .any(|s| matches!(s, Step::RemoveOldVersion { .. }))
    );
    assert_eq!(p.steps.last(), Some(&Step::RecordState));
}

#[test]
fn reserved_and_trailing_dot_shortcut_names_are_sanitized() {
    let p = plan_raw(
        r#","shortcuts":[["a.exe","CON"],["a.exe","foo."]]"#,
        false,
        None,
    )
    .unwrap();
    let links: Vec<PathBuf> = p
        .steps
        .iter()
        .filter_map(|s| {
            if let Step::Shortcut(x) = s {
                Some(x.link.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        links,
        [
            PathBuf::from(r"C:\SM\wingetrs Apps\_CON.lnk"),
            PathBuf::from(r"C:\SM\wingetrs Apps\foo_.lnk")
        ]
    );
}

#[test]
fn persist_with_a_different_target() {
    let p = plan_raw(
        r#","persist":[["conf\\app.ini","settings.ini"]]"#,
        false,
        None,
    )
    .unwrap();
    let ps: Vec<_> = p
        .steps
        .iter()
        .filter_map(|s| {
            if let Step::Persist(x) = s {
                Some(x.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(ps.len(), 1);
    assert_eq!(ps[0].in_version, PathBuf::from(r"conf\app.ini"));
    assert_eq!(ps[0].in_persist, PathBuf::from("settings.ini"));
    assert_eq!(ps[0].kind, PersistKind::File);
}

fn plan_bin(bin: &str) -> Result<wingetrs::install::plan::InstallPlan, PlanError> {
    let j = format!(
        r#"{{"version":"1","url":"https://x/z.zip","hash":"{}","bin":{bin}}}"#,
        "a".repeat(64)
    );
    let m = resolve(&j, Arch::X64).unwrap();
    let l = root();
    plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &m,
        layout: &l,
        shortcut_root: Path::new(SC),
        installed: None,
        launcher_present: false,
        keep_old: false,
        manifest_sha256: "",
    })
}

fn shims(p: &wingetrs::install::plan::InstallPlan) -> Vec<(String, Option<String>)> {
    p.steps
        .iter()
        .filter_map(|s| match s {
            Step::Shim(x) => Some((x.name.clone(), x.args.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn zigup_args_are_substituted() {
    let p = plan_bin(r#"[["zigup.exe","zigup","--path-link","\"$persist_dir\\zig\\zig.exe\""]]"#)
        .unwrap();
    let expected = format!(
        "--path-link \"{}\\zig\\zig.exe\"",
        root().persist_dir("app").display()
    );
    assert_eq!(shims(&p), [("zigup".to_string(), Some(expected))]);
}

#[test]
fn empty_alias_falls_back_to_the_file_stem() {
    let p = plan_bin(r#"[["tools\\run.exe",""]]"#).unwrap();
    assert_eq!(shims(&p)[0].0, "run");
}

#[test]
fn unsafe_shim_alias_is_refused_by_the_planner() {
    for alias in [
        "..\\\\evil",
        "a/b",
        "CON",
        "x.",
        "a/",
        ".\\\\a",
        "a\\u0001b",
        "a|b",
        "a*b",
    ] {
        let r = plan_bin(&format!(r#"[["a.exe","{alias}"]]"#));
        assert!(matches!(r, Err(PlanError::UnsafePath(_))), "{alias}: {r:?}");
    }
    assert!(plan_bin(r#"[["a.exe","ok"]]"#).is_ok());
}

#[test]
fn control_characters_and_quotes_in_bin_paths_are_refused() {
    for p in [r#""a\r\nargs = evil.exe""#, r#""a\".exe""#] {
        let r = plan_bin(&format!(r#"[[{p},"ok"]]"#));
        assert!(matches!(r, Err(PlanError::UnsafePath(_))), "{p}: {r:?}");
    }
}

#[test]
fn control_characters_in_args_are_refused() {
    let r = plan_bin(r#"[["a.exe","a","--x\r\npath = C:\\evil.exe"]]"#);
    assert!(matches!(r, Err(PlanError::UnsafeArgs(_))), "{r:?}");
    assert!(plan_bin(r#"[["a.exe","a","--x path = C:\\evil.exe"]]"#).is_ok());
}

fn plan_url(
    url_file: &str,
    extra: &str,
) -> Result<wingetrs::install::plan::InstallPlan, PlanError> {
    let j = format!(
        r#"{{"version":"1","url":"https://x/{url_file}","hash":"{}"{extra}}}"#,
        "a".repeat(64)
    );
    let m = resolve(&j, Arch::X64).unwrap();
    let l = root();
    plan_install(&PlanInput {
        app: "app",
        bucket: "main",
        manifest: &m,
        layout: &l,
        shortcut_root: Path::new(SC),
        installed: None,
        launcher_present: true,
        keep_old: false,
        manifest_sha256: "",
    })
}

#[test]
fn sevenzip_manifest_resolves() {
    let p = plan_url("tool.7z", "").unwrap().to_string();
    assert!(
        p.contains(r"Extrair [1] tool.7z (7z) para C:\W\apps\app\.staging-1"),
        "{p}"
    );
}

#[test]
fn msi_manifest_resolves() {
    let p = plan_url("tool.msi", r#","extract_dir":"Files\\Tool""#)
        .unwrap()
        .to_string();
    assert!(
        p.contains(
            r"Extrair [1] tool.msi (MSI, sem executar o instalador) para C:\W\apps\app\.staging-1 (só a subpasta Files\Tool)"
        ),
        "{p}"
    );
}

#[test]
fn engine_shown_for_tar_xz_and_nupkg_only() {
    let line = |f: &str| {
        let p = plan_url(f, "").unwrap().to_string();
        p.lines()
            .find(|l| l.contains("Extrair") || l.contains("Copiar"))
            .unwrap()
            .trim()
            .to_string()
    };
    assert_eq!(
        line("a.tar.xz"),
        r"2. Extrair [1] a.tar.xz (tar.xz) para C:\W\apps\app\.staging-1"
    );
    assert_eq!(
        line("a.nupkg"),
        r"2. Extrair [1] a.nupkg (zip) para C:\W\apps\app\.staging-1"
    );
    assert_eq!(
        line("a.zip"),
        r"2. Extrair [1] a.zip para C:\W\apps\app\.staging-1"
    );
    assert_eq!(
        line("a.tar.gz"),
        r"2. Extrair [1] a.tar.gz para C:\W\apps\app\.staging-1"
    );
    assert_eq!(
        line("a.exe"),
        r"2. Copiar [1] a.exe para C:\W\apps\app\.staging-1"
    );
}

#[test]
fn rar_still_unsupported() {
    let j = format!(
        r#"{{"version":"1","url":"https://x/tool.rar","hash":"{}"}}"#,
        "a".repeat(64)
    );
    match resolve(&j, Arch::X64) {
        Err(wingetrs::install::manifest::ManifestError::Unsupported { reason }) => {
            assert!(reason.contains(".rar"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
}
