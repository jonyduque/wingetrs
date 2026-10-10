//! Windows registry scan (HKCU only).
use std::path::Path;
use std::process::Command;

use winreg::RegKey;
use winreg::enums::{
    HKEY_CURRENT_USER, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_EXPAND_SZ,
    REG_SZ,
};

use super::traits::{EffectError, RegHit, RegistryScan};

const MAX_DEPTH: u32 = 8;
const KEY_SCAN: u32 = KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS;

/// HKCU scanner; with `WINGETRS_TEST_ENV_KEY` set every key is relative to it.
pub struct RegScanner {
    base: Option<String>,
}

impl RegScanner {
    pub fn from_env() -> RegScanner {
        RegScanner {
            base: std::env::var("WINGETRS_TEST_ENV_KEY")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }

    /// Scanner rooted at an explicit HKCU-relative base (tests).
    pub fn with_base(base: &str) -> RegScanner {
        RegScanner {
            base: Some(base.to_string()).filter(|b| !b.is_empty()),
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
        let _span =
            tracing::info_span!("reg_values_containing", roots = roots.len(), threads).entered();
        let needle = needle.to_lowercase();
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let mut raw: Vec<RawHit> = Vec::new();
        // Work items in sequential-walk order: (display path, full path, depth, values only).
        // Subtrees are split two levels deep so a few huge subtrees (e.g. `Classes\*`) do not
        // serialise the scan.
        let mut tasks: Vec<(String, String, u32, bool)> = Vec::new();
        for root in roots {
            // A missing root is not an error: nothing to find.
            let Ok(k) = hkcu.open_subkey_with_flags(self.full(root), KEY_SCAN) else {
                continue;
            };
            scan_values(&k, root, &needle, &mut raw);
            for sub in k.enum_keys().flatten() {
                let path = format!("{root}\\{sub}");
                let full = format!("{}\\{sub}", self.full(root));
                let Ok(sk) = k.open_subkey_with_flags(&sub, KEY_SCAN) else {
                    continue;
                };
                tasks.push((path.clone(), full.clone(), 1, true));
                for sub2 in sk.enum_keys().flatten() {
                    tasks.push((
                        format!("{path}\\{sub2}"),
                        format!("{full}\\{sub2}"),
                        2,
                        false,
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
                let Some((path, full, depth, values_only)) = tasks.get(i) else {
                    break;
                };
                let mut out = Vec::new();
                if let Ok(k) = hkcu.open_subkey_with_flags(full, KEY_SCAN) {
                    if *values_only {
                        scan_values(&k, path, &needle, &mut out);
                    } else {
                        walk(&k, path, &needle, *depth, &mut out);
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

fn walk(key: &RegKey, path: &str, needle: &str, depth: u32, out: &mut Vec<RawHit>) {
    scan_values(key, path, needle, out);
    if depth >= MAX_DEPTH {
        return;
    }
    for sub in key.enum_keys().flatten() {
        if let Ok(k) = key.open_subkey_with_flags(&sub, KEY_SCAN) {
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
        let Ok(k) = hkcu.open_subkey_with_flags(self.full(parent), KEY_SCAN) else {
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
        let out = Command::new("reg")
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
            .open_subkey_with_flags(self.full(key), KEY_SET_VALUE)
            .map_err(|e| eff("abrir chave", e))?;
        k.delete_value(value_name)
            .map_err(|e| eff("apagar valor", e))
    }

    fn delete_key(&self, key: &str) -> Result<(), EffectError> {
        if key.trim_matches('\\').is_empty() {
            return Err(EffectError("registro: chave vazia recusada".into()));
        }
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        hkcu.delete_subkey_all(self.full(key))
            .map_err(|e| eff("apagar chave", e))
    }
}
