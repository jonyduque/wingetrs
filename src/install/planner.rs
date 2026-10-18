//! Plan builder: turns a resolved manifest, the layout and the current state into
//! an [`InstallPlan`]. Pure: no I/O.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::install::layout::Layout;
use crate::install::manifest::{Arch, Resolved};
use crate::install::plan::{
    InstallPlan, PersistKind, PersistSpec, ShimKind, ShimSpec, ShortcutSpec, Step,
};
use crate::install::safepath;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledInfo {
    pub version: String,
}

#[derive(Debug, Clone)]
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

fn shim_kind(path: &str) -> ShimKind {
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

fn shortcut_link(root: &Path, name: &str) -> PathBuf {
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

/// Phase 2c keys are parsed but not yet executed; each later task removes its `if`.
pub fn refuse_2c(m: &Resolved) -> Result<(), PlanError> {
    let refuse = |k: &str| {
        Err(PlanError::Unsupported(format!(
            "usa {k} (PowerShell/instalador próprio): suportado na fase 2c"
        )))
    };
    if m.hooks.pre_install.is_some() {
        return refuse("pre_install");
    }
    if m.hooks.post_install.is_some() {
        return refuse("post_install");
    }
    if m.hooks.pre_uninstall.is_some() {
        return refuse("pre_uninstall");
    }
    if m.hooks.post_uninstall.is_some() {
        return refuse("post_uninstall");
    }
    if m.installer.is_some() {
        return refuse("installer");
    }
    if m.uninstaller.is_some() {
        return refuse("uninstaller");
    }
    if m.psmodule.is_some() {
        return refuse("psmodule");
    }
    if m.innosetup {
        // one line on purpose: the task that supports innosetup deletes exactly this line
        #[rustfmt::skip]
        return Err(PlanError::Unsupported("usa innosetup: suportado na fase 2c".into()));
    }
    Ok(())
}

pub fn plan_install(input: &PlanInput<'_>) -> Result<InstallPlan, PlanError> {
    let m = input.manifest;
    refuse_2c(m)?;
    let (app, version, layout) = (input.app, m.version.as_str(), input.layout);
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
    for index in 0..m.sources.len() {
        steps.push(Step::Extract { index });
    }
    steps.push(Step::Promote);
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
        let args = b.args.as_deref().map(sub);
        if let Some(a) = args.as_deref().filter(|a| a.chars().any(char::is_control)) {
            return Err(PlanError::UnsafeArgs(format!("{a:?}")));
        }
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
        steps,
    })
}
