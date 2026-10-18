//! Phase 2c tests (Task 152). Every test runs a real PowerShell on synthetic scripts in a temp root.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use wingetrs::install::prelude::{CONTRACT_VARS, PRELUDE, PRELUDE_FUNCTIONS};

const FUNCTIONS: [&str; 24] = [
    "info",
    "warn",
    "error",
    "abort",
    "is_admin",
    "ensure",
    "get_config",
    "appsdir",
    "appdir",
    "versiondir",
    "currentdir",
    "persistdir",
    "shimdir",
    "modulesdir",
    "installed",
    "Select-CurrentVersion",
    "Find-BucketDirectory",
    "Get-HelperPath",
    "Get-CommandPath",
    "Get-AbsolutePath",
    "friendly_path",
    "movedir",
    "Split-PathLikeEnvVar",
    "ConvertToPrettyJson",
];

struct Out {
    code: i32,
    out: String,
}

/// Runs `body` after dot-sourcing the prelude; `root` is `$__wr_root`, `work` holds the scripts.
fn run_with(shell: &str, work: &Path, root: &Path, body: &str) -> Out {
    std::fs::write(work.join("prelude.ps1"), PRELUDE).unwrap();
    let mut s = String::from("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n");
    s += &format!("$__wr_root = '{}'\n", root.display());
    s += "$__wr_effects = $null\n$__wr_env_seed = $null\n$__wr_shortcut_root = ''\n";
    s += &format!(
        "$__wr_helpers = @{{ '7zip' = '{}\\7z.exe' }}\n",
        root.display()
    );
    s += &format!("$bucketsdir = '{}\\buckets'\n", root.display());
    for v in CONTRACT_VARS {
        s += &format!("if (-not (Test-Path variable:{v})) {{ throw 'missing {v}' }}\n");
    }
    s += &format!(". '{}'\n{body}\n", work.join("prelude.ps1").display());
    let script = work.join("t.ps1");
    std::fs::write(&script, s).unwrap();
    let o = Command::new(shell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .output()
        .unwrap();
    let mut out = String::from_utf8_lossy(&o.stdout).into_owned();
    out += &String::from_utf8_lossy(&o.stderr);
    Out {
        code: o.status.code().unwrap_or(-1),
        out,
    }
}

fn pwsh() -> String {
    wingetrs::bridge::find_pwsh(std::env::var_os("PATH").as_deref())
}

fn run(work: &Path, root: &Path, body: &str) -> Out {
    run_with(&pwsh(), work, root, body)
}

/// Fresh (work, root) pair under one temp dir.
fn dirs() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let t = tempfile::tempdir().unwrap();
    let work = t.path().join("work");
    let root = t.path().join("root");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&root).unwrap();
    (t, work, root)
}

fn lines(o: &Out) -> Vec<String> {
    o.out.lines().map(|l| l.trim_end().to_string()).collect()
}

