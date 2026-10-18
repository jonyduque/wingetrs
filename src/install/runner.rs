//! runner.ps1 generator (Task 155).
//!
//! Pure: builds the text of the PowerShell runner that defines the Scoop
//! variables a hook expects (probe D3), dot-sources the prelude and then runs
//! the hook from its own file. The hook text never enters the runner.

use std::path::{Path, PathBuf};

use crate::install::manifest::{Arch, HookKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptOp {
    Install,
    Update,
    Uninstall,
}

#[derive(Debug, Clone)]
pub struct ScriptContext {
    pub op: ScriptOp,
    pub app: String,
    /// Always the version being installed (the NEW one during an update).
    pub version: String,
    pub old_version: Option<String>,
    pub bucket: Option<String>,
    pub arch: Option<Arch>,
    pub root: PathBuf,
    /// Real version folder (the OLD one in uninstall hooks during an update).
    pub version_dir: PathBuf,
    pub current: PathBuf,
    pub persist_dir: PathBuf,
    pub fnames: Vec<String>,
    pub purge: bool,
    /// The OLD manifest in uninstall hooks during an update.
    pub manifest_path: PathBuf,
    pub bucketsdir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarValue {
    Str(String),
    Bool(bool),
    StrArray(Vec<String>),
    JsonFile(PathBuf),
    /// Hashtable of strings (`@{'k'='v'}`), e.g. `$__wr_helpers` (Task 156).
    Table(Vec<(String, String)>),
    Null,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RunnerError {
    #[error("nome de variável inválido para o PowerShell: {0:?}")]
    BadName(String),
}

pub enum ScriptBody<'a> {
    Hook { script_path: &'a Path },
    Ps1File { path: &'a Path, args: &'a [String] },
}

pub struct RunnerInput<'a> {
    pub contract: &'a [(String, VarValue)],
    pub vars: &'a [(String, VarValue)],
    pub prelude_path: &'a Path,
    pub body: ScriptBody<'a>,
}

fn path_str(p: &Path) -> VarValue {
    VarValue::Str(p.to_string_lossy().into_owned())
}

fn is_install_hook(kind: HookKind) -> bool {
    matches!(
        kind,
        HookKind::PreInstall | HookKind::InstallerScript | HookKind::PostInstall
    )
}

/// Variables visible to a hook, following the D3 table (Scoop e6aa3b3).
/// Variables marked "—" in the table are omitted.
pub fn script_vars(kind: HookKind, cx: &ScriptContext) -> Vec<(String, VarValue)> {
    let install_hook = is_install_hook(kind);
    let cmd = match cx.op {
        ScriptOp::Install => "install",
        ScriptOp::Update => "update",
        ScriptOp::Uninstall => "uninstall",
    };
    let dir = match kind {
        HookKind::PostInstall => cx.current.clone(),
        _ => cx.version_dir.clone(),
    };
    let mut v: Vec<(String, VarValue)> = Vec::new();
    let mut put = |name: &str, value: VarValue| v.push((name.to_string(), value));

    put("cmd", VarValue::Str(cmd.to_string()));
    put("app", VarValue::Str(cx.app.clone()));
    put("version", VarValue::Str(cx.version.clone()));
    put("dir", path_str(&dir));
    if install_hook {
        put("original_dir", path_str(&cx.version_dir));
    }
    put("persist_dir", path_str(&cx.persist_dir));
    // `runner_ps1` writes it as a constant (`GLOBAL_CONSTANT`), not as an assignment.
    put("global", VarValue::Bool(false));
    put(
        "bucket",
        cx.bucket
            .clone()
            .map(VarValue::Str)
            .unwrap_or(VarValue::Null),
    );
    put(
        "architecture",
        cx.arch
            .map(|a| VarValue::Str(a.key().to_string()))
            .unwrap_or(VarValue::Null),
    );
    put("manifest", VarValue::JsonFile(cx.manifest_path.clone()));
    if install_hook {
        put("fname", VarValue::StrArray(cx.fnames.clone()));
    }
    if cx.op == ScriptOp::Uninstall && !install_hook {
        put("purge", VarValue::Bool(cx.purge));
    }
    put("bucketsdir", path_str(&cx.bucketsdir));
    put("scoopdir", path_str(&cx.root));
    // $global is always false, so $globaldir points at a folder inside our root
    // instead of %ProgramData%\scoop (never an empty path that resolves to "\").
    put("globaldir", path_str(&cx.root.join("global")));
    put("cachedir", path_str(&cx.root.join("cache")));
    // scoop-update.ps1:262 defines it inside update(), which calls install_app
    // (:389), so dynamic scope exposes it to the new version's install hooks too.
    if let (ScriptOp::Update, Some(old)) = (cx.op, &cx.old_version) {
        put("old_version", VarValue::Str(old.clone()));
    }
    match kind {
        HookKind::InstallerScript => {
            put("Path", path_str(&dir));
            put("Name", VarValue::StrArray(cx.fnames.clone()));
            put("AppName", VarValue::Str(cx.app.clone()));
            put("Uninstall", VarValue::Bool(false));
        }
        HookKind::UninstallerScript => {
            put("Path", path_str(&dir));
            put("AppName", VarValue::Str(cx.app.clone()));
            put("Uninstall", VarValue::Bool(true));
        }
        _ => {}
    }
    v
}

/// PowerShell single-quoted literal. PowerShell treats U+2018..U+201B as single
/// quotes too (probe D7), so each of them is doubled like `'`.
pub fn ps_single_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if let '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' = ch {
            out.push(ch);
        }
        out.push(ch);
    }
    out.push('\'');
    out
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn literal(value: &VarValue) -> String {
    match value {
        VarValue::Str(s) => ps_single_quoted(s),
        VarValue::Bool(true) => "$true".to_string(),
        VarValue::Bool(false) => "$false".to_string(),
        VarValue::Null => "$null".to_string(),
        VarValue::StrArray(items) => {
            let parts: Vec<String> = items.iter().map(|s| ps_single_quoted(s)).collect();
            format!("@({})", parts.join(","))
        }
        VarValue::Table(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|(k, v)| format!("{}={}", ps_single_quoted(k), ps_single_quoted(v)))
                .collect();
            format!("@{{{}}}", parts.join(";"))
        }
        VarValue::JsonFile(p) => format!(
            "(Get-Content -LiteralPath {} -Raw -Encoding UTF8 | ConvertFrom-Json)",
            ps_single_quoted(&p.to_string_lossy())
        ),
    }
}

