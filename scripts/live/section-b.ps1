#Requires -Version 7.0
<#
.SYNOPSIS
    Live proof, section B (2c): Scoop scripts, innosetup, `if ($global)` blocks and removals. No UAC.

.EXAMPLE
    pwsh -File scripts/live/section-b.ps1 -DryRun
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
Start-Section -Name 'B' -Title '2c — scripts do Scoop, innosetup, if ($global) (sem UAC)' -OutDir $OutDir -W $W -Wdev $Wdev -StartAt $StartAt -DryRun:$DryRun
$P = $Ctx.RunDir
$apps = Join-Path $WData 'apps'
$zipBackup = Join-Path $P 'hkcu-7zip-antes.reg'
$zipAfter = Join-Path $P 'hkcu-7zip-depois.reg'
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\wingetrs Apps'

try {
    # B1 pester
    Invoke-Step @{
        Id = 'B1a'; Title = 'pester: plano (dry-run)'
        Cmd = @($W, 'install', 'main/pester', '--dry-run')
        Expect = @('pester.6.2.0.nupkg', 'só a subpasta tools', 'Ligar módulo PowerShell', 'PSModulePath do usuário')
    }
    Invoke-Step @{
        Id = 'B1b'; Title = 'pester: instalar'; Mode = 'Stdout'
        Cmd = @($W, 'install', 'main/pester', '--yes')
    }
    Invoke-Step @{
        Id = 'B1c'; Title = 'pester: pwsh novo lista o módulo'; Mode = 'Script'
        Show = @('pwsh -NoProfile -Command "Get-Module -ListAvailable Pester | Select-Object Name, Version, Path"   # ambiente de um terminal novo')
        Run = { Invoke-Fresh 'pwsh.exe' @('-NoProfile', '-Command', 'Get-Module -ListAvailable Pester | Format-List Name, Version, Path') }
        ExpectExit = 0; Expect = @('6.2.0', 'wingetrs\modules\Pester')
    }

    # B2 xming
    Invoke-Step @{
        Id = 'B2a'; Title = 'xming: plano (dry-run)'
        Cmd = @($W, 'install', 'main/xming', '--dry-run')
        Expect = @('Instalar innounp 2.71.2', 'Instalar xming 6.9.0.31', 'Xming-mesa-6-9-0-31-setup.exe (Inno Setup, com o innounp)', 'md5 e580debb')
    }
    Invoke-Step @{
        Id = 'B2b'; Title = 'xming: instalar'; Mode = 'Stdout'
        Cmd = @($W, 'install', 'main/xming', '--yes')
        Checks = @(@{ Name = 'shims\XLaunch.exe existe'; Test = { Test-Path -LiteralPath (Join-Path $WData 'shims\XLaunch.exe') } })
    }
    Invoke-Step @{
        Id = 'B2c'; Title = 'innounp aparece no list?'
        Cmd = @($W, 'list', '--color', 'never')
        Checks = @(@{ Name = 'linhas do list com innounp/xming'; Test = { @(Get-WList $W | Where-Object { $_ -match 'innounp|xming' }) -join ' / ' } })
        Notes = @('o innounp aparece no list?')
    }

    # B3 yori
    Invoke-Step @{
        Id = 'B3a'; Title = 'yori: plano (dry-run)'
        Cmd = @($W, 'install', 'main/yori', '--dry-run')
        Expect = @('YoriInit.d', 'hardlink', 'Yori.lnk', 'Rodar o script post_install do manifesto (PowerShell)',
            'Script de terceiro (post_install), roda com os seus privilégios', 'Análise: pede confirmação', 'Remove-Item "$dir\pkginfo.ini" -Force')
    }
    Invoke-Step @{
        Id = 'B3b'; Title = 'yori: instalar com --allow-scripts (seletor: s)'; Mode = 'Attached'
        Cmd = @($W, 'install', 'main/yori', '--allow-scripts')
        Instructions = @('No seletor, aperte s.')
        Checks = @(
            @{ Name = 'pkginfo.ini não existe (o hook rodou)'; Test = { -not (Test-Path -LiteralPath (Join-Path $apps 'yori\current\pkginfo.ini')) } }
            @{ Name = 'apps\yori\current existe'; Test = { Test-Path -LiteralPath (Join-Path $apps 'yori\current') } }
            @{ Name = 'atalho Yori.lnk no Menu Iniciar (wingetrs Apps)'; Test = { Test-Path -LiteralPath (Join-Path $startMenu 'Yori.lnk') } }
        )
        Ask = @('A confirmação mostrou o script e o seletor?')
    }

    # B4 7zip
    Invoke-Step @{
        Id = 'B4a'; Title = '7zip: plano (dry-run)'
        Cmd = @($W, 'install', 'main/7zip', '--dry-run')
        Expect = @('Análise: pede confirmação', 'só roda em instalação global; não roda no wingetrs (mexe no registro da máquina (HKLM)) (linha 4)',
            'uninstaller.script (na remoção)', 'Análise: pede atenção redobrada; revise antes de permitir', 'mexe no registro do usuário (HKCU ou reg) (linha 1)')
    }
    Invoke-Step @{
        Id = 'B4b'; Title = '7zip: instalar com --allow-scripts (seletor: s)'; Mode = 'Attached'
        Cmd = @($W, 'install', 'main/7zip', '--allow-scripts')
        Instructions = @('No seletor, aperte s.')
        Checks = @(
            @{ Name = 'install-context.reg e uninstall-context.reg em apps\7zip\current'; Test = {
                    (Test-Path -LiteralPath (Join-Path $apps '7zip\current\install-context.reg')) -and (Test-Path -LiteralPath (Join-Path $apps '7zip\current\uninstall-context.reg')) } }
            @{ Name = '.reg sem {{7zip_dir}} e com o caminho do wingetrs'; Test = {
                    $t = (Get-Content -LiteralPath (Join-Path $apps '7zip\current\install-context.reg') -Raw)
                    (-not $t.Contains('{{7zip_dir}}')) -and ($t -match 'wingetrs') } }
            @{ Name = 'HKCU\Software\Classes\*\shellex\ContextMenuHandlers\7-Zip continua ausente (nada importado)'; Test = {
                    -not (Test-Path -LiteralPath 'HKCU:\Software\Classes\*\shellex\ContextMenuHandlers\7-Zip') } }
        )
    }
    Invoke-Step @{
        Id = 'B4c'; Title = 'Ordem do where.exe 7z (terminal novo)'; Mode = 'Script'
        Show = @('where.exe 7z   # com o PATH de um terminal novo')
        Run = { Invoke-Fresh 'where.exe' @('7z') }
        Notes = @('ordem: shim do wingetrs x ~/scoop x Program Files')
    }

    # B5 git and refusals
    Invoke-Step @{
        Id = 'B5a'; Title = 'git: plano (dry-run, não instale)'
        Cmd = @($W, 'install', 'main/git', '--dry-run')
        Expect = @('Rodar o script pre_install', 'Definir variável de usuário GIT_INSTALL_ROOT', 'Rodar o script post_install',
            'só roda em instalação global', 'mexe no registro do usuário (HKCU ou reg)')
    }
    Invoke-Step @{
        Id = 'B5b'; Title = 'python: recusa (dry-run)'
        Cmd = @($W, 'install', 'main/python', '--dry-run'); ExpectExit = 1
        Expect = @('o script uninstaller.script foi recusado', 'usa -Global')
        Notes = @('motivo repetido (achado 8)?')
    }
    Invoke-Step @{
        Id = 'B5c'; Title = 'vulkan: recusa (dry-run)'
        Cmd = @($W, 'install', 'main/vulkan', '--dry-run'); ExpectExit = 1
        Expect = @('o script pre_uninstall foi recusado', 'mexe no registro da máquina (HKLM)')
    }
    Invoke-Step @{
        Id = 'B5d'; Title = 'pwsh: recusa (dry-run)'
        Cmd = @($W, 'install', 'main/pwsh', '--dry-run'); ExpectExit = 1
        Expect = @('o script post_install foi recusado', 'mexe no registro da máquina (HKLM)')
    }
    Invoke-Step @{
        Id = 'B5e'; Title = 'lazarus (extras): recusa (dry-run)'
        Cmd = @($W, 'install', 'extras/lazarus', '--dry-run'); ExpectExit = 1
        Expect = @('o script post_install foi recusado', 'mexe no registro da máquina (HKLM)')
    }

    # B6 removals
    Invoke-Step @{
        Id = 'B6a'; Title = 'Remover pester'; Mode = 'Stdout'; Destructive = $true
        Cmd = @($W, 'uninstall', 'pester', '--yes')
        Checks = @(@{ Name = 'apps\pester removido'; Test = { -not (Test-Path -LiteralPath (Join-Path $apps 'pester')) } })
    }
    Invoke-Step @{
        Id = 'B6b'; Title = 'Remover xming'; Mode = 'Stdout'; Destructive = $true
        Cmd = @($W, 'uninstall', 'xming', '--yes')
        Checks = @(@{ Name = 'apps\xming removido'; Test = { -not (Test-Path -LiteralPath (Join-Path $apps 'xming')) } })
    }
    Invoke-Step @{
        Id = 'B6c'; Title = 'Remover yori (--purge, seletor: s)'; Mode = 'Attached'; Destructive = $true
        Cmd = @($W, 'uninstall', 'yori', '--allow-scripts', '--purge')
        Instructions = @('No seletor, aperte s.')
        Checks = @(
            @{ Name = 'apps\yori removido'; Test = { -not (Test-Path -LiteralPath (Join-Path $apps 'yori')) } }
            @{ Name = 'persist\yori removido'; Test = { -not (Test-Path -LiteralPath (Join-Path $WData 'persist\yori')) } }
            @{ Name = 'atalho Yori.lnk removido'; Test = { -not (Test-Path -LiteralPath (Join-Path $startMenu 'Yori.lnk')) } }
        )
        Ask = @('Mostrou o plano de remoção em passos e o seletor?')
    }
    Invoke-Step @{
        Id = 'B6d'; Title = 'Remover 7zip (--purge, seletor: s)'; Mode = 'Attached'; Destructive = $true
        Cmd = @($W, 'uninstall', '7zip', '--allow-scripts', '--purge')
        Instructions = @('No seletor, aperte s. O uninstaller.script apaga HKCU\Software\7-Zip (o backup é restaurado logo depois).')
        Checks = @(@{ Name = 'apps\7zip removido'; Test = { -not (Test-Path -LiteralPath (Join-Path $apps '7zip')) } })
        Ask = @('Mostrou o plano de remoção em passos, o uninstaller.script antes de rodar e o seletor?')
    }
    Invoke-Step @{
        Id = 'B6e'; Title = 'HKCU\Software\7-Zip foi apagado pelo script (esperado)'
        Cmd = @('reg', 'query', 'HKCU\Software\7-Zip'); ExpectExit = 1
    }
    Invoke-Step @{
        Id = 'B6f'; Title = 'Restaurar HKCU\Software\7-Zip do backup'; Mode = 'Script'; Destructive = $true
        Show = @("reg import $(Quote-Ps $zipBackup)", "reg export 'HKCU\Software\7-Zip' $(Quote-Ps $zipAfter) /y",
            "Compare-Object (Get-Content $(Quote-Ps $zipBackup)) (Get-Content $(Quote-Ps $zipAfter))")
        When = { Test-Path -LiteralPath $zipBackup }; WhenWhy = 'o backup hkcu-7zip-antes.reg existe (seção 0)'
        Run = {
            & reg.exe import $zipBackup 2>&1
            if ($LASTEXITCODE -ne 0) { "reg import falhou ($LASTEXITCODE)"; return }
            & reg.exe export 'HKCU\Software\7-Zip' $zipAfter /y 2>&1
            $d = @(Compare-Object (Get-Content -LiteralPath $zipBackup) (Get-Content -LiteralPath $zipAfter))
            "diferenças: $($d.Count)"
            $d | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }
            $global:LASTEXITCODE = $d.Count
        }
        ExpectExit = 0
        Checks = @(@{ Name = 'Compare-Object vazio'; Test = { (Test-Path -LiteralPath $zipAfter) -and -not (Compare-Object (Get-Content -LiteralPath $zipBackup) (Get-Content -LiteralPath $zipAfter)) } })
    }
    Invoke-Step @{
        Id = 'B6g'; Title = 'list: só o 7zip do scoop sobra'
        Cmd = @($W, 'list', '--color', 'never')
        Checks = @(
            @{ Name = 'nenhuma linha com pester, xming ou yori'; Test = { -not (Get-WList $W | Where-Object { $_ -match 'pester|xming|yori' }) } }
            @{ Name = 'linhas com 7zip/innounp'; Test = { @(Get-WList $W | Where-Object { $_ -match '7zip|innounp' }) -join ' / ' } }
        )
        Ask = @('A linha do 7zip que sobrou é só a de origem scoop?')
        Notes = @('o innounp saiu junto?')
    }
    Invoke-Step @{
        Id = 'B6h'; Title = 'Remover innounp se ficou'; Mode = 'Stdout'; Destructive = $true
        Cmd = @($W, 'uninstall', 'innounp', '--yes')
        When = { Test-Path -LiteralPath (Join-Path $apps 'innounp') }; WhenWhy = 'apps\innounp ainda existe'
        Checks = @(@{ Name = 'apps\innounp removido'; Test = { -not (Test-Path -LiteralPath (Join-Path $apps 'innounp')) } })
    }
} finally {
    Complete-Section
}
exit (Get-SectionExit)
