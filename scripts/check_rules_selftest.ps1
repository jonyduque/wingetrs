#Requires -Version 7.0
<#
.SYNOPSIS
    Proves check_rules.ps1 still sees violations: plants each case in its own temp dir, runs the
    checker on it and compares the exit code. Exit 0 = every case behaved, 1 = the instrument went blind.
#>
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$checker = Join-Path $PSScriptRoot 'check_rules.ps1'
$bound = @(
    'async fn a() -> u8 {',
    '    let h = tokio::task::spawn_blocking(|| 1);',
    '    let lock =',
    '        tokio::task::spawn_blocking(|| 2)',
    '            .await;',
    '    let r = spawn_blocking(|| 3).await;',
    '    let job: JoinHandle<u8> = tokio::task::spawn_blocking(|| 4);',
    '    // tokio::task::spawn_blocking(|| 5);',
    '    tokio::task::spawn_blocking(|| 6)',
    '        .await',
    '}'
) -join "`n"
$cases = @(
    @{ Name = 'arquivo vazio'; Text = ''; Want = 0 },
    @{ Name = 'uma linha, caminho completo'; Text = 'fn f() { tokio::task::spawn_blocking(|| 1); }'; Want = 1 },
    @{ Name = 'uma linha, nome curto'; Text = 'fn f() { spawn_blocking(|| 1); }'; Want = 1 },
    @{ Name = 'let _ ='; Text = 'let _ = tokio::task::spawn_blocking(|| 1);'; Want = 1 },
    @{ Name = 'let _h ='; Text = 'let _h = tokio::task::spawn_blocking(|| 1);'; Want = 1 },
    @{ Name = '1a linha nao olha a ultima'; Text = "tokio::task::spawn_blocking(|| 1);`nlet x ="; Want = 1 },
    @{ Name = 'forma do spawn_boot'; Text = "fn f(c: u8) {`n    let c = c;`n    tokio::task::spawn_blocking(move || {`n        let _ = c;`n    });`n}"; Want = 1 },
    @{ Name = 'formas ligadas'; Text = $bound; Want = 0 },
    @{ Name = 'regra antiga viva'; Text = 'fn f() { println!("x"); }'; Want = 1 },
    @{ Name = 'anstyle fora de cli/style.rs'; Text = 'const X: anstyle::Style = anstyle::Style::new();'; Want = 1 },
    @{ Name = 'limite de relogio'; Text = 'fn t() { assert!(s.elapsed() < d); }'; Want = 1 },
    @{ Name = 'limite de relogio com clock-ok'; Text = 'fn t() { assert!(s.elapsed() < d); } // clock-ok: cap only'; Want = 0 },
    @{ Name = 'PwshHost fora de script_host_win/prod_win'; Text = 'fn f() { let _ = PwshHost::with_program(p); }'; Want = 1 },
    @{ Name = 'PwshHost com apelido'; Text = 'use crate::install::script_host_win::PwshHost as Host;'; Want = 1 }
)
# Task 160: test-side rules (`-TestsDir`): file under it/, text, expected exit.
$testCases = @(
    @{ Name = 'locate fora do arquivo ao vivo'; File = 'it/other.rs'; Text = 'fn f() { let _ = PwshHost::locate(); }'; Want = 1 },
    @{ Name = 'locate em teste ao vivo sem ignore'; File = 'it/script_host_live.rs'; Text = "#[test]`nfn t() { let _ = PwshHost::locate(); }"; Want = 1 },
    @{ Name = 'locate em teste ao vivo com ignore'; File = 'it/script_host_live.rs'; Text = "#[tokio::test]`n#[ignore = `"live: x`"]`nasync fn t() { let _ = PwshHost::locate(); }"; Want = 0 },
    @{ Name = 'with_program com falso'; File = 'it/other.rs'; Text = 'fn f() { let _ = PwshHost::with_program(p); }'; Want = 0 },
    @{ Name = 'apelido por type em teste'; File = 'it/other.rs'; Text = 'type H = PwshHost;'; Want = 1 },
    @{ Name = 'build_ctx em teste comum'; File = 'it/other.rs'; Text = "#[test]`nfn t() { let _ = build_ctx(a); }"; Want = 1 },
    @{ Name = 'build_ctx em teste ao vivo'; File = 'it/other.rs'; Text = "#[test]`n#[ignore = `"live: x`"]`nfn t() { let _ = build_ctx(a); }"; Want = 0 },
    @{ Name = 'build_ctx em ajudante depois de teste ao vivo'; File = 'it/other.rs'; Text = "#[test]`n#[ignore = `"live: x`"]`nfn t() {}`nfn helper() { let _ = build_ctx(a); }"; Want = 1 }
)
$base = Join-Path ([IO.Path]::GetTempPath()) ("check_rules_selftest-" + [guid]::NewGuid())
$bad = @()
try {
    $n = 0
    foreach ($c in $cases) {
        $n++
        $dir = Join-Path $base "case$n"
        New-Item -ItemType Directory -Force $dir | Out-Null
        [IO.File]::WriteAllText((Join-Path $dir 'x.rs'), $c.Text)
        pwsh -NoProfile -File $checker -Path $dir *> $null
        if ($LASTEXITCODE -ne $c.Want) { $bad += "  [$($c.Name)] exit $LASTEXITCODE, esperado $($c.Want)" }
    }
    # tests/it/: a module file without `mod` in main.rs must fail; registered ones pass.
    foreach ($it in @(@{ Main = "mod a;`n"; Want = 1 }, @{ Main = "mod a;`n#[allow(dead_code)]`nmod b;`nmod spans;`n"; Want = 0 })) {
        $n++
        $dir = Join-Path $base "case$n"
        New-Item -ItemType Directory -Force (Join-Path $dir 'spans') | Out-Null
        [IO.File]::WriteAllText((Join-Path $dir 'main.rs'), $it.Main)
        [IO.File]::WriteAllText((Join-Path $dir 'a.rs'), '')
        [IO.File]::WriteAllText((Join-Path $dir 'b.rs'), '')
        [IO.File]::WriteAllText((Join-Path $dir 'spans/mod.rs'), '')
        pwsh -NoProfile -File $checker -Path $dir -ItDir $dir *> $null
        if ($LASTEXITCODE -ne $it.Want) { $bad += "  [tests/it sem mod ($($it.Want))] exit $LASTEXITCODE, esperado $($it.Want)" }
    }
    foreach ($c in $testCases) {
        $n++
        $dir = Join-Path $base "case$n"
        $src = Join-Path $dir 'src'
        $tests = Join-Path $dir 'tests'
        $file = Join-Path $tests $c.File
        New-Item -ItemType Directory -Force $src, (Split-Path -Parent $file) | Out-Null
        [IO.File]::WriteAllText($file, $c.Text)
        pwsh -NoProfile -File $checker -Path $src -TestsDir $tests *> $null
        if ($LASTEXITCODE -ne $c.Want) { $bad += "  [$($c.Name)] exit $LASTEXITCODE, esperado $($c.Want)" }
    }
} finally {
    Remove-Item -Recurse -Force $base -ErrorAction SilentlyContinue
}
if ($bad.Count -gt 0) { $bad; exit 1 }
"[OK] check_rules: $($cases.Count + $testCases.Count + 2) casos plantados"
exit 0
