//! Phase 2c tests (Task 150).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::BTreeSet;

use wingetrs::install::manifest::{Arch, HookKind, resolve};
use wingetrs::install::script_scan::{
    CallRoots, FlagKind, SCOOP_FUNCTIONS, ScriptReport, Verdict, env_var_names, scan, scan_called,
    scan_manifest,
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
        // Task 192: the package's own program gets an informational `CalledFile`.
        let unexpected: Vec<FlagKind> = r
            .flags
            .iter()
            .filter(|f| !(f.kind == FlagKind::CalledFile && f.verdict == Verdict::Confirm))
            .map(|f| f.kind)
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
fn typographic_quotes_delimit_like_pwsh() {
    // pwsh closes the `@'` here-string at `’@` (U+2019), so the RunAs line runs.
    let s = "$t = @'\nx\n\u{2019}@\nStart-Process \"$dir\\x.exe\" -Verb RunAs\n'@'";
    let r = scan(s, SUP);
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", kinds(&r));
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::Elevation)
        .unwrap();
    assert_eq!(f.line, 4);
    // `’` closes the `'` string: the Scoop function call between them is code.
    let s = "Write-Output 'abc\u{2019}; Expand-DarkArchive a b; \u{2018}x'";
    let r = scan(s, SUP);
    assert!(has(&r, FlagKind::UnknownScoopFunction), "{:?}", kinds(&r));
    // Double typographic quotes (U+201C/U+201D/U+201E) too.
    let s = "$t = @\"\nx\n\u{201D}@\nStart-Process x -Verb RunAs\n\u{201E}@\u{201C}";
    let r = scan(s, SUP);
    assert!(has(&r, FlagKind::Elevation), "{:?}", kinds(&r));
    assert!(!has(&r, FlagKind::HiddenCode), "{:?}", kinds(&r));
    // Variable names inside `env_var_names` too.
    let got = env_var_names("Set-EnvVar \u{201A}JAVA_HOME\u{201B} 'v'");
    assert!(got.contains("JAVA_HOME"), "{got:?}");
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

// ---- Task 164: `$global` is a constant `$false`; `if ($global)` blocks are informational ----

const GLOBAL_ONLY: &str = "só roda em instalação global";
const OWN_GLOBAL: &str =
    "usa `$global` como nome próprio (incompatível com o `$global` fixo do wingetrs)";

fn informational(r: &ScriptReport) -> bool {
    r.verdict == Verdict::Confirm
        && r.flags
            .iter()
            .any(|f| f.verdict == Verdict::Confirm && f.detail.starts_with(GLOBAL_ONLY))
}

#[test]
fn global_block_is_informational() {
    let r = scan(
        "if ($global) { $c = $c -replace 'HKEY_CURRENT_USER', 'HKEY_LOCAL_MACHINE' }",
        SUP,
    );
    assert_eq!(r.verdict, Verdict::Confirm, "{:?}", r.flags);
    let f: Vec<_> = r
        .flags
        .iter()
        .filter(|f| f.detail.starts_with(GLOBAL_ONLY))
        .collect();
    assert_eq!(f.len(), 1, "{:?}", r.flags);
    assert_eq!(f[0].verdict, Verdict::Confirm);
}

#[test]
fn global_block_multiline() {
    let r = scan(
        "if ($global) {\n    $c = $c -replace 'HKEY_CURRENT_USER', 'HKEY_LOCAL_MACHINE'\n}",
        SUP,
    );
    assert_eq!(r.verdict, Verdict::Confirm, "{:?}", r.flags);
    let f: Vec<_> = r
        .flags
        .iter()
        .filter(|f| f.detail.starts_with(GLOBAL_ONLY))
        .collect();
    assert_eq!(f.len(), 1, "{:?}", r.flags);
    assert_eq!(f[0].verdict, Verdict::Confirm);
    assert_eq!(f[0].line, 2);
}

#[test]
fn global_block_else_still_scanned() {
    for s in [
        "if ($global) { $x = 1 } else { Set-ItemProperty 'HKLM:\\Software\\X' -Name a -Value 1 }",
        "if ($global) {\n    $x = 1\n} else {\n    Set-ItemProperty 'HKLM:\\Software\\X' -Name a -Value 1\n}",
    ] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
        assert!(
            r.flags
                .iter()
                .any(|f| f.kind == FlagKind::Hklm && f.verdict == Verdict::Refuse),
            "{s}: {:?}",
            r.flags
        );
    }
}

#[test]
fn global_and_condition() {
    let hklm = "Set-ItemProperty 'HKLM:\\Software\\X' -Name a -Value 1";
    let r = scan(&format!("if ($global -and $x) {{ {hklm} }}"), SUP);
    assert!(informational(&r), "{:?}", r.flags);
    let r = scan(&format!("if ($global -or $x) {{ {hklm} }}"), SUP);
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", r.flags);
    let r = scan(&format!("if (!$global) {{ {hklm} }}"), SUP);
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", r.flags);
}

#[test]
fn global_block_unbalanced_fails_closed() {
    let r = scan(
        "if ($global) { \"}\" ; Set-ItemProperty 'HKLM:\\Software\\X' -Name a -Value 1 }",
        SUP,
    );
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", r.flags);
}

#[test]
fn own_global_parameter_refused() {
    for s in ["function f($global) { }", "foreach ($global in $a) { }"] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
        assert!(
            r.flags
                .iter()
                .any(|f| f.verdict == Verdict::Refuse && f.detail == OWN_GLOBAL),
            "{s}: {:?}",
            r.flags
        );
    }
}
const HKLM: &str = "Set-ItemProperty 'HKLM:\\Software\\X' -Name a -Value 1";

