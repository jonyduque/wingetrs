//! wingetrs: fast package manager over the winget and scoop catalogs.
pub mod bridge;
pub mod buckets;
pub mod cache;
pub mod catalog;
pub mod cli;
pub mod config;
pub mod core;
pub mod fs_retry;
pub mod http;
pub mod icons;
pub mod install;
pub mod installed;
pub mod job_log;
pub mod pins;
pub mod sources;
pub mod tui;
pub mod update;
