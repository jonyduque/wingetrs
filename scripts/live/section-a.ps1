#Requires -Version 7.0
<#
.SYNOPSIS
    Live proof, section A (Task 131, 2b): rust plan, selector + Ctrl+C before msiexec, Ctrl+C
    during msiexec /a, and the msi.lock queue with the winget bridge. No UAC.

.DESCRIPTION
    Ctrl+C steps run in a new console window: a Ctrl+C here would also stop this script.

.EXAMPLE
    pwsh -File scripts/live/section-a.ps1 -DryRun
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
Start-Section -Name 'A' -Title 'Task 131 — reserva msiexec /a (sem UAC)' -OutDir $OutDir -W $W -Wdev $Wdev -StartAt $StartAt -DryRun:$DryRun
$rustDir = Join-Path $WData 'apps\rust'

try {
    Invoke-Step @{
        Id = 'A1'; Title = 'Plano do rust (dry-run)'
        Cmd = @($W, 'install', 'main/rust', '--dry-run')
        Expect = @('Instalar rust 1.99.1', 'https://static.rust-lang.org/dist/rust-1.99.1-x86_64-pc-windows-msvc.msi', 'sha256 11d5b1d2',
            'MSI, sem executar o instalador', 'só a subpasta Rust', 'Se o extrator não suportar este MSI, pede --allow-msiexec', 'Simulação: nada foi alterado.')
    }
    Invoke-Step @{
        Id = 'A2'; Title = 'Seletor e Ctrl+C durante o download'; Mode = 'Window'
        Cmd = @($W, 'install', 'main/rust', '--allow-msiexec')
        Instructions = @('Abre uma janela nova. Lá: no seletor "Continuar?" (cursor em Não) aperte s.',
            'Durante "Baixando" aperte Ctrl+C. Leia a saída e feche a janela (exit).')
        ExpectExit = 130
        Checks = @(@{ Name = "apps\rust não existe"; Test = { -not (Test-Path -LiteralPath $rustDir) } })
        Ask = @('A confirmação mostrou o plano SEM URL nem sha256?',
            'Apareceu "Se o extrator recusar este MSI, usa o Windows Installer (msiexec /a)"?',
            'O seletor apareceu na linha com Não marcado, e s aceitou?',
            'A linha de progresso tinha spinner, fase, MiB, velocidade e relógio?',
            'O Ctrl+C mostrou "Cancelado."?')
        Notes = @('sobrou pasta sem versão? (qual)')
    }
    Invoke-Step @{
        Id = 'A3'; Title = 'Ctrl+C durante o msiexec /a e instalação completa'; Mode = 'Window'
        Cmd = @($W, 'install', 'main/rust', '--allow-msiexec', '--yes')
        Instructions = @('Abre uma janela nova. Quando a linha mudar para a fase do Windows Installer, aperte Ctrl+C UMA vez.',
            'Espere terminar (o msiexec /a não se cancela), leia e feche a janela (exit).')
        Uac = 'NÃO deve abrir UAC (o msiexec /a não pede). Se abrir, anote.'
        Checks = @(@{ Name = 'apps\rust\current existe'; Test = { Test-Path -LiteralPath (Join-Path $rustDir 'current') } })
        Ask = @('O extrator recusou e passou para msiexec /a?',
            'O Ctrl+C mostrou UMA vez que o passo não pode ser interrompido e esperou terminar?',
            'A linha continuou viva (spinner, relógio; "sem atividade há 1 min" se parou)?',
            'Nenhum UAC apareceu?')
        Notes = @('tempo total da instalação')
    }
    Invoke-Step @{
        Id = 'A3b'; Title = 'rustc --version num terminal novo'; Mode = 'Script'
        Show = @('rustc --version   # com o PATH de um terminal novo')
        Run = { Invoke-Fresh 'rustc' @('--version') }
        ExpectExit = 0; Expect = @('rustc 1.99.1')
    }
    Invoke-Step @{
        Id = 'A4a'; Title = 'Remover o rust antes do teste da fila'; Mode = 'Stdout'
        Cmd = @($W, 'uninstall', 'rust', '--yes')
        Checks = @(@{ Name = 'apps\rust removido'; Test = { -not (Test-Path -LiteralPath $rustDir) } })
    }
    Invoke-Step @{
        Id = 'A4b'; Title = 'Ponte e reserva ao mesmo tempo (fila do msi.lock)'; Mode = 'Parallel'
        Cmd = @($W, 'install', 'main/rust', '--allow-msiexec', '--yes')
        Cmd2 = @($W, 'install', 'sharkdp.hyperfine', '--yes')
        Between = 'Quando a janela 1 entrar na fase do Windows Installer'
        Instructions = @('Abre a janela 1 com a instalação do rust. Este terminal roda o hyperfine quando você apertar Enter.',
            'Depois que o hyperfine terminar, espere a janela 1 terminar e feche-a (exit).')
        Uac = 'NÃO deve abrir UAC (hyperfine é zip de usuário). Se abrir, anote.'
        NotExpect = @('1618')
        Ask = @('Este terminal ficou em "Aguardando o Windows Installer (outro instalador em uso)" com relógio até a janela 1 terminar o msiexec?',
            'Depois rodou o winget install (saída do winget, progresso na linha)?',
            'Nenhum seletor apareceu (--yes) e nenhum UAC?')
        Notes = @('horários (job.start/job.end no log) das duas execuções')
    }
    Invoke-Step @{
        Id = 'A4c'; Title = 'hyperfine --version num terminal novo'; Mode = 'Script'
        Show = @('hyperfine --version   # com o PATH de um terminal novo')
        Run = { Invoke-Fresh 'hyperfine' @('--version') }
        ExpectExit = 0; Expect = @('hyperfine 1.20.0')
    }
    Invoke-Step @{
        Id = 'A4d'; Title = 'Remover o rust (hyperfine fica para a seção E)'; Mode = 'Stdout'
        Cmd = @($W, 'uninstall', 'rust', '--yes')
        Uac = 'NÃO deve abrir UAC.'
        Checks = @(@{ Name = 'apps\rust removido'; Test = { -not (Test-Path -LiteralPath $rustDir) } })
    }
} finally {
    Complete-Section
}
exit (Get-SectionExit)
