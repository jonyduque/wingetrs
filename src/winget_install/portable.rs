//! Native `portable` and zip-with-portable packages (Task 189): the installer becomes a wingetrs
//! app of phase 2a (`apps\<id>\<version>`, a shim per command, the user PATH, `state.db`).
//!
//! winget keeps them under `%LOCALAPPDATA%\Microsoft\WinGet\Packages` with an HKCU ARP entry
//! (PortableFlow.cpp); wingetrs writes neither: its own root and its own app record.

use super::plan::{PlanOrBridge, plan_with_mode};
use super::select::{Host, Refusal, Scope, ScopeWanted};
use crate::config::InstallerMode;
use crate::install::ops::{OpsCtx, OpsError, plan_manifest};
use crate::install::plan::InstallPlan;
use crate::install::safepath::safe_components;
use crate::sources::winget::manifest::{
    Agreement, EffectiveInstaller, InstallerKind, MergedManifest,
};

/// Bucket recorded for these apps (`state.db`, `install.json`): their manifest is the winget
/// catalog's, never a scoop bucket's.
pub const WINGET_BUCKET: &str = "winget";

/// Most `NestedInstallerFiles` accepted (the catalog of 2026-10-18 has at most a handful).
const MAX_NESTED_FILES: usize = 64;

/// `portable`, or `zip` whose nested installer is `portable`.
pub fn is_portable(i: &EffectiveInstaller) -> bool {
    match i.kind {
        InstallerKind::Portable => true,
        InstallerKind::Zip => i.nested_kind == Some(InstallerKind::Portable),
        _ => false,
    }
}

/// A portable package ready to plan as a wingetrs app.
#[derive(Debug, Clone, PartialEq)]
pub struct PortableSpec {
    pub id: String,
    pub name: String,
    pub version: String,
    /// The installer as a scoop-format manifest (download, hash, `bin`): planned by the phase 2a
    /// planner and written as the app's `manifest.json`.
    pub manifest: String,
    /// The manifest's license agreements (shown before the confirmation).
    pub agreements: Vec<Agreement>,
}

#[derive(Debug, thiserror::Error)]
pub enum PortableError {
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Manifest(String),
    #[error(transparent)]
    Ops(#[from] OpsError),
}

fn one_component(name: &str) -> bool {
    safe_components(name).is_some_and(|c| c.len() == 1 && c[0] == name)
}

/// A file name that needs no quoting anywhere: ASCII letters, digits and `. _ - +`.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c))
        && one_component(name)
}

/// A command alias: one safe component that can name a shim.
fn alias_of(raw: Option<&str>) -> Result<Option<String>, Refusal> {
    let Some(a) = raw.map(str::trim).filter(|a| !a.is_empty()) else {
        return Ok(None);
    };
    if !one_component(a) || !crate::install::shim::is_safe_shim_name(a) {
        return Err(Refusal(format!(
            "apelido de comando inválido no manifesto: {a}"
        )));
    }
    Ok(Some(a.to_string()))
}

/// Name of the downloaded `.exe` of a plain `portable`: the URL's own file name when it is a
/// plain `.exe` name, else `<alias>.exe`, else `<id>.exe`.
fn exe_name(url: &str, alias: Option<&str>, id: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or("");
    if plain_name(last) && last.to_ascii_lowercase().ends_with(".exe") {
        return last.to_string();
    }
    format!("{}.exe", alias.unwrap_or(id))
}

/// `bin` entry of the scoop manifest: the path alone, or `[path, alias]`.
fn bin_entry(path: &str, alias: Option<String>) -> serde_json::Value {
    match alias {
        Some(a) => serde_json::json!([path, a]),
        None => serde_json::json!(path),
    }
}

