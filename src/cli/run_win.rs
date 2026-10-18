//! Runs the operation commands against the production context.
use super::Command;
use super::install_cmd::{self, Interrupts, TtyUi};
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
    let mut ctrl_c = ConsoleCtrlC::default();
    let result = install_cmd::execute(&ctx, &ui, cmd, &mut ctrl_c).await;
    line.finish();
    result
}

/// Ctrl+C of the console. While held, tokio's handler answers it (the process lives on); once
/// released, no listener is left and Windows ends the process as usual.
#[derive(Default)]
struct ConsoleCtrlC(Option<tokio::signal::windows::CtrlC>);

impl Interrupts for ConsoleCtrlC {
    fn hold(&mut self) {
        if self.0.is_none() {
            match tokio::signal::windows::ctrl_c() {
                Ok(c) => self.0 = Some(c),
                // Without a listener Ctrl+C keeps its default: it ends the process.
                Err(e) => tracing::warn!("Ctrl+C não pôde ser tratado: {e}"),
            }
        }
    }

    fn release(&mut self) {
        self.0 = None;
    }

    async fn next(&mut self) {
        self.hold();
        match &mut self.0 {
            Some(c) => {
                if c.recv().await.is_none() {
                    std::future::pending::<()>().await;
                }
            }
            None => std::future::pending::<()>().await,
        }
    }
}
