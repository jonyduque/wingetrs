//! The steps of installing, upgrading and removing a GitHub release app that the CLI and the
//! TUI share (Tasks 210-211): download and classify an asset, the Scoop plan of a portable
//! one, the installer's run and record, the upgrade's file choice and the record-only removal.
//! No terminal and no screen here: the callers ask and show.
use anyhow::bail;

use crate::github::install::{GhEnv, GhTarget, Mode};
use crate::github::{Asset, Release};
use crate::install::ops::{self, OpsCtx, OpsError};
use crate::install::progress::ProgressEvent;
use crate::install::state::AppRecord;
use crate::job_log::{JobInfo, JobLog, Outcome};

fn finish_log<T, E: std::fmt::Display>(log: JobLog, result: &Result<T, E>, ok: &str) {
    match result {
        Ok(_) => log.finish(Outcome::Ok, ok),
        Err(e) => log.finish(Outcome::Failed, &e.to_string()),
    }
}
/// The release being installed and where it goes.
pub struct Rel<'a> {
    pub t: &'a GhTarget,
    pub env: &'a GhEnv<'a>,
    pub release: &'a Release,
    pub app: String,
    pub bucket: String,
    pub version: String,
    /// What is installed now, on an upgrade (Task 211).
    pub prior: Option<Prior>,
    /// `--keep`: an upgrade keeps the old version on disk.
    pub keep: bool,
}

/// The installed version an upgrade replaces, and how it was put in place.
#[derive(Debug, Clone, PartialEq)]
pub struct Prior {
    pub version: String,
    pub mode: crate::github::install::Mode,
}

/// An installer asset, downloaded and verified.
#[derive(Debug, Clone, PartialEq)]
pub struct InstallerAsset {
    pub asset: Asset,
    pub generator: &'static str,
    pub blob: std::path::PathBuf,
    pub asks_admin: bool,
}

/// The assets an install uses: the installer (the default) and the program (alone, or the
/// installer's fallback).
pub struct Selection {
    pub installer: Option<InstallerAsset>,
    pub program: Option<Asset>,
}

/// What one candidate asset is.
pub enum AssetKind {
    Installer(InstallerAsset),
    Program(Asset),
    /// MSI/MSIX/AppX: the reason it is out of scope.
    Package(Asset, String),
}

impl AssetKind {
    pub fn asset(&self) -> &Asset {
        match self {
            AssetKind::Installer(i) => &i.asset,
            AssetKind::Program(a) | AssetKind::Package(a, _) => a,
        }
    }

    pub fn detail(&self) -> String {
        match self {
            AssetKind::Installer(i) => format!("instalador {}", i.generator),
            AssetKind::Program(_) => "programa".into(),
            AssetKind::Package(_, why) => why.clone(),
        }
    }

    pub fn into_selection(self) -> anyhow::Result<Selection> {
        match self {
            AssetKind::Installer(i) => Ok(Selection {
                installer: Some(i),
                program: None,
            }),
            AssetKind::Program(a) => Ok(Selection {
                installer: None,
                program: Some(a),
            }),
            AssetKind::Package(_, why) => bail!("{why}"),
        }
    }
}

/// Downloads `asset` into the cache and checks it against the API's digest.
pub async fn fetch_asset(
    ctx: &OpsCtx<'_>,
    rel: &Rel<'_>,
    asset: &Asset,
) -> anyhow::Result<std::path::PathBuf> {
    use crate::github::install as gh;
    use crate::install::manifest::{HashAlgo, HashSpec, Source};
    let source = Source {
        url: gh::download_url(&rel.env.web, asset),
        file_name: gh::safe_name(asset)?.to_string(),
        hash: HashSpec {
            algo: HashAlgo::Sha256,
            hex: hex::encode(asset.sha256),
        },
        extract_dir: None,
        extract_to: None,
    };
    let mut paths =
        crate::install::download::fetch_all(ctx.http, ctx.data, &[source], ctx.progress).await?;
    paths
        .pop()
        .ok_or_else(|| anyhow::anyhow!("{}: download sem arquivo", asset.name))
}

