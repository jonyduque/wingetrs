//! MSI source layout from the Directory, Component and File tables (phase 2b, Task 124).

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::install::safepath::safe_components;

/// Deepest Directory chain accepted (the S3 corpus peaked at 10).
const MAX_DEPTH: usize = 64;

pub struct DirRow {
    pub directory: String,
    pub parent: Option<String>,
    pub default_dir: String,
}

pub struct CompRow {
    pub component: String,
    pub directory: String,
}

pub struct FileRow {
    pub file: String,
    pub component: String,
    pub file_name: String,
    pub version: Option<String>,
    pub sequence: i32,
}

/// A file key and its path relative to `SourceDir`.
#[derive(Debug, PartialEq, Eq)]
pub struct Placed {
    pub file: String,
    pub rel: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("nome inseguro no MSI: {0}")]
    Unsafe(String),
    #[error("diretório pai ausente no MSI: {0}")]
    MissingParent(String),
    #[error("ciclo na tabela Directory do MSI: {0}")]
    Cycle(String),
    #[error("diretório aninhado demais no MSI: {0}")]
    TooDeep(String),
    #[error("componente sem diretório no MSI: {0}")]
    MissingDirectory(String),
}

/// Source-side name of a `DefaultDir`: after `:` if present, long form of `short|long`.
fn source_name(default_dir: &str) -> &str {
    let src = default_dir
        .split_once(':')
        .map_or(default_dir, |(_, src)| src);
    long_name(src)
}

fn long_name(v: &str) -> &str {
    v.split_once('|').map_or(v, |(_, long)| long)
}

/// One path component, or `None` for `.`.
fn component(name: &str) -> Result<Option<String>, LayoutError> {
    if name == "." {
        return Ok(None);
    }
    match safe_components(name) {
        Some(mut v) if v.len() == 1 => Ok(v.pop()),
        _ => Err(LayoutError::Unsafe(name.to_string())),
    }
}

fn dir_path(
    key: &str,
    by_key: &HashMap<&str, &DirRow>,
    memo: &mut HashMap<String, Vec<String>>,
) -> Result<Vec<String>, LayoutError> {
    if let Some(p) = memo.get(key) {
        return Ok(p.clone());
    }
    // Walk up to the root (no parent or parent == self), collecting source names.
    let mut chain: Vec<&DirRow> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut cur = key;
    let mut base = Vec::new();
    loop {
        if let Some(p) = memo.get(cur) {
            base = p.clone();
            break;
        }
        let row = by_key
            .get(cur)
            .ok_or_else(|| LayoutError::MissingParent(cur.to_string()))?;
        if !seen.insert(cur) {
            return Err(LayoutError::Cycle(cur.to_string()));
        }
        match row.parent.as_deref() {
            None => break,
            Some(parent) if parent == row.directory => break,
            Some(parent) => {
                if chain.len() >= MAX_DEPTH {
                    return Err(LayoutError::TooDeep(key.to_string()));
                }
                chain.push(row);
                cur = parent;
            }
        }
    }
    let mut path = base;
    for row in chain.iter().rev() {
        if let Some(c) = component(source_name(&row.default_dir))? {
            path.push(c);
        }
        memo.insert(row.directory.clone(), path.clone());
    }
    memo.entry(key.to_string()).or_insert_with(|| path.clone());
    Ok(path)
}

/// Numeric parts of a file version; `None` when unversioned (empty or a companion-file key).
fn version_parts(v: Option<&str>) -> Option<Vec<u64>> {
    let v = v?;
    if !v.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some(
        v.split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect(),
    )
}

fn cmp_versions(a: &[u64], b: &[u64]) -> Ordering {
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| {
            let x = a.get(i).copied().unwrap_or(0);
            let y = b.get(i).copied().unwrap_or(0);
            x.cmp(&y)
        })
        .find(|o| o.is_ne())
        .unwrap_or(Ordering::Equal)
}

/// `msiexec /a` collision rule measured in S3: versioned beats unversioned, higher
/// `Version` wins, tie goes to the lower `Sequence`.
fn beats(a: &FileRow, b: &FileRow) -> bool {
    match (
        version_parts(a.version.as_deref()),
        version_parts(b.version.as_deref()),
    ) {
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (Some(x), Some(y)) if cmp_versions(&x, &y).is_ne() => cmp_versions(&x, &y).is_gt(),
        _ => a.sequence < b.sequence,
    }
}

/// Places every file under `SourceDir` the way `msiexec /a` does, sorted by `Sequence`.
pub fn layout(
    dirs: &[DirRow],
    comps: &[CompRow],
    files: &[FileRow],
) -> Result<Vec<Placed>, LayoutError> {
    let by_key: HashMap<&str, &DirRow> = dirs.iter().map(|d| (d.directory.as_str(), d)).collect();
    let comp_dir: HashMap<&str, &str> = comps
        .iter()
        .map(|c| (c.component.as_str(), c.directory.as_str()))
        .collect();
    let mut memo = HashMap::new();
    // Collision key (lowercased path) -> (winning row, its path).
    let mut slots: HashMap<String, (&FileRow, Vec<String>)> = HashMap::new();
    for f in files {
        let dir = comp_dir
            .get(f.component.as_str())
            .ok_or_else(|| LayoutError::MissingDirectory(f.component.clone()))?;
        if !by_key.contains_key(dir) {
            return Err(LayoutError::MissingDirectory(f.component.clone()));
        }
        let mut rel = dir_path(dir, &by_key, &mut memo)?;
        let name = long_name(&f.file_name);
        match component(name)? {
            Some(c) => rel.push(c),
            None => return Err(LayoutError::Unsafe(f.file_name.clone())),
        }
        let key = rel.join("\\").to_lowercase();
        match slots.get(&key) {
            Some((cur, _)) if !beats(f, cur) => {}
            _ => {
                slots.insert(key, (f, rel));
            }
        }
    }
    let mut out: Vec<(&FileRow, Vec<String>)> = slots.into_values().collect();
    out.sort_by(|a, b| {
        a.0.sequence
            .cmp(&b.0.sequence)
            .then(a.0.file.cmp(&b.0.file))
    });
    Ok(out
        .into_iter()
        .map(|(f, rel)| Placed {
            file: f.file.clone(),
            rel,
        })
        .collect())
}
