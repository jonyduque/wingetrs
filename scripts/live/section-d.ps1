#Requires -Version 7.0
<#
.SYNOPSIS
    Live proof, section D (UAC block, everything from C:\Program Files\wingetrs): refused UAC,
    four machine installers (one UAC each), TUI removals with the helper (one UAC), log check and
    restoring the test orphans from wingetrs' .reg backups.

.EXAMPLE
    pwsh -File scripts/live/section-d.ps1 -DryRun
#>
[CmdletBinding()]
param(
    [string]$OutDir,
    [string]$W = 'C:\Program Files\wingetrs\wingetrs.exe',
    [string]$Wdev,
    [string]$StartAt,
    [switch]$DryRun
)
. (Join-Path $PSScriptRoot '_common.ps1')
Start-Section -Name 'D' -Title 'Bloco de UAC (de C:\Program Files\wingetrs)' -OutDir $OutDir -W $W -Wdev $Wdev -StartAt $StartAt -DryRun:$DryRun
$P = $Ctx.RunDir
$orfHkcu = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan'
$orfHklm = 'HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan'
$orfHkcuPs = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan'
$orfHklmPs = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan'
$backups = Join-Path $WData 'backups'
$staging = Join-Path (Split-Path $W) 'staging'

function Get-OrphanBackup([string]$Prefix) {
    Get-ChildItem -LiteralPath $backups -Recurse -Filter '*wingetrs-test-orphan*.reg' -ErrorAction SilentlyContinue |
        Where-Object Name -like "$Prefix*" | Sort-Object LastWriteTime | Select-Object -Last 1
}

function Get-JobLines {
    $log = Get-State 'log'; $start = Get-State 'logStart'
    if (-not $log) { $log = Join-Path $WData ("log\wingetrs.log.{0}" -f (Get-Date -Format 'yyyy-MM-dd')); $start = 0 }
    $files = @(Get-ChildItem -LiteralPath (Join-Path $WData 'log') -Filter 'wingetrs.log.*' -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -eq $log -or $_.Name -gt (Split-Path $log -Leaf) } | Sort-Object Name)
    foreach ($f in $files) {
        $skip = if ($f.FullName -eq $log) { [int]$start } else { 0 }
        Get-Content -LiteralPath $f.FullName | Select-Object -Skip $skip | Select-String 'job\.(start|end)' |
            ForEach-Object { $_.Line.Substring(0, [Math]::Min(300, $_.Line.Length)) }
    }
}

