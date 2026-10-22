//! Task 209: release asset choice and installer/program classification.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use wingetrs::github::Asset;
use wingetrs::github::pick::{Pick, pick, unsupported};
use wingetrs::install::manifest::Arch;
use wingetrs::install::sniff::{PeKind, classify_pe};

const RIPGREP: &[u8] = include_bytes!("../fixtures/github/ripgrep.json");
const BAT: &[u8] = include_bytes!("../fixtures/github/bat.json");
const JQ: &[u8] = include_bytes!("../fixtures/github/jq.json");
const NPP: &[u8] = include_bytes!("../fixtures/github/notepad-plus-plus.json");
const PWSH: &[u8] = include_bytes!("../fixtures/github/powershell.json");

/// Assets of a Task 208 fixture (name, size and the real digest).
fn from_fixture(body: &[u8]) -> Vec<Asset> {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap();
    v["assets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            let mut sha256 = [0u8; 32];
            let digest = a["digest"].as_str().unwrap();
            hex::decode_to_slice(digest.strip_prefix("sha256:").unwrap(), &mut sha256).unwrap();
            Asset {
                name: a["name"].as_str().unwrap().to_string(),
                size: a["size"].as_u64().unwrap(),
                sha256,
                url: a["browser_download_url"].as_str().unwrap().to_string(),
            }
        })
        .collect()
}

/// Real name lists (latest releases on 2026-10-22); one distinct digest per name.
fn from_names(names: &[&str]) -> Vec<Asset> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut sha256 = [0u8; 32];
            sha256[..8].copy_from_slice(&(i as u64 + 1).to_le_bytes());
            Asset {
                name: n.to_string(),
                size: 1000,
                sha256,
                url: format!("https://github.com/o/r/releases/download/v1/{n}"),
            }
        })
        .collect()
}

const FZF: &[&str] = &[
    "fzf-0.74.5-android_arm64.tar.gz",
    "fzf-0.74.5-darwin_amd64.tar.gz",
    "fzf-0.74.5-darwin_arm64.tar.gz",
    "fzf-0.74.5-freebsd_amd64.tar.gz",
    "fzf-0.74.5-linux_amd64.tar.gz",
    "fzf-0.74.5-linux_arm64.tar.gz",
    "fzf-0.74.5-linux_armv5.tar.gz",
    "fzf-0.74.5-linux_armv6.tar.gz",
    "fzf-0.74.5-linux_armv7.tar.gz",
    "fzf-0.74.5-linux_loong64.tar.gz",
    "fzf-0.74.5-linux_ppc64le.tar.gz",
    "fzf-0.74.5-linux_riscv64.tar.gz",
    "fzf-0.74.5-linux_s390x.tar.gz",
    "fzf-0.74.5-openbsd_amd64.tar.gz",
    "fzf-0.74.5-windows_amd64.zip",
    "fzf-0.74.5-windows_arm64.zip",
    "fzf_0.74.5_amd64.deb",
    "fzf_0.74.5_arm64.deb",
    "fzf_0.74.5_armv5.deb",
    "fzf_0.74.5_armv6.deb",
    "fzf_0.74.5_armv7.deb",
    "fzf_0.74.5_checksums.txt",
    "fzf_0.74.5_loong64.deb",
    "fzf_0.74.5_ppc64le.deb",
    "fzf_0.74.5_riscv64.deb",
    "fzf_0.74.5_s390x.deb",
];

const ZOXIDE: &[&str] = &[
    "zoxide-0.10.0-aarch64-apple-darwin.tar.gz",
    "zoxide-0.10.0-aarch64-linux-android.tar.gz",
    "zoxide-0.10.0-aarch64-pc-windows-msvc.zip",
    "zoxide-0.10.0-aarch64-unknown-linux-musl.tar.gz",
    "zoxide-0.10.0-arm-unknown-linux-musleabihf.tar.gz",
    "zoxide-0.10.0-armv7-linux-androideabi.tar.gz",
    "zoxide-0.10.0-armv7-unknown-linux-musleabihf.tar.gz",
    "zoxide-0.10.0-i686-unknown-linux-musl.tar.gz",
    "zoxide-0.10.0-riscv64gc-unknown-linux-musl.tar.gz",
    "zoxide-0.10.0-x86_64-apple-darwin.tar.gz",
    "zoxide-0.10.0-x86_64-pc-windows-msvc.zip",
    "zoxide-0.10.0-x86_64-unknown-linux-musl.tar.gz",
    "zoxide_0.10.0-1_amd64.deb",
    "zoxide_0.10.0-1_arm64.deb",
    "zoxide_0.10.0-1_armhf.deb",
    "zoxide_0.10.0-1_i386.deb",
    "zoxide_0.10.0-1_riscv64.deb",
];

