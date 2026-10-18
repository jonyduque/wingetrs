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
    // Fails closed (review of ce3af24): a refusal rule seen only inside a comment asks
    // for a strong confirmation, never a refusal.
    assert_eq!(r.verdict, Verdict::ConfirmStrong, "{:?}", kinds(&r));
    assert_eq!(kinds(&r), vec![FlagKind::HiddenCode]);
    assert_eq!(
        r.flags[0].detail,
        "possível código escondido em comentário ou here-string: mexe no registro da máquina (HKLM)"
    );
    assert_eq!(r.flags[0].line, 1);
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
        "Remove-Item x # iex",
        "<# iex\nInvoke-WebRequest #>\nRemove-Item x",
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
fn refusal_hidden_in_here_string_is_strong_not_refuse() {
    let r = scan("$t = @'\nHKLM:\\x\n'@\nSet-Content \"$dir\\a.reg\" $t", SUP);
    assert_eq!(r.verdict, Verdict::ConfirmStrong, "{:?}", kinds(&r));
    assert_eq!(kinds(&r), vec![FlagKind::HiddenCode]);
    assert_eq!(r.flags[0].line, 2);
}

#[test]
fn hash_bypass_still_refused_without_hidden_flag() {
    let r = scan(
        "$x =#@'\nStart-Process \"$dir\\x.exe\" -Verb RunAs\n'@'",
        SUP,
    );
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", kinds(&r));
    assert!(!has(&r, FlagKind::HiddenCode), "{:?}", kinds(&r));
    // Same rule in the clean line and in a trailing comment: one refusal, no hidden flag.
    let r = scan("Set-ItemProperty 'HKLM:\\X' a 1 # HKLM:", SUP);
    assert_eq!(kinds(&r), vec![FlagKind::Hklm]);
}

#[test]
fn ambiguous_comment_start_fails_closed() {
    // Whether `<#` glued to a word opens a block comment is ambiguous; the stripper
    // hides the RunAs line, the raw text still shows it.
    let r = scan("Get-Thing a<#b\nStart-Process x -Verb RunAs", SUP);
    assert_eq!(r.verdict, Verdict::ConfirmStrong, "{:?}", kinds(&r));
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::HiddenCode)
        .unwrap();
    assert_eq!(
        f.detail,
        "possível código escondido em comentário ou here-string: pede elevação (RunAs/sudo)"
    );
    assert_eq!(f.line, 2);
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
fn hash_after_non_word_char_is_a_comment() {
    // `=#` opens a comment in pwsh, so `@'` is no here-string and the RunAs line runs.
    let r = scan(
        "$x =#@'\nStart-Process \"$dir\\x.exe\" -Verb RunAs\n'@'",
        SUP,
    );
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", kinds(&r));
    assert!(has(&r, FlagKind::Elevation));
    // Glued to a word, `#` is part of it and the rest of the line still counts.
    let r = scan("Get-Thing a#b; iex x", SUP);
    assert!(has(&r, FlagKind::DynamicCode), "{:?}", kinds(&r));
}

#[test]
fn backticks_inside_words_are_ignored() {
    let cases: &[(&str, FlagKind, Verdict)] = &[
        (
            r#"Start-Process "$dir\x.exe" -Verb Run`As"#,
            FlagKind::Elevation,
            Verdict::Refuse,
        ),
        (
            r"Set-ItemProperty 'HK`LM:\SOFTWARE\X' -Name a -Value 1",
            FlagKind::Hklm,
            Verdict::Refuse,
        ),
        (
            "Inv`oke-Expression $s",
            FlagKind::DynamicCode,
            Verdict::ConfirmStrong,
        ),
    ];
    for (s, k, v) in cases {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, *v, "{s}: {:?}", kinds(&r));
        assert!(has(&r, *k), "{s}: {:?}", kinds(&r));
    }
}

