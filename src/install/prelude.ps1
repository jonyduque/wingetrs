# wingetrs prelude: Scoop functions used by manifest scripts (Task 152). ASCII only.
#
# Contract: the runner defines these variables BEFORE dot-sourcing this file
# (see CONTRACT_VARS in prelude.rs):
#   $__wr_root          wingetrs root (apps\, persist\, shims\, modules\ live under it)
#   $__wr_effects       effects collector
#   $__wr_env_seed      environment seed
#   $__wr_helpers       hashtable helper name -> absolute exe path
#   $__wr_shortcut_root shortcut root
# Optional: $bucketsdir (Find-BucketDirectory throws if it is empty).
# This file only defines functions: loading it has no side effects.

# messages -- core.ps1:311-314
function abort($msg, [int] $exit_code = 1) { throw "abort: $msg" }
function error($msg) { Write-Host "ERROR $msg" -f darkred }
function warn($msg) { Write-Host "WARN  $msg" -f darkyellow }
function info($msg) { Write-Host "INFO  $msg" -f darkgray }

# core.ps1:304 -- wingetrs refuses to run elevated
function is_admin { return $false }

# core.ps1:622
function ensure($dir) {
    if (!(Test-Path -Path $dir)) {
        New-Item -Path $dir -ItemType Directory | Out-Null
    }
    Convert-Path -Path $dir
}

# core.ps1:118 -- fixed table instead of the Scoop config file; unknown name -> $default
function get_config($name, $default) {
    switch ("$name".ToLowerInvariant()) {
        'no_junction' { return $false }
        'no_junctions' { return $false }
        'use_lessmsi' { return $true }
        'use_external_7zip' { return $false }
        default { return $default }
    }
}

# core.ps1:367-385 -- paths under $__wr_root; global is not supported
function appsdir($global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    "$__wr_root\apps"
}
function shimdir($global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    "$__wr_root\shims"
}
function modulesdir($global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    "$__wr_root\modules"
}
function appdir($app, $global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    "$__wr_root\apps\$app"
}
function versiondir($app, $version, $global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    "$__wr_root\apps\$app\$version"
}
function currentdir($app, $global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    "$__wr_root\apps\$app\current"
}
function persistdir($app, $global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    "$__wr_root\persist\$app"
}

