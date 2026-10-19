#Requires -Version 7.0
<#
.SYNOPSIS
    Live proof, section 0 (preparação): build, "antes" snapshot, 7-Zip backup, Program Files copy
    (printed for the admin terminal), installer_mode dry-runs and the TUI setting.

.EXAMPLE
    pwsh -File scripts/live/section-0.ps1            # new run folder
    pwsh -File scripts/live/section-0.ps1 -DryRun    # only prints the steps
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
Start-Section -Name '0' -Title 'Preparação' -OutDir $OutDir -W $W -Wdev $Wdev -StartAt $StartAt -DryRun:$DryRun -NewRun
$P = $Ctx.RunDir
$Wdev = $Ctx.Wdev
$pwsh = Get-PwshPath
$cfg = Join-Path $WData 'config.toml'
$wDir = Split-Path $W

try {
    Invoke-Step @{
        Id = '0.1a'; Title = 'Compilar (cargo build --release)'
        Cmd = @('cargo', 'build', '--release', '--manifest-path', (Join-Path $RepoRoot 'Cargo.toml'))
        Checks = @(@{ Name = "existe $Wdev"; Test = { Test-Path -LiteralPath $Wdev } })
    }
    Invoke-Step @{
        Id = '0.1b'; Title = 'Retrato "antes"'
        Cmd = @($pwsh, '-NoProfile', '-File', (Join-Path $PSScriptRoot 'snapshot.ps1'), '-Name', 'antes', '-Out', (Join-Path $P 'antes'), '-Exe', $Wdev)
        Checks = @(@{ Name = 'antes\meta.json existe'; Test = { Test-Path -LiteralPath (Join-Path $P 'antes\meta.json') } })
    }
    Invoke-Step @{
        Id = '0.1c'; Title = 'Backup de HKCU\Software\7-Zip (usado em B6)'
        Cmd = @('reg', 'export', 'HKCU\Software\7-Zip', (Join-Path $P 'hkcu-7zip-antes.reg'), '/y')
        Checks = @(
            @{ Name = 'hkcu-7zip-antes.reg existe'; Test = { Test-Path -LiteralPath (Join-Path $P 'hkcu-7zip-antes.reg') } }
            @{ Name = 'contém Path64 = C:\Program Files\7-Zip\'; Test = { (Get-Content -LiteralPath (Join-Path $P 'hkcu-7zip-antes.reg') -Raw) -match '"Path64"="C:\\\\Program Files\\\\7-Zip\\\\"' } }
        )
    }
    Invoke-Step @{
        Id = '0.1d'; Title = 'Marcar o início do log do wingetrs (usado em D4)'; Mode = 'Script'
        Show = @('$log = "$env:LOCALAPPDATA\wingetrs\log\wingetrs.log.<hoje>"; $logStart = número de linhas atual')
        Run = {
            $log = Join-Path $WData ("log\wingetrs.log.{0}" -f (Get-Date -Format 'yyyy-MM-dd'))
            $n = if (Test-Path -LiteralPath $log) { @(Get-Content -LiteralPath $log).Count } else { 0 }
            Set-State 'log' $log
            Set-State 'logStart' $n
            Set-State 'logStartTime' (Get-Date).ToString('o')
            "log: $log; linha inicial: $n"
        }
    }
    Invoke-Step @{
        Id = '0.1e'; Title = 'Processos de agentes sobrando (leftovers.ps1)'
        Cmd = @($pwsh, '-NoProfile', '-File', (Join-Path $RepoRoot 'scripts\leftovers.ps1'))
        Notes = @('processos listados (se houver)')
    }
    Invoke-Step @{
        Id = '0.2'; Title = 'Copiar o binário para Program Files'; Mode = 'Admin'
        Instructions = @('Abra um PowerShell 7 como administrador (Win+X => Terminal (Admin)); ele fica aberto até o fim (C2, D5, E).',
            'Cole o bloco abaixo nele. Ou responda s para o roteiro abrir um PowerShell elevado só para este bloco (um UAC a mais).')
        AdminText = (@(
                "New-Item -ItemType Directory -Force $(Quote-Ps $wDir) | Out-Null"
                "Copy-Item $(Quote-Ps $Wdev) $(Quote-Ps $W) -Force"
                "(Get-Acl $(Quote-Ps $wDir)).Owner"
                "(Get-Acl $(Quote-Ps $W)).Owner"
                "icacls $(Quote-Ps $wDir)"
                "(Get-FileHash $(Quote-Ps $Wdev)).Hash -eq (Get-FileHash $(Quote-Ps $W)).Hash"
            ) -join "`n")
        Checks = @(
            @{ Name = "existe $W"; Test = { Test-Path -LiteralPath $W } }
            @{ Name = 'hash igual ao de target\release'; Test = { (Get-FileHash -LiteralPath $Wdev).Hash -eq (Get-FileHash -LiteralPath $W).Hash } }
            @{ Name = 'exe e pastas acima: donos admin e escrita só para SYSTEM/Administradores/TrustedInstaller'; Test = { Test-AdminOnlyPath $W } }
            @{ Name = 'icacls da pasta'; Test = { (& icacls $wDir 2>&1 | ForEach-Object { ([string]$_).Trim() } | Where-Object { $_ }) -join ' ; ' } }
        )
    }
    Invoke-Step @{
        Id = '0.3a'; Title = 'installer_mode padrão: PuTTY (dry-run)'
        Cmd = @($W, 'install', 'PuTTY.PuTTY', '--dry-run')
        Expect = @('msiexec.exe /i "<instalador>" /passive /norestart', 'Mostra o progresso do instalador', 'Pede permissão de administrador', 'Roda numa pasta temporária do wingetrs')
        NotExpect = @('/log', '/l*v')
    }
    Invoke-Step @{
        Id = '0.3b'; Title = 'installer_mode padrão: ASP.NET Core 8 (dry-run)'
        Cmd = @($W, 'install', 'Microsoft.DotNet.AspNetCore.8', '--dry-run')
        Expect = @('<instalador> /passive /norestart', 'Mostra o progresso do instalador', 'Pede permissão de administrador', 'Roda numa pasta temporária do wingetrs')
        NotExpect = @('/log', '/l*v')
    }
    Invoke-Step @{
        Id = '0.3c'; Title = 'installer_mode padrão: TreeSize (dry-run)'
        Cmd = @($W, 'install', 'JAMSoftware.TreeSize.Free', '--dry-run')
        Expect = @('/SILENT /SUPPRESSMSGBOXES /NORESTART /CurrentUser /LOG="<LOGPATH>"', 'Mostra o progresso do instalador')
    }
    Invoke-Step @{
        Id = '0.3d'; Title = 'TUI: Ajustes => "Janela do instalador (winget)" => "sem janela"'; Mode = 'Manual'
        Show = @("& $(Quote-Ps $W)")
        Instructions = @('Abra a TUI em outro terminal com o comando abaixo; volte aqui depois de sair dela.',
            'Ajustes => "Janela do instalador (winget)" => "sem janela"; saia com q.')
        Checks = @(@{ Name = 'config.toml tem installer_mode = "silent"'; Test = { (Test-Path -LiteralPath $cfg) -and ((Get-Content -LiteralPath $cfg -Raw) -match 'installer_mode\s*=\s*"silent"') } })
    }
    Invoke-Step @{
        Id = '0.3e'; Title = 'installer_mode silencioso: PuTTY (dry-run)'
        Cmd = @($W, 'install', 'PuTTY.PuTTY', '--dry-run')
        Expect = @('/quiet'); NotExpect = @('/passive', 'Mostra o progresso do instalador')
        Notes = @('texto que aparece no lugar de "Mostra o progresso do instalador"')
    }
    Invoke-Step @{
        Id = '0.3f'; Title = 'installer_mode silencioso: ASP.NET Core 8 (dry-run)'
        Cmd = @($W, 'install', 'Microsoft.DotNet.AspNetCore.8', '--dry-run')
        Expect = @('/quiet'); NotExpect = @('/passive', 'Mostra o progresso do instalador')
    }
    Invoke-Step @{
        Id = '0.3g'; Title = 'installer_mode silencioso: TreeSize (dry-run)'
        Cmd = @($W, 'install', 'JAMSoftware.TreeSize.Free', '--dry-run')
        Expect = @('/VERYSILENT'); NotExpect = @('Mostra o progresso do instalador')
    }
    Invoke-Step @{
        Id = '0.3h'; Title = 'TUI: voltar o ajuste para "mostrar progresso"'; Mode = 'Manual'
        Show = @("& $(Quote-Ps $W)")
        Instructions = @('Na TUI: Ajustes => "Janela do instalador (winget)" => "mostrar progresso"; saia com q.')
        Checks = @(@{ Name = 'config.toml sem installer_mode = "silent"'; Test = { -not ((Test-Path -LiteralPath $cfg) -and ((Get-Content -LiteralPath $cfg -Raw) -match 'installer_mode\s*=\s*"silent"')) } })
    }
    Invoke-Step @{
        Id = '0.3i'; Title = 'PuTTY voltou a /passive (dry-run)'
        Cmd = @($W, 'install', 'PuTTY.PuTTY', '--dry-run')
        Expect = @('/passive /norestart', 'Mostra o progresso do instalador')
    }
} finally {
    Complete-Section
}
exit (Get-SectionExit)
