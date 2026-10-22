//! The elevated helper's own source for a native winget install (Task 202): it downloads
//! `source2.msix` into its run folder (administrator-only staging), has its signature verified
//! (`CatalogVerifier`; `catalog_verify_win` in production), checks the signer and the package
//! identity, finds the package's `versionData` through the signed index and the manifest
//! through it, each by its hash, and takes the installer's URL, SHA-256 and parameters from that
//! manifest. The installer is downloaded into the same run folder, checked and handed back to
//! run from there. Nothing about the installer comes from the request.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use super::worker::{ElevatedKind, PackageRequest, params_check};
use crate::core::version::Version;
use crate::http::{Http, HttpError};
use crate::sources::winget::manifest::MergedManifest;
use crate::sources::winget::{index, manifest, version_data};
use crate::winget_install::launch::{Elevation, install_elevation};
use crate::winget_install::plan::elevated_kind;
use crate::winget_install::switches::{self, LOG_TOKEN, Program};

/// The catalog's name on the CDN, and of its copy in the run folder.
pub const CATALOG_FILE: &str = "source2.msix";
/// The index taken out of the catalog, in the run folder.
const INDEX_FILE: &str = "index.db";
/// Largest catalog accepted (3.8 MB measured in 2026-10).
pub const MSIX_MAX: usize = 64 << 20;
/// Largest `versionData.mszyml` accepted (about 1 KB measured).
pub const VERSION_DATA_MAX: usize = 1 << 20;
/// Largest manifest accepted (about 5 KB measured).
pub const MANIFEST_MAX: usize = 1 << 20;
/// Largest `AppxManifest.xml` read from the catalog.
const APPX_MANIFEST_MAX: u64 = 1 << 20;
/// Oldest signing certificate accepted. The catalog is signed again at each publication with a
/// certificate valid for 3 days; its timestamp keeps an old signature valid forever, so without
/// this floor an old catalog (listing an old, maybe vulnerable, version) would be accepted.
pub const CATALOG_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// `Identity Name` of the catalog package.
pub const SOURCE_IDENTITY: &str = "Microsoft.Winget.Source";
/// Common name and organization of the catalog's signer.
pub const SIGNER_NAME: &str = "Microsoft Corporation";

/// Progress line: downloading and checking the signed catalog.
pub const CHECKING_CATALOG: &str = "auxiliar elevado: conferindo o catálogo assinado do winget";
/// Progress line: downloading and checking the installer.
pub const DOWNLOADING_INSTALLER: &str = "auxiliar elevado: baixando e conferindo o instalador";
/// Progress line: the installer runs.
pub const INSTALLER_RUNNING: &str = "auxiliar elevado: instalador rodando";

/// The signer of a verified catalog, as the verifier read it from the certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    pub common_name: String,
    pub organization: String,
    pub not_before: SystemTime,
}

/// Verifies the signature of the catalog file at `msix` (the whole package, its chain to a
/// Microsoft root) and returns its signer; the reason (Portuguese) otherwise.
pub trait CatalogVerifier {
    fn verify(&self, msix: &Path) -> Result<Signer, String>;
}

/// What `prepare` needs from the helper.
pub struct SignedCtx<'a> {
    /// `Http::for_elevated` in production: no user proxy, only the machine's roots.
    pub http: &'a Http,
    /// `sources::winget::CDN_BASE` in production.
    pub cdn_base: &'a str,
    pub verifier: &'a dyn CatalogVerifier,
    pub now: SystemTime,
    /// Folders an install folder parameter may point into (`SystemEnv::program_files_roots`).
    pub program_files: &'a [String],
    /// `System32\msiexec.exe` from the known folder.
    pub msiexec: &'a Path,
    /// Plain `http://` installer URLs accepted (only the tests' local server serves them).
    pub allow_http: bool,
}

/// The installer ready to run: `program params` from `dir`, the run folder holding the checked
/// `installer` (the program itself, or the package `msiexec` installs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub program: PathBuf,
    pub params: String,
    pub dir: PathBuf,
    pub installer: PathBuf,
    pub kind: ElevatedKind,
}