/// What `asset` is: a package out of scope, an installer or a program (`.exe` assets are
/// downloaded and classified).
pub async fn classify_asset(
    ctx: &OpsCtx<'_>,
    rel: &Rel<'_>,
    asset: Asset,
) -> anyhow::Result<AssetKind> {
    use crate::github::install as gh;
    use crate::install::sniff::PeKind;
    if let Some(why) = crate::github::pick::unsupported(&asset.name) {
        return Ok(AssetKind::Package(asset, why));
    }
    if !asset.name.to_ascii_lowercase().ends_with(".exe") {
        return Ok(AssetKind::Program(asset));
    }
    let blob = fetch_asset(ctx, rel, &asset).await?;
    let bytes = std::fs::read(&blob)?;
    Ok(match gh::exe_kind(&bytes)? {
        PeKind::Program => AssetKind::Program(asset),
        PeKind::Installer(generator) => AssetKind::Installer(InstallerAsset {
            asks_admin: gh::asks_admin(&bytes),
            asset,
            generator,
            blob,
        }),
    })
}

/// The Scoop plan of a portable `asset`: downloaded, verified, looked into, then the
/// synthetic manifest through `plan_for_raw`.
pub async fn release_plan(
    ctx: &OpsCtx<'_>,
    rel: &Rel<'_>,
    asset: &Asset,
) -> anyhow::Result<crate::install::plan::InstallPlan> {
    use crate::github::install as gh;
    let blob = fetch_asset(ctx, rel, asset).await?;
    let portable = {
        let scratch = gh::Scratch::new(&ctx.data.tmp_dir(), "inspect")?;
        gh::inspect(&blob, gh::safe_name(asset)?, scratch.path())?
    };
    let url = gh::download_url(&rel.env.web, asset);
    let raw = gh::manifest_json(&url, asset, &portable, &rel.version);
    Ok(ops::plan_for_raw(
        ctx,
        &rel.app,
        &rel.bucket,
        &raw,
        rel.keep,
    )?)
}

/// Runs `f` on this thread, letting a multi-thread runtime move its other tasks away.
pub fn blocking<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
        Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(f),
        _ => f(),
    }
}

pub type InstallerRan =
    Result<crate::github::install::Installed, crate::github::install::InstallerFailure>;

/// The installer's run under `install.lock`, from a verified copy named as the asset, with its
/// job log; a run that installed is recorded. The inner error is the installer's failure.
pub async fn run_release_installer(
    ctx: &OpsCtx<'_>,
    rel: &Rel<'_>,
    inst: &InstallerAsset,
) -> anyhow::Result<InstallerRan> {
    use crate::github::install::{self as gh, InstallerFailure, InstallerRun};
    (ctx.progress)(&ProgressEvent::Phase(
        crate::install::progress::Phase::Preparing,
    ));
    let layout = ctx.data.install_layout();
    let _lock = crate::winget_install::ops::lock_install(&layout, ctx.wait)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let scratch = gh::Scratch::new(&ctx.data.tmp_dir(), "run")?;
    let program = gh::launch_copy(&inst.blob, &inst.asset, scratch.path())?;
    let args = gh::silent_args(inst.generator);
    let mut log = JobLog::start(JobInfo {
        kind: if rel.prior.is_some() {
            "upgrade"
        } else {
            "install"
        },
        tool: "wingetrs",
        target: format!("{}/{}", rel.bucket, rel.app),
        name: rel.app.clone(),
        from: rel.prior.as_ref().map(|p| p.version.clone()),
        to: Some(rel.version.clone()),
        cmd: Some(format!("{} {args}", inst.asset.name).trim_end().to_string()),
    });
    let run = InstallerRun {
        program: &program,
        generator: inst.generator,
        app: &rel.app,
        version: &rel.version,
        asks_admin: inst.asks_admin,
    };
    let ran = blocking(|| gh::run_installer(rel.env, &ctx.launch, ctx.progress, &run));
    drop(scratch);
    match &ran {
        Ok(done) => {
            log.code = Some(done.code);
            let url = gh::download_url(&rel.env.web, &inst.asset);
            let manifest = gh::installer_manifest_json(&url, &inst.asset, &rel.version);
            let recorded = gh::record_installer(
                &layout,
                &rel.app,
                &rel.bucket,
                &rel.version,
                &manifest,
                done,
            );
            finish_log(log, &recorded, "instalado");
            recorded?;
            // The installer replaced the old version: its record folder goes (it holds only
            // `manifest.json` and `install.json`).
            if let Some(p) = rel
                .prior
                .as_ref()
                .filter(|p| matches!(p.mode, gh::Mode::Installer(_)))
                .filter(|p| !rel.keep && p.version != rel.version)
            {
                let old = layout.version_dir(&rel.app, &p.version);
                if let Err(e) = std::fs::remove_dir_all(&old) {
                    tracing::warn!(dir = %old.display(), error = %e, "versão anterior não apagada");
                }
            }
        }
        Err(InstallerFailure::Cancelled) => log.finish(Outcome::Cancelled, "cancelado"),
        Err(e) => {
            if let InstallerFailure::Code(c) = e {
                log.code = Some(*c);
            }
            log.finish(Outcome::Failed, &e.to_string());
        }
    }
    Ok(ran)
}

