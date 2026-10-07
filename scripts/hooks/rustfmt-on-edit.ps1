#Requires -Version 7.0
# PostToolUse hook (Edit|Write): run rustfmt on the edited .rs file so agents
# never spend a gate round-trip on formatting. Silent on success; never blocks.
$ErrorActionPreference = 'SilentlyContinue'
$payload = [Console]::In.ReadToEnd() | ConvertFrom-Json
$path = $payload.tool_input.file_path
if ($path -and $path.EndsWith('.rs') -and (Test-Path $path)) {
    rustfmt --edition 2024 --quiet $path 2>$null | Out-Null
}
exit 0