#[derive(Debug, thiserror::Error)]
pub enum SignedError {
    #[error("pedido recusado: {0}")]
    Request(String),
    #[error("catálogo assinado inválido: {0}")]
    Catalog(String),
    #[error("assinatura do catálogo recusada: {0}")]
    Signature(String),
    #[error("{0} não está no catálogo assinado do winget")]
    NotInCatalog(String),
    #[error("a versão {0} não está no catálogo assinado do winget (atualize o catálogo)")]
    VersionMissing(String),
    #[error("integridade: {0}")]
    Integrity(&'static str),
    #[error("manifesto do catálogo assinado: {0}")]
    Manifest(String),
    #[error("instalador recusado para elevação: {0}")]
    Refused(String),
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("E/S: {0}")]
    Io(String),
}

fn io(e: std::io::Error) -> SignedError {
    SignedError::Io(e.to_string())
}

/// Writes `bytes` to the new file `path` (an existing one is an error).
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), SignedError> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    f.write_all(bytes).map_err(io)?;
    f.sync_all().map_err(io)
}

/// The signer must be Microsoft's, and its certificate at most `CATALOG_MAX_AGE` old at `now`
/// (a certificate dated after `now` passes: only a clock behind would do that).
pub fn check_signer(s: &Signer, now: SystemTime) -> Result<(), SignedError> {
    if s.common_name != SIGNER_NAME || s.organization != SIGNER_NAME {
        return Err(SignedError::Signature(format!(
            "signatário {} ({})",
            s.common_name, s.organization
        )));
    }
    if now
        .duration_since(s.not_before)
        .is_ok_and(|age| age > CATALOG_MAX_AGE)
    {
        return Err(SignedError::Signature(format!(
            "catálogo assinado há mais de {} dias",
            CATALOG_MAX_AGE.as_secs() / 86_400
        )));
    }
    Ok(())
}

/// The package identity in the catalog's `AppxManifest.xml` must be `SOURCE_IDENTITY`.
pub fn check_identity(msix: &[u8]) -> Result<(), SignedError> {
    let bad = |e: String| SignedError::Catalog(e);
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(msix)).map_err(|e| bad(e.to_string()))?;
    let entry = zip
        .by_name("AppxManifest.xml")
        .map_err(|e| bad(format!("AppxManifest.xml: {e}")))?;
    let mut xml = String::new();
    entry
        .take(APPX_MANIFEST_MAX + 1)
        .read_to_string(&mut xml)
        .map_err(|e| bad(format!("AppxManifest.xml: {e}")))?;
    if xml.len() as u64 > APPX_MANIFEST_MAX {
        return Err(bad("AppxManifest.xml grande demais".into()));
    }
    let doc = roxmltree::Document::parse(&xml).map_err(|e| bad(e.to_string()))?;
    let root = doc.root_element();
    let name = root
        .has_tag_name("Package")
        .then(|| root.children().find(|n| n.has_tag_name("Identity")))
        .flatten()
        .and_then(|n| n.attribute("Name"));
    if name != Some(SOURCE_IDENTITY) {
        return Err(SignedError::Signature(format!(
            "identidade do pacote {}",
            name.unwrap_or("ausente")
        )));
    }
    Ok(())
}

