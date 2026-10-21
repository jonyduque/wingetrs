//! Native uninstall flow and the Kindle case (Task 184): the plan shown to the user, the run
//! (unelevated for HKCU and user-folder uninstallers, through the elevated helper otherwise),
//! the wait for the ARP key to go, the shared-data warning with its folder backup, and the
//! registry-only removal of a record that is not the principal one.
use super::codes::{self, Outcome};
use super::elevated_rules::{KnownFolders, elevated_duplicate, expand_known, in_admin_folder};
use super::launch::{Elevation, LaunchSpec, Launcher, Show, Verb, uninstall_elevation};
use super::msix::{self, MsixOps};
use super::ops::{Queued, WingetCtx, retry_1618, uac_refused};
use super::orphans::{ArpRegistry, Orphan, check_export_in, export_args, fresh_backup_file};
use super::uninstall_cmd::{ArpDetails, Rule, UninstallCommand, uninstall_command, valid_arp_key};
use crate::bridge::elevated::ElevatedRunner;
use crate::bridge::worker::{DeleteReason, MachineHive};
use crate::install::msi_queue;
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

/// The user's own folders where an app may keep its data (the caller's known folders), and
/// the wingetrs data root (a candidate that contains it is never offered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFolders {
    pub local_app_data: PathBuf,
    pub app_data: PathBuf,
    pub documents: PathBuf,
    pub wingetrs_root: PathBuf,
}

/// Size of a folder tree, counted without following reparse points.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FolderSize {
    pub bytes: u64,
    pub entries: usize,
}

/// Another installation of the same app shares the data folders (spec D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedData {
    /// The candidate data folders that exist (offered for backup before removing).
    pub folders: Vec<PathBuf>,
    /// Their total size (`measure_folders`), once measured; shown in the plan.
    pub size: Option<FolderSize>,
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

/// `1,5 GiB`, `20,0 MiB`, `3,2 KiB`, `12 B`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "GiB"), (1 << 20, "MiB"), (1 << 10, "KiB")];
    for (unit, name) in UNITS {
        if bytes >= unit {
            let tenths = bytes.saturating_mul(10) / unit;
            return format!("{},{} {name}", tenths / 10, tenths % 10);
        }
    }
    format!("{bytes} B")
}

/// Plan line of a removal that runs elevated (its own UAC).
pub const UNINSTALL_ADMIN_PROMPT: &str = "Pede permissão de administrador (UAC)";
/// Plan line of a machine record removed by winget: winget may open the uninstaller's UAC
/// prompt itself (Task 203).
pub const WINGET_ADMIN_PROMPT: &str = "O winget pode pedir permissão de administrador";

impl UninstallPlan {
    /// `Desinstalar jq 1.7.1`: the first line of every confirmation (CLI and TUI).
    pub fn title(&self) -> String {
        format!(
            "Desinstalar {}",
            super::plan::name_version(&self.name, self.version.as_deref())
        )
    }

    /// What the user must consider before saying yes, one capitalized line each. The CLI
    /// numbers them and the TUI lists them: both read this list. The UAC comes first, as in
    /// the install plan.
    pub fn steps(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.elevation == Elevation::Helper {
            out.push(UNINSTALL_ADMIN_PROMPT.to_string());
        } else if self.winget_may_elevate() {
            out.push(WINGET_ADMIN_PROMPT.to_string());
        }
        match (&self.target, &self.command) {
            (UninstallTarget::Msix { family }, _) => {
                out.push(format!("Remove o pacote MSIX {family} (só deste usuário)"));
            }
            // A portable app winget installed: its uninstaller is winget itself.
            (UninstallTarget::Arp { .. }, Some(c)) if winget_product_code(c).is_some() => {
                out.push("Remove pelo winget (pode mostrar o progresso dele)".into());
            }
            (UninstallTarget::Arp { .. }, Some(c)) if c.interactive => {
                out.push("Abre a janela do desinstalador".into());
            }
            (UninstallTarget::Arp { .. }, Some(_)) => {
                out.push("Roda o desinstalador registrado do app".into());
            }
            (UninstallTarget::Arp { .. }, None) => {
                out.push("Sem comando de desinstalação registrado".into());
            }
        }
        out
    }

