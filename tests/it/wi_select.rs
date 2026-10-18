//! Installer selection (Task 173).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use wingetrs::sources::winget::manifest::{self, EffectiveInstaller};
use wingetrs::winget_install::select::{Host, Scope, ScopeWanted, Selection, Verdict, choose};

const HEAD: &str = "PackageIdentifier: A.B\nPackageVersion: 1.0\n";

fn host(arch: &str) -> Host {
    Host {
        arch: arch.into(),
        os: [10, 0, 22631, 0],
        locale: "pt-BR".into(),
    }
}

/// One installer entry; `extra` is already indented by two spaces.
fn inst(arch: &str, extra: &str) -> String {
    format!("- Architecture: {arch}\n  InstallerUrl: https://x/a\n  InstallerSha256: AA\n{extra}")
}

fn effs(root: &str, items: &[String]) -> Vec<EffectiveInstaller> {
    let yaml = format!("{HEAD}{root}Installers:\n{}", items.concat());
    manifest::parse(yaml.as_bytes()).unwrap().effective()
}

fn idx(items: &[String], h: &Host, w: ScopeWanted) -> usize {
    choose(&effs("", items), h, w).unwrap().index
}

const PU: ScopeWanted = ScopeWanted::Prefer(Scope::User);

fn pick_root(root: &str, items: &[String]) -> Selection {
    choose(&effs(root, items), &host("x64"), PU).unwrap()
}

#[test]
fn native_arch_beats_earlier_fallback() {
    let both = [inst("x86", ""), inst("x64", "")];
    assert_eq!(idx(&both, &host("x64"), PU), 1);
    assert_eq!(idx(&both, &host("x86"), PU), 0);
    assert_eq!(idx(&[inst("x86", "")], &host("x64"), PU), 0);
}

#[test]
fn arm64_falls_back_x64_then_x86() {
    let h = host("arm64");
    assert_eq!(idx(&[inst("x86", ""), inst("x64", "")], &h, PU), 1);
    assert_eq!(idx(&[inst("x86", "")], &h, PU), 0);
    assert_eq!(idx(&[inst("x64", ""), inst("arm64", "")], &h, PU), 1);
}

#[test]
fn user_preferred_over_earlier_machine() {
    let items = [
        inst("x64", "  Scope: machine\n"),
        inst("x64", "  Scope: user\n"),
    ];
    assert_eq!(idx(&items, &host("x64"), PU), 1);
    assert_eq!(
        idx(&items, &host("x64"), ScopeWanted::Prefer(Scope::Machine)),
        0
    );
}

#[test]
fn only_scope_refuses_when_absent() {
    let only_m = ScopeWanted::Only(Scope::Machine);
    let err = choose(
        &effs("", &[inst("x64", "  Scope: user\n")]),
        &host("x64"),
        only_m,
    )
    .unwrap_err();
    assert!(err.0.contains("escopo máquina"), "{}", err.0);
    assert_eq!(idx(&[inst("x64", "")], &host("x64"), only_m), 0);
}

#[test]
fn scope_beats_arch() {
    let items = [
        inst("x64", "  Scope: machine\n"),
        inst("x86", "  Scope: user\n"),
    ];
    assert_eq!(idx(&items, &host("x64"), PU), 1);
}

#[test]
fn locale_exact_then_language_then_any() {
    let l = |t: &str| inst("x64", &format!("  InstallerLocale: {t}\n"));
    let h = host("x64");
    assert_eq!(idx(&[l("en-US"), l("pt-PT"), l("pt-BR")], &h, PU), 2);
    assert_eq!(idx(&[l("en-US"), l("pt-PT")], &h, PU), 1);
    assert_eq!(idx(&[l("en-US")], &h, PU), 0);
    // unspecified beats a different language
    assert_eq!(idx(&[l("en-US"), inst("x64", "")], &h, PU), 1);
}

#[test]
fn min_os_above_host_is_skipped() {
    let high = inst("x64", "  MinimumOSVersion: 10.0.99999.0\n");
    let h = host("x64");
    assert_eq!(idx(&[high.clone(), inst("x86", "")], &h, PU), 1);
    let err = choose(&effs("", &[high]), &h, PU).unwrap_err();
    assert!(err.0.contains("exige Windows"), "{}", err.0);
    assert!(err.0.contains("10.0.99999.0"), "{}", err.0);
}

