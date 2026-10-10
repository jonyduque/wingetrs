//! Windows filesystem helpers (junction, in-use detection).
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const ERROR_SHARING_VIOLATION: i32 = 32;

/// `.exe`/`.dll` files under `dir` that another process holds open (sharing violation
/// when opened with no sharing). Unreadable entries are skipped.
pub fn files_in_use(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let p = entry.path();
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() && is_binary(&p) && in_use(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn is_binary(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("dll"))
}

fn in_use(p: &Path) -> bool {
    match OpenOptions::new().read(true).share_mode(0).open(p) {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

/// Creates an NTFS junction `link` -> `target` (no privilege needed, unlike symlinks).
pub fn create_junction(target: &Path, link: &Path) -> std::io::Result<()> {
    junction::create(target, link)
}

/// Target of the junction `link`.
pub fn junction_target(link: &Path) -> std::io::Result<PathBuf> {
    junction::get_target(link)
}

/// Removes the junction itself, never what it points to.
pub fn remove_junction(link: &Path) -> std::io::Result<()> {
    std::fs::remove_dir(link)
}