/// Builds the app of a portable installer `i` of `m`; every name and path that ends up joined
/// to a folder is checked here (and again by the 2a planner).
pub fn spec(m: &MergedManifest, i: &EffectiveInstaller) -> Result<PortableSpec, Refusal> {
    let (id, version) = (&m.package_identifier, &m.package_version);
    if !one_component(id) {
        return Err(Refusal(format!(
            "identificador inválido para uma pasta: {id}"
        )));
    }
    if !one_component(version) {
        return Err(Refusal(format!(
            "versão inválida para uma pasta: {version}"
        )));
    }
    if i.url.contains('#') || !super::fetch::scheme_allowed(&i.url) {
        return Err(Refusal(format!("URL do instalador recusada: {}", i.url)));
    }
    if i.sha256.len() != 64 || !i.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Refusal("InstallerSha256 ausente ou inválido".to_string()));
    }
    let (file, bins) = if i.kind == InstallerKind::Portable {
        let alias = alias_of(i.commands.first().map(String::as_str))?;
        let exe = exe_name(&i.url, alias.as_deref(), id);
        (exe.clone(), vec![bin_entry(&exe, alias)])
    } else {
        if i.nested_files.is_empty() {
            return Err(Refusal(
                "zip portable sem NestedInstallerFiles no manifesto".to_string(),
            ));
        }
        if i.nested_files.len() > MAX_NESTED_FILES {
            return Err(Refusal(format!(
                "zip portable com {} arquivos aninhados (máximo {MAX_NESTED_FILES})",
                i.nested_files.len()
            )));
        }
        let mut bins = Vec::with_capacity(i.nested_files.len());
        for f in &i.nested_files {
            let parts = safe_components(&f.relative_file_path)
                .filter(|p| !p.is_empty())
                .ok_or_else(|| {
                    Refusal(format!(
                        "caminho fora do pacote no manifesto: {}",
                        f.relative_file_path
                    ))
                })?;
            let alias = alias_of(f.portable_command_alias.as_deref())?;
            bins.push(bin_entry(&parts.join("\\"), alias));
        }
        (format!("{id}.zip"), bins)
    };
    let name = m
        .package_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(id)
        .to_string();
    let mut manifest = serde_json::json!({
        "version": version,
        "description": name,
        "url": format!("{}#/{file}", i.url),
        "hash": i.sha256.to_ascii_lowercase(),
        "bin": bins,
    });
    if let Some(home) = &m.package_url {
        manifest["homepage"] = serde_json::json!(home);
    }
    Ok(PortableSpec {
        id: id.clone(),
        name,
        version: version.clone(),
        manifest: manifest.to_string(),
        agreements: m.agreements.clone(),
    })
}

/// The 2a install plan of `spec` against the current root and `state.db` (an upgrade when an
/// older version of it is installed).
pub fn install_plan(
    ctx: &OpsCtx<'_>,
    spec: &PortableSpec,
    keep_old: bool,
) -> Result<InstallPlan, OpsError> {
    plan_manifest(ctx, &spec.id, WINGET_BUCKET, &spec.manifest, keep_old)
}

/// Plans installing or upgrading the winget catalog's version of portable package `id`: reads
/// its manifest (the cached blob, else the CDN at `cdn_base`) and plans it for `host`.
pub async fn plan_from_catalog(
    ctx: &OpsCtx<'_>,
    cdn_base: &str,
    host: &Host,
    id: &str,
    keep_old: bool,
) -> Result<InstallPlan, PortableError> {
    let pkg = ctx
        .catalog
        .get("winget", id)
        .map_err(|e| PortableError::Manifest(e.to_string()))?
        .ok_or_else(|| PortableError::Manifest(format!("{id} não está no catálogo do winget")))?;
    let hash = pkg.winget_hash.as_deref().ok_or_else(|| {
        PortableError::Manifest(format!("{id}: pacote winget sem hash de manifesto"))
    })?;
    let blobs = crate::cache::BlobStore::new(ctx.data.blobs_dir());
    let (m, _) = crate::sources::winget::details::merged_manifest(
        ctx.http,
        &blobs,
        cdn_base,
        &pkg.entry.id,
        hash,
        Some(&pkg.entry.version),
    )
    .await
    .map_err(|e| PortableError::Manifest(e.to_string()))?;
    match plan_with_mode(
        &m,
        host,
        ScopeWanted::Prefer(Scope::User),
        None,
        InstallerMode::default(),
    ) {
        Ok(PlanOrBridge::Portable(spec)) => Ok(install_plan(ctx, &spec, keep_old)?),
        Ok(_) => Err(PortableError::Refused(format!(
            "{id} {} não é mais um pacote portable: remova e instale de novo",
            m.package_version
        ))),
        Err(Refusal(why)) => Err(PortableError::Refused(why)),
    }
}
