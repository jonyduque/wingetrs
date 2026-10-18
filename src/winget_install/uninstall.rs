//! Native uninstall flow and the Kindle case (Task 184): the plan shown to the user, the run
//! (unelevated for HKCU and user-folder uninstallers, through the elevated helper otherwise),
//! the wait for the ARP key to go, the shared-data warning with its folder backup, and the
//! registry-only removal of a record that is not the principal one.
use super::codes::{self, Outcome};
use super::elevated_rules::{KnownFolders, elevated_duplicate, expand_known, in_admin_folder};
use super::launch::{Elevation, LaunchSpec, Launcher, Show, uninstall_elevation};
use super::msix::{self, MsixOps};
use super::orphans::{ArpRegistry, Orphan, backup_file, check_export_in, export_args};
use super::uninstall_cmd::{ArpDetails, Rule, UninstallCommand, uninstall_command, valid_arp_key};
use crate::bridge::elevated::ElevatedRunner;
use crate::bridge::worker::{DeleteReason, MachineHive};
use crate::install::safepath::safe_components;
use crate::installed::Hive;
use crate::installed::arp::expand_vars;
use crate::job_log;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often the ARP key is checked after the uninstaller exits.
const POLL_EVERY: Duration = Duration::from_millis(500);
/// How long the key may stay before the run counts as without effect (NSIS copies itself to
/// `%TEMP%` and exits before it finishes).
const POLL_FOR: Duration = Duration::from_secs(120);
/// Numbered names tried when the `.reg` backup name is already taken.
const MAX_BACKUP_SUFFIX: u32 = 100;
/// Entries copied per folder backup at most (a profile folder is not a disk).
const MAX_BACKUP_ENTRIES: usize = 200_000;

pub const KEY_STAYED: &str = "o desinstalador terminou, mas o registro continua";

/// What gets uninstalled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallTarget {
    Arp { hive: Hive, key: String },
    Msix { family: String },
}

/// The target of an `overrides::app_key` (`arp/hklm64|hklm32|hkcu/<sub>`, `msix/<family>`).
pub fn target_of(app_key: &str) -> Option<UninstallTarget> {
    if let Some(rest) = app_key.strip_prefix("arp/") {
        let (tag, key) = rest.split_once('/')?;
        let hive = match tag {
            "hklm64" => Hive::MachineX64,
            "hklm32" => Hive::MachineX86,
            "hkcu" => Hive::User,
            _ => return None,
        };
        return valid_arp_key(key).then(|| UninstallTarget::Arp {
            hive,
            key: key.to_string(),
        });
    }
    let family = app_key.strip_prefix("msix/")?;
    (!family.is_empty() && !family.contains(['/', '\\'])).then(|| UninstallTarget::Msix {
        family: family.to_string(),
    })
}

/// The user's own folders where an app may keep its data (the caller's known folders).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFolders {
    pub local_app_data: PathBuf,
    pub app_data: PathBuf,
    pub documents: PathBuf,
}

/// Another installation of the same app shares the data folders (spec D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedData {
    /// The candidate data folders that exist (offered for backup before removing).
    pub folders: Vec<PathBuf>,
    /// Whether "remove only the registry record" (option C) is offered.
    pub registry_only_allowed: bool,
    /// Why option C is not offered, when the record is not the principal one but the elevated
    /// helper would refuse to delete it.
    pub registry_only_refused: Option<String>,
}

/// One uninstall, as shown in the dry run and in every confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallPlan {
    pub target: UninstallTarget,
    pub name: String,
    pub version: Option<String>,
    pub command: Option<UninstallCommand>,
    pub elevation: Elevation,
    pub shared: Option<SharedData>,
}

