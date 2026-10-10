//! Install plan types: the ordered steps the executor applies, the `--dry-run`
//! prints and the TUI shows.
use std::fmt;
use std::path::PathBuf;

use crate::install::layout::Layout;
use crate::install::manifest::{Arch, HashAlgo, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ShimKind {
    Exe,
    Cmd,
    Bat,
    Ps1,
    Jar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShimSpec {
    pub name: String,
    /// Absolute, under `current`.
    pub target: PathBuf,
    pub args: Option<String>,
    pub kind: ShimKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutSpec {
    /// Absolute path of the `.lnk`.
    pub link: PathBuf,
    pub target: PathBuf,
    pub args: Option<String>,
    pub icon: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistKind {
    Dir,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistSpec {
    /// Relative to the version directory.
    pub in_version: PathBuf,
    /// Relative to `<persist>\<app>`.
    pub in_persist: PathBuf,
    pub kind: PersistKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    EnsureLauncher,
    Download { index: usize, source: Source },
    Extract { index: usize },
    Promote,
    Persist(PersistSpec),
    LinkCurrent,
    Shim(ShimSpec),
    Shortcut(ShortcutSpec),
    EnvSet { name: String, value: String },
    EnvAddPath { entry: PathBuf },
    EnsureShimsOnPath,
    RecordState,
    RemoveOldVersion { version: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub app: String,
    pub bucket: String,
    pub version: String,
    pub arch: Option<Arch>,
    pub upgrade_from: Option<String>,
    /// wingetrs root the plan was built against (used to print full paths).
    pub root: PathBuf,
    /// SHA-256 (hex) of the manifest JSON the plan was built from; recorded in `state.db`.
    pub manifest_sha256: String,
    pub steps: Vec<Step>,
}

/// What uninstalling an app removes, built from its `state.db` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallPlan {
    pub app: String,
    /// Also delete `persist\<app>`.
    pub purge: bool,
    pub artifacts: Vec<crate::install::state::Artifact>,
    /// Remove each recorded PATH entry that no other app still records (checked at execution).
    pub remove_path_entry: bool,
}

impl UninstallPlan {
    /// `None` when `app` has no record.
    pub fn from_state(
        state: &crate::install::state::StateDb,
        app: &str,
        purge: bool,
    ) -> Result<Option<UninstallPlan>, crate::install::state::StateError> {
        Ok(state.get(app)?.map(|(_, artifacts)| UninstallPlan {
            app: app.to_string(),
            purge,
            artifacts,
            remove_path_entry: true,
        }))
    }
}

impl ShimKind {
    /// Files the shim step creates in `shims\`.
    fn files(self, name: &str) -> String {
        match self {
            ShimKind::Exe => format!("{name}.exe + {name}.shim"),
            ShimKind::Cmd | ShimKind::Bat | ShimKind::Jar => format!("{name}.cmd"),
            ShimKind::Ps1 => format!("{name}.cmd + {name}.ps1"),
        }
    }
}

fn algo_name(a: HashAlgo) -> &'static str {
    match a {
        HashAlgo::Sha256 => "sha256",
        HashAlgo::Sha1 => "sha1",
        HashAlgo::Sha512 => "sha512",
        HashAlgo::Md5 => "md5",
    }
}

fn with_args(args: &Option<String>) -> String {
    match args {
        Some(a) => format!(" {a}"),
        None => String::new(),
    }
}

impl InstallPlan {
    fn source(&self, index: usize) -> Option<&Source> {
        self.steps.iter().find_map(|s| match s {
            Step::Download { index: i, source } if *i == index => Some(source),
            _ => None,
        })
    }

    fn step_line(&self, step: &Step, l: &Layout) -> String {
        let (app, v) = (self.app.as_str(), self.version.as_str());
        let staging = l.staging_dir(app, v);
        let version_dir = l.version_dir(app, v);
        let current = l.current(app);
        match step {
            Step::EnsureLauncher => format!(
                "Instalar o lançador de comandos em {}",
                l.launcher().display()
            ),
            Step::Download { index, source } => format!(
                "Baixar [{}] {} ({}, {} {})",
                index + 1,
                source.url,
                source.file_name,
                algo_name(source.hash.algo),
                source.hash.hex
            ),
            Step::Extract { index } => {
                let mut dest = staging.clone();
                let mut extra = String::new();
                let mut name = format!("[{}]", index + 1);
                let mut verb = "Extrair";
                if let Some(src) = self.source(*index) {
                    name = format!("[{}] {}", index + 1, src.file_name);
                    let lower = src.file_name.to_ascii_lowercase();
                    if ![".zip", ".tar.gz", ".tgz"]
                        .iter()
                        .any(|e| lower.ends_with(e))
                    {
                        verb = "Copiar";
                    }
                    if let Some(to) = &src.extract_to {
                        dest = dest.join(to);
                    }
                    if let Some(dir) = &src.extract_dir {
                        extra = format!(" (só a subpasta {dir})");
                    }
                }
                format!("{verb} {name} para {}{extra}", dest.display())
            }
            Step::Promote => format!("Mover {} para {}", staging.display(), version_dir.display()),
            Step::Persist(p) => {
                let what = match p.kind {
                    PersistKind::Dir => "pasta",
                    PersistKind::File => "arquivo",
                };
                let how = match p.kind {
                    PersistKind::Dir => "junção",
                    PersistKind::File => "hardlink",
                };
                format!(
                    "Persistir {what} {} -> {} ({how})",
                    version_dir.join(&p.in_version).display(),
                    l.persist_dir(app).join(&p.in_persist).display()
                )
            }
            Step::LinkCurrent => format!(
                "Apontar {} -> {} (junção)",
                current.display(),
                version_dir.display()
            ),
            Step::Shim(s) => format!(
                "Criar comando {} em {} ({}) -> {}{}",
                s.name,
                l.shims().display(),
                s.kind.files(&s.name),
                s.target.display(),
                with_args(&s.args)
            ),
            Step::Shortcut(s) => {
                let mut line = format!(
                    "Criar atalho {} -> {}{}",
                    s.link.display(),
                    s.target.display(),
                    with_args(&s.args)
                );
                if let Some(icon) = &s.icon {
                    line.push_str(&format!(" (ícone {})", icon.display()));
                }
                line
            }
            Step::EnvSet { name, value } => {
                format!("Definir variável de usuário {name} = {value}")
            }
            Step::EnvAddPath { entry } => {
                format!("Adicionar ao PATH do usuário: {}", entry.display())
            }
            Step::EnsureShimsOnPath => {
                format!("Garantir {} no PATH do usuário", l.shims().display())
            }
            Step::RecordState => format!(
                "Registrar {app} {v} ({}) em {}",
                self.bucket,
                l.state_db().display()
            ),
            Step::RemoveOldVersion { version } => format!(
                "Remover a versão antiga {}",
                l.version_dir(app, version).display()
            ),
        }
    }
}

impl fmt::Display for InstallPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arch = self.arch.map(|a| a.key()).unwrap_or("sem arquitetura");
        match &self.upgrade_from {
            Some(old) => writeln!(
                f,
                "Atualizar {} de {old} para {} (bucket {}, {arch})",
                self.app, self.version, self.bucket
            )?,
            None => writeln!(
                f,
                "Instalar {} {} (bucket {}, {arch})",
                self.app, self.version, self.bucket
            )?,
        }
        let layout = Layout::new(self.root.clone());
        for (i, step) in self.steps.iter().enumerate() {
            writeln!(f, "{:>3}. {}", i + 1, self.step_line(step, &layout))?;
        }
        Ok(())
    }
}
