//! Install from a GitHub release (Task 210): the `gh:` target and its URL forms, the bucket
//! manifests that already download from the repository, the synthetic Scoop manifest of a
//! portable asset, and the run of an installer asset (silent switches by generator, the ARP
//! read before and after, the record of the new key).
use std::path::{Path, PathBuf};

use crate::catalog::{Catalog, CatalogError};
use crate::github::{Asset, GhError, GhRepo, percent_decode, valid_part};
use crate::install::archive::ArchiveKind;
use crate::install::executor::LaunchGate;
use crate::install::layout::Layout;
use crate::install::progress::{Phase, ProgressEvent};
use crate::install::safepath::safe_components;
use crate::install::sniff::{self, PeKind, Sniffed, classify_pe};
use crate::install::state::{self, AppRecord, InstallerRecord, StateDb, StateError};
use crate::installed::Hive;
use crate::winget_install::launch::{LaunchSpec, Launcher, Show, Verb};

/// The GitHub REST API.
pub const API_BASE: &str = "https://api.github.com";
/// Where release assets are downloaded from.
pub const WEB_BASE: &str = "https://github.com";

/// Inno Setup silent switches (closed list, never from the release).
pub const INNO_ARGS: &str = "/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP-";
/// NSIS silent switch.
pub const NSIS_ARGS: &str = "/S";

/// `ERROR_SUCCESS_REBOOT_REQUIRED`: installed, a restart finishes it.
pub const REBOOT_CODE: i32 = 3010;
/// `ERROR_CANCELLED`: the user refused the installer's own UAC prompt.
const UAC_REFUSED: i32 = 1223;

/// `gh:<owner>/<repo>[@<tag>]`, from that form or from a `https://github.com/...` address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhTarget {
    pub repo: GhRepo,
    pub tag: Option<String>,
}

impl GhTarget {
    /// The bucket of the app in `state.db`: `github:<owner>/<repo>`.
    pub fn bucket(&self) -> String {
        format!("github:{}/{}", self.repo.owner, self.repo.repo)
    }

    /// `<owner>/<repo>`.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.repo.owner, self.repo.repo)
    }
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 128
        && !tag.contains(['/', '\\', '%'])
        && !tag.contains("..")
        && !tag.chars().any(char::is_control)
}

fn target(owner: &str, repo: &str, tag: Option<String>) -> Result<GhTarget, GhError> {
    if !valid_part(owner) || !valid_part(repo) {
        return Err(GhError::InvalidRepo(format!("{owner}/{repo}")));
    }
    if let Some(t) = &tag
        && !valid_tag(t)
    {
        return Err(GhError::InvalidTag(t.clone()));
    }
    Ok(GhTarget {
        repo: GhRepo {
            owner: owner.to_string(),
            repo: repo.to_string(),
        },
        tag,
    })
}

