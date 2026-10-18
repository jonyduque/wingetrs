Status: DONE

Commits:
1900fa0 feat(winget-install): native upgrade (in place, uninstall first, deny) and engine choice

Gate:
[OK]   rules 9s
[OK]   rules-selftest 26s
[OK]   fmt 3s
[OK]   clippy 41s
[OK]   test (1637 passed, 85 ignored) 96s
[OK]   gate verde

Mutation proofs:
1. Install before uninstall (anchor `let previous = previous.ok_or(UpgradeError::NoUninstallPlan)?;` -> `install(wctx, plan).await?; let previous = ...`), test uninstall_previous_order:
[OK] mutação PEGA: '...' fez o teste falhar (exit 101).
2. No RequireExplicitUpgrade filter (`.filter(|r| !explicit_only(r))` -> always true), test explicit_upgrade_excluded_from_all:
[OK] mutação PEGA: '.filter(|r| !explicit_only(r))' -> '.filter(|r| { let _ = explicit_only(r); true })' fez o teste falhar (exit 101).
3. Winget respects the engine refusal (`WingetEngine::Winget => planned.map(|_| Engine::Bridge(None))`), test engine_setting_winget_always_bridge:
[OK] mutação PEGA: 'WingetEngine::Winget => Ok(Engine::Bridge(None)),' -> 'WingetEngine::Winget => planned.map(|_| Engine::Bridge(None)),' fez o teste falhar (exit 101).

Decisions:
- Equivalence test (10) found a real gap: winget skips `deny` installers while another fits; the planner chose first. Added `engine::plan_upgrade` (drops denied installers, refuses only if all deny) and the test uses it.
- `upgrade` takes (WingetCtx, UninstallCtx, plan with upgrade_from, Option<UninstallPlan>), returns typed `UpgradeError` (thiserror).
- Added a guard: offered version must be greater than installed (core::version; tests for greater, smaller, same with extra zeros).
- UninstallFirst installs with a clone of the plan rebuilt without the `Upgrade` switch (fresh install).
- Extra tests: route case-insensitivity, no-previous error, uninstall-first without switch.

Concerns: none. Engine-choice wiring into CLI/TUI is not part of this task. Another agent's transient lib compile error appeared once in clippy and disappeared on rerun.