const LAZYGIT: &[&str] = &[
    "checksums.txt",
    "lazygit_0.66.2_darwin_arm64.tar.gz",
    "lazygit_0.66.2_darwin_x86_64.tar.gz",
    "lazygit_0.66.2_freebsd_32-bit.tar.gz",
    "lazygit_0.66.2_freebsd_arm64.tar.gz",
    "lazygit_0.66.2_freebsd_armv6.tar.gz",
    "lazygit_0.66.2_freebsd_x86_64.tar.gz",
    "lazygit_0.66.2_linux_32-bit.tar.gz",
    "lazygit_0.66.2_linux_arm64.tar.gz",
    "lazygit_0.66.2_linux_armv6.tar.gz",
    "lazygit_0.66.2_linux_x86_64.tar.gz",
    "lazygit_0.66.2_windows_32-bit.zip",
    "lazygit_0.66.2_windows_arm64.zip",
    "lazygit_0.66.2_windows_x86_64.zip",
];

const NEOVIM: &[&str] = &[
    "nvim-linux-arm64.appimage",
    "nvim-linux-arm64.appimage.zsync",
    "nvim-linux-arm64.tar.gz",
    "nvim-linux-x86_64.appimage",
    "nvim-linux-x86_64.appimage.zsync",
    "nvim-linux-x86_64.tar.gz",
    "nvim-macos-arm64.tar.gz",
    "nvim-macos-x86_64.tar.gz",
    "nvim-win-arm64.msi",
    "nvim-win-arm64.zip",
    "nvim-win64.msi",
    "nvim-win64.zip",
];

const GH: &[&str] = &[
    "gh_2.103.0_checksums.txt",
    "gh_2.103.0_linux_386.deb",
    "gh_2.103.0_linux_386.rpm",
    "gh_2.103.0_linux_386.tar.gz",
    "gh_2.103.0_linux_amd64.deb",
    "gh_2.103.0_linux_amd64.rpm",
    "gh_2.103.0_linux_amd64.tar.gz",
    "gh_2.103.0_linux_arm64.deb",
    "gh_2.103.0_linux_arm64.rpm",
    "gh_2.103.0_linux_arm64.tar.gz",
    "gh_2.103.0_linux_armv6.deb",
    "gh_2.103.0_linux_armv6.rpm",
    "gh_2.103.0_linux_armv6.tar.gz",
    "gh_2.103.0_macOS_amd64.zip",
    "gh_2.103.0_macOS_arm64.zip",
    "gh_2.103.0_macOS_universal.pkg",
    "gh_2.103.0_windows_386.msi",
    "gh_2.103.0_windows_386.zip",
    "gh_2.103.0_windows_amd64.msi",
    "gh_2.103.0_windows_amd64.zip",
    "gh_2.103.0_windows_arm64.msi",
    "gh_2.103.0_windows_arm64.zip",
];

