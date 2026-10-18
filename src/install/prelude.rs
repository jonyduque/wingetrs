//! Own PowerShell prelude of Scoop functions (Task 152).
#![allow(clippy::unwrap_used, clippy::expect_used)]

/// The prelude text (ASCII only); defines functions and nothing else.
pub const PRELUDE: &str = include_str!("prelude.ps1");

/// Scoop function names implemented by the prelude so far.
pub const PRELUDE_FUNCTIONS: &[&str] = &[
    "info",
    "warn",
    "error",
    "abort",
    "is_admin",
    "ensure",
    "get_config",
    "appsdir",
    "appdir",
    "versiondir",
    "currentdir",
    "persistdir",
    "shimdir",
    "modulesdir",
    "installed",
    "Select-CurrentVersion",
    "Find-BucketDirectory",
    "Get-HelperPath",
    "Get-CommandPath",
    "Get-AbsolutePath",
    "friendly_path",
    "movedir",
    "Split-PathLikeEnvVar",
    "ConvertToPrettyJson",
];

/// Variables the runner defines before dot-sourcing the prelude.
pub const CONTRACT_VARS: &[&str] = &[
    "__wr_root",
    "__wr_effects",
    "__wr_env_seed",
    "__wr_helpers",
    "__wr_shortcut_root",
];
