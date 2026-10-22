//! Plan builder: turns a resolved manifest, the layout and the current state into
//! an [`InstallPlan`]. Pure except for reading the files a script runs, for the gate
//! (`script_scan::scan_gated`, Task 192), and asking PowerShell's parser (Task 227).
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::install::installer_args::installer_args;
use crate::install::layout::Layout;
use crate::install::manifest::{Arch, HookKind, InstallerSpec, Resolved};
use crate::install::plan::{
    InstallPlan, PersistKind, PersistSpec, ShimKind, ShimSpec, ShortcutSpec, Step,
};
use crate::install::prelude::PRELUDE_FUNCTIONS;
use crate::install::safepath;
use crate::install::script_ast::AstGate;
use crate::install::script_scan::{self, CallRoots, Verdict};
use crate::install::shim;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledInfo {
    pub version: String,
    /// The OLD version's manifest, read from its written `manifest.json` (SHA-256 checked),
    /// never from the catalog: its uninstall hooks run before an upgrade. `None` for apps
    /// installed before Phase 2c (nothing to run).
    pub old_manifest: Option<Resolved>,
}

#[derive(Clone)]
pub struct PlanInput<'a> {
    pub app: &'a str,
    pub bucket: &'a str,
    pub manifest: &'a Resolved,
    pub layout: &'a Layout,
    pub shortcut_root: &'a Path,
    pub installed: Option<InstalledInfo>,
    pub launcher_present: bool,
    pub keep_old: bool,
    /// SHA-256 (hex) of the manifest JSON, carried into the plan.
    pub manifest_sha256: &'a str,
    /// PowerShell's parser for the script gate (`OpsCtx.runners.ast`).
    pub ast: &'a dyn AstGate,
}

impl std::fmt::Debug for PlanInput<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanInput")
            .field("app", &self.app)
            .field("bucket", &self.bucket)
            .field("manifest", &self.manifest)
            .field("layout", &self.layout)
            .field("shortcut_root", &self.shortcut_root)
            .field("installed", &self.installed)
            .field("launcher_present", &self.launcher_present)
            .field("keep_old", &self.keep_old)
            .field("manifest_sha256", &self.manifest_sha256)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PlanError {
    #[error("{app} {version} já está instalado")]
    AlreadyInstalled { app: String, version: String },
    #[error("dois executáveis com o mesmo nome de atalho de comando: {0}")]
    ShimConflict(String),
    #[error("caminho inseguro no manifesto: {0}")]
    UnsafePath(String),
    #[error("argumento inseguro no manifesto: {0}")]
    UnsafeArgs(String),
    #[error("não suportado: {0}")]
    Unsupported(String),
    #[error("o script {hook} foi recusado: {}", .reasons.join("; "))]
    ScriptRefused { hook: String, reasons: Vec<String> },
}

/// The variables [`substitute`] replaces, longest first.
const SHIM_ARG_VARS: [&str; 5] = ["$persist_dir", "$architecture", "$version", "$dir", "$app"];

/// The `bin` arguments `raw` as the shim text gets them (Task 191). The manifest's text passes
/// the shim allowlist with its variables standing for themselves; `$version`/`$app` must be
/// plain values; then each argument (split as cmd would) is substituted and written back
/// quoted when it needs it, so a folder of the user's profile with `&` or `(` stays inert.
fn shim_args(
    raw: &str,
    app: &str,
    version: &str,
    sub: &dyn Fn(&str) -> String,
) -> Result<String, PlanError> {
    let unsafe_args = |why: String| PlanError::UnsafeArgs(format!("{raw:?}: {why}"));
    let template = SHIM_ARG_VARS
        .iter()
        .fold(raw.to_string(), |t, v| t.replace(v, "X"));
    shim::check_args(&template).map_err(unsafe_args)?;
    for (var, value) in [("$version", version), ("$app", app)] {
        if raw.contains(var) && !shim::is_plain_value(value) {
            return Err(unsafe_args(format!(
                "{var} = {value:?} não é um valor simples"
            )));
        }
    }
    let tokens: Vec<(String, bool)> = shim::split_args_quoted(raw)
        .into_iter()
        .map(|(t, quoted)| (sub(&t), quoted))
        .collect();
    shim::cmd_args(&tokens).map_err(unsafe_args)
}