#[test]
fn os_version_numeric_not_text() {
    let items = [inst("x64", "  MinimumOSVersion: 10.0.9.0\n")];
    assert_eq!(idx(&items, &host("x64"), PU), 0);
    // missing parts are zero, garbage is dropped
    assert_eq!(
        idx(&[inst("x64", "  MinimumOSVersion: 10\n")], &host("x64"), PU),
        0
    );
    let bad = [inst("x64", "  MinimumOSVersion: abc\n")];
    assert!(choose(&effs("", &bad), &host("x64"), PU).is_err());
}

#[test]
fn no_arch_is_refused_with_arch() {
    let err = choose(&effs("", &[inst("arm64", "")]), &host("x64"), PU).unwrap_err();
    assert!(err.0.contains("arquitetura (x64)"), "{}", err.0);
}

#[test]
fn bridge_reasons_one_fixture_each() {
    let bridge = |root: &str| match pick_root(root, &[inst("x64", "")]).verdict {
        Verdict::Bridge(m) => m,
        v => panic!("expected bridge, got {v:?}"),
    };
    for (t, want) in [
        ("zip", "tipo zip"),
        ("portable", "tipo portable"),
        ("font", "tipo font"),
        ("pwa", "tipo pwa"),
        ("msstore", "tipo msstore"),
    ] {
        let m = bridge(&format!("InstallerType: {t}\n"));
        assert!(m.contains(want), "{m}");
    }
    // no InstallerType anywhere
    match pick_root("", &[inst("x64", "")]).verdict {
        Verdict::Bridge(m) => assert!(m.contains("tipo desconhecido"), "{m}"),
        v => panic!("{v:?}"),
    }
    let m = bridge(
        "InstallerType: msi\nDependencies:\n  PackageDependencies:\n  - PackageIdentifier: C.D\n",
    );
    assert!(m.contains("dependências"), "{m}");
}

#[test]
fn exe_silent_rules() {
    let v = |root: &str| pick_root(root, &[inst("x64", "")]).verdict;
    assert_eq!(
        v("InstallerType: exe\n"),
        Verdict::Native { interactive: true }
    );
    assert_eq!(
        v("InstallerType: exe\nInstallerSwitches:\n  SilentWithProgress: /S\n"),
        Verdict::Native { interactive: false }
    );
    assert_eq!(
        v("InstallerType: exe\nInstallerSwitches:\n  Silent: /S\n"),
        Verdict::Native { interactive: false }
    );
}

#[test]
fn classic_types_are_native() {
    for t in ["msi", "wix", "burn", "inno", "nullsoft", "msix", "appx"] {
        let s = pick_root(&format!("InstallerType: {t}\n"), &[inst("x64", "")]);
        assert_eq!(s.verdict, Verdict::Native { interactive: false }, "{t}");
    }
}

#[test]
fn platform_is_not_a_filter() {
    // winget's ManifestComparator has no Platform filter: Windows.Universal stays eligible
    let items = [inst(
        "x64",
        "  Platform:
  - Windows.Universal
",
    )];
    assert_eq!(idx(&items, &host("x64"), PU), 0);
    let items = [
        inst(
            "x86",
            "  Platform:
  - Windows.Desktop
",
        ),
        inst(
            "x64",
            "  Platform:
  - Windows.Universal
",
        ),
    ];
    assert_eq!(idx(&items, &host("x64"), PU), 1);
}

#[test]
#[ignore = "live: lê o registro"]
fn host_current_prints() {
    let h = wingetrs::winget_install::host_win::current();
    println!("{h:?}");
    assert!(h.os[0] >= 10);
    assert!(!h.locale.is_empty());
}

#[test]
fn winget_order_locale_then_scope_then_arch() {
    // preferred scope wins only weakly against an unspecified scope: native arch beats it
    let weak = [inst("x86", "  Scope: user\n"), inst("x64", "")];
    assert_eq!(idx(&weak, &host("x64"), PU), 1);
    // locale is the first comparator: it beats native architecture
    let l = |a: &str, t: &str| inst(a, &format!("  InstallerLocale: {t}\n"));
    assert_eq!(
        idx(&[l("x64", "en-US"), l("x86", "pt-BR")], &host("x64"), PU),
        1
    );
}

