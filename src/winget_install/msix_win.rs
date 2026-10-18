//! PackageManager deployment (Task 179).
//!
//! Every call blocks until the deployment finishes; callers run it in an awaited
//! `spawn_blocking`. User scope only.
use super::msix::{MsixError, MsixOps, file_uri, remove_each};
use std::path::Path;
use windows::Foundation::Uri;
use windows::Management::Deployment::{DeploymentOptions, DeploymentResult, PackageManager};
use windows::core::HSTRING;

/// E_INVALIDARG: the file path cannot become a `file:` URI.
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;

fn api_err(e: &windows::core::Error) -> MsixError {
    MsixError {
        hresult: e.code().0,
        text: e.message(),
    }
}

/// Turns the joined deployment operation (or its failed `DeploymentResult`) into an error.
// Port: AppInstallerCommonCore_Deployment.cpp WaitForDeployment.
fn wait(joined: windows::core::Result<DeploymentResult>) -> Result<(), MsixError> {
    let result = joined.map_err(|e| api_err(&e))?;
    let code = result.ExtendedErrorCode().map_err(|e| api_err(&e))?;
    if code.is_ok() {
        return Ok(());
    }
    let text = result
        .ErrorText()
        .map(|t| t.to_string())
        .unwrap_or_default();
    Err(MsixError {
        hresult: code.0,
        text,
    })
}

/// Deploys an MSIX file for the current user. No forced shutdown of a running app:
/// a package in use fails with `ERROR_PACKAGES_IN_USE` and the user is asked to close it.
// Port: AppInstallerCommonCore_Deployment.cpp StartAddPackage (AddPackageAsync, DeploymentOptions::None).
pub fn add_package(file: &Path) -> Result<(), MsixError> {
    let uri = file_uri(file).ok_or_else(|| MsixError {
        hresult: E_INVALIDARG,
        text: format!("caminho inválido para o MSIX: {}", file.display()),
    })?;
    let uri = Uri::CreateUri(&HSTRING::from(uri)).map_err(|e| api_err(&e))?;
    let pm = PackageManager::new().map_err(|e| api_err(&e))?;
    let op = pm
        .AddPackageAsync(&uri, None, DeploymentOptions::None)
        .map_err(|e| api_err(&e))?;
    // Blocking wait (IAsyncOperationWithProgress::join).
    wait(op.join())
}

/// Removes every package of `family` installed for the current user; returns how many.
// Port: AppInstallerCommonCore_Deployment.cpp RemovePackage (RemovePackageAsync by full name).
pub fn remove_package(family: &str) -> Result<usize, MsixError> {
    let pm = PackageManager::new().map_err(|e| api_err(&e))?;
    let mut full_names = Vec::new();
    for p in pm
        .FindPackagesByUserSecurityIdPackageFamilyName(&HSTRING::new(), &HSTRING::from(family))
        .map_err(|e| api_err(&e))?
    {
        let name = p
            .Id()
            .and_then(|id| id.FullName())
            .map_err(|e| api_err(&e))?;
        full_names.push(name.to_string());
    }
    remove_each(&full_names, |name| {
        let op = pm
            .RemovePackageAsync(&HSTRING::from(name))
            .map_err(|e| api_err(&e))?;
        wait(op.join())
    })
}

/// `MsixOps` over the real `PackageManager`.
pub struct SystemMsix;

impl MsixOps for SystemMsix {
    fn add(&self, file: &Path) -> Result<(), MsixError> {
        add_package(file)
    }
    fn remove(&self, family: &str) -> Result<usize, MsixError> {
        remove_package(family)
    }
}
