//! Objective tie-breaks for ambiguous correlations (spec §5.4).
//!
//! Every rule only narrows candidates that already tied; when no rule leaves exactly one,
//! the correlation stays ambiguous (never pick silently a candidate that may be wrong).
use crate::core::version::Version;

/// Which tie-break resolved an ambiguous correlation (shown in `list --json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Tiebreak {
    /// The winget tracking catalog (`installed.db`) recorded this id for the install.
    WingetTracking,
    /// Only this candidate's ARP version range contains the installed version.
    ArpVersionRange,
    /// The base id beat `<base>.<channel>` variants absent from the display name.
    Channel,
}

/// Channel/variant suffixes recognised by [`by_channel`] (compared case-insensitively).
pub const CHANNEL_SUFFIXES: &[&str] = &[
    "Beta",
    "Canary",
    "Dev",
    "Nightly",
    "Preview",
    "Pre-Release",
    "Insiders",
    "Essentials",
    "Shared",
];

/// A tied candidate with the ARP version range its catalog entry declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub arp_min_version: Option<String>,
    pub arp_max_version: Option<String>,
}

/// Rule 1: keeps the candidates the winget tracking catalog recorded for this install.
pub fn by_tracking<'a>(candidates: &[&'a str], tracked: &[String]) -> Vec<&'a str> {
    candidates
        .iter()
        .copied()
        .filter(|c| tracked.iter().any(|t| t.eq_ignore_ascii_case(c)))
        .collect()
}

fn present(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|s| !s.is_empty())
}

/// Rule 2: keeps the candidates whose ARP range `[min, max]` contains the installed version.
/// Applies only when every candidate declares both bounds: the catalog range covers the
/// published versions, so an older install falls outside its own package's range and a
/// candidate without a range cannot be ruled in or out. Otherwise nothing is dropped.
pub fn by_arp_range<'a>(installed: Option<&str>, candidates: &[&'a Candidate]) -> Vec<&'a str> {
    let all = || candidates.iter().map(|c| c.id.as_str()).collect();
    let Some(raw) = present(installed) else {
        return all();
    };
    let inst = Version::parse(raw);
    if inst.is_unknown() {
        return all();
    }
    let mut kept = Vec::new();
    for c in candidates {
        let (Some(min), Some(max)) = (
            present(c.arp_min_version.as_deref()),
            present(c.arp_max_version.as_deref()),
        ) else {
            return all();
        };
        if Version::parse(min) <= inst && inst <= Version::parse(max) {
            kept.push(c.id.as_str());
        }
    }
    kept
}

/// Lowercase letters and digits only, so "Pre-Release" also matches "PreRelease"/"pre release".
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Rule 3: the base id wins when every other candidate is `<base>.<known channel>` and the
/// installed display name mentions none of those channels.
pub fn by_channel<'a>(display_name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    if candidates.len() < 2 {
        return None;
    }
    let name = squash(display_name);
    let channel_of = |base: &str, other: &str| -> Option<&'static str> {
        let rest = other
            .get(..base.len())
            .filter(|p| p.eq_ignore_ascii_case(base))
            .and_then(|_| other.get(base.len()..))?
            .strip_prefix('.')?;
        CHANNEL_SUFFIXES
            .iter()
            .copied()
            .find(|s| s.eq_ignore_ascii_case(rest))
    };
    // At most one id can be the `<base>.` prefix of all others, so the first winner is the only one.
    candidates.iter().copied().find(|base| {
        candidates
            .iter()
            .filter(|o| !o.eq_ignore_ascii_case(base))
            .all(|o| channel_of(base, o).is_some_and(|s| !name.contains(&squash(s))))
    })
}

/// A rule that would drop every candidate decides nothing.
fn narrow<'a>(kept: Vec<&'a str>, current: Vec<&'a str>) -> Vec<&'a str> {
    if kept.is_empty() { current } else { kept }
}

/// Applies rules 1–3 in order, each on the candidates left by the previous one.
/// Returns the winner and the rule that decided, or `None` (stays ambiguous).
pub fn break_tie(
    display_name: &str,
    installed: Option<&str>,
    candidates: &[Candidate],
    tracked: &[String],
) -> Option<(String, Tiebreak)> {
    let all: Vec<&str> = candidates.iter().map(|c| c.id.as_str()).collect();

    let left = narrow(by_tracking(&all, tracked), all.clone());
    if left.len() == 1 && left.len() < all.len() {
        return Some((left[0].to_owned(), Tiebreak::WingetTracking));
    }

    let ranged: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| left.contains(&c.id.as_str()))
        .collect();
    let before = left.len();
    let left = narrow(by_arp_range(installed, &ranged), left);
    if left.len() == 1 && left.len() < before {
        return Some((left[0].to_owned(), Tiebreak::ArpVersionRange));
    }

    by_channel(display_name, &left).map(|id| (id.to_owned(), Tiebreak::Channel))
}
