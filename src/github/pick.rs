//! Choice of the release asset for this host (Task 209): filters by name, then
//! tiebreaks between builds that differ only in toolchain, package or architecture
//! token. Anything still ambiguous goes to the user; nothing is guessed.
use crate::github::Asset;
use crate::install::manifest::Arch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    One(Asset),
    /// More than one candidate left: the user chooses (release order kept).
    Choose(Vec<Asset>),
    /// Nothing usable; the reason is shown to the user.
    None(String),
}

/// Signatures, checksums, debug symbols and packages for other systems.
const DISCARD_SUFFIXES: &[&str] = &[
    ".sha256",
    ".sha512",
    ".sig",
    ".asc",
    ".pem",
    ".sbom",
    ".pdb",
    ".txt",
    ".json",
    ".deb",
    ".rpm",
    ".apk",
    ".dmg",
    ".pkg",
    ".appimage",
    ".tar.bz2",
    ".zst",
];

/// Other systems (and source archives); a name with one of these tokens is dropped.
const OTHER_OS: &[&str] = &[
    "linux",
    "darwin",
    "macos",
    "apple",
    "android",
    "freebsd",
    "osx",
    "openbsd",
    "netbsd",
    "dragonfly",
    "solaris",
    "illumos",
    "mac",
    "ios",
    "ubuntu",
    "debian",
    "fedora",
    "alpine",
    "musl",
];
const SOURCE: &[&str] = &["source", "src"];

const WINDOWS: &[&str] = &["windows", "win", "win64", "win32", "msvc", "mingw"];

const INSTALLER_PACKAGES: &[(&str, &str)] = &[
    (".msixbundle", "pacote MSIX"),
    (".appxbundle", "pacote AppX"),
    (".msix", "pacote MSIX"),
    (".appx", "pacote AppX"),
    (".msi", "instalador MSI"),
];

/// The installable formats (also removed before comparing two names as "the same build"): a
/// name without one of them (`install.ps1`, `LICENSE`, `.txt`) is never a candidate.
const EXTENSIONS: &[&str] = &[
    ".tar.gz",
    ".tar.xz",
    ".tgz",
    ".zip",
    ".7z",
    ".exe",
    ".nupkg",
    ".msixbundle",
    ".appxbundle",
    ".msix",
    ".appx",
    ".msi",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NameArch {
    Host(Arch),
    /// 32-bit ARM, MIPS, PowerPC…: never a Windows host here.
    Foreign,
}

/// Lowercase tokens; `x86_64` and `32-bit` are folded first so the split keeps them whole.
fn tokens(name: &str) -> Vec<String> {
    let lower = name
        .to_ascii_lowercase()
        .replace("x86_64", "x64")
        .replace("x86-64", "x64")
        .replace("32-bit", "x86");
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn token_arch(t: &str) -> Option<NameArch> {
    match t {
        "x64" | "amd64" | "win64" => Some(NameArch::Host(Arch::X64)),
        "aarch64" | "arm64" => Some(NameArch::Host(Arch::Arm64)),
        "i686" | "i386" | "x86" | "win32" | "386" => Some(NameArch::Host(Arch::X86)),
        "s390x" | "loong64" | "powerpc" => Some(NameArch::Foreign),
        t if t.starts_with("arm") || t.starts_with("mips") => Some(NameArch::Foreign),
        t if t.starts_with("ppc") || t.starts_with("riscv") => Some(NameArch::Foreign),
        _ => None,
    }
}

fn installable(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    EXTENSIONS.iter().any(|e| lower.ends_with(e))
}

fn discarded(name: &str, toks: &[String]) -> bool {
    let lower = name.to_ascii_lowercase();
    DISCARD_SUFFIXES.iter().any(|s| lower.ends_with(s))
        || lower.contains("checksums")
        || toks
            .iter()
            .any(|t| OTHER_OS.contains(&t.as_str()) || SOURCE.contains(&t.as_str()))
}

/// Why a chosen asset cannot be installed by this path (`None` = supported).
pub fn unsupported(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    INSTALLER_PACKAGES
        .iter()
        .find(|(ext, _)| lower.ends_with(ext))
        .map(|(_, what)| format!("{what}: fora do escopo da instalação por release"))
}

struct Candidate<'a> {
    asset: &'a Asset,
    /// Name without extension, architecture, OS and toolchain tokens.
    key: Vec<String>,
    exact_arch: bool,
    gnu: bool,
    msvc: bool,
    installer_package: bool,
}

fn candidate<'a>(asset: &'a Asset, toks: &[String], exact_arch: bool) -> Candidate<'a> {
    let lower = asset.name.to_ascii_lowercase();
    let stem = EXTENSIONS
        .iter()
        .find_map(|e| lower.strip_suffix(e))
        .unwrap_or(&lower);
    let key = tokens(stem)
        .into_iter()
        .filter(|t| token_arch(t).is_none() && !WINDOWS.contains(&t.as_str()) && t != "gnu")
        .collect();
    Candidate {
        asset,
        key,
        exact_arch,
        gnu: toks.iter().any(|t| t == "gnu"),
        msvc: toks.iter().any(|t| t == "msvc"),
        installer_package: unsupported(&asset.name).is_some(),
    }
}

