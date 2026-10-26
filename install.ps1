<#
.SYNOPSIS
    Installs (or updates, or removes) wingetrs, for the current user or for all users.

.DESCRIPTION
    Downloads wingetrs.exe from the latest GitHub release, checks it against the published
    wingetrs.exe.sha256 and copies it to the install folder, which is then added to the PATH:
      - user scope:    %LOCALAPPDATA%\Programs\wingetrs, user PATH (HKCU\Environment);
      - machine scope: %ProgramFiles%\wingetrs, machine PATH (HKLM). Only this one lets
        wingetrs start its elevated helper (it refuses to run from a folder the user can write).
    An elevated terminal installs for all users. A normal one asks which scope; all users
    relaunches the script elevated (one UAC prompt). -Scope skips the question.
    Running it again updates to the latest release. Works in Windows PowerShell 5.1 and 7.

    irm https://raw.githubusercontent.com/jonyduque/wingetrs/main/install.ps1 | iex

    With parameters:
    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/jonyduque/wingetrs/main/install.ps1))) -Uninstall

.PARAMETER Version
    Release tag to install (e.g. v0.2.1). Default: the latest release.

.PARAMETER Dir
    Install folder. Default: depends on the scope (see above).

.PARAMETER Scope
    User or Machine. Default: Machine in an elevated terminal, otherwise ask (User when there
    is no console to ask on).

.PARAMETER Uninstall
    Removes wingetrs.exe and its PATH entry for the chosen scope. Data in
    %LOCALAPPDATA%\wingetrs (catalog, apps installed by wingetrs) is kept.

.PARAMETER DryRun
    Shows what would be done (download, folder, PATH, UAC) without changing anything, then
    asks whether to run it for real.

.PARAMETER Relaunched
    Internal: set on the elevated copy this script starts, which waits for Enter at the end.
