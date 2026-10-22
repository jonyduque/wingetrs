# PreToolUse hook (Bash/PowerShell): refuses `cargo fmt` without `--check`.
# `cargo fmt -- <files>` still formats the whole crate and rewrote other agents'
# uncommitted work (Task 209, 2026-10-22). `cargo fmt --check` (the gate) passes.
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
try {
    $payload = [Console]::In.ReadToEnd() | ConvertFrom-Json
} catch {
    exit 0
}
$cmd = [string]$payload.tool_input.command
if (-not $cmd) { exit 0 }

# `cargo fmt` as a command: at the start or after a separator, optionally with a toolchain (+nightly).
$pattern = '(?im)(^|[;&|(\n]|&&|\|\|)\s*cargo(\.exe)?\s+(\+\S+\s+)?fmt\b(?<rest>[^;&|\n]*)'
foreach ($m in [regex]::Matches($cmd, $pattern)) {
    if ($m.Groups['rest'].Value -notmatch '--check\b') {
        $out = @{
            hookSpecificOutput = @{
                hookEventName            = 'PreToolUse'
                permissionDecision       = 'deny'
                permissionDecisionReason = 'cargo fmt sem --check formata o crate inteiro e mexe no WIP de outros agentes. Use: rustfmt --edition 2024 <seus arquivos>. (Para formatar tudo de propósito, o usuário roda `! cargo fmt`.)'
            }
        }
        $out | ConvertTo-Json -Depth 4 -Compress
        exit 0
    }
}
exit 0