/// Keeps the members of `group` that pass `better` when at least one does.
fn prefer(group: &mut Vec<usize>, cands: &[Candidate], better: impl Fn(&Candidate) -> bool) {
    if group.iter().any(|&i| better(&cands[i])) {
        group.retain(|&i| better(&cands[i]));
    }
}

pub fn pick(assets: &[Asset], host: Arch) -> Pick {
    let all: Vec<(&Asset, Vec<String>)> = assets.iter().map(|a| (a, tokens(&a.name))).collect();
    // A release that names no other system is Windows-only (Notepad++, Windows Terminal).
    let windows_only = !all
        .iter()
        .any(|(_, t)| t.iter().any(|t| OTHER_OS.contains(&t.as_str())));

    let mut cands: Vec<Candidate> = Vec::new();
    for (asset, toks) in &all {
        if discarded(&asset.name, toks) || !installable(&asset.name) {
            continue;
        }
        let archs: Vec<NameArch> = toks.iter().filter_map(|t| token_arch(t)).collect();
        if !archs.is_empty() && !archs.contains(&NameArch::Host(host)) {
            continue;
        }
        if !windows_only && !toks.iter().any(|t| WINDOWS.contains(&t.as_str())) {
            continue;
        }
        // Same digest = same file published under two names (jq).
        if cands.iter().any(|c| c.asset.sha256 == asset.sha256) {
            continue;
        }
        cands.push(candidate(asset, toks, !archs.is_empty()));
    }

    // Tiebreaks only between names of the same build.
    let mut keep: Vec<usize> = Vec::new();
    let mut seen: Vec<&Vec<String>> = Vec::new();
    for c in &cands {
        if seen.contains(&&c.key) {
            continue;
        }
        seen.push(&c.key);
        let mut group: Vec<usize> = (0..cands.len())
            .filter(|&i| cands[i].key == c.key)
            .collect();
        if group.iter().any(|&i| cands[i].msvc) {
            group.retain(|&i| !cands[i].gnu);
        }
        prefer(&mut group, &cands, |c| !c.installer_package);
        prefer(&mut group, &cands, |c| c.exact_arch);
        keep.extend(group);
    }
    keep.sort_unstable();

    match keep.as_slice() {
        [] => Pick::None(format!(
            "nenhum arquivo do release serve para Windows {}",
            match host {
                Arch::X64 => "x64",
                Arch::X86 => "x86",
                Arch::Arm64 => "arm64",
            }
        )),
        [one] => {
            let asset = cands[*one].asset;
            match unsupported(&asset.name) {
                Some(why) => Pick::None(why),
                None => Pick::One(asset.clone()),
            }
        }
        many => Pick::Choose(many.iter().map(|&i| cands[i].asset.clone()).collect()),
    }
}