    /// A machine record (HKLM) removed by winget: winget, not wingetrs, may ask for the UAC.
    pub fn winget_may_elevate(&self) -> bool {
        match (&self.target, &self.command) {
            (UninstallTarget::Arp { hive, .. }, Some(c)) => {
                *hive != Hive::User && winget_product_code(c).is_some()
            }
            _ => false,
        }
    }

    /// The shared-data warning (spec D6), apart from the steps (Task 203): the CLI prints it
    /// as its own warning block and the TUI lists it after the steps.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(s) = &self.shared {
            out.push(format!(
                "Outra instalação de {} usa as mesmas pastas: copie antes de remover.",
                self.name
            ));
            for p in &s.folders {
                out.push(format!("Pasta compartilhada: {}", p.display()));
            }
            if let Some(size) = s.size {
                out.push(format!(
                    "Tamanho da cópia: {} em {} itens",
                    human_size(size.bytes),
                    size.entries
                ));
            }
            if s.registry_only_allowed {
                out.push("Também é possível remover só o registro desta instalação".into());
            } else if let Some(why) = &s.registry_only_refused {
                out.push(format!("Remover só o registro não está disponível: {why}"));
            }
        }
        out
    }

    /// Only for `--dry-run`: the registered uninstall command line.
    pub fn dry_run_details(&self) -> Vec<String> {
        match (&self.target, &self.command) {
            (UninstallTarget::Arp { .. }, Some(c)) => {
                let line = format!("{} {}", c.program, c.args);
                vec![format!("Comando: {}", line.trim())]
            }
            _ => Vec::new(),
        }
    }
}

/// The title, then one step per line and the warnings (what the TUI confirmation lists; no
/// command line).
impl fmt::Display for UninstallPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.title())?;
        for step in self.steps().into_iter().chain(self.warnings()) {
            write!(f, "\n{step}")?;
        }
        Ok(())
    }
}

/// Whether a program is `winget` (by file name, any case, quoted or not, with or without
/// `.exe`).
fn is_winget(program: &str) -> bool {
    Path::new(program.trim().trim_matches('"'))
        .file_stem()
        .is_some_and(|s| s.eq_ignore_ascii_case("winget"))
}

/// The product code of a record whose uninstaller is `winget uninstall --product-code <id>`
/// (what winget registers for the portable apps it installs, as jq), `None` for any other
/// line. The one decision behind the plan's "Remove pelo winget" and the run through the
/// bridge (Task 203): the registry line may be whole in `program` (unquoted, no `.exe`, as
/// `split_command` leaves it) or split into `program` and `args`. Only that exact shape and a
/// plain id pass: nothing else from the registry reaches winget's arguments.
pub fn winget_product_code(c: &UninstallCommand) -> Option<String> {
    let line = if c.args.trim().is_empty() {
        c.program.trim().to_string()
    } else {
        if !is_winget(&c.program) {
            return None;
        }
        format!("winget {}", c.args.trim())
    };
    let mut t = line.split_whitespace();
    let ok = t.next().is_some_and(is_winget)
        && t.next()
            .is_some_and(|v| v.eq_ignore_ascii_case("uninstall"))
        && t.next()
            .is_some_and(|v| v.eq_ignore_ascii_case("--product-code"));
    let id = t.next().filter(|_| ok)?;
    // A leading `-` would be read by winget as one of its options.
    let plain = id.len() <= 255
        && !id.starts_with('-')
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '{' | '}'));
    (plain && t.next().is_none()).then(|| id.to_string())
}

