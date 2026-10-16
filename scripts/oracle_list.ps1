#requires -Version 7
# Oracle for `wingetrs list`: compares correlation against Get-WinGetPackage (read-only).
param(
    [string]$Exe = 'target/release/wingetrs.exe',
    [string]$WingetrsJson,
    [string]$WingetJson,
    [string]$AllowFile = (Join-Path $PSScriptRoot 'oracle_list.allow.json'),
    [string]$OutJson,
    [switch]$SelfTest
)
$ErrorActionPreference = 'Stop'

function Get-Norm([string]$v) { ($v -replace '\s+', '') }

function Get-MatchedWinget($row) {
    $c = $row.correlation
    if ($c -is [string] -or $null -eq $c) { return $null }
    $m = $c.PSObject.Properties['Matched']
    if (-not $m -or $m.Value.source -ne 'winget') { return $null }
    $m.Value.package_id
}

function Get-RowState($row) {
    $c = $row.correlation
    if ($c -is [string]) { return $c }
    ($c.PSObject.Properties | Select-Object -First 1).Name
}

function Invoke-Oracle($winget, $rows, $allow) {
    $wg = @($winget | Where-Object { $_.Source -eq 'winget' })
    $matchedRows = @($rows | Where-Object { Get-MatchedWinget $_ })
    $wgIds = @{}
    foreach ($p in $wg) { $wgIds[$p.Id.ToLowerInvariant()] = $p }
    $byId = @{}
    foreach ($r in $matchedRows) { $byId[(Get-MatchedWinget $r).ToLowerInvariant()] = $r }

    $hit = @(); $diff = @(); $miss = @()
    foreach ($p in $wg) {
        $r = $byId[$p.Id.ToLowerInvariant()]
        if ($r) {
            $hit += $p
            if ((Get-Norm $r.installed_version) -ne (Get-Norm $p.InstalledVersion)) {
                $diff += "$($p.Id): winget=$($p.InstalledVersion) wingetrs=$($r.installed_version)"
            }
        } else {
            $same = @($rows | Where-Object { $_.name -eq $p.Name -or $_.name.StartsWith($p.Name) }) | Select-Object -First 1
            $state = if ($same) { Get-RowState $same } else { 'ausente' }
            $miss += [pscustomobject]@{ id = $p.Id; name = $p.Name; estado = $state }
        }
    }
    $okPrec = @($matchedRows | Where-Object { $wgIds.ContainsKey((Get-MatchedWinget $_).ToLowerInvariant()) })
    $allowIds = @($allow | ForEach-Object { $_.id.ToLowerInvariant() })
    $missNet = @($miss | Where-Object { $allowIds -notcontains $_.id.ToLowerInvariant() })
    $accepted = $miss.Count - $missNet.Count
    $total = $wg.Count
    [pscustomobject]@{
        total_winget = $total
        casados = $hit.Count
        recall_bruto = if ($total) { $hit.Count / $total } else { 1.0 }
        recall_liquido = if ($total - $accepted) { $hit.Count / ($total - $accepted) } else { 1.0 }
        aceitas = $accepted
        versao_igual = $hit.Count - $diff.Count
        concordancia = if ($hit.Count) { ($hit.Count - $diff.Count) / $hit.Count } else { 1.0 }
        versoes_diferentes = $diff
        matched_winget_wingetrs = $matchedRows.Count
        precisao_ok = $okPrec.Count
        precisao = if ($matchedRows.Count) { $okPrec.Count / $matchedRows.Count } else { 1.0 }
        faltas = $miss
    }
}

function Read-Json([string]$path) { Get-Content -Raw -LiteralPath $path | ConvertFrom-Json -Depth 20 }

if ($SelfTest) {
    $fx = Join-Path $PSScriptRoot '../tests/fixtures/oracle_list'
    $res = Invoke-Oracle (Read-Json "$fx/winget.json") (Read-Json "$fx/wingetrs.json") (Read-Json "$fx/allow.json")
    # 6 winget ids; matched A,B (E swapped to Z.Other does not count); 1 of 2 same version;
    # wingetrs Matched winget rows: A, B, Z.Other => 2/3 listed by winget; F accepted.
    $expect = [ordered]@{ total_winget = 6; casados = 2; versao_igual = 1; matched_winget_wingetrs = 3; precisao_ok = 2; aceitas = 1 }
    $fail = @()
    foreach ($k in $expect.Keys) { if ($res.$k -ne $expect[$k]) { $fail += "$k esperado $($expect[$k]) obtido $($res.$k)" } }
    if ([math]::Abs($res.precisao - 2 / 3) -gt 1e-9) { $fail += "precisao esperada 2/3 obtida $($res.precisao)" }
    if ([math]::Abs($res.recall_liquido - 2 / 5) -gt 1e-9) { $fail += "recall_liquido esperado 2/5 obtido $($res.recall_liquido)" }
    $states = @($res.faltas | ForEach-Object { "$($_.id)=$($_.estado)" }) | Sort-Object
    $wantStates = @('C.Missing=ausente', 'D.Amb=Ambiguous', 'E.Swapped=Matched', 'F.Allowed=Unmatched') | Sort-Object
    if (($states -join ';') -ne ($wantStates -join ';')) { $fail += "faltas: $($states -join ';')" }
    if ($fail) { $fail | ForEach-Object { Write-Host "[FAIL] $_" }; exit 1 }
    Write-Host '[OK]   oracle_list selftest'
    exit 0
}

if (-not $WingetJson) {
    if (-not (Get-Module -ListAvailable Microsoft.WinGet.Client)) {
        Write-Error 'Modulo Microsoft.WinGet.Client ausente; instale-o (Install-Module Microsoft.WinGet.Client).'
    }
    Import-Module Microsoft.WinGet.Client
    $winget = Get-WinGetPackage | Select-Object Id, Name, InstalledVersion, Source
} else { $winget = Read-Json $WingetJson }
if ($WingetrsJson) { $rows = Read-Json $WingetrsJson }
else { $rows = (& $Exe list --json | Out-String | ConvertFrom-Json -Depth 20) }
$allow = if (Test-Path -LiteralPath $AllowFile) { Read-Json $AllowFile } else { @() }

$r = Invoke-Oracle $winget $rows $allow
$pct = { param($x) '{0:P1}' -f $x }
Write-Host "Oraculo de list: $($r.total_winget) pacotes do winget, $(@($rows).Count) linhas do wingetrs"
Write-Host "recall (bruto)        : $($r.casados)/$($r.total_winget) = $(& $pct $r.recall_bruto)"
Write-Host "recall (liquido)      : $($r.casados)/$($r.total_winget - $r.aceitas) = $(& $pct $r.recall_liquido) ($($r.aceitas) aceitas)"
Write-Host "concordancia de versao: $($r.versao_igual)/$($r.casados) = $(& $pct $r.concordancia)"
Write-Host "precisao              : $($r.precisao_ok)/$($r.matched_winget_wingetrs) = $(& $pct $r.precisao)"
$r.versoes_diferentes | Select-Object -First 6 | ForEach-Object { Write-Host "  versao: $_" }
$r.faltas | Select-Object -First 10 | ForEach-Object { Write-Host "  falta: $($_.id) [$($_.name)] -> $($_.estado)" }
if ($OutJson) { $r | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutJson -Encoding utf8 }
