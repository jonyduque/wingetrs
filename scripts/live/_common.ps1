#Requires -Version 7.0
<#
.SYNOPSIS
    Shared step runner for the guided live-proof sections (dot-sourced by section-*.ps1).

.DESCRIPTION
    Each section script calls Start-Section, then Invoke-Step once per roteiro step, then
    Complete-Section in a `finally`. A step is a hashtable:

      Id, Title          required
      Mode               Capture (default; stdout+stderr teed), Stdout (stdout teed, stderr live
                         on the console: progress line stays visible), Attached (inherits the
                         console: selectors and installer windows work; nothing captured),
                         Window (new console window: Ctrl+C there does not stop this script),
                         Parallel (Cmd in a new window, then Cmd2 here), Script (Run scriptblock),
                         Admin (AdminText for the user's elevated terminal), Manual (TUI etc.)
      Cmd / Cmd2         string[]: executable followed by its arguments (never a command line)
      Run                scriptblock (Script mode); its output is teed
      Show               string[] shown as the command (Script/Manual modes)
      AdminText          string or scriptblock returning the text to run as administrator
      Instructions       string[] printed before the prompt
      Uac                warning text: the step opens (or must not open) a UAC prompt
      Destructive        $true: asks an explicit "s" before running (uninstall, reg import...)
      When / WhenWhy     scriptblock; false => step skipped automatically, with the reason
      ExpectExit         int, 'nonzero' or 'any' (default 0 for commands, none for the rest)
      Expect / NotExpect substrings looked for in the captured output (asked when not captured)
      Checks             @(@{ Name = '...'; Test = { $true/$false or info text } })
      Ask                yes/no questions for visual checks
      Notes              free-text prompts ("anote ...")
      Watch              $true: polls %LOCALAPPDATA%\wingetrs\installers\run-* while it runs
      After              scriptblock run after the step (e.g. save state); gets the record

    Nothing here elevates by itself: Admin steps print the exact text for the user's elevated
    terminal and only use Start-Process -Verb RunAs after an explicit "s".
#>
Set-StrictMode -Version Latest
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$OutputEncoding = [Text.Encoding]::UTF8
$PSNativeCommandUseErrorActionPreference = $false

$script:RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$script:ProvaRoot = Join-Path $env:LOCALAPPDATA 'wingetrs-prova'
$script:WData = Join-Path $env:LOCALAPPDATA 'wingetrs'
$script:Ctx = $null

# ---------------------------------------------------------------- formatting helpers

function Quote-Ps([string]$s) { "'" + $s.Replace("'", "''") + "'" }

function Format-Cmd([string[]]$Cmd) {
    $parts = foreach ($c in $Cmd) {
        if ($c -eq '' -or $c -match '[\s''"`$;&|(){}@,<>#]') { Quote-Ps $c } else { $c }
    }
    '& ' + ($parts -join ' ')
}

function Remove-Ansi([string]$s) { $s -replace '\x1b\[[0-9;?]*[ -/]*[@-~]', '' }

function Write-Banner([string]$Text, [string]$Color = 'Cyan') {
    Write-Host ''
    Write-Host ('=' * 78) -ForegroundColor $Color
    Write-Host $Text -ForegroundColor $Color
    Write-Host ('=' * 78) -ForegroundColor $Color
}

function Read-Choice([string]$Prompt, [string[]]$Allowed) {
    while ($true) {
        $r = (Read-Host $Prompt).Trim().ToLowerInvariant()
        if ($Allowed -contains $r) { return $r }
        Write-Host "  resposta inválida; use: $(($Allowed | ForEach-Object { if ($_ -eq '') { 'Enter' } else { $_ } }) -join ' / ')" -ForegroundColor Yellow
    }
}

function Get-PwshPath { (Get-Process -Id $PID).Path }

# ---------------------------------------------------------------- run dir and state

function New-RunDir { Join-Path $script:ProvaRoot ('run-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }

function Get-LatestRunDir {
    $last = Get-ChildItem -Directory -LiteralPath $script:ProvaRoot -Filter 'run-*' -ErrorAction SilentlyContinue |
        Sort-Object Name | Select-Object -Last 1
    if ($last) { $last.FullName } else { $null }
}

function Get-State([string]$Key) {
    $f = Join-Path $script:Ctx.RunDir 'state.json'
    if (-not (Test-Path -LiteralPath $f)) { return $null }
    $h = Get-Content -LiteralPath $f -Raw | ConvertFrom-Json -AsHashtable
    if ($h.ContainsKey($Key)) { $h[$Key] } else { $null }
}

function Set-State([string]$Key, $Value) {
    $f = Join-Path $script:Ctx.RunDir 'state.json'
    $h = if (Test-Path -LiteralPath $f) { Get-Content -LiteralPath $f -Raw | ConvertFrom-Json -AsHashtable } else { @{} }
    $h[$Key] = $Value
    $h | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $f -Encoding utf8
}

# ---------------------------------------------------------------- environment helpers

# Value of an environment variable as a new terminal would see it (registry, not this process).
function Get-FreshEnv([string]$Name) {
    $u = (Get-Item 'HKCU:\Environment').GetValue($Name, $null, 'DoNotExpandEnvironmentNames')
    $m = (Get-Item 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment').GetValue($Name, $null, 'DoNotExpandEnvironmentNames')
    $vals = if ($Name -eq 'Path') { @($m, $u) } elseif ($u) { @($u) } else { @($m) }
    (@($vals | Where-Object { $_ } | ForEach-Object { [Environment]::ExpandEnvironmentVariables($_) })) -join ';'
}

function Find-OnPath([string]$Name, [string]$PathValue) {
    $exts = if ([IO.Path]::HasExtension($Name)) { @('') } else { @($env:PATHEXT -split ';' | Where-Object { $_ }) }
    foreach ($d in ($PathValue -split ';' | Where-Object { $_ })) {
        foreach ($e in $exts) {
            $c = Join-Path $d ($Name + $e)
            if (Test-Path -LiteralPath $c -PathType Leaf) { return $c }
        }
    }
    $null
}

# Runs a program with the PATH/PSModulePath of a new terminal and returns its output lines.
# Sets $global:LASTEXITCODE.
function Invoke-Fresh([string]$Name, [string[]]$ArgList = @()) {
    $path = Get-FreshEnv 'Path'
    $exe = Find-OnPath $Name $path
    if (-not $exe) { "(PATH de um terminal novo) $Name não encontrado"; $global:LASTEXITCODE = 9009; return }
    "(PATH de um terminal novo) $exe"
    $psi = [Diagnostics.ProcessStartInfo]::new($exe)
    foreach ($a in $ArgList) { $psi.ArgumentList.Add($a) }
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.StandardOutputEncoding = [Text.Encoding]::UTF8
    $psi.StandardErrorEncoding = [Text.Encoding]::UTF8
    $psi.Environment['Path'] = $path
    $mp = Get-FreshEnv 'PSModulePath'
    if ($mp) { $psi.Environment['PSModulePath'] = $mp }
    $p = [Diagnostics.Process]::Start($psi)
    $errTask = $p.StandardError.ReadToEndAsync()
    $out = $p.StandardOutput.ReadToEnd()
    $p.WaitForExit()
    $out -split "`r?`n" | Where-Object { $_ -ne '' }
    $errTask.Result -split "`r?`n" | Where-Object { $_ -ne '' }
    $global:LASTEXITCODE = $p.ExitCode
}

# `wingetrs list` output lines (read-only), used by checks.
function Get-WList([string]$Exe) {
    if (-not (Test-Path -LiteralPath $Exe)) { return @() }
    @(& $Exe list --color never 2>&1 | ForEach-Object { Remove-Ansi ([string]$_) })
}

function Get-RunDirs {
    @(Get-ChildItem -Force -Directory -LiteralPath (Join-Path $script:WData 'installers') -Filter 'run-*' -ErrorAction SilentlyContinue)
}

function Test-ArpKey([string]$Path) { Test-Path -LiteralPath $Path }

# True when the item and each folder above it (up to, not including, the drive root) are owned
# by SYSTEM/Administrators/TrustedInstaller and no other SID has a write-type allow ACE.
function Test-AdminOnlyPath([string]$Path) {
    $okSids = @('S-1-5-18', 'S-1-5-32-544', 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
    $writeMask = 0x2 -bor 0x4 -bor 0x10 -bor 0x100 -bor 0x10000 -bor 0x40000 -bor 0x80000 -bor 0x10000000 -bor 0x40000000
    $bad = [Collections.Generic.List[string]]::new()
    $cur = Get-Item -LiteralPath $Path -Force
    while ($cur -and $cur.FullName -ne [IO.Path]::GetPathRoot($cur.FullName)) {
        $acl = Get-Acl -LiteralPath $cur.FullName
        $owner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
        if ($okSids -notcontains $owner) { $bad.Add("$($cur.FullName): dono $($acl.Owner)") }
        foreach ($ace in $acl.Access) {
            if ($ace.AccessControlType -ne 'Allow') { continue }
            if (([int64]$ace.FileSystemRights -band $writeMask) -eq 0) { continue }
            $sid = try { $ace.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value } catch { [string]$ace.IdentityReference }
            if ($okSids -contains $sid) { continue }
            if ($sid -eq 'S-1-3-0' -and ($ace.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly)) { continue }
            $bad.Add("$($cur.FullName): $($ace.IdentityReference) $($ace.FileSystemRights)")
        }
        $cur = if ($cur -is [IO.FileInfo]) { $cur.Directory } else { $cur.Parent }
    }
    if ($bad.Count -eq 0) { return $true }
    $bad | ForEach-Object { Write-Host "    ACL: $_" -ForegroundColor Yellow }
    $false
}

# ---------------------------------------------------------------- section lifecycle

function Start-Section {
    param(
        [Parameter(Mandatory)][string]$Name,
        [Parameter(Mandatory)][string]$Title,
        [string]$OutDir, [string]$W, [string]$Wdev, [string]$StartAt,
        [switch]$DryRun, [switch]$NewRun
    )
    $run = if ($OutDir) { [IO.Path]::GetFullPath($OutDir) }
           elseif ($NewRun) { New-RunDir }
           else { $l = Get-LatestRunDir; if ($l) { $l } else { New-RunDir } }
    if (-not $Wdev) { $Wdev = Join-Path $script:RepoRoot 'target\release\wingetrs.exe' }
    $script:Ctx = [ordered]@{
        Section = $Name; Title = $Title; RunDir = $run; W = $W; Wdev = $Wdev
        DryRun = [bool]$DryRun; StartAt = $StartAt; Started = (-not $StartAt)
        Quit = $false; Results = [Collections.Generic.List[object]]::new()
        Log = Join-Path $run "section-$Name.log"; Began = Get-Date
    }
    if (-not $DryRun) {
        New-Item -ItemType Directory -Force -Path $run | Out-Null
        Add-Content -LiteralPath $script:Ctx.Log -Value "`n##### Seção $Name iniciada em $(Get-Date -Format 's')" -Encoding utf8
    }
    $mode = if ($DryRun) { 'SIMULAÇÃO (-DryRun): nada é executado nem gravado' } else { "pasta da execução: $run" }
    Write-Banner "Seção $Name — $Title`n$mode`nwingetrs (W): $W`nwingetrs dev (Wdev): $Wdev" 'Green'
    if (-not $DryRun) {
        Write-Host 'Em cada passo: Enter executa · p pula · q encerra a seção (o resultado é gravado).' -ForegroundColor Green
    }
}

function New-Record([hashtable]$S, [string[]]$Display) {
    [ordered]@{
        Id = $S.Id; Title = $S.Title; Command = ($Display -join "`n"); Status = ''; Exit = $null
        Checks = [Collections.Generic.List[object]]::new()
        Answers = [Collections.Generic.List[object]]::new()
        Notes = [Collections.Generic.List[object]]::new()
        Output = [Collections.Generic.List[string]]::new()
        Captured = $false
    }
}

function Get-StepDisplay([hashtable]$S) {
    $mode = if ($S.Mode) { $S.Mode } else { 'Capture' }
    switch ($mode) {
        'Admin' {
            $t = if ($S.AdminText -is [scriptblock]) { try { & $S.AdminText } catch { "<calculado na hora: $($_.Exception.Message)>" } } else { $S.AdminText }
            return @("[ADM] no terminal de administrador:") + @(([string]($t -join "`n")) -split "`r?`n")
        }
        'Parallel' { return @("Janela 1: $(Format-Cmd $S.Cmd)", "Este terminal: $(Format-Cmd $S.Cmd2)") }
        { $_ -in 'Script', 'Manual' } { if ($S.Show) { return @($S.Show) } else { return @('(sem comando)') } }
        default { return @(Format-Cmd $S.Cmd) }
    }
}

function Add-Check($Rec, [string]$Name, $Ok, [string]$Detail = '') {
    $Rec.Checks.Add([pscustomobject]@{ Name = $Name; Ok = $Ok; Detail = $Detail })
    $tag = if ($Ok -eq $true) { '[OK]     ' } elseif ($Ok -eq $false) { '[FALHOU] ' } else { '[INFO]   ' }
    $color = if ($Ok -eq $true) { 'Green' } elseif ($Ok -eq $false) { 'Red' } else { 'Gray' }
    Write-Host "  $tag$Name$(if ($Detail) { " — $Detail" })" -ForegroundColor $color
}

function Invoke-Native([string[]]$Cmd, [switch]$StdoutOnly, $Rec) {
    $exe = $Cmd[0]
    $rest = @($Cmd | Select-Object -Skip 1)
    $global:LASTEXITCODE = 0
    try {
        if ($StdoutOnly) {
            & $exe @rest | ForEach-Object { $line = Remove-Ansi ([string]$_); Write-Host $line; $Rec.Output.Add($line) }
        } else {
            & $exe @rest 2>&1 | ForEach-Object { $line = Remove-Ansi ([string]$_); Write-Host $line; $Rec.Output.Add($line) }
        }
        $LASTEXITCODE
    } catch {
        $Rec.Output.Add("erro ao iniciar: $($_.Exception.Message)")
        Write-Host "  erro ao iniciar: $($_.Exception.Message)" -ForegroundColor Red
        -1
    }
}

# Inherits this console (selectors, installer windows, TUI work). Output is not captured.
function Invoke-Attached([string[]]$Cmd) {
    $psi = [Diagnostics.ProcessStartInfo]::new($Cmd[0])
    foreach ($a in @($Cmd | Select-Object -Skip 1)) { $psi.ArgumentList.Add($a) }
    $psi.UseShellExecute = $false
    $psi.WorkingDirectory = $script:RepoRoot
    $p = [Diagnostics.Process]::Start($psi)
    $p.WaitForExit()
    $p.ExitCode
}

# Starts the command in a new console window (pwsh -NoExit). Ctrl+C there reaches only that
# window. The exit code is written to $CodeFile when the command ends (also after Ctrl+C).
function Start-StepWindow([string[]]$Cmd, [string]$Title, [string]$CodeFile, [switch]$Wait) {
    $argsLit = (@($Cmd | Select-Object -Skip 1) | ForEach-Object { Quote-Ps $_ }) -join ', '
    $text = @"
`$host.UI.RawUI.WindowTitle = $(Quote-Ps "wingetrs prova: $Title")
Write-Host $(Quote-Ps (Format-Cmd $Cmd)) -ForegroundColor Cyan
`$psi = [System.Diagnostics.ProcessStartInfo]::new($(Quote-Ps $Cmd[0]))
foreach (`$a in @($argsLit)) { `$psi.ArgumentList.Add(`$a) }
`$psi.UseShellExecute = `$false
`$p = [System.Diagnostics.Process]::Start(`$psi)
try { `$p.WaitForExit() } finally {
    `$p.WaitForExit()
    Set-Content -LiteralPath $(Quote-Ps $CodeFile) -Value `$p.ExitCode
    Write-Host ''
    Write-Host ('Código de saída: {0}. Feche esta janela (digite exit) para voltar ao roteiro.' -f `$p.ExitCode) -ForegroundColor Yellow
}
"@
    Remove-Item -LiteralPath $CodeFile -ErrorAction SilentlyContinue
    $enc = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($text))
    Start-Process -FilePath (Get-PwshPath) -ArgumentList @('-NoProfile', '-NoExit', '-EncodedCommand', $enc) `
        -WorkingDirectory $script:RepoRoot -PassThru -Wait:$Wait
}

function Read-CodeFile([string]$CodeFile) {
    if (Test-Path -LiteralPath $CodeFile) { [int](Get-Content -LiteralPath $CodeFile -Raw).Trim() } else { $null }
}

function Start-RunWatch {
    $ev = [Threading.ManualResetEventSlim]::new($false)
    $dir = Join-Path $script:WData 'installers'
    $job = Start-ThreadJob -ArgumentList $dir, $ev -ScriptBlock {
        param($dir, $ev)
        $seen = @{}
        do {
            foreach ($d in @(Get-ChildItem -Force -Directory -LiteralPath $dir -Filter 'run-*' -ErrorAction SilentlyContinue)) {
                $files = @(Get-ChildItem -Force -Recurse -File -LiteralPath $d.FullName -ErrorAction SilentlyContinue | ForEach-Object Name)
                if (-not $seen.ContainsKey($d.Name) -or $files.Count -gt $seen[$d.Name].Count) { $seen[$d.Name] = $files }
            }
        } while (-not $ev.Wait(250))
        $seen
    }
    @{ Job = $job; Event = $ev }
}

function Stop-RunWatch($W) {
    $W.Event.Set()
    $r = Receive-Job -Job $W.Job -Wait -AutoRemoveJob
    $W.Event.Dispose()
    if ($r -is [hashtable]) { $r } else { @{} }
}

$script:StepKeys = @('Id', 'Title', 'Mode', 'Cmd', 'Cmd2', 'Between', 'Run', 'Show', 'AdminText', 'Instructions', 'Uac',
    'Destructive', 'When', 'WhenWhy', 'ExpectExit', 'Expect', 'NotExpect', 'Checks', 'Ask', 'Notes', 'Watch', 'After')

# A bug in a step (or in this runner) is recorded as a failed step and the section goes on.
function Invoke-Step([hashtable]$Spec) {
    if ($script:Ctx.Quit) { return }
    $before = $script:Ctx.Results.Count
    try {
        Invoke-StepCore $Spec
    } catch {
        Write-Host "  ERRO DO ROTEIRO no passo $($Spec['Id']): $($_.Exception.Message) (linha $($_.InvocationInfo.ScriptLineNumber))" -ForegroundColor Red
        $rec = New-Record @{ Id = $Spec['Id']; Title = $Spec['Title'] } @('(erro do roteiro)')
        $rec.Status = 'erro do roteiro'
        $rec.Checks.Add([pscustomobject]@{ Name = 'roteiro sem erro'; Ok = $false; Detail = $_.Exception.Message })
        if ($script:Ctx.Results.Count -eq $before) { $script:Ctx.Results.Add($rec) } else { $script:Ctx.Results[-1].Checks.Add($rec.Checks[0]) }
        if (-not $script:Ctx.DryRun) { try { Write-StepLog $rec } catch { } }
    }
}

function Invoke-StepCore([hashtable]$Spec) {
    $c = $script:Ctx
    # Strict mode rejects missing hashtable keys read as properties: fill them with $null.
    $unknown = @($Spec.Keys | Where-Object { $script:StepKeys -notcontains $_ })
    if ($unknown) { throw "passo $($Spec['Id']): chave desconhecida: $($unknown -join ', ')" }
    $S = @{}
    foreach ($k in $script:StepKeys) { $S[$k] = $Spec[$k] }
    $mode = if ($S.Mode) { $S.Mode } else { 'Capture' }
    $display = Get-StepDisplay $S
    $rec = New-Record $S $display

    if (-not $c.Started) {
        if ($S.Id -eq $c.StartAt) { $c.Started = $true } else { $rec.Status = 'pulado (antes de -StartAt)'; $c.Results.Add($rec); return }
    }

    Write-Banner "[$($c.Section) · $($S.Id)] $($S.Title)"
    if ($S.Uac) { Write-Host "  !!! [UAC] $($S.Uac)" -ForegroundColor White -BackgroundColor DarkRed }
    if ($S.Destructive) { Write-Host '  !!! Passo que remove ou altera algo na máquina: pede confirmação explícita.' -ForegroundColor White -BackgroundColor DarkMagenta }
    foreach ($i in @($S.Instructions)) { if ($i) { Write-Host "  * $i" -ForegroundColor Yellow } }
    foreach ($l in $display) { Write-Host "  > $l" -ForegroundColor White }

    if ($c.DryRun) {
        Write-Host "  modo: $mode" -ForegroundColor DarkGray
        if ($S.When) { Write-Host "  condição: só roda se $($S.WhenWhy)" -ForegroundColor DarkGray }
        if ($null -ne $S.ExpectExit) { Write-Host "  código de saída esperado: $($S.ExpectExit)" -ForegroundColor DarkGray }
        foreach ($e in @($S.Expect)) { if ($e) { Write-Host "  espera: $e" -ForegroundColor DarkGray } }
        foreach ($e in @($S.NotExpect)) { if ($e) { Write-Host "  não espera: $e" -ForegroundColor DarkGray } }
        foreach ($k in @($S.Checks)) { if ($k) { Write-Host "  verifica: $($k.Name)" -ForegroundColor DarkGray } }
        if ($S.Watch) { Write-Host '  verifica: pastas installers\run-* durante e depois' -ForegroundColor DarkGray }
        foreach ($q in @($S.Ask)) { if ($q) { Write-Host "  pergunta: $q (s/n)" -ForegroundColor DarkGray } }
        foreach ($q in @($S.Notes)) { if ($q) { Write-Host "  anotar: $q" -ForegroundColor DarkGray } }
        $rec.Status = 'simulado'
        $c.Results.Add($rec)
        return
    }

    if ($S.When) {
        $go = try { [bool](& $S.When) } catch { Write-Host "  condição falhou: $($_.Exception.Message)" -ForegroundColor Red; $false }
        if (-not $go) {
            Write-Host "  pulado automaticamente: $($S.WhenWhy) não se aplica" -ForegroundColor DarkYellow
            $rec.Status = "pulado (condição: $($S.WhenWhy))"
            $c.Results.Add($rec)
            return
        }
    }

    $ran = $false
    $code = $null
    if ($mode -eq 'Admin') {
        $text = if ($S.AdminText -is [scriptblock]) { (& $S.AdminText) -join "`n" } else { [string]$S.AdminText }
        $r = Read-Choice '  [Enter] já rodei no terminal de administrador · s rodar agora por UAC (Start-Process -Verb RunAs) · p pular · q sair' @('', 's', 'p', 'q')
        if ($r -eq 'q') { $c.Quit = $true; $rec.Status = 'interrompido'; $c.Results.Add($rec); return }
        if ($r -eq 'p') { $rec.Status = 'pulado'; $c.Results.Add($rec); Write-StepLog $rec; return }
        if ($r -eq 's') {
            $outFile = Join-Path $c.RunDir "$($S.Id)-admin.txt"
            $wrapped = "& {`n$text`n} *>&1 | Tee-Object -FilePath $(Quote-Ps $outFile)`nRead-Host 'Enter para fechar esta janela de administrador'"
            $enc = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($wrapped))
            Write-Host '  abrindo PowerShell elevado (UAC)...' -ForegroundColor Yellow
            try {
                Start-Process -FilePath (Get-PwshPath) -Verb RunAs -Wait -ArgumentList @('-NoProfile', '-EncodedCommand', $enc)
                if (Test-Path -LiteralPath $outFile) { Get-Content -LiteralPath $outFile | ForEach-Object { $rec.Output.Add($_) }; $rec.Captured = $true }
                $rec.Status = 'executado (UAC pelo roteiro)'
            } catch {
                $rec.Output.Add("UAC recusado ou falhou: $($_.Exception.Message)")
                $rec.Status = 'falhou ao elevar'
            }
        } else {
            $rec.Status = 'executado pelo usuário (terminal de administrador)'
        }
        $ran = $true
    } else {
        $prompt = if ($mode -eq 'Manual') { '  [Enter] quando terminar · p pular · q sair' } else { '  [Enter] executar · p pular · q sair' }
        $r = Read-Choice $prompt @('', 'p', 'q')
        if ($r -eq 'q') { $c.Quit = $true; $rec.Status = 'interrompido'; $c.Results.Add($rec); return }
        if ($r -eq 'p') { $rec.Status = 'pulado'; $c.Results.Add($rec); Write-StepLog $rec; return }
        if ($S.Destructive) {
            $y = Read-Choice '  Confirma? digite s para executar (Enter = pular)' @('', 's')
            if ($y -ne 's') { $rec.Status = 'pulado (não confirmado)'; $c.Results.Add($rec); Write-StepLog $rec; return }
        }
        $watch = $null
        if ($S.Watch) { $watch = Start-RunWatch }
        try {
            switch ($mode) {
                'Capture' { $code = Invoke-Native $S.Cmd -Rec $rec; $rec.Captured = $true }
                'Stdout' { $code = Invoke-Native $S.Cmd -StdoutOnly -Rec $rec; $rec.Captured = $true }
                'Attached' { $code = Invoke-Attached $S.Cmd }
                'Window' {
                    $cf = Join-Path $c.RunDir "$($S.Id).exitcode"
                    Write-Host '  abrindo nova janela; siga as instruções lá e feche-a ao terminar...' -ForegroundColor Yellow
                    Start-StepWindow $S.Cmd $S.Title $cf -Wait | Out-Null
                    $code = Read-CodeFile $cf
                }
                'Parallel' {
                    $cf = Join-Path $c.RunDir "$($S.Id)-janela1.exitcode"
                    $proc = Start-StepWindow $S.Cmd $S.Title $cf
                    Read-Choice "  $($S.Between) — Enter para rodar o comando deste terminal" @('') | Out-Null
                    $code = Invoke-Native $S.Cmd2 -StdoutOnly -Rec $rec
                    $rec.Captured = $true
                    Write-Host '  aguardando a janela 1 terminar (feche-a com exit depois de ler)...' -ForegroundColor Yellow
                    $proc.WaitForExit()
                    $c1 = Read-CodeFile $cf
                    $rec.Output.Add("janela 1: código de saída $c1")
                    Add-Check $rec "janela 1 terminou com 0" ($c1 -eq 0) "código $c1"
                }
                'Script' {
                    $global:LASTEXITCODE = 0
                    try {
                        & $S.Run 2>&1 | ForEach-Object { $line = Remove-Ansi ([string]$_); Write-Host $line; $rec.Output.Add($line) }
                    } catch {
                        $rec.Output.Add("erro: $($_.Exception.Message)")
                        Write-Host "  erro: $($_.Exception.Message)" -ForegroundColor Red
                        $global:LASTEXITCODE = -1
                    }
                    $code = $LASTEXITCODE
                    $rec.Captured = $true
                }
                'Manual' { }
                default { throw "modo desconhecido: $mode" }
            }
        } finally {
            if ($watch) { $seen = Stop-RunWatch $watch }
        }
        $ran = $true
        $rec.Status = 'executado'
        $rec.Exit = $code
        if ($null -ne $code) { Write-Host "  código de saída: $code" -ForegroundColor DarkGray }
        if ($watch) {
            $names = @($seen.Keys)
            Add-Check $rec 'apareceu pasta installers\run-* durante a execução' ($names.Count -gt 0) ($names -join ', ')
            foreach ($n in $names) {
                $files = @($seen[$n])
                Add-Check $rec "$n só com a cópia do instalador, sem .log" (($files.Count -le 1) -and -not ($files | Where-Object { $_ -like '*.log' })) ($files -join ', ')
            }
            $left = @(Get-RunDirs)
            Add-Check $rec 'nenhuma pasta run-* sobrou' ($left.Count -eq 0) (($left | ForEach-Object Name) -join ', ')
        }
    }

    if ($ran) { Test-StepResult $S $rec $mode $code }
    if ($S.After) { try { & $S.After $rec } catch { Write-Host "  After: $($_.Exception.Message)" -ForegroundColor Red } }
    $c.Results.Add($rec)
    Write-StepLog $rec
}

function Test-StepResult([hashtable]$S, $Rec, [string]$Mode, $Code) {
    $expectExit = $S.ExpectExit
    if ($null -eq $expectExit -and $Mode -in 'Capture', 'Stdout', 'Attached', 'Window', 'Parallel') { $expectExit = 0 }
    $ee = if ($null -eq $expectExit) { '' } else { [string]$expectExit }
    if ($ee -and $ee -ne 'any' -and $null -ne $Code) {
        if ($ee -eq 'nonzero') { Add-Check $Rec 'código de saída diferente de 0' ($Code -ne 0) "código $Code" }
        else { Add-Check $Rec "código de saída $ee" ($Code -eq [int]$ee) "código $Code" }
    } elseif ($ee -and $ee -ne 'any' -and $null -eq $Code -and $Mode -in 'Window', 'Parallel') {
        Add-Check $Rec "código de saída $expectExit" $false 'janela fechada antes do fim (sem código)'
    }
    $text = $Rec.Output -join "`n"
    foreach ($e in @($S.Expect)) {
        if (-not $e) { continue }
        if ($Rec.Captured) { Add-Check $Rec "saída contém: $e" ($text.Contains($e)) }
        else { Ask-User $Rec "Apareceu na tela: '$e'?" }
    }
    foreach ($e in @($S.NotExpect)) {
        if (-not $e) { continue }
        if ($Rec.Captured) { Add-Check $Rec "saída NÃO contém: $e" (-not $text.Contains($e)) }
        else { Ask-User $Rec "A tela NÃO mostrou '$e'?" }
    }
    foreach ($k in @($S.Checks)) {
        if (-not $k) { continue }
        try {
            $v = & $k.Test
            if ($v -is [bool]) { Add-Check $Rec $k.Name $v }
            else { Add-Check $Rec $k.Name 'info' (@($v) -join ' | ') }
        } catch { Add-Check $Rec $k.Name $false "erro: $($_.Exception.Message)" }
    }
    foreach ($q in @($S.Ask)) { if ($q) { Ask-User $Rec $q } }
    foreach ($q in @($S.Notes)) {
        if (-not $q) { continue }
        $a = Read-Host "  anote: $q (Enter = nada)"
        $Rec.Notes.Add([pscustomobject]@{ Q = $q; A = $a })
    }
}

function Ask-User($Rec, [string]$Question) {
    $a = Read-Choice "  $Question (s/n, Enter = sem resposta)" @('s', 'n', '')
    $Rec.Answers.Add([pscustomobject]@{ Q = $Question; A = $(if ($a) { $a } else { '-' }) })
}

function Write-StepLog($Rec) {
    $lines = @("", "----- [$($script:Ctx.Section) · $($Rec.Id)] $($Rec.Title)  ($(Get-Date -Format 's'))", $Rec.Command, "status: $($Rec.Status); saída: $($Rec.Exit)")
    $lines += @($Rec.Output)
    Add-Content -LiteralPath $script:Ctx.Log -Value $lines -Encoding utf8
}

function Complete-Section {
    $c = $script:Ctx
    if (-not $c) { return }
    $checks = @($c.Results | ForEach-Object { $_.Checks } | Where-Object { $_ })
    $fails = @($checks | Where-Object { $_.Ok -eq $false })
    $nos = @($c.Results | ForEach-Object { $_.Answers } | Where-Object { $_ -and $_.A -eq 'n' })
    Write-Banner ("Seção $($c.Section): {0} passos · verificações {1} OK / {2} falhas · respostas 'n': {3}" -f `
            $c.Results.Count, @($checks | Where-Object { $_.Ok -eq $true }).Count, $fails.Count, $nos.Count) 'Green'
    if ($c.DryRun) { Write-Host 'Simulação: nada foi executado nem gravado.' -ForegroundColor Green; return }

    $md = [Collections.Generic.List[string]]::new()
    $md.Add("# Prova ao vivo — seção $($c.Section): $($c.Title)")
    $md.Add('')
    $md.Add("- Início: $($c.Began.ToString('s')) · fim: $((Get-Date).ToString('s'))")
    $md.Add("- wingetrs: ``$($c.W)`` · dev: ``$($c.Wdev)``")
    $md.Add("- Verificações automáticas: $(@($checks | Where-Object { $_.Ok -eq $true }).Count) OK, $($fails.Count) falhas; respostas 'n': $($nos.Count)")
    $md.Add("- Log completo: ``$($c.Log)``")
    $md.Add('')
    $md.Add('| Passo | Título | Status | Saída | Auto | Usuário |')
    $md.Add('|---|---|---|---|---|---|')
    foreach ($r in $c.Results) {
        $ok = @($r.Checks | Where-Object { $_.Ok -eq $true }).Count
        $bad = @($r.Checks | Where-Object { $_.Ok -eq $false }).Count
        $ans = @($r.Answers | ForEach-Object A) -join ''
        $md.Add("| $($r.Id) | $($r.Title -replace '\|', '/') | $($r.Status) | $($r.Exit) | $ok OK / $bad falhas | $ans |")
    }
    foreach ($r in $c.Results) {
        $md.Add('')
        $md.Add("## $($r.Id) — $($r.Title)")
        $md.Add('')
        $md.Add('```powershell')
        $md.Add($r.Command)
        $md.Add('```')
        $md.Add("- Status: $($r.Status)$(if ($null -ne $r.Exit) { " · código de saída $($r.Exit)" })")
        foreach ($k in $r.Checks) {
            $tag = if ($k.Ok -eq $true) { 'OK' } elseif ($k.Ok -eq $false) { '**FALHOU**' } else { 'info' }
            $md.Add("- [$tag] $($k.Name)$(if ($k.Detail) { " — $($k.Detail)" })")
        }
        foreach ($a in $r.Answers) { $md.Add("- Usuário: $($a.Q) → **$($a.A)**") }
        foreach ($n in $r.Notes) { $md.Add("- Anotação: $($n.Q) → $($n.A)") }
        if ($r.Output.Count -gt 0) {
            $tail = @($r.Output | Select-Object -Last 25)
            $md.Add('')
            $md.Add("<details><summary>Trecho do log ($($tail.Count) de $($r.Output.Count) linhas)</summary>")
            $md.Add('')
            $md.Add('```text')
            $tail | ForEach-Object { $md.Add($_) }
            $md.Add('```')
            $md.Add('</details>')
        } elseif ($r.Status -like 'executado*') {
            $md.Add('- Saída: no terminal ou na janela (não capturada)')
        }
    }
    $f = Join-Path $c.RunDir "section-$($c.Section).md"
    $n = 2
    while (Test-Path -LiteralPath $f) { $f = Join-Path $c.RunDir "section-$($c.Section)-$n.md"; $n++ }
    $md | Set-Content -LiteralPath $f -Encoding utf8
    Write-Host "Resultado: $f" -ForegroundColor Green
}

function Get-SectionExit {
    if ($script:Ctx.Quit) { return 3 }
    $fails = @($script:Ctx.Results | ForEach-Object { $_.Checks } | Where-Object { $_ -and $_.Ok -eq $false })
    if ($fails.Count -gt 0) { 1 } else { 0 }
}
