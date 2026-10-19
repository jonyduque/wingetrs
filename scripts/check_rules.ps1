#Requires -Version 7.0
<#
.SYNOPSIS
    Project rules a diff can break, checked mechanically over src/ (production code only).
    Each rule exists because the defect escaped at least once and was caught only by a reviewer.
    Exit 0 = clean, 1 = violations (listed as path:line: rule).
#>
[CmdletBinding()] param([string]$Path = 'src', [string]$ItDir = '', [string]$TestsDir = '')
Set-StrictMode -Version Latest
[Console]::OutputEncoding = [Text.Encoding]::UTF8
# Any internal error must fail the gate, never pass it silently (a .Count on a one-line file once skipped checks).
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$rules = @(
    # Tasks 18, 20 and final review: swallowed worker panics returned partial results silently.
    @{ Name = 'join engole panico'; Pattern = '\.join\(\)\s*\.(ok\(\)|unwrap_or_default\(\)|unwrap_or\()'; Skip = '' },
    @{ Name = 'join engole panico (if let Ok)'; Pattern = 'if let Ok\([^)]*\)\s*=\s*[\w.]*\.join\(\)'; Skip = '' },
    # The TUI owns the terminal: stdout only from main.rs and cli/.
    @{ Name = 'println fora de main/cli'; Pattern = '\b(e?println!|e?print!)\('; Skip = '(^|[\\/])src[\\/](main\.rs|cli[\\/])' },
    # Task 199: inside cli/ every line goes out through emit.rs, with its text from output.rs
    # (glyph, color, alignment in one place). Any stdout()/stderr() handle (std::io or anstream,
    # locked or not, `write!` or `write_all`) and dbg! are a print by another name; only the
    # selector (picker.rs) owns its terminal.
    @{ Name = 'saida do CLI fora de cli/emit.rs'; Pattern = '\b(e?println!|e?print!|dbg!)\(|\bstd(out|err)\(\)'; Skip = '^(?!(.*[\\/])?src[\\/]cli[\\/])|(^|[\\/])src[\\/]cli[\\/](emit|picker)\.rs$' },
    # Platform code lives in *_win.rs behind cfg on the module, never inline.
    @{ Name = 'cfg!(windows) na logica'; Pattern = 'cfg!\(\s*windows\s*\)'; Skip = '' },
    # Colors only through theme tokens.
    @{ Name = 'Color:: fora de theme.rs'; Pattern = '\bColor::'; Skip = '^src[\\/]tui[\\/]theme\.rs$' },
    # CLI colors only through src/cli/style.rs.
    @{ Name = 'anstyle fora de cli/style.rs'; Pattern = '\b(anstyle|AnsiColor)::'; Skip = '^src[\\/]cli[\\/]style\.rs$' },
    # Windows APIs only in *_win.rs.
    @{ Name = 'API Windows fora de *_win.rs'; Pattern = '(?<![\w:])(winreg|windows)::'; Skip = '_win\.rs$' }
)