#[test]
fn global_passthrough_only_with_the_exact_variable() {
    for s in [
        "Get-Thing -Global:$globalx",
        "Get-Thing -Global:$global2",
        "Get-Thing -Glo",
        "Get-Thing -Glob",
        "Get-Thing -GLOBA",
        "Get-Thing -Glob:$true",
    ] {
        let r = scan(s, SUP);
        assert!(has(&r, FlagKind::GlobalSwitch), "{s}: {:?}", kinds(&r));
    }
    for s in [
        "Get-Thing -Global:$global",
        "Get-Thing -Global:$Global)",
        "Get-Thing -Globe",
    ] {
        let r = scan(s, SUP);
        assert!(!has(&r, FlagKind::GlobalSwitch), "{s}: {:?}", kinds(&r));
    }
}

#[test]
fn hklm_through_dotnet_refused() {
    for s in [
        "[Microsoft.Win32.Registry]::LocalMachine.CreateSubKey('SOFTWARE\\X')",
        "$k = [Microsoft.Win32.RegistryKey]::OpenBaseKey('LocalMachine', 'Registry64')",
        "$k = [Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine, 0)",
    ] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}");
        assert!(has(&r, FlagKind::Hklm), "{s}: {:?}", kinds(&r));
    }
}

#[test]
fn program_files_write_through_aliases_and_dotnet() {
    for s in [
        r#"cp "$dir\x" "$env:ProgramFiles\X""#,
        r#"robocopy "$dir" "$env:ProgramW6432\X" /E"#,
        r#"[IO.File]::WriteAllText("$env:CommonProgramFiles\x.txt", 'a')"#,
        r#"mkdir "${env:CommonProgramFiles(x86)}\X""#,
    ] {
        let r = scan(s, SUP);
        assert!(has(&r, FlagKind::SystemWrite), "{s}: {:?}", kinds(&r));
    }
}

/// Refused with exactly one flag, of kind `k`.
fn refused_only_by(s: &str, k: FlagKind) {
    let r = scan(s, SUP);
    assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", kinds(&r));
    assert_eq!(kinds(&r), vec![k], "{s}");
}

#[test]
fn prelude_internals_refused() {
    for s in [
        "__wr_effect env_set X 1",
        "$__wr_effects = 'x'",
        "Write-Output ${__wr_effects}",
        "Get-Item function:__wr_effect",
        "Get-Variable __wr*",
    ] {
        refused_only_by(s, FlagKind::PreludeInternal);
    }
    let r = scan("Get-Variable __wr*", SUP);
    assert_eq!(
        r.flags[0].detail,
        "mexe nos internos do prelúdio do wingetrs (__wr)"
    );
}

#[test]
fn prelude_redefinition_refused() {
    for s in [
        "function ensure($d) { $d }",
        "function global:Ensure { }",
        "filter info { $_ }",
        "Set-Item function:ensure { }",
        "New-Item -Path function:appdir -Value { 'x' }",
        "Remove-Item function:Get-HelperPath",
        "${function:ensure} = { }",
    ] {
        refused_only_by(s, FlagKind::PreludeRedefinition);
    }
    let r = scan("function global:Ensure { }", SUP);
    assert_eq!(
        r.flags[0].detail,
        "redefine a função do prelúdio do wingetrs: ensure"
    );
    // Own helper functions and reading a non-prelude function are fine.
    for s in ["function my_helper { }", "Get-Item function:my_helper"] {
        let r = scan(s, SUP);
        assert!(r.flags.is_empty(), "{s}: {:?}", kinds(&r));
    }
}

#[test]
fn set_environment_variable_target_decides() {
    for s in [
        "[Environment]::SetEnvironmentVariable('X', 'v', 'User')",
        "[System.Environment]::SetEnvironmentVariable(\"X\", $v, \"Machine\")",
        "[Environment]::SetEnvironmentVariable('X', 'v', [EnvironmentVariableTarget]::User)",
        "[Environment]::SetEnvironmentVariable('X', (Get-Thing 'a', 'b'), 'user')",
    ] {
        refused_only_by(s, FlagKind::EnvironmentVariable);
    }
    for s in [
        "[Environment]::SetEnvironmentVariable('X', 'v')",
        "[Environment]::SetEnvironmentVariable('X', 'User, Machine', 'Process')",
    ] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}: {:?}", kinds(&r));
        assert_eq!(kinds(&r), vec![FlagKind::EnvironmentVariable], "{s}");
    }
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
