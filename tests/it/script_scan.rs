//! Phase 2c tests (Task 150).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::BTreeSet;

use wingetrs::install::manifest::{Arch, HookKind, resolve};
use wingetrs::install::script_scan::{
    FlagKind, SCOOP_FUNCTIONS, ScriptReport, Verdict, env_var_names, scan, scan_manifest,
};

const SUP: &[&str] = &["ensure"];

fn kinds(r: &ScriptReport) -> Vec<FlagKind> {
    r.flags.iter().map(|f| f.kind).collect()
}

fn has(r: &ScriptReport, k: FlagKind) -> bool {
    r.flags.iter().any(|f| f.kind == k)
}

#[test]
fn plain_script_is_confirm() {
    let r = scan(r#"Remove-Item "$dir\x" -Force"#, SUP);
    assert_eq!(r.verdict, Verdict::Confirm);
    assert!(r.flags.is_empty(), "{:?}", kinds(&r));
}

#[test]
fn runas_refused() {
    let r = scan(r#"Start-Process "$dir\x.exe" -Verb RunAs"#, SUP);
    assert_eq!(r.verdict, Verdict::Refuse);
    assert!(has(&r, FlagKind::Elevation), "{:?}", kinds(&r));
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::Elevation)
        .unwrap();
    assert_eq!(f.detail, "pede elevação (RunAs/sudo)");
    assert_eq!(f.verdict, Verdict::Refuse);
}

#[test]
fn sudo_refused() {
    let r = scan("sudo Remove-Item x", SUP);
    assert_eq!(r.verdict, Verdict::Refuse);
    assert!(has(&r, FlagKind::Elevation), "{:?}", kinds(&r));
}

#[test]
fn hklm_refused() {
    let r = scan(r"Set-ItemProperty 'HKLM:\SOFTWARE\X' -Name a -Value 1", SUP);
    assert_eq!(r.verdict, Verdict::Refuse);
    let f = r.flags.iter().find(|f| f.kind == FlagKind::Hklm).unwrap();
    assert_eq!(f.detail, "mexe no registro da máquina (HKLM)");
}

#[test]
fn hklm_in_comment_ignored() {
    let r = scan("# HKLM:\\x\nRemove-Item x", SUP);
    assert_eq!(r.verdict, Verdict::Confirm, "{:?}", kinds(&r));
    assert!(r.flags.is_empty());
}

#[test]
fn hklm_in_here_string_ignored() {
    let r = scan("$t = @'\nHKLM:\\x\n'@\nSet-Content \"$dir\\a.reg\" $t", SUP);
    assert!(!has(&r, FlagKind::Hklm), "{:?}", kinds(&r));
}

#[test]
fn global_switch_refused() {
    let r = scan(r#"Add-Path -Path "$dir\bin" -Global"#, SUP);
    assert_eq!(r.verdict, Verdict::Refuse);
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::GlobalSwitch)
        .unwrap();
    assert_eq!(f.detail, "usa -Global (instalação para todos os usuários)");
}

#[test]
fn program_files_write_refused() {
    let r = scan(r#"Copy-Item "$dir\x" "$env:ProgramFiles\X""#, SUP);
    assert_eq!(r.verdict, Verdict::Refuse);
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::SystemWrite)
        .unwrap();
    assert_eq!(f.detail, "grava em Program Files/Windows");
}

#[test]
fn unknown_scoop_function_refused_with_name() {
    let r = scan(r#"Expand-DarkArchive "$dir\a.exe" "$dir\x""#, SUP);
    assert_eq!(r.verdict, Verdict::Refuse);
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::UnknownScoopFunction)
        .unwrap();
    assert_eq!(
        f.detail,
        "função do Scoop ainda não suportada: Expand-DarkArchive"
    );

    let r = scan("APPDIR foo", SUP);
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::UnknownScoopFunction)
        .unwrap();
    assert_eq!(f.detail, "função do Scoop ainda não suportada: appdir");
}

#[test]
fn supported_scoop_function_ok() {
    let r = scan(
        r#"Expand-DarkArchive "$dir\a.exe" "$dir\x""#,
        &["ensure", "Expand-DarkArchive"],
    );
    assert!(!has(&r, FlagKind::UnknownScoopFunction), "{:?}", kinds(&r));
    assert_eq!(r.verdict, Verdict::Confirm);
}

#[test]
fn scoop_command_refused() {
    let r = scan("scoop install sudo", SUP);
    assert_eq!(r.verdict, Verdict::Refuse);
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::ScoopCommand)
        .unwrap();
    assert_eq!(
        f.detail,
        "chama o comando scoop (instalaria no ~/scoop); não suportado"
    );
}

#[test]
fn msiexec_refused() {
    for s in [
        "Start-Process msiexec -ArgumentList '/i x.msi'",
        "msiexec /i x",
    ] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}");
        let f = r
            .flags
            .iter()
            .find(|f| f.kind == FlagKind::Msiexec)
            .unwrap_or_else(|| panic!("{s}: {:?}", kinds(&r)));
        assert_eq!(
            f.detail,
            "chama o msiexec; o wingetrs só usa msiexec /a pela fila própria"
        );
    }
}