/// The `packages.hash` (the SHA-256 of its `versionData`) of `id` in the v2 index at `db`.
fn package_hash(db: &Path, id: &str) -> Result<Vec<u8>, SignedError> {
    use rusqlite::OptionalExtension;
    let sql = |e: rusqlite::Error| SignedError::Catalog(e.to_string());
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(sql)?;
    let major: rusqlite::types::Value = c
        .query_row(
            "select value from metadata where name='majorVersion'",
            [],
            |r| r.get(0),
        )
        .map_err(sql)?;
    let major = match major {
        rusqlite::types::Value::Text(t) => t,
        rusqlite::types::Value::Integer(i) => i.to_string(),
        _ => String::new(),
    };
    if major != "2" {
        return Err(SignedError::Catalog(format!("índice v{major}")));
    }
    let hash: Option<Option<Vec<u8>>> = c
        .query_row("select hash from packages where id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(sql)?;
    hash.flatten()
        .filter(|h| h.len() == 32)
        .ok_or_else(|| SignedError::NotInCatalog(id.to_string()))
}

/// Downloads the catalog into `run_dir`, verifies it (signature, signer, age, identity, the
/// index against the block map) and returns the hash of `id`'s `versionData`. The catalog and
/// the index are removed from `run_dir` before returning.
async fn catalog_hash(
    ctx: &SignedCtx<'_>,
    id: &str,
    run_dir: &Path,
) -> Result<Vec<u8>, SignedError> {
    let msix = run_dir.join(CATALOG_FILE);
    let db = run_dir.join(INDEX_FILE);
    let body = ctx
        .http
        .get_limited(&format!("{}{CATALOG_FILE}", ctx.cdn_base), MSIX_MAX)
        .await?;
    write_new(&msix, &body)?;
    drop(body);
    let checked = (|| {
        let signer = ctx.verifier.verify(&msix).map_err(SignedError::Signature)?;
        check_signer(&signer, ctx.now)?;
        // What the verifier checked, read back from the administrator-only folder.
        let bytes = std::fs::read(&msix).map_err(io)?;
        check_identity(&bytes)?;
        let index =
            index::validate_msix(&bytes).map_err(|e| SignedError::Catalog(e.to_string()))?;
        drop(bytes);
        write_new(&db, &index)?;
        package_hash(&db, id)
    })();
    for f in [&msix, &db] {
        if let Err(e) = std::fs::remove_file(f)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(error = %e, file = %f.display(), "signed install: catalog file left in the run folder");
        }
    }
    checked
}

/// A `rP` of `versionData`: a relative URL path of non-empty components, no `.`/`..`, no
/// backslash, colon, query, fragment, escape, space or control character.
fn valid_rel_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 1024
        && !p.starts_with('/')
        && p.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
        && !p.chars().any(|c| {
            c.is_control() || c.is_whitespace() || matches!(c, '\\' | ':' | '?' | '#' | '%' | '@')
        })
}

/// The manifest of `id` `version` reached from the signed `hash`: `versionData` checked against
/// it, the version entry found by version (never by text alone), the manifest checked against
/// the entry's SHA-256 and naming the same package and version.
async fn signed_manifest(
    ctx: &SignedCtx<'_>,
    id: &str,
    version: &str,
    hash: &[u8],
) -> Result<MergedManifest, SignedError> {
    let url = format!(
        "{}{}",
        ctx.cdn_base,
        version_data::version_data_path(id, hash)
    );
    let vd = ctx.http.get_limited(&url, VERSION_DATA_MAX).await?;
    if Sha256::digest(&vd).as_slice() != hash {
        return Err(SignedError::Integrity(
            "versionData não confere com o catálogo assinado",
        ));
    }
    let vd =
        version_data::decode_and_parse(&vd).map_err(|e| SignedError::Manifest(e.to_string()))?;
    let wanted = Version::parse(version);
    let equal: Vec<_> = vd
        .entries
        .iter()
        .filter(|e| Version::parse(&e.version) == wanted)
        .collect();
    // Among equal versions (`1.0` and `1.0.0`), the one written as asked.
    let entry = equal
        .iter()
        .find(|e| e.version == version)
        .or(equal.first())
        .ok_or_else(|| SignedError::VersionMissing(version.to_string()))?;
    if !valid_rel_path(&entry.rel_path) {
        return Err(SignedError::Manifest(
            "caminho do manifesto inválido".into(),
        ));
    }
    let bytes = ctx
        .http
        .get_limited(&format!("{}{}", ctx.cdn_base, entry.rel_path), MANIFEST_MAX)
        .await?;
    if !hex::encode(Sha256::digest(&bytes)).eq_ignore_ascii_case(entry.sha256.trim()) {
        return Err(SignedError::Integrity(
            "manifesto não confere com o catálogo assinado",
        ));
    }
    let m = manifest::parse(&bytes).map_err(|e| SignedError::Manifest(e.to_string()))?;
    if !m.package_identifier.eq_ignore_ascii_case(id)
        || Version::parse(&m.package_version) != wanted
    {
        return Err(SignedError::Manifest(
            "o manifesto é de outro pacote ou versão".into(),
        ));
    }
    Ok(m)
}

