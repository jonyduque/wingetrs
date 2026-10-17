//! All integration tests in one binary: one link instead of one per file.
//! `tests/install_extract.rs` stays a binary of its own: its `release_on_retry` installs a
//! process-global tracing subscriber, and so does `spans`; only the first one wins.

#[allow(clippy::unwrap_used, clippy::expect_used)]
mod spans;

mod bridge;
mod buckets;
mod cache;
mod catalog;
mod cli_color;
mod cli_install;
mod cli_offline;
mod cli_parse;
mod config;
mod correlate;
mod http;
mod icons;
mod index_file;
mod install_7z;
mod install_archive;
mod install_download;
mod install_executor;
mod install_foundation;
mod install_manifest;
mod install_msi;
mod install_msi_layout;
mod install_msi_live;
mod install_msi_queue;
mod install_ops;
mod install_planner;
mod install_residue;
mod install_shim;
mod install_state;
mod install_target;
mod install_win_effects;
mod install_win_scan;
mod installed;
mod normalize;
mod overrides;
mod pins;
mod scoop;
mod scoop_winget_match;
mod search;
mod tui_jobs;
mod tui_loop;
mod tui_state;
mod tui_theme;
mod tui_view;
mod update;
mod version;
mod winget_details;
mod winget_index;
mod winget_manifest;
mod winget_version_data;