const HUGO: &[&str] = &[
    "hugo_0.168.0_checksums.txt",
    "hugo_0.168.0_darwin-universal.pkg",
    "hugo_0.168.0_dragonfly-amd64.tar.gz",
    "hugo_0.168.0_freebsd-amd64.tar.gz",
    "hugo_0.168.0_Linux-64bit.tar.gz",
    "hugo_0.168.0_linux-amd64.deb",
    "hugo_0.168.0_linux-amd64.tar.gz",
    "hugo_0.168.0_linux-arm.tar.gz",
    "hugo_0.168.0_linux-arm64.deb",
    "hugo_0.168.0_linux-arm64.tar.gz",
    "hugo_0.168.0_netbsd-amd64.tar.gz",
    "hugo_0.168.0_openbsd-amd64.tar.gz",
    "hugo_0.168.0_solaris-amd64.tar.gz",
    "hugo_0.168.0_windows-amd64.zip",
    "hugo_0.168.0_windows-arm64.zip",
    "hugo_extended_0.168.0_darwin-universal.pkg",
    "hugo_extended_0.168.0_Linux-64bit.tar.gz",
    "hugo_extended_0.168.0_linux-amd64.deb",
    "hugo_extended_0.168.0_linux-amd64.tar.gz",
    "hugo_extended_0.168.0_linux-arm64.deb",
    "hugo_extended_0.168.0_linux-arm64.tar.gz",
    "hugo_extended_0.168.0_windows-amd64.zip",
    "hugo_extended_withdeploy_0.168.0_darwin-universal.pkg",
    "hugo_extended_withdeploy_0.168.0_Linux-64bit.tar.gz",
    "hugo_extended_withdeploy_0.168.0_linux-amd64.deb",
    "hugo_extended_withdeploy_0.168.0_linux-amd64.tar.gz",
    "hugo_extended_withdeploy_0.168.0_linux-arm64.deb",
    "hugo_extended_withdeploy_0.168.0_linux-arm64.tar.gz",
    "hugo_extended_withdeploy_0.168.0_windows-amd64.zip",
    "hugo_withdeploy_0.168.0_darwin-universal.pkg",
    "hugo_withdeploy_0.168.0_Linux-64bit.tar.gz",
    "hugo_withdeploy_0.168.0_linux-amd64.deb",
    "hugo_withdeploy_0.168.0_linux-amd64.tar.gz",
    "hugo_withdeploy_0.168.0_linux-arm64.deb",
    "hugo_withdeploy_0.168.0_linux-arm64.tar.gz",
    "hugo_withdeploy_0.168.0_windows-amd64.zip",
];

const TERMINAL: &[&str] = &[
    "GroupPolicyTemplates_1.25.2733.0.zip",
    "Microsoft.Windows.Console.ConPTY.1.25.260930003.nupkg",
    "Microsoft.WindowsTerminal_1.25.2733.0_8wekyb3d8bbwe.msixbundle",
    "Microsoft.WindowsTerminal_1.25.2733.0_8wekyb3d8bbwe.msixbundle_Windows10_PreinstallKit.zip",
    "Microsoft.WindowsTerminal_1.25.2733.0_arm64.zip",
    "Microsoft.WindowsTerminal_1.25.2733.0_x64.zip",
    "Microsoft.WindowsTerminal_1.25.2733.0_x86.zip",
];

fn names(p: &Pick) -> Vec<String> {
    match p {
        Pick::One(a) => vec![a.name.clone()],
        Pick::Choose(v) => v.iter().map(|a| a.name.clone()).collect(),
        Pick::None(_) => vec![],
    }
}

