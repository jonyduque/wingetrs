//! Plan builder: turns a resolved manifest, the layout and the current state into
//! an [`InstallPlan`]. Pure: no I/O.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::install::layout::Layout;
use crate::install::manifest::{Arch, Resolved};
use crate::install::plan::{
    InstallPlan, PersistKind, PersistSpec, ShimKind, ShimSpec, ShortcutSpec, Step,
};

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
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PlanError {
    #[error("{app} {version} já está instalado")]
    AlreadyInstalled { app: String, version: String },
    #[error("dois executáveis com o mesmo nome de atalho de comando: {0}")]
    ShimConflict(String),
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

fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    Path::new(p).is_absolute()
        || p.starts_with("\\\\")
        || (b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && matches!(b[2], b'\\' | b'/'))
}

/// `base` joined with every component of the manifest path `rel`.
fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for c in components(rel) {
        out.push(c);
    }
    out
}

fn rel_path(rel: &str) -> PathBuf {
    join_rel(Path::new(""), rel)
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
/// characters become `_`, and `.`/`..` cannot climb out of the shortcut root.
fn sanitize_component(c: &str) -> String {
    if c == "." || c == ".." {
        return "_".to_string();
    }
    c.chars()
        .map(|ch| {
            if matches!(ch, '<' | '>' | ':' | '"' | '/' | '|' | '?' | '*') || ch.is_control() {
                '_'
            } else {
                ch
            }
        })
        .collect()
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

pub fn plan_install(input: &PlanInput<'_>) -> Result<InstallPlan, PlanError> {
    let m = input.manifest;
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
            in_version: rel_path(&p.source),
            in_persist: rel_path(&p.target),
            kind: persist_kind(&p.source),
        }));
    }
    steps.push(Step::LinkCurrent);

    let mut names = HashSet::new();
    for b in &m.bin {
        let name = b.alias.clone().unwrap_or_else(|| stem(&b.path));
        if !names.insert(name.to_lowercase()) {
            return Err(PlanError::ShimConflict(name));
        }
        steps.push(Step::Shim(ShimSpec {
            name,
            target: join_rel(&current, &b.path),
            args: b.args.as_deref().map(sub),
            kind: shim_kind(&b.path),
        }));
    }
    for s in &m.shortcuts {
        steps.push(Step::Shortcut(ShortcutSpec {
            link: shortcut_link(input.shortcut_root, &s.name),
            target: join_rel(&current, &s.target),
            args: s.args.as_deref().map(sub),
            icon: s.icon.as_deref().map(|i| join_rel(&current, i)),
        }));
    }
    for (name, value) in &m.env_set {
        steps.push(Step::EnvSet {
            name: name.clone(),
            value: sub(value),
        });
    }
    for entry in &m.env_add_path {
        let entry = sub(entry);
        let entry = if is_absolute(&entry) {
            PathBuf::from(entry)
        } else {
            join_rel(&current, &entry)
        };
        steps.push(Step::EnvAddPath { entry });
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
        steps,
    })
}