/// `Some` when `input` names a GitHub release: `gh:<owner>/<repo>[@<tag>]`,
/// `https://github.com/<owner>/<repo>` (trailing `/`, `.git` and `www.` allowed) or
/// `https://github.com/<owner>/<repo>/releases/tag/<tag>`. Any other GitHub address is an
/// error; anything else is `None` (a catalog name).
pub fn parse_target(input: &str) -> Option<Result<GhTarget, GhError>> {
    let s = input.trim();
    if let Some(rest) = strip_prefix_ci(s, "gh:") {
        let (spec, tag) = match rest.split_once('@') {
            Some((spec, tag)) => (spec, Some(tag.to_string())),
            None => (rest, None),
        };
        return Some(match spec.split_once('/') {
            Some((owner, repo)) => target(owner, repo, tag),
            None => Err(GhError::InvalidRepo(spec.to_string())),
        });
    }
    let rest = strip_prefix_ci(s, "https://").or_else(|| strip_prefix_ci(s, "http://"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let unsupported = || Err(GhError::UnsupportedUrl(s.to_string()));
    // `user@github.com` or `github.com:443`: refused, never read as plain GitHub.
    let bare = host.rsplit('@').next().unwrap_or(host);
    let bare = bare.split(':').next().unwrap_or(bare);
    let bare = strip_prefix_ci(bare, "www.").unwrap_or(bare);
    if !bare.eq_ignore_ascii_case("github.com") {
        return None;
    }
    if host.contains(['@', ':']) {
        return Some(unsupported());
    }
    if strip_prefix_ci(s, "http://").is_some() || path.contains(['?', '#']) {
        return Some(unsupported());
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    let parts: Vec<&str> = path.split('/').collect();
    Some(match parts.as_slice() {
        [owner, repo] => {
            let repo = repo.strip_suffix(".git").unwrap_or(repo);
            target(owner, repo, None)
        }
        [owner, repo, "releases", "tag", tag] => match percent_decode(tag) {
            Some(tag) => target(owner, repo, Some(tag)),
            None => Err(GhError::InvalidTag((*tag).to_string())),
        },
        _ => unsupported(),
    })
}

/// The app name of `repo`: its name in lowercase, one safe path component.
pub fn app_name(repo: &GhRepo) -> Result<String, GhError> {
    let name = repo.repo.to_ascii_lowercase();
    match safe_components(&name) {
        Some(c) if c.len() == 1 && c[0] == name => Ok(name),
        _ => Err(GhError::InvalidRepo(repo.repo.clone())),
    }
}

/// The version of a release: its tag without a leading `v`, one safe path component.
pub fn version_of(tag: &str) -> Result<String, GhError> {
    let v = match tag.strip_prefix(['v', 'V']) {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
        _ => tag,
    };
    match safe_components(v) {
        Some(c) if c.len() == 1 && c[0] == v => Ok(v.to_string()),
        _ => Err(GhError::InvalidTag(tag.to_string())),
    }
}

/// Every download URL of a Scoop manifest (`url` and `architecture.*.url`, string or list).
fn manifest_urls(v: &serde_json::Value) -> Vec<&str> {
    fn push<'a>(u: Option<&'a serde_json::Value>, out: &mut Vec<&'a str>) {
        match u {
            Some(serde_json::Value::String(s)) => out.push(s),
            Some(serde_json::Value::Array(a)) => out.extend(a.iter().filter_map(|x| x.as_str())),
            _ => {}
        }
    }
    let mut out = Vec::new();
    push(v.get("url"), &mut out);
    if let Some(archs) = v.get("architecture").and_then(|a| a.as_object()) {
        for arch in archs.values() {
            push(arch.get("url"), &mut out);
        }
    }
    out
}

/// The Scoop apps of the catalog that download from `repo`'s releases: `(bucket, app)`.
pub fn bucket_manifests(
    catalog: &Catalog,
    repo: &GhRepo,
) -> Result<Vec<(String, String)>, CatalogError> {
    let prefix =
        format!("{WEB_BASE}/{}/{}/releases/download/", repo.owner, repo.repo).to_ascii_lowercase();
    let mut out = Vec::new();
    for p in catalog.scoop_raw_containing(&prefix)? {
        let Some(v) = p
            .raw
            .as_deref()
            .and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok())
        else {
            continue;
        };
        let hit = manifest_urls(&v)
            .iter()
            .any(|u| u.to_ascii_lowercase().starts_with(&prefix));
        if let (true, Some(bucket)) = (hit, p.entry.source_name.strip_prefix("scoop:")) {
            out.push((bucket.to_string(), p.entry.id.clone()));
        }
    }
    Ok(out)
}

/// Where `asset` is downloaded from: its URL with `https://github.com` replaced by `web`
/// (production passes [`WEB_BASE`]; tests a local server).
pub fn download_url(web: &str, asset: &Asset) -> String {
    match asset.url.strip_prefix(WEB_BASE) {
        Some(rest) => format!("{}{rest}", web.trim_end_matches('/')),
        None => asset.url.clone(),
    }
}

/// `sha256:<hex>` of `asset` (the API's `digest`).
pub fn digest(asset: &Asset) -> String {
    format!("sha256:{}", hex::encode(asset.sha256))
}

/// The name of `asset` as one safe path component (a release names its own files).
pub fn safe_name(asset: &Asset) -> Result<&str, InspectError> {
    match safe_components(&asset.name) {
        Some(c) if c.len() == 1 && c[0] == asset.name => Ok(&asset.name),
        _ => Err(InspectError::UnsafeName(asset.name.clone())),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error("nome de arquivo inseguro no release: {0}")]
    UnsafeName(String),
    #[error("instalador {0}: fora do escopo da instalação portátil")]
    Installer(&'static str),
    #[error("{0}: formato não suportado na instalação por release")]
    Unsupported(String),
    #[error("nenhum programa .exe no topo de {0}")]
    NoProgram(String),
    #[error("{0}")]
    Pe(String),
    #[error("{0}")]
    Extract(String),
    #[error("E/S: {0}")]
    Io(String),
}

fn io(e: std::io::Error) -> InspectError {
    InspectError::Io(e.to_string())
}

/// What a portable asset holds: the final file name when the URL needs `#/<name>`, the only
/// top folder to strip, and the programs (`bin`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portable {
    pub rename: Option<String>,
    pub extract_dir: Option<String>,
    pub bin: Vec<String>,
}

fn is_exe(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".exe")
}

