//! Scoop app install, uninstall and upgrade (phase 2a).
pub mod archive;
pub mod classify_2c;
pub mod download;
pub mod executor;
pub mod extract;
pub(crate) mod extract_7z;
pub(crate) mod extract_msi;
pub mod inno;
pub mod installer_args;
pub mod layout;
pub mod link;
pub mod manifest;
pub mod msi_layout;
pub mod msi_queue;
pub mod msiexec_log;
pub mod ops;
pub mod path_edit;
pub mod plan;
pub mod planner;
pub mod prelude;
pub mod progress;
pub mod residue;
pub mod runner;
pub mod safepath;
pub mod script_effects;
pub mod script_policy;
pub mod script_scan;
pub mod shim;
pub mod snapshot;
pub mod sniff;
pub mod state;
pub mod target;
pub mod traits;

#[cfg(windows)]
pub mod env_win;
#[cfg(windows)]
pub mod fs_win;
#[cfg(windows)]
pub mod inno_win;
#[cfg(windows)]
pub mod installer_win;
#[cfg(windows)]
pub mod msiexec_win;
#[cfg(windows)]
pub mod prod_win;
#[cfg(windows)]
pub mod registry_win;
#[cfg(windows)]
pub mod script_host_win;
#[cfg(windows)]
pub mod shortcut_win;
#[cfg(windows)]
pub mod trash_win;
