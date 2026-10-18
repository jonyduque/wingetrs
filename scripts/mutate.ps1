#Requires -Version 7.0
<#
.SYNOPSIS
    Mutation proof: replace one exact anchor in a file, run a test command,
    restore the file, and report whether the tests caught the mutation.

.DESCRIPTION
    EXIT CODE:
      0 -> mutation CAUGHT (tests failed with the mutant). This is what you want.
      1 -> mutation SURVIVED (tests passed with the mutant): the rule is not verified.
      2 -> inconclusive: anchor missing/ambiguous, or the mutant does not compile.
    The file is always restored (byte-identical), even on Ctrl+C.

.EXAMPLE
    pwsh -File scripts/mutate.ps1 -Path src/core/version.rs -Anchor 'a.cmp(&b)' -Replacement 'b.cmp(&a)' -TestCommand 'cargo test --quiet version'

.EXAMPLE
    Multi-line anchors: put the exact text in files (a command-line argument cannot carry newlines reliably).
    pwsh -File scripts/mutate.ps1 -Path src/x.rs -AnchorFile target/tmp/a.txt -ReplacementFile target/tmp/r.txt -TestCommand 'cargo test --quiet x'
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Path,
    [string]$Anchor,
    [AllowEmptyString()] [string]$Replacement,
    [string]$AnchorFile,
    [string]$ReplacementFile,
    [Parameter(Mandatory)] [string]$TestCommand
)

Set-StrictMode -Version Latest
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if (-not (Test-Path $Path)) { "[!] arquivo não existe: $Path"; exit 2 }

if ($AnchorFile) { $Anchor = [IO.File]::ReadAllText((Resolve-Path $AnchorFile)) }
if ($ReplacementFile) { $Replacement = [IO.File]::ReadAllText((Resolve-Path $ReplacementFile)) }
if ([string]::IsNullOrEmpty($Anchor)) { "[!] informe -Anchor ou -AnchorFile"; exit 2 }
if ($null -eq $Replacement) { "[!] informe -Replacement ou -ReplacementFile"; exit 2 }
$orig = [IO.File]::ReadAllBytes((Resolve-Path $Path))
$text = [Text.Encoding]::UTF8.GetString($orig)
# Match line endings: a multi-line anchor written with LF still finds a CRLF file, and vice versa.
if ($text.Contains("`r`n")) {
    $Anchor = $Anchor -replace "(?<!`r)`n", "`r`n"
    $Replacement = $Replacement -replace "(?<!`r)`n", "`r`n"
} else {
    $Anchor = $Anchor -replace "`r`n", "`n"
    $Replacement = $Replacement -replace "`r`n", "`n"
}
$count = ([regex]::Matches($text, [regex]::Escape($Anchor))).Count
if ($count -ne 1) { "[!] âncora encontrada $count vezes (precisa ser 1): $(($Anchor -split "`r?`n")[0])"; exit 2 }

# Baseline: a test that already fails would turn every mutant into a false "PEGA" (Task 164).
$baseOut = Invoke-Expression "$TestCommand 2>&1" | ForEach-Object { "$_" }
if ($LASTEXITCODE -ne 0) {
    "[?] INCONCLUSIVO: o teste já falha sem a mutação (exit $LASTEXITCODE). Conserte o teste ou o código antes de provar a mutação."
    $baseOut | Select-String -Pattern 'panicked at|FAILED|^error' | Select-Object -First 5 | ForEach-Object { "  $($_.Line)" }
    exit 2
}

$mutant = $text.Replace($Anchor, $Replacement)
try {
    [IO.File]::WriteAllText((Resolve-Path $Path), $mutant, [Text.UTF8Encoding]::new($false))
    $out = Invoke-Expression "$TestCommand 2>&1" | ForEach-Object { "$_" }
    $code = $LASTEXITCODE
} finally {
    [IO.File]::WriteAllBytes((Resolve-Path $Path), $orig)
}

# Not '^error: ': cargo prints "error: test failed, to rerun pass ..." when a test fails,
# which would turn every caught mutant into "does not compile".
$compileFail = $out | Select-String -Pattern 'could not compile|^error\[E\d+\]' -Quiet
if ($code -ne 0 -and $compileFail) {
    "[?] INCONCLUSIVO: o mutante não compila (exit $code). Escolha uma mutação que compile."
    $out | Select-String -Pattern '^error' | Select-Object -First 5 | ForEach-Object { "  $($_.Line)" }
    exit 2
}
if ($code -ne 0) {
    "[OK] mutação PEGA: '$Anchor' -> '$Replacement' fez o teste falhar (exit $code)."
    $out | Select-String -Pattern 'panicked at|FAILED|test result' | Select-Object -First 6 | ForEach-Object { "  $($_.Line)" }
    exit 0
}
"[X] mutação SOBREVIVEU: '$Anchor' -> '$Replacement' e os testes passaram. A regra não está coberta."
exit 1
