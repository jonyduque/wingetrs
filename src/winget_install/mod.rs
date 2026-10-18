//! Native install/uninstall of winget packages (phase 2d).

#[cfg(windows)]
pub mod arp_read_win;
pub mod codes;
pub mod elevated_rules;
pub mod engine;
pub mod fetch;
#[cfg(windows)]
pub mod host_win;
pub mod launch;
#[cfg(windows)]
pub mod launch_win;
pub mod msix;
#[cfg(windows)]
pub mod msix_win;
pub mod ops;
pub mod orphans;
#[cfg(windows)]
pub mod orphans_win;
pub mod plan;
pub mod post_check;
#[cfg(windows)]
pub mod prod_win;
pub mod select;
pub mod switches;
pub mod uninstall;
pub mod uninstall_cmd;
#[cfg(windows)]
pub mod verified_win;
