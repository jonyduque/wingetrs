Status: DONE_WITH_CONCERNS

Commits:
ba4345c feat(winget-install): exit code classification (success, expected, defaults per type)

Gate: rules/rules-selftest/fmt OK; clippy FAIL only in another agent's file: `tests\it\wi_switches.rs:12:5: error: used unwrap() on a Result value`. Own proof: `cargo test --quiet --test it wi_codes::` -> 10 passed; `cargo clippy --all-targets` has no diagnostic mentioning codes.rs / wi_codes.rs.

Mutation proofs (scripts/mutate.ps1, test cmd `cargo test --quiet --test it wi_codes::`; each caught, file restored):
1. defaults before manifest expected -> `wi_codes::manifest_expected_beats_default` FAILED (assertion failed: failed(Category::ContactSupport)).
2. `e.installer_return_code as u32 == code` -> `e.installer_return_code == code as i64` -> `wi_codes::compares_as_u32` FAILED (BlockedByPolicy assertion).
3. `if msiexec {` -> `if true {` -> `wi_codes::uninstall_only_zero` FAILED (classify_uninstall(false, 3010) not Failed).
(The literal "[OK] mutação PEGA" line was cut by `tail -4`; failing test output above is the real result.)

Tables (docs/reference/winget-cli/AppInstallerCommonCore_Manifest_ManifestCommon.cpp): Msi/Wix/Burn :1094-1116; Inno :1118-1125; Msix :1126-1144; category names :345-424. Source matches all brief-required entries. MSIX numeric values (15603.. etc.) taken from windows-sys constants in the cargo registry.

Decisions: Msix/Appx kinds use the MSIX table in classify_install; classify_msix returns Ok for hresult >= 0 and Unknown (with hex) otherwise; Custom message generic.
Concerns: clippy failure in wi_switches.rs (not mine); orchestrator reruns gate.
