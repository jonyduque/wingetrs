#Requires -Version 7.0
<#
.SYNOPSIS
    Read-only capture of the machine state the live proof compares ("antes"/"depois").

.DESCRIPTION
    Writes only inside -Out: ~/scoop listing, HKCU\Environment export, user/machine PATH,
    ARP entries (HKCU and HKLM, 64- and 32-bit views, read with Get-ItemProperty), MSIX
    packages, Start Menu "wingetrs Apps" shortcuts, `wingetrs list`, the wingetrs root
    (apps, shims, persist, modules), installers\run-* and Program Files staging, the
    HKCU\Software\7-Zip export (B4 backup) and scripts/leftovers.ps1.
    `wingetrs list` itself may append to its own log under %LOCALAPPDATA%\wingetrs\log.
    EXIT 0 = all areas captured, 1 = some area failed (listed in meta.json).

.EXAMPLE
    pwsh -File scripts/live/snapshot.ps1 -Name antes
    pwsh -File scripts/live/snapshot.ps1 -Name depois -Out "$env:TEMP\x\depois"
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[\w.-]+$')][string]$Name,
    [string]$Out,
    [string]$Exe = (Join-Path $PSScriptRoot '..\..\target\release\wingetrs.exe'),
    [switch]$Force
)
Set-StrictMode -Version Latest
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$OutputEncoding = [Text.Encoding]::UTF8
$PSNativeCommandUseErrorActionPreference = $false

if (-not $Out) { $Out = Join-Path $env:LOCALAPPDATA ("wingetrs-prova\{0}-{1}" -f (Get-Date -Format 'yyyyMMdd-HHmmss'), $Name) }
$Out = [IO.Path]::GetFullPath($Out)
if ((Test-Path -LiteralPath $Out) -and (Get-ChildItem -LiteralPath $Out -Force | Select-Object -First 1) -and -not $Force) {
    Write-Output "[x] $Out já tem arquivos; use outra pasta ou -Force"
    exit 1
}
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$errors = [ordered]@{}
$wdata = Join-Path $env:LOCALAPPDATA 'wingetrs'

function Save-Area([string]$Area, [scriptblock]$Body) {
    try {
        $msg = & $Body
        Write-Output ("[ok] {0,-10} {1}" -f $Area, $msg)
    } catch {
        $script:errors[$Area] = $_.Exception.Message
        Write-Output ("[x]  {0,-10} {1}" -f $Area, $_.Exception.Message)
    }
}

function Get-Prop($Obj, [string]$Name) {
    if ($null -eq $Obj) { return '' }
    $p = $Obj.PSObject.Properties[$Name]
    if ($p) { [string]$p.Value } else { '' }
}

function Export-RegKey([string]$Key, [string]$File) {
    $null = & reg.exe query $Key 2>&1
    if ($LASTEXITCODE -ne 0) { Set-Content -LiteralPath $File -Value '; chave ausente' -Encoding utf8; return 'chave ausente' }
    $null = & reg.exe export $Key $File /y 2>&1
    if ($LASTEXITCODE -ne 0) { throw "reg export $Key falhou ($LASTEXITCODE)" }
    'exportada'
}

Save-Area 'scoop' {
    $root = if ($env:SCOOP) { $env:SCOOP } else { Join-Path $env:USERPROFILE 'scoop' }
    $rows = @(Get-ChildItem -Recurse -Force -LiteralPath $root -ErrorAction SilentlyContinue | ForEach-Object {
            [pscustomobject]@{
                FullName = $_.FullName
                Length = if ($_.PSIsContainer) { '' } else { [string]$_.Length }
                LastWriteTimeUtc = $_.LastWriteTimeUtc.ToString('o')
            }
        })
    $rows | Export-Csv -LiteralPath (Join-Path $Out 'scoop.csv') -NoTypeInformation -Encoding utf8
    "$($rows.Count) itens em $root"
}

Save-Area 'env' { Export-RegKey 'HKCU\Environment' (Join-Path $Out 'env.reg') }