#[test]
fn twelve_repositories_on_x64() {
    let one: [(&str, Vec<Asset>, &str); 8] = [
        (
            "bat",
            from_fixture(BAT),
            "bat-v0.26.1-x86_64-pc-windows-msvc.zip",
        ),
        ("fzf", from_names(FZF), "fzf-0.74.5-windows_amd64.zip"),
        (
            "zoxide",
            from_names(ZOXIDE),
            "zoxide-0.10.0-x86_64-pc-windows-msvc.zip",
        ),
        (
            "ripgrep",
            from_fixture(RIPGREP),
            "ripgrep-15.2.0-x86_64-pc-windows-msvc.zip",
        ),
        (
            "lazygit",
            from_names(LAZYGIT),
            "lazygit_0.66.2_windows_x86_64.zip",
        ),
        ("jq", from_fixture(JQ), "jq-win64.exe"),
        ("neovim", from_names(NEOVIM), "nvim-win64.zip"),
        ("gh", from_names(GH), "gh_2.103.0_windows_amd64.zip"),
    ];
    for (repo, assets, want) in one {
        let got = pick(&assets, Arch::X64);
        assert!(matches!(got, Pick::One(_)), "{repo}: {got:?}");
        assert_eq!(names(&got), [want], "{repo}");
    }

    let choose: [(&str, Vec<Asset>, &[&str]); 4] = [
        (
            "hugo",
            from_names(HUGO),
            &[
                "hugo_0.168.0_windows-amd64.zip",
                "hugo_extended_0.168.0_windows-amd64.zip",
                "hugo_extended_withdeploy_0.168.0_windows-amd64.zip",
                "hugo_withdeploy_0.168.0_windows-amd64.zip",
            ],
        ),
        (
            "PowerShell",
            from_fixture(PWSH),
            &[
                "PowerShell-7.6.7-win-fxdependent.zip",
                "PowerShell-7.6.7-win-fxdependentWinDesktop.zip",
                "PowerShell-7.6.7-win-x64.zip",
            ],
        ),
        (
            "Terminal",
            from_names(TERMINAL),
            &[
                "GroupPolicyTemplates_1.25.2733.0.zip",
                "Microsoft.Windows.Console.ConPTY.1.25.260930003.nupkg",
                "Microsoft.WindowsTerminal_1.25.2733.0_8wekyb3d8bbwe.msixbundle",
                "Microsoft.WindowsTerminal_1.25.2733.0_8wekyb3d8bbwe.msixbundle_Windows10_PreinstallKit.zip",
                "Microsoft.WindowsTerminal_1.25.2733.0_x64.zip",
            ],
        ),
        (
            "Notepad++",
            from_fixture(NPP),
            &[
                "npp.8.9.8.1.Installer.x64.exe",
                "npp.8.9.8.1.portable.minimalist.x64.7z",
                "npp.8.9.8.1.portable.x64.7z",
                "npp.8.9.8.1.portable.x64.zip",
            ],
        ),
    ];
    for (repo, assets, want) in choose {
        let got = pick(&assets, Arch::X64);
        assert!(matches!(got, Pick::Choose(_)), "{repo}: {got:?}");
        assert_eq!(names(&got), want, "{repo}");
    }
}

#[test]
fn other_hosts_get_their_architecture() {
    let got = pick(&from_fixture(RIPGREP), Arch::Arm64);
    assert_eq!(names(&got), ["ripgrep-15.2.0-aarch64-pc-windows-msvc.zip"]);
    let got = pick(&from_names(LAZYGIT), Arch::X86);
    assert_eq!(names(&got), ["lazygit_0.66.2_windows_32-bit.zip"]);
    let got = pick(&from_names(GH), Arch::X86);
    assert_eq!(names(&got), ["gh_2.103.0_windows_386.zip"]);
    let got = pick(&from_fixture(JQ), Arch::Arm64);
    assert_eq!(names(&got), ["jq-windows-arm64.exe"]);
}

#[test]
fn nothing_for_windows_is_none() {
    let linux_only = from_names(&[
        "tool-1.0-x86_64-unknown-linux-gnu.tar.gz",
        "tool-1.0-aarch64-apple-darwin.tar.gz",
        "tool-1.0-source.tar.gz",
        "tool_1.0_amd64.deb",
    ]);
    assert!(matches!(pick(&linux_only, Arch::X64), Pick::None(_)));
    assert!(matches!(pick(&[], Arch::X64), Pick::None(_)));
    // Windows build only for another architecture.
    let arm_only = from_names(&[
        "tool-1.0-aarch64-pc-windows-msvc.zip",
        "tool-1.0-linux.tar.gz",
    ]);
    assert!(matches!(pick(&arm_only, Arch::X64), Pick::None(_)));
    // Distribution names are other systems too, never "no token" builds.
    let ubuntu = from_names(&["t-x64-ubuntu.tar.gz", "t-arm64-windows.zip"]);
    assert!(matches!(pick(&ubuntu, Arch::X64), Pick::None(_)));
    for n in ["mac", "ios", "musl", "android", "debian"] {
        let list = from_names(&[&format!("t-x64-{n}.tar.gz"), "t-x64-windows.zip"]);
        assert_eq!(names(&pick(&list, Arch::X64)), ["t-x64-windows.zip"], "{n}");
    }
}