$violations = @()
$files = Get-ChildItem -Recurse -File -Path $Path -Filter *.rs
foreach ($f in $files) {
    $rel = [IO.Path]::GetRelativePath($root, $f.FullName)
    $lines = @(Get-Content $f.FullName)
    $inTests = $false
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        # Unit-test modules are exempt (they may print or unwrap).
        if ($line -match '^\s*#\[cfg\(test\)\]') { $inTests = $true }
        if ($inTests) { continue }
        if ($line -match '^\s*//') { continue }
        foreach ($r in $rules) {
            if ($r.Skip -and $rel -match $r.Skip) { continue }
            if ($line -match $r.Pattern) {
                $violations += "{0}:{1}: [{2}] {3}" -f $rel, ($i + 1), $r.Name, $line.Trim()
            }
        }
        # Task 44: a dropped spawn_blocking handle lets tokio swallow the worker panic.
        # Bound = `let <name> =` (name not starting with `_`) on this line or ending the previous
        # line, or `.await` on this line or starting the next one.
        if ($line -match '\bspawn_blocking\s*\(') {
            $bound = ($line -match '\blet\s+(mut\s+)?[A-Za-z]\w*\s*(:[^=]+)?=.*spawn_blocking') -or
                     ($line -match 'spawn_blocking.*\.await')
            # $i -gt 0: $lines[-1] is the LAST line in PowerShell, not "no line".
            if (-not $bound -and $i -gt 0) {
                $bound = $lines[$i - 1] -match '\blet\s+(mut\s+)?[A-Za-z]\w*\s*(:[^=]+)?=\s*$'
            }
            if (-not $bound -and $i + 1 -lt $lines.Count) {
                $bound = $lines[$i + 1] -match '^\s*\.await'
            }
            if (-not $bound) {
                $violations += "{0}:{1}: [{2}] {3}" -f $rel, ($i + 1), 'spawn_blocking sem handle', $line.Trim()
            }
        }
    }
}
# Tasks 112 and 129: wall-clock bounds in tests (`elapsed() < x`) failed the gate under load or
# after the machine slept. Checked in test code too (unlike the rules above); a deliberate one
# carries `clock-ok: <why>` on the same line.
$clockRoots = @($Path)
if ($Path -eq 'src' -and (Test-Path 'tests')) { $clockRoots += 'tests' }
foreach ($f in (Get-ChildItem -Recurse -File -Path $clockRoots -Filter *.rs)) {
    $rel = [IO.Path]::GetRelativePath($root, $f.FullName)
    $lines = @(Get-Content $f.FullName)
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '\.elapsed\(\)\s*<' -and $lines[$i] -notmatch 'clock-ok') {
            $violations += "{0}:{1}: [{2}] {3}" -f $rel, ($i + 1), 'limite de relogio em teste', $lines[$i].Trim()
        }
    }
}
# Task 160: the real PowerShell host runs third-party code. In src/ it is named only where it is
# built and wired; in tests/ the real one (`PwshHost::locate`) only in the live file, where every
# test carries `#[ignore = "live: ...` right below its attribute (`with_program(<fake>)` is free),
# and `build_ctx(` (production wiring, real host included) only inside a live-ignored test. An
# alias (`PwshHost as X`, `type X = PwshHost`) would hide the name from these checks: refused.
$pwshTests = if ($TestsDir) { $TestsDir } elseif ($Path -eq 'src' -and (Test-Path 'tests')) { 'tests' } else { '' }
$pwshFiles = @(Get-ChildItem -Recurse -File -Path $Path -Filter *.rs | ForEach-Object { @{ F = $_; Test = $false } })
if ($pwshTests) {
    $pwshFiles += @(Get-ChildItem -Recurse -File -Path $pwshTests -Filter *.rs | ForEach-Object { @{ F = $_; Test = $true } })
}
$pwshWired = '^src[\\/]install[\\/](script_host_win|prod_win)\.rs$'
foreach ($pf in $pwshFiles) {
    $f = $pf.F
    $rel = [IO.Path]::GetRelativePath($root, $f.FullName)
    $isTest = $pf.Test
    $isLive = $isTest -and ([IO.Path]::GetRelativePath((Resolve-Path $pwshTests).Path, $f.FullName) -match '^it[\\/]script_host_live\.rs$')
    $lines = @(Get-Content $f.FullName)
    # Whether the fn being read is a test whose attributes include `#[ignore = "live:`.
    $inTestAttrs = $false; $attrLive = $false; $fnLive = $false
    for ($i = 0; $i -lt $lines.Count; $i++) {
        $line = $lines[$i]
        if ($line -match '^\s*//') { continue }
        if ($line -match '\bPwshHost\s+as\b|=\s*PwshHost\s*;') {
            $violations += "{0}:{1}: [{2}] {3}" -f $rel, ($i + 1), 'PwshHost com apelido', $line.Trim()
        }
        if ($isTest) {
            if ($line -match '^\s*#\[(tokio::)?test\b') { $inTestAttrs = $true; $attrLive = $false }
            elseif ($inTestAttrs -and $line -match '^\s*#\[ignore\s*=\s*"live:') { $attrLive = $true }
            if ($line -match '^\s*(pub(\([^)]*\))?\s+)?(async\s+)?fn\s') {
                $fnLive = $inTestAttrs -and $attrLive
                $inTestAttrs = $false
            }
            if ($line -match '\bPwshHost\s*::\s*locate\b' -and -not $isLive) {
                $violations += "{0}:{1}: [{2}] {3}" -f $rel, ($i + 1), 'PwshHost real fora do teste ao vivo', $line.Trim()
            }
            if ($line -match '\bbuild_ctx\s*\(' -and -not $fnLive) {
                $violations += "{0}:{1}: [{2}] {3}" -f $rel, ($i + 1), 'build_ctx em teste sem ignore live', $line.Trim()
            }
        } elseif ($line -match '\bPwshHost\b' -and $rel -notmatch $pwshWired) {
            $violations += "{0}:{1}: [{2}] {3}" -f $rel, ($i + 1), 'PwshHost fora de script_host_win/prod_win', $line.Trim()
        }
        if ($isLive -and $line -match '^\s*#\[(tokio::)?test\b') {
            $j = $i + 1
            while ($j -lt $lines.Count -and $lines[$j] -match '^\s*$') { $j++ }
            if ($j -ge $lines.Count -or $lines[$j] -notmatch '^\s*#\[ignore\s*=\s*"live:') {
                $violations += "{0}:{1}: [{2}] {3}" -f $rel, ($i + 1), 'teste do PwshHost sem ignore live', $line.Trim()
            }
        }
    }
}
# Task 133: tests/it/ is one binary; a file there without `mod <stem>;` in main.rs is never
# compiled nor run, and nothing else would say so.
if (-not $ItDir -and $Path -eq 'src') { $ItDir = 'tests/it' }
if ($ItDir -and (Test-Path (Join-Path $ItDir 'main.rs'))) {
    $mainText = Get-Content -Raw (Join-Path $ItDir 'main.rs')
    $mods = @(Get-ChildItem -File -Path $ItDir -Filter *.rs | Where-Object Name -ne 'main.rs' | ForEach-Object BaseName) +
            @(Get-ChildItem -Directory -Path $ItDir | Where-Object { Test-Path (Join-Path $_.FullName 'mod.rs') } | ForEach-Object Name)
    foreach ($m in $mods) {
        if ($mainText -notmatch "(?m)^\s*(pub\s+)?mod\s+$([regex]::Escape($m))\s*;") {
            $violations += "{0}: [{1}] {2}" -f (Join-Path $ItDir 'main.rs'), 'modulo de teste sem mod no main.rs', $m
        }
    }
}
if ($violations.Count -gt 0) {
    $violations | ForEach-Object { "  $_" }
    exit 1
}
exit 0