/// Program or installer, from the bytes of a `.exe`.
pub fn exe_kind(bytes: &[u8]) -> Result<PeKind, InspectError> {
    classify_pe(bytes).map_err(|e| InspectError::Pe(e.to_string()))
}

/// Whether the PE's manifest asks for an administrator (`requireAdministrator` or
/// `highestAvailable`): the installer opens its own UAC prompt. Only the image is read (the
/// overlay is the installer's payload).
pub fn asks_admin(bytes: &[u8]) -> bool {
    let end = match sniff::pe_overlay_start(bytes) {
        0 => bytes.len(),
        n => usize::try_from(n).map_or(bytes.len(), |n| n.min(bytes.len())),
    };
    let image = &bytes[..end];
    [b"requireAdministrator".as_slice(), b"highestAvailable"]
        .iter()
        .any(|m| image.windows(m.len()).any(|w| w == *m))
}

/// Looks into the verified download `file` of asset `name`: a `.exe` program is its own
/// `bin`; an archive is extracted into `scratch` (an empty folder the caller removes) and the
/// `.exe` programs at its top, inside its only folder when it has one, are the `bin`.
pub fn inspect(file: &Path, name: &str, scratch: &Path) -> Result<Portable, InspectError> {
    let bytes_head = {
        use std::io::Read;
        let mut head = Vec::with_capacity(64 * 1024);
        std::fs::File::open(file)
            .map_err(io)?
            .take(64 * 1024)
            .read_to_end(&mut head)
            .map_err(io)?;
        head
    };
    let sniffed = sniff::sniff(&bytes_head);
    let known = ArchiveKind::of(name);
    if is_exe(name) || (sniffed == Sniffed::Pe && known == Ok(ArchiveKind::Plain)) {
        let bytes = std::fs::read(file).map_err(io)?;
        if let PeKind::Installer(g) = exe_kind(&bytes)? {
            return Err(InspectError::Installer(g));
        }
        let rename = (!is_exe(name)).then(|| format!("{name}.exe"));
        let bin = rename.clone().unwrap_or_else(|| name.to_string());
        return Ok(Portable {
            rename,
            extract_dir: None,
            bin: vec![bin],
        });
    }
    let rename = match known {
        Ok(ArchiveKind::Plain) if sniffed == Sniffed::Zip => Some(format!("{name}.zip")),
        Ok(ArchiveKind::Zip | ArchiveKind::TarGz | ArchiveKind::SevenZip | ArchiveKind::TarXz) => {
            None
        }
        _ => return Err(InspectError::Unsupported(name.to_string())),
    };
    let file_name = rename.as_deref().unwrap_or(name);
    crate::install::extract::extract(file, file_name, None, None, scratch)
        .map_err(|e| InspectError::Extract(e.to_string()))?;
    let top = entries(scratch)?;
    let (extract_dir, base) = match top.as_slice() {
        [(n, true)] => (Some(n.clone()), scratch.join(n)),
        _ => (None, scratch.to_path_buf()),
    };
    let mut bin = Vec::new();
    for (n, dir) in entries(&base)? {
        if dir || !is_exe(&n) {
            continue;
        }
        let bytes = std::fs::read(base.join(&n)).map_err(io)?;
        if exe_kind(&bytes)? == PeKind::Program {
            bin.push(n);
        }
    }
    if bin.is_empty() {
        return Err(InspectError::NoProgram(name.to_string()));
    }
    Ok(Portable {
        rename,
        extract_dir,
        bin,
    })
}

