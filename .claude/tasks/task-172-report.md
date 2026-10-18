Status: DONE_WITH_CONCERNS

Commits:
c8b7783 feat(winget-install): uninstall command from an ARP key (msi, quiet, inno, burn, nsis)

Gate: verify.ps1 failed at clippy ONLY on another agent's file: tests\it\script_runner.rs:223:5 and :224:15 `used unwrap() on a Result value` (not mine). rules/rules-selftest/fmt OK. Per coordinator rule, not repeated. My proofs: `cargo test --quiet --test it wi_uninstall_cmd::` -> ok. 10 passed; 0 failed; 1 ignored; `installed::` -> ok. 16 passed (existing missing_uninstaller tests green). Clippy errors filtered to my paths: none.

Mutation proofs:
- Quiet before MSI: [OK] mutação PEGA: 'if d.windows_installer && pack_guid(&d.key).is_some() {' -> '... && non_blank(&d.quiet_uninstall_string).is_none() {' fez o teste falhar (exit 101).
- nsis ignored: [OK] mutação PEGA: 'if nsis_from_manifest {' -> 'if true {' fez o teste falhar (exit 101).
- _is1 alone: [OK] mutação PEGA: ' && non_blank(&d.inno_app_path).is_some() {' -> ' {' fez o teste falhar (exit 101).
- Live: `cargo test --release --quiet --test it live_split_matches_shell -- --ignored` -> test result: ok. 1 passed; 0 failed.

Decisions:
- Live test uses a copy of cmd.exe in a temp dir (named "wingetrs t172 <pid>", with a space): SHEvaluateSystemCommandTemplate returns 0x800401F5 for nonexistent programs, so the brief's literal nonexistent paths cannot work.
- split_command/uninstall_command fall back to the whole string as program when no .exe is found.
- Switch dedupe by whitespace token, case-insensitive; existing casing kept.

Concerns: gate not green due to script_runner.rs (other agent).

## Review fix
Commit: see `git log -1 --grep "review of task 172"`. Added tests: burn by upgrade code alone, `_IS1` case, combined-signal order, control chars in valid_arp_key; live test uses tempfile guard. Gate: clippy failed only at src\install\script_scan.rs:391 (other agent, unused `supported`). wi_uninstall_cmd:: 13 passed.
Mutations (all [OK] mutação PEGA): removing `|| bundle_upgrade_code`; Inno requires !nsis_from_manifest; Burn requires !nsis_from_manifest; removing is_control check.
