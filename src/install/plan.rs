//! Install plan types: the ordered steps the executor applies, the `--dry-run`
//! prints and the TUI shows.
use std::fmt;
use std::path::PathBuf;

use crate::install::archive::ArchiveKind;
use crate::install::layout::Layout;
use crate::install::manifest::{Arch, HashAlgo, HookKind, Source};

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
    Download {
        index: usize,
        source: Source,
    },
    Extract {
        index: usize,
    },
    /// Source `index` is an Inno Setup installer (`innosetup: true`), unpacked by innounp.
    ExtractInno {
        index: usize,
    },
    Promote,
    /// Runs `file` (relative to the version folder, no PowerShell) with the already
    /// substituted and split `args`; deletes it afterwards unless `keep` (Task 151).
    RunInstaller {
        file: String,
        args: Vec<String>,
        keep: bool,
    },
    /// Runs the `.ps1` file `file` (relative to the version folder) through the script host,
    /// with the substituted and split `args` (Task 158). Never deleted afterwards.
    RunPs1Installer {
        file: String,
        args: Vec<String>,
    },
    /// Runs a manifest hook (`pre_install`, `installer.script`, `post_install`) through the
    /// script host (Task 156).
    Hook {
        kind: HookKind,
        script: String,
    },
    /// Upgrade only (Task 159): a hook of the OLD version's written manifest
    /// (`pre_uninstall`, `uninstaller.script`, `post_uninstall`), run before `Promote` with
    /// `$cmd = 'update'`, the NEW `$version` and the OLD `$manifest`.
    UpgradeUninstallHook {
        kind: HookKind,
        script: String,
    },
    /// Upgrade only (Task 159): the OLD version's `uninstaller.file` (relative to the OLD
    /// version folder), with the arguments already substituted. A `.ps1` runs through the host.
    UpgradeRunUninstaller {
        file: String,
        args: Vec<String>,
    },
    Persist(PersistSpec),
    LinkCurrent,
    Shim(ShimSpec),
    Shortcut(ShortcutSpec),
    /// Junction `<root>\modules\<name>` -> `current`, and `modules` in the user `PSModulePath`.
    PsModule {
        name: String,
    },
    EnvSet {
        name: String,
        value: String,
    },
    EnvAddPath {
        entry: PathBuf,
    },
    EnsureShimsOnPath,
    /// Writes the manifest text as-is to `<version_dir>\manifest.json`.
    WriteManifest {
        json: String,
    },
    RecordState,
    RemoveOldVersion {
        version: String,
    },
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
    /// The manifest has an `uninstaller` that will run third-party code at removal time:
    /// installing it needs the same consent as running an installer (Task 151).
    pub third_party_uninstaller: bool,
    /// The manifest has uninstall scripts (`pre_uninstall`, `post_uninstall`, `uninstaller.script`,
    /// a `.ps1` uninstaller): installing it needs the script consent too (Task 159).
    pub has_uninstall_scripts: bool,
    /// Warnings for the user, shown with the plan (e.g. the old uninstaller will not run).
    pub notes: Vec<String>,
    pub steps: Vec<Step>,
}

/// What uninstalling an app removes, built from its `state.db` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallPlan {
    pub app: String,
    /// Version recorded when the plan was built (checked again before executing).
    pub version: String,
    /// Also delete `persist\<app>`.
    pub purge: bool,
    pub artifacts: Vec<crate::install::state::Artifact>,
    /// PATH entries of this app that other apps still record: `(entry, other apps)`.
    pub path_users: Vec<(String, Vec<String>)>,
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
        use crate::install::state::ArtifactKind;
        let Some((rec, artifacts)) = state.get(app)? else {
            return Ok(None);
        };
        let mut path_users = Vec::new();
        for a in artifacts.iter().filter(|a| a.kind == ArtifactKind::Path) {
            let users = state.path_users(app, &a.target)?;
            if !users.is_empty() {
                path_users.push((a.target.clone(), users));
            }
        }
        Ok(Some(UninstallPlan {
            app: app.to_string(),
            version: rec.version,
            purge,
            artifacts,
            path_users,
            remove_path_entry: true,
        }))
    }

    /// One user-facing line per artifact, in `artifacts` order.
    pub fn artifact_lines(&self) -> Vec<String> {
        use crate::install::state::ArtifactKind as K;
        self.artifacts
            .iter()
            .map(|a| match a.kind {
                // Assumes `remove_path_entry == true` (fixed by `from_state`); the executor keeps PATH when false.
                K::Path => match self.path_users.iter().find(|(t, _)| *t == a.target) {
                    Some((_, users)) => format!(
                        "Manter entrada no PATH: {} (ainda usada por: {})",
                        a.target,
                        users.join(", ")
                    ),
                    None => format!("Desfazer entrada no PATH: {}", a.target),
                },
                K::Persist if self.purge => {
                    format!("Apagar {} (dados persistentes)", a.target)
                }
                K::Persist => format!("Manter {} (dados persistentes; use --purge)", a.target),
                other => format!("Desfazer {}: {}", artifact_kind(other), a.target),
            })
            .collect()
    }
}

