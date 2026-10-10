//! Windows registry scan (HKCU only).
use std::path::Path;
use std::process::Command;

use winreg::RegKey;
use winreg::enums::{
    HKEY_CURRENT_USER, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ,
    REG_OPTION_OPEN_LINK, REG_SZ,
};

use super::env_win::EMPTY_TEST_KEY;
use super::traits::{EffectError, RegHit, RegistryScan};

const MAX_DEPTH: u32 = 8;
const KEY_SCAN: u32 = KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS;

/// Opens a subkey for scanning without following registry symbolic links: a link key is
/// opened as itself (a `REG_LINK` value, no string data, no subkeys), so a link planted
/// inside a scanned tree cannot redirect the walk elsewhere.
fn open_nolink<P: AsRef<std::ffi::OsStr>>(key: &RegKey, path: P) -> std::io::Result<RegKey> {
    key.open_subkey_with_options_flags(path, REG_OPTION_OPEN_LINK, KEY_SCAN)
}

fn is_link(key: &RegKey) -> bool {
    key.enum_values()
        .flatten()
        .any(|(n, _)| n == "SymbolicLinkValue")
}

/// HKCU scanner; with `WINGETRS_TEST_ENV_KEY` set every key is relative to it.
pub struct RegScanner {
    base: Option<String>,
}

impl RegScanner {
    /// Reads `WINGETRS_TEST_ENV_KEY`; present (even empty) means a test base, never real HKCU.
    pub fn from_env() -> RegScanner {
        let var = std::env::var_os("WINGETRS_TEST_ENV_KEY");
        let var = var.as_ref().map(|v| v.to_string_lossy());
        RegScanner {
            base: RegScanner::resolve_base(var.as_deref()),
        }
    }

    /// `None` = real HKCU; `Some("")` falls back to a fixed test key so an empty base can
    /// never reach the real registry.
    pub fn resolve_base(var: Option<&str>) -> Option<String> {
        var.map(|v| {
            if v.is_empty() {
                EMPTY_TEST_KEY.to_string()
            } else {
                v.to_string()
            }
        })
    }

    /// Scanner over the real HKCU. The only way to get one besides an unset env variable.
    pub fn real() -> RegScanner {
        RegScanner { base: None }
    }

    /// Scanner rooted at an explicit HKCU-relative base (tests); empty base is never real HKCU.
    pub fn with_base(base: &str) -> RegScanner {
        RegScanner {
            base: RegScanner::resolve_base(Some(base)),
        }
    }

    /// Scan with `threads` workers over the first-level subkeys of each root; result order is
    /// the same for any thread count (roots in order, subkeys in enumeration order).
    pub fn values_containing_with(
        &self,
        roots: &[&str],
        needle: &str,
        threads: usize,
    ) -> Vec<RegHit> {
        self.scan(roots, needle, threads, true)
    }

    /// Exhaustive recursive scan of every root, including `Classes` (coverage proof and
    /// A/B reference for the targeted scan).
    pub fn values_containing_full(
        &self,
        roots: &[&str],
        needle: &str,
        threads: usize,
    ) -> Vec<RegHit> {
        self.scan(roots, needle, threads, false)
    }

