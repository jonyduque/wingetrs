//! Windows recycle bin.
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::{
    FOF_ALLOWUNDO, FOF_NO_UI, FOFX_RECYCLEONDELETE, FileOperation, IFileOperation, IShellItem,
    SHCreateItemFromParsingName,
};
use windows::core::PCWSTR;

use super::traits::{EffectError, Trash};

/// Sends files and directories to the Recycle Bin through `IFileOperation`.
pub struct ShellTrash;

struct ComGuard(bool);
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: paired with a successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

fn err(what: &str, e: windows::core::Error) -> EffectError {
    EffectError(format!("Lixeira: {what}: {e}"))
}

impl Trash for ShellTrash {
    fn recycle(&self, path: &Path) -> Result<(), EffectError> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: COM calls on a thread initialised below; `wide` is NUL terminated and
        // outlives the calls.
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let _guard = ComGuard(hr.is_ok());
            let op: IFileOperation =
                CoCreateInstance(&FileOperation, None, CLSCTX_ALL).map_err(|e| err("criar", e))?;
            op.SetOperationFlags(FOF_ALLOWUNDO | FOF_NO_UI | FOFX_RECYCLEONDELETE)
                .map_err(|e| err("flags", e))?;
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
                .map_err(|e| err("abrir item", e))?;
            op.DeleteItem(&item, None).map_err(|e| err("excluir", e))?;
            op.PerformOperations().map_err(|e| err("executar", e))?;
            if op
                .GetAnyOperationsAborted()
                .map_err(|e| err("estado", e))?
                .as_bool()
            {
                return Err(EffectError(format!(
                    "Lixeira: operação abortada para {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}