impl fmt::Display for UninstallPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Desinstalar {}", self.name)?;
        if let Some(v) = &self.version {
            write!(f, " {v}")?;
        }
        writeln!(f)?;
        match (&self.target, &self.command) {
            (UninstallTarget::Msix { family }, _) => {
                writeln!(f, "  remove o pacote MSIX {family} (só deste usuário)")?;
            }
            (UninstallTarget::Arp { .. }, Some(c)) => {
                let line = format!("{} {}", c.program, c.args);
                writeln!(f, "  comando: {}", line.trim())?;
            }
            (UninstallTarget::Arp { .. }, None) => writeln!(f, "  sem comando")?,
        }
        if self.elevation == Elevation::Helper {
            writeln!(f, "  pede UAC")?;
        }
        if self.command.as_ref().is_some_and(|c| c.interactive) {
            writeln!(f, "  abre a janela do desinstalador")?;
        }
        if let Some(s) = &self.shared {
            writeln!(
                f,
                "  Outra instalação de {} usa as mesmas pastas: copie antes de remover.",
                self.name
            )?;
            for p in &s.folders {
                writeln!(f, "    pasta: {}", p.display())?;
            }
            if s.registry_only_allowed {
                writeln!(
                    f,
                    "  também é possível remover só o registro desta instalação"
                )?;
            } else if let Some(why) = &s.registry_only_refused {
                writeln!(f, "  remover só o registro não está disponível: {why}")?;
            }
        }
        Ok(())
    }
}

