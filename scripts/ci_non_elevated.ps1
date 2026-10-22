#Requires -Version 7.4
<#
.SYNOPSIS
    CI only (Task 222): the GitHub runner is elevated without UAC, so the tests named in
    tests/it/non_elevated.txt skip themselves in the gate. This runs them for real as a
    throwaway standard user (not in Administrators), then removes that user and its profile.
    Exit 0 = they ran and passed, or the standard user could not be set up (warning, the
    fallback the user chose); 1 = a test ran and failed.

    -DryRun   creates nothing: prints the test executable, the names and the commands.
    -SelfTest checks Test-RunOutput against canned outputs.
    Never run it without one of those outside a CI runner: it creates a local user.
#>
[CmdletBinding()] param([switch]$DryRun, [switch]$SelfTest)
Set-StrictMode -Version Latest
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# '' when the run passed: exit 0, exactly `$Expected` passed (the whole list) and nothing skipped.
function Test-RunOutput([int]$ExitCode, [string]$Text, [int]$Expected) {
    if ($ExitCode -ne 0) { return "código de saída $ExitCode" }
    if ($Text -notmatch "(?m)^test result: ok\. $Expected passed;") {
        return "faltou 'test result: ok. $Expected passed'"
    }
    if ($Text -match 'pulado:') { return 'um teste se pulou como usuário comum' }
    return ''
}

if ($SelfTest) {
    $ok = "running 10 tests`ntest a ... ok`n`ntest result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 900 filtered out; finished in 1.00s`n"
    $cases = @(
        @{ Name = 'os 10 passaram'; Code = 0; Text = $ok; Pass = $true },
        @{ Name = 'só 9 passaram'; Code = 0; Text = ($ok -replace '10 passed', '9 passed'); Pass = $false },
        @{ Name = '100 não é 10'; Code = 0; Text = ($ok -replace '10 passed', '100 passed'); Pass = $false },
        @{ Name = 'código de saída 1'; Code = 1; Text = $ok; Pass = $false },
        @{ Name = 'resultado FAILED'; Code = 0; Text = ($ok -replace 'ok\. 10 passed; 0 failed', 'FAILED. 10 passed; 1 failed'); Pass = $false },
        @{ Name = 'linha pulado'; Code = 0; Text = "test a ... pulado: processo elevado na CI (x)`nok`n$ok"; Pass = $false },
        @{ Name = 'saída vazia'; Code = 0; Text = ''; Pass = $false }
    )
    $bad = @($cases | Where-Object { ((Test-RunOutput $_.Code $_.Text 10) -eq '') -ne $_.Pass } | ForEach-Object Name)
    if ($bad.Count -gt 0) { "[FAIL] Test-RunOutput: $($bad -join ', ')"; exit 1 }
    "[OK] Test-RunOutput: $($cases.Count) casos"
    exit 0
}

function Add-Summary([string]$Line) {
    if ($env:GITHUB_STEP_SUMMARY) { $Line | Add-Content -Path $env:GITHUB_STEP_SUMMARY }
}

$names = @(Get-Content (Join-Path $root 'tests/it/non_elevated.txt') | ForEach-Object Trim | Where-Object { $_ })
if ($names.Count -eq 0) { '[FAIL] tests/it/non_elevated.txt vazio'; exit 1 }

# The `it` test binary the gate already built (same profile: nothing is recompiled).
Push-Location $root
try {
    $json = @(cargo test --test it --no-run --locked --message-format=json 2>$null)
    if ($LASTEXITCODE -ne 0) { "[FAIL] cargo test --no-run saiu com $LASTEXITCODE"; exit 1 }
} finally { Pop-Location }
$exe = $json | Where-Object { $_ -like '{*' } | ForEach-Object { $_ | ConvertFrom-Json } |
    Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'it' -and $_.executable } |
    Select-Object -Last 1 -ExpandProperty executable
if (-not $exe) { '[FAIL] executável de teste it não encontrado'; exit 1 }

$rand = -join (1..6 | ForEach-Object { '{0:x}' -f [Security.Cryptography.RandomNumberGenerator]::GetInt32(16) })
$user = "wingetrs-ci-$rand"
$tempBase = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$work = Join-Path $tempBase $user
$outFile = Join-Path $work 'stdout.txt'
$errFile = Join-Path $work 'stderr.txt'
$testArgs = @('--exact', '--test-threads=1', '--nocapture') + $names
# The child's own folders; `CI` removed (it is not elevated, so nothing skips either way).
$childEnv = [ordered]@{
    TEMP = $work; TMP = $work; USERPROFILE = $work
    APPDATA = Join-Path $work 'AppData'; LOCALAPPDATA = Join-Path $work 'LocalAppData'; CI = $null
}