#[test]
fn global_block_forms_that_are_informational() {
    for s in [
        format!("if ($Global) {{ {HKLM} }}"),
        format!("if (  $GLOBAL  ) {{ {HKLM} }}"),
        format!("if ($global) {{ $y = 1 }} elseif ($global) {{ {HKLM} }}"),
        format!("if ($global)\n{{\n    {HKLM}\n}}"),
        format!("if ($global -and   $x_1) {{ {HKLM} }}"),
        format!("if ($global) {{\n    # machine-wide\n    {HKLM}\n}}"),
        format!("if ($global) {{ $h = @{{ a = 1 }}; {HKLM} }}"),
        format!("if ($global) {{ if ($x) {{ {HKLM} }} }}"),
        format!("if ($global) {{ $p = \"$dir\\x\"; {HKLM} }}"),
    ] {
        let r = scan(&s, SUP);
        assert!(informational(&r), "{s}: {:?}", r.flags);
    }
}

#[test]
fn global_block_doubts_fail_closed() {
    for s in [
        // Conditions other than `$global` / `$global -and ...`.
        format!("if (($global)) {{ {HKLM} }}"),
        format!("if ($global -and   $x -eq 1) {{ {HKLM} }}"),
        format!("if ($global -and $x.y) {{ {HKLM} }}"),
        format!("if ($global -and $x $y) {{ {HKLM} }}"),
        format!("if ($global -and (Test-Path $x)) {{ {HKLM} }}"),
        format!("if (-not $global) {{ {HKLM} }}"),
        format!("if ($global -and -not $x) {{ {HKLM} }}"),
        format!("if ($global -and $x -eq 'a') {{ {HKLM} }}"),
        format!("if ($global -and $x -eq \"a\") {{ {HKLM} }}"),
        format!("if ($global -xor $x) {{ {HKLM} }}"),
        // `-and` and `-or` have the same precedence: `($global -and $x) -or $y`.
        format!("if ($global -and $x -or $y) {{ {HKLM} }}"),
        format!("if ($global -and $x -xor $y) {{ {HKLM} }}"),
        // True with the constant `$false`.
        format!("if ($global -eq $false) {{ {HKLM} }}"),
        format!("if ($global -ne $true) {{ {HKLM} }}"),
        format!("if ($global -andx $x) {{ {HKLM} }}"),
        format!("if ($globalx) {{ {HKLM} }}"),
        format!("if ($global:x) {{ {HKLM} }}"),
        format!("if ($script:global) {{ {HKLM} }}"),
        format!("if ($global\n) {{ {HKLM} }}"),
        // `elseif` only right after a masked block.
        format!("if ($x) {{ $y = 1 }} elseif ($global) {{ {HKLM} }}"),
        format!("elseif ($global) {{ {HKLM} }}"),
        // Opening brace two lines below the condition.
        format!("if ($global)\n\n{{ {HKLM} }}"),
        // No closing brace.
        format!("if ($global) {{ {HKLM}"),
        // Brace in a single-quoted string, a comment, a backtick escape.
        format!("if ($global) {{ '{{' ; {HKLM} }}"),
        format!("if ($global) {{\n    # }}\n    {HKLM}\n}}"),
        format!("if ($global) {{ $x = `}} ; {HKLM} }}"),
        // Here-string and block comment inside the block.
        format!("if ($global) {{\n$t = @'\nx\n'@\n{HKLM}\n}}"),
        format!("if ($global) {{ <# x #> {HKLM} }}"),
        // A brace inside a multi-line block comment: pwsh closes the block at line 5.
        format!("if ($global) {{\n<#\n{{\n#>\n}}\n{HKLM}\n}}"),
        // `x#y` is a word to pwsh, so the quote after it opens a string holding `{`.
        format!("if ($global) {{\n    $a = x#y '\n{{\n'\n# '\n}}\n{HKLM}\n}}"),
        // `a#b` is not a comment to pwsh.
        format!("if ($global) {{ $a = b#c; {HKLM} }}"),
    ] {
        let r = scan(&s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
        assert!(!informational(&r), "{s}: {:?}", r.flags);
    }
}

#[test]
fn global_keyword_in_string_or_comment_is_not_a_block() {
    for s in [
        format!("$s = \"if ($global) {{\"\n{HKLM}\n$t = '}}'"),
        format!("# if ($global) {{\n{HKLM}\n# }}"),
        format!("$t = @'\nif ($global) {{\n'@\n{HKLM}\n}}"),
        format!("$elif ($global) {{ {HKLM} }}"),
    ] {
        let r = scan(&s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
    }
}

#[test]
fn global_block_code_after_it_is_live() {
    let r = scan(&format!("if ($global) {{ $a = 1 }}\n{HKLM}"), SUP);
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", r.flags);
    let f = r.flags.iter().find(|f| f.kind == FlagKind::Hklm).unwrap();
    assert_eq!((f.line, f.verdict), (2, Verdict::Refuse));
}

#[test]
fn global_block_command_rules() {
    // A command seen only inside the block is informational.
    let r = scan("if ($global) {\n    sudo x\n}", SUP);
    assert!(informational(&r), "{:?}", r.flags);
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::Elevation)
        .unwrap();
    assert_eq!(f.line, 2);
    assert!(
        f.detail.ends_with("(pede elevação (RunAs/sudo))"),
        "{}",
        f.detail
    );
    // The same command outside the block still refuses.
    let r = scan("if ($global) { sudo x }\nsudo y", SUP);
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", r.flags);
    let r = scan("if ($global) { get_hash x }", SUP);
    assert!(informational(&r), "{:?}", r.flags);
    let r = scan("if ($global) { get_hash x }\nget_hash y", SUP);
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", r.flags);
}

#[test]
fn global_block_hidden_code_is_informational() {
    // A refusal seen only in the raw text of a block comment line inside the body.
    let r = scan("if ($global) {\n    $x = 1 # HKLM:\\x\n}", SUP);
    assert!(informational(&r), "{:?}", r.flags);
    assert!(has(&r, FlagKind::HiddenCode), "{:?}", kinds(&r));
    // The same outside a block keeps its strong confirmation.
    let r = scan("$x = 1 # HKLM:\\x", SUP);
    assert_eq!(r.verdict, Verdict::ConfirmStrong, "{:?}", r.flags);
}

#[test]
fn own_global_forms() {
    for s in [
        "param($a, [switch]$Global)",
        "function f {\n    param(\n        $a,\n        $GLOBAL\n    )\n}",
        "filter g ([bool]${global}) { }",
        "function script:f ($x, $global = $false) { }",
        "foreach ( $Global in $a) { }",
    ] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
        assert!(has(&r, FlagKind::GlobalName), "{s}: {:?}", kinds(&r));
    }
    let r = scan(
        "function f {\n    param(\n        $a,\n        $global\n    )\n}",
        SUP,
    );
    let f = r
        .flags
        .iter()
        .find(|f| f.kind == FlagKind::GlobalName)
        .unwrap();
    assert_eq!(f.line, 4);
    for s in [
        "function f($globalx) { }",
        "foreach ($g in $global) { }",
        "$global:x = 1",
        "$global = $true",
        "Get-Thing -Global:$global",
        "function f($x) { appdir $x $global }",
        "# function f($global) { }",
        "param($global:x)",
    ] {
        let r = scan(s, SUP);
        assert!(!has(&r, FlagKind::GlobalName), "{s}: {:?}", kinds(&r));
    }
}