fn non_blank(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// `name` when it is one safe path component (passes `safe_components` unchanged).
fn one_component(name: &str) -> Option<&str> {
    match safe_components(name)?.as_slice() {
        [only] if only == name => Some(name),
        _ => None,
    }
}

/// The existing data folders among `%LOCALAPPDATA%\<Publisher>`, `%APPDATA%\<Publisher>`,
/// `%LOCALAPPDATA%\<Name>`, `%APPDATA%\<Name>` and `Documents\<Name>`.
fn data_folders(d: &ArpDetails, u: &UserFolders, exists: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
    let publisher = non_blank(&d.publisher).and_then(one_component);
    let name = non_blank(&d.display_name).and_then(one_component);
    let mut candidates: Vec<(&Path, &str)> = Vec::new();
    if let Some(p) = publisher {
        candidates.push((&u.local_app_data, p));
        candidates.push((&u.app_data, p));
    }
    if let Some(n) = name {
        candidates.push((&u.local_app_data, n));
        candidates.push((&u.app_data, n));
        candidates.push((&u.documents, n));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for (base, comp) in candidates {
        let p = base.join(comp);
        if p.parent() != Some(base) || out.contains(&p) {
            continue;
        }
        if exists(&p.to_string_lossy()) {
            out.push(p);
        }
    }
    out
}

/// The plan for one ARP entry: its command (`uninstall_command`), who runs it, and the shared
/// data warning when `has_others` (option C only for a record that is not the principal one).
pub fn plan_uninstall(
    d: &ArpDetails,
    nsis_hint: bool,
    has_others: bool,
    is_main: bool,
    k: &KnownFolders,
    u: &UserFolders,
    folders: &dyn Fn(&str) -> bool,
) -> Result<UninstallPlan, String> {
    if !valid_arp_key(&d.key) {
        return Err(format!("nome de chave ARP inválido: {:?}", d.key));
    }
    let command = uninstall_command(d, nsis_hint).ok_or_else(|| {
        "sem desinstalador registrado; use \"Remover registro órfão\" se for sobra".to_string()
    })?;
    let msiexec = command.rule == Rule::Msi;
    let admin =
        expand_known(&command.program, k).is_some_and(|p| in_admin_folder(Path::new(&p), k));
    let elevation = uninstall_elevation(d.hive, msiexec, admin);
    let shared = has_others.then(|| SharedData {
        folders: data_folders(d, u, folders),
        registry_only_allowed: !is_main,
        registry_only_refused: None,
    });
    let name = non_blank(&d.display_name).unwrap_or(&d.key).to_string();
    Ok(UninstallPlan {
        target: UninstallTarget::Arp {
            hive: d.hive,
            key: d.key.clone(),
        },
        name,
        version: non_blank(&d.display_version).map(str::to_string),
        command: Some(command),
        elevation,
        shared,
    })
}

/// `plan_uninstall` knowing the other records of the app: option C for an HKLM record is
/// offered only when the elevated helper will accept it (`elevated_duplicate`: another HKLM
/// record with a strictly greater version; HKCU records never count).
pub fn plan_uninstall_among(
    d: &ArpDetails,
    nsis_hint: bool,
    others: &[ArpDetails],
    is_main: bool,
    k: &KnownFolders,
    u: &UserFolders,
    folders: &dyn Fn(&str) -> bool,
) -> Result<UninstallPlan, String> {
    let mut plan = plan_uninstall(d, nsis_hint, !others.is_empty(), is_main, k, u, folders)?;
    if let Some(s) = plan.shared.as_mut()
        && s.registry_only_allowed
        && d.hive != Hive::User
        && !elevated_duplicate(d, others)
    {
        s.registry_only_allowed = false;
        s.registry_only_refused = Some(
            "o auxiliar de administrador só apaga um registro HKLM quando outro registro HKLM \
             do mesmo app tem versão maior, e a outra instalação está em HKCU (que o usuário \
             controla) ou não é mais nova; rode o desinstalador desta instalação"
                .to_string(),
        );
    }
    Ok(plan)
}

/// Time source of the wait for the key (a fake in tests: no real waits).
pub trait Clock {
    fn now(&self) -> Duration;
    fn sleep(&self, d: Duration);
}

/// The real clock.
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> SystemClock {
        SystemClock {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> SystemClock {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// What the uninstall needs from the outside world.
pub struct UninstallCtx<'a> {
    pub launcher: &'a dyn Launcher,
    pub elevated: &'a dyn ElevatedRunner,
    pub msix: &'a dyn MsixOps,
    pub key_exists: &'a dyn Fn(Hive, &str) -> bool,
    pub clock: &'a dyn Clock,
    /// The value of one environment variable of the user (for unelevated uninstallers).
    pub user_expand: &'a dyn Fn(&str) -> Option<String>,
    /// `System32\msiexec.exe` (`launch_win::msiexec_path`), never from PATH.
    pub msiexec: &'a Path,
}

/// How the uninstall ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninstallStatus {
    Removed,
    /// The uninstaller said success but the record is still there.
    NoEffect,
}

impl UninstallStatus {
    pub fn job_outcome(self) -> job_log::Outcome {
        match self {
            UninstallStatus::Removed => job_log::Outcome::Ok,
            UninstallStatus::NoEffect => job_log::Outcome::NoEffect,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallReport {
    pub status: UninstallStatus,
    pub warning: Option<String>,
    /// `Some(initiated)` when the uninstaller asked for a reboot.
    pub reboot: Option<bool>,
}

fn machine_hive(h: Hive) -> Option<MachineHive> {
    match h {
        Hive::MachineX64 => Some(MachineHive::Hklm64),
        Hive::MachineX86 => Some(MachineHive::Hklm32),
        Hive::User => None,
    }
}

/// The program an unelevated uninstall starts: the injected `msiexec`, or the registry's
/// program expanded with the user's environment, which must be an absolute path (never PATH).
fn unelevated_program(ctx: &UninstallCtx<'_>, c: &UninstallCommand) -> Result<PathBuf, String> {
    if c.rule == Rule::Msi {
        return Ok(ctx.msiexec.to_path_buf());
    }
    let expanded = expand_vars(&c.program, &|name: &str| (ctx.user_expand)(name))
        .ok_or_else(|| format!("variável desconhecida no desinstalador: {}", c.program))?;
    let p = PathBuf::from(expanded.trim());
    if !p.is_absolute() {
        return Err(format!("desinstalador sem caminho absoluto: {}", c.program));
    }
    Ok(p)
}

/// Waits for the ARP key to go: checks every `POLL_EVERY` until `POLL_FOR`.
fn wait_key_gone(ctx: &UninstallCtx<'_>, hive: Hive, key: &str) -> bool {
    let start = ctx.clock.now();
    loop {
        if !(ctx.key_exists)(hive, key) {
            return true;
        }
        if ctx.clock.now().saturating_sub(start) >= POLL_FOR {
            return false;
        }
        ctx.clock.sleep(POLL_EVERY);
    }
}

fn reboot_of(o: &Outcome) -> Option<bool> {
    match o {
        Outcome::OkReboot { initiated } => Some(*initiated),
        _ => None,
    }
}

/// Runs the plan: HKCU and user-folder uninstallers in this (unelevated) process, the others
/// through the elevated helper, MSIX through `remove`; then waits for the ARP key to go.
pub fn uninstall(ctx: &UninstallCtx<'_>, plan: &UninstallPlan) -> Result<UninstallReport, String> {
    let (hive, key) = match &plan.target {
        UninstallTarget::Msix { family } => {
            return match ctx.msix.remove(family) {
                Ok(0) => Ok(UninstallReport {
                    status: UninstallStatus::NoEffect,
                    warning: Some("nenhum pacote da família estava instalado".to_string()),
                    reboot: None,
                }),
                Ok(_) => Ok(UninstallReport {
                    status: UninstallStatus::Removed,
                    warning: None,
                    reboot: None,
                }),
                Err(e) => Err(match msix::outcome(&e) {
                    Outcome::Failed { message, .. } => format!("{message}: {e}"),
                    _ => e.to_string(),
                }),
            };
        }
        UninstallTarget::Arp { hive, key } => (*hive, key.as_str()),
    };
    let c = plan
        .command
        .as_ref()
        .ok_or_else(|| "sem desinstalador registrado".to_string())?;
    let msiexec = c.rule == Rule::Msi;
    let code = match plan.elevation {
        Elevation::Never => {
            let spec = LaunchSpec {
                program: unelevated_program(ctx, c)?,
                params: c.args.clone(),
                dir: None,
                show: Show::Normal,
            };
            ctx.launcher
                .run(&spec)
                .map_err(|e| format!("falha ao iniciar o desinstalador: {e}"))?
        }
        Elevation::Helper => {
            let h = machine_hive(hive)
                .ok_or_else(|| "registro do usuário nunca roda elevado".to_string())?;
            ctx.elevated
                .run_uninstaller(h, key, c.rule, c.rule == Rule::Nsis)
                .map_err(|e| e.to_string())?
        }
    };
    let outcome = codes::classify_uninstall(msiexec, code);
    if let Outcome::Failed { message, .. } = &outcome {
        return Err(message.clone());
    }
    let reboot = reboot_of(&outcome);
    if wait_key_gone(ctx, hive, key) {
        Ok(UninstallReport {
            status: UninstallStatus::Removed,
            warning: None,
            reboot,
        })
    } else {
        Ok(UninstallReport {
            status: UninstallStatus::NoEffect,
            warning: Some(KEY_STAYED.to_string()),
            reboot,
        })
    }
}

/// `FILE_ATTRIBUTE_REPARSE_POINT`.
#[cfg(windows)]
fn reparse_attribute(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn reparse_attribute(_m: &std::fs::Metadata) -> bool {
    false
}

/// A symlink, junction or any other reparse point (never followed).
fn is_reparse(m: &std::fs::Metadata) -> bool {
    m.file_type().is_symlink() || reparse_attribute(m)
}

/// Copies `src` into `dst` (a new folder), never following reparse points; each skip and
/// error goes to `notes`.
fn copy_tree(src: &Path, dst: &Path, notes: &mut Vec<String>) {
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    let mut copied = 0usize;
    while let Some((from, to)) = stack.pop() {
        if let Err(e) = std::fs::create_dir_all(&to) {
            notes.push(format!("não foi possível criar {}: {e}", to.display()));
            continue;
        }
        let entries = match std::fs::read_dir(&from) {
            Ok(r) => r,
            Err(e) => {
                notes.push(format!("não foi possível ler {}: {e}", from.display()));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(x) => x,
                Err(e) => {
                    notes.push(format!("não foi possível ler {}: {e}", from.display()));
                    continue;
                }
            };
            copied += 1;
            if copied > MAX_BACKUP_ENTRIES {
                notes.push(format!(
                    "cópia interrompida em {} itens: {}",
                    MAX_BACKUP_ENTRIES,
                    src.display()
                ));
                return;
            }
            let path = entry.path();
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    notes.push(format!("não foi possível ler {}: {e}", path.display()));
                    continue;
                }
            };
            if is_reparse(&meta) {
                tracing::warn!(path = %path.display(), "backup: reparse point skipped");
                notes.push(format!(
                    "pulado (link ou junção, não seguido): {}",
                    path.display()
                ));
                continue;
            }
            let target = to.join(entry.file_name());
            if meta.is_dir() {
                stack.push((path, target));
            } else if let Err(e) = std::fs::copy(&path, &target) {
                notes.push(format!("falha ao copiar {}: {e}", path.display()));
            }
        }
    }
}

/// Copies each folder to `dest\<n>-<folder name>` (recursive, never following reparse points);
/// returns the skips and errors.
pub fn backup_folders(folders: &[PathBuf], dest: &Path) -> Vec<String> {
    let mut notes = Vec::new();
    for (i, folder) in folders.iter().enumerate() {
        let name = folder.file_name().map(|n| n.to_string_lossy().into_owned());
        let Some(name) = name.as_deref().and_then(one_component) else {
            notes.push(format!("nome de pasta inválido: {}", folder.display()));
            continue;
        };
        let target = dest.join(format!("{}-{name}", i + 1));
        if target.parent() != Some(dest) {
            notes.push(format!("nome de pasta inválido: {}", folder.display()));
            continue;
        }
        match std::fs::symlink_metadata(folder) {
            Ok(m) if is_reparse(&m) => {
                tracing::warn!(path = %folder.display(), "backup: reparse point skipped");
                notes.push(format!(
                    "pulado (link ou junção, não seguido): {}",
                    folder.display()
                ));
            }
            Ok(m) if m.is_dir() => copy_tree(folder, &target, &mut notes),
            Ok(_) => notes.push(format!("não é uma pasta: {}", folder.display())),
            Err(e) => notes.push(format!("pasta ilegível {}: {e}", folder.display())),
        }
    }
    notes
}

/// A `.reg` backup path nobody wrote yet (so a stale file never stands in for a failed export).
fn fresh_backup_file(backups: &Path, ts: &str, o: &Orphan) -> Result<PathBuf, String> {
    let first = backup_file(backups, ts, o);
    let dir = first
        .parent()
        .ok_or_else(|| "pasta de backup inválida".to_string())?
        .to_path_buf();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("não foi possível criar {}: {e}", dir.display()))?;
    let stem = first
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let candidates = std::iter::once(first)
        .chain((2..=MAX_BACKUP_SUFFIX).map(|n| dir.join(format!("{stem}-{n}.reg"))));
    for c in candidates {
        match std::fs::symlink_metadata(&c) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(c),
            Err(e) => return Err(format!("não foi possível conferir {}: {e}", c.display())),
            Ok(_) => continue,
        }
    }
    Err(format!("nomes de backup esgotados em {}", dir.display()))
}

/// Option C: removes only the registry record of an installation that is not the principal
/// one. Exports and checks the `.reg` first (as for orphans); HKCU deletes in process, HKLM
/// through the elevated helper (which checks the duplicate rule again). Returns the `.reg`.
pub fn remove_registry_only(
    plan: &UninstallPlan,
    reg: &dyn ArpRegistry,
    elevated: &dyn ElevatedRunner,
    backups: &Path,
    ts: &str,
) -> Result<PathBuf, String> {
    let shared = plan
        .shared
        .as_ref()
        .ok_or_else(|| "não há outra instalação deste app".to_string())?;
    if !shared.registry_only_allowed {
        return Err(shared
            .registry_only_refused
            .clone()
            .unwrap_or_else(|| "esta é a instalação principal: use o desinstalador".to_string()));
    }
    let UninstallTarget::Arp { hive, key } = &plan.target else {
        return Err("só registros do Windows podem ser removidos assim".to_string());
    };
    if !valid_arp_key(key) {
        return Err(format!("nome de chave ARP inválido: {key:?}"));
    }
    let o = Orphan {
        hive: *hive,
        key: key.clone(),
        name: plan.name.clone(),
        missing: String::new(),
    };
    let file = fresh_backup_file(backups, ts, &o)?;
    reg.export(&export_args(*hive, key, &file))
        .map_err(|e| format!("falha ao exportar {key}: {e}"))?;
    check_export_in(&file, *hive, key)?;
    match machine_hive(*hive) {
        None => reg
            .delete_user_key(key)
            .map_err(|e| format!("falha ao apagar {key}: {e}"))?,
        Some(h) => elevated
            .delete_arp_key(h, key, DeleteReason::Duplicate)
            .map_err(|e| format!("falha ao apagar {key}: {e}"))?,
    }
    Ok(file)
}
