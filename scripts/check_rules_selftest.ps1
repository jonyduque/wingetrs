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
    @{ Name = 'anstyle fora de cli/style.rs'; Text = 'const X: anstyle::Style = anstyle::Style::new();'; Want = 1 }
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
} finally {
    Remove-Item -Recurse -Force $base -ErrorAction SilentlyContinue
}
if ($bad.Count -gt 0) { $bad; exit 1 }
"[OK] check_rules: $($cases.Count) casos plantados"
exit 0