/// The UTF-8 names in `dir` with whether each is a folder, sorted.
fn entries(dir: &Path) -> Result<Vec<(String, bool)>, InspectError> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).map_err(io)? {
        let e = e.map_err(io)?;
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        out.push((name, e.file_type().map_err(io)?.is_dir()));
    }
    out.sort();
    Ok(out)
}

/// The synthetic Scoop manifest of a portable asset (written as the app's `manifest.json`).
pub fn manifest_json(url: &str, asset: &Asset, p: &Portable, version: &str) -> String {
    let url = match &p.rename {
        Some(n) => format!("{url}#/{n}"),
        None => url.to_string(),
    };
    let mut m = serde_json::json!({
        "version": version,
        "url": url,
        "hash": digest(asset),
        "bin": p.bin,
    });
    if let (Some(dir), Some(obj)) = (&p.extract_dir, m.as_object_mut()) {
        obj.insert("extract_dir".into(), dir.clone().into());
    }
    m.to_string()
}

/// The manifest recorded for an app its own installer put in place.
pub fn installer_manifest_json(url: &str, asset: &Asset, version: &str) -> String {
    serde_json::json!({
        "version": version,
        "url": url,
        "hash": digest(asset),
    })
    .to_string()
}

/// One ARP key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArpKey {
    pub hive: Hive,
    pub key: String,
    pub name: String,
    /// `DisplayVersion`.
    pub version: Option<String>,
}

/// The ARP (HKCU and HKLM 64/32-bit) as the installer flow reads it. Production:
/// `installed::arp_win`; tests: an in-memory list.
pub trait ArpView: Send + Sync {
    /// Every key now.
    fn keys(&self) -> std::io::Result<Vec<ArpKey>>;
    /// `UninstallString` of one key.
    fn uninstall_string(&self, hive: Hive, key: &str) -> Option<String>;
}

/// What a GitHub release install needs from the outside world.
pub struct GhEnv<'a> {
    /// [`API_BASE`] in production.
    pub api: String,
    /// [`WEB_BASE`] in production.
    pub web: String,
    pub launcher: &'a dyn Launcher,
    pub arp: &'a dyn ArpView,
}

/// The silent switches of an installer `generator` (closed list); unknown ones run bare.
pub fn silent_args(generator: &str) -> &'static str {
    match generator {
        "Inno Setup" => INNO_ARGS,
        "NSIS" => NSIS_ARGS,
        _ => "",
    }
}

/// Why the installer did not install: the caller falls back to the program asset.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstallerFailure {
    #[error("não iniciou: {0}")]
    Launch(String),
    #[error("autorização de administrador recusada")]
    UacRefused,
    #[error("saiu com o código {0}")]
    Code(i32),
    #[error("registro de programas ilegível: {0}")]
    Registry(String),
    #[error("cancelado antes de iniciar")]
    Cancelled,
}

/// A run that installed (exit code 0 or [`REBOOT_CODE`]): its ARP key, when it wrote or
/// changed one, and that key's `UninstallString`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub arp: Option<ArpKey>,
    pub uninstall_string: Option<String>,
    pub code: i32,
}