Save-Area 'path' {
    $u = (Get-Item 'HKCU:\Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    $m = (Get-Item 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    @($u -split ';') | Set-Content -LiteralPath (Join-Path $Out 'path-user.txt') -Encoding utf8
    @($m -split ';') | Set-Content -LiteralPath (Join-Path $Out 'path-machine.txt') -Encoding utf8
    "usuário $(@($u -split ';' | Where-Object { $_ }).Count) · máquina $(@($m -split ';' | Where-Object { $_ }).Count) entradas"
}

Save-Area 'arp' {
    $roots = [ordered]@{
        'HKCU' = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
        'HKCU32' = 'HKCU:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
        'HKLM64' = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall'
        'HKLM32' = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
    }
    $rows = foreach ($view in $roots.Keys) {
        foreach ($k in @(Get-ChildItem -LiteralPath $roots[$view] -ErrorAction SilentlyContinue)) {
            $p = Get-ItemProperty -LiteralPath $k.PSPath -ErrorAction SilentlyContinue
            [pscustomobject]@{
                Key = "$view\$($k.PSChildName)"
                DisplayName = Get-Prop $p 'DisplayName'
                DisplayVersion = Get-Prop $p 'DisplayVersion'
                Publisher = Get-Prop $p 'Publisher'
                UninstallString = Get-Prop $p 'UninstallString'
            }
        }
    }
    @($rows) | Export-Csv -LiteralPath (Join-Path $Out 'arp.csv') -NoTypeInformation -Encoding utf8
    "$(@($rows).Count) chaves"
}

Save-Area 'msix' {
    $rows = @(Get-AppxPackage -ErrorAction Stop | ForEach-Object { [pscustomobject]@{ PackageFullName = $_.PackageFullName } })
    $rows | Export-Csv -LiteralPath (Join-Path $Out 'msix.csv') -NoTypeInformation -Encoding utf8
    "$($rows.Count) pacotes"
}

Save-Area 'startmenu' {
    $dir = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\wingetrs Apps'
    $shell = New-Object -ComObject WScript.Shell
    $rows = @(Get-ChildItem -Recurse -Force -LiteralPath $dir -ErrorAction SilentlyContinue | ForEach-Object {
            # CreateShortcut on an existing .lnk only reads it (nothing is saved).
            $target = if ($_.Extension -eq '.lnk') { $shell.CreateShortcut($_.FullName).TargetPath } else { '' }
            [pscustomobject]@{ FullName = $_.FullName; Target = $target }
        })
    $rows | Export-Csv -LiteralPath (Join-Path $Out 'startmenu.csv') -NoTypeInformation -Encoding utf8
    "$($rows.Count) itens"
}

Save-Area 'list' {
    $f = Join-Path $Out 'list.txt'
    if (-not (Test-Path -LiteralPath $Exe)) { Set-Content -LiteralPath $f -Value "wingetrs ausente: $Exe" -Encoding utf8; throw "wingetrs ausente: $Exe" }
    $lines = @(& $Exe list --color never 2>&1 | ForEach-Object { [string]$_ -replace '\x1b\[[0-9;?]*[ -/]*[@-~]', '' })
    $code = $LASTEXITCODE
    $lines | Set-Content -LiteralPath $f -Encoding utf8
    if ($code -ne 0) { throw "wingetrs list saiu com $code" }
    "$($lines.Count) linhas"
}

Save-Area 'root' {
    $dirs = 'apps', 'shims', 'persist', 'modules' | ForEach-Object { Join-Path $wdata $_ }
    $rows = @(Get-ChildItem -Recurse -Force -LiteralPath $dirs -ErrorAction SilentlyContinue | ForEach-Object {
            [pscustomobject]@{ FullName = $_.FullName; Length = if ($_.PSIsContainer) { '' } else { [string]$_.Length } }
        })
    $rows | Export-Csv -LiteralPath (Join-Path $Out 'root.csv') -NoTypeInformation -Encoding utf8
    "$($rows.Count) itens"
}

Save-Area 'run' {
    $rows = @(Get-ChildItem -Force -LiteralPath (Join-Path $wdata 'installers') -Filter 'run-*' -ErrorAction SilentlyContinue |
            ForEach-Object { [pscustomobject]@{ FullName = $_.FullName } })
    $rows | Export-Csv -LiteralPath (Join-Path $Out 'run.csv') -NoTypeInformation -Encoding utf8
    "$($rows.Count) pastas run-*"
}

Save-Area 'staging' {
    $rows = @(Get-ChildItem -Force -LiteralPath 'C:\Program Files\wingetrs\staging' -ErrorAction SilentlyContinue |
            ForEach-Object { [pscustomobject]@{ FullName = $_.FullName } })
    $rows | Export-Csv -LiteralPath (Join-Path $Out 'staging.csv') -NoTypeInformation -Encoding utf8
    "$($rows.Count) itens"
}

Save-Area '7zip' { Export-RegKey 'HKCU\Software\7-Zip' (Join-Path $Out '7zip.reg') }

Save-Area 'leftovers' {
    $script = Join-Path $PSScriptRoot '..\leftovers.ps1'
    $lines = @(& (Get-Process -Id $PID).Path -NoProfile -File $script 2>&1 | ForEach-Object { [string]$_ })
    $code = $LASTEXITCODE
    ($lines + "exit $code") | Set-Content -LiteralPath (Join-Path $Out 'leftovers.txt') -Encoding utf8
    if ($code -eq 0) { 'nenhum processo' } else { "$($lines.Count - 2) processo(s) listado(s)" }
}

[ordered]@{
    name = $Name; time = (Get-Date).ToString('o'); computer = $env:COMPUTERNAME
    exe = $Exe; exeHash = if (Test-Path -LiteralPath $Exe) { (Get-FileHash -LiteralPath $Exe).Hash } else { $null }
    errors = $errors
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $Out 'meta.json') -Encoding utf8

Write-Output "Retrato '$Name' em $Out"
if ($errors.Count -gt 0) { exit 1 }
exit 0
