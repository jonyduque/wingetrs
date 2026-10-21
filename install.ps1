<#
.SYNOPSIS
    Installs (or updates, or removes) wingetrs. The scope follows the terminal: a normal one
    installs for the current user, an elevated one ("Run as administrator") for all users.

.DESCRIPTION
    Downloads wingetrs.exe from the latest GitHub release, checks it against the published
    wingetrs.exe.sha256 and copies it to the install folder, which is then added to the PATH:
      - normal terminal:   %LOCALAPPDATA%\Programs\wingetrs, user PATH (HKCU\Environment);
      - elevated terminal: %ProgramFiles%\wingetrs, machine PATH (HKLM). Only this one lets
        wingetrs start its elevated helper (it refuses to run from a folder the user can write).
    Running it again updates to the latest release. Works in Windows PowerShell 5.1 and 7.

    irm https://raw.githubusercontent.com/jonyduque/wingetrs/main/install.ps1 | iex

    With parameters:
    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/jonyduque/wingetrs/main/install.ps1))) -Uninstall

.PARAMETER Version
    Release tag to install (e.g. v0.2.1). Default: the latest release.

.PARAMETER Dir
    Install folder. Default: depends on the scope (see above).

.PARAMETER Uninstall
    Removes wingetrs.exe and its PATH entry for the scope of this terminal. Data in
    %LOCALAPPDATA%\wingetrs (catalog, apps installed by wingetrs) is kept.
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