if ($DryRun) {
    "executável: $exe"
    "testes ($($names.Count)):"
    $names | ForEach-Object { "  $_" }
    'comandos:'
    "  New-LocalUser -Name $user -Password *** -AccountNeverExpires -PasswordNeverExpires -UserMayNotChangePassword"
    "  Add-LocalGroupMember -SID S-1-5-32-545 -Member $user"
    "  icacls `"$root`" /grant `"${user}:(OI)(CI)RX`" /Q"
    "  New-Item -ItemType Directory $work; icacls `"$work`" /grant `"${user}:(OI)(CI)M`" /Q"
    "  Start-Process -FilePath `"$exe`" -ArgumentList '$($testArgs -join ' ')' -Credential ${user}:*** -WorkingDirectory `"$root`" -RedirectStandardOutput `"$outFile`" -RedirectStandardError `"$errFile`" -Wait -PassThru"
    "  ambiente do filho: $(($childEnv.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join '; ')"
    "  finally: Remove-LocalUser $user; perfil (Win32_UserProfile) e $work removidos"
    exit 0
}

if ($env:GITHUB_ACTIONS -ne 'true') { '[FAIL] só roda num runner do GitHub (cria um usuário local); use -DryRun'; exit 1 }

# Secret of this step only: never printed, never written; GitHub masks it in the log anyway.
$alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!#%+-=_.'
do {
    $chars = 1..32 | ForEach-Object { $alphabet[[Security.Cryptography.RandomNumberGenerator]::GetInt32($alphabet.Length)] }
    $plain = -join $chars
} until ($plain -cmatch '[A-Z]' -and $plain -cmatch '[a-z]' -and $plain -match '[0-9]' -and $plain -match '[^A-Za-z0-9]')
if ($env:GITHUB_ACTIONS -eq 'true') { "::add-mask::$plain" }
$secure = ConvertTo-SecureString -String $plain -AsPlainText -Force
$plain = $null

$fallback = ''
$code = $null
$sid = $null
$rc = 1
try {
    try {
        if ((Get-Service seclogon).Status -ne 'Running') { Start-Service seclogon }
        $sid = (New-LocalUser -Name $user -Password $secure -AccountNeverExpires -PasswordNeverExpires `
                -UserMayNotChangePassword -Description 'wingetrs CI (temporary)').SID.Value
        # Users (by SID: the group name is localized) holds "allow log on locally".
        Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $user -ErrorAction SilentlyContinue
        & icacls $root /grant "${user}:(OI)(CI)RX" /Q | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "icacls na raiz saiu com $LASTEXITCODE" }
        New-Item -ItemType Directory -Force $work, $childEnv.APPDATA, $childEnv.LOCALAPPDATA | Out-Null
        & icacls $work /grant "${user}:(OI)(CI)M" /Q | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "icacls na pasta de trabalho saiu com $LASTEXITCODE" }
        $cred = [pscredential]::new($user, $secure)
        $p = Start-Process -FilePath $exe -ArgumentList $testArgs -Credential $cred -WorkingDirectory $root `
            -Environment $childEnv -RedirectStandardOutput $outFile -RedirectStandardError $errFile `
            -Wait -PassThru
        $code = $p.ExitCode
    } catch {
        $fallback = "$_"
    }
    if ($fallback) {
        $msg = "os $($names.Count) testes não elevados ficaram só pulados nesta execução (usuário comum indisponível: $fallback)"
        "::warning::$msg"
        Add-Summary "Usuário comum: $msg"
        $rc = 0
    } else {
        $text = ((Get-Content -Raw $outFile -ErrorAction SilentlyContinue) ?? '') + "`n" +
            ((Get-Content -Raw $errFile -ErrorAction SilentlyContinue) ?? '')
        # A missing exit code is a failure, never a 0.
        $why = Test-RunOutput ($code ?? -1) $text $names.Count
        if ($why) {
            "[FAIL] testes como usuário comum: $why"
            $text -split "`r?`n" | Select-Object -Last 200
            Add-Summary "Usuário comum: FALHOU ($why)"
        } else {
            "[OK]   $($names.Count) passaram como usuário comum"
            Add-Summary "Usuário comum: $($names.Count) passaram como usuário comum"
            $rc = 0
        }
    }
} finally {
    if ($sid) {
        Remove-LocalUser -SID $sid -ErrorAction SilentlyContinue
        Get-CimInstance Win32_UserProfile | Where-Object SID -EQ $sid | Remove-CimInstance -ErrorAction SilentlyContinue
    }
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
exit $rc
