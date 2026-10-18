//! Installer selection (Task 173): which `EffectiveInstaller` to run on this host, and
//! whether it runs natively or through the winget bridge.
// Filters and ordering follow winget-cli's ManifestComparator.cpp (architecture,
// minimum OS version, locale, scope), trimmed to what the native path needs.
use crate::sources::winget::manifest::{EffectiveInstaller, InstallerKind};

/// Facts about the machine that decide the choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    /// winget architecture name: `x64`, `x86` or `arm64`.
    pub arch: String,
    /// Windows version: major, minor, build, revision.
    pub os: [u32; 4],
    /// Preferred UI languages (BCP-47, such as `pt-BR`), most preferred first.
    pub locale: Languages,
}

/// Ordered language preference list; `"pt-BR".into()` makes a one-item list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Languages(Vec<String>);

impl Languages {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

impl From<Vec<String>> for Languages {
    fn from(v: Vec<String>) -> Self {
        Languages(v)
    }
}

impl From<&str> for Languages {
    fn from(s: &str) -> Self {
        Languages(vec![s.to_string()])
    }
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
pub(crate) fn arch_rank(installer: &str, host_arch: &str) -> Option<usize> {
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

/// 2 = same locale, 1 = same language, 0 = other or unspecified.
fn locale_level(installer: &Option<String>, host: &str) -> u8 {
    let Some(l) = installer.as_deref() else {
        return 0;
    };
    if l.eq_ignore_ascii_case(host) {
        return 2;
    }
    let lang = |s: &str| {
        s.split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    u8::from(!lang(l).is_empty() && lang(l) == lang(host))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Negative,
    Weak,
    Strong,
}

/// Does `first` beat `second` on locale?
// LocaleComparator::IsFirstBetter in ManifestComparator.cpp.
fn locale_cmp(first: &EffectiveInstaller, second: &EffectiveInstaller, host: &Languages) -> Cmp {
    // no preference at all: nothing to compare (winget returns Negative)
    if host.is_empty() {
        return Cmp::Negative;
    }
    // the first preference either installer matches decides, so earlier languages weigh more
    for preferred in host.as_slice() {
        let (f, s) = (
            locale_level(&first.locale, preferred),
            locale_level(&second.locale, preferred),
        );
        if f >= 1 || s >= 1 {
            return if f > s { Cmp::Strong } else { Cmp::Negative };
        }
    }
    if first.locale.is_none() && second.locale.is_some() {
        Cmp::Weak
    } else {
        Cmp::Negative
    }
}

// ScopeComparator::IsFirstBetter in ManifestComparator.cpp.
fn scope_cmp(first: &EffectiveInstaller, second: &EffectiveInstaller, pref: Scope) -> Cmp {
    let (f, s) = (parse_scope(&first.scope), parse_scope(&second.scope));
    if f == Some(pref) && s != Some(pref) {
        if s.is_none() { Cmp::Weak } else { Cmp::Strong }
    } else {
        Cmp::Negative
    }
}

// MachineArchitectureComparator::IsFirstBetter in ManifestComparator.cpp.
fn arch_cmp(first: &EffectiveInstaller, second: &EffectiveInstaller, host_arch: &str) -> Cmp {
    let (f, s) = (
        arch_rank(&first.architecture, host_arch),
        arch_rank(&second.architecture, host_arch),
    );
    match (f, s) {
        // lower rank is better; rank 0 is the native architecture (the strong match)
        (Some(f), Some(s)) if f < s => {
            if f == 0 {
                Cmp::Strong
            } else {
                Cmp::Weak
            }
        }
        _ => Cmp::Negative,
    }
}

/// winget's `ManifestComparator::IsFirstBetter`: comparators in priority order (locale, scope,
/// architecture), the first strong result in either direction wins, else the first weak one.
// ManifestComparator::IsFirstBetter and the comparator order, ManifestComparator.cpp:766-775, 831-895.
fn is_first_better(
    first: &EffectiveInstaller,
    second: &EffectiveInstaller,
    host: &Host,
    pref: Scope,
) -> bool {
    let rounds = [
        (
            locale_cmp(first, second, &host.locale),
            locale_cmp(second, first, &host.locale),
        ),
        (
            scope_cmp(first, second, pref),
            scope_cmp(second, first, pref),
        ),
        (
            arch_cmp(first, second, &host.arch),
            arch_cmp(second, first, &host.arch),
        ),
    ];
    let mut weak = None;
    for (fwd, rev) in rounds {
        if fwd == Cmp::Strong {
            return true;
        }
        if rev == Cmp::Strong {
            return false;
        }
        if weak.is_none() {
            if fwd == Cmp::Weak {
                weak = Some(true);
            } else if rev == Cmp::Weak {
                weak = Some(false);
            }
        }
    }
    weak.unwrap_or(false)
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

    // Installers that declare the system architecture unsupported are dropped first.
    // IsSystemArchitectureUnsupportedByInstaller, ManifestComparator.cpp.
    let supported: Vec<(usize, &EffectiveInstaller)> = installers
        .iter()
        .enumerate()
        .filter(|(_, i)| {
            !i.unsupported_os_architectures
                .iter()
                .any(|a| a.eq_ignore_ascii_case(&host.arch))
        })
        .collect();
    let by_arch: Vec<(usize, &EffectiveInstaller)> = supported
        .iter()
        .copied()
        .filter(|(_, i)| arch_rank(&i.architecture, &host.arch).is_some())
        .collect();
    if by_arch.is_empty() {
        return Err(Refusal(if supported.len() < installers.len() {
            format!(
                "o instalador declara esta arquitetura ({}) como não suportada",
                host.arch
            )
        } else {
            format!("nenhum instalador para esta arquitetura ({})", host.arch)
        }));
    }

    // Minimum OS version; remember why installers were turned away. No Platform filter:
    // winget's ManifestComparator.cpp has none (Windows.Universal MSIX must stay eligible).
    let mut lowest_needed: Option<([u32; 4], String)> = None;
    let mut bad_version = false;
    let mut by_os = Vec::new();
    for (idx, i) in by_arch {
        if let Some(text) = &i.minimum_os_version {
            match parse_os(text) {
                Some(min) if min <= host.os => {}
                Some(min) => {
                    if lowest_needed.as_ref().is_none_or(|(l, _)| min < *l) {
                        lowest_needed = Some((min, text.trim().to_string()));
                    }
                    continue;
                }
                None => {
                    bad_version = true;
                    continue;
                }
            }
        }
        by_os.push((idx, i));
    }
    if by_os.is_empty() {
        return Err(Refusal(match lowest_needed {
            Some((_, text)) => format!("exige Windows {text} ou mais novo"),
            None if bad_version => "versão mínima do Windows inválida no manifesto".to_string(),
            None => "nenhum instalador aplicável a este Windows".to_string(),
        }));
    }

    let by_scope: Vec<(usize, &EffectiveInstaller)> = by_os
        .into_iter()
        .filter(|(_, i)| match wanted {
            ScopeWanted::Prefer(_) => true,
            ScopeWanted::Only(s) => scope_rank(&i.scope, s) < 2,
        })
        .collect();

    // Like winget: walk the manifest order, a later installer replaces the best only when
    // strictly better, so ties keep the earlier one.
    let mut best: Option<(usize, &EffectiveInstaller)> = None;
    for (idx, i) in by_scope {
        if best.is_none_or(|(_, b)| is_first_better(i, b, host, pref)) {
            best = Some((idx, i));
        }
    }
    let Some((index, installer)) = best else {
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