    fn scan(&self, roots: &[&str], needle: &str, threads: usize, targeted: bool) -> Vec<RegHit> {
        let _span = tracing::info_span!(
            "reg_values_containing",
            roots = roots.len(),
            threads,
            targeted
        )
        .entered();
        let needle = needle.to_lowercase();
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let mut raw: Vec<RawHit> = Vec::new();
        // Work items in sequential-walk order. Subtrees are split two levels deep so a few
        // huge subtrees do not serialise the scan.
        let mut tasks: Vec<(String, String, Kind)> = Vec::new();
        for root in roots {
            // A missing root is not an error: nothing to find.
            let Ok(k) = open_nolink(&hkcu, self.full(root)) else {
                continue;
            };
            scan_values(&k, root, &needle, &mut raw);
            let classes = targeted && is_classes_root(root);
            for sub in k.enum_keys().flatten() {
                let path = format!("{root}\\{sub}");
                let full = format!("{}\\{sub}", self.full(root));
                if classes {
                    // spec §8: associations, shell verbs and ProgIDs only.
                    if PRUNED_CLASSES.iter().any(|p| p.eq_ignore_ascii_case(&sub)) {
                        continue;
                    }
                    if WALKED_CLASSES.iter().any(|p| p.eq_ignore_ascii_case(&sub)) {
                        tasks.push((path, full, Kind::Walk(1)));
                        continue;
                    }
                    if CLASSES_CONTAINERS
                        .iter()
                        .any(|p| p.eq_ignore_ascii_case(&sub))
                    {
                        tasks.push((path.clone(), full.clone(), Kind::Values));
                        if let Ok(sk) = open_nolink(&k, &sub) {
                            for sub2 in sk.enum_keys().flatten() {
                                tasks.push((
                                    format!("{path}\\{sub2}"),
                                    format!("{full}\\{sub2}"),
                                    Kind::Targeted,
                                ));
                            }
                        }
                    } else {
                        tasks.push((path, full, Kind::Targeted));
                    }
                    continue;
                }
                let Ok(sk) = open_nolink(&k, &sub) else {
                    continue;
                };
                tasks.push((path.clone(), full.clone(), Kind::Values));
                for sub2 in sk.enum_keys().flatten() {
                    tasks.push((
                        format!("{path}\\{sub2}"),
                        format!("{full}\\{sub2}"),
                        Kind::Walk(2),
                    ));
                }
            }
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let work = || {
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let mut done: Vec<(usize, Vec<RawHit>)> = Vec::new();
            loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((path, full, kind)) = tasks.get(i) else {
                    break;
                };
                let mut out = Vec::new();
                if let Ok(k) = open_nolink(&hkcu, full) {
                    match kind {
                        Kind::Values => scan_values(&k, path, &needle, &mut out),
                        Kind::Walk(depth) => walk(&k, path, &needle, *depth, &mut out),
                        Kind::Targeted => targeted_class(&k, path, &needle, &mut out),
                    }
                }
                done.push((i, out));
            }
            done
        };
        let mut all: Vec<(usize, Vec<RawHit>)> = Vec::new();
        if threads <= 1 {
            all = work();
        } else {
            std::thread::scope(|s| {
                let handles: Vec<_> = (0..threads).map(|_| s.spawn(work)).collect();
                for h in handles {
                    match h.join() {
                        Ok(part) => all.extend(part),
                        Err(p) => std::panic::resume_unwind(p),
                    }
                }
            });
        }
        all.sort_by_key(|(i, _)| *i);
        // Root-level values first, then subkey results in enumeration order.
        for (_, part) in all {
            raw.extend(part);
        }
        raw.into_iter()
            .map(|(key, value_name, data)| RegHit {
                key,
                value_name,
                data,
            })
            .collect()
    }

    /// Full HKCU-relative path for a scanner-relative key.
    fn full(&self, key: &str) -> String {
        match &self.base {
            Some(b) if key.is_empty() => b.clone(),
            Some(b) => format!("{b}\\{key}"),
            None => key.to_string(),
        }
    }
}

fn eff(what: &str, e: impl std::fmt::Display) -> EffectError {
    EffectError(format!("registro: {what}: {e}"))
}

type RawHit = (String, Option<String>, String);

fn scan_values(key: &RegKey, path: &str, needle: &str, out: &mut Vec<RawHit>) {
    for (name, val) in key.enum_values().flatten() {
        if val.vtype != REG_SZ && val.vtype != REG_EXPAND_SZ {
            continue;
        }
        let Some(data) = decode_sz(&val.bytes) else {
            continue;
        };
        if data.to_lowercase().contains(needle) {
            out.push((path.to_string(), (!name.is_empty()).then_some(name), data));
        }
    }
}

enum Kind {
    /// Values of the key only.
    Values,
    /// Full recursive walk starting at this depth.
    Walk(u32),
    /// Association/shell-verb subset of a `Classes` child.
    Targeted,
}

/// `Classes` children never scanned: not associations (CLSID registrations, caches, ...).
const PRUNED_CLASSES: &[&str] = &[
    "Local Settings",
    "CLSID",
    "Interface",
    "TypeLib",
    "WOW6432Node",
    "shellex",
];
/// `Classes` children whose own children are the ProgID-like keys.
const MAX_NESTED_SHELL: u32 = 3;
/// `Classes` children small enough to walk completely (toast icon registrations).
const WALKED_CLASSES: &[&str] = &["AppUserModelId"];
const CLASSES_CONTAINERS: &[&str] = &["Applications", "SystemFileAssociations"];

fn is_classes_root(root: &str) -> bool {
    root.rsplit('\\')
        .next()
        .is_some_and(|l| l.eq_ignore_ascii_case("Classes"))
}

/// Opens the child of `key` named `name` (case-insensitive) and returns it with the name as
/// stored, so reported paths match what a full walk reports.
fn open_ci(key: &RegKey, name: &str) -> Option<(String, RegKey)> {
    let actual = key
        .enum_keys()
        .flatten()
        .find(|k| k.eq_ignore_ascii_case(name))?;
    let sub = open_nolink(key, &actual).ok()?;
    Some((actual, sub))
}

