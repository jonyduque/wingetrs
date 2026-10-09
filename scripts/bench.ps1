# Wall-clock benchmark of a whole process: warmup runs, then min/median/max in ms.
# Usage: pwsh -File scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'search powertoys' -Runs 10 -Warmup 2
# -Args is one string split on spaces (pwsh -File cannot pass arrays); -DelayMs sleeps between runs.
# -Baseline <exe>: also runs a do-nothing process interleaved with the target (same machine load)
#   and reports the median of the paired differences (target - baseline). Process start-up on this
#   machine swings 38-67 ms with background load, so compare deltas, not absolute times.
#   Build the baseline with: cargo build --release --example noop  (target/release/examples/noop.exe)
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Alias('Args')][string]$Arguments = '',
    [int]$Runs = 10,
    [int]$Warmup = 2,
    [int]$DelayMs = 0,
    [string]$Baseline = ''
)
$ErrorActionPreference = 'Stop'
$argv = @($Arguments -split '\s+' | Where-Object { $_ -ne '' })

function Measure-Once([string]$path, [string[]]$a) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    & $path @a *> $null
    $sw.Stop()
    return $sw.Elapsed.TotalMilliseconds
}
function Get-Median([double[]]$xs) {
    $s = $xs | Sort-Object
    $n = $s.Count
    if ($n % 2 -eq 1) { return $s[[int][math]::Floor($n / 2)] }
    return ($s[$n / 2 - 1] + $s[$n / 2]) / 2
}

for ($i = 0; $i -lt $Warmup; $i++) {
    & $Exe @argv *> $null
    if ($Baseline) { & $Baseline *> $null }
    if ($DelayMs -gt 0) { Start-Sleep -Milliseconds $DelayMs }
}
$samples = New-Object System.Collections.Generic.List[double]
$base = New-Object System.Collections.Generic.List[double]
$delta = New-Object System.Collections.Generic.List[double]
$lastExit = 0
for ($i = 0; $i -lt $Runs; $i++) {
    $t = Measure-Once $Exe $argv
    $lastExit = $LASTEXITCODE
    $samples.Add($t)
    if ($Baseline) {
        $b = Measure-Once $Baseline @()
        $base.Add($b)
        $delta.Add($t - $b)
    }
    if ($DelayMs -gt 0) { Start-Sleep -Milliseconds $DelayMs }
}
$sorted = $samples | Sort-Object
$n = $sorted.Count
$line = '{0} {1} | runs={2} warmup={3} exit={4} | min={5:N1} ms median={6:N1} ms max={7:N1} ms' -f `
    $Exe, ($argv -join ' '), $n, $Warmup, $lastExit, $sorted[0], (Get-Median $samples), $sorted[$n - 1]
if ($Baseline) {
    $line += ' | baseline median={0:N1} ms | delta median={1:N1} ms' -f (Get-Median $base), (Get-Median $delta)
}
$line