const GLOBAL_CONSTANT: &str =
    "Set-Variable -Name global -Value $false -Option Constant,AllScope -Scope Global\n";

const CATCH: &str = "catch { [Console]::Error.WriteLine($_.ToString()); exit 1 }";

/// Text of runner.ps1. No `$ErrorActionPreference` (decision R1: Scoop does not
/// set `Stop`, so non-terminating errors do not fail the hook).
/// Whoever writes the file must use UTF-8 with BOM (powershell 5.1 reads BOM-less files as ANSI).
pub fn runner_ps1(input: &RunnerInput<'_>) -> Result<String, RunnerError> {
    let mut s = String::new();
    s.push_str("[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)\n");
    s.push_str("$OutputEncoding = [Console]::OutputEncoding\n");
    // wingetrs never installs globally: `$global` is a constant `$false` in every scope
    // (`AllScope`, so `$global = $true` in the hook's child scope fails instead of
    // creating a local copy), and `if ($global) { ... }` blocks can never run.
    s.push_str(GLOBAL_CONSTANT);
    for (name, value) in input.contract.iter().chain(input.vars.iter()) {
        if !valid_name(name) {
            return Err(RunnerError::BadName(name.clone()));
        }
        if name.eq_ignore_ascii_case("global") {
            continue;
        }
        s.push_str(&format!("${name} = {}\n", literal(value)));
    }
    s.push_str(&format!(
        ". {}\n",
        ps_single_quoted(&input.prelude_path.to_string_lossy())
    ));
    match &input.body {
        // install.ps1:165 (Invoke-HookScript): child scope via Invoke-Command.
        ScriptBody::Hook { script_path } => {
            s.push_str(&format!(
                "$__wr_hook = [System.IO.File]::ReadAllText({})\n",
                ps_single_quoted(&script_path.to_string_lossy())
            ));
            s.push_str(&format!(
                "try {{ Invoke-Command ([scriptblock]::Create($__wr_hook)) }} {CATCH}\n"
            ));
        }
        ScriptBody::Ps1File { path, args } => {
            let parts: Vec<String> = args.iter().map(|a| ps_single_quoted(a)).collect();
            s.push_str(&format!(
                "try {{ & {} @({}) }} {CATCH}\n",
                ps_single_quoted(&path.to_string_lossy()),
                parts.join(",")
            ));
        }
    }
    s.push_str("exit 0\n");
    Ok(s)
}
