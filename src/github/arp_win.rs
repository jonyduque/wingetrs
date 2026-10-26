//! The ARP as a GitHub release install reads it (Task 210): every key now and one key's
//! `UninstallString`, from the registry.
use crate::github::install::{ArpKey, ArpView};
use crate::installed::{Hive, InstallSource};
use crate::winget_install::arp_read_win::read_details;

/// The ARP a GitHub release installer writes to (HKCU and HKLM 64/32-bit, Task 210).
pub struct ReleaseArp;

impl ArpView for ReleaseArp {
    fn keys(&self) -> std::io::Result<Vec<ArpKey>> {
        let apps = crate::installed::read_arp().map_err(std::io::Error::other)?;
        Ok(apps
            .into_iter()
            .filter_map(|a| match a.source {
                InstallSource::Arp(hive) => Some(ArpKey {
                    hive,
                    key: a.key,
                    name: a.name,
                    version: a.version,
                }),
                _ => None,
            })
            .collect())
    }

    fn uninstall_string(&self, hive: Hive, key: &str) -> Option<String> {
        read_details(hive, key)
            .ok()
            .flatten()
            .and_then(|d| d.uninstall_string)
    }
}
