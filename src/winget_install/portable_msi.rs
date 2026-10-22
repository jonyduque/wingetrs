//! Whether an MSI of the winget catalog can be installed "portable" (Task 229): its files
//! extracted by our own MSI reader into the wingetrs root, without running the installer.
//! Reads the tables only; nothing runs. Blockers are what the app needs and portable mode
//! cannot give it; losses are what the installer would do that portable mode skips.
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use msi::{Package, Row};

use crate::install::extract::{ExtractError, catch_decoder};
use crate::install::extract_7z::panic_message;
use crate::install::extract_msi::{Pkg, int, rows, tables, text};
use crate::install::msi_layout;

/// Base custom action types (`Type & 0x3F`) that run code: DLL, EXE and script, from
/// `Binary`, an installed file, a directory, text or a property.
const CODE_TYPES: [i32; 14] = [1, 2, 5, 6, 17, 18, 21, 22, 34, 37, 38, 50, 53, 54];
/// Base types that install another MSI (nested install): they block whether deferred or not.
const NESTED_TYPES: [i32; 3] = [7, 23, 39];
/// `msidbCustomActionTypeInScript`: the action is deferred (runs in the install script).
const IN_SCRIPT: i32 = 0x400;
/// Directory ids whose files land in Windows folders.
const SYSTEM_DIRS: [&str; 6] = [
    "SystemFolder",
    "System64Folder",
    "System16Folder",
    "WindowsFolder",
    "FontsFolder",
    "WindowsVolume",
];
/// Standard Windows Installer folders: a file's main folder is the first `Directory` below them.
const STANDARD_DIRS: [&str; 40] = [
    "TARGETDIR",
    "ROOTDRIVE",
    "ProgramFilesFolder",
    "ProgramFiles64Folder",
    "ProgramFiles6432Folder",
    "CommonFilesFolder",
    "CommonFiles64Folder",
    "CommonFiles6432Folder",
    "AppDataFolder",
    "LocalAppDataFolder",
    "CommonAppDataFolder",
    "AdminToolsFolder",
    "DesktopFolder",
    "FavoritesFolder",
    "FontsFolder",
    "MyPicturesFolder",
    "NetHoodFolder",
    "PersonalFolder",
    "PrintHoodFolder",
    "ProgramMenuFolder",
    "RecentFolder",
    "SendToFolder",
    "StartMenuFolder",
    "StartupFolder",
    "System16Folder",
    "System64Folder",
    "SystemFolder",
    "System6432Folder",
    "TempFolder",
    "TemplateFolder",
    "WindowsFolder",
    "WindowsVolume",
    "PerUserProgramFilesFolder",
    "PerUserCommonFilesFolder",
    "ProgramFilesFolderX86",
    "CommonFilesFolderX86",
    "UserProfileFolder",
    "MyMusicFolder",
    "MyVideoFolder",
    "GlobalAssemblyCache",
];
/// Rollback-only custom action (`msidbCustomActionTypeRollback`): runs only when an install fails.
const ROLLBACK: i32 = 0x100;
/// `msidbCustomActionTypeRollback | msidbCustomActionTypeCommit`: both set is not rollback-only.
const ROLLBACK_COMMIT: i32 = 0x300;
/// Deferred actions of the WiX standard library whose only effect is internet shortcuts.
const INTERNET_SHORTCUT_ACTIONS: [&str; 3] = [
    "Wix4CreateInternetShortcuts_X86",
    "Wix4CreateInternetShortcuts_X64",
    "Wix4CreateInternetShortcuts_A64",
];
/// Summary Information word count bit: elevated privileges are not required.
const WORD_COUNT_NO_ELEVATION: i32 = 0x8;
/// `Registry.Root` of `HKEY_LOCAL_MACHINE`.
const ROOT_HKLM: i32 = 2;
/// `Registry.Root` `msidbRegistryRootDependent`: HKLM in a per-machine install.
const ROOT_DEPENDENT: i32 = -1;
/// Registry keys (under HKCR, or `Software\Classes\` + key) that register COM.
const COM_KEYS: [&str; 4] = ["clsid\\", "typelib\\", "appid\\", "interface\\"];

