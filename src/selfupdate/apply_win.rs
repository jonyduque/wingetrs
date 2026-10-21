//! Windows side of `self update` (Task 213): which scope the executable's folder is in, the
//! real UAC launch of the elevated copy, and that copy's entry point.
use super::{Outcome, SelfUpdateError};
use std::path::Path;

/// Whether `exe` and every folder above it are administrator-only by owner and DACL (and none
/// a reparse point): then only an elevated copy may replace it.
pub fn admin_scope(exe: &Path) -> bool {
    crate::bridge::arp_ops_win::hold_admin_chain(exe).is_ok()
}

/// [`super::elevated_apply_started`] through the shell's `runas` (one UAC prompt).
pub fn elevated_apply(
    exe: &Path,
    notice: &mut dyn FnMut(),
    started: &mut dyn FnMut(),
) -> Result<Outcome, SelfUpdateError> {
    let launcher = crate::winget_install::launch_win::ShellLauncher;
    super::elevated_apply_started(&launcher, exe, notice, started)
}

/// `self apply-elevated`: runs elevated, from its own executable (never a path received), only
/// when that executable's chain is administrator-only and it is not a development build. It
/// writes nothing outside the executable's folder (no data dir, no log, no output) and answers
/// by its exit code (`super::exit_code`).
pub fn run_elevated() -> i32 {
    let r = elevated_main();
    super::exit_code(&r)
}

fn elevated_main() -> Result<Outcome, SelfUpdateError> {
    let exe = std::env::current_exe()?;
    super::refuse_dev_build(&exe)?;
    if !admin_scope(&exe) {
        return Err(SelfUpdateError::Other(
            "pasta do executável não é só de administrador".into(),
        ));
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let http = crate::http::Http::new().map_err(|e| SelfUpdateError::Other(e.to_string()))?;
        super::elevated_run(
            &http,
            super::WEB_BASE,
            super::DL_BASE,
            &exe,
            env!("CARGO_PKG_VERSION"),
        )
        .await
    })
}