#[test]
fn iex_strong() {
    let r = scan("iex (Get-Content x -Raw)", SUP);
    assert_eq!(r.verdict, Verdict::ConfirmStrong);
    assert!(has(&r, FlagKind::DynamicCode), "{:?}", kinds(&r));
}

#[test]
fn call_operator_with_variable_strong() {
    let r = scan("& $exe /S", SUP);
    assert_eq!(r.verdict, Verdict::ConfirmStrong);
    assert!(has(&r, FlagKind::DynamicCode), "{:?}", kinds(&r));
}

#[test]
fn download_strong() {
    let r = scan(r#"Invoke-WebRequest https://x -OutFile "$dir\a""#, SUP);
    assert_eq!(r.verdict, Verdict::ConfirmStrong);
    assert_eq!(kinds(&r), vec![FlagKind::Download]);
}

#[test]
fn reg_import_strong() {
    let r = scan(
        r#"reg import "$bucketsdir\extras\scripts\vscode\install-context.reg""#,
        SUP,
    );
    assert_eq!(r.verdict, Verdict::ConfirmStrong);
    assert_eq!(kinds(&r), vec![FlagKind::RegistryUser]);
}

#[test]
fn hkcu_strong() {
    let r = scan(
        r"New-ItemProperty -Path 'HKCU:\Software\X' -Name a -Value 1",
        SUP,
    );
    assert_eq!(r.verdict, Verdict::ConfirmStrong);
    assert_eq!(kinds(&r), vec![FlagKind::RegistryUser]);
}

#[test]
fn task_and_service_strong() {
    for s in ["schtasks /create /tn x /tr y", "Stop-Service x"] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}");
        assert_eq!(kinds(&r), vec![FlagKind::ScheduledTaskOrService], "{s}");
    }
}

#[test]
fn absolute_path_strong() {
    let r = scan(r#"Copy-Item "$dir\a" 'D:\Tools\a'"#, SUP);
    assert_eq!(r.verdict, Verdict::ConfirmStrong);
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::AbsolutePath)
        .unwrap();
    assert_eq!(f.detail, "caminho absoluto fora da pasta do app");
}

#[test]
fn worst_verdict_wins() {
    let r = scan("iex x\nSet-ItemProperty 'HKLM:\\X' a 1", SUP);
    assert_eq!(r.verdict, Verdict::Refuse);
    assert!(has(&r, FlagKind::DynamicCode), "{:?}", kinds(&r));
    assert!(has(&r, FlagKind::Hklm), "{:?}", kinds(&r));
    let lines: Vec<usize> = r.flags.iter().map(|f| f.line).collect();
    assert_eq!(lines, vec![1, 2]);
}

#[test]
fn env_var_names_literal() {
    let got = env_var_names("Get-EnvVar -Name 'JAVA_HOME'; Set-EnvVar 'X' 'v'");
    let want: BTreeSet<String> = ["JAVA_HOME", "X"].iter().map(|s| s.to_string()).collect();
    assert_eq!(got, want);
}

#[test]
fn scan_manifest_reports_per_hook() {
    let json = format!(
        r#"{{"version":"1","url":"https://x/a.zip","hash":"{}","pre_install":"iex x","uninstaller":{{"script":["Remove-Item \"$dir\\x\""]}}}}"#,
        "0".repeat(64)
    );
    let m = resolve(&json, Arch::X64).unwrap();
    let got = scan_manifest(&m, SUP);
    let hooks: Vec<HookKind> = got.iter().map(|(h, _)| *h).collect();
    assert_eq!(
        hooks,
        vec![HookKind::PreInstall, HookKind::UninstallerScript]
    );
    assert_eq!(got[0].1.verdict, Verdict::ConfirmStrong);
    assert_eq!(got[1].1.verdict, Verdict::Confirm);
}

const SCRIPTS_1_TO_20: &[&str] = &[
    r#"Remove-Item "$dir\x" -Force"#,
    r#"Start-Process "$dir\x.exe" -Verb RunAs"#,
    "sudo Remove-Item x",
    r"Set-ItemProperty 'HKLM:\SOFTWARE\X' -Name a -Value 1",
    "# HKLM:\\x\nRemove-Item x",
    "$t = @'\nHKLM:\\x\n'@\nSet-Content \"$dir\\a.reg\" $t",
    r#"Add-Path -Path "$dir\bin" -Global"#,
    r#"Copy-Item "$dir\x" "$env:ProgramFiles\X""#,
    r#"Expand-DarkArchive "$dir\a.exe" "$dir\x""#,
    "APPDIR foo",
    "scoop install sudo",
    "Start-Process msiexec -ArgumentList '/i x.msi'",
    "msiexec /i x",
    "iex (Get-Content x -Raw)",
    "& $exe /S",
    r#"Invoke-WebRequest https://x -OutFile "$dir\a""#,
    r#"reg import "$bucketsdir\extras\scripts\vscode\install-context.reg""#,
    r"New-ItemProperty -Path 'HKCU:\Software\X' -Name a -Value 1",
    "schtasks /create /tn x /tr y",
    "Stop-Service x",
    r#"Copy-Item "$dir\a" 'D:\Tools\a'"#,
    "iex x\nSet-ItemProperty 'HKLM:\\X' a 1",
];