#[test]
fn global_block_only_in_statement_position() {
    // In argument position `if` is a string and the `{...}` is a live scriptblock.
    for s in [
        format!("& {{ param($a,$b,$c) & $c }} if ($global) {{ {HKLM} }}"),
        format!("Write-Output if ($global) {{ {HKLM} }}"),
        format!("$x = if ($global) {{ {HKLM} }}"),
        format!("f -p if ($global) {{ {HKLM} }}"),
        format!("f | g if ($global) {{ {HKLM} }}"),
        format!("f a, if ($global) {{ {HKLM} }}"),
        format!(". if ($global) {{ {HKLM} }}"),
        format!("& {{ if ($global) {{ {HKLM} }} }}"),
        // Line continuation: `if` on the next line is still an argument.
        format!("& {{ param($a,$b,$c) & $c }} `\nif ($global) {{ {HKLM} }}"),
        format!("& {{ param($a,$b,$c) & $c }} `\r\n  if ($global) {{ {HKLM} }}"),
        format!("& {{ param($a,$b,$c) & $c }} `  \nif ($global) {{ {HKLM} }}"),
        format!("f a,\nif ($global) {{ {HKLM} }}"),
        format!("f |\nif ($global) {{ {HKLM} }}"),
        // A backtick in a comment is no continuation to pwsh, but on doubt: fail closed.
        format!("& {{ param($a,$b,$c) & $c }} # x `\nif ($global) {{ {HKLM} }}"),
        format!("<# x #> if ($global) {{ {HKLM} }}"),
        // A comment after a continuing comma hides it from the raw text.
        format!("f a, # c\nif ($global) {{ {HKLM} }}"),
    ] {
        let r = scan(&s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
        assert!(!informational(&r), "{s}: {:?}", r.flags);
    }
    for s in [
        format!("$a = 1; if ($global) {{ {HKLM} }}"),
        format!("$a = 1\n  if ($global) {{ {HKLM} }}"),
        format!("$a = 1 # note\nif ($global) {{ {HKLM} }}"),
        // A continuation onto an empty line ends there.
        format!("$a = `\n\nif ($global) {{ {HKLM} }}"),
    ] {
        let r = scan(&s, SUP);
        assert!(informational(&r), "{s}: {:?}", r.flags);
    }
}
#[test]
fn global_condition_unicode_dashes_fail_closed() {
    // pwsh reads U+2013, U+2014 and U+2015 as operator dashes: `-or $true` runs the body.
    for dash in ['\u{2013}', '\u{2014}', '\u{2015}'] {
        let s = format!("if ($global -and $x {dash}or $true) {{ {HKLM} }}");
        let r = scan(&s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
        let s = format!("if ($global {dash}and $x) {{ {HKLM} }}");
        let r = scan(&s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
    }
}

#[test]
fn global_block_in_another_runspace_fails_closed() {
    for s in [
        // `$global` is the runner's constant only in its own runspace.
        format!("Start-Job {{ $global = $true; if ($global) {{ {HKLM} }} }}"),
        format!("pwsh -c {{ $global = $true; if ($global) {{ {HKLM} }} }}"),
        format!("Start-Job {{ if ($global) {{ {HKLM} }} }}"),
        // Nested in a scriptblock literal or a function body: never masked.
        format!("Invoke-Thing {{ if ($global) {{ {HKLM} }} }}"),
        format!("Invoke-Thing {{\n    if ($global) {{\n        {HKLM}\n    }}\n}}"),
        format!("$sb = {{ if ($global) {{ {HKLM} }} }}; & $sb"),
        format!("function f {{ if ($global) {{ {HKLM} }} }}"),
    ] {
        let r = scan(&s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
        assert!(!informational(&r), "{s}: {:?}", r.flags);
    }
}

#[test]
fn global_named_elsewhere_disables_masking() {
    for s in [
        format!("$global = $true\nif ($global) {{ {HKLM} }}"),
        format!("appdir $app $global\nif ($global) {{ {HKLM} }}"),
        format!("Start-Job {{ $global = $true }}\nif ($global) {{ {HKLM} }}"),
        format!("$x = \"$global\"\nif ($global) {{ {HKLM} }}"),
        format!("Add-Path x -Global\nif ($global) {{ {HKLM} }}"),
        format!("$global:y = 1\nif ($global) {{ {HKLM} }}"),
        format!("if ($global) {{ Add-Path x -Global; {HKLM} }}"),
    ] {
        let r = scan(&s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
        assert!(!informational(&r), "{s}: {:?}", r.flags);
    }
    // In conditions and in the exact `-Global:$global` passthrough it does not count.
    for s in [
        format!("if (!$global) {{ $a = 1 }}\nif ($global) {{ {HKLM} }}"),
        format!("Get-Thing x -Global:$global\nif ($global) {{ {HKLM} }}"),
        format!("# $global = $true\nif ($global) {{ {HKLM} }}"),
        format!("$globaldir = 1\nif ($global) {{ {HKLM} }}"),
    ] {
        let r = scan(&s, SUP);
        assert!(informational(&r), "{s}: {:?}", r.flags);
    }
}
/// `post_install` of `main/7zip` (catalog text, 2026-10-18): the shape shared by git, vscode
/// and the other `.reg` templating scripts.
const SEVENZIP_SHAPE: &str = r#"$7zip_dir = $dir -replace '\\', '\\'
Get-ChildItem -Path "$bucketsdir\$bucket\scripts\7-zip" -Filter '*.reg' -File | ForEach-Object {
    $content = Get-Content -Path $_.FullName -Encoding utf8
    if ($global) { $content = $content -replace 'HKEY_CURRENT_USER(?=.+Classes)', 'HKEY_LOCAL_MACHINE' }
    $content -replace '{{7zip_dir}}', $7zip_dir | Set-Content -Path "$dir\$($_.Name)" -Encoding unicode
}"#;

#[test]
fn global_block_in_foreach_object_is_informational() {
    let r = scan(SEVENZIP_SHAPE, SUP);
    assert!(informational(&r), "{:?}", r.flags);
    for s in [
        format!("$a | ForEach-Object {{ if ($global) {{ {HKLM} }} }}"),
        format!("$a | foreach-object {{\n    if ($global) {{\n        {HKLM}\n    }}\n}}"),
        format!("$a |% {{ if ($global) {{ {HKLM} }} }}"),
        format!("$a | % {{ if ($global) {{ {HKLM} }} }} | Out-Null"),
        format!("($a | % {{ if ($global) {{ {HKLM} }} }})"),
        // Statement blocks in statement position.
        format!("foreach ($a in $b) {{ if ($global) {{ {HKLM} }} }}"),
        format!("for ($i = 0; $i -lt 2; $i++) {{ if ($global) {{ {HKLM} }} }}"),
        format!("while ($x) {{ if ($global) {{ {HKLM} }} }}"),
        format!("if ($x) {{ if ($global) {{ {HKLM} }} }}"),
        format!("if ($x) {{ }} elseif ($y) {{ if ($global) {{ {HKLM} }} }}"),
        format!("if ($x) {{ }}\nelse\n{{ if ($global) {{ {HKLM} }} }}"),
        format!("try {{ if ($global) {{ {HKLM} }} }} catch {{ }}"),
        format!("try {{ }} catch {{ if ($global) {{ {HKLM} }} }}"),
        format!("try {{ }} catch {{ }} finally {{ if ($global) {{ {HKLM} }} }}"),
        format!("foreach ($a in $b) {{ $a | % {{ if ($global) {{ {HKLM} }} }} }}"),
        // `-Filter` is a parameter, not the `filter` keyword.
        format!("gci -Filter x | % {{ if ($global) {{ {HKLM} }} }}"),
        format!("foreach ($a in (gci 'x')) {{ if ($global) {{ {HKLM} }} }}"),
        format!(
            "$a | % {{\n    if (Test-Path \"$d\\$_.reg\") {{\n        if ($global) {{ {HKLM} }}\n    }}\n}}"
        ),
    ] {
        let r = scan(&s, SUP);
        assert!(informational(&r), "{s}: {:?}", r.flags);
    }
}

#[test]
fn global_block_in_other_frames_fails_closed() {
    for s in [
        format!("$a | ForEach-Object -Parallel {{ if ($global) {{ {HKLM} }} }}"),
        format!("$a | % -Begin {{}} -Process {{ if ($global) {{ {HKLM} }} }}"),
        format!("$a | % {{ if ($global) {{ {HKLM} }} }} -End {{ }}"),
        format!("$a | ForEach-Object -Process {{ if ($global) {{ {HKLM} }} }}"),
        format!("$a | Where-Object {{ if ($global) {{ {HKLM} }} }}"),
        format!("ForEach-Object {{ if ($global) {{ {HKLM} }} }}"),
        format!("$a || % {{ if ($global) {{ {HKLM} }} }}"),
        format!("$a | x% {{ if ($global) {{ {HKLM} }} }}"),
        format!("$a | My-ForEach-Object {{ if ($global) {{ {HKLM} }} }}"),
        format!("x ForEach-Object {{ if ($global) {{ {HKLM} }} }}"),
        format!("Write-Output if ($x) {{ if ($global) {{ {HKLM} }} }}"),
        format!("& {{ }} elseif ($y) {{ if ($global) {{ {HKLM} }} }}"),
        format!("Write-Output try {{ if ($global) {{ {HKLM} }} }}"),
        format!("$sb = {{ if ($global) {{ {HKLM} }} }}; & $sb"),
        format!("$h = @{{ a = {{ if ($global) {{ {HKLM} }} }} }}"),
        // Keyword blocks out of statement position or out of order.
        format!("& {{ }} else {{ if ($global) {{ {HKLM} }} }}"),
        format!("Write-Output foreach ($a in $b) {{ if ($global) {{ {HKLM} }} }}"),
        format!("catch {{ if ($global) {{ {HKLM} }} }}"),
        format!("& {{ }} finally {{ if ($global) {{ {HKLM} }} }}"),
        format!("if ($x) {{ }} elsif ($y) {{ if ($global) {{ {HKLM} }} }}"),
        format!("switch ($x) {{ a {{ if ($global) {{ {HKLM} }} }} }}"),
        // An unbalanced `}` before the block.
        format!("}}\nif ($global) {{ {HKLM} }}"),
    ] {
        let r = scan(&s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
        assert!(!informational(&r), "{s}: {:?}", r.flags);
    }
}

#[test]
fn mask_killer_words_disable_masking() {
    let body = format!("$a | ForEach-Object {{ if ($global) {{ {HKLM} }} }}");
    for extra in [
        "Set-Alias ForEach-Object Start-Job",
        "Set-Alias x y",
        "iex ('Set-'+'Alias ForEach-Object Start-Job')",
        "iex $x",
        "sal x y",
        "nal x y",
        "Set-Item alias:x y",
        "Invoke-Expression x",
        "[ScriptBlock]::Create('x')",
        "$sb.Invoke()",
        "& $sb",
        ". $sb",
        "Invoke-Item x",
        "Add-Type x",
        "$ExecutionContext.x",
        "Get-Command x",
        "gcm x",
        "New-Module x",
        "Import-Module x",
        "ipmo x",
        "New-Alias x y",
        "Remove-Item alias:%",
        "# -Parallel",
        "Start-Job { }",
        "Start-ThreadJob { }",
        "Invoke-Command { }",
        "[powershell]::Create()",
        "[runspacefactory]::CreateRunspace()",
        "pwsh -c x",
        "& powershell.exe x",
        "function g { }",
        "filter g { }",
        "# FUNCTION",
    ] {
        let s = format!("{extra}\n{body}");
        let r = scan(&s, SUP);
        assert_eq!(r.verdict, Verdict::Refuse, "{s}: {:?}", r.flags);
        assert!(!informational(&r), "{s}: {:?}", r.flags);
    }
    for extra in [
        "gci -Filter x",
        "$functions = 1",
        "$myfilter = 2",
        "$salt = 1",
        "$pwsharch = 1",
        "$mypowershellx = 1",
        "$x = 'banal'",
        "Get-Thing -iexx",
    ] {
        let s = format!("{extra}\n{body}");
        let r = scan(&s, SUP);
        assert!(informational(&r), "{s}: {:?}", r.flags);
    }
}

#[test]
fn parameter_defaults_disable_masking() {
    for extra in [
        "$PSDefaultParameterValues['ForEach-Object:Parallel'] = $true",
        "Set-Variable -Name PSDefaultParameterValues -Value @{}",
        "$PSBoundParameters.Clear()",
    ] {
        let s = format!("{extra}\n{SEVENZIP_SHAPE}");
        let r = scan(&s, SUP);
        assert!(!informational(&r), "{s}: {:?}", r.flags);
        assert!(
            r.flags
                .iter()
                .any(|f| f.kind == FlagKind::Hklm && f.verdict == Verdict::Refuse),
            "{s}: {:?}",
            r.flags
        );
    }
}

// Task 192: files a script runs, and aliases.

/// A wingetrs root with `buckets\extras\scripts\<name>` = `body` and the Task 149 marker.
fn bucket_root(name: &str, body: &str, marker: bool) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let bucket = t.path().join("buckets").join("extras");
    std::fs::create_dir_all(bucket.join("scripts")).unwrap();
    std::fs::write(bucket.join("scripts").join(name), body).unwrap();
    if marker {
        std::fs::write(bucket.join(".scripts-ok"), b"").unwrap();
    }
    t
}

fn called_flag(r: &ScriptReport) -> Option<&wingetrs::install::script_scan::Flag> {
    r.flags.iter().find(|f| f.kind == FlagKind::CalledFile)
}

#[test]
fn call_quoted_path_is_confirm_strong() {
    let r = scan(r#"& "$dir\setup.ps1" /S"#, SUP);
    assert_eq!(r.verdict, Verdict::ConfirmStrong, "{:?}", r.flags);
    let f = called_flag(&r).expect("CalledFile flag");
    assert_eq!(f.verdict, Verdict::ConfirmStrong);
    assert_eq!(
        f.detail,
        r"roda um arquivo que não é mostrado: $dir\setup.ps1"
    );
    for s in [
        r"& '$dir\setup.ps1'",
        r#". "$bucketsdir\extras\scripts\foo.ps1""#,
        r". .\helper.ps1",
        r"&.\tools\install.cmd",
        r#"if ($x) { & "C:\Tools\x.exe" }"#,
        r#"&("$dir\x.ps1")"#,
        r#"& ("$dir\x.ps1")"#,
        r#".("$dir\x.ps1")"#,
        r#". ("$dir\x.ps1")"#,
        r"& (Join-Path $dir x.ps1)",
    ] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}: {:?}", r.flags);
        assert!(called_flag(&r).is_some(), "{s}: {:?}", r.flags);
    }
    // Commands, operators and redirections are not files.
    for s in [
        "& git status",
        "a && b",
        "x 2>&1",
        "$a.Trim()",
        "& { ensure 'x' }",
    ] {
        let r = scan(s, SUP);
        assert!(called_flag(&r).is_none(), "{s}: {:?}", r.flags);
    }
}

#[test]
fn dot_source_bucket_script_scanned() {
    let t = bucket_root(
        "foo.ps1",
        "Set-ItemProperty 'HKLM:\\Software\\X' -Name a -Value 1\r\n",
        true,
    );
    let roots = CallRoots::new(t.path(), None);
    let hook = "ensure 'x'\n. \"$bucketsdir\\extras\\scripts\\foo.ps1\"\n";
    let (r, called) = scan_called(hook, SUP, &roots);
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", r.flags);
    let hklm = r.flags.iter().find(|f| f.kind == FlagKind::Hklm).unwrap();
    assert_eq!(hklm.line, 2);
    assert!(
        hklm.detail.contains(r"$bucketsdir\extras\scripts\foo.ps1"),
        "{}",
        hklm.detail
    );
    assert_eq!(called.len(), 1);
    assert!(called[0].script.contains("HKLM:"));
    assert_eq!(called[0].report.verdict, Verdict::Refuse);
    // A clean called file is shown: its call only informs.
    let t = bucket_root("ok.ps1", "ensure 'ok'\n", true);
    let roots = CallRoots::new(t.path(), None);
    let (r, called) = scan_called(r#". "$bucketsdir\extras\scripts\ok.ps1""#, SUP, &roots);
    assert_eq!(r.verdict, Verdict::Confirm, "{:?}", r.flags);
    assert_eq!(called_flag(&r).unwrap().verdict, Verdict::Confirm);
    assert_eq!(called[0].script, "ensure 'ok'\n");
    // The plain scan never reads files.
    assert_eq!(
        scan(r#". "$bucketsdir\extras\scripts\ok.ps1""#, SUP).verdict,
        Verdict::ConfirmStrong
    );
}

#[test]
fn unresolvable_target_confirm_strong() {
    let t = bucket_root("foo.ps1", "ensure 'x'\n", true);
    let roots = CallRoots {
        dir: Some(t.path().join("buckets")),
        ..CallRoots::new(t.path(), None)
    };
    for s in [
        r#". "$bucketsdir\extras\scripts\missing.ps1""#,
        r#". "$bucketsdir\extras\..\extras\scripts\foo.ps1""#,
        r#". "$bucketsdir\extras\scripts\..\scripts\foo.ps1""#,
        r#". "$bucketsdir\other\scripts\foo.ps1""#,
        r#". "$bucketsdir\$bucket\scripts\foo.ps1""#,
        r#"& "$dir\extras\scripts\$name.ps1""#,
        r#"& "$dir\..\buckets\extras\scripts\foo.ps1""#,
        r#"& "$dir\extras\scripts\foo.txt""#,
        r#"& "$persist_dir\foo.ps1""#,
    ] {
        let (r, called) = scan_called(s, SUP, &roots);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}: {:?}", r.flags);
        assert!(called.is_empty(), "{s}");
    }
    // `$dir` resolves when it is there; without the Task 149 marker the bucket does not.
    let (r, called) = scan_called(r#"& "$dir\extras\scripts\foo.ps1""#, SUP, &roots);
    assert_eq!(
        (r.verdict, called.len()),
        (Verdict::Confirm, 1),
        "{:?}",
        r.flags
    );
    let t = bucket_root("foo.ps1", "ensure 'x'\n", false);
    let roots = CallRoots::new(t.path(), None);
    let (r, called) = scan_called(r#". "$bucketsdir\extras\scripts\foo.ps1""#, SUP, &roots);
    assert_eq!(r.verdict, Verdict::ConfirmStrong, "{:?}", r.flags);
    assert!(called.is_empty());
    // A file that calls itself keeps the strong confirmation and ends.
    let t = bucket_root("loop.ps1", ". \"$PSScriptRoot\\loop.ps1\"\n", true);
    let roots = CallRoots::new(t.path(), None);
    let (r, called) = scan_called(r#". "$bucketsdir\extras\scripts\loop.ps1""#, SUP, &roots);
    assert_eq!(r.verdict, Verdict::ConfirmStrong, "{:?}", r.flags);
    assert_eq!(called.len(), 1);
}

#[test]
fn alias_redefinition_confirm_strong() {
    for s in [
        "Set-Alias shim Out-Null",
        "New-Alias -Name Add-Path -Value Out-Null",
        "sal x y",
        "New-Item alias:Add-Path -Value Out-Null",
        "Set-Item -Path Alias:shim -Value Out-Null",
    ] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}: {:?}", r.flags);
        let f = r.flags.iter().find(|f| f.kind == FlagKind::Alias).unwrap();
        assert_eq!(f.detail, "redefine comando por apelido");
    }
    assert!(!has(&scan("Get-Alias", SUP), FlagKind::Alias));
}

#[test]
fn called_file_disables_global_masking() {
    // Task 164's masking of `if ($global)` bodies stays off when another file runs.
    let s = "if ($global) { Set-ItemProperty 'HKLM:\\X' -Name a -Value 1 }\n. \"$dir\\x.ps1\"\n";
    let r = scan(s, SUP);
    assert_eq!(r.verdict, Verdict::Refuse, "{:?}", r.flags);
}

#[test]
fn package_program_call_is_confirm() {
    for (s, name) in [
        (r#"& "$dir\setup.exe" /S"#, "setup.exe"),
        (r#"& "$persist_dir\tool.com""#, "tool.com"),
        (r#"& "$dir\bin\X.EXE" --quiet"#, "X.EXE"),
        (r"$dir\x.exe /S", "x.exe"),
    ] {
        let r = scan(s, SUP);
        let f = called_flag(&r).unwrap_or_else(|| panic!("{s}: {:?}", r.flags));
        assert_eq!(f.verdict, Verdict::Confirm, "{s}");
        assert_eq!(
            f.detail,
            format!("roda um programa do pacote: {name}"),
            "{s}"
        );
        assert_eq!(r.verdict, Verdict::Confirm, "{s}: {:?}", r.flags);
    }
}

#[test]
fn script_or_interpreter_call_stays_confirm_strong() {
    for ext in ["ps1", "psm1", "cmd", "bat", "vbs", "js", "wsf", "hta"] {
        let s = format!(r#"& "$dir\tool.{ext}""#);
        let r = scan(&s, SUP);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}: {:?}", r.flags);
        assert_eq!(
            called_flag(&r).unwrap().verdict,
            Verdict::ConfirmStrong,
            "{s}"
        );
    }
    let r = scan(r#"& "$dir\tool""#, SUP);
    assert_eq!(r.verdict, Verdict::ConfirmStrong, "{:?}", r.flags);
}

#[test]
fn program_outside_package_stays_confirm_strong() {
    for s in [
        r#"& "C:\Tools\x.exe""#,
        r#"& "$env:TEMP\x.exe""#,
        r#"& "$bucketsdir\extras\scripts\x.exe""#,
        r#"& "$dir\..\x.exe""#,
        r#"& "$dir\$name.exe""#,
        r#"& "$dirx\x.exe""#,
    ] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}: {:?}", r.flags);
        assert_eq!(
            called_flag(&r).unwrap().verdict,
            Verdict::ConfirmStrong,
            "{s}"
        );
    }
}

#[test]
fn computed_call_target_is_confirm_strong() {
    for s in [
        r#"&("$dir\x.exe")"#,
        r"& (Join-Path $dir x.exe)",
        r#". ("$dir\x.ps1")"#,
    ] {
        let r = scan(s, SUP);
        let f = called_flag(&r).unwrap_or_else(|| panic!("{s}: {:?}", r.flags));
        assert_eq!(f.verdict, Verdict::ConfirmStrong, "{s}");
        assert_eq!(
            f.detail, "roda um arquivo ou comando calculado na hora",
            "{s}"
        );
    }
}

#[test]
fn path_in_command_position_is_a_call() {
    for s in [
        r".\x.ps1",
        "./x.cmd",
        r"..\x.ps1",
        r"Push-Location $dir; .\setup.ps1",
        r"if ($true) { .\setup.cmd }",
        r"C:\Tools\x.exe /S",
        r"$dir\x.cmd",
        r"ensure 'x' && .\x.ps1",
    ] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}: {:?}", r.flags);
        assert_eq!(
            called_flag(&r).unwrap().verdict,
            Verdict::ConfirmStrong,
            "{s}"
        );
    }
    // Arguments are not commands.
    for s in [
        r"Copy-Item .\a .\b",
        r#"$x = "$dir\a.ps1""#,
        r"Get-Thing -Path $dir\x.ps1",
        r#"$v = "$dir\bin;$dir\lib;$dir\fonts".Replace('\','\\')"#,
        r"$p = '$dir;.\x.ps1'",
    ] {
        let r = scan(s, SUP);
        assert!(called_flag(&r).is_none(), "{s}: {:?}", r.flags);
    }
    // A `$dir` path is resolved like `& "$dir\..."`.
    let t = bucket_root("ok.ps1", "ensure 'x'\n", true);
    let roots = CallRoots {
        dir: Some(t.path().join("buckets").join("extras").join("scripts")),
        ..CallRoots::default()
    };
    let (r, called) = scan_called(r"$dir\ok.ps1", SUP, &roots);
    assert_eq!(
        (r.verdict, called.len()),
        (Verdict::Confirm, 1),
        "{:?}",
        r.flags
    );
}

#[test]
fn file_run_by_another_program_is_confirm_strong() {
    for s in [
        r#"Start-Process "$dir\x.ps1""#,
        r#"Start-Process -FilePath "$dir\x.cmd" -Wait"#,
        r#"saps "$dir\x.bat""#,
        r#"start "$dir\x.cmd""#,
        r#"powershell -File "$dir\x.ps1""#,
        r#"powershell.exe -ExecutionPolicy Bypass -File "$dir\x.ps1""#,
        r#"pwsh -File "$dir\x.ps1""#,
        r#"cmd /c "$dir\x.bat""#,
        "cmd /c x.bat",
        r#"cmd.exe /c "$dir\x.cmd""#,
        r#"Invoke-Item "$dir\x.ps1""#,
        r#"ii "$dir\x.cmd""#,
        r#"Import-Module "$dir\x.psm1""#,
        r#"ipmo "$dir\x.psm1""#,
        r#"wscript "$dir\x.vbs""#,
        r#"cscript //nologo "$dir\x.js""#,
        r#"mshta "$dir\x.hta""#,
        r#"rundll32 "$dir\x.dll",Entry"#,
        r#"regsvr32 /s "$dir\x.dll""#,
        r#"mshta "$dir\x.exe""#,
        r#"[Diagnostics.Process]::Start("$dir\x.cmd")"#,
        r#"[System.Diagnostics.Process]::Start("$dir\x.ps1")"#,
        r#"Start-Process '$dir\x.exe'"#,
        r#"if ($x) { Start-Process "$dir\x.cmd" }"#,
    ] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}: {:?}", r.flags);
        let f = called_flag(&r).unwrap_or_else(|| panic!("{s}: {:?}", r.flags));
        assert!(
            f.detail
                .starts_with("roda um arquivo que não é mostrado por outro programa"),
            "{s}: {}",
            f.detail
        );
    }
    // The package's own program (double-quoted `.exe` in `$dir`) and plain commands are not.
    for s in [
        r#"Start-Process "$dir\x.exe" -ArgumentList '/S' -Wait"#,
        r#"Start-Process -FilePath "$dir\setup.exe" -ArgumentList "/D=$dir""#,
        r#"[System.Diagnostics.Process]::Start("$dir\x.exe")"#,
        r#"pwsh -NoProfile -Command "Get-Date""#,
        "Write-Host 'start the app'",
        "Start-Sleep 1",
    ] {
        let r = scan(s, SUP);
        assert!(called_flag(&r).is_none(), "{s}: {:?}", r.flags);
    }
}

#[test]
fn package_program_needs_double_quotes_and_a_file_name() {
    for s in [r"& '$dir\x.exe'", r#"& "$dir\x.exe\""#] {
        let r = scan(s, SUP);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}: {:?}", r.flags);
    }
}

#[test]
fn junction_out_of_base_stays_unresolved() {
    let t = bucket_root("ok.ps1", "ensure 'x'\n", true);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("evil.ps1"), "ensure 'x'\n").unwrap();
    let scripts = t.path().join("buckets").join("extras").join("scripts");
    wingetrs::install::link::create(outside.path(), &scripts.join("j")).unwrap();
    let dir = t.path().join("app");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ok.ps1"), "ensure 'x'\n").unwrap();
    wingetrs::install::link::create(outside.path(), &dir.join("j")).unwrap();
    let roots = CallRoots {
        dir: Some(dir.clone()),
        ..CallRoots::new(t.path(), None)
    };
    for s in [
        r#". "$bucketsdir\extras\scripts\j\evil.ps1""#,
        r#"& "$dir\j\evil.ps1""#,
    ] {
        let (r, called) = scan_called(s, SUP, &roots);
        assert_eq!(r.verdict, Verdict::ConfirmStrong, "{s}: {:?}", r.flags);
        assert!(called.is_empty(), "{s}");
    }
    // The same roots resolve a file that stays inside.
    for s in [
        r#". "$bucketsdir\extras\scripts\ok.ps1""#,
        r#"& "$dir\ok.ps1""#,
    ] {
        let (r, called) = scan_called(s, SUP, &roots);
        assert_eq!((r.verdict, called.len()), (Verdict::Confirm, 1), "{s}");
    }
}

/// Task 203 item 8: the same reason on two lines (`-Global` twice, as in python's
/// `uninstaller.script`) is one flag, on its first line; other reasons stay. The same holds
/// for a called file run from two lines (its reasons join the caller's once).
#[test]
fn same_reason_on_two_lines_is_one_flag() {
    let script = [
        r#"Get-Thing -Path "$dir\a""#,
        r#"Add-Path -Path "$dir\bin" -Global"#,
        r#"Remove-Path -Path "$dir\bin" -Global"#,
        r#"Start-Process "$dir\x.exe" -Verb RunAs"#,
    ]
    .join("\n");
    let script = script.as_str();
    let r = scan(script, SUP);
    let global: Vec<usize> = r
        .flags
        .iter()
        .filter(|f| f.kind == FlagKind::GlobalSwitch)
        .map(|f| f.line)
        .collect();
    assert_eq!(global, vec![2], "{:?}", r.flags);
    assert!(has(&r, FlagKind::Elevation), "{:?}", kinds(&r));
    let details: Vec<&str> = r.flags.iter().map(|f| f.detail.as_str()).collect();
    let unique: BTreeSet<&str> = details.iter().copied().collect();
    assert_eq!(details.len(), unique.len(), "{details:?}");

    let t = bucket_root(
        "foo.ps1",
        r"Set-ItemProperty 'HKLM:\Software\X' -Name a -Value 1",
        true,
    );
    let roots = CallRoots::new(t.path(), None);
    let call = r#". "$bucketsdir\extras\scripts\foo.ps1""#;
    let hook = format!("ensure 'x'\n{call}\n{call}\n");
    let hook = hook.as_str();
    let (r, _) = scan_called(hook, SUP, &roots);
    let hklm: Vec<usize> = r
        .flags
        .iter()
        .filter(|f| f.kind == FlagKind::Hklm)
        .map(|f| f.line)
        .collect();
    assert_eq!(hklm, vec![2], "{:?}", r.flags);
    assert_eq!(r.verdict, Verdict::Refuse);
}
