#Requires -Version 7.0
<#
.SYNOPSIS
    Extract the section of one task from the plan into .claude/tasks/task-N-brief.md
    and append the shared execution rules and report contract
    (.claude/tasks/_common.md), so nothing binding stays stuck in the plan preamble.

.EXAMPLE
    pwsh -File scripts/brief.ps1 3
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory, Position = 0)] [int]$Task,
    [string]$Plan = 'docs/superpowers/plans/2026-10-07-wingetrs-fase1.md'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$lines = Get-Content $Plan
$start = -1
for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match "^### Task $Task\b") { $start = $i; break }
}
if ($start -lt 0) { "[!] Task $Task não encontrada em $Plan"; exit 2 }
$end = $lines.Count
for ($j = $start + 1; $j -lt $lines.Count; $j++) {
    if ($lines[$j] -match '^### Task \d+' -or $lines[$j] -match '^## ') { $end = $j; break }
}
$section = $lines[$start..($end - 1)]
$common = (Get-Content '.claude/tasks/_common.md' -Raw).Replace('{N}', "$Task")
$out = ".claude/tasks/task-$Task-brief.md"
$header = @(
    "<!-- gerado por scripts/brief.ps1 a partir de $Plan; não editar à mão -->",
    ''
)
Set-Content -Path $out -Value ($header + $section + @('', $common)) -Encoding utf8NoBOM
"[OK] $out ($((Get-Content $out).Count) linhas)"