fn all_defined(shell: &str) {
    let (_t, work, root) = dirs();
    let names = FUNCTIONS
        .iter()
        .map(|n| format!("'{n}'"))
        .collect::<Vec<_>>()
        .join(",");
    let body = format!(
        "foreach ($n in @({names})) {{ if (-not (Get-Command $n -CommandType Function -ErrorAction SilentlyContinue)) {{ \"MISSING $n\" }} }}\n'done'"
    );
    let o = run_with(shell, &work, &root, &body);
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec!["done"], "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn prelude_functions_all_defined() {
    all_defined(&pwsh());
    let got: BTreeSet<&str> = PRELUDE_FUNCTIONS.iter().copied().collect();
    let want: BTreeSet<&str> = FUNCTIONS.iter().copied().collect();
    assert_eq!(PRELUDE_FUNCTIONS.len(), got.len());
    assert_eq!(got, want);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn prelude_is_ascii_and_loads_without_side_effects() {
    assert!(PRELUDE.is_ascii());
    let (_t, work, root) = dirs();
    let o = run(&work, &root, "");
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(o.out.trim(), "", "load wrote to stdout/stderr");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn messages_match_scoop() {
    let (_t, work, root) = dirs();
    let o = run(&work, &root, "info x\nwarn x\nerror x");
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec!["INFO  x", "WARN  x", "ERROR x"]);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn abort_throws_and_exits_nonzero() {
    let (_t, work, root) = dirs();
    let o = run(&work, &root, "abort 'falhou'; 'depois'");
    assert_ne!(o.code, 0);
    assert!(o.out.contains("abort: falhou"), "{}", o.out);
    assert!(!o.out.contains("depois"), "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn is_admin_false() {
    let (_t, work, root) = dirs();
    let o = run(&work, &root, "is_admin");
    assert_eq!(lines(&o), vec!["False"], "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn ensure_creates_and_returns_path() {
    let (_t, work, root) = dirs();
    let o = run(
        &work,
        &root,
        "ensure \"$__wr_root\\a\\b\"\nensure \"$__wr_root\\a\\b\"",
    );
    assert_eq!(o.code, 0, "{}", o.out);
    let want = root.join("a").join("b");
    assert!(want.is_dir());
    let l = lines(&o);
    assert_eq!(l.len(), 2, "{}", o.out);
    for x in l {
        assert!(x.eq_ignore_ascii_case(&want.display().to_string()), "{x}");
    }
}

#[test]
#[ignore = "live: roda pwsh real"]
fn paths_under_root() {
    let (_t, work, root) = dirs();
    let body = "appdir foo\nversiondir foo '1.0'\ncurrentdir foo\npersistdir foo\nshimdir\nappsdir\nmodulesdir";
    let o = run(&work, &root, body);
    assert_eq!(o.code, 0, "{}", o.out);
    let r = root.display().to_string();
    let want = [
        format!("{r}\\apps\\foo"),
        format!("{r}\\apps\\foo\\1.0"),
        format!("{r}\\apps\\foo\\current"),
        format!("{r}\\persist\\foo"),
        format!("{r}\\shims"),
        format!("{r}\\apps"),
        format!("{r}\\modules"),
    ];
    assert_eq!(lines(&o), want);
    let g = run(&work, &root, "appdir foo $true");
    assert_ne!(g.code, 0);
    assert!(g.out.contains("global n\u{e3}o suportado"), "{}", g.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn modulesdir_variable_not_defined() {
    let (_t, work, root) = dirs();
    let o = run(&work, &root, "Test-Path variable:modulesdir");
    assert_eq!(lines(&o), vec!["False"], "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn get_config_table_and_default() {
    let (_t, work, root) = dirs();
    let body = "get_config 'USE_LESSMSI' $false\nget_config 'NO_JUNCTIONS' $true\nget_config 'ZZZ_X' 'pad'";
    let o = run(&work, &root, body);
    assert_eq!(lines(&o), vec!["True", "False", "pad"], "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn installed_and_select_current_version() {
    let (_t, work, root) = dirs();
    let app = root.join("apps").join("foo");
    std::fs::create_dir_all(app.join("1.2")).unwrap();
    let body = "New-Item -ItemType Junction -Path \"$__wr_root\\apps\\foo\\current\" -Target \"$__wr_root\\apps\\foo\\1.2\" | Out-Null\n\
        installed foo\nSelect-CurrentVersion foo\ninstalled bar\n'[' + (Select-CurrentVersion bar) + ']'";
    let o = run(&work, &root, body);
    assert_eq!(o.code, 0, "{}", o.out);
    assert_eq!(lines(&o), vec!["True", "1.2", "False", "[]"], "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn bucket_helper_command_paths() {
    let (_t, work, root) = dirs();
    std::fs::create_dir_all(root.join("buckets").join("main").join("bucket")).unwrap();
    std::fs::create_dir_all(root.join("shims")).unwrap();
    let target = root.join("tool.exe");
    std::fs::write(&target, b"x").unwrap();
    std::fs::write(
        root.join("shims").join("tool.cmd"),
        format!("@rem {}\r\n@echo off\r\n", target.display()),
    )
    .unwrap();
    let body = "Find-BucketDirectory main\nFind-BucketDirectory main -Root\nFind-BucketDirectory\n\
        Get-HelperPath 7zip\n'[' + (Get-HelperPath Git) + ']'\nGet-CommandPath \"$__wr_root\\shims\\tool.cmd\"";
    let o = run(&work, &root, body);
    assert_eq!(o.code, 0, "{}", o.out);
    let r = root.display().to_string();
    let want = [
        format!("{r}\\buckets\\main\\bucket"),
        format!("{r}\\buckets\\main"),
        format!("{r}\\buckets\\main\\bucket"),
        format!("{r}\\7z.exe"),
        "[]".to_string(),
        target.display().to_string(),
    ];
    assert_eq!(lines(&o), want, "{}", o.out);
}

#[test]
#[ignore = "live: roda pwsh real"]
fn misc_utils() {
    let (_t, work, root) = dirs();
    std::fs::create_dir_all(root.join("from").join("sub")).unwrap();
    std::fs::write(root.join("from").join("sub").join("a.txt"), b"a").unwrap();
    let body = "movedir \"$__wr_root\\from\" \"$__wr_root\\to\"\n\
        Test-Path \"$__wr_root\\to\\sub\\a.txt\"\nTest-Path \"$__wr_root\\from\"\n\
        (ConvertToPrettyJson ([pscustomobject]@{ a = 1; b = @('x') })) -replace \"`r`n\", '|'\n\
        friendly_path ((Get-PSProvider 'FileSystem').Home + '\\x')\n\
        friendly_path 'D:\\other'\n\
        Get-AbsolutePath 'C:\\a\\b\\'\nGet-AbsolutePath 'C:\\'\n\
        $in, $rest = Split-PathLikeEnvVar 'C:\\a*;C:\\z' 'C:\\abc;C:\\b;C:\\z'\n\"$in#$rest\"";
    let o = run(&work, &root, body);
    assert_eq!(o.code, 0, "{}", o.out);
    let want = [
        "True",
        "False",
        "{|    \"a\":  1,|    \"b\":  \"x\"|}",
        "~\\x",
        "D:\\other",
        "C:\\a\\b",
        "C:\\",
        "C:\\abc;C:\\z#C:\\b",
    ];
    let got = lines(&o);
    // ConvertTo-Json spacing differs between PowerShell editions: compare ignoring spaces.
    let squash = |s: &str| s.replace(' ', "");
    assert_eq!(got.len(), want.len(), "{}", o.out);
    for (g, w) in got.iter().zip(want) {
        assert_eq!(squash(g), squash(w), "{}", o.out);
    }
}

#[test]
#[ignore = "live: roda pwsh real"]
fn loads_on_windows_powershell_5_1() {
    let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let ps = format!(r"{sysroot}\System32\WindowsPowerShell\v1.0\powershell.exe");
    assert!(Path::new(&ps).is_file(), "{ps}");
    all_defined(&ps);
}