#[test]
fn neutral_is_applicable_and_ranks_between() {
    let h = host("x64");
    assert_eq!(idx(&[inst("x86", ""), inst("neutral", "")], &h, PU), 1);
    assert_eq!(idx(&[inst("neutral", ""), inst("x64", "")], &h, PU), 1);
    assert_eq!(idx(&[inst("neutral", "")], &host("arm64"), PU), 0);
}

#[test]
fn min_os_equal_to_host_with_missing_zero_is_accepted() {
    let items = [inst("x64", "  MinimumOSVersion: 10.0.22631\n")];
    assert_eq!(idx(&items, &host("x64"), PU), 0);
}

#[test]
fn unreadable_min_os_has_its_own_reason() {
    let bad = [inst("x64", "  MinimumOSVersion: abc\n")];
    let err = choose(&effs("", &bad), &host("x64"), PU).unwrap_err();
    assert!(
        err.0.contains("versão mínima do Windows inválida"),
        "{}",
        err.0
    );
}

#[test]
fn each_dependency_list_sends_to_the_bridge() {
    for dep in [
        "WindowsFeatures:\n    - NetFx3\n",
        "WindowsLibraries:\n    - Microsoft.VCLibs\n",
        "ExternalDependencies:\n    - Something\n",
    ] {
        let root = format!("InstallerType: msi\nDependencies:\n  {dep}");
        match pick_root(&root, &[inst("x64", "")]).verdict {
            Verdict::Bridge(m) => assert!(m.contains("dependências"), "{dep}: {m}"),
            v => panic!("{dep}: {v:?}"),
        }
    }
}

#[test]
fn unknown_named_type_goes_to_the_bridge() {
    match pick_root("InstallerType: foo\n", &[inst("x64", "")]).verdict {
        Verdict::Bridge(m) => assert!(m.contains("tipo foo"), "{m}"),
        v => panic!("{v:?}"),
    }
}

fn host_langs(arch: &str, langs: &[&str]) -> Host {
    Host {
        locale: langs
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .into(),
        ..host(arch)
    }
}

#[test]
fn locale_prefers_ui_language_list() {
    let l = |t: &str| inst("x64", &format!("  InstallerLocale: {t}\n"));
    let items = [l("pt-BR"), l("en-US")];
    let h = host_langs("x64", &["en-US", "pt-BR"]);
    assert_eq!(idx(&items, &h, PU), 1);
    // the order of the list matters: flipped, the other one wins
    let h = host_langs("x64", &["pt-BR", "en-US"]);
    assert_eq!(idx(&items, &h, PU), 0);
}

#[test]
fn locale_language_only_match_ranks_below_exact() {
    let l = |t: &str| inst("x64", &format!("  InstallerLocale: {t}\n"));
    let h = host_langs("x64", &["pt-BR"]);
    assert_eq!(idx(&[l("pt-PT"), l("pt-BR")], &h, PU), 1);
    assert_eq!(idx(&[l("pt-BR"), l("pt-PT")], &h, PU), 0);
    // a language-only match of the first preference still beats an exact match of the second
    let h = host_langs("x64", &["pt-BR", "en-US"]);
    assert_eq!(idx(&[l("en-US"), l("pt-PT")], &h, PU), 1);
}

#[test]
fn unsupported_os_architecture_excluded() {
    let h = host("arm64");
    // the x64 installer is native-ranked second but marked unsupported on arm64
    let items = [
        inst("x64", "  UnsupportedOSArchitectures:\n  - arm64\n"),
        inst("x86", ""),
    ];
    assert_eq!(idx(&items, &h, PU), 1);
    // only candidate unsupported: refused with a reason
    let err = choose(
        &effs(
            "",
            &[inst("x64", "  UnsupportedOSArchitectures:\n  - arm64\n")],
        ),
        &h,
        PU,
    )
    .unwrap_err();
    assert!(err.0.contains("não suportada"), "{}", err.0);
    // root-level list applies to installers without their own
    let root = "UnsupportedOSArchitectures:\n- arm64\n";
    assert!(choose(&effs(root, &[inst("x64", "")]), &h, PU).is_err());
    // another host architecture is unaffected
    assert_eq!(idx(&items, &host("x64"), PU), 0);
}
