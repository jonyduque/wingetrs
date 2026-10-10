//! Real effects for the operation commands (HKCU, Start Menu, Recycle Bin, registry scan).
//! With `WINGETRS_TEST_ENV_KEY` / `WINGETRS_TEST_SHORTCUT_DIR` set, nothing real is touched.
use super::Command;
use super::install_cmd::{self, LineProgress, TtyUi};
use crate::cache::DataDir;
use crate::catalog::Catalog;
use crate::config::Config;
use crate::http::Http;
use crate::install::env_win::RegEnvStore;
use crate::install::fs_win;
use crate::install::ops::{OpsCtx, ResidueScan, installed_by_scoop};
use crate::install::registry_win::RegScanner;
use crate::install::shortcut_win::{self, ComShortcuts};
use crate::install::traits::Effects;
use crate::install::trash_win::ShellTrash;
use crate::update::LockWait;
use std::io::IsTerminal;
use std::path::Path;

pub async fn run(
    cmd: Command,
    data: &DataDir,
    cfg: &Config,
    http: &Http,
    catalog: &Catalog,
) -> anyhow::Result<()> {
    let env = RegEnvStore::new();
    let shortcuts = ComShortcuts::new().map_err(|e| anyhow::anyhow!("{e}"))?;
    let trash = ShellTrash;
    let registry = RegScanner::from_env();
    let line = LineProgress::new(std::io::stderr().is_terminal());
    let progress = |url: &str, done: u64, total: Option<u64>| line.update(url, done, total);
    let link_target = |p: &Path| shortcut_win::link_target(p).ok();
    // A test run (test registry key) never reads the real Desktop, Start Menu or AppData.
    let residue = if std::env::var_os("WINGETRS_TEST_ENV_KEY").is_some() {
        ResidueScan::none()
    } else {
        ResidueScan::user_profile(&link_target, &installed_by_scoop)
    };
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
        wait: LockWait::from_env(),
        progress: &progress,
        in_use: &fs_win::files_in_use,
        residue,
    };
    let result = install_cmd::execute(&ctx, &TtyUi::new(cfg), cmd).await;
    line.finish();
    result
}
