//! Phase 2c tests (Task 155).

use std::path::{Path, PathBuf};
use std::process::Command;

use wingetrs::install::manifest::{Arch, HookKind};
use wingetrs::install::runner::{
    RunnerError, RunnerInput, ScriptBody, ScriptContext, ScriptOp, VarValue, ps_single_quoted,
    runner_ps1, script_vars,
};

fn cx(op: ScriptOp, root: &Path) -> ScriptContext {
    let app_dir = root.join("apps").join("demo");
    ScriptContext {
        op,
        app: "demo".to_string(),
        version: "1.0".to_string(),
        old_version: None,
        bucket: Some("main".to_string()),
        arch: Some(Arch::X64),
        root: root.to_path_buf(),
        version_dir: app_dir.join("1.0"),
        current: app_dir.join("current"),
        persist_dir: root.join("persist").join("demo"),
        fnames: vec!["demo.zip".to_string()],
        purge: false,
        manifest_path: app_dir.join("1.0").join("manifest.json"),
        bucketsdir: root.join("buckets"),
    }
}

fn get<'a>(vars: &'a [(String, VarValue)], name: &str) -> Option<&'a VarValue> {
    vars.iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

fn str_of(p: &Path) -> VarValue {
    VarValue::Str(p.to_string_lossy().into_owned())
}

#[test]
fn quotes_ascii_and_typographic_escaped() {
    assert_eq!(
        ps_single_quoted("a'b\u{2018}c\u{2019}d\u{201A}e\u{201B}f"),
        "'a''b\u{2018}\u{2018}c\u{2019}\u{2019}d\u{201A}\u{201A}e\u{201B}\u{201B}f'"
    );
}

fn hook_runner(
    vars: &[(String, VarValue)],
    prelude: &Path,
    script: &Path,
) -> Result<String, RunnerError> {
    runner_ps1(&RunnerInput {
        contract: &[],
        vars,
        prelude_path: prelude,
        body: ScriptBody::Hook {
            script_path: script,
        },
    })
}

#[test]
fn hook_never_interpolated() {
    let tmp = tempfile::tempdir().unwrap();
    let hook = tmp.path().join("hook.ps1");
    std::fs::write(&hook, "Write-Output 'MARKER_ZZZ_9'").unwrap();
    let prelude = tmp.path().join("prelude.ps1");
    let vars = script_vars(HookKind::PreInstall, &cx(ScriptOp::Install, tmp.path()));
    let text = hook_runner(&vars, &prelude, &hook).unwrap();
    assert!(!text.contains("MARKER_ZZZ_9"));
    assert!(text.contains(&*hook.to_string_lossy()));
}

#[test]
fn order_vars_prelude_hook() {
    let tmp = tempfile::tempdir().unwrap();
    let hook = tmp.path().join("hook.ps1");
    let prelude = tmp.path().join("prelude.ps1");
    let contract = vec![("contract_x".to_string(), VarValue::Bool(true))];
    let vars = script_vars(HookKind::PreInstall, &cx(ScriptOp::Install, tmp.path()));
    let text = runner_ps1(&RunnerInput {
        contract: &contract,
        vars: &vars,
        prelude_path: &prelude,
        body: ScriptBody::Hook { script_path: &hook },
    })
    .unwrap();
    let last_var = &vars.last().unwrap().0;
    let last_assign = text.rfind(&format!("${last_var} = ")).unwrap();
    let contract_assign = text.find("$contract_x = $true").unwrap();
    let prelude_idx = text
        .find(&format!(
            ". {}",
            ps_single_quoted(&prelude.to_string_lossy())
        ))
        .unwrap();
    let hook_idx = text.find("ReadAllText").unwrap();
    assert!(contract_assign < last_assign);
    assert!(last_assign < prelude_idx);
    assert!(prelude_idx < hook_idx);
    assert!(!text.contains("ErrorActionPreference"));
    assert!(text.trim_end().ends_with("exit 0"));
}

#[test]
fn vars_pre_install() {
    let root = PathBuf::from(r"C:\r");
    let c = cx(ScriptOp::Install, &root);
    let v = script_vars(HookKind::PreInstall, &c);
    assert_eq!(get(&v, "dir"), Some(&str_of(&c.version_dir)));
    assert_eq!(get(&v, "original_dir"), Some(&str_of(&c.version_dir)));
    assert_eq!(get(&v, "cmd"), Some(&VarValue::Str("install".into())));
    assert_eq!(
        get(&v, "fname"),
        Some(&VarValue::StrArray(vec!["demo.zip".into()]))
    );
    assert_eq!(get(&v, "global"), Some(&VarValue::Bool(false)));
    assert_eq!(get(&v, "bucketsdir"), Some(&str_of(&c.bucketsdir)));
    assert_eq!(
        get(&v, "architecture"),
        Some(&VarValue::Str("64bit".into()))
    );
    assert_eq!(get(&v, "purge"), None);
    assert_eq!(get(&v, "old_version"), None);
    assert_eq!(get(&v, "Path"), None);
}

#[test]
fn vars_post_install_dir_is_current() {
    let root = PathBuf::from(r"C:\r");
    let c = cx(ScriptOp::Install, &root);
    let v = script_vars(HookKind::PostInstall, &c);
    assert_eq!(get(&v, "dir"), Some(&str_of(&c.current)));
    assert_eq!(get(&v, "original_dir"), Some(&str_of(&c.version_dir)));
}

#[test]
fn vars_update_install_hooks_cmd_update() {
    let root = PathBuf::from(r"C:\r");
    let mut c = cx(ScriptOp::Update, &root);
    c.old_version = Some("0.9".into());
    let v = script_vars(HookKind::PreInstall, &c);
    assert_eq!(get(&v, "cmd"), Some(&VarValue::Str("update".into())));
    assert_eq!(get(&v, "old_version"), None);
}

