//! Installer selection (Task 173): which `EffectiveInstaller` to run on this host, and
//! whether it runs natively or through the winget bridge.
// Filters and ordering follow winget-cli's ManifestComparator.cpp (architecture, platform,
// minimum OS version, locale, scope), trimmed to what the native path needs.
use crate::sources::winget::manifest::{EffectiveInstaller, InstallerKind};

/// Facts about the machine that decide the choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    /// winget architecture name: `x64`, `x86` or `arm64`.
    pub arch: String,
    /// Windows version: major, minor, build, revision.
    pub os: [u32; 4],
    /// BCP-47 locale name such as `pt-BR`.
    pub locale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    User,
    Machine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeWanted {
    /// Best effort: the wanted scope first, unspecified next, the other last.
    Prefer(Scope),
    /// Refuse installers declared for the other scope.
    Only(Scope),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Run it ourselves; `interactive` when the installer has no silent switch.
    Native { interactive: bool },
    /// Hand it to winget; the text says why (Portuguese, for the user).
    Bridge(String),
}

#[derive(Debug, Clone)]
pub struct Selection {
    /// Position in the slice given to `choose`.
    pub index: usize,
    pub installer: EffectiveInstaller,
    pub verdict: Verdict,
}

/// Why nothing could be chosen (Portuguese, for the user).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal(pub String);

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refusal {}

/// Architectures that run on `arch`, best first (same preference as `bridge::runs_on`).
fn runs_on(arch: &str) -> &'static [&'static str] {
    match arch {
        "x64" => &["x64", "x86"],
        "x86" => &["x86"],
        "arm64" => &["arm64", "x64", "x86"],
        _ => &[],
    }
}

/// 0 = native, 1 = neutral, then the fallbacks in order; `None` = does not run here.
fn arch_rank(installer: &str, host_arch: &str) -> Option<usize> {
    let list = runs_on(&host_arch.to_ascii_lowercase());
    if installer.eq_ignore_ascii_case("neutral") {
        return Some(1);
    }
    let pos = list
        .iter()
        .position(|a| installer.eq_ignore_ascii_case(a))?;
    Some(if pos == 0 { 0 } else { pos + 1 })
}

fn parse_scope(s: &Option<String>) -> Option<Scope> {
    match s.as_deref() {
        Some(s) if s.eq_ignore_ascii_case("user") => Some(Scope::User),
        Some(s) if s.eq_ignore_ascii_case("machine") => Some(Scope::Machine),
        _ => None,
    }
}

/// equal to the wanted scope < unspecified < the other one.
fn scope_rank(installer: &Option<String>, wanted: Scope) -> u8 {
    match parse_scope(installer) {
        Some(s) if s == wanted => 0,
        None => 1,
        Some(_) => 2,
    }
}

/// Numeric, part by part; a missing part is 0; anything else is `None`.
fn parse_os(text: &str) -> Option<[u32; 4]> {
    let mut out = [0u32; 4];
    for (n, part) in text.trim().split('.').enumerate() {
        *out.get_mut(n)? = part.parse().ok()?;
    }
    Some(out)
}

fn locale_rank(installer: &Option<String>, host: &str) -> u8 {
    let Some(l) = installer.as_deref() else {
        return 2;
    };
    if l.eq_ignore_ascii_case(host) {
        return 0;
    }
    let lang = |s: &str| {
        s.split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    if !lang(l).is_empty() && lang(l) == lang(host) {
        1
    } else {
        3
    }
}

fn scope_name(s: Scope) -> &'static str {
    match s {
        Scope::Machine => "máquina",
        Scope::User => "usuário",
    }
}

/// Picks the installer for `host`, or says why none fits.
pub fn choose(
    installers: &[EffectiveInstaller],
    host: &Host,
    wanted: ScopeWanted,
) -> Result<Selection, Refusal> {
    let (ScopeWanted::Prefer(pref) | ScopeWanted::Only(pref)) = wanted;

    let by_arch: Vec<(usize, &EffectiveInstaller)> = installers
        .iter()
        .enumerate()
        .filter(|(_, i)| arch_rank(&i.architecture, &host.arch).is_some())
        .collect();
    if by_arch.is_empty() {
        return Err(Refusal(format!(
            "nenhum instalador para esta arquitetura ({})",
            host.arch
        )));
    }

    // Platform and minimum OS version; remember the lowest version we turned away.
    let mut lowest_needed: Option<([u32; 4], String)> = None;
    let mut by_os = Vec::new();
    for (idx, i) in by_arch {
        if !(i.platform.is_empty() || i.platform.iter().any(|p| p == "Windows.Desktop")) {
            continue;
        }
        if let Some(text) = &i.minimum_os_version {
            match parse_os(text) {
                Some(min) if min <= host.os => {}
                Some(min) => {
                    if lowest_needed.as_ref().is_none_or(|(l, _)| min < *l) {
                        lowest_needed = Some((min, text.trim().to_string()));
                    }
                    continue;
                }
                None => continue,
            }
        }
        by_os.push((idx, i));
    }
    if by_os.is_empty() {
        return Err(Refusal(match lowest_needed {
            Some((_, text)) => format!("exige Windows {text} ou mais novo"),
            None => "nenhum instalador para a plataforma Windows.Desktop".to_string(),
        }));
    }

    let by_scope: Vec<(usize, &EffectiveInstaller)> = by_os
        .into_iter()
        .filter(|(_, i)| match wanted {
            ScopeWanted::Prefer(_) => true,
            ScopeWanted::Only(s) => scope_rank(&i.scope, s) < 2,
        })
        .collect();

    // `min_by_key` keeps the first of equal keys, i.e. the manifest order.
    let Some((index, installer)) = by_scope.into_iter().min_by_key(|(_, i)| {
        (
            scope_rank(&i.scope, pref),
            arch_rank(&i.architecture, &host.arch),
            locale_rank(&i.locale, &host.locale),
        )
    }) else {
        return Err(Refusal(format!(
            "nenhum instalador para escopo {}",
            scope_name(pref)
        )));
    };

    Ok(Selection {
        index,
        verdict: verdict(installer),
        installer: installer.clone(),
    })
}

fn verdict(i: &EffectiveInstaller) -> Verdict {
    match &i.kind {
        InstallerKind::Unknown(s) if s.is_empty() => {
            return Verdict::Bridge("tipo desconhecido: instalado pelo winget".to_string());
        }
        InstallerKind::Zip
        | InstallerKind::Portable
        | InstallerKind::Font
        | InstallerKind::Pwa
        | InstallerKind::Msstore
        | InstallerKind::Unknown(_) => {
            return Verdict::Bridge(format!("tipo {}: instalado pelo winget", i.kind.name()));
        }
        _ => {}
    }
    if let Some(d) = &i.dependencies
        && (!d.windows_features.is_empty()
            || !d.windows_libraries.is_empty()
            || !d.package_dependencies.is_empty()
            || !d.external_dependencies.is_empty())
    {
        return Verdict::Bridge("tem dependências: instalado pelo winget".to_string());
    }
    let no_silent = i.switches.silent.is_none() && i.switches.silent_with_progress.is_none();
    Verdict::Native {
        interactive: i.kind == InstallerKind::Exe && no_silent,
    }
}
