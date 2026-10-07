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
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Path,
    [Parameter(Mandatory)] [string]$Anchor,
    [Parameter(Mandatory)] [AllowEmptyString()] [string]$Replacement,
    [Parameter(Mandatory)] [string]$TestCommand
)

Set-StrictMode -Version Latest
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
if (-not (Test-Path $Path)) { "[!] arquivo não existe: $Path"; exit 2 }

$orig = [IO.File]::ReadAllBytes((Resolve-Path $Path))
$text = [Text.Encoding]::UTF8.GetString($orig)
$count = ([regex]::Matches($text, [regex]::Escape($Anchor))).Count
if ($count -ne 1) { "[!] âncora encontrada $count vezes (precisa ser 1): $Anchor"; exit 2 }

$mutant = $text.Replace($Anchor, $Replacement)
try {
    [IO.File]::WriteAllText((Resolve-Path $Path), $mutant, [Text.UTF8Encoding]::new($false))
    $out = Invoke-Expression "$TestCommand 2>&1" | ForEach-Object { "$_" }
    $code = $LASTEXITCODE
} finally {
    [IO.File]::WriteAllBytes((Resolve-Path $Path), $orig)
}

$compileFail = $out | Select-String -Pattern 'could not compile|^error\[E\d+\]|^error: ' -Quiet
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
