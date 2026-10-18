//! Own PowerShell prelude of Scoop functions (Task 152).

/// The prelude text (ASCII only); defines functions and nothing else.
pub const PRELUDE: &str = include_str!("prelude.ps1");

/// Scoop function names implemented by the prelude so far (internal `__wr_*` helpers excluded).
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
    "Get-EnvVar",
    "Set-EnvVar",
    "Add-Path",
    "Remove-Path",
    "env",
    "add_first_in_path",
    "remove_from_path",
    "shim",
    "rm_shim",
    "shortcut_folder",
    "startmenu_shortcut",
];

/// Variables the runner defines before dot-sourcing the prelude.
pub const CONTRACT_VARS: &[&str] = &[
    "__wr_root",
    "__wr_effects",
    "__wr_env_seed",
    "__wr_helpers",
    "__wr_shortcut_root",
];
