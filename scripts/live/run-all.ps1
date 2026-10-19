#Requires -Version 7.0
<#
.SYNOPSIS
    Runs the live-proof sections in order (0, A, B, C, D, E) in one run folder; resumes with -From.

.DESCRIPTION
    -From 0 (default) creates a new run folder under %LOCALAPPDATA%\wingetrs-prova\run-<data>.
    -From B (etc.) reuses -RunDir, or the most recent run folder when -RunDir is omitted.
    Asks before each section; a section that ends with q stops the run.
    EXIT 0 = all sections ran without failed checks, 1 = some check failed, 3 = stopped by the user.

.EXAMPLE
    pwsh -File scripts/live/run-all.ps1
    pwsh -File scripts/live/run-all.ps1 -From C
    pwsh -File scripts/live/run-all.ps1 -DryRun
    pwsh -File scripts/live/run-all.ps1 -From B -Auto      # no prompts; UAC/visual steps deferred
    pwsh -File scripts/live/run-all.ps1 -From C -Usuario   # only the deferred steps, interactive
#>
[CmdletBinding()]
param(
    [ValidateSet('0', 'A', 'B', 'C', 'D', 'E')][string]$From = '0',
    [string]$RunDir,
    [string]$W = 'C:\Program Files\wingetrs\wingetrs.exe',
    [string]$Wdev,
    [switch]$DryRun,
    [switch]$Auto,
    [switch]$Usuario
)
Set-StrictMode -Version Latest
[Console]::OutputEncoding = [Text.Encoding]::UTF8

if ($Auto -and $Usuario) { Write-Output '[x] use -Auto ou -Usuario, não os dois'; exit 1 }
$env:WINGETRS_PROVA_MODE = if ($Auto) { 'auto' } elseif ($Usuario) { 'usuario' } else { '' }

$root = Join-Path $env:LOCALAPPDATA 'wingetrs-prova'
if (-not $RunDir) {
    if ($From -eq '0') {
        $RunDir = Join-Path $root ('run-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
    } else {
        $last = Get-ChildItem -Directory -LiteralPath $root -Filter 'run-*' -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
        if (-not $last) { Write-Output "[x] nenhuma execução anterior em $root; comece com -From 0 ou passe -RunDir"; exit 1 }
        $RunDir = $last.FullName
    }
}
$RunDir = [IO.Path]::GetFullPath($RunDir)
Write-Output "Pasta da execução: $RunDir"

$order = '0', 'A', 'B', 'C', 'D', 'E'
$sections = $order[([array]::IndexOf($order, $From))..($order.Count - 1)]
$common = @{ OutDir = $RunDir; W = $W }
if ($Wdev) { $common.Wdev = $Wdev }
if ($DryRun) { $common.DryRun = $true }

$worst = 0
foreach ($s in $sections) {
    if (-not $DryRun -and -not $Auto) {
        $r = (Read-Host "Seção $s — Enter começa · q encerra (retome depois com -From $s)").Trim().ToLowerInvariant()
        if ($r -eq 'q') { Write-Output "Encerrado antes da seção $s. Retome com: pwsh -File scripts/live/run-all.ps1 -From $s -RunDir '$RunDir'"; exit 3 }
    }
    & (Join-Path $PSScriptRoot "section-$($s.ToLowerInvariant()).ps1") @common
    $code = $LASTEXITCODE
    if ($code -eq 3) {
        Write-Output "Seção $s encerrada pelo usuário. Retome com: pwsh -File scripts/live/run-all.ps1 -From $s -RunDir '$RunDir' (ou section-$($s.ToLowerInvariant()).ps1 -StartAt <passo>)"
        exit 3
    }
    if ($code -gt $worst) { $worst = $code }
}
if ($DryRun) { Write-Output 'Simulação concluída: nada foi executado nem gravado.' }
else { Write-Output "Fim. Resultados (section-*.md) e logs em $RunDir" }
exit $worst
