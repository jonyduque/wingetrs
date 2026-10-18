//! Runs the operation commands against the production context.
use super::Command;
use super::install_cmd::{self, Flags, Interrupts, Stop, TtyUi, Ui};
use super::winget_cmd::{self, HelperShutdown, WingetFlows};
use crate::bridge::BridgeJob;
use crate::bridge::worker::ScopeArg;
use crate::cache::{BlobStore, DataDir};
use crate::catalog::Catalog;
use crate::config::{Behavior, Config};
use crate::http::Http;
use crate::install::ops::OpsCtx;
use crate::install::prod_win::{ProdEffects, build_ctx};
use crate::install::target::Candidate;
use crate::installed::correlate::{ListRow, correlate};
use crate::installed::{Hive, InstalledApp};
use crate::sources::winget::manifest::MergedManifest;
use crate::tui::app::{Done, InstallFail};
use crate::update::LockWait;
use crate::winget_install::arp_read_win::read_details;
use crate::winget_install::engine::{
    self, UpgradeError, UpgradeRoute, installed_facts, plan_upgrade, upgrade_route,
};
use crate::winget_install::launch::Elevation;
use crate::winget_install::launch_win::{ShellLauncher, msiexec_path};
use crate::winget_install::msix_win::SystemMsix;
use crate::winget_install::ops::{self as wops, InstallReport, WingetCtx, WingetOpsError};
use crate::winget_install::orphans::{self, Orphan};
use crate::winget_install::orphans_win::SystemArp;
use crate::winget_install::plan::{PlanOrBridge, WingetPlan, plan};
use crate::winget_install::select::{Refusal, ScopeWanted};
use crate::winget_install::uninstall::{
    self, SystemClock, UninstallCtx, UninstallPlan, UninstallReport, UninstallTarget, target_of,
};
use crate::winget_install::uninstall_cmd::ArpDetails;
use crate::winget_install::{folders_win, host_win, prod_win};
use anyhow::Context;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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
    // One elevated helper per command, asked to quit (5 s) when `flows` drops, on every path.
    let flows = ProdFlows {
        ctx: &ctx,
        behavior: cfg.behavior,
        helper: HelperShutdown::for_this_exe(),
    };
    let mut ctrl_c = ConsoleCtrlC::default();
    let result = install_cmd::execute(&ctx, &ui, cmd, &mut ctrl_c, &flows).await;
    line.finish();
    drop(flows);
    result
}

/// The real winget flows: catalog manifests, this machine, the registry, the installers.
struct ProdFlows<'c, 'a> {
    ctx: &'c OpsCtx<'a>,
    behavior: Behavior,
    helper: HelperShutdown,
}

fn scan(data: &DataDir) -> Vec<InstalledApp> {
    let scan = crate::installed::scan_all_in(Some(data));
    for e in &scan.errors {
        tracing::warn!("coleta de instalados: {e}");
    }
    scan.apps
}

/// The record still there (a read error counts as there: never "removed" by mistake).
fn key_exists(hive: Hive, key: &str) -> bool {
    !matches!(read_details(hive, key), Ok(None))
}

fn user_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn is_dir(p: &str) -> bool {
    Path::new(p).is_dir()
}

impl ProdFlows<'_, '_> {
    async fn manifest(&self, source: &str, id: &str) -> anyhow::Result<MergedManifest> {
        let pkg = self
            .ctx
            .catalog
            .get(source, id)?
            .context("pacote não encontrado no catálogo")?;
        let hash = pkg
            .winget_hash
            .as_deref()
            .context("pacote winget sem hash de manifesto")?;
        let blobs = BlobStore::new(self.ctx.data.blobs_dir());
        let (m, _) = crate::sources::winget::details::merged_manifest(
            self.ctx.http,
            &blobs,
            crate::sources::winget::CDN_BASE,
            &pkg.entry.id,
            hash,
            Some(&pkg.entry.version),
        )
        .await?;
        Ok(m)
    }

    fn wctx(&self) -> Result<WingetCtx<'_>, WingetOpsError> {
        prod_win::build(
            self.ctx.data,
            self.ctx.http,
            self.ctx.wait,
            self.ctx.progress,
            self.ctx.launch.clone(),
            &self.helper.0,
        )
        .map_err(|e| WingetOpsError::Io(e.to_string()))
    }

    /// The ARP records of `row` other than the main one.
    fn others(&self, row: &ListRow) -> Vec<ArpDetails> {
        row.others
            .iter()
            .filter_map(|o| match target_of(&o.app_key) {
                Some(UninstallTarget::Arp { hive, key }) => read_details(hive, &key).ok()?,
                _ => None,
            })
            .collect()
    }
}

impl WingetFlows for ProdFlows<'_, '_> {
    fn behavior(&self) -> Behavior {
        self.behavior
    }

    async fn plan_install(
        &self,
        cand: &Candidate,
        wanted: ScopeWanted,
    ) -> anyhow::Result<Result<PlanOrBridge, Refusal>> {
        let m = self
            .manifest(&cand.entry.source_name, &cand.entry.id)
            .await?;
        Ok(plan(&m, &host_win::current(), wanted, None))
    }

    async fn install(&self, plan: &WingetPlan) -> Result<InstallReport, WingetOpsError> {
        wops::install(&self.wctx()?, plan).await
    }

