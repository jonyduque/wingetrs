<#
.SYNOPSIS
    Installs (or updates, or removes) wingetrs for the current user, without elevation.

.DESCRIPTION
    Downloads wingetrs.exe from the latest GitHub release, checks it against the published
    wingetrs.exe.sha256, copies it to %LOCALAPPDATA%\Programs\wingetrs and adds that folder to
    the user PATH (HKCU\Environment). Running it again updates to the latest release.
    Works in Windows PowerShell 5.1 and PowerShell 7.

    irm https://raw.githubusercontent.com/jonyduque/wingetrs/main/install.ps1 | iex

    With parameters:
    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/jonyduque/wingetrs/main/install.ps1))) -Uninstall

.PARAMETER Version
    Release tag to install (e.g. v0.2.1). Default: the latest release.

.PARAMETER Dir
    Install folder. Default: %LOCALAPPDATA%\Programs\wingetrs.

.PARAMETER Uninstall
    Removes wingetrs.exe and its PATH entry. Data in %LOCALAPPDATA%\wingetrs (catalog, apps
    installed by wingetrs) is kept.
#>
[CmdletBinding()]
param(
    [string]$Version = '',
    [string]$Dir = '',
    [switch]$Uninstall
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repo = 'jonyduque/wingetrs'
if (-not $Dir) { $Dir = Join-Path $env:LOCALAPPDATA 'Programs\wingetrs' }
# Tests point this at a scratch key under HKCU instead of the real Environment key.
$envKey = if ($env:WINGETRS_INSTALL_ENV_KEY) { $env:WINGETRS_INSTALL_ENV_KEY } else { 'Environment' }

function Write-Step([string]$Text) { Write-Host "  $Text" }
# Errors end the run with one red line; `exit` would close the terminal under `irm | iex`.
function Fail([string]$Text) { throw $Text }

function Test-Elevated {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    (New-Object Security.Principal.WindowsPrincipal $id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# The user PATH is read and written raw: expanding it would freeze %VARS% into fixed paths.
function Get-UserPath {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($envKey)
    if (-not $key) { return '' }
    try { [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) }
    finally { $key.Close() }
}

function Set-UserPath([string]$Value) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($envKey)
    try {
        if ($Value) { $key.SetValue('Path', $Value, [Microsoft.Win32.RegistryValueKind]::ExpandString) }
        else { $key.DeleteValue('Path', $false) }
    } finally { $key.Close() }
    if ($envKey -eq 'Environment') {
        # Setting a user variable through .NET broadcasts WM_SETTINGCHANGE, so new terminals
        # opened from Explorer see the new PATH; the dummy variable is removed right after.
        [Environment]::SetEnvironmentVariable('WINGETRS_INSTALL_PING', '1', 'User')
        [Environment]::SetEnvironmentVariable('WINGETRS_INSTALL_PING', $null, 'User')
    }
}

function Split-PathList([string]$Value) {
    @($Value -split ';' | Where-Object { $_ -ne '' })
}

function Same-Dir([string]$A, [string]$B) {
    $x = [Environment]::ExpandEnvironmentVariables($A).TrimEnd('\')
    $y = [Environment]::ExpandEnvironmentVariables($B).TrimEnd('\')
    [string]::Equals($x, $y, [StringComparison]::OrdinalIgnoreCase)
}

function Invoke-Main {
    if (Test-Elevated) {
        Fail 'rode num PowerShell sem "Executar como administrador": o wingetrs instala só para o seu usuário'
    }

    $exe = Join-Path $Dir 'wingetrs.exe'

    if ($Uninstall) {
        Write-Host 'Removendo o wingetrs'
        $kept = @(Split-PathList (Get-UserPath) | Where-Object { -not (Same-Dir $_ $Dir) })
        Set-UserPath ($kept -join ';')
        Write-Step "PATH do usuário sem $Dir"
        foreach ($f in @($exe, "$exe.old")) {
            if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f -Force }
        }
        if ((Test-Path -LiteralPath $Dir) -and -not (Get-ChildItem -LiteralPath $Dir -Force)) {
            Remove-Item -LiteralPath $Dir -Force
        }
        Write-Step "removido: $exe"
        Write-Step "dados mantidos em $(Join-Path $env:LOCALAPPDATA 'wingetrs')"
        return
    }

    if ($PSVersionTable.PSVersion.Major -lt 6) {
        # Windows PowerShell 5.1 may default to TLS 1.0, which GitHub refuses.
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    }

    $base = if ($Version) { "https://github.com/$repo/releases/download/$Version" } else { "https://github.com/$repo/releases/latest/download" }
    $label = if ($Version) { $Version } else { 'último release' }
    Write-Host "Instalando o wingetrs ($label)"

    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    $tmp = Join-Path $Dir ('wingetrs.exe.download-' + [Guid]::NewGuid().ToString('N'))
    try {
        Write-Step 'baixando wingetrs.exe'
        Invoke-WebRequest -UseBasicParsing -Uri "$base/wingetrs.exe" -OutFile $tmp
        $sumText = [string](Invoke-WebRequest -UseBasicParsing -Uri "$base/wingetrs.exe.sha256").Content
        $expected = ($sumText.Trim() -split '\s+')[0].ToUpperInvariant()
        if ($expected -notmatch '^[0-9A-F]{64}$') { Fail "wingetrs.exe.sha256 inválido: $sumText" }
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $tmp).Hash.ToUpperInvariant()
        if ($actual -ne $expected) { Fail "SHA-256 não confere (esperado $expected, baixado $actual)" }
        Write-Step "SHA-256 conferido: $actual"

        # A running wingetrs.exe cannot be overwritten but can be renamed out of the way.
        if (Test-Path -LiteralPath $exe) {
            if (Test-Path -LiteralPath "$exe.old") { Remove-Item -LiteralPath "$exe.old" -Force -ErrorAction SilentlyContinue }
            Move-Item -LiteralPath $exe -Destination "$exe.old" -Force
        }
        Move-Item -LiteralPath $tmp -Destination $exe
        if (Test-Path -LiteralPath "$exe.old") { Remove-Item -LiteralPath "$exe.old" -Force -ErrorAction SilentlyContinue }
    } finally {
        if (Test-Path -LiteralPath $tmp) { Remove-Item -LiteralPath $tmp -Force }
    }
    Write-Step "salvo em $exe"

    $entries = Split-PathList (Get-UserPath)
    if (@($entries | Where-Object { Same-Dir $_ $Dir }).Count -eq 0) {
        Set-UserPath ((@($entries) + $Dir) -join ';')
        Write-Step "PATH do usuário: + $Dir"
    } else {
        Write-Step 'PATH do usuário já tem a pasta'
    }
    if (@(Split-PathList $env:Path | Where-Object { Same-Dir $_ $Dir }).Count -eq 0) { $env:Path = "$env:Path;$Dir" }

    $installed = & $exe --version
    Write-Host "Pronto: $installed"
    Write-Step 'terminais abertos antes da instalação precisam ser reabertos para achar o wingetrs'
}

try { Invoke-Main } catch { Write-Host "x $($_.Exception.Message)" -ForegroundColor Red }