/// One installer run, never elevated by wingetrs.
pub struct InstallerRun<'a> {
    /// The verified copy to launch (its name ends in `.exe`).
    pub program: &'a Path,
    pub generator: &'static str,
    /// The app (for the phase and to prefer its ARP key).
    pub app: &'a str,
    /// The release's version (an existing key that changed to it may be the app's).
    pub version: &'a str,
    /// The installer's manifest asks for an administrator: its UAC prompt is announced.
    pub asks_admin: bool,
}

fn same_key(a: &ArpKey, b: &ArpKey) -> bool {
    a.hive == b.hive && a.key.eq_ignore_ascii_case(&b.key)
}

/// Runs the installer with `Verb::Open` (it may ask for its own UAC prompt), then looks for
/// the ARP key it added. A failure here is never an error of the command: the caller decides.
pub fn run_installer(
    env: &GhEnv<'_>,
    gate: &LaunchGate,
    progress: &(dyn Fn(&ProgressEvent) + Send + Sync),
    run: &InstallerRun<'_>,
) -> Result<Installed, InstallerFailure> {
    let before = env
        .arp
        .keys()
        .map_err(|e| InstallerFailure::Registry(e.to_string()))?;
    let spec = LaunchSpec {
        program: run.program.to_path_buf(),
        params: silent_args(run.generator).to_string(),
        dir: run.program.parent().map(Path::to_path_buf),
        show: Show::Normal,
        verb: Verb::Open,
    };
    if !gate.try_launch_for("instalador") {
        return Err(InstallerFailure::Cancelled);
    }
    progress(&ProgressEvent::Phase(Phase::ReleaseInstaller {
        name: run.app.to_string(),
        asks_admin: run.asks_admin,
    }));
    let code = match env.launcher.run(&spec) {
        Ok(c) => c,
        Err(e) if e.raw_os_error() == Some(UAC_REFUSED) => {
            return Err(InstallerFailure::UacRefused);
        }
        Err(e) => return Err(InstallerFailure::Launch(e.to_string())),
    };
    if code != 0 && code != REBOOT_CODE {
        return Err(InstallerFailure::Code(code));
    }
    progress(&ProgressEvent::Phase(Phase::Verifying));
    // The installer already succeeded: an unreadable registry only loses the key.
    let after = env.arp.keys().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "registro de programas ilegível depois do instalador");
        Vec::new()
    });
    let arp = installed_key(&before, after, run.app, run.version);
    let uninstall_string = arp
        .as_ref()
        .and_then(|k| env.arp.uninstall_string(k.hive, &k.key));
    Ok(Installed {
        arp,
        uninstall_string,
        code,
    })
}

/// Whether `app` is a whole word of `display_name` (any case): bounded by the ends, a space, a
/// hyphen or a dot. `git` names "Git" and "Git 2.47", not "GitHub Desktop" nor "Gitea".
pub fn names_app(display_name: &str, app: &str) -> bool {
    let name = display_name.to_lowercase();
    let app = app.to_lowercase();
    if app.is_empty() {
        return false;
    }
    let sep = |c: Option<char>| c.is_none_or(|c| matches!(c, ' ' | '-' | '.'));
    name.match_indices(&app).any(|(at, _)| {
        sep(name[..at].chars().next_back()) && sep(name[at + app.len()..].chars().next())
    })
}

