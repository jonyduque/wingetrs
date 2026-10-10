//! Directory links behind a platform-neutral API: NTFS junctions on Windows, symlinks elsewhere.
#[cfg(not(windows))]
mod imp {
    use std::path::{Path, PathBuf};
    pub fn create(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }
    pub fn target(link: &Path) -> std::io::Result<PathBuf> {
        std::fs::read_link(link)
    }
    pub fn remove(link: &Path) -> std::io::Result<()> {
        std::fs::remove_file(link)
    }
}

#[cfg(windows)]
mod imp {
    pub use crate::install::fs_win::{
        create_junction as create, junction_target as target, remove_junction as remove,
    };
}

pub use imp::{create, remove, target};