#[test]
fn no_text_says_seguro() {
    let mut details = 0;
    for s in SCRIPTS_1_TO_20 {
        for f in scan(s, SUP).flags {
            details += 1;
            assert!(!f.detail.to_lowercase().contains("segur"), "{}", f.detail);
        }
    }
    assert!(details >= 19, "{details}");
}

#[test]
fn scoop_functions_table_matches_oracle_count() {
    assert_eq!(SCOOP_FUNCTIONS.len(), 262);
    for name in ["Expand-7zipArchive", "ensure", "Expand-ZstdArchive"] {
        assert!(SCOOP_FUNCTIONS.contains(&name), "{name}");
    }
}

#[test]
fn alternate_spellings_flagged() {
    let cases: &[(&str, FlagKind)] = &[
        ("Start-Process x -RunAs", FlagKind::Elevation),
        ("gsudo Remove-Item x", FlagKind::Elevation),
        ("Get-Thing -Global:$true", FlagKind::GlobalSwitch),
        ("Get-Thing -Global:$global -Global", FlagKind::GlobalSwitch),
        (
            r"Remove-Item 'Registry::HKEY_LOCAL_MACHINE\X'",
            FlagKind::Hklm,
        ),
        (r"reg add HKLM\Software\X /f", FlagKind::Hklm),
        (
            r#"Remove-Item "${env:ProgramFiles(x86)}\X""#,
            FlagKind::SystemWrite,
        ),
        (
            r"New-Item 'C:\Windows\x' -ItemType File",
            FlagKind::SystemWrite,
        ),
        (". $script", FlagKind::DynamicCode),
        ("[scriptblock]::Create($s)", FlagKind::DynamicCode),
        ("$x | Invoke-Expression", FlagKind::DynamicCode),
        (
            "(New-Object Net.WebClient).DownloadFile($u, $f)",
            FlagKind::Download,
        ),
        ("curl.exe -o a https://x", FlagKind::Download),
        (
            r"Remove-ItemProperty -Path 'HKCU:\Software\X' -Name a",
            FlagKind::RegistryUser,
        ),
        (r"reg delete HKCU\Software\X /f", FlagKind::RegistryUser),
        (
            "Register-ScheduledTask -TaskName x",
            FlagKind::ScheduledTaskOrService,
        ),
        ("sc.exe create x", FlagKind::ScheduledTaskOrService),
        (r"Copy-Item a '\\server\share\a'", FlagKind::AbsolutePath),
        (
            r#"Copy-Item a "$env:USERPROFILE\a""#,
            FlagKind::AbsolutePath,
        ),
        (
            "[Environment]::GetFolderPath('Desktop')",
            FlagKind::AbsolutePath,
        ),
    ];
    for (s, k) in cases {
        let r = scan(s, SUP);
        assert!(has(&r, *k), "{s}: {:?}", kinds(&r));
    }
}

#[test]
fn near_misses_not_flagged() {
    for s in [
        r#"& "$dir\x.exe" /S"#,
        r#"Copy-Item "$dir\curl.exe" "$dir\bin""#,
        "$iex = 1",
        r"Get-ItemProperty 'HKCU:\Software\X'",
        r"reg query HKCU\Software\X",
        "if ($a -and $b) { Remove-Item x }",
        "Write-Host 'Index 1'",
        "$global:x = 1",
        r#"Get-Thing -Path "$persist_dir\bin" -Global:$Global -Force"#,
        "Remove-Item x # HKLM:\\y",
        "<# iex\nHKLM:\\x #>\nRemove-Item x",
        "Write-Host \"#notacomment iex\"",
    ] {
        let r = scan(s, SUP);
        let unexpected: Vec<FlagKind> = kinds(&r)
            .into_iter()
            .filter(|k| !(s.starts_with("Write-Host \"#") && *k == FlagKind::DynamicCode))
            .collect();
        assert!(unexpected.is_empty(), "{s}: {unexpected:?}");
    }
    // `#` inside a string is not a comment: the string content still counts.
    let r = scan("Write-Host \"#notacomment iex\"", SUP);
    assert!(has(&r, FlagKind::DynamicCode));
}

#[test]
fn line_numbers_follow_the_original_script() {
    let r = scan(
        "<# a\nb #>\n$t = @'\nx\n'@\n# c\nSet-ItemProperty 'HKLM:\\X' a 1",
        SUP,
    );
    let f = r.flags.iter().find(|f| f.kind == FlagKind::Hklm).unwrap();
    assert_eq!(f.line, 7);
}

#[test]
fn variables_lowercase_without_dollar() {
    let r = scan(r#"Copy-Item "$Dir\x" "$env:APPDATA\y" $Persist_Dir"#, SUP);
    let want: BTreeSet<String> = ["dir", "env:appdata", "persist_dir"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(r.variables, want);
}