/// A ProgID/extension/`*`/`Directory` key: own values, `DefaultIcon`, `shell\<verb>` (+ its
/// `command`) and `Background\shell\<verb>` (+ `command`). Never `shellex`.
fn targeted_class(key: &RegKey, path: &str, needle: &str, out: &mut Vec<RawHit>) {
    scan_values(key, path, needle, out);
    if let Some((n, k)) = open_ci(key, "DefaultIcon") {
        scan_values(&k, &format!("{path}\\{n}"), needle, out);
    }
    if let Some((n, k)) = open_ci(key, "Application") {
        scan_values(&k, &format!("{path}\\{n}"), needle, out);
    }
    targeted_shell(key, path, needle, out);
    if let Some((n, bg)) = open_ci(key, "Background") {
        targeted_shell(&bg, &format!("{path}\\{n}"), needle, out);
    }
}

fn targeted_shell(key: &RegKey, path: &str, needle: &str, out: &mut Vec<RawHit>) {
    targeted_shell_at(key, path, needle, 0, out);
}

/// Cascading verbs (`shell\\v\\shell\\w\\command`) nest up to `MAX_NESTED_SHELL` levels.
fn targeted_shell_at(key: &RegKey, path: &str, needle: &str, nest: u32, out: &mut Vec<RawHit>) {
    let Some((sn, shell)) = open_ci(key, "shell") else {
        return;
    };
    let spath = format!("{path}\\{sn}");
    scan_values(&shell, &spath, needle, out);
    for verb in shell.enum_keys().flatten() {
        let Ok(vk) = open_nolink(&shell, &verb) else {
            continue;
        };
        let vpath = format!("{spath}\\{verb}");
        scan_values(&vk, &vpath, needle, out);
        if let Some((cn, cmd)) = open_ci(&vk, "command") {
            scan_values(&cmd, &format!("{vpath}\\{cn}"), needle, out);
        }
        if nest < MAX_NESTED_SHELL {
            targeted_shell_at(&vk, &vpath, needle, nest + 1, out);
        }
    }
}

fn walk(key: &RegKey, path: &str, needle: &str, depth: u32, out: &mut Vec<RawHit>) {
    scan_values(key, path, needle, out);
    if depth >= MAX_DEPTH {
        return;
    }
    for sub in key.enum_keys().flatten() {
        if let Ok(k) = open_nolink(key, &sub) {
            walk(&k, &format!("{path}\\{sub}"), needle, depth + 1, out);
        }
    }
}

fn decode_sz(bytes: &[u8]) -> Option<String> {
    let (pairs, _) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16(&units[..end]).ok()
}

impl RegistryScan for RegScanner {
    fn values_containing(&self, roots: &[&str], needle: &str) -> Result<Vec<RegHit>, EffectError> {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
        Ok(self.values_containing_with(roots, needle, threads))
    }

    fn subkeys_named(&self, parent: &str, names: &[&str]) -> Result<Vec<String>, EffectError> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(k) = open_nolink(&hkcu, self.full(parent)) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for sub in k.enum_keys().flatten() {
            if names.iter().any(|n| n.eq_ignore_ascii_case(&sub)) {
                out.push(format!("{parent}\\{sub}"));
            }
        }
        Ok(out)
    }

    fn export(&self, key: &str, file: &Path) -> Result<(), EffectError> {
        let out = Command::new(reg_exe())
            .arg("export")
            .arg(format!("HKCU\\{}", self.full(key)))
            .arg(file)
            .arg("/y")
            .output()
            .map_err(|e| eff("reg.exe", e))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(eff(
                "exportar",
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ))
        }
    }

    fn delete_value(&self, key: &str, value_name: &str) -> Result<(), EffectError> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let k = hkcu
            .open_subkey_with_options_flags(self.full(key), REG_OPTION_OPEN_LINK, KEY_SET_VALUE)
            .map_err(|e| eff("abrir chave", e))?;
        k.delete_value(value_name)
            .map_err(|e| eff("apagar valor", e))
    }

    fn delete_key(&self, key: &str) -> Result<(), EffectError> {
        if key.trim_matches('\\').is_empty() {
            return Err(EffectError("registro: chave vazia recusada".into()));
        }
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        // Refuse to delete through a registry symbolic link.
        let full = self.full(key);
        if let Ok(k) = open_nolink(&hkcu, &full)
            && is_link(&k)
        {
            return Err(EffectError("registro: link simbólico recusado".into()));
        }
        hkcu.delete_subkey_all(full)
            .map_err(|e| eff("apagar chave", e))
    }
}

/// `reg.exe` by absolute path (no PATH search).
fn reg_exe() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    std::path::Path::new(&root).join("System32").join("reg.exe")
}