/// What runs for `req` with the manifest `m`: the installer at `req.installer`, which must be
/// one that runs elevated; its command line built like the plan's (`install_args_unlogged`) and
/// passing the elevation allowlist; its URL (https) and SHA-256 (64 hex digits).
fn installer_of(
    ctx: &SignedCtx<'_>,
    m: &MergedManifest,
    req: &PackageRequest,
) -> Result<(ElevatedKind, switches::CommandLine, reqwest::Url, String), SignedError> {
    let installers = m.effective();
    let inst = usize::try_from(req.installer)
        .ok()
        .and_then(|i| installers.get(i))
        .ok_or_else(|| SignedError::Refused("instalador inexistente no manifesto".into()))?;
    if install_elevation(&inst.kind, inst.scope.as_deref(), inst.elevation.as_deref())
        != Elevation::Helper
    {
        return Err(SignedError::Refused(
            "o instalador não roda como administrador".into(),
        ));
    }
    let kind = elevated_kind(&inst.kind)
        .ok_or_else(|| SignedError::Refused(format!("tipo {}", inst.kind.name())))?;
    let cmd =
        switches::install_args_unlogged(&inst.kind, &inst.switches, req.mode.into(), req.upgrade)
            .map_err(|e| SignedError::Refused(e.to_string()))?;
    if cmd.args.contains(LOG_TOKEN) {
        return Err(SignedError::Refused("log nos parâmetros".into()));
    }
    params_check(kind, &cmd.args, ctx.program_files)
        .map_err(|r| SignedError::Refused(r.to_string()))?;
    let sha = inst.sha256.trim().to_ascii_lowercase();
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(SignedError::Manifest(
            "SHA-256 do instalador inválido".into(),
        ));
    }
    let url = reqwest::Url::parse(inst.url.trim())
        .map_err(|e| SignedError::Manifest(format!("URL do instalador: {e}")))?;
    let scheme_ok = url.scheme() == "https" || (ctx.allow_http && url.scheme() == "http");
    if !scheme_ok {
        return Err(SignedError::Refused("URL do instalador sem https".into()));
    }
    Ok((kind, cmd, url, sha))
}

/// SHA-256 of the file at `path`, lowercase hex.
fn file_sha256(path: &Path) -> Result<String, SignedError> {
    let mut f = std::fs::File::open(path).map_err(io)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(io)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Everything before the run, for a request `serve` already checked: the signed catalog, the
/// manifest, the installer chosen by `req.installer` and its command line, the installer
/// downloaded into `run_dir` (a fresh administrator-only folder) and checked against the
/// manifest's SHA-256. Progress lines go to `on_line`.
pub async fn prepare(
    ctx: &SignedCtx<'_>,
    req: &PackageRequest,
    run_dir: &Path,
    on_line: &mut dyn FnMut(&str),
) -> Result<Staged, SignedError> {
    req.check()
        .map_err(|r| SignedError::Request(r.to_string()))?;
    on_line(CHECKING_CATALOG);
    let hash = catalog_hash(ctx, &req.id, run_dir).await?;
    let m = signed_manifest(ctx, &req.id, &req.version, &hash).await?;
    let (kind, cmd, url, sha) = installer_of(ctx, &m, req)?;
    on_line(DOWNLOADING_INSTALLER);
    let installer = run_dir.join(if kind.is_msi() {
        "installer.msi"
    } else {
        "installer.exe"
    });
    let http = ctx.http.with_strict_download_redirects()?;
    let complete = http
        .get_to_file_until(url.as_str(), &installer, |_, _| {}, &|| false)
        .await?;
    if !complete {
        return Err(SignedError::Io("download interrompido".into()));
    }
    if file_sha256(&installer)? != sha {
        if let Err(e) = std::fs::remove_file(&installer) {
            tracing::warn!(error = %e, "signed install: mismatched installer left in the run folder");
        }
        return Err(SignedError::Integrity(
            "o instalador baixado não confere com o SHA-256 do manifesto",
        ));
    }
    let (program, params) = match cmd.program {
        Program::Msiexec => (
            ctx.msiexec.to_path_buf(),
            switches::msiexec_params(&installer, &cmd.args),
        ),
        Program::Installer => (installer.clone(), cmd.args),
    };
    switches::validate(&params).map_err(|e| SignedError::Refused(e.to_string()))?;
    Ok(Staged {
        program,
        params,
        dir: run_dir.to_path_buf(),
        installer,
        kind,
    })
}
