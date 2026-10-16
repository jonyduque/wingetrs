#Requires -Version 7.0
<#
.SYNOPSIS
    Project rules a diff can break, checked mechanically over src/ (production code only).
    Each rule exists because the defect escaped at least once and was caught only by a reviewer.
    Exit 0 = clean, 1 = violations (listed as path:line: rule).
#>
[CmdletBinding()] param([string]$Path = 'src')
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
    @{ Name = 'println fora de main/cli'; Pattern = '\b(e?println!|e?print!)\('; Skip = '^src[\\/](main\.rs|cli[\\/])' },
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
if ($violations.Count -gt 0) {
    $violations | ForEach-Object { "  $_" }
    exit 1
}
exit 0