/// After an upgrade of an app its installer had put in place: when the app is portable now
/// (the new installer failed, or the release has none), the name in the Windows list of the
/// old program, which stays installed.
pub fn old_program_left(ctx: &OpsCtx<'_>, rel: &Rel<'_>) -> Option<String> {
    use crate::github::install as gh;
    let Some(Prior {
        mode: gh::Mode::Installer(arp),
        ..
    }) = &rel.prior
    else {
        return None;
    };
    let now = gh::installed_mode(&ctx.data.install_layout(), &rel.app, &rel.version).ok()?;
    if now != gh::Mode::Portable {
        return None;
    }
    let name = arp.as_ref().and_then(|k| {
        let keys = rel.env.arp.keys().ok()?;
        keys.into_iter()
            .find(|x| x.hive == k.hive && x.key.eq_ignore_ascii_case(&k.key))
            .map(|x| x.name)
    });
    Some(name.unwrap_or_else(|| rel.app.clone()))
}
// ---- upgrade (Task 211) ----

/// What an upgrade starts from: the installed record, the latest release and the file the
/// saved rule names.
pub struct UpgradeStart {
    pub rec: AppRecord,
    pub t: GhTarget,
    pub release: Release,
    pub version: String,
    pub mode: Mode,
    /// The installed file's name with the new version (`None`: the record names no file).
    pub expected: Option<String>,
    /// The release's files for this host (all of them when none fits), in release order.
    pub candidates: Vec<Asset>,
}

impl UpgradeStart {
    /// The file named [`UpgradeStart::expected`], when the release has it.
    pub fn found(&self) -> Option<&Asset> {
        let e = self.expected.as_deref()?;
        self.release.assets.iter().find(|a| a.name == e)
    }

    /// The candidate named `name`.
    pub fn candidate(&self, name: &str) -> Option<&Asset> {
        self.candidates.iter().find(|a| a.name == name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.candidates.iter().map(|a| a.name.as_str()).collect()
    }

    /// The release being installed over the record.
    pub fn rel<'a>(&'a self, env: &'a GhEnv<'a>, keep: bool) -> Rel<'a> {
        Rel {
            t: &self.t,
            env,
            release: &self.release,
            app: self.rec.app.clone(),
            bucket: self.rec.bucket.clone(),
            version: self.version.clone(),
            prior: Some(Prior {
                version: self.rec.version.clone(),
                mode: self.mode.clone(),
            }),
            keep,
        }
    }
}

/// `app`'s record and mode when it comes from a GitHub release.
pub fn release_record(ctx: &OpsCtx<'_>, app: &str) -> anyhow::Result<Option<(AppRecord, Mode)>> {
    release_record_in(&ctx.data.install_layout(), app)
}

/// [`release_record`] in `layout`.
pub fn release_record_in(
    layout: &crate::install::layout::Layout,
    app: &str,
) -> anyhow::Result<Option<(AppRecord, Mode)>> {
    let state = ops::read_state(layout)?;
    let Some((rec, _)) = state.get(app)? else {
        return Ok(None);
    };
    if ops::release_repo(&rec.bucket).is_none() {
        return Ok(None);
    }
    let mode = crate::github::install::installed_mode(layout, &rec.app, &rec.version)
        .map_err(|e| anyhow::anyhow!("{}: install.json ilegível ({e}); rode repair", rec.app))?;
    Ok(Some((rec, mode)))
}

