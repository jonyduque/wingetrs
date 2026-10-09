#Requires -Version 7.0
<#
.SYNOPSIS
    Lists processes left behind by agent shells (Claude Code bash/pwsh sessions) that are still
    running after a task: wait loops, whole-disk `find`, dev servers. Report only - never kills.

.DESCRIPTION
    Why: a subagent's forgotten `find / -name oracle.py` ran for hours at full CPU and invalidated
    the benchmarks; another left an `until ... sleep` loop. The orchestrator runs this after every
    task and asks the user before stopping anything that is not clearly the task's own.
    EXIT 0 = nothing found, 1 = leftovers listed.

.EXAMPLE
    pwsh -File scripts/leftovers.ps1 -MinAgeMinutes 2
#>
param([int]$MinAgeMinutes = 2)
Set-StrictMode -Version Latest
[Console]::OutputEncoding = [Text.Encoding]::UTF8

$all = Get-CimInstance Win32_Process
$byParent = $all | Group-Object ParentProcessId -AsHashTable -AsString
$roots = $all | Where-Object { $_.CommandLine -and $_.CommandLine -match 'shell-snapshots' }
$cutoff = (Get-Date).AddMinutes(-$MinAgeMinutes)
$seen = @{}
$found = @()

function Walk($p, $depth) {
    if ($seen.ContainsKey([string]$p.ProcessId)) { return }
    $seen[[string]$p.ProcessId] = $true
    if ($p.CreationDate -lt $cutoff) { $script:found += $p }
    $kids = $byParent[[string]$p.ProcessId]
    if ($kids -and $depth -lt 6) { foreach ($k in $kids) { Walk $k ($depth + 1) } }
}
foreach ($r in $roots) { Walk $r 0 }

# The shell running this script (and its parents) is not a leftover.
$self = $PID
$chain = @{}
$cur = $all | Where-Object ProcessId -eq $self
while ($cur) { $chain[[string]$cur.ProcessId] = $true; $cur = $all | Where-Object ProcessId -eq $cur.ParentProcessId }
$found = $found | Where-Object { -not $chain.ContainsKey([string]$_.ProcessId) }

if (-not $found) { "[OK] nenhum processo de agente sobrando (> $MinAgeMinutes min)"; exit 0 }
"[!] processos de shells de agente ainda vivos:"
$found | Sort-Object CreationDate | ForEach-Object {
    $cmd = if ($_.CommandLine) { $_.CommandLine } else { '' }
    $tail = $cmd.Substring([math]::Max(0, $cmd.Length - 140))
    "  PID {0,-6} {1,-10} desde {2:dd/MM HH:mm}  ...{3}" -f $_.ProcessId, $_.Name, $_.CreationDate, $tail
}
"[i] Não encerre automaticamente: confirme que é da tarefa (ou peça ao usuário) e use taskkill /PID <n> /F."
exit 1
