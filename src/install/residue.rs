//! Residue scan after uninstall: pure classification plus recoverable cleanup.
use crate::install::traits::{Effects, RegHit};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResidueKind {
    RegValue {
        key: String,
        value_name: Option<String>,
    },
    RegKey {
        key: String,
    },
    Folder {
        path: PathBuf,
    },
    Shortcut {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    High,
    Low,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Residue {
    pub kind: ResidueKind,
    pub confidence: Confidence,
    pub preselected: bool,
    pub note: Option<String>,
}

pub struct ResidueInput<'a> {
    pub app: &'a str,
    /// `apps\<app>\` that the uninstall just removed.
    pub removed_dir: &'a Path,
    pub homepage: Option<&'a str>,
    pub reg_hits: &'a [RegHit],
    /// `(.lnk path, target)` pairs from Desktop, Start Menu and Startup.
    pub shortcuts: &'a [(PathBuf, PathBuf)],
    /// Candidate folders under `%APPDATA%`/`%LOCALAPPDATA%`.
    pub folders: &'a [PathBuf],
    /// Candidate subkeys of `HKCU\Software` (HKCU-relative).
    pub software_keys: &'a [String],
    /// The same app is listed as installed by scoop/winget.
    pub other_installs: bool,
}

pub const OTHER_INSTALL_NOTE: &str = "pode ser de outra instalação";

/// Lowercased, backslash-separated path text without trailing separators.
fn norm(s: &str) -> String {
    s.replace('/', "\\")
        .to_lowercase()
        .trim_end_matches('\\')
        .to_string()
}

/// True when `text` mentions `dir` as a whole path (the dir itself or something inside it),
/// not merely a sibling sharing the prefix (`apps\rg` vs `apps\rg-all`).
fn mentions_dir(text: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return false;
    }
    let text = norm(text);
    let mut from = 0;
    while let Some(pos) = text[from..].find(dir) {
        let end = from + pos + dir.len();
        match text[end..].chars().next() {
            None | Some('\\' | '"' | '\'' | ',' | ';' | ' ') => return true,
            _ => {}
        }
        from = from + pos + 1;
        while !text.is_char_boundary(from) {
            from += 1;
        }
    }
    false
}

/// Owner of a `https://github.com/<owner>/<repo>…` homepage.
pub fn github_owner(homepage: &str) -> Option<&str> {
    let lower = homepage.to_ascii_lowercase();
    let rest_at = ["https://github.com/", "http://github.com/"]
        .iter()
        .find(|p| lower.starts_with(*p))
        .map(|p| p.len())?;
    let rest = homepage.get(rest_at..)?;
    let (owner, repo) = rest.split_once('/')?;
    let valid = !owner.is_empty()
        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !repo.is_empty();
    valid.then_some(owner)
}

pub fn classify(input: &ResidueInput<'_>) -> Vec<Residue> {
    let dir = norm(&input.removed_dir.to_string_lossy());
    let mut out: Vec<Residue> = Vec::new();
    let mut push = |r: Residue| {
        if !out.iter().any(|o| o.kind == r.kind) {
            out.push(r);
        }
    };
    let high = |kind| Residue {
        kind,
        confidence: Confidence::High,
        preselected: true,
        note: None,
    };

    for h in input.reg_hits {
        if mentions_dir(&h.data, &dir) {
            push(high(ResidueKind::RegValue {
                key: h.key.clone(),
                value_name: h.value_name.clone(),
            }));
        }
    }
    for (lnk, target) in input.shortcuts {
        if mentions_dir(&target.to_string_lossy(), &dir) {
            push(high(ResidueKind::Shortcut { path: lnk.clone() }));
        }
    }

    let mut names: Vec<&str> = vec![input.app];
    if let Some(owner) = input.homepage.and_then(github_owner) {
        names.push(owner);
    }
    let named = |n: &str| !n.is_empty() && names.iter().any(|x| x.eq_ignore_ascii_case(n));
    let low = |kind| Residue {
        kind,
        confidence: Confidence::Low,
        preselected: false,
        note: input.other_installs.then(|| OTHER_INSTALL_NOTE.to_string()),
    };
    for f in input.folders {
        let leaf = f.file_name().map(|n| n.to_string_lossy());
        if leaf.is_some_and(|n| named(&n)) {
            push(low(ResidueKind::Folder { path: f.clone() }));
        }
    }
    for k in input.software_keys {
        let leaf = k.rsplit('\\').next().unwrap_or_default();
        if named(leaf) {
            push(low(ResidueKind::RegKey { key: k.clone() }));
        }
    }
    out
}

/// `YYYYMMDD-HHMMSSZ` (UTC, hence the `Z`) for the current time.
fn stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Civil-from-days (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// Atomically reserves a fresh `backups\<stamp>` (or `<stamp>-2`, `-3`…), so a later
/// clean never writes into, and overwrites, an earlier backup.
fn reserve_backup_dir(backups: &Path, stamp: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(backups)?;
    let mut n = 1u32;
    loop {
        let dir = match n {
            1 => backups.join(stamp),
            _ => backups.join(format!("{stamp}-{n}")),
        };
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 10_000 => n += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Removes the selected residues recoverably: folders and shortcuts go to the Recycle Bin;
/// registry keys/values are exported to `backups\<stamp>\<app>.reg` first and only deleted
/// when that export succeeded. One result per selected item, in order.
pub fn clean(
    selected: &[Residue],
    app: &str,
    effects: &Effects<'_>,
    backups: &Path,
) -> Vec<Result<(), String>> {
    let stamp = stamp();
    let mut dir_ready: Option<Result<PathBuf, String>> = None;
    // Each distinct key is exported once; the result is reused for later items of that key.
    let mut exported: BTreeMap<String, Result<(), String>> = BTreeMap::new();
    let mut export = |key: &str| -> Result<(), String> {
        if let Some(r) = exported.get(key) {
            return r.clone();
        }
        let ready = dir_ready
            .get_or_insert_with(|| {
                reserve_backup_dir(backups, &stamp).map_err(|e| {
                    format!(
                        "não foi possível criar a pasta de backup em {}: {e}",
                        backups.display()
                    )
                })
            })
            .clone();
        let r = ready.and_then(|dir| {
            let file = match exported.len() {
                0 => dir.join(format!("{app}.reg")),
                n => dir.join(format!("{app}-{}.reg", n + 1)),
            };
            effects
                .registry
                .export(key, &file)
                .map_err(|e| format!("backup de {key} falhou, chave mantida: {e}"))
        });
        exported.insert(key.to_string(), r.clone());
        r
    };

    selected
        .iter()
        .map(|r| match &r.kind {
            ResidueKind::Folder { path } | ResidueKind::Shortcut { path } => {
                effects.trash.recycle(path).map_err(|e| {
                    format!(
                        "não foi possível mover {} para a Lixeira: {e}",
                        path.display()
                    )
                })
            }
            ResidueKind::RegKey { key } => export(key).and_then(|()| {
                effects
                    .registry
                    .delete_key(key)
                    .map_err(|e| format!("não foi possível apagar {key}: {e}"))
            }),
            ResidueKind::RegValue { key, value_name } => export(key).and_then(|()| {
                let name = value_name.as_deref().unwrap_or("");
                effects
                    .registry
                    .delete_value(key, name)
                    .map_err(|e| format!("não foi possível apagar o valor {name:?} de {key}: {e}"))
            }),
        })
        .collect()
}