/// The latest release of `rec`'s repository (API) against the installed version: `None` when
/// it is not newer. The rate limit is [`crate::github::GhError::RateLimited`].
pub async fn start_upgrade(
    ctx: &OpsCtx<'_>,
    env: &GhEnv<'_>,
    rec: &AppRecord,
) -> anyhow::Result<Option<UpgradeStart>> {
    use crate::core::version::Version;
    use crate::github::install as gh;
    use crate::github::pick::{Pick, pick};
    let Some(repo) = ops::release_repo(&rec.bucket) else {
        bail!("{} não vem de um release do GitHub", rec.app);
    };
    let layout = ctx.data.install_layout();
    let mode = gh::installed_mode(&layout, &rec.app, &rec.version)
        .map_err(|e| anyhow::anyhow!("{}: install.json ilegível ({e}); rode repair", rec.app))?;
    let t = GhTarget { repo, tag: None };
    let release = crate::github::latest(ctx.http, &env.api, &t.repo).await?;
    let version = gh::version_of(&release.tag)?;
    if Version::parse(&version) <= Version::parse(&rec.version) {
        return Ok(None);
    }
    // The saved rule: the file the installed version came from (an altered record has none).
    let installed = crate::install::state::installed_manifest(
        &layout,
        &rec.app,
        &rec.version,
        &rec.manifest_sha256,
    )
    .ok()
    .flatten()
    .and_then(|raw| gh::asset_of_manifest(&raw));
    let expected = installed.map(|n| gh::expected_asset(&n, &rec.version, &version));
    let candidates = match pick(&release.assets, crate::install::manifest::Arch::host()) {
        Pick::One(a) => vec![a],
        Pick::Choose(list) => list,
        Pick::None(_) => release.assets.clone(),
    };
    Ok(Some(UpgradeStart {
        rec: rec.clone(),
        t,
        release,
        version,
        mode,
        expected,
        candidates,
    }))
}

/// The selection of an upgrade to `asset` (the saved name, or the user's pick): classified; an
/// installer gets the other candidate of the release as its fallback, as an install picks it.
/// A portable app whose file became an installer is refused (its shims would stay behind).
pub async fn select_upgrade(
    ctx: &OpsCtx<'_>,
    rel: &Rel<'_>,
    candidates: &[Asset],
    asset: Asset,
) -> anyhow::Result<Selection> {
    let sel = match classify_asset(ctx, rel, asset).await? {
        AssetKind::Installer(i) => {
            let others: Vec<&Asset> = candidates
                .iter()
                .filter(|a| a.name != i.asset.name)
                .filter(|a| crate::github::pick::unsupported(&a.name).is_none())
                .collect();
            let program = match others.as_slice() {
                [only] => Some((*only).clone()),
                _ => None,
            };
            Selection {
                installer: Some(i),
                program,
            }
        }
        other => other.into_selection()?,
    };
    if let (
        Some(inst),
        Some(Prior {
            mode: Mode::Portable,
            ..
        }),
    ) = (&sel.installer, &rel.prior)
    {
        bail!(
            "{} agora é um instalador ({}) e a versão instalada é portátil; remova com `wingetrs uninstall {}` e instale de novo",
            inst.asset.name,
            inst.generator,
            rel.app
        );
    }
    Ok(sel)
}

/// A prepared upgrade, owned (the TUI carries it from the confirmation to its job).
#[derive(Debug, Clone, PartialEq)]
pub struct ReleaseUpgrade {
    pub t: GhTarget,
    pub release: Release,
    pub app: String,
    pub bucket: String,
    pub version: String,
    pub prior: Prior,
    pub keep: bool,
    pub installer: Option<InstallerAsset>,
    pub program: Option<Asset>,
}

impl ReleaseUpgrade {
    pub fn new(start: UpgradeStart, keep: bool, sel: Selection) -> ReleaseUpgrade {
        ReleaseUpgrade {
            app: start.rec.app.clone(),
            bucket: start.rec.bucket.clone(),
            prior: Prior {
                version: start.rec.version.clone(),
                mode: start.mode,
            },
            t: start.t,
            release: start.release,
            version: start.version,
            keep,
            installer: sel.installer,
            program: sel.program,
        }
    }

