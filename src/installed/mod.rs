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
    Wingetrs,
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

/// Apps installed by wingetrs under `<data>\apps`.
fn collect_wingetrs_in(data: &crate::cache::DataDir) -> Collected {
    timed("scan_wingetrs", || {
        Ok(crate::install::state::scan_wingetrs(&data.install_layout()))
    })
}

/// A named collector; the name only identifies it in tests.
type Collector<'a> = (&'static str, Box<dyn Fn() -> Collected + Send + Sync + 'a>);

/// Every collector `scan_all_in` runs. Without `data` the wingetrs one reports an error.
fn collectors(data: Option<&crate::cache::DataDir>) -> Vec<Collector<'_>> {
    vec![
        ("arp", Box::new(collect_arp)),
        ("msix", Box::new(collect_msix)),
        ("scoop", Box::new(collect_scoop)),
        (
            "wingetrs",
            Box::new(move || match data {
                Some(d) => collect_wingetrs_in(d),
                None => Err("wingetrs: pasta de dados desconhecida".to_string()),
            }),
        ),
    ]
}

/// Runs the collectors in parallel; a failing collector is reported, not fatal.
pub fn scan_all_in(data: Option<&crate::cache::DataDir>) -> InstalledScan {
    scan_with(&collectors(data))
}

/// [`scan_all_in`] with `%LOCALAPPDATA%\wingetrs`.
pub fn scan_all() -> InstalledScan {
    let data = crate::cache::DataDir::from_env().ok();
    scan_all_in(data.as_ref())
}

/// Runs `fns` in parallel. A collector error is reported; a collector panic is propagated.
fn scan_with(fns: &[Collector<'_>]) -> InstalledScan {
    let start = std::time::Instant::now();
    let results: Vec<Collected> = std::thread::scope(|s| {
        let handles: Vec<_> = fns.iter().map(|(_, f)| s.spawn(f)).collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
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

#[cfg(test)]
mod scan_tests {
    use super::*;

    #[test]
    fn collector_panic_is_propagated() {
        fn ok() -> Collected {
            Ok(Vec::new())
        }
        fn boom() -> Collected {
            panic!("collector boom")
        }
        let fns: Vec<Collector<'_>> = vec![("ok", Box::new(ok)), ("boom", Box::new(boom))];
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scan_with(&fns)));
        assert!(r.is_err(), "collector panic must not become a warning");
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn scan_all_in_runs_the_wingetrs_collector() {
        let t = tempfile::tempdir().unwrap();
        let data = crate::cache::DataDir::new(t.path());
        let current = data.install_layout().current("foo");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(
            current.join("install.json"),
            br#"{"bucket":"main","version":"1.0","wingetrs":1}"#,
        )
        .unwrap();
        // Only the wingetrs collector runs: the others read the real registry and profile.
        let only: Vec<Collector<'_>> = collectors(Some(&data))
            .into_iter()
            .filter(|(name, _)| *name == "wingetrs")
            .collect();
        let scan = scan_with(&only);
        assert_eq!(scan.errors, Vec::<String>::new());
        assert_eq!(scan.apps.len(), 1);
        assert_eq!(scan.apps[0].source, InstallSource::Wingetrs);
        assert_eq!(scan.apps[0].key, "foo");
        assert_eq!(scan.apps[0].version.as_deref(), Some("1.0"));
        let none = collectors(None);
        let (_, f) = none.iter().find(|(n, _)| *n == "wingetrs").unwrap();
        assert!(f().is_err(), "unknown data folder is reported");
    }
}
