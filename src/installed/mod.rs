//! Installed package detection.
#[cfg(windows)]
mod appx_win;
pub mod arp;
#[cfg(windows)]
mod arp_win;
pub mod correlate;
pub mod guid;
pub mod normalize;
pub mod par;
pub mod scoop;
pub mod tiebreak;
pub mod winget_tracking;

pub use arp::filter_arp;
pub use guid::{pack_guid, unpack_guid};
pub use scoop::read_scoop;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    MachineX64,
    MachineX86,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallSource {
    Arp(Hive),
    Msix,
    Scoop,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstalledApp {
    pub source: InstallSource,
    /// ARP subkey name, MSIX family name, or scoop app name
    pub key: String,
    pub name: String,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub product_code: Option<String>,
    pub upgrade_code: Option<String>,
    pub family_name: Option<String>,
    pub scoop_bucket: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawArpEntry {
    pub hive: Option<Hive>,
    pub key_name: String,
    pub display_name: Option<String>,
    pub publisher: Option<String>,
    pub display_version: Option<String>,
    pub windows_installer: Option<u32>,
    pub system_component: Option<u32>,
    pub parent_key_name: Option<String>,
    pub release_type: Option<String>,
}

/// Keeps one MSIX app per family name: the highest version (winget-style ordering).
pub fn dedupe_msix_by_family(apps: Vec<InstalledApp>) -> Vec<InstalledApp> {
    use crate::core::version::Version;
    let mut out: Vec<InstalledApp> = Vec::with_capacity(apps.len());
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for app in apps {
        let fam = match (&app.source, &app.family_name) {
            (InstallSource::Msix, Some(f)) => f.to_ascii_lowercase(),
            _ => {
                out.push(app);
                continue;
            }
        };
        match seen.get(&fam) {
            Some(&i) => {
                let new_v = Version::parse(app.version.as_deref().unwrap_or(""));
                let old_v = Version::parse(out[i].version.as_deref().unwrap_or(""));
                if new_v > old_v {
                    out[i] = app;
                }
            }
            None => {
                seen.insert(fam, out.len());
                out.push(app);
            }
        }
    }
    out
}

/// "ms-resource:..." or blank display names fall back to the package id name.
pub fn msix_display_name(display: Option<&str>, id_name: &str) -> String {
    match display.map(str::trim) {
        Some(d) if !d.is_empty() && !d.starts_with("ms-resource:") => d.to_string(),
        _ => id_name.to_string(),
    }
}

#[derive(Debug, Default)]
pub struct InstalledScan {
    pub apps: Vec<InstalledApp>,
    pub errors: Vec<String>,
    pub elapsed_ms: u128,
}

/// Runs `f` inside an info span named `step` and logs its wall time (visible with `-v`).
pub fn timed<T>(step: &'static str, f: impl FnOnce() -> T) -> T {
    let span = tracing::info_span!("step", step);
    let _g = span.enter();
    let t = std::time::Instant::now();
    let out = f();
    tracing::info!(step, ms = t.elapsed().as_secs_f64() * 1000.0, "span");
    out
}

type Collected = Result<Vec<InstalledApp>, String>;

fn collect_scoop() -> Collected {
    timed("scan_scoop", || {
        Ok(scoop::scoop_root()
            .map(|r| read_scoop(&r))
            .unwrap_or_default())
    })
}

#[cfg(windows)]
fn collect_arp() -> Collected {
    arp_win::read_arp().map_err(|e| format!("arp: {e}"))
}
#[cfg(not(windows))]
fn collect_arp() -> Collected {
    Ok(Vec::new())
}

#[cfg(windows)]
fn collect_msix() -> Collected {
    appx_win::read_appx()
        .map(dedupe_msix_by_family)
        .map_err(|e| format!("msix: {e}"))
}
#[cfg(not(windows))]
fn collect_msix() -> Collected {
    Ok(Vec::new())
}

/// Runs the three collectors in parallel; a failing collector is reported, not fatal.
pub fn scan_all() -> InstalledScan {
    let start = std::time::Instant::now();
    let fns: [fn() -> Collected; 3] = [collect_arp, collect_msix, collect_scoop];
    let results: Vec<Collected> = std::thread::scope(|s| {
        let handles: Vec<_> = fns.iter().map(|f| s.spawn(f)).collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err("collector panicked".to_string()))
            })
            .collect()
    });
    let mut scan = InstalledScan::default();
    for r in results {
        match r {
            Ok(mut a) => scan.apps.append(&mut a),
            Err(e) => scan.errors.push(e),
        }
    }
    scan.elapsed_ms = start.elapsed().as_millis();
    scan
}
