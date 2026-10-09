#Requires -Version 7.0
<#
.SYNOPSIS
    Project rules a diff can break, checked mechanically over src/ (production code only).
    Each rule exists because the defect escaped at least once and was caught only by a reviewer.
    Exit 0 = clean, 1 = violations (listed as path:line: rule).
#>
Set-StrictMode -Version Latest
[Console]::OutputEncoding = [Text.Encoding]::UTF8
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
    # Windows APIs only in *_win.rs.
    @{ Name = 'API Windows fora de *_win.rs'; Pattern = '(?<![\w:])(winreg|windows)::'; Skip = '_win\.rs$' }
)

$violations = @()
$files = Get-ChildItem -Recurse -File -Path src -Filter *.rs
foreach ($f in $files) {
    $rel = [IO.Path]::GetRelativePath($root, $f.FullName)
    $lines = Get-Content $f.FullName
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
    }
}
if ($violations.Count -gt 0) {
    $violations | ForEach-Object { "  $_" }
    exit 1
}
exit 0