#[test]
fn vars_update_uninstall_hooks() {
    let root = PathBuf::from(r"C:\r");
    let mut c = cx(ScriptOp::Update, &root);
    let old_dir = root.join("apps").join("demo").join("1.0");
    let old_manifest = old_dir.join("manifest.json");
    c.version = "2.0".into();
    c.old_version = Some("1.0".into());
    c.version_dir = old_dir.clone();
    c.manifest_path = old_manifest.clone();
    let v = script_vars(HookKind::PreUninstall, &c);
    assert_eq!(get(&v, "version"), Some(&VarValue::Str("2.0".into())));
    assert_eq!(get(&v, "old_version"), Some(&VarValue::Str("1.0".into())));
    assert_eq!(get(&v, "dir"), Some(&str_of(&old_dir)));
    assert_eq!(get(&v, "manifest"), Some(&VarValue::JsonFile(old_manifest)));
    assert_eq!(get(&v, "cmd"), Some(&VarValue::Str("update".into())));
    assert_eq!(get(&v, "original_dir"), None);
    assert_eq!(get(&v, "fname"), None);
    assert_eq!(get(&v, "purge"), None);
}

#[test]
fn vars_uninstall() {
    let root = PathBuf::from(r"C:\r");
    let mut c = cx(ScriptOp::Uninstall, &root);
    c.purge = true;
    for kind in [
        HookKind::PreUninstall,
        HookKind::UninstallerScript,
        HookKind::PostUninstall,
    ] {
        let v = script_vars(kind, &c);
        assert_eq!(get(&v, "cmd"), Some(&VarValue::Str("uninstall".into())));
        assert_eq!(get(&v, "purge"), Some(&VarValue::Bool(true)));
        assert_eq!(get(&v, "dir"), Some(&str_of(&c.version_dir)));
        assert_eq!(get(&v, "original_dir"), None);
        assert_eq!(get(&v, "fname"), None);
        assert_eq!(get(&v, "old_version"), None);
    }
}

#[test]
fn installer_and_uninstaller_script_extras() {
    let root = PathBuf::from(r"C:\r");
    let c = cx(ScriptOp::Install, &root);
    let v = script_vars(HookKind::InstallerScript, &c);
    assert_eq!(get(&v, "Path"), Some(&str_of(&c.version_dir)));
    assert_eq!(
        get(&v, "Name"),
        Some(&VarValue::StrArray(vec!["demo.zip".into()]))
    );
    assert_eq!(get(&v, "AppName"), Some(&VarValue::Str("demo".into())));
    assert_eq!(get(&v, "Uninstall"), Some(&VarValue::Bool(false)));

    let u = cx(ScriptOp::Uninstall, &root);
    let v = script_vars(HookKind::UninstallerScript, &u);
    assert_eq!(get(&v, "Uninstall"), Some(&VarValue::Bool(true)));
    assert_eq!(get(&v, "Path"), Some(&str_of(&u.version_dir)));
    assert_eq!(get(&v, "AppName"), Some(&VarValue::Str("demo".into())));
    assert_eq!(get(&v, "Name"), None);

    let v = script_vars(HookKind::PostInstall, &c);
    assert_eq!(get(&v, "Path"), None);
    assert_eq!(get(&v, "Uninstall"), None);
}

#[test]
fn bad_var_name_err() {
    let vars = vec![("a b".to_string(), VarValue::Null)];
    let r = hook_runner(&vars, Path::new(r"C:\p.ps1"), Path::new(r"C:\h.ps1"));
    assert!(matches!(r, Err(RunnerError::BadName(_))));
}

fn run_pwsh(dir: &Path, runner: &str) -> std::io::Result<(i32, String)> {
    let file = dir.join("runner.ps1");
    std::fs::write(&file, runner)?;
    let out = Command::new("pwsh")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&file)
        .output()?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    ))
}

#[test]
fn end_to_end_pwsh() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mut c = cx(ScriptOp::Install, root);
    std::fs::create_dir_all(&c.version_dir).unwrap();
    std::fs::write(&c.manifest_path, r#"{"version":"1.0"}"#).unwrap();
    c.app = "o\u{2019}brien".into();
    let prelude = root.join("prelude.ps1");
    std::fs::write(&prelude, "").unwrap();
    let hook = root.join("hook.ps1");
    let vars = script_vars(HookKind::PreInstall, &c);

    std::fs::write(&hook, "\"$cmd|$dir|$version|$($manifest.version)\"").unwrap();
    let (code, out) = run_pwsh(root, &hook_runner(&vars, &prelude, &hook).unwrap()).unwrap();
    assert_eq!(code, 0);
    assert_eq!(
        out.trim_end(),
        format!("install|{}|1.0|1.0", c.version_dir.to_string_lossy())
    );

    std::fs::write(&hook, "throw 'x'").unwrap();
    let (code, _) = run_pwsh(root, &hook_runner(&vars, &prelude, &hook).unwrap()).unwrap();
    assert_eq!(code, 1);

    let missing = root.join("nao-existe");
    std::fs::write(
        &hook,
        format!(
            "Remove-Item {}",
            ps_single_quoted(&missing.to_string_lossy())
        ),
    )
    .unwrap();
    let (code, _) = run_pwsh(root, &hook_runner(&vars, &prelude, &hook).unwrap()).unwrap();
    assert_eq!(code, 0);

    std::fs::write(&hook, "$app").unwrap();
    let (code, out) = run_pwsh(root, &hook_runner(&vars, &prelude, &hook).unwrap()).unwrap();
    assert_eq!(code, 0);
    assert_eq!(out.trim_end(), "o\u{2019}brien");
}
