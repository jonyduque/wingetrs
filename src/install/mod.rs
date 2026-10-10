//! Scoop app install, uninstall and upgrade (phase 2a).
pub mod download;
pub mod executor;
pub mod extract;
pub mod layout;
pub mod link;
pub mod manifest;
pub mod ops;
pub mod path_edit;
pub mod plan;
pub mod planner;
pub mod residue;
pub mod safepath;
pub mod shim;
pub mod snapshot;
pub mod state;
pub mod target;
pub mod traits;

#[cfg(windows)]
pub mod env_win;
#[cfg(windows)]
pub mod fs_win;
#[cfg(windows)]
pub mod prod_win;
#[cfg(windows)]
pub mod registry_win;
#[cfg(windows)]
pub mod shortcut_win;
#[cfg(windows)]
pub mod trash_win;
