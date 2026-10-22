//! Production wiring of `OpsCtx`: real HKCU, Start Menu, Recycle Bin and registry scan.
//! With `WINGETRS_TEST_ENV_KEY` / `WINGETRS_TEST_SHORTCUT_DIR` set, nothing real is touched
//! (and the residue scan reads neither Desktop, Start Menu nor AppData).
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::cache::DataDir;
use crate::catalog::Catalog;
use crate::http::Http;
use crate::install::download::Progress;
use crate::install::env_win::RegEnvStore;
use crate::install::executor::{Msiexec, Runners};
use crate::install::fs_win;
use crate::install::msiexec_win;
use crate::install::ops::{OpsCtx, ResidueScan, installed_by_scoop};
use crate::install::registry_win::RegScanner;
use crate::install::script_ast::{self, AstGate};
use crate::install::script_ast_win::PwshAst;
use crate::install::script_host_win::PwshHost;
use crate::install::shortcut_win::{self, ComShortcuts};
use crate::install::traits::{EffectError, Effects};
use crate::install::trash_win::ShellTrash;
use crate::install::{inno_win, installer_win};
use crate::update::LockWait;

/// Owns the real effect implementations; [`ProdEffects::effects`] borrows them.
pub struct ProdEffects {
    env: RegEnvStore,
    shortcuts: ComShortcuts,
    trash: ShellTrash,
    registry: RegScanner,
}

impl ProdEffects {
    pub fn new() -> Result<ProdEffects, EffectError> {
        Ok(ProdEffects {
            env: RegEnvStore::new(),
            shortcuts: ComShortcuts::new()?,
            trash: ShellTrash,
            registry: RegScanner::from_env(),
        })
    }

    /// `build_effects`: the borrowed bundle `OpsCtx::effects` wants.
    pub fn effects(&self) -> Effects<'_> {
        Effects {
            env: &self.env,
            shortcuts: &self.shortcuts,
            trash: &self.trash,
            registry: &self.registry,
        }
    }
}

fn link_target(p: &Path) -> Option<PathBuf> {
    shortcut_win::link_target(p).ok()
}

/// The script gate's parser for `program`, one per wingetrs run (Task 227): every `OpsCtx` and
/// the TUI's confirmation share its process and its cache, so the confirmation shows what the
/// plan already analysed. Its process dies with this one (the job closes with it).
fn shared_ast(program: &Path) -> Arc<dyn AstGate> {
    static AST: OnceLock<Arc<PwshAst>> = OnceLock::new();
    let shared = AST.get_or_init(|| Arc::new(PwshAst::for_program(program.to_path_buf())));
    if shared.program() == program {
        Arc::clone(shared) as Arc<dyn AstGate>
    } else {
        // Another PowerShell than the first one located (pwsh 7 installed meanwhile).
        Arc::new(PwshAst::for_program(program.to_path_buf()))
    }
}

/// The parser `build_ctx` wires, for a front end that shows script reports outside an
/// `OpsCtx` (the TUI's confirmation); the same choice under `WINGETRS_TEST_ENV_KEY`.
pub fn production_ast() -> Arc<dyn AstGate> {
    if std::env::var_os("WINGETRS_TEST_ENV_KEY").is_some() {
        script_ast::under_test_key()
    } else {
        shared_ast(PwshHost::locate().program())
    }
}

/// `build_ctx`: the production `OpsCtx`. Callers supply what differs between front ends:
/// `wait` (CLI: `LockWait::from_env()`, TUI: `NoWait`) and `progress`.
pub fn build_ctx<'a>(
    prod: &'a ProdEffects,
    data: &'a DataDir,
    http: &'a Http,
    catalog: &'a Catalog,
    wait: LockWait,
    progress: Progress<'a>,
) -> OpsCtx<'a> {
    // Under test nothing real is scanned, and `msiexec` never runs.
    let (residue, msiexec, runners) = if std::env::var_os("WINGETRS_TEST_ENV_KEY").is_some() {
        // No script runs (`scripts: None`); the plan's gate is the lexer's alone.
        let runners = Runners {
            ast: script_ast::under_test_key(),
            ..Runners::disabled()
        };
        (ResidueScan::none(), Msiexec::DISABLED, runners)
    } else {
        (
            ResidueScan::user_profile(&link_target, &installed_by_scoop),
            Msiexec {
                busy: msiexec_win::installer_busy,
                launch: msiexec_win::msiexec_admin,
            },
            {
                let host = PwshHost::locate();
                // The parser is the same PowerShell that would run the script.
                let ast = shared_ast(host.program());
                Runners {
                    innounp: inno_win::run,
                    installer: Some(installer_win::run),
                    // Masked by `ops::exec_ctx` unless the user consented (`--allow-scripts`).
                    scripts: Some(Arc::new(host)),
                    ast,
                }
            },
        )
    };
    OpsCtx {
        data,
        http,
        catalog,
        effects: prod.effects(),
        wait,
        progress,
        in_use: &fs_win::files_in_use,
        residue,
        msiexec,
        launch: Default::default(),
        runners,
    }
}
