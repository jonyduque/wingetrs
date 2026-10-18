//! PackageManager deployment (Task 179).
//!
//! Every call blocks until the deployment finishes; callers run it in an awaited
//! `spawn_blocking`. User scope only; a running app is never force-closed.
use super::msix::{
    AddResult, Deployer, MsixError, MsixOps, add_with_deferred_fallback, file_uri, remove_each,
};
use std::path::{Path, PathBuf};
use windows::Foundation::Uri;
use windows::Management::Deployment::{DeploymentOptions, DeploymentResult, PackageManager};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::Storage::Packaging::Appx::{
    AppxBundleFactory, AppxFactory, IAppxBundleFactory, IAppxFactory, IAppxManifestPackageId,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, IStream, STGM_READ, STGM_SHARE_DENY_WRITE,
};
use windows::Win32::UI::Shell::SHCreateStreamOnFileEx;
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

/// Per-thread COM apartment for the Appx factories; uninitializes only what it initialized.
struct ComInit {
    owned: bool,
}

impl ComInit {
    fn new() -> Result<ComInit, MsixError> {
        // SAFETY: plain COM initialization for the current thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            // Thread already in an STA: COM is usable, but the init is not ours.
            return Ok(ComInit { owned: false });
        }
        hr.ok().map_err(|e| api_err(&e))?;
        Ok(ComInit { owned: true })
    }
}

impl Drop for ComInit {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: balances the successful CoInitializeEx in `new`.
            unsafe { CoUninitialize() };
        }
    }
}

fn read_stream(file: &Path) -> Result<IStream, MsixError> {
    // SAFETY: the path is a valid NUL-terminated wide string for the call's duration.
    unsafe {
        SHCreateStreamOnFileEx(
            &HSTRING::from(file.as_os_str()),
            (STGM_READ | STGM_SHARE_DENY_WRITE).0,
            FILE_ATTRIBUTE_NORMAL.0,
            false,
            None,
        )
    }
    .map_err(|e| api_err(&e))
}

fn id_full_name(id: &IAppxManifestPackageId) -> Result<String, MsixError> {
    // SAFETY: the returned string is owned by us and freed below with CoTaskMemFree.
    let p = unsafe { id.GetPackageFullName() }.map_err(|e| api_err(&e))?;
    // SAFETY: `p` is a valid NUL-terminated wide string returned by the call above.
    let s = unsafe { p.to_string() };
    // SAFETY: frees the CoTaskMem string returned by GetPackageFullName.
    unsafe { CoTaskMemFree(Some(p.0 as *const core::ffi::c_void)) };
    s.map_err(|e| MsixError {
        hresult: E_INVALIDARG,
        text: e.to_string(),
    })
}

/// Full name of a package or bundle file (bundle ⇒ the bundle's own full name, as winget's MsixInfo).
// Port: AppInstallerMsixInfo (GetPackageFullNameWide) via IAppxFactory / IAppxBundleFactory.
pub fn file_full_name(file: &Path) -> Result<String, MsixError> {
    let _com = ComInit::new()?;
    // SAFETY: COM is initialized above; the factories and readers are used on this thread only.
    let package = unsafe {
        CoCreateInstance::<_, IAppxFactory>(&AppxFactory, None, CLSCTX_INPROC_SERVER)
            .and_then(|f| f.CreatePackageReader(&read_stream(file).map_err(to_win)?))
            .and_then(|r| r.GetManifest())
            .and_then(|m| m.GetPackageId())
    };
    let id = match package {
        Ok(id) => id,
        Err(package_err) => {
            // SAFETY: as above.
            let bundle = unsafe {
                CoCreateInstance::<_, IAppxBundleFactory>(
                    &AppxBundleFactory,
                    None,
                    CLSCTX_INPROC_SERVER,
                )
                .and_then(|f| f.CreateBundleReader(&read_stream(file).map_err(to_win)?))
                .and_then(|r| r.GetManifest())
                .and_then(|m| m.GetPackageId())
            };
            bundle.map_err(|bundle_err| MsixError {
                hresult: package_err.code().0,
                text: format!(
                    "não é um pacote ({}) nem um bundle ({})",
                    package_err.message(),
                    bundle_err.message()
                ),
            })?
        }
    };
    id_full_name(&id)
}

fn to_win(e: MsixError) -> windows::core::Error {
    windows::core::Error::new(windows::core::HRESULT(e.hresult), e.text)
}

/// The real deployment steps for one file.
struct WinDeployer {
    pm: PackageManager,
    uri: Uri,
    file: PathBuf,
}

impl Deployer for WinDeployer {
    // Port: AppInstallerCommonCore_Deployment.cpp StartAddPackage (AddPackageAsync, DeploymentOptions::None).
    fn add(&self) -> Result<(), MsixError> {
        let op = self
            .pm
            .AddPackageAsync(&self.uri, None, DeploymentOptions::None)
            .map_err(|e| api_err(&e))?;
        // Blocking wait (IAsyncOperationWithProgress::join).
        wait(op.join())
    }
    fn full_name(&self) -> Result<String, MsixError> {
        file_full_name(&self.file)
    }
    fn is_installed(&self, full_name: &str) -> Result<bool, MsixError> {
        match self
            .pm
            .FindPackageByUserSecurityIdPackageFullName(&HSTRING::new(), &HSTRING::from(full_name))
        {
            Ok(_) => Ok(true),
            // A null result (not installed) arrives as an empty error with S_OK.
            Err(e) if e.code().is_ok() => Ok(false),
            Err(e) => Err(api_err(&e)),
        }
    }
    // Port: AppInstallerCommonCore_Deployment.cpp StartStagePackage (StagePackageAsync).
    fn stage(&self) -> Result<(), MsixError> {
        let op = self
            .pm
            .StagePackageAsync(&self.uri, None)
            .map_err(|e| api_err(&e))?;
        wait(op.join())
    }
    fn register(&self, full_name: &str) -> Result<(), MsixError> {
        let op = self
            .pm
            .RegisterPackageByFullNameAsync(
                &HSTRING::from(full_name),
                None,
                DeploymentOptions::None,
            )
            .map_err(|e| api_err(&e))?;
        wait(op.join())
    }
    fn remove(&self, full_name: &str) -> Result<(), MsixError> {
        let op = self
            .pm
            .RemovePackageAsync(&HSTRING::from(full_name))
            .map_err(|e| api_err(&e))?;
        wait(op.join())
    }
}

/// Deploys an MSIX file for the current user. A running app is never closed:
/// in use ⇒ stage + register, and a registration still blocked ⇒ `Deferred`.
pub fn add_package(file: &Path) -> Result<AddResult, MsixError> {
    let uri = file_uri(file).ok_or_else(|| MsixError {
        hresult: E_INVALIDARG,
        text: format!("caminho inválido para o MSIX: {}", file.display()),
    })?;
    let uri = Uri::CreateUri(&HSTRING::from(uri)).map_err(|e| api_err(&e))?;
    let pm = PackageManager::new().map_err(|e| api_err(&e))?;
    add_with_deferred_fallback(&WinDeployer {
        pm,
        uri,
        file: file.to_path_buf(),
    })
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
    fn add(&self, file: &Path) -> Result<AddResult, MsixError> {
        add_package(file)
    }
    fn remove(&self, family: &str) -> Result<usize, MsixError> {
        remove_package(family)
    }
}