    async fn install_bridge<I: Interrupts>(
        &self,
        ui: &dyn Ui,
        cand: &Candidate,
        scope: Option<ScopeArg>,
        f: Flags,
        stop: Stop<'_, I>,
    ) -> anyhow::Result<()> {
        winget_cmd::install_bridge(self.ctx, &self.helper.0, ui, cand, scope, f, stop).await
    }

    fn installed(&self) -> anyhow::Result<Vec<ListRow>> {
        let data = self.ctx.data;
        let apps = scan(data);
        let tracking = crate::installed::winget_tracking::load_default();
        let normalizer = crate::installed::normalize::NameNormalizer::new();
        let overrides = crate::installed::overrides::load(data);
        Ok(correlate(
            &apps,
            self.ctx.catalog,
            &normalizer,
            &tracking,
            &overrides,
        )?)
    }

    fn plan_uninstall(
        &self,
        row: &ListRow,
        target: &UninstallTarget,
    ) -> Result<UninstallPlan, String> {
        match target {
            UninstallTarget::Msix { .. } => Ok(UninstallPlan {
                target: target.clone(),
                name: row.name.clone(),
                version: row.installed_version.clone(),
                command: None,
                elevation: Elevation::Never,
                shared: None,
            }),
            UninstallTarget::Arp { hive, key } => {
                let d = read_details(*hive, key)
                    .map_err(|e| format!("registro ilegível ({key}): {e}"))?
                    .ok_or_else(|| format!("o registro {key} não existe mais"))?;
                let k = crate::bridge::arp_ops_win::known_folders()
                    .map_err(|e| format!("pastas do sistema ilegíveis: {e}"))?;
                let u = folders_win::user_folders(self.ctx.data.root())
                    .map_err(|e| format!("pastas do usuário ilegíveis: {e}"))?;
                // No manifest is read for a removal: the NSIS hint stays off.
                uninstall::plan_uninstall(&d, false, &self.others(row), true, &k, &u, &is_dir)
            }
        }
    }

    fn uninstall(&self, plan: &UninstallPlan) -> Result<UninstallReport, String> {
        let msiexec = msiexec_path().map_err(|e| format!("msiexec.exe não encontrado: {e}"))?;
        let clock = SystemClock::new();
        let u = UninstallCtx {
            launcher: &ShellLauncher,
            elevated: &self.helper.0,
            msix: &SystemMsix,
            key_exists: &key_exists,
            clock: &clock,
            user_expand: &user_env,
            msiexec: &msiexec,
        };
        uninstall::uninstall(&u, plan)
    }

    fn pins(&self) -> BTreeSet<String> {
        crate::pins::load(self.ctx.data)
    }

    async fn plan_upgrade(
        &self,
        row: &ListRow,
        package_id: &str,
        wanted: ScopeWanted,
    ) -> anyhow::Result<(Result<PlanOrBridge, Refusal>, Option<UninstallPlan>)> {
        let m = self.manifest("winget", package_id).await?;
        let installed = row
            .installed_version
            .as_deref()
            .with_context(|| format!("{}: versão instalada desconhecida", row.name))?;
        let target = target_of(&row.app_key);
        let details = match &target {
            Some(UninstallTarget::Arp { hive, key }) => read_details(*hive, key).ok().flatten(),
            _ => None,
        };
        let facts = details.as_ref().map(installed_facts).unwrap_or_default();
        let planned = plan_upgrade(&m, &host_win::current(), wanted, installed, &facts);
        let previous = match (&planned, &target) {
            (Ok(PlanOrBridge::Native(p)), Some(t))
                if upgrade_route(&p.installer) == UpgradeRoute::UninstallFirst =>
            {
                Some(self.plan_uninstall(row, t).map_err(anyhow::Error::msg)?)
            }
            _ => None,
        };
        Ok((planned, previous))
    }

    async fn upgrade(
        &self,
        plan: &WingetPlan,
        previous: Option<&UninstallPlan>,
    ) -> Result<InstallReport, UpgradeError> {
        let w = self.wctx()?;
        let clock = SystemClock::new();
        let u = UninstallCtx {
            launcher: &ShellLauncher,
            elevated: &self.helper.0,
            msix: &SystemMsix,
            key_exists: &key_exists,
            clock: &clock,
            user_expand: &user_env,
            msiexec: &w.msiexec,
        };
        engine::upgrade(&w, &u, plan, previous).await
    }

    fn upgrade_bridge(
        &self,
        job: &BridgeJob,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<Done, InstallFail> {
        let msi_lock = self.ctx.data.install_layout().msi_lock();
        let mut exec = winget_cmd::run_tool;
        crate::tui::jobs::run_bridge_elevated(
            &self.helper.0,
            job,
            None,
            &msi_lock,
            "",
            &mut exec,
            on_line,
        )
    }

    fn orphans(&self) -> anyhow::Result<Vec<Orphan>> {
        Ok(orphans::orphans(&scan(self.ctx.data)))
    }

    fn backups(&self) -> PathBuf {
        self.ctx.data.install_layout().backups()
    }

    fn remove_orphan(&self, o: &Orphan, ts: &str) -> Result<PathBuf, String> {
        orphans::remove_orphan(&SystemArp, &self.helper.0, &self.backups(), ts, o)
    }
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
