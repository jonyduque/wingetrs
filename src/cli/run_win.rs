//! Runs the operation commands against the production context.
use super::Command;
use super::install_cmd::{self, TtyUi};
use crate::cache::DataDir;
use crate::catalog::Catalog;
use crate::config::Config;
use crate::http::Http;
use crate::install::prod_win::{ProdEffects, build_ctx};
use crate::update::LockWait;

pub async fn run(
    cmd: Command,
    data: &DataDir,
    cfg: &Config,
    http: &Http,
    catalog: &Catalog,
) -> anyhow::Result<()> {
    let prod = ProdEffects::new().map_err(|e| anyhow::anyhow!("{e}"))?;
    let ui = TtyUi::new(cfg);
    let line = ui.progress();
    let progress = |ev: &crate::install::progress::ProgressEvent| line.update(ev);
    let ctx = build_ctx(&prod, data, http, catalog, LockWait::from_env(), &progress);
    let result = install_cmd::execute(&ctx, &ui, cmd).await;
    line.finish();
    result
}
