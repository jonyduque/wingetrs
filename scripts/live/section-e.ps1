#Requires -Version 7.0
<#
.SYNOPSIS
    Live proof, section E: cleanup (hyperfine, test orphan keys, 7-Zip key if still different),
    "depois" snapshot, comparison with "antes" and leftovers. Every destructive step asks first.

.EXAMPLE
    pwsh -File scripts/live/section-e.ps1 -DryRun
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
Start-Section -Name 'E' -Title 'Retratos "depois", comparação e limpeza' -OutDir $OutDir -W $W -Wdev $Wdev -StartAt $StartAt -DryRun:$DryRun
$P = $Ctx.RunDir
$Wdev = $Ctx.Wdev
$pwsh = Get-PwshPath
$orfHkcu = 'HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan'
$orfHklm = 'HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan'
$orfHkcuPs = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan'
$orfHklmPs = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\wingetrs-test-orphan'
$zipBackup = Join-Path $P 'hkcu-7zip-antes.reg'
$zipNow = Join-Path $P 'hkcu-7zip-agora.reg'
$wDir = Split-Path $W

function Test-ZipDiffers {
    if (-not (Test-Path -LiteralPath $zipBackup)) { return $false }
    $null = & reg.exe export 'HKCU\Software\7-Zip' $zipNow /y 2>&1
    if ($LASTEXITCODE -ne 0) { return $true }
    [bool](Compare-Object (Get-Content -LiteralPath $zipBackup) (Get-Content -LiteralPath $zipNow))
}

try {
    Invoke-Step @{
        Id = 'E1'; Title = 'Remover o hyperfine'; Mode = 'Stdout'; Destructive = $true
        Cmd = @($W, 'uninstall', 'sharkdp.hyperfine', '--yes')
        Checks = @(@{ Name = 'list sem hyperfine'; Test = { -not (Get-WList $W | Where-Object { $_ -match 'hyperfine' }) } })
        Notes = @('removido pela ponte (winget uninstall) ou pelo ARP?')
    }
    Invoke-Step @{
        Id = 'E2'; Title = 'Apagar a chave de teste HKCU'; Destructive = $true
        Cmd = @('reg', 'delete', $orfHkcu, '/f')
        When = { Test-Path -LiteralPath $orfHkcuPs }; WhenWhy = 'a chave HKCU de teste existe'
        Checks = @(@{ Name = 'chave HKCU sumiu'; Test = { -not (Test-Path -LiteralPath $orfHkcuPs) } })
    }
    Invoke-Step @{
        Id = 'E3'; Title = 'Apagar a chave de teste HKLM'; Mode = 'Admin'; Destructive = $true
        When = { Test-Path -LiteralPath $orfHklmPs }; WhenWhy = 'a chave HKLM de teste existe'
        AdminText = "reg delete $(Quote-Ps $orfHklm) /f"
        Checks = @(@{ Name = 'chave HKLM sumiu'; Test = { -not (Test-Path -LiteralPath $orfHklmPs) } })
    }
    Invoke-Step @{
        Id = 'E4'; Title = 'Restaurar HKCU\Software\7-Zip do backup (só se ainda diferente)'; Mode = 'Script'; Destructive = $true
        Show = @("reg import $(Quote-Ps $zipBackup)")
        When = { Test-ZipDiffers }; WhenWhy = 'HKCU\Software\7-Zip difere do backup da seção 0'
        Run = { & reg.exe import $zipBackup 2>&1 }
        ExpectExit = 0
        Checks = @(@{ Name = 'HKCU\Software\7-Zip igual ao backup'; Test = { -not (Test-ZipDiffers) } })
    }
    Invoke-Step @{
        Id = 'E5'; Title = 'Retrato "depois"'
        Cmd = @($pwsh, '-NoProfile', '-File', (Join-Path $PSScriptRoot 'snapshot.ps1'), '-Name', 'depois', '-Out', (Join-Path $P 'depois'), '-Exe', $Wdev, '-Force')
        Checks = @(@{ Name = 'depois\meta.json existe'; Test = { Test-Path -LiteralPath (Join-Path $P 'depois\meta.json') } })
    }
    Invoke-Step @{
        Id = 'E6'; Title = 'Comparar "antes" x "depois"'
        Cmd = @($pwsh, '-NoProfile', '-File', (Join-Path $PSScriptRoot 'compare.ps1'), '-Before', (Join-Path $P 'antes'), '-After', (Join-Path $P 'depois'))
        Expect = @('~/scoop sem diferenças')
        Ask = @('HKCU\Environment: no máximo Path/PSModulePath que já existiam?',
            'ARP, MSIX, raiz do wingetrs (apps, persist sem yori/7zip) e list sem diferenças?',
            'run-* e staging vazios?')
        Notes = @('o que mudou no HKCU\Environment', 'sobras (pacote e área)')
    }
    Invoke-Step @{
        Id = 'E7'; Title = 'Processos de agentes sobrando (leftovers.ps1)'
        Cmd = @($pwsh, '-NoProfile', '-File', (Join-Path $RepoRoot 'scripts\leftovers.ps1'))
        Notes = @('processos novos (se houver)')
    }
    Invoke-Step @{
        Id = 'E8'; Title = 'Remover C:\Program Files\wingetrs (opcional: mantenha se for usar o auxiliar depois)'; Mode = 'Admin'; Destructive = $true
        When = { Test-Path -LiteralPath $wDir }; WhenWhy = "$wDir existe"
        Instructions = @('Pule (p) se for usar o auxiliar depois. Depois feche o terminal de administrador.')
        AdminText = "Remove-Item -Recurse $(Quote-Ps $wDir)"
        Checks = @(@{ Name = "$wDir removido"; Test = { -not (Test-Path -LiteralPath $wDir) } })
    }
    Invoke-Step @{
        Id = 'E9'; Title = 'Lembrete final'; Mode = 'Manual'
        Show = @("Remove-Item -Recurse $(Quote-Ps $P)   # só depois de colar as saídas nos relatórios")
        Instructions = @('Cole os section-*.md desta pasta nos relatórios das Tasks 131, 163 e 188.',
            'C:\Windows\Installer\SourceHash{...} do rust fica (permanente, aceito pela spec da 2b).',
            'O roteiro não apaga a pasta da execução: faça isso você mesmo quando terminar.')
    }
} finally {
    Complete-Section
}
exit (Get-SectionExit)