#[test]
fn each_tiebreak_alone() {
    // msvc beats gnu, in both list orders.
    for list in [
        [
            "t-x86_64-pc-windows-gnu.zip",
            "t-x86_64-pc-windows-msvc.zip",
        ],
        [
            "t-x86_64-pc-windows-msvc.zip",
            "t-x86_64-pc-windows-gnu.zip",
        ],
    ] {
        assert_eq!(
            names(&pick(&from_names(&list), Arch::X64)),
            ["t-x86_64-pc-windows-msvc.zip"]
        );
    }
    // Compressed beats the installer package of the same build.
    for list in [
        ["t-win-x64.msi", "t-win-x64.zip"],
        ["t-win-x64.msix", "t-win-x64.zip"],
        ["t-win-x64.appx", "t-win-x64.7z"],
        ["t-win-x64.msixbundle", "t-win-x64.exe"],
    ] {
        assert_eq!(
            names(&pick(&from_names(&list), Arch::X64)),
            [list[1]],
            "{list:?}"
        );
    }
    // Exact architecture beats no architecture token, in both orders.
    for list in [
        ["npp.1.Installer.exe", "npp.1.Installer.x64.exe"],
        ["npp.1.Installer.x64.exe", "npp.1.Installer.exe"],
    ] {
        assert_eq!(
            names(&pick(&from_names(&list), Arch::X64)),
            ["npp.1.Installer.x64.exe"]
        );
    }
    // Different builds are never decided by a tiebreak.
    let two = from_names(&["t-win-x64.zip", "t-win-x64-full.zip"]);
    assert!(matches!(pick(&two, Arch::X64), Pick::Choose(_)));
}

#[test]
fn discarded_names() {
    let noise = [
        "t-win-x64.zip.sha256",
        "t-win-x64.zip.sha512",
        "t-win-x64.zip.sig",
        "t-win-x64.zip.asc",
        "t-win-x64.pem",
        "t-win-x64.sbom",
        "t-win-x64.pdb",
        "t-win-x64-checksums.zip",
        "t-win-x64.txt",
        "t-win-x64.json",
        "t-win-x64.deb",
        "t-win-x64.rpm",
        "t-win-x64.apk",
        "t-win-x64.dmg",
        "t-win-x64.pkg",
        "t-win-x64.AppImage",
        "t-win-x64.tar.bz2",
        "t-win-x64.tar.zst",
        "t-win-x64-linux.zip",
        "t-win-x64-darwin.zip",
        "t-win-x64-macos.zip",
        "t-win-x64-apple.zip",
        "t-win-x64-android.zip",
        "t-win-x64-freebsd.zip",
        "t-win-x64-source.zip",
        "t-win-x64-src.zip",
    ];
    for n in noise {
        let list = from_names(&[n, "t-win-x64.zip"]);
        assert_eq!(names(&pick(&list, Arch::X64)), ["t-win-x64.zip"], "{n}");
    }
}

#[test]
fn architecture_and_os_tokens() {
    // Every x64 spelling is x64; every x86 spelling is out on an x64 host.
    for n in [
        "t-x86_64-windows.zip",
        "t-amd64-windows.zip",
        "t-x64-windows.zip",
        "t-win64.zip",
    ] {
        let list = from_names(&[n, "t-i686-windows.zip"]);
        assert_eq!(names(&pick(&list, Arch::X64)), [n]);
    }
    for n in [
        "t-i686-windows.zip",
        "t-i386-windows.zip",
        "t-x86-windows.zip",
        "t-win32.zip",
        "t-386-windows.zip",
        "t-32-bit-windows.zip",
        "t-aarch64-windows.zip",
        "t-arm64-windows.zip",
        "t-armv7-windows.zip",
    ] {
        let list = from_names(&[n, "t-x64-windows.zip"]);
        assert_eq!(names(&pick(&list, Arch::X64)), ["t-x64-windows.zip"], "{n}");
    }
    // Every Windows spelling counts as an OS token.
    for n in [
        "t-x64-windows.zip",
        "t-x64-win.zip",
        "t-x64-pc-windows.zip",
        "t-x64-msvc.zip",
        "t-x64-mingw.zip",
    ] {
        let list = from_names(&[n, "t-x64.zip", "t-x64-linux.tar.gz"]);
        assert_eq!(names(&pick(&list, Arch::X64)), [n], "{n}");
    }
}

