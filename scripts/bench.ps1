# Wall-clock benchmark of a whole process: warmup runs, then min/median/max in ms.
# Usage: pwsh -File scripts/bench.ps1 -Exe target/release/wingetrs.exe -Args 'search powertoys' -Runs 10 -Warmup 2
# -Args is one string split on spaces (pwsh -File cannot pass arrays); -DelayMs sleeps between runs.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Alias('Args')][string]$Arguments = '',
    [int]$Runs = 10,
    [int]$Warmup = 2,
    [int]$DelayMs = 0
)
$ErrorActionPreference = 'Stop'
$argv = @($Arguments -split '\s+' | Where-Object { $_ -ne '' })
for ($i = 0; $i -lt $Warmup; $i++) {
    & $Exe @argv *> $null
    if ($DelayMs -gt 0) { Start-Sleep -Milliseconds $DelayMs }
}
$samples = New-Object System.Collections.Generic.List[double]
$lastExit = 0
for ($i = 0; $i -lt $Runs; $i++) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    & $Exe @argv *> $null
    $sw.Stop()
    $lastExit = $LASTEXITCODE
    $samples.Add($sw.Elapsed.TotalMilliseconds)
    if ($DelayMs -gt 0) { Start-Sleep -Milliseconds $DelayMs }
}
$sorted = $samples | Sort-Object
$n = $sorted.Count
if ($n % 2 -eq 1) { $median = $sorted[[int][math]::Floor($n / 2)] }
else { $median = ($sorted[$n / 2 - 1] + $sorted[$n / 2]) / 2 }
'{0} {1} | runs={2} warmup={3} exit={4} | min={5:N1} ms median={6:N1} ms max={7:N1} ms' -f `
    $Exe, ($argv -join ' '), $n, $Warmup, $lastExit, $sorted[0], $median, $sorted[$n - 1]