/// The ARP key of the app after its installer ran. Candidates are the new keys and the keys
/// that were already there and whose `DisplayVersion` changed (an update or a reinstall reuses
/// its key). A candidate whose `DisplayName` names `app` as a whole word ([`names_app`]) wins
/// (new before changed). A candidate without the name counts only as the one candidate, and a
/// changed one only when its new `DisplayVersion` is the release's `version` (Edge and Chrome
/// update themselves in the background). Else the only key of all that names `app`. `None`:
/// the installer did not register itself, or nothing tells which key is its own (a wrong key
/// would make removal run another product's uninstaller).
pub fn installed_key(
    before: &[ArpKey],
    after: Vec<ArpKey>,
    app: &str,
    version: &str,
) -> Option<ArpKey> {
    use crate::core::version::Version;
    let named = |k: &ArpKey| names_app(&k.name, app);
    let old = |k: &ArpKey| before.iter().find(|b| same_key(b, k));
    let new: Vec<&ArpKey> = after.iter().filter(|k| old(k).is_none()).collect();
    let changed: Vec<&ArpKey> = after
        .iter()
        .filter(|k| old(k).is_some_and(|b| b.version != k.version))
        .collect();
    if let Some(k) = new.iter().chain(&changed).find(|k| named(k)) {
        return Some((*k).clone());
    }
    let release = Version::parse(version);
    match (new.as_slice(), changed.as_slice()) {
        ([only], []) => return Some((*only).clone()),
        ([], [only])
            if only
                .version
                .as_deref()
                .is_some_and(|v| Version::parse(v) == release) =>
        {
            return Some((*only).clone());
        }
        _ => {}
    }
    match after
        .iter()
        .filter(|k| named(k))
        .collect::<Vec<_>>()
        .as_slice()
    {
        [only] => Some((*only).clone()),
        _ => None,
    }
}

/// A fresh empty folder under `<root>\tmp`, removed with everything in it when dropped.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(tmp: &Path, tag: &str) -> std::io::Result<Scratch> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::fs::create_dir_all(tmp)?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = tmp.join(format!("gh-{tag}-{}-{nanos}-{seq}", std::process::id()));
        std::fs::create_dir(&dir)?;
        Ok(Scratch(dir))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            tracing::warn!(dir = %self.0.display(), error = %e, "pasta temporária não apagada");
        }
    }
}

/// Copies the verified `blob` of `asset` to `<dir>\<asset name>` and checks its SHA-256
/// again: what the shell launches (the cache names blobs without extension).
pub fn launch_copy(blob: &Path, asset: &Asset, dir: &Path) -> Result<PathBuf, InspectError> {
    let name = safe_name(asset)?;
    let dest = dir.join(name);
    if dest.parent() != Some(dir) {
        return Err(InspectError::UnsafeName(asset.name.clone()));
    }
    std::fs::copy(blob, &dest).map_err(io)?;
    let bytes = std::fs::read(&dest).map_err(io)?;
    if crate::cache::sha256_hex(&bytes) != hex::encode(asset.sha256) {
        return Err(InspectError::Io(format!(
            "{}: cópia diferente do arquivo conferido",
            asset.name
        )));
    }
    Ok(dest)
}

/// Records an app its own installer put in place: `apps\<app>\<version>\` with
/// `manifest.json` and `install.json` (mode `installer`, the ARP key and its
/// `UninstallString`), `current` pointing at it, and the `state.db` row (no artifacts).
pub fn record_installer(
    layout: &Layout,
    app: &str,
    bucket: &str,
    version: &str,
    manifest: &str,
    installed: &Installed,
) -> Result<(), StateError> {
    let vdir = layout.version_dir(app, version);
    std::fs::create_dir_all(&vdir)?;
    std::fs::write(vdir.join("manifest.json"), manifest)?;
    let record = installed.arp.as_ref().map(|k| InstallerRecord {
        hive: k.hive,
        key: k.key.clone(),
        uninstall_string: installed.uninstall_string.clone(),
    });
    state::write_installer_json(&vdir, bucket, version, record.as_ref())?;
    let current = layout.current(app);
    if crate::install::link::target(&current).is_ok() {
        crate::install::link::remove(&current)?;
    }
    crate::install::link::create(&vdir, &current)?;
    let mut db = StateDb::open(&layout.state_db())?;
    db.record(
        &AppRecord {
            app: app.to_string(),
            bucket: bucket.to_string(),
            version: version.to_string(),
            arch: None,
            installed_at: crate::cache::now_unix(),
            manifest_sha256: crate::cache::sha256_hex(manifest.as_bytes()),
        },
        &[],
    )
}
