#Requires -Version 7.0
<#
.SYNOPSIS
    Live proof, section C (2d without UAC): machine MSI without elevation, test orphan keys,
    helper refusal from target\release, user/msix installs and upgrades.

.EXAMPLE
    pwsh -File scripts/live/section-c.ps1 -DryRun
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
Start-Section -Name 'C' -Title '2d sem UAC — usuário, msix, ponte, upgrades, recusa da pasta' -OutDir $OutDir -W $W -Wdev $Wdev -StartAt $StartAt -DryRun:$DryRun
$P = $Ctx.RunDir
$Wdev = $Ctx.Wdev
$puttyMsi = Join-Path $P 'putty.msi'
$puttyLog = Join-Path $P 'putty-semadmin.log'
$orfSub = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan'
$orfHkcu = "HKCU\$orfSub"
$orfHklm = "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan"
$orfHkcuPs = "HKCU:\$orfSub"
$orfHklmPs = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan'
$backups = Join-Path $WData 'backups'

try {
    # C1
    Invoke-Step @{
        Id = 'C1a'; Title = 'Baixar o MSI do PuTTY 0.85'; Mode = 'Script'
        Show = @("Invoke-WebRequest https://the.earth.li/~sgtatham/putty/0.85/w64/putty-64bit-0.85-installer.msi -OutFile $(Quote-Ps $puttyMsi)")
        Run = { Invoke-WebRequest -Uri 'https://the.earth.li/~sgtatham/putty/0.85/w64/putty-64bit-0.85-installer.msi' -OutFile $puttyMsi; "baixado: $((Get-Item -LiteralPath $puttyMsi).Length) bytes" }
        Checks = @(@{ Name = 'putty.msi existe'; Test = { Test-Path -LiteralPath $puttyMsi } })
    }
    Invoke-Step @{
        Id = 'C1b'; Title = 'MSI de máquina sem elevação (msiexec /i /quiet)'; Mode = 'Script'
        Instructions = @('Hipótese: MSI de máquina exige elevação => código diferente de 0 (provável 1603 ou 1925).',
            'Se der 0, o PuTTY foi instalado: o D2 pula o PuTTY e o D3 o remove.')
        Show = @("(Start-Process msiexec.exe -ArgumentList '/i', '`"$puttyMsi`"', '/quiet', '/norestart', '/l*v', '`"$puttyLog`"' -Wait -PassThru).ExitCode")
        When = { Test-Path -LiteralPath $puttyMsi }; WhenWhy = 'putty.msi foi baixado (C1a)'
        Run = {
            $p = Start-Process msiexec.exe -ArgumentList '/i', "`"$puttyMsi`"", '/quiet', '/norestart', '/l*v', "`"$puttyLog`"" -Wait -PassThru
            "ExitCode: $($p.ExitCode)"
            if (Test-Path -LiteralPath $puttyLog) {
                Select-String -LiteralPath $puttyLog -Pattern 'Error' -SimpleMatch | Select-Object -First 5 | ForEach-Object { "log: $($_.Line.Trim())" }
            }
            $global:LASTEXITCODE = $p.ExitCode
        }
        ExpectExit = 'nonzero'
        After = { param($rec) Set-State 'puttyInstalledInC1' ($rec.Exit -eq 0) }
        Notes = @('código de saída e a linha "Error" do log')
    }

    # C2
    Invoke-Step @{
        Id = 'C2a'; Title = 'Chave de teste HKLM (órfão)'; Mode = 'Admin'
        AdminText = (@(
                "reg add $(Quote-Ps $orfHklm) /v DisplayName /t REG_SZ /d 'wingetrs test orphan HKLM' /f"
                "reg add $(Quote-Ps $orfHklm) /v UninstallString /t REG_SZ /d '`"C:\Program Files\wingetrs-test\u.exe`"' /f"
                "reg export $(Quote-Ps $orfHklm) $(Quote-Ps (Join-Path $P 'orf-hklm-antes.reg')) /reg:64 /y"
            ) -join "`n")
        Checks = @(
            @{ Name = 'chave HKLM existe com DisplayName'; Test = { (Get-ItemProperty -LiteralPath $orfHklmPs -ErrorAction Stop).DisplayName -eq 'wingetrs test orphan HKLM' } }
            @{ Name = 'orf-hklm-antes.reg existe'; Test = { Test-Path -LiteralPath (Join-Path $P 'orf-hklm-antes.reg') } }
        )
    }
    Invoke-Step @{
        Id = 'C2b'; Title = 'Chave de teste HKCU (órfão)'; Mode = 'Script'
        Show = @(
            "reg add '$orfHkcu' /v DisplayName /t REG_SZ /d 'wingetrs test orphan HKCU' /f"
            "reg add '$orfHkcu' /v UninstallString /t REG_SZ /d `"$env:LOCALAPPDATA\wingetrs-test\u.exe`" /f"
            "reg export '$orfHkcu' $(Quote-Ps (Join-Path $P 'orf-hkcu-antes.reg')) /y")
        Run = {
            & reg.exe add $orfHkcu /v DisplayName /t REG_SZ /d 'wingetrs test orphan HKCU' /f 2>&1
            & reg.exe add $orfHkcu /v UninstallString /t REG_SZ /d "$env:LOCALAPPDATA\wingetrs-test\u.exe" /f 2>&1
            & reg.exe export $orfHkcu (Join-Path $P 'orf-hkcu-antes.reg') /y 2>&1
        }
        ExpectExit = 0
        Checks = @(
            @{ Name = 'chave HKCU existe com DisplayName'; Test = { (Get-ItemProperty -LiteralPath $orfHkcuPs -ErrorAction Stop).DisplayName -eq 'wingetrs test orphan HKCU' } }
            @{ Name = 'orf-hkcu-antes.reg existe'; Test = { Test-Path -LiteralPath (Join-Path $P 'orf-hkcu-antes.reg') } }
        )
    }
    Invoke-Step @{
        Id = 'C2c'; Title = 'Órfãos (dry-run)'
        Cmd = @($W, 'cleanup', '--orphans', '--dry-run')
        Expect = @('wingetrs test orphan HKCU', 'wingetrs test orphan HKLM', 'HKLM 64 bits', 'Cities: Skylines', 'Counter-Strike: Global Offensive',
            'USB-Driver-Installer 1.0.3.0', 'O app pode ter mudado de lugar (ex.: Steam em outra pasta).', 'Antes de apagar, cada registro é exportado para um .reg em')
    }

    # C3
    Invoke-Step @{
        Id = 'C3'; Title = 'Recusa do auxiliar rodando de target\release'; Mode = 'Attached'
        Cmd = @($Wdev, 'cleanup', '--orphans')
        Instructions = @('Seletor de marcar: desça até "wingetrs test orphan HKLM", Espaço, Enter; confirme com s.')
        Uac = 'NÃO deve abrir UAC. Se abrir, clique Não e anote.'
        ExpectExit = 'any'
        Expect = @('Para usar funções de administrador, o wingetrs precisa estar numa pasta que só o administrador grava')
        Checks = @(
            @{ Name = 'chave HKLM continua'; Test = { Test-Path -LiteralPath $orfHklmPs } }
            @{ Name = '.reg em backups (últimos 15 min)'; Test = {
                    $r = @(Get-ChildItem -LiteralPath $backups -Recurse -Filter '*.reg' -ErrorAction SilentlyContinue | Where-Object LastWriteTime -gt (Get-Date).AddMinutes(-15) | ForEach-Object FullName)
                    if ($r) { $r -join ', ' } else { 'nenhum' } } }
        )
        Ask = @('Nenhum item veio marcado e o rodapé era "setas movem · Espaço marca · Enter confirma · Esc cancela"?',
            'Nenhum UAC abriu?', 'O aviso da pasta apareceu uma vez só?')
        Notes = @('o aviso "Aguardando autorização ..." chegou a aparecer antes da recusa?')
    }

    # C4
    $inst = @(
        @{ Id = 'C4a'; Pkg = 'JAMSoftware.TreeSize.Free'; Ask = @('Janela de progresso do Inno (/SILENT)?'); Uac = 'O instalador (elevatesSelf) pode pedir UAC sozinho: se aparecer, aceite e anote (não é do wingetrs).'; Notes = @('o próprio instalador pediu UAC? a linha disse "Aguardando autorização..." e depois "rodando o instalador (pode pedir autorização de administrador)"?') }
        @{ Id = 'C4b'; Pkg = 'M2Team.NanaZip'; Ask = @('Nenhuma janela de instalador?'); Check = @{ Name = 'Get-AppxPackage *NanaZip* existe'; Test = { [bool](Get-AppxPackage -Name '*NanaZip*') } } }
        @{ Id = 'C4c'; Pkg = 'SumatraPDF.SumatraPDF'; Ask = @('Sem janela (-s)?') }
        @{ Id = 'C4d'; Pkg = 'Kumospace.Kumospace'; Ask = @('Abriu a janela do instalador (concluída por você)?'); Notes = @('hash divergente? (se sim, pule)') }
    )
    foreach ($i in $inst) {
        $step = @{
            Id = $i.Id; Title = "Instalar $($i.Pkg) (usuário/msix, seletor: s)"; Mode = 'Attached'
            Cmd = @($W, 'install', $i.Pkg)
            Instructions = @('No seletor, aperte s.')
            Ask = @('A confirmação mostrou o plano em passos SEM "Instalador:" (URL) nem "SHA-256:", e o seletor com Não marcado?') + $i.Ask
        }
        if ($i["Uac"]) { $step.Uac = $i["Uac"] } else { $step.Uac = 'NÃO deve abrir UAC do wingetrs.' }
        if ($i["Notes"]) { $step.Notes = $i["Notes"] }
        if ($i["Check"]) { $step.Checks = @($i["Check"]) }
        Invoke-Step $step
    }
    Invoke-Step @{
        Id = 'C4e'; Title = 'Os quatro aparecem no list'
        Cmd = @($W, 'list', '--color', 'never')
        Expect = @('TreeSize', 'NanaZip', 'SumatraPDF', 'Kumospace')
    }
    foreach ($u in @(
            @{ Id = 'C4f'; Pkg = 'JAMSoftware.TreeSize.Free' }
            @{ Id = 'C4g'; Pkg = 'M2Team.NanaZip'; Check = @{ Name = 'NanaZip removido (Get-AppxPackage vazio)'; Test = { -not (Get-AppxPackage -Name '*NanaZip*') } } }
            @{ Id = 'C4h'; Pkg = 'SumatraPDF.SumatraPDF' }
            @{ Id = 'C4i'; Pkg = 'Kumospace.Kumospace' })) {
        $step = @{
            Id = $u.Id; Title = "Remover $($u.Pkg) (seletor: s)"; Mode = 'Attached'; Destructive = $true
            Cmd = @($W, 'uninstall', $u.Pkg)
            Instructions = @('No seletor, aperte s.')
            Uac = 'NÃO deve abrir UAC do wingetrs.'
            Ask = @('Mostrou o plano de remoção em passos e o seletor?')
            Notes = @('abriu janela de desinstalador?')
        }
        if ($u["Check"]) { $step.Checks = @($u["Check"]) }
        Invoke-Step $step
    }

    # C5
    Invoke-Step @{
        Id = 'C5a'; Title = 'Sumatra ainda não instalado (upgrade dry-run)'
        Cmd = @($W, 'upgrade', 'SumatraPDF.SumatraPDF', '--dry-run'); ExpectExit = 'any'
        Expect = @("'SumatraPDF.SumatraPDF' não está instalado")
    }
    Invoke-Step @{
        Id = 'C5b'; Title = 'Instalar Sumatra 3.6 pelo winget (usuário)'; Mode = 'Attached'
        Cmd = @('winget', 'install', '--id', 'SumatraPDF.SumatraPDF', '--version', '3.6', '--scope', 'user', '--exact', '--silent')
    }
    Invoke-Step @{
        Id = 'C5c'; Title = 'Upgrade Sumatra (dry-run)'
        Cmd = @($W, 'upgrade', 'SumatraPDF.SumatraPDF', '--dry-run')
        Expect = @('SumatraPDF 3.6.1 (exe, usuário)', '-s -install -with-preview'); NotExpect = @('Pede permissão')
    }
    Invoke-Step @{
        Id = 'C5d'; Title = 'Upgrade Sumatra'; Mode = 'Stdout'
        Cmd = @($W, 'upgrade', 'SumatraPDF.SumatraPDF', '--yes')
    }
    Invoke-Step @{
        Id = 'C5e'; Title = 'Instalar clippy 1.7.8 pelo winget'; Mode = 'Attached'
        Cmd = @('winget', 'install', '--id', '0-don.clippy', '--version', '1.7.8', '--exact', '--silent')
    }
    Invoke-Step @{
        Id = 'C5f'; Title = 'Upgrade clippy (dry-run)'
        Cmd = @($W, 'upgrade', '0-don.clippy', '--dry-run')
        Expect = @('(nullsoft, usuário)', '/S', 'Remove a versão instalada antes (o desinstalador pode abrir janela ou pedir permissão de administrador)')
    }
    Invoke-Step @{
        Id = 'C5g'; Title = 'Upgrade clippy (remove a 1.7.8 antes)'; Mode = 'Stdout'
        Cmd = @($W, 'upgrade', '0-don.clippy', '--yes')
        Notes = @('como a 1.7.8 foi removida antes')
    }
    Invoke-Step @{
        Id = 'C5h'; Title = 'list: Sumatra 3.6.1 e uma linha só do clippy 1.7.9'
        Cmd = @($W, 'list', '--color', 'never')
        Expect = @('3.6.1', '1.7.9')
        Checks = @(@{ Name = 'uma linha só com clippy'; Test = { @(Get-WList $W | Where-Object { $_ -match 'clippy' }).Count -eq 1 } })
    }
    Invoke-Step @{
        Id = 'C5i'; Title = 'Remover Sumatra'; Mode = 'Stdout'; Destructive = $true
        Cmd = @($W, 'uninstall', 'SumatraPDF.SumatraPDF', '--yes')
    }
    Invoke-Step @{
        Id = 'C5j'; Title = 'Remover clippy'; Mode = 'Stdout'; Destructive = $true
        Cmd = @($W, 'uninstall', '0-don.clippy', '--yes')
        Checks = @(@{ Name = 'list sem Sumatra e sem clippy'; Test = { -not (Get-WList $W | Where-Object { $_ -match 'Sumatra|clippy' }) } })
    }
} finally {
    Complete-Section
}
exit (Get-SectionExit)