/// Replaces scoop manifest variables (`$dir`, `$persist_dir`, `$version`, `$app`,
/// `$architecture`). A manifest without architecture variants uses the host's.
pub fn substitute(
    template: &str,
    app: &str,
    version: &str,
    arch: Option<Arch>,
    layout: &Layout,
) -> String {
    let arch = arch.unwrap_or_else(Arch::host).key();
    // Longest names first so `$persist_dir` and `$architecture` are not split.
    template
        .replace("$persist_dir", &layout.persist_dir(app).to_string_lossy())
        .replace("$architecture", arch)
        .replace("$version", version)
        .replace("$dir", &layout.current(app).to_string_lossy())
        .replace("$app", app)
}

/// Splits a manifest relative path on both separators.
fn components(p: &str) -> impl Iterator<Item = &str> {
    p.split(['\\', '/']).filter(|c| !c.is_empty())
}

/// Components of a manifest-supplied relative path that must name something inside
/// its base directory: `..`, absolute, drive, UNC, ADS, reserved names and empty
/// paths are refused.
fn safe_rel(raw: &str) -> Result<Vec<String>, PlanError> {
    match safepath::safe_components(raw) {
        Some(c) if !c.is_empty() => Ok(c),
        _ => Err(PlanError::UnsafePath(raw.to_string())),
    }
}

/// `base` joined with the validated manifest path `rel`.
fn join_rel(base: &Path, rel: &str) -> Result<PathBuf, PlanError> {
    let mut out = base.to_path_buf();
    out.extend(safe_rel(rel)?);
    Ok(out)
}

fn rel_path(rel: &str) -> Result<PathBuf, PlanError> {
    join_rel(Path::new(""), rel)
}

/// Resolves one `env_add_path` entry (already substituted) under `current`.
/// `Ok(None)` for empty entries, which scoop drops; `.` is the app directory itself
/// (scoop joins it to `$dir`, as in `"env_add_path": "."`).
fn env_path_entry(raw: &str, current: &Path) -> Result<Option<PathBuf>, PlanError> {
    let unsafe_path = || PlanError::UnsafePath(raw.to_string());
    match raw.trim() {
        "" => return Ok(None),
        "." => return Ok(Some(current.to_path_buf())),
        _ => {}
    }
    let cur = current.to_string_lossy();
    let rel = match raw.strip_prefix(cur.as_ref()) {
        Some("") => return Ok(Some(current.to_path_buf())),
        Some(rest) if rest.starts_with(['\\', '/']) => rest,
        Some(_) => return Err(unsafe_path()),
        None => raw,
    };
    let comps = safepath::safe_components(rel).ok_or_else(unsafe_path)?;
    let mut out = current.to_path_buf();
    out.extend(comps);
    Ok(Some(out))
}

pub(crate) fn shim_kind(path: &str) -> ShimKind {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".cmd") {
        ShimKind::Cmd
    } else if lower.ends_with(".bat") {
        ShimKind::Bat
    } else if lower.ends_with(".ps1") {
        ShimKind::Ps1
    } else if lower.ends_with(".jar") {
        ShimKind::Jar
    } else {
        ShimKind::Exe
    }
}

/// File name without its extension.
fn stem(path: &str) -> String {
    let file = components(path).last().unwrap_or(path);
    match file.rsplit_once('.') {
        Some((s, _)) if !s.is_empty() => s.to_string(),
        _ => file.to_string(),
    }
}