    pub fn rel<'a>(&'a self, env: &'a GhEnv<'a>) -> Rel<'a> {
        Rel {
            t: &self.t,
            env,
            release: &self.release,
            app: self.app.clone(),
            bucket: self.bucket.clone(),
            version: self.version.clone(),
            prior: Some(self.prior.clone()),
            keep: self.keep,
        }
    }

    /// The lines of its confirmation: what runs, its fallback and the origin.
    pub fn notes(&self) -> Vec<String> {
        let mut out = Vec::new();
        match &self.installer {
            Some(i) => {
                out.push(format!(
                    "Roda o instalador {} (origem não revisada): {}",
                    i.generator, i.asset.name
                ));
                if i.asks_admin {
                    out.push(crate::winget_install::plan::SELF_ADMIN_PROMPT.to_string());
                }
                if let Some(p) = &self.program {
                    out.push(format!("Se falhar: instala só {}", p.name));
                }
            }
            None => {
                if let Some(p) = &self.program {
                    out.push(format!("Instala {} (origem não revisada)", p.name));
                }
            }
        }
        out.push(format!("origem não revisada: {}", self.t.slug()));
        out.push(format!(
            "release imutável: {}",
            if self.release.immutable {
                "sim"
            } else {
                "não"
            }
        ));
        out
    }
}

/// The end of an upgrade run by [`run_upgrade`]: what to say, and a warning.
#[derive(Debug, Clone, PartialEq)]
pub struct UpgradeDone {
    pub summary: String,
    pub warning: Option<String>,
}

/// The installer did not register itself (the CLI says it in yellow).
pub const NOT_REGISTERED: &str = "o instalador não se registrou em Programas instalados; para remover, use o desinstalador do próprio programa";

/// The old program an installer had put in place stays when the app ends portable.
pub fn old_program_text(name: &str) -> String {
    format!(
        "a versão anterior, instalada pelo instalador, continua no Windows: {name} em Programas instalados"
    )
}

/// Runs a confirmed upgrade without a terminal (the TUI's job): the new installer, then, when
/// it fails, the program through the Scoop plan; the same steps as the CLI's. `None`: cancelled
/// before the installer started.
pub async fn run_upgrade(
    ctx: &OpsCtx<'_>,
    env: &GhEnv<'_>,
    up: &ReleaseUpgrade,
) -> anyhow::Result<Option<UpgradeDone>> {
    use crate::github::install::InstallerFailure;
    let rel = up.rel(env);
    let failure = match &up.installer {
        Some(inst) => match run_release_installer(ctx, &rel, inst).await? {
            Ok(done) => {
                return Ok(Some(UpgradeDone {
                    summary: format!("Atualizado: {} {}", up.app, up.version),
                    warning: done.arp.is_none().then(|| NOT_REGISTERED.to_string()),
                }));
            }
            Err(InstallerFailure::Cancelled) => return Ok(None),
            Err(e) => Some(e),
        },
        None => None,
    };
    let Some(program) = &up.program else {
        bail!(
            "o instalador falhou: {}",
            failure.map(|f| f.to_string()).unwrap_or_default()
        );
    };
    let fallback = async {
        let plan = release_plan(ctx, &rel, program).await?;
        let plans = std::slice::from_ref(&plan);
        let log = JobLog::start(JobInfo::for_plans("upgrade", plans));
        let applied = ops::apply(ctx, plans).await;
        finish_log(log, &applied, "atualizado");
        applied?;
        Ok::<_, anyhow::Error>(())
    };
    match (fallback.await, &failure) {
        (Ok(()), _) => {}
        (Err(e), Some(f)) => {
            return Err(e.context(format!(
                "o instalador falhou ({f}) e a instalação só do executável também falhou"
            )));
        }
        (Err(e), None) => return Err(e),
    }
    let mut warnings: Vec<String> = Vec::new();
    if let Some(f) = &failure {
        warnings.push(format!(
            "o instalador falhou ({f}); instalado só o executável"
        ));
    }
    if let Some(name) = old_program_left(ctx, &rel) {
        warnings.push(old_program_text(&name));
    }
    Ok(Some(UpgradeDone {
        summary: format!("Atualizado: {} {}", up.app, up.version),
        warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
    }))
}

