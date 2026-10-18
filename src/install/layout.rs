//! Path layout of the wingetrs root.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Layout {
        Layout { root: root.into() }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn apps(&self) -> PathBuf {
        self.root.join("apps")
    }
    pub fn app_dir(&self, app: &str) -> PathBuf {
        self.apps().join(app)
    }
    pub fn version_dir(&self, app: &str, v: &str) -> PathBuf {
        self.app_dir(app).join(v)
    }
    pub fn staging_dir(&self, app: &str, v: &str) -> PathBuf {
        self.app_dir(app).join(format!(".staging-{v}"))
    }
    pub fn current(&self, app: &str) -> PathBuf {
        self.app_dir(app).join("current")
    }
    pub fn persist_dir(&self, app: &str) -> PathBuf {
        self.root.join("persist").join(app)
    }
    pub fn shims(&self) -> PathBuf {
        self.root.join("shims")
    }
    /// Folder of PowerShell module junctions (`psmodule`).
    pub fn modules(&self) -> PathBuf {
        self.root.join("modules")
    }
    pub fn launcher(&self) -> PathBuf {
        self.shims().join(".launcher").join("shim.exe")
    }
    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub fn state_db(&self) -> PathBuf {
        self.root.join("state.db")
    }
    pub fn install_lock(&self) -> PathBuf {
        self.root.join("install.lock")
    }
    /// Orders the Windows Installer runs of wingetrs and its bridge winget jobs.
    pub fn msi_lock(&self) -> PathBuf {
        self.root.join("msi.lock")
    }
    /// Verbose log of attempt `n` of `msiexec /a` for `app`.
    pub fn msiexec_log(&self, app: &str, n: u32) -> PathBuf {
        self.root.join("log").join(format!("msiexec-{app}-{n}.log"))
    }
}