/// The removal goes through `winget uninstall --product-code` on a user-scope (`HKCU`) record:
/// winget refuses it from an administrator terminal (Task 218). Machine records are unaffected.
pub fn winget_user_scope(plan: &UninstallPlan) -> bool {
    matches!(
        plan.target,
        UninstallTarget::Arp {
            hive: Hive::User,
            ..
        }
    ) && plan
        .command
        .as_ref()
        .and_then(winget_product_code)
        .is_some()
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

/// Folder names that hold whole vendor or system trees, never one app's data (compared
/// without case).
const GENERIC_NAMES: [&str; 12] = [
    "Microsoft",
    "Google",
    "Apple",
    "Adobe",
    "Mozilla",
    "Temp",
    "Packages",
    "Programs",
    "Windows",
    "Intel",
    "NVIDIA",
    "wingetrs",
];

fn is_generic(name: &str) -> bool {
    GENERIC_NAMES.iter().any(|g| g.eq_ignore_ascii_case(name))
}

/// The components of `p`, lowercased, for a case-insensitive containment test.
fn lower_components(p: &Path) -> Vec<String> {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// Whether `inner` is `outer` or lies below it (case-insensitive, by component).
fn contains_path(outer: &Path, inner: &Path) -> bool {
    lower_components(inner).starts_with(&lower_components(outer))
}

/// The existing data folders among `%LOCALAPPDATA%\<Publisher>`, `%APPDATA%\<Publisher>`,
/// `%LOCALAPPDATA%\<Name>`, `%APPDATA%\<Name>` and `Documents\<Name>`; never a generic vendor or
/// system name, nor a folder that contains the wingetrs root.
fn data_folders(d: &ArpDetails, u: &UserFolders, exists: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
    let usable = |s: &str| !is_generic(s);
    let publisher = non_blank(&d.publisher)
        .and_then(one_component)
        .filter(|s| usable(s));
    let name = non_blank(&d.display_name)
        .and_then(one_component)
        .filter(|s| usable(s));
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
        if p.parent() != Some(base) || out.contains(&p) || contains_path(&p, &u.wingetrs_root) {
            continue;
        }
        if exists(&p.to_string_lossy()) {
            out.push(p);
        }
    }
    out
}

const REGISTRY_ONLY_REFUSED: &str = "o auxiliar de administrador só apaga um registro HKLM \
     quando outro registro HKLM do mesmo app tem versão maior, e a outra instalação está em \
     HKCU (que o usuário controla) ou não é mais nova; rode o desinstalador desta instalação";

/// The plan for one ARP entry: its command (`uninstall_command`), who runs it, and, when
/// `others` (the other records of the same app) is not empty, the shared data warning. Option C
/// (registry only) is offered for a record that is not the principal one, and for an HKLM
/// record only when the elevated helper will accept it (`elevated_duplicate`: another HKLM
/// record with a strictly greater version; HKCU records never count).
pub fn plan_uninstall(
    d: &ArpDetails,
    nsis_hint: bool,
    others: &[ArpDetails],
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
    // winget asks for the uninstaller's own UAC itself when it needs one.
    let elevation = if winget_product_code(&command).is_some() {
        Elevation::Never
    } else {
        uninstall_elevation(d.hive, msiexec, admin)
    };
    let shared = (!others.is_empty()).then(|| {
        let helper_refuses = d.hive != Hive::User && !elevated_duplicate(d, others);
        SharedData {
            folders: data_folders(d, u, folders),
            size: None,
            registry_only_allowed: !is_main && !helper_refuses,
            registry_only_refused: (!is_main && helper_refuses)
                .then(|| REGISTRY_ONLY_REFUSED.to_string()),
        }
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
    /// Each progress line of the elevated helper while it runs an uninstaller
    /// (`worker::WAITING_UNINSTALLER`, Task 190).
    pub helper_line: &'a dyn Fn(&str),
    /// The bridge's tool runner (`(program, args, on_line)` -> exit code and last lines, as
    /// `bridge::spawn_win::run`): runs `winget uninstall --product-code` (Task 203).
    pub bridge: ToolRunner<'a>,
}

/// A runner of a bridge tool: `(program, args, on_line)` -> exit code and last lines.
pub type ToolRunner<'a> =
    &'a dyn Fn(&str, &[String], &mut dyn FnMut(&str)) -> std::io::Result<(i32, Vec<String>)>;

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

/// The Windows Installer queue an `msiexec /x` goes through, as installs do: `msi.lock`, the
/// mutex and the 1618 retries (`ops::retry_1618`).
pub struct MsiQueue<'a> {
    /// `Layout::msi_lock()`.
    pub lock: PathBuf,
    /// Whether the Windows Installer mutex is held (`Global\_MSIExecute`).
    pub busy: &'a (dyn Fn() -> bool + Send + Sync),
    pub sleep: &'a (dyn Fn(Duration) + Send + Sync),
}

impl<'a> MsiQueue<'a> {
    /// The queue of an install context (same lock, mutex check and sleep).
    pub fn of(w: &WingetCtx<'a>) -> MsiQueue<'a> {
        MsiQueue {
            lock: w.data.install_layout().msi_lock(),
            busy: w.busy,
            sleep: w.sleep,
        }
    }
}

/// [`uninstall_queued`] without `msi.lock` and without the mutex check; an `msiexec /x` that
/// answers 1618 is still retried (sleeping on `ctx.clock`).
pub fn uninstall(ctx: &UninstallCtx<'_>, plan: &UninstallPlan) -> Result<UninstallReport, String> {
    uninstall_queued(ctx, None, plan)
}

/// `winget uninstall --product-code <id>` through the bridge's runner (unelevated: winget asks
/// for the uninstaller's UAC itself), then the wait for the ARP key, as a native run.
fn uninstall_by_winget(
    ctx: &UninstallCtx<'_>,
    id: &str,
    hive: Hive,
    key: &str,
) -> Result<UninstallReport, String> {
    let args: Vec<String> = [
        "uninstall",
        "--product-code",
        id,
        "--exact",
        "--silent",
        "--disable-interactivity",
        "--accept-source-agreements",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let (code, tail) = (ctx.bridge)("winget", &args, &mut |line: &str| {
        tracing::debug!(line, "winget uninstall");
    })
    .map_err(|e| format!("falha ao iniciar o winget: {e}"))?;
    if code != 0 {
        let last = tail.last().map(|l| format!(": {l}")).unwrap_or_default();
        return Err(format!(
            "winget uninstall terminou com o código 0x{:08X}{last}",
            code as u32
        ));
    }
    if wait_key_gone(ctx, hive, key) {
        Ok(UninstallReport {
            status: UninstallStatus::Removed,
            warning: None,
            reboot: None,
        })
    } else {
        Ok(UninstallReport {
            status: UninstallStatus::NoEffect,
            warning: Some(KEY_STAYED.to_string()),
            reboot: None,
        })
    }
}

/// One launch of the uninstaller (unelevated or through the helper); its exit code.
fn launch_once(
    ctx: &UninstallCtx<'_>,
    plan: &UninstallPlan,
    c: &UninstallCommand,
    hive: Hive,
    key: &str,
) -> Result<i32, String> {
    match plan.elevation {
        Elevation::Never => {
            let spec = LaunchSpec {
                program: unelevated_program(ctx, c)?,
                params: c.args.clone(),
                dir: None,
                show: Show::Normal,
                verb: Verb::Open,
            };
            ctx.launcher.run(&spec).map_err(|e| {
                if uac_refused(&e) {
                    codes::UAC_REFUSED.to_string()
                } else {
                    format!("falha ao iniciar o desinstalador: {e}")
                }
            })
        }
        Elevation::Helper => {
            let h = machine_hive(hive)
                .ok_or_else(|| "registro do usuário nunca roda elevado".to_string())?;
            ctx.elevated
                .run_uninstaller_lines(h, key, c.rule, c.rule == Rule::Nsis, &mut |l: &str| {
                    (ctx.helper_line)(l)
                })
                .map_err(|e| e.to_string())
        }
    }
}

/// Runs the plan: HKCU and user-folder uninstallers in this (unelevated) process, the others
/// through the elevated helper, MSIX through `remove`; then waits for the ARP key to go. An
/// `msiexec /x` goes through `queue` (`msi.lock`, mutex, 1618 retried) like an install.
pub fn uninstall_queued(
    ctx: &UninstallCtx<'_>,
    queue: Option<&MsiQueue<'_>>,
    plan: &UninstallPlan,
) -> Result<UninstallReport, String> {
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
    // The plan says "Remove pelo winget" for this very decision: winget runs it, before any
    // check of a native command (Task 203).
    if let Some(id) = winget_product_code(c) {
        return uninstall_by_winget(ctx, &id, hive, key);
    }
    let msiexec = c.rule == Rule::Msi;
    let code = if msiexec {
        let _msi_lock = match queue {
            Some(q) => Some(
                msi_queue::lock_msi(&q.lock, &mut |_| {})
                    .map_err(|e| format!("fila do msiexec: {e}"))?,
            ),
            None => None,
        };
        let no_mutex = || false;
        let clock_sleep = |d: Duration| ctx.clock.sleep(d);
        let (busy, sleep): (&dyn Fn() -> bool, &dyn Fn(Duration)) = match queue {
            Some(q) => (q.busy, q.sleep),
            None => (&no_mutex, &clock_sleep),
        };
        match retry_1618(busy, sleep, &mut || {}, &mut || {
            launch_once(ctx, plan, c, hive, key)
        }) {
            Ok(code) => code,
            Err(Queued::Attempt(e) | Queued::Queue(e)) => return Err(e),
        }
    } else {
        launch_once(ctx, plan, c, hive, key)?
    };
    let outcome = codes::classify_uninstall(msiexec, code);
    // A Burn bundle (or its QuietUninstallString) asking for a reboot: removed only if its
    // key goes, as with msiexec; otherwise the code is the failure.
    let (outcome, key_must_go) = match outcome {
        Outcome::Failed { .. } if matches!(c.rule, Rule::Burn | Rule::Quiet) => {
            match codes::reboot_code(code) {
                Some(o) => (o, true),
                None => (outcome, false),
            }
        }
        o => (o, false),
    };
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
    } else if key_must_go {
        Err(format!(
            "o desinstalador pediu reinício (código {}), mas o registro continua",
            code as u32
        ))
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

fn reparse_note(p: &Path) -> String {
    tracing::warn!(path = %p.display(), "backup: reparse point skipped");
    format!(
        "pulado (ponto de reanálise: link, junção ou arquivo só na nuvem do OneDrive): {}",
        p.display()
    )
}

/// Largest folder backup (all folders of one uninstall together).
pub const MAX_BACKUP_BYTES: u64 = 2 << 30;

/// Limits of a folder backup: the size cap and the free space at the destination.
pub struct BackupLimits<'a> {
    pub max_bytes: u64,
    /// Free bytes on the volume of the path (`None` when it cannot be read).
    pub free_space: &'a dyn Fn(&Path) -> Option<u64>,
}

impl<'a> BackupLimits<'a> {
    /// `MAX_BACKUP_BYTES` and the caller's free-space reader.
    pub fn standard(free_space: &'a dyn Fn(&Path) -> Option<u64>) -> BackupLimits<'a> {
        BackupLimits {
            max_bytes: MAX_BACKUP_BYTES,
            free_space,
        }
    }
}

/// The total size of the folders, without following reparse points (they are not counted,
/// as the copy skips them). Unreadable entries count as empty; past `MAX_BACKUP_ENTRIES`
/// entries it is an error.
pub fn measure_folders(folders: &[PathBuf]) -> Result<FolderSize, String> {
    let mut size = FolderSize::default();
    let mut stack: Vec<PathBuf> = Vec::new();
    for f in folders {
        match std::fs::symlink_metadata(f) {
            Ok(m) if !is_reparse(&m) && m.is_dir() => stack.push(f.clone()),
            _ => {}
        }
    }
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            size.entries += 1;
            if size.entries > MAX_BACKUP_ENTRIES {
                return Err(format!(
                    "as pastas têm mais de {MAX_BACKUP_ENTRIES} itens: cópia recusada"
                ));
            }
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if is_reparse(&meta) {
                continue;
            }
            if meta.is_dir() {
                stack.push(path);
            } else {
                size.bytes = size.bytes.saturating_add(meta.len());
            }
        }
    }
    Ok(size)
}

/// Copies `src` into `dst` (a new folder), never following reparse points and stopping past
/// `budget` bytes; each skip and error goes to `notes`.
fn copy_tree(src: &Path, dst: &Path, budget: &mut u64, notes: &mut Vec<String>) {
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
                notes.push(reparse_note(&path));
                continue;
            }
            let target = to.join(entry.file_name());
            if meta.is_dir() {
                stack.push((path, target));
                continue;
            }
            // The tree grew after it was measured: stop instead of passing the cap.
            let Some(left) = budget.checked_sub(meta.len()) else {
                notes.push(format!(
                    "cópia interrompida: a pasta cresceu além do medido: {}",
                    src.display()
                ));
                return;
            };
            *budget = left;
            if let Err(e) = std::fs::copy(&path, &target) {
                notes.push(format!("falha ao copiar {}: {e}", path.display()));
            }
        }
    }
}