fn artifact_kind(k: crate::install::state::ArtifactKind) -> &'static str {
    use crate::install::state::ArtifactKind as K;
    match k {
        K::Shim => "atalho de linha de comando",
        K::Shortcut => "atalho do Menu Iniciar",
        K::Persist => "dados persistentes",
        K::Path => "entrada no PATH",
        K::Env => "variável de ambiente",
        K::PsModule => "módulo PowerShell",
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
                    let kind = ArchiveKind::of(&src.file_name);
                    verb = kind.map_or("Copiar", ArchiveKind::verb);
                    if let Some(engine) = kind.ok().and_then(ArchiveKind::engine) {
                        name = format!("{name} ({engine})");
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
            Step::ExtractInno { index } => {
                let mut dest = staging.clone();
                let mut name = format!("[{}]", index + 1);
                let mut extra = String::new();
                if let Some(src) = self.source(*index) {
                    name = format!("[{}] {}", index + 1, src.file_name);
                    let to = src.extract_to.as_deref();
                    dest.extend(
                        to.and_then(crate::install::safepath::safe_components)
                            .into_iter()
                            .flatten(),
                    );
                    if let Some(dir) = &src.extract_dir {
                        extra = format!(" (só {dir})");
                    }
                }
                format!(
                    "Extrair {name} (Inno Setup, com o innounp) para {}{extra}",
                    dest.display()
                )
            }
            Step::Promote => format!("Mover {} para {}", staging.display(), version_dir.display()),
            Step::RunInstaller { file, args, .. } => {
                // `file` came from a manifest: joined only through safepath.
                let mut program = version_dir.clone();
                program.extend(
                    crate::install::safepath::safe_components(file)
                        .into_iter()
                        .flatten(),
                );
                format!(
                    "Rodar o instalador {} {}",
                    program.display(),
                    args.join(" ")
                )
            }
            Step::RunPs1Installer { file, args } => {
                // `file` came from a manifest: joined only through safepath.
                let mut program = version_dir.clone();
                program.extend(
                    crate::install::safepath::safe_components(file)
                        .into_iter()
                        .flatten(),
                );
                format!(
                    "Rodar o instalador PowerShell {} {}",
                    program.display(),
                    args.join(" ")
                )
            }
            Step::Hook { kind, .. } => {
                format!("Rodar o script {} do manifesto (PowerShell)", kind.key())
            }
            Step::UpgradeUninstallHook { kind, .. } => format!(
                "Rodar o script {} da versão antiga (PowerShell)",
                kind.key()
            ),
            Step::UpgradeRunUninstaller { file, args } => {
                let old = self.upgrade_from.as_deref().unwrap_or(v);
                let mut program = l.version_dir(app, old);
                program.extend(
                    crate::install::safepath::safe_components(file)
                        .into_iter()
                        .flatten(),
                );
                format!(
                    "Rodar o desinstalador da versão antiga {} {}",
                    program.display(),
                    args.join(" ")
                )
            }
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
            Step::PsModule { name } => format!(
                "Ligar módulo PowerShell {} -> {} e garantir {} no PSModulePath do usuário",
                l.modules().join(name).display(),
                l.current(app).display(),
                l.modules().display()
            ),
            Step::EnvSet { name, value } => {
                format!("Definir variável de usuário {name} = {value}")
            }
            Step::EnvAddPath { entry } => {
                format!("Adicionar ao PATH do usuário: {}", entry.display())
            }
            Step::EnsureShimsOnPath => {
                format!("Garantir {} no PATH do usuário", l.shims().display())
            }
            Step::WriteManifest { .. } => format!(
                "Gravar {}",
                l.version_dir(app, v).join("manifest.json").display()
            ),
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
        for note in &self.notes {
            writeln!(f, "Aviso: {note}")?;
        }
        Ok(())
    }
}