try {
    Invoke-Step @{
        Id = 'D0'; Title = 'O wingetrs de Program Files existe e é o mesmo do target\release'; Mode = 'Script'
        Show = @("Test-Path $(Quote-Ps $W)")
        Run = { if (Test-Path -LiteralPath $W) { "ok: $W" } else { "ausente: $W (rode o passo 0.2)" } }
        Checks = @(
            @{ Name = "existe $W"; Test = { Test-Path -LiteralPath $W } }
            @{ Name = 'hash igual ao de target\release'; Test = { (Get-FileHash -LiteralPath $Ctx.Wdev).Hash -eq (Get-FileHash -LiteralPath $W).Hash } }
        )
    }
    Invoke-Step @{
        Id = 'D1'; Title = '[UAC recusado] PuTTY pelo CLI'; Mode = 'Attached'
        Cmd = @($W, 'install', 'PuTTY.PuTTY')
        Instructions = @('No seletor, aperte s. Quando o UAC aparecer, clique NÃO.')
        Uac = 'Abre UAC: clique NÃO (recusa de propósito).'
        ExpectExit = 'nonzero'
        Expect = @('Aguardando autorização para executar como administrador', 'Pede permissão de administrador para o Windows Installer', 'Permissão de administrador recusada.')
        Checks = @(
            @{ Name = 'list sem PuTTY'; Test = { -not (Get-WList $W | Where-Object { $_ -match 'PuTTY' }) } }
            @{ Name = 'nenhuma pasta installers\run-* sobrando'; Test = { @(Get-RunDirs).Count -eq 0 } }
        )
        Notes = @('o UAC veio em primeiro plano ou piscou na barra de tarefas?')
    }
    $d2 = @(
        @{ Id = 'D2a'; Pkg = 'PuTTY.PuTTY'; Who = 'o Windows Installer'; Win = 'janela de progresso sem perguntas (/passive)'; When = { -not (Get-State 'puttyInstalledInC1') }; WhenWhy = 'o PuTTY não foi instalado no C1' }
        @{ Id = 'D2b'; Pkg = 'Rawns.Reddit-Wallpaper-Changer'; Who = 'o Windows Installer'; Win = 'janela de progresso sem perguntas (/passive)'; Note = 'hash divergente? (URL de tag móvel: anote e pule). Não abra o app.' }
        @{ Id = 'D2c'; Pkg = 'Microsoft.DotNet.AspNetCore.8'; Who = 'o editor do arquivo'; Win = 'janela de progresso sem perguntas (/passive)' }
        @{ Id = 'D2d'; Pkg = 'PeterPawlowski.foobar2000'; Who = 'o editor do arquivo'; Win = 'nenhuma janela (/S)' }
    )
    foreach ($i in $d2) {
        $step = @{
            Id = $i.Id; Title = "[UAC] Instalar $($i.Pkg) (máquina)"; Mode = 'Attached'; Watch = $true
            Cmd = @($W, 'install', $i.Pkg)
            Instructions = @('No seletor, aperte s. Quando o UAC aparecer, ACEITE.')
            Uac = "Abre UAC: aceite. Ele deve nomear $($i.Who)."
            Expect = @('Aguardando autorização para executar como administrador')
            Ask = @("Modo padrão: $($i.Win)?")
            Notes = @('nome que aparece no UAC') + @($(if ($i["Note"]) { $i["Note"] }))
        }
        if ($i["When"]) { $step.When = $i["When"]; $step.WhenWhy = $i["WhenWhy"] }
        Invoke-Step $step
    }
    Invoke-Step @{
        Id = 'D2e'; Title = 'Os quatro instalados'
        Cmd = @($W, 'list', '--color', 'never')
        Expect = @('PuTTY', 'Reddit', 'ASP.NET', 'foobar')
    }
    Invoke-Step @{
        Id = 'D3'; Title = '[UAC x1] TUI: remoções HKLM, órfãos e Kindle com o auxiliar'; Mode = 'Manual'
        Show = @("& $(Quote-Ps $W)")
        Uac = 'A primeira remoção abre o ÚNICO UAC do auxiliar: aceite. As outras não devem abrir UAC.'
        Instructions = @(
            'Abra a TUI em outro terminal com o comando abaixo; volte aqui depois de sair dela.'
            'Aba Instalados, remova (u) nesta ordem: PuTTY, Reddit Wallpaper Changer, Microsoft ASP.NET Core Runtime 8.0, foobar2000.'
            'Órfãos: filtre por "wingetrs test"; em cada "wingetrs test orphan", "Remover registro órfão".'
            'Kindle: "Amazon Kindle", u, leia o modal e CANCELE (Esc).'
            'Saia com q.'
            'Alternativa CLI: & $W uninstall PuTTY.PuTTY etc. (um UAC por comando); órfãos: & $W cleanup --orphans, marque só os dois "wingetrs test orphan", Enter, s.')
        Checks = @(
            @{ Name = 'list sem PuTTY, Reddit, ASP.NET Core 8 e foobar'; Test = { -not (Get-WList $W | Where-Object { $_ -match 'PuTTY|Reddit|ASP\.NET Core.*8\.0\.32|foobar' }) } }
            @{ Name = 'órfão HKCU removido'; Test = { -not (Test-Path -LiteralPath $orfHkcuPs) } }
            @{ Name = 'órfão HKLM removido'; Test = { -not (Test-Path -LiteralPath $orfHklmPs) } }
            @{ Name = 'Amazon Kindle continua no list'; Test = { [bool](Get-WList $W | Where-Object { $_ -match 'Kindle' }) } }
            @{ Name = 'nenhum wingetrs.exe vivo'; Test = { -not (Get-Process wingetrs -ErrorAction SilentlyContinue) } }
        )
        Ask = @(
            'Na primeira remoção a barra mostrou "Aguardando autorização para executar como administrador" e depois "permissão de administrador concedida"?'
            'As outras três remoções e os órfãos NÃO abriram UAC novo?'
            'Cada remoção mostrou o plano em passos no modal?'
            'Órfãos: HKCU removido no processo, HKLM pelo auxiliar, toast "Registro órfão de wingetrs test orphan ... removido (cópia em ...)"?'
            'Kindle: o modal disse qual instalação e mostrou "Outra instalação de Amazon Kindle usa as mesmas pastas", "Pasta compartilhada" e "Tamanho da cópia"; nada removido?')
        Notes = @('o UAC do auxiliar apareceu ou piscou?', 'quais remoções abriram janela de desinstalador?', 'algum desinstalador deixou chave no ARP?')
    }
    Invoke-Step @{
        Id = 'D4'; Title = 'Log (job.start/job.end), pastas run-* e staging'; Mode = 'Script'
        Show = @('Get-Content $log | Select-Object -Skip $logStart | Select-String ''job\.(start|end)''',
            "Get-ChildItem -Force `"`$env:LOCALAPPDATA\wingetrs\installers`" -Filter 'run-*'", "Get-ChildItem -Force $(Quote-Ps $staging)")
        Run = {
            Get-JobLines
            "run-*: $(@(Get-RunDirs | ForEach-Object Name) -join ', ')"
            "staging: $(@(Get-ChildItem -Force -LiteralPath $staging -ErrorAction SilentlyContinue | ForEach-Object Name) -join ', ')"
        }
        Checks = @(
            @{ Name = 'número de job.start = número de job.end'; Test = {
                    $lines = @(Get-JobLines)
                    @($lines | Where-Object { $_ -match 'job\.start' }).Count -eq @($lines | Where-Object { $_ -match 'job\.end' }).Count } }
            @{ Name = 'nenhuma pasta run-* sobrando'; Test = { @(Get-RunDirs).Count -eq 0 } }
            @{ Name = 'staging vazio ou inexistente'; Test = { -not (Get-ChildItem -Force -LiteralPath $staging -ErrorAction SilentlyContinue) } }
        )
        Ask = @('Os 4 instaladores de D2 e as remoções HKLM de D3 com elevated=true, os de usuário com elevated=false, e agreements= vazio em todos?')
    }
    Invoke-Step @{
        Id = 'D5a'; Title = 'Restaurar o órfão HKCU com o .reg do wingetrs'; Mode = 'Script'; Destructive = $true
        Show = @('$bk = Get-ChildItem "$env:LOCALAPPDATA\wingetrs\backups" -Recurse -Filter ''*wingetrs-test-orphan*.reg'' | Sort-Object LastWriteTime | Select-Object -Last 2',
            'reg import <hkcu-*.reg mais novo>', "reg export '$orfHkcu' $(Quote-Ps (Join-Path $P 'orf-hkcu-depois.reg')) /y",
            'Compare-Object orf-hkcu-antes.reg orf-hkcu-depois.reg')
        Run = {
            Get-ChildItem -LiteralPath $backups -Recurse -Filter '*wingetrs-test-orphan*.reg' -ErrorAction SilentlyContinue |
                Sort-Object LastWriteTime | Select-Object -Last 4 | ForEach-Object { "backup: $($_.FullName)" }
            $h = Get-OrphanBackup 'hkcu-'
            if (-not $h) { 'nenhum backup hkcu-*wingetrs-test-orphan*.reg'; $global:LASTEXITCODE = 1; return }
            & reg.exe import $h.FullName 2>&1
            & reg.exe export $orfHkcu (Join-Path $P 'orf-hkcu-depois.reg') /y 2>&1
            $d = @(Compare-Object (Get-Content -LiteralPath (Join-Path $P 'orf-hkcu-antes.reg')) (Get-Content -LiteralPath (Join-Path $P 'orf-hkcu-depois.reg')))
            "diferenças: $($d.Count)"
            $d | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }
            $global:LASTEXITCODE = $d.Count
        }
        ExpectExit = 0
    }
    Invoke-Step @{
        Id = 'D5b'; Title = 'Restaurar o órfão HKLM (import em HKLM exige elevação)'; Mode = 'Admin'
        AdminText = {
            $h = Get-OrphanBackup 'hklm64-'
            $src = if ($h) { $h.FullName } else { '<caminho do hklm64-wingetrs-test-orphan.reg em backups>' }
            @(
                "reg import $(Quote-Ps $src)"
                "reg export $(Quote-Ps $orfHklm) $(Quote-Ps (Join-Path $P 'orf-hklm-depois.reg')) /reg:64 /y"
            )
        }
        Checks = @(
            @{ Name = 'chave HKLM de volta'; Test = { Test-Path -LiteralPath $orfHklmPs } }
            @{ Name = 'Compare-Object orf-hklm-antes/depois vazio'; Test = {
                    $a = Join-Path $P 'orf-hklm-antes.reg'; $b = Join-Path $P 'orf-hklm-depois.reg'
                    (Test-Path -LiteralPath $a) -and (Test-Path -LiteralPath $b) -and -not (Compare-Object (Get-Content -LiteralPath $a) (Get-Content -LiteralPath $b)) } }
        )
    }
} finally {
    Complete-Section
}
exit (Get-SectionExit)
