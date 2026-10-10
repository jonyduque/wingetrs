//! Production wiring of `OpsCtx` for the TUI: the real Windows effects (spec fase 2a).
//! Tests never use this: they set `WINGETRS_TEST_ENV_KEY` / `WINGETRS_TEST_SHORTCUT_DIR`
//! through the effect constructors, which this module calls.
use crate::cache::DataDir;
use crate::catalog::Catalog;
use crate::http::Http;
use crate::install::download::Progress;
use crate::install::env_win::RegEnvStore;
use crate::install::fs_win::files_in_use;
use crate::install::ops::{OpsCtx, ResidueScan, installed_by_scoop};
use crate::install::registry_win::RegScanner;
use crate::install::shortcut_win::{ComShortcuts, link_target};
use crate::install::traits::Effects;
use crate::install::trash_win::ShellTrash;
use crate::update::LockWait;
use std::path::Path;

/// Builds the production `OpsCtx` and runs `f` with it.
pub fn with_ctx<R>(
    data: &DataDir,
    http: &Http,
    catalog: &Catalog,
    wait: LockWait,
    progress: Progress<'_>,
    f: impl FnOnce(&OpsCtx<'_>) -> R,
) -> Result<R, String> {
    let env = RegEnvStore::new();
    let shortcuts = ComShortcuts::new().map_err(|e| e.0)?;
    let trash = ShellTrash;
    let registry = RegScanner::from_env();
    let link = |p: &Path| link_target(p).ok();
    let ctx = OpsCtx {
        data,
        http,
        catalog,
        effects: Effects {
            env: &env,
            shortcuts: &shortcuts,
            trash: &trash,
            registry: &registry,
        },
        wait,
        progress,
        in_use: &files_in_use,
        residue: ResidueScan::user_profile(&link, &installed_by_scoop),
    };
    Ok(f(&ctx))
}