/// Copies each folder to `dest\<n>-<folder name>` (recursive, never following reparse points).
/// Measured first: refused (nothing copied) when a folder contains `dest`, when the total
/// passes `limits.max_bytes` or when the destination has less free space than the total.
/// Returns the skips and errors of the copy.
pub fn backup_folders(
    folders: &[PathBuf],
    dest: &Path,
    limits: &BackupLimits<'_>,
) -> Result<Vec<String>, String> {
    if let Some(f) = folders.iter().find(|f| contains_path(f, dest)) {
        return Err(format!(
            "a pasta {} contém o destino da cópia: cópia recusada",
            f.display()
        ));
    }
    let size = measure_folders(folders)?;
    if size.bytes > limits.max_bytes {
        return Err(format!(
            "as pastas somam {}, acima do limite de {}: copie à mão",
            human_size(size.bytes),
            human_size(limits.max_bytes)
        ));
    }
    let free = (limits.free_space)(dest)
        .ok_or_else(|| format!("não foi possível ler o espaço livre em {}", dest.display()))?;
    if free < size.bytes {
        return Err(format!(
            "espaço livre insuficiente em {}: {} livres, a cópia precisa de {}",
            dest.display(),
            human_size(free),
            human_size(size.bytes)
        ));
    }
    let mut budget = size.bytes;
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
            Ok(m) if is_reparse(&m) => notes.push(reparse_note(folder)),
            Ok(m) if m.is_dir() => copy_tree(folder, &target, &mut budget, &mut notes),
            Ok(_) => notes.push(format!("não é uma pasta: {}", folder.display())),
            Err(e) => notes.push(format!("pasta ilegível {}: {e}", folder.display())),
        }
    }
    Ok(notes)
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
    remove_registry_only_with(plan, reg, elevated, backups, ts, &|_| {})
}

/// [`remove_registry_only`] reporting to `progress` (Task 201): an HKLM key goes through the
/// helper, whose UAC prompt shows `WaitingElevation` until it runs, then `Removendo <name>`.
pub fn remove_registry_only_with(
    plan: &UninstallPlan,
    reg: &dyn ArpRegistry,
    elevated: &dyn ElevatedRunner,
    backups: &Path,
    ts: &str,
    progress: &dyn Fn(&crate::install::progress::ProgressEvent),
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
        Some(h) => {
            use crate::install::progress::{Phase, ProgressEvent, helper_line_phase};
            let removing = Phase::Removing {
                app: plan.name.clone(),
            };
            elevated
                .delete_arp_key_lines(h, key, DeleteReason::Duplicate, &mut |line: &str| {
                    if let Some(p) = helper_line_phase(line, &removing) {
                        progress(&ProgressEvent::Phase(p));
                    }
                })
                .map_err(|e| e.context(format!("falha ao apagar {key}")))?
        }
    }
    Ok(file)
}