# versions.ps1:31 -- current\ is a junction to the version dir
function Select-CurrentVersion {
    [OutputType([String])]
    [CmdletBinding()]
    param (
        [Parameter(Mandatory = $true, Position = 0, ValueFromPipeline = $true)]
        [Alias('App')]
        [String]
        $AppName,
        [Parameter(Position = 1)]
        [Switch]
        $Global
    )
    process {
        if ($Global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
        $appPath = "$__wr_root\apps\$AppName"
        $currentPath = "$appPath\current"
        $currentVersion = $null
        foreach ($mf in @("$currentPath\scoop-manifest.json", "$currentPath\manifest.json")) {
            if (Test-Path -LiteralPath $mf) {
                try { $currentVersion = (Get-Content -LiteralPath $mf -Raw | ConvertFrom-Json).version } catch { $currentVersion = $null }
                break
            }
        }
        if ($currentVersion -eq 'nightly' -or [string]::IsNullOrEmpty($currentVersion)) {
            $currentVersion = $null
            if (Test-Path -LiteralPath $currentPath) {
                $target = (Get-Item -LiteralPath $currentPath).Target
                if ($target) { $currentVersion = Split-Path -Leaf (@($target)[0]) }
            }
        }
        if ($null -eq $currentVersion -and (Test-Path -LiteralPath $appPath)) {
            $dirs = @(Get-ChildItem -LiteralPath $appPath -Directory | Where-Object { $_.Name -ne 'current' } | Sort-Object LastWriteTime)
            if ($dirs.Count -gt 0) { $currentVersion = $dirs[-1].Name }
        }
        return $currentVersion
    }
}

# core.ps1:407
function installed($app, [Nullable[bool]]$global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    $app = ($app -split '/|\\')[-1]
    return $null -ne (Select-CurrentVersion -AppName $app)
}

# buckets.ps1:3
function Find-BucketDirectory {
    param(
        [string] $Name = 'main',
        [switch] $Root
    )
    if (($null -eq $Name) -or ($Name -eq '')) {
        $Name = 'main'
    }
    if ([string]::IsNullOrEmpty($bucketsdir)) { throw 'bucketsdir not defined by the runner' }
    $bucket = "$bucketsdir\$Name"
    if ((Test-Path "$bucket\bucket") -and !$Root) {
        $bucket = "$bucket\bucket"
    }
    return $bucket
}

# core.ps1:476 -- helper paths come from $__wr_helpers
function Get-HelperPath {
    [CmdletBinding()]
    [OutputType([String])]
    param(
        [Parameter(Mandatory = $true, Position = 0, ValueFromPipeline = $true)]
        [ValidateSet('Git', '7zip', 'Lessmsi', 'Innounp', 'Dark', 'Aria2')]
        [String]
        $Helper
    )
    process {
        if ($null -eq $__wr_helpers) { return $null }
        if ($__wr_helpers.ContainsKey($Helper)) { return $__wr_helpers[$Helper] }
        return $null
    }
}

# core.ps1:519 (Get-ShimTarget core.ps1:911 inlined)
function Get-CommandPath {
    [CmdletBinding()]
    [OutputType([String])]
    param(
        [Parameter(Mandatory = $true, ValueFromPipeline = $true)]
        [String]
        $Command
    )
    begin {
        $userShims = shimdir $false
    }
    process {
        try {
            $comm = Get-Command $Command -ErrorAction Stop
        } catch {
            return $null
        }
        $commandPath = if ($comm.Path -like "$userShims*") {
            $shimPath = $comm.Path -replace '\.exe$', '.shim'
            if ($shimPath.EndsWith('.shim')) {
                $shimTarget = (Get-Content -Path $shimPath -ErrorAction SilentlyContinue | Select-Object -First 1)
                if ($shimTarget) { $shimTarget = $shimTarget.Replace('path = ', '').Replace('"', '') }
            } else {
                $shimTarget = ((Select-String -Path $shimPath -Pattern '^(?:@rem|#)\s*(.*)$').Matches.Groups | Select-Object -Index 1).Value
            }
            if (!$shimTarget -and (Test-Path -LiteralPath $shimPath)) {
                $shimTarget = ((Select-String -Path $shimPath -Pattern '[''"]([^@&]*?)[''"]' -AllMatches).Matches.Groups | Select-Object -Last 1).Value
            }
            $shimTarget | Convert-Path -ErrorAction SilentlyContinue
        } elseif ($comm.CommandType -eq 'Application') {
            $comm.Source
        } elseif ($comm.CommandType -eq 'Alias') {
            Get-CommandPath $comm.ResolvedCommandName
        } else {
            $null
        }
        return $commandPath
    }
}

# core.ps1:628
function Get-AbsolutePath {
    [CmdletBinding()]
    [OutputType([string])]
    param (
        [Parameter(Mandatory = $true, ValueFromPipeline = $true)]
        [string]
        $Path
    )
    process {
        $resolvedPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path)
        if ($resolvedPath -match '[\\/]$') {
            $root = [System.IO.Path]::GetPathRoot($resolvedPath)
            if ($resolvedPath -ine $root) {
                $resolvedPath = $resolvedPath.TrimEnd([char[]]@('\', '/'))
            }
        }
        return $resolvedPath
    }
}

# core.ps1:663
function friendly_path($path) {
    $h = (Get-PSProvider 'FileSystem').Home
    if (!$h.EndsWith('\')) {
        $h += '\'
    }
    if ($h -eq '\') {
        return $path
    } else {
        return $path -replace ([Regex]::Escape($h)), '~\'
    }
}

# core.ps1:864
function movedir($from, $to) {
    $from = $from.trimend('\')
    $to = $to.trimend('\')

    $proc = New-Object System.Diagnostics.Process
    $proc.StartInfo.FileName = "$env:SystemRoot\System32\robocopy.exe"
    $proc.StartInfo.Arguments = "`"$from`" `"$to`" /e /move"
    $proc.StartInfo.RedirectStandardOutput = $true
    $proc.StartInfo.RedirectStandardError = $true
    $proc.StartInfo.UseShellExecute = $false
    $proc.StartInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    [void]$proc.Start()
    $stdoutTask = $proc.StandardOutput.ReadToEndAsync()
    $proc.WaitForExit()

    if ($proc.ExitCode -ge 8) {
        throw "Could not find '$(Split-Path -Leaf $from)'! (error $($proc.ExitCode))"
    }

    # wait for robocopy to terminate its threads
    1..10 | ForEach-Object {
        if (Test-Path $from) {
            Start-Sleep -Milliseconds 100
        }
    }
}

# system.ps1:76
function Split-PathLikeEnvVar {
    param(
        [string[]]$Pattern,
        [string]$Path
    )

    if ($null -eq $Path -and $Path -eq '') {
        return $null, $null
    } else {
        $splitPattern = $Pattern.Split(';', [System.StringSplitOptions]::RemoveEmptyEntries)
        $splitPath = $Path.Split(';', [System.StringSplitOptions]::RemoveEmptyEntries)
        $inPath = @()
        foreach ($p in $splitPattern) {
            $inPath += $splitPath.Where({ $_ -like $p })
            $splitPath = $splitPath.Where({ $_ -notlike $p })
        }
        return ($inPath -join ';'), ($splitPath -join ';')
    }
}

# json.ps1:6 (normalize_values json.ps1:161 inlined as a scriptblock)
function ConvertToPrettyJson {
    [CmdletBinding()]
    Param (
        [Parameter(Mandatory, ValueFromPipeline)]
        $data
    )

    Process {
        $normalize = {
            param([psobject] $json)
            $json.PSObject.Properties | ForEach-Object {
                if ($_.Value -is [System.Management.Automation.PSCustomObject]) {
                    $_.Value = & $normalize $_.Value
                }
                if ($_.Value -is [String]) {
                    [Array] $parts = ($_.Value -split '\r?\n').Trim()
                    if ($parts.Count -gt 1) {
                        $_.Value = $parts
                    }
                }
                if ($_.Value -is [Array]) {
                    if ($_.Value.Count -eq 1) {
                        if ($_.Value[0] -is [Array]) {
                            $_.Value = $_.Value
                        } else {
                            $_.Value = $_.Value[0]
                        }
                    } else {
                        $resulted_arrs = @()
                        foreach ($element in $_.Value) {
                            if ($element.Count -eq 1) {
                                $resulted_arrs += $element
                            } else {
                                $resulted_arrs += , $element
                            }
                        }
                        $_.Value = $resulted_arrs
                    }
                }
            }
            return $json
        }
        $data = & $normalize $data

        [String]$json = $data | ConvertTo-Json -Depth 8 -Compress
        [String]$output = ''
        [String]$buffer = ''
        [Int]$depth = 0
        [Bool]$inString = $false
        [String]$indent = ' ' * 4
        [Bool]$unescapeString = $true
        [String]$eol = "`r`n"

        for ($i = 0; $i -lt $json.Length; $i++) {
            $buffer = $json.Substring($i, 1)

            $objectStart = !$inString -and $buffer.Equals('{')
            $objectEnd = !$inString -and $buffer.Equals('}')
            $arrayStart = !$inString -and $buffer.Equals('[')
            $arrayEnd = !$inString -and $buffer.Equals(']')
            $colon = !$inString -and $buffer.Equals(':')
            $comma = !$inString -and $buffer.Equals(',')
            $quote = $buffer.Equals('"')
            $escape = $buffer.Equals('\')

            if ($quote) {
                $inString = !$inString
            }

            if ($escape) {
                $buffer = $json.Substring($i, 2)
                ++$i

                if ($inString -and $unescapeString) {
                    if ($buffer.Equals('\n')) {
                        $buffer = "`n"
                    } elseif ($buffer.Equals('\r')) {
                        $buffer = "`r"
                    } elseif ($buffer.Equals('\t')) {
                        $buffer = "`t"
                    } elseif ($buffer.Equals('\u')) {
                        $buffer = [regex]::Unescape($json.Substring($i - 1, 6))
                        $i += 4
                    }
                }

                $output += $buffer
                continue
            }

            if ($objectStart -or $arrayStart) {
                ++$depth
            } elseif ($objectEnd -or $arrayEnd) {
                --$depth
                $output += $eol + ($indent * $depth)
            }

            $output += $buffer

            if ($colon) {
                $output += ' '
            } elseif ($comma -or $arrayStart -or $objectStart) {
                $output += $eol
                $output += $indent * $depth
            }
        }

        return $output
    }
}

# ---- Group 2 (Task 153): intercepted global effects ----
# These functions never touch the registry, the real PATH or create .lnk files: each
# effect becomes one JSON line appended to $__wr_effects; the executor applies them.

# internal: append one effect (hashtable) as a compressed JSON line, UTF-8 without BOM, LF
function __wr_effect($effect) {
    if ([string]::IsNullOrEmpty($__wr_effects)) { throw 'effects file not defined by the runner' }
    $line = ConvertTo-Json -InputObject $effect -Compress -Depth 4
    [System.IO.File]::AppendAllText($__wr_effects, $line + "`n", (New-Object System.Text.UTF8Encoding $false))
}

# internal: looks a variable up in this session's overlay, then in the seed JSON
# ($__wr_env_seed); returns @(found, value) and never throws for an unknown name
function __wr_env_lookup($Name) {
    if ($null -eq $script:__wr_env_overlay) { $script:__wr_env_overlay = @{} }
    if ($script:__wr_env_overlay.ContainsKey($Name)) {
        return @($true, $script:__wr_env_overlay[$Name])
    }
    if (![string]::IsNullOrEmpty($__wr_env_seed)) {
        $seed = ConvertFrom-Json -InputObject $__wr_env_seed -ErrorAction Stop
        $prop = $seed.PSObject.Properties[$Name]
        if ($null -ne $prop) { return @($true, $prop.Value) }
    }
    return @($false, $null)
}

# system.ps1:30 -- reads the seed JSON ($__wr_env_seed) overlaid by this session's writes
function Get-EnvVar {
    param(
        [string]$Name,
        [switch]$Global
    )
    if ($Global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    $found, $value = __wr_env_lookup $Name
    if ($found) { return $value }
    throw ("Get-EnvVar: a vari" + [char]0xE1 + "vel '$Name' n" + [char]0xE3 + "o foi pr" + [char]0xE9 + "-carregada pelo wingetrs")
}

# system.ps1:46 -- records set_env and the session overlay; empty value removes
function Set-EnvVar {
    param(
        [string]$Name,
        $Value,
        [switch]$Global
    )
    if ($Global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    if ($null -eq $Value -or "$Value" -eq '') { $Value = $null } else { $Value = "$Value" }
    __wr_effect @{ op = 'set_env'; name = $Name; value = $Value }
    if ($null -eq $script:__wr_env_overlay) { $script:__wr_env_overlay = @{} }
    $script:__wr_env_overlay[$Name] = $Value
}

# system.ps1:96 -- records add_path (prepend, force) and updates the overlay (when the
# variable is known) and the session variable, like the original
function Add-Path {
    param(
        [string[]]$Path,
        [string]$TargetEnvVar = 'PATH',
        [switch]$Global,
        [switch]$Force,
        [switch]$Quiet
    )
    if ($Global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    __wr_effect @{ op = 'add_path'; var = $TargetEnvVar; entries = [string[]]@($Path); prepend = $true; force = [bool]$Force }
    # future sessions (overlay); an unknown variable stays unknown
    $found, $value = __wr_env_lookup $TargetEnvVar
    if ($found) {
        $inPath, $strippedPath = Split-PathLikeEnvVar $Path "$value"
        if (!$inPath -or $Force) {
            $script:__wr_env_overlay[$TargetEnvVar] = (@($Path) + @($strippedPath | Where-Object { $_ })) -join ';'
        }
    }
    # current session
    $session = (Get-Item -LiteralPath "env:$TargetEnvVar" -ErrorAction SilentlyContinue).Value
    $inPath, $strippedPath = Split-PathLikeEnvVar $Path "$session"
    if (!$inPath -or $Force) {
        Set-Item -LiteralPath "env:$TargetEnvVar" -Value ((@($Path) + @($strippedPath | Where-Object { $_ })) -join ';')
    }
}

# system.ps1:122 -- records remove_path and updates the overlay (when the variable is
# known) and the session variable; -PassThru returns what was in the stored value
function Remove-Path {
    param(
        [string[]]$Path,
        [string]$TargetEnvVar = 'PATH',
        [switch]$Global,
        [switch]$Quiet,
        [switch]$PassThru
    )
    if ($Global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    __wr_effect @{ op = 'remove_path'; var = $TargetEnvVar; entries = [string[]]@($Path) }
    # future sessions (overlay); an unknown variable stays unknown
    $inPath = $null
    $found, $value = __wr_env_lookup $TargetEnvVar
    if ($found) {
        $inPath, $strippedPath = Split-PathLikeEnvVar $Path "$value"
        if ($inPath) {
            $script:__wr_env_overlay[$TargetEnvVar] = if ($strippedPath) { $strippedPath } else { $null }
        }
    }
    # current session
    $session = (Get-Item -LiteralPath "env:$TargetEnvVar" -ErrorAction SilentlyContinue).Value
    $inSessionPath, $strippedPath = Split-PathLikeEnvVar $Path "$session"
    if ($inSessionPath) {
        if ($strippedPath) {
            Set-Item -LiteralPath "env:$TargetEnvVar" -Value $strippedPath
        } else {
            Remove-Item -LiteralPath "env:$TargetEnvVar" -ErrorAction SilentlyContinue
        }
    }
    if ($PassThru) {
        return $inPath
    }
}

# system.ps1:153 (deprecated)
function env($name, $global, $val) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    if ($PSBoundParameters.ContainsKey('val')) {
        Set-EnvVar -Name $name -Value $val
    } else {
        Get-EnvVar -Name $name
    }
}

# system.ps1:168 (deprecated)
function add_first_in_path($dir, $global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    Add-Path -Path $dir -Force
}

# system.ps1:173 (deprecated)
function remove_from_path($dir, $global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    Remove-Path -Path $dir
}

# core.ps1:949 -- records shim only; the executor keeps the shims dir on the PATH
function shim($path, $global, $name, $arg) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    if (!(Test-Path $path)) { abort "Can't shim '$(Split-Path -Leaf $path)': couldn't find '$path'." }
    if (!$name) { $name = [System.IO.Path]::GetFileNameWithoutExtension("$path") }
    $resolved = Convert-Path $path
    $a = if ($arg) { "$arg" } else { $null }
    __wr_effect @{ op = 'shim'; path = $resolved; name = "$name".ToLowerInvariant(); args = $a }
}

# install.ps1:195 -- records rm_shim only
function rm_shim($name, $shimdir, $app) {
    __wr_effect @{ op = 'rm_shim'; name = "$name" }
}

# shortcuts.ps1:22 -- the shortcut root comes from $__wr_shortcut_root (created if missing)
function shortcut_folder($global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    if ([string]::IsNullOrEmpty($__wr_shortcut_root)) { throw 'shortcut root not defined by the runner' }
    return Convert-Path (ensure $__wr_shortcut_root)
}

# shortcuts.ps1:31 -- records shortcut only (no WScript.Shell, no .lnk)
function startmenu_shortcut([System.IO.FileInfo] $target, $shortcutName, $arguments, [System.IO.FileInfo]$icon, $global) {
    if ($global) { throw ('global n' + [char]0xE3 + 'o suportado pelo wingetrs') }
    if (!$target.Exists) {
        Write-Host -f DarkRed "Creating shortcut for $shortcutName ($(Split-Path -Leaf $target)) failed: Couldn't find $target"
        return
    }
    if ($icon -and !$icon.Exists) {
        Write-Host -f DarkRed "Creating shortcut for $shortcutName ($(Split-Path -Leaf $target)) failed: Couldn't find icon $icon"
        return
    }
    $a = if ($arguments) { "$arguments" } else { $null }
    $i = if ($icon) { $icon.FullName } else { $null }
    __wr_effect @{ op = 'shortcut'; target = $target.FullName; name = "$shortcutName"; args = $a; icon = $i }
}

# ---- Group 3 (Task 154): extraction through helpers, external commands, persist ----
# Helpers (7z, lessmsi, innounp, dark) come only from $__wr_helpers as absolute paths;
# nothing is looked up on the PATH. The Windows Installer is never run from here.

# internal: absolute path of a helper, or throws when it is not listed / not on disk
function __wr_helper($name) {
    $p = $null
    if ($null -ne $__wr_helpers -and $__wr_helpers.ContainsKey($name)) { $p = $__wr_helpers[$name] }
    if ([string]::IsNullOrEmpty($p) -or !(Test-Path -LiteralPath $p -PathType Leaf)) {
        throw ("auxiliar '$name' n" + [char]0xE3 + 'o instalado')
    }
    return $p
}

# internal: a relative path taken from a manifest or archive listing; refuses '..',
# rooted paths, drives, UNC and alternate streams (install::safepath rule)
function __wr_safe_rel($p) {
    $s = "$p"
    if ($s.Contains(':') -or $s -match '^[\\/]' -or $s -match '(^|[\\/])\.\.([\\/]|$)') {
        throw ('caminho inseguro: ' + $s)
    }
    return $s
}

# internal: runs a helper through Invoke-ExternalCommand; throws with the exit code
function __wr_run($exe, $argList, $log, $path) {
    $ok = Invoke-ExternalCommand $exe $argList -LogPath $log
    if (!$ok) {
        throw ('falha ao extrair ' + $path + ' (c' + [char]0xF3 + 'digo ' + $script:__wr_last_exit + ')')
    }
}

# internal: directory that holds the archive ('.' when the path has none)
function __wr_dirof($path) {
    $d = Split-Path $path
    if ([string]::IsNullOrEmpty($d)) { return '.' }
    return $d
}

# core.ps1:713 -- elevation (-RunAs) and the Windows Installer are refused; the exit code of
# the last call stays in $script:__wr_last_exit; returns $true / $false like the original
function Invoke-ExternalCommand {
    [CmdletBinding(DefaultParameterSetName = "Default")]
    [OutputType([Boolean])]
    param (
        [Parameter(Mandatory = $true, Position = 0)]
        [Alias("Path")]
        [ValidateNotNullOrEmpty()]
        [String]
        $FilePath,
        [Parameter(Position = 1)]
        [Alias("Args")]
        [String[]]
        $ArgumentList,
        [Parameter(ParameterSetName = "UseShellExecute")]
        [Switch]
        $RunAs,
        [Parameter(ParameterSetName = "UseShellExecute")]
        [Switch]
        $Quiet,
        [Alias("Msg")]
        [String]
        $Activity,
        [Alias("cec")]
        [Hashtable]
        $ContinueExitCodes,
        [Parameter(ParameterSetName = "Default")]
        [Alias("Log")]
        [String]
        $LogPath
    )
    if ($RunAs) { throw ('eleva' + [char]0xE7 + [char]0xE3 + 'o n' + [char]0xE3 + 'o suportada pelo wingetrs') }
    if ([System.IO.Path]::GetFileNameWithoutExtension($FilePath) -like 'msi*exec') {
        throw ('o Windows Installer s' + [char]0xF3 + ' roda pela fila do wingetrs')
    }
    $script:__wr_last_exit = -1
    if ($Activity) {
        Write-Host "$Activity " -NoNewline
    }
    $Process = New-Object System.Diagnostics.Process
    $Process.StartInfo.FileName = $FilePath
    $Process.StartInfo.UseShellExecute = $false
    if ($LogPath) {
        $redirectToLogFile = $true
        $Process.StartInfo.RedirectStandardOutput = $true
        $Process.StartInfo.RedirectStandardError = $true
    }
    if ($Quiet) {
        $Process.StartInfo.UseShellExecute = $true
        $Process.StartInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    }
    if ($ArgumentList.Length -gt 0) {
        # remove existing double quotes and split arguments before each switch
        $ArgumentList = $ArgumentList.ForEach({ $_ -replace '"' -split '(?<=(?<![:\w])[/-]\w+) | (?=[/-])' })
        # legacy escaping for commands with non-standard argument handling
        $LegacyCommand = $FilePath -match '^((cmd|cscript|find|sqlcmd|wscript)(\.exe)?|.*\.(bat|cmd|js|vbs|wsf))$' -or
        ($ArgumentList -match '^/S$|^/D=[A-Z]:[\\/].*$').Length -eq 2
        $SupportArgumentList = $Process.StartInfo.PSObject.Properties.Name -contains 'ArgumentList'
        if ((-not $LegacyCommand) -and $SupportArgumentList) {
            $ArgumentList.ForEach({ $Process.StartInfo.ArgumentList.Add($_) })
        } else {
            $escapedArgs = switch -regex ($ArgumentList) {
                # quote paths starting with a drive letter
                '(?<!/D=)[A-Z]:[\\/].*' { $_ -replace '([A-Z]:[\\/].*)', '"$1"'; continue }
                # do not quote paths if it is NSIS's '/D' argument
                '/D=[A-Z]:[\\/].*' { $_; continue }
                # quote args with spaces
                ' ' { "`"$_`""; continue }
                default { $_; continue }
            }
            $Process.StartInfo.Arguments = $escapedArgs -join ' '
        }
    }
    try {
        [void]$Process.Start()
    } catch {
        if ($Activity) {
            Write-Host "Error." -ForegroundColor DarkRed
        }
        error $_.Exception.Message
        return $false
    }
    if ($redirectToLogFile) {
        # read both streams asynchronously to avoid a deadlock
        $stdoutTask = $Process.StandardOutput.ReadToEndAsync()
        $stderrTask = $Process.StandardError.ReadToEndAsync()
    }
    $Process.WaitForExit()
    if ($redirectToLogFile) {
        $utf8 = New-Object System.Text.UTF8Encoding $false
        [System.IO.File]::AppendAllText($LogPath, "$($stdoutTask.Result)", $utf8)
        [System.IO.File]::AppendAllText($LogPath, "$($stderrTask.Result)", $utf8)
    }
    $code = $Process.ExitCode
    $script:__wr_last_exit = $code
    if ($code -ne 0) {
        if ($ContinueExitCodes -and ($ContinueExitCodes.ContainsKey($code))) {
            if ($Activity) {
                Write-Host "Done." -ForegroundColor DarkYellow
            }
            warn $ContinueExitCodes[$code]
            return $true
        } else {
            if ($Activity) {
                Write-Host "Error." -ForegroundColor DarkRed
            }
            error "Exit code was $code!"
            return $false
        }
    }
    if ($Activity) {
        Write-Host "Done." -ForegroundColor Green
    }
    return $true
}

# decompress.ps1:63 -- 7z.exe from $__wr_helpers['7zip'] (use_external_7zip is always off)
function Expand-7zipArchive {
    [CmdletBinding()]
    param (
        [Parameter(Mandatory = $true, Position = 0, ValueFromPipeline = $true)]
        [String]
        $Path,
        [Parameter(Position = 1)]
        [String]
        $DestinationPath = (Split-Path $Path),
        [String]
        $ExtractDir,
        [Parameter(ValueFromRemainingArguments = $true)]
        [String]
        $Switches,
        [ValidateSet('All', 'Skip', 'Rename')]
        [String]
        $Overwrite,
        [Switch]
        $Removal
    )
    if ($ExtractDir) { [void](__wr_safe_rel $ExtractDir) }
    $7zPath = __wr_helper '7zip'
    $LogPath = "$(__wr_dirof $Path)\7zip.log"
    $DestinationPath = $DestinationPath.TrimEnd('\')
    $ArgList = @('x', $Path, "-o$DestinationPath", '-xr!*.nsis', '-y')
    $IsTar = (($Path -replace '\.[^\.]*$', '') -match '\.tar$') -or ($Path -match '\.t[abgpx]z2?$')
    if (!$IsTar -and $ExtractDir) {
        $ArgList += "-ir!$ExtractDir\*"
    }
    if ($Switches) {
        $ArgList += (-split $Switches)
    }
    switch ($Overwrite) {
        'All' { $ArgList += '-aoa' }
        'Skip' { $ArgList += '-aos' }
        'Rename' { $ArgList += '-aou' }
    }
    __wr_run $7zPath $ArgList $LogPath $Path
    if ($IsTar) {
        # check for tar: list the archive, then extract the inner tar
        $Status = Invoke-ExternalCommand $7zPath @('l', $Path) -LogPath $LogPath
        if ($Status) {
            $TarFile = (Select-String -Path $LogPath -Pattern '[^ ]*tar$').Matches.Value
            [void](__wr_safe_rel $TarFile)
            Expand-7zipArchive -Path "$DestinationPath\$TarFile" -DestinationPath $DestinationPath -ExtractDir $ExtractDir -Removal
        } else {
            throw ('falha ao listar ' + $Path + ': n' + [char]0xE3 + 'o ' + [char]0xE9 + ' um arquivo aceito pelo 7-Zip')
        }
    }
    if (!$IsTar -and $ExtractDir) {
        movedir "$DestinationPath\$ExtractDir" $DestinationPath | Out-Null
        # remove the temporary directory if it is empty
        $ExtractDirTopPath = [string] "$DestinationPath\$($ExtractDir -replace '[\\/].*')"
        if ((Get-ChildItem -Path $ExtractDirTopPath -Force -ErrorAction Ignore).Count -eq 0) {
            Remove-Item -Path $ExtractDirTopPath -Recurse -Force -ErrorAction Ignore
        }
    }
    if (Test-Path $LogPath) {
        Remove-Item $LogPath -Force
    }
    if ($Removal) {
        # remove the original archive file
        Remove-Item $Path -Force
    }
}

# decompress.ps1:169 -- only the lessmsi branch; the other branch is not ported
function Expand-MsiArchive {
    [CmdletBinding()]
    param (
        [Parameter(Mandatory = $true, Position = 0, ValueFromPipeline = $true)]
        [String]
        $Path,
        [Parameter(Position = 1)]
        [String]
        $DestinationPath = (Split-Path $Path),
        [String]
        $ExtractDir,
        [Parameter(ValueFromRemainingArguments = $true)]
        [String]
        $Switches,
        [Switch]
        $Removal
    )
    if ($ExtractDir) { [void](__wr_safe_rel $ExtractDir) }
    $MsiPath = __wr_helper 'lessmsi'
    $DestinationPath = $DestinationPath.TrimEnd('\')
    if ($ExtractDir) {
        $OriDestinationPath = $DestinationPath
        $DestinationPath = "$DestinationPath\_tmp"
    }
    $ArgList = @('x', $Path, "$DestinationPath\")
    $LogPath = "$(__wr_dirof $Path)\msi.log"
    if ($Switches) {
        $ArgList += (-split $Switches)
    }
    __wr_run $MsiPath $ArgList $LogPath $Path
    if ($ExtractDir -and (Test-Path "$DestinationPath\SourceDir")) {
        movedir "$DestinationPath\SourceDir\$ExtractDir" $OriDestinationPath | Out-Null
        Remove-Item $DestinationPath -Recurse -Force
    } elseif ($ExtractDir) {
        movedir "$DestinationPath\$ExtractDir" $OriDestinationPath | Out-Null
        Remove-Item $DestinationPath -Recurse -Force
    } elseif (Test-Path "$DestinationPath\SourceDir") {
        movedir "$DestinationPath\SourceDir" $DestinationPath | Out-Null
    }
    if (($DestinationPath -ne (Split-Path $Path)) -and (Test-Path "$DestinationPath\$(Split-Path $Path -Leaf)")) {
        Remove-Item "$DestinationPath\$(Split-Path $Path -Leaf)" -Force
    }
    if (Test-Path $LogPath) {
        Remove-Item $LogPath -Force
    }
    if ($Removal) {
        # remove the original archive file
        Remove-Item $Path -Force
    }
}

# decompress.ps1:227 -- same arguments as inno_args (inno.rs); exit 0 with no files is an error
function Expand-InnoArchive {
    [CmdletBinding()]
    param (
        [Parameter(Mandatory = $true, Position = 0, ValueFromPipeline = $true)]
        [String]
        $Path,
        [Parameter(Position = 1)]
        [String]
        $DestinationPath = (Split-Path $Path),
        [String]
        $ExtractDir,
        [Parameter(ValueFromRemainingArguments = $true)]
        [String]
        $Switches,
        [Switch]
        $Removal
    )
    if ($ExtractDir) { [void](__wr_safe_rel $ExtractDir) }
    $InnoPath = __wr_helper 'innounp'
    $LogPath = "$(__wr_dirof $Path)\innounp.log"
    $ArgList = @('-x', "-d$DestinationPath", $Path, '-y')
    if (!$ExtractDir) {
        $ArgList += '-c{app}'
    } elseif ($ExtractDir.StartsWith('{')) {
        $ArgList += "-c$ExtractDir"
    } else {
        $ArgList += "-c{app}\$ExtractDir"
    }
    if ($Switches) {
        $ArgList += (-split $Switches)
    }
    __wr_run $InnoPath $ArgList $LogPath $Path
    # innounp exits 0 with no files when the filter matches nothing (probe D4)
    $count = @(Get-ChildItem -LiteralPath $DestinationPath -Recurse -File -Force -ErrorAction SilentlyContinue).Count
    if ($count -eq 0) {
        throw ('innounp: nenhum arquivo extra' + [char]0xED + 'do')
    }
    if (Test-Path $LogPath) {
        Remove-Item $LogPath -Force
    }
    if ($Removal) {
        # remove the original archive file
        Remove-Item $Path -Force
    }
}

# decompress.ps1:267 -- System.IO.Compression.ZipFile (zip-slip safe in .NET)
function Expand-ZipArchive {
    [CmdletBinding()]
    param (
        [Parameter(Mandatory = $true, Position = 0, ValueFromPipeline = $true)]
        [String]
        $Path,
        [Parameter(Position = 1)]
        [String]
        $DestinationPath = (Split-Path $Path),
        [String]
        $ExtractDir,
        [Switch]
        $Removal
    )
    if ($ExtractDir) {
        [void](__wr_safe_rel $ExtractDir)
        $OriDestinationPath = $DestinationPath
        $DestinationPath = "$DestinationPath\_tmp"
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory((Get-AbsolutePath $Path), (Get-AbsolutePath $DestinationPath), $true)
    if ($ExtractDir) {
        movedir "$DestinationPath\$ExtractDir" $OriDestinationPath | Out-Null
        Remove-Item $DestinationPath -Recurse -Force
    }
    if ($Removal) {
        # remove the original archive file
        Remove-Item $Path -Force
    }
}

# decompress.ps1:303
function Expand-DarkArchive {
    [CmdletBinding()]
    param (
        [Parameter(Mandatory = $true, Position = 0, ValueFromPipeline = $true)]
        [String]
        $Path,
        [Parameter(Position = 1)]
        [String]
        $DestinationPath = (Split-Path $Path),
        [Parameter(ValueFromRemainingArguments = $true)]
        [String]
        $Switches,
        [Switch]
        $Removal
    )
    $DarkPath = __wr_helper 'dark'
    $LogPath = "$(__wr_dirof $Path)\dark.log"
    if ((Split-Path $DarkPath -Leaf) -eq 'wix.exe') {
        $ArgList = @('burn', 'extract', $Path, '-out', $DestinationPath, '-outba', "$DestinationPath\UX")
    } else {
        $ArgList = @('-nologo', '-x', $DestinationPath, $Path)
    }
    if ($Switches) {
        $ArgList += (-split $Switches)
    }
    __wr_run $DarkPath $ArgList $LogPath $Path
    if (Test-Path "$DestinationPath\WixAttachedContainer") {
        Rename-Item "$DestinationPath\WixAttachedContainer" 'AttachedContainer' -ErrorAction Ignore
    } else {
        if (Test-Path "$DestinationPath\AttachedContainer\a0") {
            $Xml = [xml](Get-Content -Raw "$DestinationPath\UX\manifest.xml" -Encoding utf8)
            # names come from the installer: skip any that leave their folder
            $Xml.BurnManifest.UX.Payload | ForEach-Object {
                try { [void](__wr_safe_rel $_.SourcePath); [void](__wr_safe_rel $_.FilePath) } catch { return }
                Rename-Item "$DestinationPath\UX\$($_.SourcePath)" $_.FilePath -ErrorAction Ignore
            }
            $Xml.BurnManifest.Payload | ForEach-Object {
                try { [void](__wr_safe_rel $_.SourcePath); [void](__wr_safe_rel $_.FilePath) } catch { return }
                Rename-Item "$DestinationPath\AttachedContainer\$($_.SourcePath)" $_.FilePath -ErrorAction Ignore
            }
        }
    }
    if (Test-Path $LogPath) {
        Remove-Item $LogPath -Force
    }
    if ($Removal) {
        # remove the original archive file
        Remove-Item $Path -Force
    }
}

# install.ps1:430 (persist_def, internal)
function __wr_persist_def($persist) {
    if ($persist -is [Array]) {
        $source = $persist[0]
        $target = $persist[1]
    } else {
        $source = $persist
        $target = $null
    }
    if (!$target) {
        $target = $source
    }
    return $source, $target
}

# install.ps1:444 -- $dir is the caller's variable, as in the original. Every definition is
# checked (relative, no '..') before anything is created, so links stay under $dir and
# the data under $persist_dir.
function persist_data($manifest, $original_dir, $persist_dir) {
    $persist = $manifest.persist
    if ($persist) {
        if ([string]::IsNullOrEmpty($dir)) { throw '$dir not defined by the script' }
        foreach ($p in @($persist)) {
            $s, $t = __wr_persist_def $p
            [void](__wr_safe_rel $s)
            [void](__wr_safe_rel $t)
        }

        $persist_dir = ensure $persist_dir

        if ($persist -is [String]) {
            $persist = @($persist)
        }

        $persist | ForEach-Object {
            $source, $target = __wr_persist_def $_

            Write-Host "Persisting $source"

            $source = $source.TrimEnd('/').TrimEnd('\\')

            $source = "$dir\$source"
            $target = "$persist_dir\$target"

            # if we have had persist data in the store, just create link and go
            if (Test-Path -LiteralPath $target) {
                # if there is also a source data, rename it (to keep a original backup)
                if (Test-Path -LiteralPath $source) {
                    Move-Item -Force -LiteralPath $source -Destination "$source.original"
                }
                # we don't have persist data in the store, move the source to target, then create link
            } elseif (Test-Path -LiteralPath $source) {
                # ensure target parent folder exist
                ensure (Split-Path -Path $target) | Out-Null
                Move-Item -LiteralPath $source -Destination $target
                # we have neither source nor target data: create an empty target directory
            } else {
                ensure $target | Out-Null
            }

            # create link
            if (Test-Path -LiteralPath $target -PathType Container) {
                # target is a directory, create junction
                New-Item -Path $source -ItemType Junction -Value $target | Out-Null
                & "$env:SystemRoot\System32\attrib.exe" $source +R /L | Out-Null
            } else {
                # target is a file, create hard link
                New-Item -Path $source -ItemType HardLink -Value $target | Out-Null
            }
        }
    }
}

# install.ps1:496
function unlink_persist_data($manifest, $dir) {
    $persist = $manifest.persist
    # unlink all junction / hard link in the directory
    if ($persist) {
        @($persist) | ForEach-Object {
            $source, $null = __wr_persist_def $_
            [void](__wr_safe_rel $source)
            $source = Get-Item -LiteralPath "$dir\$source" -ErrorAction SilentlyContinue
            if ($source.LinkType) {
                $source_path = $source.FullName
                # directory (junction)
                if ($source -is [System.IO.DirectoryInfo]) {
                    # remove read-only attribute on the link
                    & "$env:SystemRoot\System32\attrib.exe" -R $source_path /L | Out-Null
                    # remove the junction
                    Remove-Item -LiteralPath $source_path -Recurse -Force -ErrorAction SilentlyContinue
                } else {
                    # remove the hard link
                    Remove-Item -LiteralPath $source_path -Force -ErrorAction SilentlyContinue
                }
            }
        }
    }
}

# install.ps1:521 -- no effect: wingetrs never runs with $global
function persist_permission($manifest, $global) {
}