// ---- removal (Task 211) ----

/// `uninstall` of an app whose installer did not register itself: `(what, what to do)`.
pub fn cannot_uninstall(app: &str) -> (String, String) {
    (
        format!(
            "o wingetrs não sabe desinstalar {app}: o instalador não se registrou em Programas instalados"
        ),
        format!(
            "use o desinstalador do próprio programa e depois `wingetrs uninstall {app} --forget`"
        ),
    )
}

/// The key of an installer app is gone: removed outside wingetrs.
pub fn key_gone_text(app: &str) -> String {
    format!(
        "{app} já foi removido por fora (sumiu de Programas instalados); o wingetrs só apaga o registro dele"
    )
}

/// The uninstaller ran but the key stayed.
pub fn key_stayed_text(app: &str) -> String {
    format!("{app} continua em Programas instalados; o registro do wingetrs foi mantido")
}

/// Deletes only the wingetrs record of a release app its installer put in place (the
/// `state.db` row and the app folder; it has no artifacts): the executor's removal under
/// `install.lock`, with its job log.
pub fn forget_release(ctx: &OpsCtx<'_>, app: &str, version: &str) -> Result<(), OpsError> {
    let plan = ops::plan_uninstall(ctx, app, false)?;
    let log = JobLog::start(JobInfo {
        kind: "remove",
        tool: "wingetrs",
        target: app.to_string(),
        name: app.to_string(),
        from: Some(version.to_string()),
        to: None,
        cmd: None,
    });
    let removed = ops::uninstall_with(ctx, &plan, None).map(|_| ());
    finish_log(log, &removed, "registro removido");
    removed
}

/// The details of a release app, read from its record only (no catalog, no network): its
/// repository, the file it came from, how it was put in place and its ARP key.
pub fn release_details(
    layout: &crate::install::layout::Layout,
    app: &str,
) -> anyhow::Result<crate::core::PackageDetails> {
    use crate::core::{InstallerSummary, PackageDetails, SourceKind};
    use crate::github::install as gh;
    let Some((rec, mode)) = release_record_in(layout, app)? else {
        bail!("{app} não vem de um release do GitHub");
    };
    let raw = crate::install::state::installed_manifest(
        layout,
        &rec.app,
        &rec.version,
        &rec.manifest_sha256,
    )
    .ok()
    .flatten();
    let url = raw
        .as_deref()
        .and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok())
        .and_then(|v| v.get("url")?.as_str().map(str::to_string))
        .unwrap_or_default();
    let file = raw.as_deref().and_then(gh::asset_of_manifest);
    let slug = rec.bucket.strip_prefix("github:").unwrap_or(&rec.bucket);
    let mut lines = vec![format!(
        "Release do GitHub de {slug} (origem não revisada)."
    )];
    if let Some(f) = &file {
        lines.push(format!("Arquivo: {f}."));
    }
    let kind = match &mode {
        Mode::Portable => "portátil".to_string(),
        Mode::Installer(Some(k)) => {
            lines.push(format!(
                "Chave em Programas instalados: {}\\{}.",
                crate::install::state::hive_name(k.hive),
                k.key
            ));
            "instalador".to_string()
        }
        Mode::Installer(None) => {
            lines.push("O instalador não se registrou em Programas instalados.".to_string());
            "instalador".to_string()
        }
    };
    Ok(PackageDetails {
        source: SourceKind::Scoop,
        id: rec.app.clone(),
        name: rec.app.clone(),
        version: rec.version.clone(),
        publisher: slug.split('/').next().map(str::to_string),
        description: Some(lines.join(" ")),
        homepage: Some(format!("{}/{slug}", gh::WEB_BASE)),
        license: None,
        tags: Vec::new(),
        installers: vec![InstallerSummary {
            architecture: rec.arch.clone().unwrap_or_else(|| "-".to_string()),
            installer_type: kind,
            scope: Some("usuário".to_string()),
            url,
        }],
        available_versions: Vec::new(),
        icon: None,
    })
}
