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
/// Summary Information word count bit: elevated privileges are not required.
const WORD_COUNT_NO_ELEVATION: i32 = 0x8;
/// `Registry.Root` of `HKEY_LOCAL_MACHINE`.
const ROOT_HKLM: i32 = 2;

/// What portable mode cannot do for the app.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Blocker {
    #[error("instala o serviço do Windows {0}")]
    Service(String),
    #[error("instala driver ({0})")]
    Driver(String),
    #[error("ação personalizada {0} roda código durante a instalação")]
    DeferredAction(String),
    #[error("registra componente COM {0}")]
    Com(String),
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
        if !sequenced.contains(&action) || !CODE_TYPES.contains(&(kind & 0x3F)) {
            continue;
        }
        if kind & IN_SCRIPT != 0 {
            push(&mut blockers, Blocker::DeferredAction(action));
        } else {
            push(&mut losses, Loss::ImmediateAction(action));
        }
    }

    for table in ["Class", "TypeLib", "AppId"] {
        for r in rows(pkg, table)? {
            if !r.is_empty() {
                push(&mut blockers, Blocker::Com(first_column(&r)));
            }
        }
    }
    let mut system_registry = Vec::new();
    let mut other_registry = 0;
    for r in rows(pkg, "Registry")? {
        let key = text(&r, "Key").unwrap_or_default();
        let lower = key.to_ascii_lowercase();
        if lower.starts_with("clsid\\") || lower.starts_with("software\\classes\\clsid\\") {
            push(&mut blockers, Blocker::Com(key));
        } else if int(&r, "Root") == Some(ROOT_HKLM) && lower.starts_with("system\\") {
            push(&mut system_registry, Blocker::SystemRegistry(key));
        } else {
            other_registry += 1;
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
    let in_system = |file: &str| {
        let Some(mut cur) = file_comp.get(file).and_then(|c| comp_dir.get(c)).copied() else {
            return false;
        };
        // `layout` already refused cycles; the bound only keeps this walk finite.
        for _ in 0..=dirs.len() {
            if SYSTEM_DIRS.contains(&cur) {
                return true;
            }
            match parent.get(cur) {
                Some(p) if *p != cur => cur = p,
                _ => break,
            }
        }
        false
    };
    let (system, app): (Vec<_>, Vec<_>) = placed.into_iter().partition(|p| in_system(&p.file));
    if !system.is_empty() {
        push(&mut blockers, Blocker::SystemFiles(system.len()));
    }
    let main_dir = common_dir(app.iter().map(|p| &p.rel[..p.rel.len().saturating_sub(1)]))
        .filter(|d| d.len() >= 2);
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

/// Longest common prefix (by component) of `dirs`; `None` when there is none.
fn common_dir<'a>(mut dirs: impl Iterator<Item = &'a [String]>) -> Option<Vec<String>> {
    let mut common = dirs.next()?.to_vec();
    for d in dirs {
        let n = common.iter().zip(d).take_while(|(a, b)| same(a, b)).count();
        common.truncate(n);
    }
    Some(common)
}