/// What portable mode cannot do for the app.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Blocker {
    #[error("instala o serviço do Windows {0}")]
    Service(String),
    #[error("instala driver ({0})")]
    Driver(String),
    #[error("ação personalizada {0} roda código durante a instalação")]
    DeferredAction(String),
    #[error("ação personalizada {0} instala outro pacote MSI")]
    NestedInstall(String),
    #[error("registra componente COM {0}")]
    Com(String),
    #[error("instala assembly .NET no cache global (GAC) ({0})")]
    Gac(String),
    #[error("grava em HKLM\\{0}")]
    SystemRegistry(String),
    #[error("{0} arquivo(s) para pastas do Windows")]
    SystemFiles(usize),
    #[error("arquivos espalhados fora de uma pasta principal")]
    Scattered,
}

/// What the installer would do that portable mode skips.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Loss {
    #[error("ação personalizada {0} não roda")]
    ImmediateAction(String),
    #[error("ação personalizada {0} (só de rollback) não roda")]
    RollbackAction(String),
    #[error("atalhos de internet")]
    InternetShortcuts,
    #[error("{0} valor(es) de registro não gravado(s)")]
    Registry(usize),
    #[error("associação de arquivos: {}", .0.join(", "))]
    Associations(Vec<String>),
    #[error("variáveis de ambiente: {}", .0.join(", "))]
    Environment(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsiFacts {
    pub blockers: Vec<Blocker>,
    pub losses: Vec<Loss>,
    /// Common folder of the app's files in the `msiexec /a` tree (source names).
    pub main_dir: Option<Vec<String>>,
    /// The app's files, relative to `main_dir`, sorted.
    pub files: Vec<Vec<String>>,
    /// `(long name, target relative to main_dir)`, sorted by the `Shortcut` key.
    pub shortcuts: Vec<(String, Vec<String>)>,
    /// The package declares a per-user install (`ALLUSERS=2 MSIINSTALLPERUSER=1`); recorded only.
    pub per_user_capable: bool,
}

/// Reads the tables of the MSI at `file` (no cabinet is decoded).
pub fn analyze(file: &Path) -> Result<MsiFacts, ExtractError> {
    let src = BufReader::new(
        File::open(file).map_err(|e| ExtractError::Io(format!("abrir {}: {e}", file.display())))?,
    );
    // D4-1: the OLE and table parsers of `msi` are third-party code reading external data.
    let run = catch_decoder(|| {
        let mut pkg = Package::open(src).map_err(|e| ExtractError::Archive(format!("MSI: {e}")))?;
        facts(&mut pkg)
    });
    match run {
        Ok(r) => r,
        Err(payload) => Err(ExtractError::Decoder(panic_message(payload.as_ref()))),
    }
}

fn push<T: PartialEq>(v: &mut Vec<T>, x: T) {
    if !v.contains(&x) {
        v.push(x);
    }
}

fn facts(pkg: &mut Pkg) -> Result<MsiFacts, ExtractError> {
    let mut blockers = Vec::new();
    let mut losses = Vec::new();

    for r in rows(pkg, "ServiceInstall")? {
        push(
            &mut blockers,
            Blocker::Service(text(&r, "Name").unwrap_or_default()),
        );
    }
    for r in rows(pkg, "MsiDriverPackages")? {
        push(
            &mut blockers,
            Blocker::Driver(text(&r, "Component").unwrap_or_default()),
        );
    }

    let sequenced: HashSet<String> = rows(pkg, "InstallExecuteSequence")?
        .iter()
        .filter_map(|r| text(r, "Action"))
        .collect();
    for r in rows(pkg, "CustomAction")? {
        let (Some(action), Some(kind)) = (text(&r, "Action"), int(&r, "Type")) else {
            continue;
        };
        if !sequenced.contains(&action) {
            continue;
        }
        if NESTED_TYPES.contains(&(kind & 0x3F)) {
            push(&mut blockers, Blocker::NestedInstall(action));
        } else if !CODE_TYPES.contains(&(kind & 0x3F)) {
            continue;
        } else if kind & IN_SCRIPT == 0 {
            push(&mut losses, Loss::ImmediateAction(action));
        } else if kind & ROLLBACK_COMMIT == ROLLBACK {
            push(&mut losses, Loss::RollbackAction(action));
        } else if INTERNET_SHORTCUT_ACTIONS.contains(&action.as_str()) {
            push(&mut losses, Loss::InternetShortcuts);
        } else {
            push(&mut blockers, Blocker::DeferredAction(action));
        }
    }

    for table in ["Class", "TypeLib", "AppId"] {
        for r in rows(pkg, table)? {
            if !r.is_empty() {
                push(&mut blockers, Blocker::Com(first_column(&r)));
            }
        }
    }
    for r in rows(pkg, "SelfReg")? {
        push(
            &mut blockers,
            Blocker::Com(text(&r, "File_").unwrap_or_default()),
        );
    }
    let mut system_registry = Vec::new();
    let mut other_registry = 0;
    for r in rows(pkg, "Registry")? {
        let key = text(&r, "Key").unwrap_or_default();
        let lower = key.to_ascii_lowercase();
        let class_key = lower.strip_prefix("software\\classes\\").unwrap_or(&lower);
        let root = int(&r, "Root");
        if COM_KEYS.iter().any(|k| class_key.starts_with(k)) {
            push(&mut blockers, Blocker::Com(key));
        } else if (root == Some(ROOT_HKLM) || root == Some(ROOT_DEPENDENT))
            && lower.starts_with("system\\")
        {
            push(&mut system_registry, Blocker::SystemRegistry(key));
        } else {
            other_registry += 1;
        }
    }
    for r in rows(pkg, "MsiAssembly")? {
        // No `File_Application`: the assembly goes to the global assembly cache.
        if text(&r, "File_Application").is_none_or(|f| f.is_empty()) {
            push(
                &mut blockers,
                Blocker::Gac(text(&r, "Component_").unwrap_or_default()),
            );
        }
    }
    for b in system_registry {
        push(&mut blockers, b);
    }
    if other_registry > 0 {
        push(&mut losses, Loss::Registry(other_registry));
    }

    let key_paths: HashMap<String, String> = rows(pkg, "Component")?
        .iter()
        .filter_map(|r| Some((text(r, "Component")?, text(r, "KeyPath")?)))
        .collect();
    let (dirs, comps, files) = tables(pkg)?;
    let files: Vec<msi_layout::FileRow> = files.into_iter().map(|(f, _)| f).collect();
    let placed = msi_layout::layout(&dirs, &comps, &files)
        .map_err(|e| ExtractError::MsiLayout(e.to_string()))?;
    let parent: HashMap<&str, &str> = dirs
        .iter()
        .filter_map(|d| Some((d.directory.as_str(), d.parent.as_deref()?)))
        .collect();
    let comp_dir: HashMap<&str, &str> = comps
        .iter()
        .map(|c| (c.component.as_str(), c.directory.as_str()))
        .collect();
    let file_comp: HashMap<&str, &str> = files
        .iter()
        .map(|f| (f.file.as_str(), f.component.as_str()))
        .collect();
    let default_dir: HashMap<&str, &str> = dirs
        .iter()
        .map(|d| (d.directory.as_str(), d.default_dir.as_str()))
        .collect();
    // Directory ids from the root down to the folder of `file`.
    let chain = |file: &str| -> Vec<&str> {
        let mut out = Vec::new();
        let mut cur = file_comp.get(file).and_then(|c| comp_dir.get(c)).copied();
        // `layout` already refused cycles; the bound only keeps this walk finite.
        while let Some(d) = cur.filter(|_| out.len() <= dirs.len()) {
            out.push(d);
            cur = parent.get(d).copied().filter(|p| *p != d);
        }
        out.reverse();
        out
    };
    let in_system = |file: &str| chain(file).iter().any(|d| SYSTEM_DIRS.contains(d));
    let (system, app): (Vec<_>, Vec<_>) = placed.into_iter().partition(|p| in_system(&p.file));
    if !system.is_empty() {
        push(&mut blockers, Blocker::SystemFiles(system.len()));
    }
    // Main folder of one file: the path of the first non-standard `Directory` of its chain
    // (`None` when the file sits in a standard folder).
    let app_root = |p: &msi_layout::Placed| -> Option<Vec<String>> {
        let ids = chain(&p.file);
        let i = ids.iter().position(|d| !STANDARD_DIRS.contains(d))?;
        let below = ids[i + 1..]
            .iter()
            .filter(|d| {
                default_dir
                    .get(*d)
                    .is_some_and(|v| msi_layout::source_name(v) != ".")
            })
            .count();
        let dir_len = p.rel.len().checked_sub(1 + below)?;
        Some(p.rel[..dir_len].to_vec())
    };
    let mut roots = app.iter().map(app_root);
    let main_dir = match roots.next() {
        Some(Some(first)) => roots
            .all(|r| {
                r.is_some_and(|r| {
                    r.len() == first.len() && r.iter().zip(&first).all(|(a, b)| same(a, b))
                })
            })
            .then_some(first),
        _ => None,
    };
    if main_dir.is_none() {
        push(&mut blockers, Blocker::Scattered);
    }
    let relative = |rel: &[String]| -> Option<Vec<String>> {
        let dir = main_dir.as_ref()?;
        let inside = rel.len() >= dir.len() && rel.iter().zip(dir).all(|(a, b)| same(a, b));
        inside.then(|| rel[dir.len()..].to_vec())
    };
    let by_key: HashMap<&str, Vec<String>> = app
        .iter()
        .filter_map(|p| Some((p.file.as_str(), relative(&p.rel)?)))
        .collect();
    let mut app_files: Vec<Vec<String>> = by_key.values().cloned().collect();
    app_files.sort();

    let extensions: BTreeSet<String> = rows(pkg, "Extension")?
        .iter()
        .filter_map(|r| text(r, "Extension"))
        .map(|e| format!(".{e}"))
        .collect();
    if !extensions.is_empty() {
        push(
            &mut losses,
            Loss::Associations(extensions.into_iter().collect()),
        );
    }
    let variables: BTreeSet<String> = rows(pkg, "Environment")?
        .iter()
        .filter_map(|r| text(r, "Name"))
        .map(|n| n.trim_start_matches(['=', '-', '+', '!', '*']).to_string())
        .collect();
    if !variables.is_empty() {
        push(
            &mut losses,
            Loss::Environment(variables.into_iter().collect()),
        );
    }

    let mut shortcuts: Vec<(String, String, Vec<String>)> = Vec::new();
    for r in rows(pkg, "Shortcut")? {
        let (Some(key), Some(name)) = (text(&r, "Shortcut"), text(&r, "Name")) else {
            continue;
        };
        let target = text(&r, "Target").unwrap_or_default();
        let file = match target.strip_prefix("[#").and_then(|t| t.strip_suffix(']')) {
            Some(f) => Some(f.to_string()),
            // Advertised shortcut: the target is a Feature, the file is the component's key path.
            None => text(&r, "Component_").and_then(|c| key_paths.get(&c).cloned()),
        };
        let Some(rel) = file.and_then(|f| by_key.get(f.as_str()).cloned()) else {
            continue;
        };
        let long = name
            .split_once('|')
            .map_or(name.as_str(), |(_, l)| l)
            .to_string();
        shortcuts.push((key, long, rel));
    }
    shortcuts.sort_by(|a, b| a.0.cmp(&b.0));

    let properties: HashMap<String, String> = rows(pkg, "Property")?
        .iter()
        .filter_map(|r| Some((text(r, "Property")?, text(r, "Value").unwrap_or_default())))
        .collect();
    let per_user_capable = pkg.summary_info().word_count().unwrap_or(0) & WORD_COUNT_NO_ELEVATION
        != 0
        && (properties.get("ALLUSERS").map(String::as_str) == Some("2")
            || properties.contains_key("MSIINSTALLPERUSER"));

    Ok(MsiFacts {
        blockers,
        losses,
        main_dir,
        files: app_files,
        shortcuts: shortcuts.into_iter().map(|(_, n, r)| (n, r)).collect(),
        per_user_capable,
    })
}

fn first_column(r: &Row) -> String {
    let v = &r[0];
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

/// Windows folder names compare without case (`msiexec /a` writes both into one folder).
fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}
