//! Windows shortcuts.
//!
//! `.lnk` files via `IShellLinkW` + `IPersistFile`, under
//! `Start Menu\Programs\wingetrs Apps` (or `WINGETRS_TEST_SHORTCUT_DIR`).
use crate::install::traits::{EffectError, Shortcuts};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, IPersistFile, STGM_READ,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Programs, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellLink,
};
use windows::core::{HSTRING, Interface};

const TEST_DIR_VAR: &str = "WINGETRS_TEST_SHORTCUT_DIR";
const ROOT_NAME: &str = "wingetrs Apps";

pub struct ComShortcuts {
    root: PathBuf,
}

impl ComShortcuts {
    /// Root from `WINGETRS_TEST_SHORTCUT_DIR`, else `<FOLDERID_Programs>\wingetrs Apps`.
    pub fn new() -> Result<ComShortcuts, EffectError> {
        if let Some(dir) = ComShortcuts::test_root(std::env::var_os(TEST_DIR_VAR))? {
            return Ok(ComShortcuts::with_root(dir));
        }
        Ok(ComShortcuts::with_root(programs_dir()?.join(ROOT_NAME)))
    }

    /// Raw `WINGETRS_TEST_SHORTCUT_DIR` to a test root. Present but empty is an error:
    /// a test must never fall through to the real Start Menu.
    pub fn test_root(var: Option<OsString>) -> Result<Option<PathBuf>, EffectError> {
        match var {
            None => Ok(None),
            Some(v) if v.is_empty() => Err(EffectError(format!("{TEST_DIR_VAR} está vazia"))),
            Some(v) => Ok(Some(PathBuf::from(v))),
        }
    }

    pub fn with_root(root: impl Into<PathBuf>) -> ComShortcuts {
        ComShortcuts { root: root.into() }
    }

    /// `link` must be strictly below the root, with no `..`/`.` components.
    fn check_inside(&self, link: &Path) -> Result<(), EffectError> {
        let rel = link
            .strip_prefix(&self.root)
            .map_err(|_| outside(link, &self.root))?;
        let ok = rel.components().count() > 0
            && rel.components().all(|c| matches!(c, Component::Normal(_)));
        if ok {
            Ok(())
        } else {
            Err(outside(link, &self.root))
        }
    }
}

fn outside(link: &Path, root: &Path) -> EffectError {
    EffectError(format!(
        "atalho {} fora de {}",
        link.display(),
        root.display()
    ))
}

fn com_err(what: &str, e: windows::core::Error) -> EffectError {
    EffectError(format!("falha ao {what}: {e}"))
}

fn programs_dir() -> Result<PathBuf, EffectError> {
    // SAFETY: the returned buffer is NUL-terminated and freed with CoTaskMemFree below.
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None)
            .map_err(|e| com_err("localizar a pasta Programas do Menu Iniciar", e))?;
        let s = p.to_string();
        CoTaskMemFree(Some(p.0 as *const _));
        let s = s.map_err(|e| EffectError(format!("pasta Programas inválida: {e}")))?;
        Ok(PathBuf::from(s))
    }
}

/// Per-thread COM apartment; uninitializes only what it initialized.
struct ComInit {
    owned: bool,
}

impl ComInit {
    fn new() -> Result<ComInit, EffectError> {
        // SAFETY: plain COM initialization for the current thread.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            // Thread already in MTA: COM is usable, but the init is not ours.
            return Ok(ComInit { owned: false });
        }
        hr.ok().map_err(|e| com_err("inicializar COM", e))?;
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

fn shell_link() -> Result<IShellLinkW, EffectError> {
    // SAFETY: COM is initialized by the caller's ComInit.
    unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .map_err(|e| com_err("criar IShellLinkW", e))
}

/// Target path stored in an existing `.lnk` (`IShellLinkW::GetPath`).
pub fn link_target(link: &Path) -> Result<PathBuf, EffectError> {
    let _com = ComInit::new()?;
    let sl = shell_link()?;
    let mut buf = [0u16; 32768];
    // SAFETY: COM initialized; `buf` outlives the call; null find-data is allowed.
    unsafe {
        let pf: IPersistFile = sl.cast().map_err(|e| com_err("obter IPersistFile", e))?;
        pf.Load(&HSTRING::from(link.as_os_str()), STGM_READ)
            .map_err(|e| com_err("ler o atalho", e))?;
        sl.GetPath(&mut buf, std::ptr::null_mut(), 0)
            .map_err(|e| com_err("ler o alvo do atalho", e))?;
    }
    let end = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
    Ok(PathBuf::from(String::from_utf16_lossy(&buf[..end])))
}

impl Shortcuts for ComShortcuts {
    fn create(
        &self,
        link: &Path,
        target: &Path,
        args: Option<&str>,
        icon: Option<&Path>,
    ) -> Result<(), EffectError> {
        self.check_inside(link)?;
        if let Some(parent) = link.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| EffectError(format!("falha ao criar {}: {e}", parent.display())))?;
        }
        let _com = ComInit::new()?;
        let sl = shell_link()?;
        // SAFETY: COM initialized; all strings are owned HSTRINGs alive for each call.
        unsafe {
            sl.SetPath(&HSTRING::from(target.as_os_str()))
                .map_err(|e| com_err("definir o alvo do atalho", e))?;
            if let Some(dir) = target.parent() {
                sl.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))
                    .map_err(|e| com_err("definir a pasta de trabalho", e))?;
            }
            if let Some(a) = args {
                sl.SetArguments(&HSTRING::from(a))
                    .map_err(|e| com_err("definir os argumentos", e))?;
            }
            if let Some(i) = icon {
                sl.SetIconLocation(&HSTRING::from(i.as_os_str()), 0)
                    .map_err(|e| com_err("definir o ícone", e))?;
            }
            let pf: IPersistFile = sl.cast().map_err(|e| com_err("obter IPersistFile", e))?;
            pf.Save(&HSTRING::from(link.as_os_str()), true)
                .map_err(|e| com_err("gravar o atalho", e))?;
        }
        Ok(())
    }

    fn remove(&self, link: &Path) -> Result<(), EffectError> {
        self.check_inside(link)?;
        match std::fs::remove_file(link) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(EffectError(format!(
                    "falha ao apagar {}: {e}",
                    link.display()
                )));
            }
        }
        // Prune empty folders up to the root; the root itself only if it is "wingetrs Apps".
        let mut dir = link.parent();
        while let Some(d) = dir {
            let is_root = d == self.root;
            if is_root && self.root.file_name() != Some(std::ffi::OsStr::new(ROOT_NAME)) {
                break;
            }
            let empty = std::fs::read_dir(d)
                .map(|mut it| it.next().is_none())
                .unwrap_or(false);
            if !empty || std::fs::remove_dir(d).is_err() || is_root {
                break;
            }
            dir = d.parent();
        }
        Ok(())
    }

    fn root(&self) -> PathBuf {
        self.root.clone()
    }
}
