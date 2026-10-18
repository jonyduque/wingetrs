Status: DONE_WITH_CONCERNS

Commits:
8b491cc feat(winget-install): default switches per installer type and argument line

Gate: verify.ps1 failed only in other agents' tests (script_scan::iex_strong, spawn_env::decoder_gets_raw_bytes; 1133 passed, 2 failed). Mine: `cargo test --quiet --test it wi_switches::` = 11 passed; clippy --all-targets has no diagnostics in switches paths; check_rules OK.

Mutation proofs (scripts/mutate.ps1, test command `cargo test --quiet --test it wi_switches::`):
- [OK] mutação PEGA: '"/quiet /norestart",' -> '"/quiet",' fez o teste falhar (exit 101).
- [OK] mutação PEGA: 'if upgrade {' -> 'if true {' fez o teste falhar (exit 101).
- [OK] mutação PEGA: Custom antes do Log (parts.extend(log)/custom swapped) fez o teste falhar.

Source lines (docs/reference/winget-cli):
- Defaults msi/wix/burn: AppInstallerCommonCore_Manifest_ManifestCommon.cpp:1042-1044; nullsoft :1050-1051; inno :1057-1059.
- Order mode, Log, Custom, Update: AppInstallerCLICore_Workflows_ShellExecuteInstallerHandler.cpp:107-150 (no scope args for MSI; InstallLocation :153 left out). Experience choice :86-105.

Decisions:
- Whitespace-only manifest switches are treated as absent.
- Burn/Inno/Nullsoft/Exe => Program::Installer; Font/Pwa/Msstore/Unknown also NoCommandLine.
- Mode::Interactive uses only the manifest Interactive switch (as in the source).
- Added `#![allow(clippy::unwrap_used, clippy::expect_used)]` to the test file per coordinator.

Concerns: none for this task (gate failures belong to other agents).