function Test-Elevated {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    (New-Object Security.Principal.WindowsPrincipal $id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

$machine = Test-Elevated
if (-not $Dir) {
    if ($machine) {
        # ProgramW6432 is the 64-bit Program Files even from a 32-bit PowerShell.
        $pf = if ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
        $Dir = Join-Path $pf 'wingetrs'
    } else {
        $Dir = Join-Path $env:LOCALAPPDATA 'Programs\wingetrs'
    }
}
# Tests point this at a scratch key under HKCU instead of the real Environment key (either scope).
$testKey = $env:WINGETRS_INSTALL_ENV_KEY
$envHive = if ($machine -and -not $testKey) { [Microsoft.Win32.Registry]::LocalMachine } else { [Microsoft.Win32.Registry]::CurrentUser }
$envKey = if ($testKey) { $testKey } elseif ($machine) { 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment' } else { 'Environment' }
$scopeText = if ($machine) { 'todos os usuários' } else { 'só para você' }
$pathText = if ($machine) { 'PATH da máquina' } else { 'PATH do usuário' }

function Write-Title([string]$Text) { Write-Host $Text -ForegroundColor Cyan }
# One step line: the text in the default color, the value (a path, a hash) highlighted.
function Write-Step([string]$Text, [string]$Value = '') {
    Write-Host '  - ' -ForegroundColor DarkGray -NoNewline
    if ($Value) {
        Write-Host "$Text " -NoNewline
        Write-Host $Value -ForegroundColor Cyan
    } else {
        Write-Host $Text
    }
}
function Write-Ok([string]$Text) { Write-Host "  ok $Text" -ForegroundColor Green }
function Write-Note([string]$Text) { Write-Host "  $Text" -ForegroundColor DarkGray }
function Write-Warn([string]$Text) { Write-Host "  ! $Text" -ForegroundColor Yellow }
# Errors end the run with one red line; `exit` would close the terminal under `irm | iex`.
function Fail([string]$Text) { throw $Text }

# The PATH is read and written raw: expanding it would freeze %VARS% into fixed paths.
function Get-EnvPath {
    $key = $envHive.OpenSubKey($envKey)
    if (-not $key) { return '' }
    try { [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) }
    finally { $key.Close() }
}

function Set-EnvPath([string]$Value) {
    $key = $envHive.CreateSubKey($envKey)
    try {
        if ($Value) { $key.SetValue('Path', $Value, [Microsoft.Win32.RegistryValueKind]::ExpandString) }
        else { $key.DeleteValue('Path', $false) }
    } finally { $key.Close() }
    if (-not $testKey) {
        # Setting a variable through .NET broadcasts WM_SETTINGCHANGE, so new terminals opened
        # from Explorer see the new PATH; the dummy variable is removed right after.
        $target = if ($machine) { 'Machine' } else { 'User' }
        [Environment]::SetEnvironmentVariable('WINGETRS_INSTALL_PING', '1', $target)
        [Environment]::SetEnvironmentVariable('WINGETRS_INSTALL_PING', $null, $target)
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
    $exe = Join-Path $Dir 'wingetrs.exe'

    if ($Uninstall) {
        Write-Title "Removendo o wingetrs ($scopeText)"
        $kept = @(Split-PathList (Get-EnvPath) | Where-Object { -not (Same-Dir $_ $Dir) })
        Set-EnvPath ($kept -join ';')
        Write-Step "$pathText sem" $Dir
        foreach ($f in @($exe, "$exe.old")) {
            if (Test-Path -LiteralPath $f) { Remove-Item -LiteralPath $f -Force }
        }
        if ((Test-Path -LiteralPath $Dir) -and -not (Get-ChildItem -LiteralPath $Dir -Force)) {
            Remove-Item -LiteralPath $Dir -Force
        }
        Write-Ok "removido: $exe"
        Write-Note "dados mantidos em $(Join-Path $env:LOCALAPPDATA 'wingetrs')"
        return
    }

    if ($PSVersionTable.PSVersion.Major -lt 6) {
        # Windows PowerShell 5.1 may default to TLS 1.0, which GitHub refuses.
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    }

    $base = if ($Version) { "https://github.com/$repo/releases/download/$Version" } else { "https://github.com/$repo/releases/latest/download" }
    $label = if ($Version) { $Version } else { 'último release' }
    Write-Title "Instalando o wingetrs ($label, $scopeText)"

    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    # The download lands in the install folder itself: elevated, that is a folder only
    # administrators write, so nobody can swap the file between the check and the copy.
    $tmp = Join-Path $Dir ('wingetrs.exe.download-' + [Guid]::NewGuid().ToString('N'))
    try {
        Write-Step 'baixando' 'wingetrs.exe'
        Invoke-WebRequest -UseBasicParsing -Uri "$base/wingetrs.exe" -OutFile $tmp
        # Release assets come as application/octet-stream: Content is a byte[] there, not text.
        $sumRaw = (Invoke-WebRequest -UseBasicParsing -Uri "$base/wingetrs.exe.sha256").Content
        $sumText = if ($sumRaw -is [byte[]]) { [Text.Encoding]::UTF8.GetString($sumRaw) } else { [string]$sumRaw }
        $expected = ($sumText.Trim() -split '\s+')[0].ToUpperInvariant()
        if ($expected -notmatch '^[0-9A-F]{64}$') { Fail "wingetrs.exe.sha256 inválido: $sumText" }
        # .NET instead of Get-FileHash: a 5.1 started from PowerShell 7 inherits a PSModulePath
        # where Get-FileHash does not load.
        $stream = [IO.File]::OpenRead($tmp)
        try { $digest = [Security.Cryptography.SHA256]::Create().ComputeHash($stream) } finally { $stream.Dispose() }
        $actual = ([BitConverter]::ToString($digest) -replace '-', '').ToUpperInvariant()
        if ($actual -ne $expected) { Fail "SHA-256 não confere (esperado $expected, baixado $actual)" }
        Write-Ok 'SHA-256 conferido'

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
    Write-Step 'salvo em' $exe

    $entries = Split-PathList (Get-EnvPath)
    if (@($entries | Where-Object { Same-Dir $_ $Dir }).Count -eq 0) {
        Set-EnvPath ((@($entries) + $Dir) -join ';')
        Write-Step "$pathText +" $Dir
    } else {
        Write-Step "$pathText já tem a pasta"
    }
    if (@(Split-PathList $env:Path | Where-Object { Same-Dir $_ $Dir }).Count -eq 0) { $env:Path = "$env:Path;$Dir" }

    $userCopy = Join-Path $env:LOCALAPPDATA 'Programs\wingetrs\wingetrs.exe'
    if ($machine -and -not (Same-Dir (Split-Path $userCopy) $Dir) -and (Test-Path -LiteralPath $userCopy)) {
        Write-Warn "há também uma instalação só para você em $userCopy; a da máquina vem antes no PATH"
    }

    $installed = & $exe --version
    Write-Host ''
    Write-Host "Pronto: $installed" -ForegroundColor Green
    Write-Note 'terminais abertos antes da instalação precisam ser reabertos para achar o wingetrs'
    if (-not $machine) {
        Write-Note 'operações de administrador (ponte elevada, remoções em HKLM) pedem a instalação para todos os usuários: rode este comando num terminal de administrador'
    }
}

try { Invoke-Main } catch { Write-Host "x $($_.Exception.Message)" -ForegroundColor Red }
