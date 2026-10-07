//! Installed package detection.
#[cfg(windows)]
mod appx_win;
pub mod arp;
#[cfg(windows)]
mod arp_win;
pub mod guid;
pub mod normalize;
pub mod scoop;

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

#[derive(Debug, Default)]
pub struct InstalledScan {
    pub apps: Vec<InstalledApp>,
    pub errors: Vec<String>,
    pub elapsed_ms: u128,
}

type Collected = Result<Vec<InstalledApp>, String>;

fn collect_scoop() -> Collected {
    Ok(scoop::scoop_root()
        .map(|r| read_scoop(&r))
        .unwrap_or_default())
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
    appx_win::read_appx().map_err(|e| format!("msix: {e}"))
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
