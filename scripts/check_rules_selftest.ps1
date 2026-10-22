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
# Task 199: output of src/cli/ only through cli/emit.rs (file under <case>/, text, expected exit).
$cliCases = @(
    @{ Name = 'println em cli/x.rs'; File = 'src/cli/x.rs'; Text = 'fn f() { anstream::println!("x"); }'; Want = 1 },
    @{ Name = 'eprint em cli/x.rs'; File = 'src/cli/x.rs'; Text = 'fn f() { eprint!("x"); }'; Want = 1 },
    @{ Name = 'stderr travado em cli/x.rs'; File = 'src/cli/x.rs'; Text = 'fn f() { let _ = std::io::stderr().lock(); }'; Want = 1 },
    @{ Name = 'println em cli/emit.rs'; File = 'src/cli/emit.rs'; Text = 'fn f() { anstream::println!("x"); }'; Want = 0 },
    @{ Name = 'stdout sem trava fora do seletor'; File = 'src/cli/x.rs'; Text = 'fn f() { let _ = std::io::stdout(); }'; Want = 1 },
    @{ Name = 'stdout guardado e writeln'; File = 'src/cli/x.rs'; Text = "fn f() {`n    let mut o = std::io::stdout();`n    writeln!(o, `"x`").ok();`n}"; Want = 1 },
    @{ Name = 'stdout write_all'; File = 'src/cli/x.rs'; Text = 'fn f() { let _ = std::io::stdout().write_all(b"x"); }'; Want = 1 },
    @{ Name = 'writeln em anstream::stdout'; File = 'src/cli/x.rs'; Text = 'fn f() { let _ = writeln!(anstream::stdout(), "x"); }'; Want = 1 },
    @{ Name = 'dbg em cli/x.rs'; File = 'src/cli/x.rs'; Text = 'fn f() { dbg!(1); }'; Want = 1 },
    @{ Name = 'lock na linha seguinte'; File = 'src/cli/x.rs'; Text = "fn f() {`n    let _ = std::io::stderr()`n        .lock();`n}"; Want = 1 },
    @{ Name = 'stdout no seletor'; File = 'src/cli/picker.rs'; Text = 'fn f() { let _ = std::io::stdout(); }'; Want = 0 },
    @{ Name = 'stderr em cli/emit.rs'; File = 'src/cli/emit.rs'; Text = 'fn f() { let _ = std::io::stderr().lock(); }'; Want = 0 },
    @{ Name = 'Http::new no selfupdate elevado'; File = 'src/selfupdate/apply_win.rs'; Text = 'fn f() { let _ = crate::http::Http::new(); }'; Want = 1 },
    @{ Name = 'Http::new no bridge-worker'; File = 'src/bridge/worker.rs'; Text = 'fn f() { let _ = Http::new(); }'; Want = 1 },
    @{ Name = 'Http::new fora do lado elevado'; File = 'src/update.rs'; Text = 'fn f() { let _ = Http::new(); }'; Want = 0 },
    @{ Name = 'for_elevated no selfupdate'; File = 'src/selfupdate/apply_win.rs'; Text = 'fn f() { let _ = Http::for_elevated(&r); }'; Want = 0 }
)
# Task 222: `skip_if_elevated_in_ci` only in the tests of non_elevated.txt (files under it/, list).
$skip = "    if skip_if_elevated_in_ci(`"x`") {`n        return;`n    }`n"
$nested = "mod win {`n    #[test]`n    fn t() {`n        if skip_if_elevated_in_ci(`"x`") {`n            return;`n        }`n    }`n}`n"
$elevCases = @(
    @{ Name = 'pulo em teste da lista'; A = "#[test]`nfn t() {`n$skip}`n"; List = 'a::t'; Want = 0 },
    @{ Name = 'pulo em teste fora da lista'; A = "#[test]`nfn t() {}`n`n#[test]`nfn u() {`n$skip}`n"; List = 'a::t'; Want = 1 },
    @{ Name = 'lista com teste que nao existe'; A = "#[test]`nfn t() {`n$skip}`n"; List = "a::t`na::sumiu"; Want = 1 },
    @{ Name = 'lista com fn que nao e teste'; A = "fn t() {}`n"; List = 'a::t'; Want = 1 },
    @{ Name = 'pulo em ajudante'; A = "#[test]`nfn t() {`n    h();`n}`n`nfn h() {`n$skip}`n"; List = 'a::t'; Want = 1 },
    @{ Name = 'pulo em mod aninhado'; A = $nested; List = 'a::win::t'; Want = 0 },
    @{ Name = 'mod aninhado sem o nome do mod'; A = $nested; List = 'a::t'; Want = 1 }
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
    foreach ($c in $cliCases) {
        $n++
        $dir = Join-Path $base "case$n"
        $file = Join-Path $dir $c.File
        New-Item -ItemType Directory -Force (Split-Path -Parent $file) | Out-Null
        [IO.File]::WriteAllText($file, $c.Text)
        pwsh -NoProfile -File $checker -Path $dir *> $null
        if ($LASTEXITCODE -ne $c.Want) { $bad += "  [$($c.Name)] exit $LASTEXITCODE, esperado $($c.Want)" }
    }
    foreach ($c in $elevCases) {
        $n++
        $dir = Join-Path $base "case$n"
        $it = Join-Path $dir 'it'
        New-Item -ItemType Directory -Force (Join-Path $dir 'src'), $it | Out-Null
        [IO.File]::WriteAllText((Join-Path $it 'main.rs'), "mod a;`n")
        [IO.File]::WriteAllText((Join-Path $it 'a.rs'), $c.A)
        [IO.File]::WriteAllText((Join-Path $it 'non_elevated.txt'), $c.List + "`n")
        pwsh -NoProfile -File $checker -Path (Join-Path $dir 'src') -ItDir $it *> $null
        if ($LASTEXITCODE -ne $c.Want) { $bad += "  [$($c.Name)] exit $LASTEXITCODE, esperado $($c.Want)" }
    }
} finally {
    Remove-Item -Recurse -Force $base -ErrorAction SilentlyContinue
}
if ($bad.Count -gt 0) { $bad; exit 1 }
"[OK] check_rules: $($cases.Count + $testCases.Count + $cliCases.Count + $elevCases.Count + 2) casos plantados"
exit 0
