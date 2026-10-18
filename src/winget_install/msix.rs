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

/// Per-user MSIX deployment (no machine-wide provisioning).
pub trait MsixOps: Send + Sync {
    /// Deploys the package file for the current user.
    fn add(&self, file: &Path) -> Result<(), MsixError>;
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
