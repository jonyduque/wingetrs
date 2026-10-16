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

function Test-NameCompat([string]$a, [string]$b) {
    if ($a -eq "" -or $b -eq "") { return $false }
    $a = $a.ToLowerInvariant(); $b = $b.ToLowerInvariant()
    $a -eq $b -or $a.StartsWith($b) -or $b.StartsWith($a)
}

# 'formato' = same version written differently; 'real' = different version.
function Get-VersionClass([string]$w, [string]$r) {
    $w = (Get-Norm $w) -replace '^[<>~=]+', ''
    $r = (Get-Norm $r) -replace '^[<>~=]+', ''
    if ($w -eq $r) { return 'formato' }
    $blank = { param($v) $v -eq '' -or $v -eq 'Unknown' }
    if ((& $blank $w) -and (& $blank $r)) { return 'formato' }
    if ($w -ne '' -and $r -ne '' -and ($r.StartsWith("$w.") -or $w.StartsWith("$r."))) { return 'formato' }
    'real'
}

function Invoke-Oracle($winget, $rows, $allow) {
    $wgAll = @($winget | Where-Object { $_.Source -eq 'winget' })
    $wgDup = @($wgAll | Group-Object { $_.Id.ToLowerInvariant() } | Where-Object Count -gt 1 | ForEach-Object { $_.Group[0].Id })
    $wg = @($wgAll | Group-Object { $_.Id.ToLowerInvariant() } | ForEach-Object { $_.Group[0] })
    $matchedRows = @($rows | Where-Object { Get-MatchedWinget $_ })
    $rowDup = @($matchedRows | Group-Object { (Get-MatchedWinget $_).ToLowerInvariant() } | Where-Object Count -gt 1 | ForEach-Object { Get-MatchedWinget $_.Group[0] })
    $wgIds = @{}
    foreach ($p in $wg) { $wgIds[$p.Id.ToLowerInvariant()] = $p }
    $byId = @{}
    foreach ($r in $matchedRows) { $byId[(Get-MatchedWinget $r).ToLowerInvariant()] = $r }

    $hit = @(); $diff = @(); $miss = @(); $renamed = @()
    $fmt = 0; $real = 0
    foreach ($p in $wg) {
        $r = $byId[$p.Id.ToLowerInvariant()]
        if ($r) {
            $hit += $p
            if (-not (Test-NameCompat ([string]$r.name) ([string]$p.Name))) {
                $renamed += "$($p.Id): winget='$($p.Name)' wingetrs='$($r.name)'"
            }
            if ((Get-Norm $r.installed_version) -ne (Get-Norm $p.InstalledVersion)) {
                $diff += "$($p.Id): winget=$($p.InstalledVersion) wingetrs=$($r.installed_version)"
                if ((Get-VersionClass $p.InstalledVersion $r.installed_version) -eq 'formato') { $fmt++ } else { $real++ }
            }
        } else {
            $same = @($rows | Where-Object { Test-NameCompat ([string]$_.name) ([string]$p.Name) }) | Select-Object -First 1
            $state = if ($same) { Get-RowState $same } else { 'ausente' }
            $miss += [pscustomobject]@{ id = $p.Id; name = $p.Name; estado = $state }
        }
    }
    $distinct = @($matchedRows | ForEach-Object { (Get-MatchedWinget $_).ToLowerInvariant() } | Select-Object -Unique)
    $okPrec = @($distinct | Where-Object { $wgIds.ContainsKey($_) })
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
        versao_formato = $fmt
        versao_real = $real
        nome_diferente = $renamed.Count
        nomes_diferentes = $renamed
        matched_winget_wingetrs = $distinct.Count
        precisao_ok = $okPrec.Count
        precisao = if ($distinct.Count) { $okPrec.Count / $distinct.Count } else { 1.0 }
        ids_duplicados_wingetrs = $rowDup
        ids_duplicados_winget = $wgDup
        faltas = $miss
    }
}

function Read-Json([string]$path) { Get-Content -Raw -LiteralPath $path | ConvertFrom-Json -Depth 20 }