#[test]
fn installer_packages_are_refused() {
    let only_msi = from_names(&["t-1.0-x64.msi"]);
    let Pick::None(why) = pick(&only_msi, Arch::X64) else {
        panic!("msi picked")
    };
    assert_eq!(
        why,
        "instalador MSI: fora do escopo da instalação por release"
    );
    assert!(unsupported("a.msix").is_some());
    assert!(unsupported("a.msixbundle").is_some());
    assert!(unsupported("a.APPX").is_some());
    assert!(unsupported("a.zip").is_none());
    assert!(unsupported("a.exe").is_none());
}

// ---- PE classification ----

const IMAGE_END: usize = 0x400 + 0x1000;

/// PE32+ with one section at 0x400..0x1400, optional overlay and certificate entry.
fn pe(sections: u16, overlay: &[u8], cert: Option<(u32, u32)>, section_name: &[u8; 8]) -> Vec<u8> {
    let mut h = vec![0u8; 0x400];
    h[0] = b'M';
    h[1] = b'Z';
    h[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    h[0x80..0x84].copy_from_slice(b"PE\0\0");
    h[0x86..0x88].copy_from_slice(&sections.to_le_bytes());
    h[0x94..0x96].copy_from_slice(&0xF0u16.to_le_bytes());
    let opt = 0x98;
    h[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
    h[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
    if let Some((off, size)) = cert {
        let e = opt + 112 + 4 * 8;
        h[e..e + 4].copy_from_slice(&off.to_le_bytes());
        h[e + 4..e + 8].copy_from_slice(&size.to_le_bytes());
    }
    let table = opt + 0xF0;
    h[table..table + 8].copy_from_slice(section_name);
    h[table + 16..table + 20].copy_from_slice(&0x1000u32.to_le_bytes());
    h[table + 20..table + 24].copy_from_slice(&0x400u32.to_le_bytes());
    h.resize(IMAGE_END, 0x90);
    h.extend_from_slice(overlay);
    h
}

const TEXT: &[u8; 8] = b".text\0\0\0";
const MIB: usize = 1 << 20;

#[test]
fn program_without_overlay() {
    assert_eq!(
        classify_pe(&pe(1, &[], None, TEXT)).unwrap(),
        PeKind::Program
    );
}

#[test]
fn signed_program_is_a_program() {
    // The whole overlay is the certificate table: 2 MiB, far over 25 % of the file.
    let cert = vec![0x30u8; 2 * MIB];
    let bytes = pe(1, &cert, Some((IMAGE_END as u32, cert.len() as u32)), TEXT);
    assert_eq!(classify_pe(&bytes).unwrap(), PeKind::Program);
}

#[test]
fn nsis_marker_is_an_installer() {
    let mut overlay = vec![0u8; 64];
    overlay[8..20].copy_from_slice(b"NullsoftInst");
    assert_eq!(
        classify_pe(&pe(1, &overlay, None, TEXT)).unwrap(),
        PeKind::Installer("NSIS")
    );
}

#[test]
fn every_generator_marker() {
    let cases: [(&[u8], &str); 5] = [
        (b"Inno Setup Setup Data (6.2.0)", "Inno Setup"),
        (b"\x00rDlPtS\x02\x87eVx", "Inno Setup"),
        (b"InstallShield\x00", "InstallShield"),
        (b"SquirrelSetup\x00", "Squirrel"),
        (
            &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0x00, 0x04],
            "7-Zip SFX",
        ),
    ];
    for (mark, want) in cases {
        let mut overlay = vec![0u8; 32];
        overlay.extend_from_slice(mark);
        let got = classify_pe(&pe(1, &overlay, None, TEXT)).unwrap();
        assert_eq!(got, PeKind::Installer(want), "{want}");
    }
    let burn = pe(1, &[], None, b".wixburn");
    assert_eq!(classify_pe(&burn).unwrap(), PeKind::Installer("WiX Burn"));
}

#[test]
fn large_unmarked_overlay_is_an_installer() {
    let bytes = pe(1, &vec![0x11u8; 2 * MIB], None, TEXT);
    assert_eq!(
        classify_pe(&bytes).unwrap(),
        PeKind::Installer("instalador")
    );
}

#[test]
fn overlay_thresholds_need_both_quarter_and_mib() {
    // Over 25 % but under 1 MiB.
    let small = pe(1, &vec![0x11u8; MIB / 2], None, TEXT);
    assert_eq!(classify_pe(&small).unwrap(), PeKind::Program);
    // Over 1 MiB but under 25 %: a 6 MiB section with a 1.5 MiB overlay.
    let mut big = pe(1, &[], None, TEXT);
    let table = 0x98 + 0xF0;
    big[table + 16..table + 20].copy_from_slice(&(6 * MIB as u32).to_le_bytes());
    big.resize(0x400 + 6 * MIB, 0x90);
    big.extend_from_slice(&vec![0x11u8; MIB + MIB / 2]);
    assert_eq!(classify_pe(&big).unwrap(), PeKind::Program);
}

#[test]
fn absurd_headers_are_errors() {
    let err = |bytes: &[u8]| classify_pe(bytes).unwrap_err().to_string();
    let cases: Vec<(Vec<u8>, &str)> = vec![
        (pe(0xFFFF, &[], None, TEXT), "número de seções absurdo"),
        (pe(97, &[], None, TEXT), "número de seções absurdo"),
        (pe(0, &[], None, TEXT), "nenhuma seção"),
        (b"MZ".to_vec(), "arquivo truncado"),
        (b"not a pe at all".to_vec(), "sem assinatura MZ"),
        (
            pe(1, &[0u8; 16], Some((IMAGE_END as u32, 0x7FFF_FFFF)), TEXT),
            "assinatura fora do overlay",
        ),
        (
            pe(1, &[0u8; 16], Some((0x400, 16)), TEXT),
            "assinatura fora do overlay",
        ),
    ];
    for (bytes, want) in cases {
        assert!(err(&bytes).contains(want), "{want}: {}", err(&bytes));
    }
    // e_lfanew past the end of the file.
    let mut far = pe(1, &[], None, TEXT);
    far[0x3C..0x40].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    assert!(err(&far).contains("sem assinatura PE"));
    // Optional header far larger than any real one.
    let mut huge = pe(1, &[], None, TEXT);
    huge[0x94..0x96].copy_from_slice(&0x2000u16.to_le_bytes());
    assert!(err(&huge).contains("cabeçalho opcional grande demais"));
    // Unknown optional header magic.
    let mut odd = pe(1, &[], None, TEXT);
    odd[0x98..0x9A].copy_from_slice(&0x107u16.to_le_bytes());
    assert!(err(&odd).contains("cabeçalho opcional desconhecido"));
    // Section past the end of the file.
    let mut cut = pe(1, &[], None, TEXT);
    cut.truncate(0x800);
    assert!(err(&cut).contains("seção além do fim do arquivo"));
}

#[test]
fn pe32_header_is_read() {
    let mut bytes = pe(1, &[], None, TEXT);
    let opt = 0x98;
    bytes[opt..opt + 2].copy_from_slice(&0x10Bu16.to_le_bytes());
    bytes[opt + 92..opt + 96].copy_from_slice(&16u32.to_le_bytes());
    assert_eq!(classify_pe(&bytes).unwrap(), PeKind::Program);
    // Signed PE32: certificate entry at data directory 4 of the 32-bit layout.
    let cert = vec![0x30u8; 2 * MIB];
    bytes.extend_from_slice(&cert);
    let e = opt + 96 + 4 * 8;
    bytes[e..e + 4].copy_from_slice(&(IMAGE_END as u32).to_le_bytes());
    bytes[e + 4..e + 8].copy_from_slice(&(cert.len() as u32).to_le_bytes());
    assert_eq!(classify_pe(&bytes).unwrap(), PeKind::Program);
}
