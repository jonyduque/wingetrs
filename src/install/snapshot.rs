//! Deterministic directory pictures, to prove that nothing changed.
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// Junctions and symlinks count as links (std reports both via `is_symlink`) and are never followed.
/// Deterministic picture of a directory tree: relative path -> (kind, sha256 of content or link target).
pub fn snapshot(root: &Path) -> io::Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map_err(io::Error::other)?
            .to_string_lossy()
            .replace('\\', "/");
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            let target = std::fs::read_link(&path)?.to_string_lossy().into_owned();
            out.insert(
                rel,
                format!("link:{}", hex::encode(Sha256::digest(target.as_bytes()))),
            );
        } else if ft.is_dir() {
            out.insert(rel, "dir:".to_string());
            walk(root, &path, out)?;
        } else {
            let bytes = std::fs::read(&path)?;
            out.insert(rel, format!("file:{}", hex::encode(Sha256::digest(&bytes))));
        }
    }
    Ok(())
}