/// One shortcut name component made safe for the file system: reserved and control
/// characters become `_`, `.`/`..` cannot climb out of the shortcut root, a trailing
/// `.`/space becomes `_` and a reserved device name gets a `_` prefix.
fn sanitize_component(c: &str) -> String {
    if c == "." || c == ".." {
        return "_".to_string();
    }
    let mut out: String = c
        .chars()
        .map(|ch| {
            if matches!(ch, '<' | '>' | ':' | '"' | '/' | '|' | '?' | '*') || ch.is_control() {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let kept = out.trim_end_matches(['.', ' ']).len();
    if kept < out.len() {
        let tail = "_".repeat(out.len() - kept);
        out.truncate(kept);
        out.push_str(&tail);
    }
    if safepath::is_windows_unfriendly(&out) {
        out.insert(0, '_');
    }
    out
}

pub(crate) fn shortcut_link(root: &Path, name: &str) -> PathBuf {
    let mut parts: Vec<String> = components(name).map(sanitize_component).collect();
    if parts.is_empty() {
        parts.push("_".to_string());
    }
    let mut out = root.to_path_buf();
    let last = parts.len() - 1;
    for (i, p) in parts.into_iter().enumerate() {
        if i == last {
            out.push(format!("{p}.lnk"));
        } else {
            out.push(p);
        }
    }
    out
}

/// Scoop rule: no dot in the last component ⇒ directory.
fn persist_kind(source: &str) -> PersistKind {
    match components(source).last() {
        Some(last) if last.contains('.') => PersistKind::File,
        _ => PersistKind::Dir,
    }
}

fn runs_ps1_file(spec: &InstallerSpec) -> bool {
    spec.file
        .as_deref()
        .is_some_and(|f| f.to_lowercase().ends_with(".ps1"))
}

fn needs_powershell(spec: &InstallerSpec) -> bool {
    spec.script.is_some() || runs_ps1_file(spec)
}

/// `Step::Hook` for a manifest script, refused at planning when the scanner refuses it (or
/// a file it runs that `roots` can read).
fn hook_step(
    kind: HookKind,
    script: &str,
    roots: &CallRoots,
    ast: &dyn AstGate,
) -> Result<Step, PlanError> {
    let (report, _) = script_scan::scan_gated(script, PRELUDE_FUNCTIONS, roots, ast);
    if report.verdict == Verdict::Refuse {
        let reasons = report
            .flags
            .into_iter()
            .filter(|f| f.verdict == Verdict::Refuse)
            .map(|f| f.detail)
            .collect();
        return Err(PlanError::ScriptRefused {
            hook: kind.key().to_string(),
            reasons,
        });
    }
    Ok(Step::Hook {
        kind,
        script: script.to_string(),
    })
}

/// `Step::RunInstaller` for `installer` (Scoop `Invoke-Installer`): the file defaults to the
/// first download.
fn installer_step(m: &Resolved, spec: &InstallerSpec, vdir: &Path) -> Result<Step, PlanError> {
    let file = match (&spec.file, m.sources.first()) {
        (Some(f), _) => f.clone(),
        (None, Some(s)) => s.file_name.clone(),
        (None, None) => {
            return Err(PlanError::Unsupported(
                "installer sem file e sem arquivo baixado".into(),
            ));
        }
    };
    if safepath::safe_components(&file).is_none_or(|c| c.is_empty()) {
        return Err(PlanError::UnsafePath(file));
    }
    if file.to_lowercase().ends_with(".msi") {
        return Err(PlanError::Unsupported(
            "installer.file .msi não roda direto (o Scoop também não executa)".into(),
        ));
    }
    let args = installer_args(&spec.args, vdir, &m.version)?;
    if file.to_lowercase().ends_with(".ps1") {
        return Ok(Step::RunPs1Installer { file, args });
    }
    Ok(Step::RunInstaller {
        file,
        args,
        keep: spec.keep,
    })
}

/// Scoop's `uninstall` order for the OLD version during an update (scoop-update.ps1:351-377):
/// `pre_uninstall`, the uninstaller file, `uninstaller.script`, `post_uninstall`. Every script
/// goes through the scanner; the uninstaller file is the one the old manifest declares.
fn upgrade_uninstall_steps(
    old: &Resolved,
    old_vdir: &Path,
    root: &Path,
    ast: &dyn AstGate,
) -> Result<Vec<Step>, PlanError> {
    let mut steps = Vec::new();
    // The old version is extracted: files its hooks run from `$dir` are read.
    let roots = CallRoots::new(root, Some(old_vdir));
    let hook = |kind: HookKind, script: &str| {
        hook_step(kind, script, &roots, ast).map(|s| match s {
            Step::Hook { kind, script } => Step::UpgradeUninstallHook { kind, script },
            other => other,
        })
    };
    if let Some(script) = &old.hooks.pre_uninstall {
        steps.push(hook(HookKind::PreUninstall, script)?);
    }
    if let Some(spec) = &old.uninstaller {
        if let Some((file, args)) = uninstaller_command(old, spec, old_vdir)? {
            steps.push(Step::UpgradeRunUninstaller { file, args });
        }
        if let Some(script) = &spec.script {
            steps.push(hook(HookKind::UninstallerScript, script)?);
        }
    }
    if let Some(script) = &old.hooks.post_uninstall {
        steps.push(hook(HookKind::PostUninstall, script)?);
    }
    Ok(steps)
}

/// The `uninstaller` file and substituted arguments to run, if the manifest declares any
/// (Scoop `Invoke-Installer -Uninstall`: only with `file` or `args`, the file defaulting to
/// the first download). Same checks as the installer file.
pub fn uninstaller_command(
    m: &Resolved,
    spec: &InstallerSpec,
    vdir: &Path,
) -> Result<Option<(String, Vec<String>)>, PlanError> {
    if spec.file.is_none() && spec.args.is_empty() {
        return Ok(None);
    }
    Ok(match installer_step(m, spec, vdir)? {
        Step::RunInstaller { file, args, .. } | Step::RunPs1Installer { file, args } => {
            Some((file, args))
        }
        _ => None,
    })
}

/// Refuses (at planning, `ScriptRefused`) an installed manifest whose uninstall scripts the
/// scanner refuses; a `.ps1` uninstaller file is scanned when it is about to run. `roots`:
/// where the files the scripts run can be read (`CallRoots::new(root, Some(vdir))`).
pub fn check_uninstall_scripts(
    m: &Resolved,
    roots: &CallRoots,
    ast: &dyn AstGate,
) -> Result<(), PlanError> {
    let scripts = [
        (HookKind::PreUninstall, m.hooks.pre_uninstall.as_deref()),
        (
            HookKind::UninstallerScript,
            m.uninstaller.as_ref().and_then(|u| u.script.as_deref()),
        ),
        (HookKind::PostUninstall, m.hooks.post_uninstall.as_deref()),
    ];
    for (kind, script) in scripts {
        if let Some(script) = script {
            hook_step(kind, script, roots, ast)?;
        }
    }
    Ok(())
}

/// Hooks and the uninstaller file of an installed manifest need the user's consent to run:
/// the scripts (`pre_uninstall`, `uninstaller.script`, `post_uninstall`, a `.ps1` file).
pub fn has_uninstall_scripts(m: &Resolved) -> bool {
    m.hooks.pre_uninstall.is_some()
        || m.hooks.post_uninstall.is_some()
        || m.uninstaller.as_ref().is_some_and(needs_powershell)
}

pub fn plan_install(input: &PlanInput<'_>) -> Result<InstallPlan, PlanError> {
    let m = input.manifest;
    let (app, version, layout) = (input.app, m.version.as_str(), input.layout);
    // Nothing of this version is extracted yet: only the buckets' guarded `scripts/` is read.
    let roots = CallRoots::new(layout.root(), None);
    // Its own uninstall hooks run at removal time: the scanner already looks at them here.
    check_uninstall_scripts(m, &roots, input.ast)?;
    let upgrade_from = match &input.installed {
        Some(i) if i.version == m.version => {
            return Err(PlanError::AlreadyInstalled {
                app: app.to_string(),
                version: m.version.clone(),
            });
        }
        Some(i) => Some(i.version.clone()),
        None => None,
    };
    let mut notes = Vec::new();
    if let Some(i) = &input.installed
        && i.old_manifest.is_none()
    {
        notes.push(format!(
            "o desinstalador e os scripts de desinstalação da versão {} não vão rodar: o manifesto gravado não existe ou não pôde ser verificado",
            i.version
        ));
    }
    let sub = |s: &str| substitute(s, app, version, m.arch, layout);
    let current = layout.current(app);

    let mut steps = Vec::new();
    if !input.launcher_present {
        steps.push(Step::EnsureLauncher);
    }
    for (index, source) in m.sources.iter().enumerate() {
        steps.push(Step::Download {
            index,
            source: source.clone(),
        });
    }
    for (index, source) in m.sources.iter().enumerate() {
        // Like scoop's Invoke-Extraction: innounp only for the `.exe` sources.
        let inno = m.innosetup && source.file_name.to_lowercase().ends_with(".exe");
        steps.push(if inno {
            Step::ExtractInno { index }
        } else {
            Step::Extract { index }
        });
    }
    if let Some(old) = input
        .installed
        .as_ref()
        .and_then(|i| i.old_manifest.as_ref())
    {
        steps.extend(upgrade_uninstall_steps(
            old,
            &layout.version_dir(app, &old.version),
            layout.root(),
            input.ast,
        )?);
    }
    steps.push(Step::Promote);
    if let Some(script) = &m.hooks.pre_install {
        steps.push(hook_step(HookKind::PreInstall, script, &roots, input.ast)?);
    }
    if let Some(spec) = &m.installer {
        // install.ps1 Invoke-Installer: the file runs only with `file` or `args`, then the script.
        if spec.file.is_some() || !spec.args.is_empty() {
            steps.push(installer_step(m, spec, &layout.version_dir(app, version))?);
        }
        if let Some(script) = &spec.script {
            steps.push(hook_step(
                HookKind::InstallerScript,
                script,
                &roots,
                input.ast,
            )?);
        }
    }
    for p in &m.persist {
        steps.push(Step::Persist(PersistSpec {
            in_version: rel_path(&p.source)?,
            in_persist: rel_path(&p.target)?,
            kind: persist_kind(&p.source),
        }));
    }
    steps.push(Step::LinkCurrent);

    let mut names = HashSet::new();
    for b in &m.bin {
        let name = match b.alias.as_deref() {
            Some(a) if !a.is_empty() => a.to_string(),
            _ => stem(&b.path),
        };
        if !crate::install::shim::is_safe_shim_name(&name) {
            return Err(PlanError::UnsafePath(name));
        }
        // The target is written into `.shim`/`.cmd` text: no line breaks or quotes.
        if b.path.chars().any(|c| c.is_control() || c == '"') {
            return Err(PlanError::UnsafePath(format!("{:?}", b.path)));
        }
        let args = b
            .args
            .as_deref()
            .map(|raw| shim_args(raw, app, version, &sub))
            .transpose()?;
        if !names.insert(name.to_lowercase()) {
            return Err(PlanError::ShimConflict(name));
        }
        steps.push(Step::Shim(ShimSpec {
            name,
            target: join_rel(&current, &b.path)?,
            args,
            kind: shim_kind(&b.path),
        }));
    }
    for s in &m.shortcuts {
        steps.push(Step::Shortcut(ShortcutSpec {
            link: shortcut_link(input.shortcut_root, &s.name),
            target: join_rel(&current, &s.target)?,
            args: s.args.as_deref().map(sub),
            icon: s
                .icon
                .as_deref()
                .map(|i| join_rel(&current, i))
                .transpose()?,
        }));
    }
    if let Some(p) = &m.psmodule {
        // Defense in depth: the manifest parser already checked it.
        match safepath::safe_components(&p.name) {
            Some(c) if c.len() == 1 && c[0] == p.name => {}
            _ => return Err(PlanError::UnsafePath(format!("psmodule.name: {}", p.name))),
        }
        steps.push(Step::PsModule {
            name: p.name.clone(),
        });
    }
    for (name, value) in &m.env_set {
        steps.push(Step::EnvSet {
            name: name.clone(),
            value: sub(value),
        });
    }
    for entry in &m.env_add_path {
        if let Some(entry) = env_path_entry(&sub(entry), &current)? {
            steps.push(Step::EnvAddPath { entry });
        }
    }
    steps.push(Step::EnsureShimsOnPath);
    if let Some(script) = &m.hooks.post_install {
        steps.push(hook_step(HookKind::PostInstall, script, &roots, input.ast)?);
    }
    steps.push(Step::WriteManifest {
        json: m.raw_json.clone(),
    });
    steps.push(Step::RecordState);
    if let Some(old) = &upgrade_from
        && !input.keep_old
    {
        steps.push(Step::RemoveOldVersion {
            version: old.clone(),
        });
    }

    Ok(InstallPlan {
        app: app.to_string(),
        bucket: input.bucket.to_string(),
        version: m.version.clone(),
        arch: m.arch,
        upgrade_from,
        root: layout.root().to_path_buf(),
        manifest_sha256: input.manifest_sha256.to_string(),
        third_party_uninstaller: m.uninstaller.is_some(),
        has_uninstall_scripts: has_uninstall_scripts(m),
        notes,
        steps,
    })
}
