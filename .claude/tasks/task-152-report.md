Status: DONE_WITH_CONCERNS

Commits:
ddabe04 feat(install): own PowerShell prelude, group 1 (messages, paths, config)

Gate:
Full gate NOT green because of other agents' files (not mine):
  [OK]   rules / rules-selftest / fmt / clippy 71s (clippy --all-targets clean, includes my files)
  [FAIL] test exit 101: error[E0432] unresolved imports winget_install::launch::{Elevation,LaunchSpec,Launcher,Show,...}, launch_win::{ShellLauncher,is_elevated,msiexec_path}
  error: could not compile `wingetrs` (test "it") due to 3 previous errors
Own proof (before the other agents' tests broke the build):
  cargo test --quiet --test it prelude_core:: -- --ignored  => test result: ok. 13 passed; 0 failed

Mutation proofs:
[OK] mutação PEGA: 'function is_admin { return $false }' -> 'function is_admin { return $true }' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'throw "abort: $msg"' -> 'Write-Host "abort: $msg"' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'function modulesdir' -> '$modulesdir = 1; function modulesdir' fez o teste falhar (exit 101).
(TestCommand used: cargo test --quiet --test it prelude_core::<test> -- --ignored; first attempt was INCONCLUSIVE because of another agent's switches.rs not compiling, re-run clean.)

get_config names used by catalog manifests (read-only immutable query of catalog.db, kind='scoop', raw): NO_JUNCTIONS (19), USE_LESSMSI (6). Table: no_junction/no_junctions => false, use_lessmsi => true, use_external_7zip => false, else $default (case-insensitive).

Decisions:
- All 13 tests are #[ignore = "live: roda pwsh real"] (orchestrator rule); run with `-- --ignored`.
- Non-ASCII message "global não suportado pelo wingetrs" built with [char]0xE3 so the .ps1 stays ASCII; test sets Console OutputEncoding UTF-8.
- Select-CurrentVersion: manifest.json version, else junction target leaf, else newest version dir; installed treats $null global as local; Get-ShimTarget and normalize_values inlined (no extra functions beyond the 24).
- Test 12 passes `'1.0'` quoted (bare 1.0 is a PowerShell number); JSON comparison ignores spaces (5.1 vs 7 ConvertTo-Json spacing).

Concerns:
- Full gate test step blocked by other agents' winget_install::launch work; orchestrator should rerun the gate.
- Git warns LF->CRLF for prelude.ps1 (autocrlf); content stays ASCII.
