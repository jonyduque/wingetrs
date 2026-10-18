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