#>
# No positional parameters: `.\install.ps1 --dry-run` used to land in -Version and fail with a
# 404 on a release named "--dry-run"; now an unknown word is an error before anything runs.
[CmdletBinding(PositionalBinding = $false)]
param(
    [string]$Version = '',
    [string]$Dir = '',
    [ValidateSet('', 'User', 'Machine')]
    [string]$Scope = '',
    [switch]$Uninstall,
    [Alias('dry-run')]
    [switch]$DryRun,
    [switch]$Relaunched
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repo = 'jonyduque/wingetrs'
# A release tag (v0.3.0, v0.4.0-rc1), checked before any question or download.
if ($Version -and $Version -notmatch '^v?\d+\.\d+\.\d+([-.][0-9A-Za-z.]+)?$') {
    Write-Host "x versão inválida: '$Version' (use uma tag de release, ex.: v0.3.0)" -ForegroundColor Red
    return
}
$scriptUrl = "https://raw.githubusercontent.com/$repo/main/install.ps1"

# Estilos ANSI. Montados com o caractere ESC: `$PSStyle` e o escape `` `e `` só existem no
# PowerShell 7, e o script também roda no Windows PowerShell 5.1.
$esc = [char]27
function Get-Rgb([int]$R, [int]$G, [int]$B) { "$esc[38;2;$R;$G;${B}m" }
$cPurple = Get-Rgb 154 92 219
$cPeach  = Get-Rgb 255 117 117
$cGray   = Get-Rgb 128 128 128
$cPink   = Get-Rgb 224 91 191
$cDim    = "$esc[2m"
$cBold   = "$esc[1m"
$cReset  = "$esc[0m"

# UTF-8 para as setas e o "·"; o valor anterior volta no fim (Restore-Console), porque sob
# `irm | iex` a mudança ficaria valendo no terminal de quem rodou.
$prevOutputEncoding = [Console]::OutputEncoding
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
function Restore-Console { try { [Console]::OutputEncoding = $prevOutputEncoding } catch { } }

function Write-Title([string]$Text) {
    Write-Host "$cBold$cPink`n$Text$cReset`n"
}
# One step line: the text in the default color, the value (a path, a hash) highlighted.
function Write-Step([string]$Text, [string]$Value = '') {
    Write-Host "  - " -NoNewline
    if ($Value) {
        Write-Host "$cGray$Text$cReset " -NoNewline
        Write-Host "$cPeach$Value$cReset"
    } else {
        Write-Host $Text
    }
}
function Write-Ok([string]$Text) { Write-Host "  ok $Text" -ForegroundColor Green }
function Write-Note([string]$Text) { Write-Host "  $cGray$Text$cReset" }
function Write-Warn([string]$Text) { Write-Host "  ! $Text" -ForegroundColor Yellow }

function Test-Elevated {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    (New-Object Security.Principal.WindowsPrincipal $id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# Key=Hotkey; Value=Binding
function Get-Footer([System.Collections.IDictionary]$keybinds){
    $divisor = " · "
    $sKey = "$esc[1m$(Get-Rgb 150 150 150)"
    $sBind = "$esc[22m$(Get-Rgb 110 110 110)"
    $reset = $cReset

    $keys =  @($keybinds.Keys)
    $total = $keys.Count
    $parts = [System.Collections.Generic.List[string]]::new()

    for ($i=0;$i -lt $Total; $i++){
        $key  = $keys[$i].ToString().Trim()
        $bind = $keybinds[$keys[$i]].ToString().Trim()
        $parts.Add("$sKey$key $sBind$bind")
    }

    $footer = "  " + ($parts -join $divisor) + $reset
    Write-Host $footer
}

# Arrow-key choice in the TUI style: up/down (or left/right, Tab) move, Enter picks, Esc
# cancels ($null). Returns $Default when there is no console to read keys from.
function Read-Choice([string]$Question, [System.Collections.IDictionary]$Options, [int]$Default = 0) {
    try { if ([Console]::IsInputRedirected) { return $Default } } catch { return $Default }
    if ($Options.Count -eq 0) { return $null }

    Write-Title $Question

    # [Math]::Clamp não existe no .NET Framework do 5.1.
    $sel = [Math]::Max(0, [Math]::Min($Default, $Options.Count - 1))
    $keyOptions = @($Options.Keys)
    $keybinds = [ordered]@{
        # Arrows as [char]: a 5.1 reading this file without a BOM decodes it as ANSI, and the
        # UTF-8 bytes of these symbols contain a smart quote that ends the string.
        "$([char]0x2193)$([char]0x2191)" = "movem"
        "Enter" = "escolhe"
        "Esc"   = "cancela"
    }

    # The first draw writes where the cursor is; its top line is only known after it, because
    # writing at the bottom of the window scrolls the buffer (a top read before the draw goes
    # stale and SetCursorPosition then fails with "Actual value was 43").
    $startTop = -1
    $cursorVisibleOriginal = [Console]::CursorVisible

    $draw = {
        if ($startTop -ge 0) { [Console]::SetCursorPosition(0, $startTop) }

        for ($i = 0; $i -lt $Options.Count; $i++) {
            $k = $keyOptions[$i]
            $v = $Options[$k]

            if ($i -eq $sel) {
                # Colunas 1 e 2 recebem roxo + negrito; a última coluna recebe roxo + DimOn
                $colArrow = "$cPurple$cBold{0,-2}$cReset" -f " $([char]0x2192)"
                $colKey   = "$cPurple$cBold{0,-25}$cReset" -f $k
                $colDesc  = "$cPurple$cDim{0,-30}$cReset" -f $v

                Write-Host "  $colArrow $colKey $colDesc"
            } else {
                $colArrow = "$cGray{0,-2}$cReset" -f "  "
                $colKey   = "{0,-25}" -f $k
                $colDesc  = "$cDim{0,-30}$cReset" -f $v

                Write-Host "  $colArrow $colKey $colDesc"
            }
        }

        Get-Footer $keybinds
    }

    try {
        [Console]::CursorVisible = $false
        & $draw
        $startTop = [Math]::Max(0, [Console]::CursorTop - $Options.Count - 1)

        while ($true) {
            $k = [Console]::ReadKey($true)
            # Each draw ends below the footer, so Enter and Esc leave the cursor there as is.
            switch ($k.Key) {
                { $_ -in 'UpArrow', 'LeftArrow' }           { $sel = ($sel + $Options.Count - 1) % $Options.Count }
                { $_ -in 'DownArrow', 'RightArrow', 'Tab' } { $sel = ($sel + 1) % $Options.Count }
                'Enter'  { return $sel }
                'Escape' { return $null }
            }
            & $draw
        }
    }
    finally {
        [Console]::CursorVisible = $cursorVisibleOriginal
    }
}

$elevated = Test-Elevated
# An elevated terminal defaults to all users; a normal one asks, and all users then relaunches
# the script elevated (Invoke-Relaunch). The user scope in an elevated terminal is allowed: it
# is the same user's folder and HKCU.
$cancelled = $false
if ($Scope) {
    $machine = $Scope -eq 'Machine'
} elseif ($elevated) {
    $machine = $true
} else {
    $verb = if ($Uninstall) { 'Remover' } else { 'Instalar' }
    # [ordered]: a posição decide o escopo ($pick -eq 1 = todos os usuários).
    $pick = Read-Choice "$verb o wingetrs para quem?" ([ordered]@{
        'Só para você' = 'Sem administrador'
        'Para todos os usuários' = 'Pede autorização de administrador.'
    })
    $cancelled = $null -eq $pick
    $machine = $pick -eq 1
}
$dirGiven = [bool]$Dir
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

function Quote-Arg([string]$Value) {
    if ($Value -match '[\s;]') { '"' + $Value + '"' } else { $Value }
}

# Runs this script again elevated for the machine scope and waits for it. From a file the copy
# runs that same file; under `irm | iex` there is no file, so it downloads the script again.
function Invoke-Relaunch {
    if ($Version -and $Version -notmatch '^[\w.\-]+$') { Fail "versão inválida: $Version" }
    if ($Dir.Contains('"') -or $Dir.Contains("'")) { Fail "pasta inválida: $Dir" }
    $hostExe = (Get-Process -Id $PID).Path
    $extra = @('-Scope', 'Machine', '-Relaunched')
    if ($Version) { $extra += @('-Version', $Version) }
    if ($dirGiven) { $extra += @('-Dir', $Dir) }
    if ($Uninstall) { $extra += '-Uninstall' }
    if ($PSCommandPath) {
        $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $PSCommandPath) + $extra
    } else {
        $tail = ($extra | ForEach-Object { if ($_ -like '-*') { $_ } else { "'$_'" } }) -join ' '
        $cmd = "[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12; " +
            "& ([scriptblock]::Create((Invoke-RestMethod -UseBasicParsing '$scriptUrl'))) $tail"
        $enc = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($cmd))
        $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', $enc)
    }
    Write-Note 'Aguardando autorização para executar como administrador'
    try {
        $p = Start-Process -FilePath $hostExe -ArgumentList (($argv | ForEach-Object { Quote-Arg $_ }) -join ' ') -Verb RunAs -Wait -PassThru
    } catch {
        Fail 'autorização de administrador recusada; nada foi alterado'
    }
    if ($p.ExitCode -ne 0) { Fail "a janela de administrador terminou com erro (código $($p.ExitCode))" }
    $exe = Join-Path $Dir 'wingetrs.exe'
    if ($Uninstall) {
        Write-Ok "removido para todos os usuários: $exe"
        return
    }
    if (@(Split-PathList $env:Path | Where-Object { Same-Dir $_ $Dir }).Count -eq 0) { $env:Path = "$env:Path;$Dir" }
    $installed = & $exe --version
    Write-Host ''
    Write-Host "Pronto: $installed (todos os usuários)" -ForegroundColor Green
    Write-Note 'Terminais abertos antes da instalação precisam ser reabertos para achar o wingetrs'
}

function Invoke-Main {
    if ($cancelled) {
        Write-Note 'Cancelado. Nada foi alterado.'
        return
    }
    if ($machine -and -not $elevated) {
        Invoke-Relaunch
        return
    }
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
    Write-Note 'Terminais abertos antes da instalação precisam ser reabertos para achar o wingetrs'
    if (-not $machine) {
        Write-Note 'Operações de administrador (ponte elevada, remoções em HKLM) pedem a instalação para todos os usuários: rode de novo e escolha "Para todos os usuários"'
    }
}

# -DryRun: the same decisions as Invoke-Main, printed instead of done (no download, no write).
# Returns $true when the user then asks for the real run.
function Invoke-DryRun {
    if ($cancelled) {
        Write-Note 'Cancelado. Nada foi alterado.'
        return $false
    }
    $exe = Join-Path $Dir 'wingetrs.exe'
    $label = if ($Version) { $Version } else { 'último release' }
    $action = if ($Uninstall) { 'remover' } else { 'instalar' }
    Write-Title "Simulação: $action o wingetrs ($(if ($Uninstall) { $scopeText } else { "$label, $scopeText" }))"
    if ($machine -and -not $elevated) {
        Write-Step 'pediria autorização de administrador (UAC) e continuaria numa janela elevada'
    }
    $hasDir = @(Split-PathList (Get-EnvPath) | Where-Object { Same-Dir $_ $Dir }).Count -gt 0
    if ($Uninstall) {
        if ($hasDir) { Write-Step "tiraria do $pathText" $Dir } else { Write-Step "$pathText não tem a pasta" }
        if (Test-Path -LiteralPath $exe) { Write-Step 'apagaria' $exe } else { Write-Step 'não há wingetrs em' $Dir }
        Write-Step 'apagaria a pasta se ficar vazia' $Dir
        Write-Note "dados em $(Join-Path $env:LOCALAPPDATA 'wingetrs') ficariam"
    } else {
        $base = if ($Version) { "https://github.com/$repo/releases/download/$Version" } else { "https://github.com/$repo/releases/latest/download" }
        Write-Step 'baixaria' "$base/wingetrs.exe"
        Write-Step 'conferiria o SHA-256 com' "$base/wingetrs.exe.sha256"
        if (Test-Path -LiteralPath $exe) {
            $current = try { & $exe --version 2>$null } catch { 'versão desconhecida' }
            Write-Step "substituiria $current em" $exe
        } else {
            Write-Step 'salvaria em' $exe
        }
        if ($hasDir) { Write-Step "$pathText já tem a pasta" } else { Write-Step "poria no $pathText" $Dir }
        $userCopy = Join-Path $env:LOCALAPPDATA 'Programs\wingetrs\wingetrs.exe'
        if ($machine -and -not (Same-Dir (Split-Path $userCopy) $Dir) -and (Test-Path -LiteralPath $userCopy)) {
            Write-Warn "há também uma instalação só para você em $userCopy; a da máquina viria antes no PATH"
        }
    }
    Write-Host ''
    Write-Note 'Simulação: nada foi alterado.'
    $verbReal = if ($Uninstall) { 'remoção' } else { 'instalação' }
    # Default "Não": without a console to ask on, a dry run never turns into a real one.
    $go = Read-Choice "Iniciar a $verbReal real?" ([ordered]@{
        'Sim' = "faz a $verbReal agora"
        'Não' = 'sai sem alterar nada'
    }) -Default 1
    return ($go -eq 0)
}

$failed = $false
try {
    if ($DryRun) {
        if (Invoke-DryRun) { Invoke-Main }
    } else {
        Invoke-Main
    }
} catch { $failed = $true; Write-Host "x $($_.Exception.Message)" -ForegroundColor Red } finally { Restore-Console }
if ($Relaunched) {
    # The elevated copy runs in its own window: keep it open until the user has read it.
    Read-Host 'Enter fecha esta janela' | Out-Null
    if ($failed) { exit 1 }
}