if ($SelfTest) {
    $fx = Join-Path $PSScriptRoot '../tests/fixtures/oracle_list'
    $res = Invoke-Oracle (Read-Json "$fx/winget.json") (Read-Json "$fx/wingetrs.json") (Read-Json "$fx/allow.json")
    # 9 distinct winget ids (I.Dup listed twice); hits A,B,H.Bar,I.Dup (E swapped to Z.Other does not count);
    # H.Bar is hit through a row named "Foo App" => 1 name mismatch; 3 of 4 same version (B is a real difference);
    # wingetrs distinct Matched winget ids: a.same,b.diff,z.other,h.bar,i.dup => 4/5 listed by winget; F accepted.
    $expect = [ordered]@{ total_winget = 9; casados = 4; versao_igual = 3; versao_real = 1; versao_formato = 0
        nome_diferente = 1; matched_winget_wingetrs = 5; precisao_ok = 4; aceitas = 1 }
    $fail = @()
    foreach ($k in $expect.Keys) { if ($res.$k -ne $expect[$k]) { $fail += "$k esperado $($expect[$k]) obtido $($res.$k)" } }
    if ([math]::Abs($res.precisao - 4 / 5) -gt 1e-9) { $fail += "precisao esperada 4/5 obtida $($res.precisao)" }
    if ([math]::Abs($res.recall_liquido - 4 / 8) -gt 1e-9) { $fail += "recall_liquido esperado 4/8 obtido $($res.recall_liquido)" }
    if (($res.ids_duplicados_wingetrs -join ';') -ne 'I.Dup') { $fail += "duplicados wingetrs: $($res.ids_duplicados_wingetrs -join ';')" }
    if (($res.ids_duplicados_winget -join ';') -ne 'I.Dup') { $fail += "duplicados winget: $($res.ids_duplicados_winget -join ';')" }
    $states = @($res.faltas | ForEach-Object { "$($_.id)=$($_.estado)" }) | Sort-Object
    $wantStates = @('C.Missing=ausente', 'D.Amb=Ambiguous', 'E.Swapped=Matched', 'F.Allowed=Unmatched', 'G.Foo=Matched') | Sort-Object
    if (($states -join ';') -ne ($wantStates -join ';')) { $fail += "faltas: $($states -join ';')" }
    $vc = @{ a = Get-VersionClass '> 1.4.11' '1.4.11'; b = Get-VersionClass 'Unknown' ''; c = Get-VersionClass '8.0.31' '8.0.31.36421'; d = Get-VersionClass '2.0' '2.1' }
    if (($vc.a, $vc.b, $vc.c, $vc.d -join ',') -ne 'formato,formato,formato,real') { $fail += "classe de versao: $($vc.a),$($vc.b),$($vc.c),$($vc.d)" }
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
Write-Host "concordancia de versao: $($r.versao_igual)/$($r.casados) = $(& $pct $r.concordancia) (formato: $($r.versao_formato), real: $($r.versao_real))"
Write-Host "precisao              : $($r.precisao_ok)/$($r.matched_winget_wingetrs) ids distintos = $(& $pct $r.precisao)"
Write-Host "casado com nome diferente: $($r.nome_diferente)"
if ($r.ids_duplicados_wingetrs) { Write-Host "  ids duplicados no wingetrs: $($r.ids_duplicados_wingetrs -join ', ')" }
if ($r.ids_duplicados_winget) { Write-Host "  ids duplicados no winget: $($r.ids_duplicados_winget -join ', ')" }
$r.nomes_diferentes | Select-Object -First 6 | ForEach-Object { Write-Host "  nome: $_" }
$r.versoes_diferentes | Select-Object -First 6 | ForEach-Object { Write-Host "  versao: $_" }
$r.faltas | Select-Object -First 8 | ForEach-Object { Write-Host "  falta: $($_.id) [$($_.name)] -> $($_.estado)" }
if ($OutJson) { $r | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutJson -Encoding utf8 }
