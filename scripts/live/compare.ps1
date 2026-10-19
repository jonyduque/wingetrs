#Requires -Version 7.0
<#
.SYNOPSIS
    Compares two snapshot.ps1 folders area by area (added / removed / changed) and prints a summary.

.DESCRIPTION
    Read-only. EXIT 0 = no unexpected change; 2 = ~/scoop changed (the proof must not touch it);
    1 = a snapshot folder is missing.

.EXAMPLE
    pwsh -File scripts/live/compare.ps1 -Before <run>\antes -After <run>\depois
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Before,
    [Parameter(Mandatory)][string]$After,
    [int]$MaxLines = 60
)
Set-StrictMode -Version Latest
[Console]::OutputEncoding = [Text.Encoding]::UTF8

foreach ($d in $Before, $After) {
    if (-not (Test-Path -LiteralPath $d -PathType Container)) { Write-Output "[x] pasta de retrato não encontrada: $d"; exit 1 }
}

# Keyed CSV areas: key column and the columns whose change counts as "changed".
$csvAreas = [ordered]@{
    'scoop.csv' = @{ Label = '~/scoop'; Key = 'FullName'; Fields = @('Length', 'LastWriteTimeUtc') }
    'arp.csv' = @{ Label = 'ARP (desinstalação)'; Key = 'Key'; Fields = @('DisplayName', 'DisplayVersion', 'Publisher', 'UninstallString') }
    'msix.csv' = @{ Label = 'MSIX'; Key = 'PackageFullName'; Fields = @() }
    'startmenu.csv' = @{ Label = 'Menu Iniciar (wingetrs Apps)'; Key = 'FullName'; Fields = @('Target') }
    'root.csv' = @{ Label = 'raiz do wingetrs (apps, shims, persist, modules)'; Key = 'FullName'; Fields = @('Length') }
    'run.csv' = @{ Label = 'installers\run-*'; Key = 'FullName'; Fields = @() }
    'staging.csv' = @{ Label = 'Program Files\wingetrs\staging'; Key = 'FullName'; Fields = @() }
}
$textAreas = [ordered]@{
    'env.reg' = 'HKCU\Environment'
    'path-user.txt' = 'PATH do usuário'
    'path-machine.txt' = 'PATH da máquina'
    'list.txt' = 'wingetrs list'
    '7zip.reg' = 'HKCU\Software\7-Zip'
    'leftovers.txt' = 'leftovers.ps1 (informativo)'
}

$summary = [Collections.Generic.List[object]]::new()

function Show-Lines([string]$Prefix, [string[]]$Lines) {
    $n = 0
    foreach ($l in $Lines) {
        if ($n -ge $MaxLines) { Write-Output "    ... mais $($Lines.Count - $MaxLines)"; break }
        Write-Output "  $Prefix $l"
        $n++
    }
}

foreach ($file in $csvAreas.Keys) {
    $a = $csvAreas[$file]
    Write-Output ''
    Write-Output "== $($a.Label) ($file) =="
    $fb = Join-Path $Before $file; $fa = Join-Path $After $file
    if (-not (Test-Path -LiteralPath $fb) -or -not (Test-Path -LiteralPath $fa)) {
        Write-Output '  (não capturado em um dos retratos)'
        $summary.Add([pscustomobject]@{ Area = $a.Label; Added = '?'; Removed = '?'; Changed = '?' }); continue
    }
    $b = @{}; foreach ($r in @(Import-Csv -LiteralPath $fb)) { $b[$r.($a.Key)] = $r }
    $x = @{}; foreach ($r in @(Import-Csv -LiteralPath $fa)) { $x[$r.($a.Key)] = $r }
    $added = @($x.Keys | Where-Object { -not $b.ContainsKey($_) } | Sort-Object)
    $removed = @($b.Keys | Where-Object { -not $x.ContainsKey($_) } | Sort-Object)
    $changed = @(foreach ($k in ($x.Keys | Where-Object { $b.ContainsKey($_) } | Sort-Object)) {
            $diffs = @(foreach ($f in $a.Fields) { if ($b[$k].$f -ne $x[$k].$f) { "${f}: '$($b[$k].$f)' -> '$($x[$k].$f)'" } })
            if ($diffs) { "$k ($($diffs -join '; '))" }
        })
    $label = { param($k, $row) if ($file -eq 'arp.csv' -and $row.DisplayName) { "$k = $($row.DisplayName) $($row.DisplayVersion)" } else { $k } }
    Show-Lines '+' @($added | ForEach-Object { & $label $_ $x[$_] })
    Show-Lines '-' @($removed | ForEach-Object { & $label $_ $b[$_] })
    Show-Lines '~' $changed
    if (-not ($added -or $removed -or $changed)) { Write-Output '  (sem diferenças)' }
    $summary.Add([pscustomobject]@{ Area = $a.Label; Added = $added.Count; Removed = $removed.Count; Changed = $changed.Count })
}

foreach ($file in $textAreas.Keys) {
    $label = $textAreas[$file]
    Write-Output ''
    Write-Output "== $label ($file) =="
    $fb = Join-Path $Before $file; $fa = Join-Path $After $file
    if (-not (Test-Path -LiteralPath $fb) -or -not (Test-Path -LiteralPath $fa)) {
        Write-Output '  (não capturado em um dos retratos)'
        $summary.Add([pscustomobject]@{ Area = $label; Added = '?'; Removed = '?'; Changed = '?' }); continue
    }
    $lb = @(Get-Content -LiteralPath $fb | Where-Object { $_ -ne '' })
    $la = @(Get-Content -LiteralPath $fa | Where-Object { $_ -ne '' })
    $d = @(Compare-Object -ReferenceObject $lb -DifferenceObject $la)
    $added = @($d | Where-Object SideIndicator -eq '=>' | ForEach-Object InputObject)
    $removed = @($d | Where-Object SideIndicator -eq '<=' | ForEach-Object InputObject)
    Show-Lines '+' $added
    Show-Lines '-' $removed
    if (-not $d) { Write-Output '  (sem diferenças)' }
    $summary.Add([pscustomobject]@{ Area = $label; Added = $added.Count; Removed = $removed.Count; Changed = '' })
}

$scoop = $summary | Where-Object Area -eq '~/scoop'
$scoopChanged = $scoop -and (($scoop.Added -ne 0) -or ($scoop.Removed -ne 0) -or ($scoop.Changed -ne 0))

Write-Output ''
Write-Output '== Resumo =='
Write-Output ('  {0,-48} {1,6} {2,6} {3,6}' -f 'área', '+', '-', '~')
foreach ($s in $summary) { Write-Output ('  {0,-48} {1,6} {2,6} {3,6}' -f $s.Area, $s.Added, $s.Removed, $s.Changed) }
Write-Output ''
if ($scoopChanged) {
    Write-Output '[x] ~/scoop mudou: inesperado (a prova não pode tocar no ~/scoop).'
    exit 2
}
Write-Output '[ok] ~/scoop sem diferenças. Confira as outras áreas contra o roteiro (E): ARP, MSIX, raiz e list sem sobras.'
exit 0
