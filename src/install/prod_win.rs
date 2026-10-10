//! Production wiring of `OpsCtx`: real HKCU, Start Menu, Recycle Bin and registry scan.
//! With `WINGETRS_TEST_ENV_KEY` / `WINGETRS_TEST_SHORTCUT_DIR` set, nothing real is touched
//! (and the residue scan reads neither Desktop, Start Menu nor AppData).
use std::path::{Path, PathBuf};

use crate::cache::DataDir;
use crate::catalog::Catalog;
use crate::http::Http;
use crate::install::download::Progress;
use crate::install::env_win::RegEnvStore;
use crate::install::fs_win;
use crate::install::ops::{OpsCtx, ResidueScan, installed_by_scoop};
use crate::install::registry_win::RegScanner;
use crate::install::shortcut_win::{self, ComShortcuts};
use crate::install::traits::{EffectError, Effects};
use crate::install::trash_win::ShellTrash;
use crate::update::LockWait;

/// Owns the real effect implementations; [`ProdEffects::effects`] borrows them.
pub struct ProdEffects {
    env: RegEnvStore,
    shortcuts: ComShortcuts,
    trash: ShellTrash,
    registry: RegScanner,
}

impl ProdEffects {
    pub fn new() -> Result<ProdEffects, EffectError> {
        Ok(ProdEffects {
            env: RegEnvStore::new(),
            shortcuts: ComShortcuts::new()?,
            trash: ShellTrash,
            registry: RegScanner::from_env(),
        })
    }

    /// `build_effects`: the borrowed bundle `OpsCtx::effects` wants.
    pub fn effects(&self) -> Effects<'_> {
        Effects {
            env: &self.env,
            shortcuts: &self.shortcuts,
            trash: &self.trash,
            registry: &self.registry,
        }
    }
}

fn link_target(p: &Path) -> Option<PathBuf> {
    shortcut_win::link_target(p).ok()
}

/// `build_ctx`: the production `OpsCtx`. Callers supply what differs between front ends:
/// `wait` (CLI: `LockWait::from_env()`, TUI: `NoWait`) and `progress`.
pub fn build_ctx<'a>(
    prod: &'a ProdEffects,
    data: &'a DataDir,
    http: &'a Http,
    catalog: &'a Catalog,
    wait: LockWait,
    progress: Progress<'a>,
) -> OpsCtx<'a> {
    let residue = if std::env::var_os("WINGETRS_TEST_ENV_KEY").is_some() {
        ResidueScan::none()
    } else {
        ResidueScan::user_profile(&link_target, &installed_by_scoop)
    };
    OpsCtx {
        data,
        http,
        catalog,
        effects: prod.effects(),
        wait,
        progress,
        in_use: &fs_win::files_in_use,
        residue,
    }
}
