#Requires -Version 7.0
<#
.SYNOPSIS
    Project gate: cargo fmt --check, clippy (warnings = errors), tests.
    Prints a compact summary; the full log goes to target/verify.log.
    Exit 0 = green, 1 = a step failed.
#>
[CmdletBinding()]
param([switch]$NoTests)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force target | Out-Null
$log = Join-Path $root 'target/verify.log'
"verify $(Get-Date -Format o)" | Set-Content $log

$steps = @(
    @{ Name = 'fmt';    Cmd = { cargo fmt --all -- --check 2>&1 } },
    @{ Name = 'clippy'; Cmd = { cargo clippy --quiet --all-targets --message-format short -- -D warnings 2>&1 } }
)
if (-not $NoTests) {
    $steps += @{ Name = 'test'; Cmd = { cargo test --quiet --all-targets 2>&1 } }
}

$failed = $false
foreach ($s in $steps) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $out = & $s.Cmd | ForEach-Object { "$_" }
    $code = $LASTEXITCODE
    $sw.Stop()
    "==== $($s.Name) (exit $code)" | Add-Content $log
    $out | Add-Content $log
    if ($code -eq 0) {
        $extra = ''
        if ($s.Name -eq 'test') {
            $passed = ($out | Select-String -Pattern 'test result: ok\. (\d+) passed' -AllMatches |
                ForEach-Object { $_.Matches } | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum
            $ignored = ($out | Select-String -Pattern '(\d+) ignored' -AllMatches |
                ForEach-Object { $_.Matches } | ForEach-Object { [int]$_.Groups[1].Value } | Measure-Object -Sum).Sum
            $extra = " ($passed passed, $ignored ignored)"
        }
        "[OK]   $($s.Name)$extra $([int]$sw.Elapsed.TotalSeconds)s"
    } else {
        $failed = $true
        "[FAIL] $($s.Name) exit $code"
        # Only the lines that matter: errors, failing tests, panics. Capped.
        $out | Select-String -Pattern '^(error|warning)|^---- .* stdout ----|panicked at|assertion|FAILED|^Diff in|left:|right:' |
            Select-Object -First 40 | ForEach-Object { "  $($_.Line)" }
        "  (log completo: target/verify.log)"
        break
    }
}
if ($failed) { exit 1 } else { "[OK]   gate verde"; exit 0 }
