//! MSIX install/remove logic (Task 179).
//!
//! Pure parts (URI building, result classification, the removal loop) live
//! here; the WinRT `PackageManager` calls live in `msix_win.rs`.
use super::codes::{self, Outcome};
use std::path::Path;

/// Failure of a deployment operation: the HRESULT and the `ErrorText` reported by Windows.
#[derive(Debug, Clone, Default, PartialEq, Eq, thiserror::Error)]
#[error("{text} (0x{:08x})", *.hresult as u32)]
pub struct MsixError {
    pub hresult: i32,
    pub text: String,
}

/// `HRESULT_FROM_WIN32(ERROR_PACKAGES_IN_USE)`.
pub const ERROR_PACKAGES_IN_USE: i32 = 0x8007_3D02_u32 as i32;

/// How a successful deployment ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddResult {
    /// Registered now.
    Installed,
    /// Staged; Windows registers it when the running app closes.
    Deferred,
}

impl AddResult {
    /// Extra text for the user, if any.
    pub fn user_text(self) -> Option<&'static str> {
        match self {
            AddResult::Installed => None,
            AddResult::Deferred => Some("o app será atualizado quando for fechado"),
        }
    }
}

/// Per-user MSIX deployment (no machine-wide provisioning).
pub trait MsixOps: Send + Sync {
    /// Deploys the package file for the current user, deferring registration
    /// when the app is running (never force-closes it).
    fn add(&self, file: &Path) -> Result<AddResult, MsixError>;
    /// Removes every installed package of the family for the current user; returns how many.
    fn remove(&self, family: &str) -> Result<usize, MsixError>;
}

/// Maps a deployment failure to an outcome through the MSIX table.
pub fn outcome(e: &MsixError) -> Outcome {
    codes::classify_msix(e.hresult)
}

/// Bytes kept literally in a `file:` URI path; everything else is percent-encoded.
fn keep(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&b)
}

/// `file:///C:/a%20b/x.msix` for an absolute drive path; `None` for relative,
/// drive-relative, UNC or non-UTF-8 paths.
pub fn file_uri(path: &Path) -> Option<String> {
    let s = path.to_str()?;
    let s = s.strip_prefix(r"\\?\").unwrap_or(s);
    let b = s.as_bytes();
    let drive = b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/');
    if !drive {
        return None;
    }
    let mut out = String::with_capacity(8 + s.len() * 3);
    out.push_str("file:///");
    for &c in b {
        let c = if c == b'\\' { b'/' } else { c };
        if keep(c) {
            out.push(char::from(c));
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    Some(out)
}

/// The deployment steps of one package file, for the current user.
pub trait Deployer {
    /// `AddPackageAsync(uri, None, DeploymentOptions::None)`.
    fn add(&self) -> Result<(), MsixError>;
    /// Package full name read from the file.
    fn full_name(&self) -> Result<String, MsixError>;
    /// Whether this exact full name is already installed for the user.
    fn is_installed(&self, full_name: &str) -> Result<bool, MsixError>;
    /// `StagePackageAsync(uri)`.
    fn stage(&self) -> Result<(), MsixError>;
    /// `RegisterPackageByFullNameAsync(full_name, None, DeploymentOptions::None)`.
    fn register(&self, full_name: &str) -> Result<(), MsixError>;
    /// `RemovePackageAsync(full_name)`.
    fn remove(&self, full_name: &str) -> Result<(), MsixError>;
}

/// Adds the package; if the app is in use, stages it and registers it, and a
/// registration still blocked by the running app becomes `Deferred`. On another
/// stage/register failure the staged package is removed, but only if that full
/// name was not installed before (unknown ⇒ left alone).
// Port: AppInstallerCommonCore_Deployment.cpp:176-265 AddPackageWithDeferredFallback; InstallFlow.cpp:206-247 MsixInstall.
pub fn add_with_deferred_fallback(d: &impl Deployer) -> Result<AddResult, MsixError> {
    let in_use = match d.add() {
        Ok(()) => return Ok(AddResult::Installed),
        Err(e) if e.hresult == ERROR_PACKAGES_IN_USE => e,
        Err(e) => return Err(e),
    };
    let full_name = match d.full_name() {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(error = %e, "msix: full name unreadable, no deferred fallback");
            return Err(in_use);
        }
    };
    let was_installed = d.is_installed(&full_name).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "msix: install state unknown, cleanup disabled");
        true
    });
    let result = d.stage().and_then(|()| match d.register(&full_name) {
        Ok(()) => Ok(AddResult::Installed),
        Err(e) if e.hresult == ERROR_PACKAGES_IN_USE => Ok(AddResult::Deferred),
        Err(e) => Err(e),
    });
    if result.is_err()
        && !was_installed
        && let Err(e) = d.remove(&full_name)
    {
        tracing::warn!(error = %e, "msix: cleanup of staged package failed");
    }
    result
}

/// Removes each full name in order, stopping at the first failure; returns how many were removed.
// Port: AppInstallerCommonCore_Deployment.cpp RemovePackage (one RemovePackageAsync per full name).
pub fn remove_each(
    full_names: &[String],
    mut remove: impl FnMut(&str) -> Result<(), MsixError>,
) -> Result<usize, MsixError> {
    for name in full_names {
        remove(name)?;
    }
    Ok(full_names.len())
}
