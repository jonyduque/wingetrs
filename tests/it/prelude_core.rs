//! Phase 2c tests (Task 152). Every test runs a real PowerShell on synthetic scripts in a temp root.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use wingetrs::install::prelude::{CONTRACT_VARS, PRELUDE, PRELUDE_FUNCTIONS};

const FUNCTIONS: [&str; 44] = [
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
    "Get-EnvVar",
    "Set-EnvVar",
    "Add-Path",
    "Remove-Path",
    "env",
    "add_first_in_path",
    "remove_from_path",
    "shim",
    "rm_shim",
    "shortcut_folder",
    "startmenu_shortcut",
    "Expand-7zipArchive",
    "Expand-MsiArchive",
    "Expand-InnoArchive",
    "Expand-DarkArchive",
    "Expand-ZipArchive",
    "Invoke-ExternalCommand",
    "persist_data",
    "unlink_persist_data",
    "persist_permission",
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
    let body = "get_config 'USE_LESSMSI' $false\nget_config 'NO_JUNCTIONS' $true\nget_config 'NO_JUNCTION' $true\nget_config 'ZZZ_X' 'pad'";
    let o = run(&work, &root, body);
    assert_eq!(
        lines(&o),
        vec!["True", "False", "False", "pad"],
        "{}",
        o.out
    );
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

#[test]
#[ignore = "live: roda pwsh real"]
fn find_bucket_directory_requires_bucketsdir() {
    let (_t, work, root) = dirs();
    let o = run(
        &work,
        &root,
        "$bucketsdir = ''
Find-BucketDirectory main",
    );
    assert_ne!(o.code, 0, "{}", o.out);
    assert!(o.out.contains("bucketsdir not defined"), "{}", o.out);
}

#[test]
fn prelude_text_is_declared_ascii_and_clean() {
    assert!(PRELUDE.is_ascii());
    let got: BTreeSet<&str> = PRELUDE_FUNCTIONS.iter().copied().collect();
    let want: BTreeSet<&str> = FUNCTIONS.iter().copied().collect();
    assert_eq!(
        PRELUDE_FUNCTIONS.len(),
        got.len(),
        "duplicate in PRELUDE_FUNCTIONS"
    );
    assert_eq!(got, want);
    let mut defined = BTreeSet::new();
    // every `function <name>` anywhere in the text (also after `;` or `{`), word-bounded
    for (i, _) in PRELUDE.match_indices("function") {
        let before_ok = PRELUDE[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-'));
        let rest = &PRELUDE[i + "function".len()..];
        if before_ok && rest.starts_with([' ', '\t']) {
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            // internal helpers (`__wr_*`) are not part of the Scoop surface
            if !name.starts_with("__wr_") {
                defined.insert(name);
            }
        }
    }
    let defined: BTreeSet<&str> = defined.iter().map(String::as_str).collect();
    assert_eq!(
        defined, got,
        "functions in prelude.ps1 differ from PRELUDE_FUNCTIONS"
    );
    for bad in [
        "Invoke-Expression",
        "iex ",
        "DownloadString",
        "Invoke-WebRequest",
        "Start-Process",
        "msiexec",
        "scoopdir",
        "HKCU:",
        "SetEnvironmentVariable",
        "[Environment]::",
    ] {
        assert!(!PRELUDE.contains(bad), "forbidden text in prelude: {bad}");
    }
}

#[test]
fn prelude_has_no_global_parameter() {
    let re = fancy_regex::Regex::new(r"(?i)\$global\b").unwrap();
    for (i, line) in PRELUDE.lines().enumerate() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let code = line
            .replace("[Alias('global')]", "")
            .replace("[Alias('Global')]", "");
        assert!(
            !re.is_match(&code).unwrap(),
            "prelude.ps1:{}: {line}",
            i + 1
        );
    }
}
/// Calls the prelude functions the ways manifests pass Scoop's `$global` (positional,
/// `-global $global`, `-Global:$global`) with the runner's constant `$global` (Task 164).
fn constant_global_calls(shell: &str) {
    let (_t, work, root) = dirs();
    let effects = work.join("effects.jsonl");
    let body = format!(
        "Set-Variable -Name global -Value $false -Option Constant,AllScope -Scope Global\n\
        $__wr_effects = '{}'\n\
        appdir foo $global\n\
        appdir foo -global $global\n\
        versiondir foo '1.0' $global\n\
        installed foo $global\n\
        Select-CurrentVersion foo -Global:$global\n\
        Add-Path -Path x -Global:$global\n\
        'done'",
        effects.display()
    );
    let o = run_with(shell, &work, &root, &body);
    assert!(
        std::fs::read_to_string(&effects)
            .unwrap_or_default()
            .contains("add_path")
    );
    assert_eq!(o.code, 0, "{}", o.out);
    let r = root.display().to_string();
    assert_eq!(
        lines(&o),
        vec![
            format!("{r}\\apps\\foo"),
            format!("{r}\\apps\\foo"),
            format!("{r}\\apps\\foo\\1.0"),
            "False".to_string(),
            "done".to_string(),
        ],
        "{}",
        o.out
    );
    for body in ["appdir foo -global $true", "Add-Path -Path x -Global"] {
        let body = format!(
            "Set-Variable -Name global -Value $false -Option Constant,AllScope -Scope Global\n{body}"
        );
        let g = run_with(shell, &work, &root, &body);
        assert_ne!(g.code, 0, "{body}: {}", g.out);
        assert!(
            g.out.contains("global n\u{e3}o suportado"),
            "{body}: {}",
            g.out
        );
    }
}

#[test]
#[ignore = "live: roda pwsh real"]
fn prelude_accepts_constant_global() {
    constant_global_calls(&pwsh());
    let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    constant_global_calls(&format!(
        r"{sysroot}\System32\WindowsPowerShell\v1.0\powershell.exe"
    ));
}
