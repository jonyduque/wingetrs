Status: DONE_WITH_CONCERNS

Commits:
e2939fd feat(tui): install, uninstall and upgrade with confirmation and progress
(5fc4c1c in the range is the parallel CLI agent's commit, not mine)

Gate:
[OK]   rules 4s
[OK]   fmt 4s
[OK]   clippy 74s
[OK]   test (395 passed, 16 ignored) 20s
[OK]   gate verde

Mutation proofs:
1. winget not installable:
   `scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'if e.source != SourceKind::Scoop {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test tui_state i_on_winget'`
   [OK] mutação PEGA: 'if e.source != SourceKind::Scoop {' -> 'if false {' fez o teste falhar (exit 101).
2. Esc closes the modal:
   mutate of the Confirm `KeyCode::Esc => Vec::new()` arm so it re-opens the modal, test `esc_on_confirm`:
   [OK] mutação PEGA: '            KeyCode::Esc => Vec::new(),   (test esc_on_confirm_closes_without_executing failed: assertion a.modal.is_none())

Decisions:
- `i` acts only on the Search tab, `u` only on Installed, `U` only on Updates; elsewhere those letters still type into the query (limits the "empty query" key hijack).
- `U` upgrades the selected row (must be origin wingetrs + Matched); with no visible row it means "all" (`Cmd::PlanUpgradeAll`).
- `Modal::Picking` and `Modal::Residues` also carry the candidates/residues; Picking is used when `resolve_target` returns `Many`.
- Running ignores all keys including Ctrl+C (status "aguarde"): quitting mid-install would leave the runtime waiting on the blocking task anyway.
- Jobs run in `spawn_blocking` + `Handle::block_on(apply)` (Catalog is not Sync); a worker panic becomes a status message like `spawn_scan` (precedent), the handle is always awaited.

Concerns:
- Real jobs (plan/execute/clean in run.rs) and `ops_win.rs` are untested (need real effects); only state and snapshots are covered. `ops_win::with_ctx` duplicates the production wiring the CLI agent put in `src/cli/prod_win.rs`; could be unified later.
- With an empty query, typing a search that starts with `i` on the Search tab triggers install instead (as the brief specifies, like `j/k`).
- Upgrade-all plans only apps whose catalog entry resolves uniquely; otherwise the whole plan fails with a status message.
- Files touched beyond the list: src/tui/mod.rs (new module), src/tui/ops_win.rs (new).

## Round 2 (review CHANGES_REQUESTED)
Commit: see `git log` (fix(tui): testable jobs, worker panic re-raised, coalesced progress, planning guard)
Gate: [OK]   test (407 passed, 16 ignored) / [OK]   gate verde
1. Worker panic: `jobs::join_job` returns `Msg::Panicked(payload)`; event loop calls `jobs::propagate` (resume_unwind); `run` wraps block_on in catch_unwind, restores the terminal, then resume_unwind. (The ratatui panic hook already ran at the original panic.) Test: a_worker_panic_reaches_the_main_loop_and_is_reraised.
2. Jobs moved to src/tui/jobs.rs taking `&OpsCtx`; tests/tui_jobs.rs with fakes: plan_install Ready/Err, run_action Apply with progress then Remove, held lock => InstallFail::Busy. Choose not testable with a bucket-restricted resolve (one package per id per bucket).
3. `ProgressThrottle` (50 ms, final always passes), test progress_is_coalesced_to_about_twenty_per_second.
4. `App.planning` + status "planejando…"; tests repeated_plan_keys_are_ignored_while_planning, a_failed_plan_frees_the_keys_again.
Mutations (all `[OK] mutação PEGA`): Msg::Panicked resume_unwind -> Tick; OpsError::Busy mapping; `|| total == Some(done)` removed; `if app.planning {` -> `if false {`.
src/tui/ops_win.rs untouched; migrate to the shared builder next round.

## Round 3
Commit: refactor(tui): use the shared production context (src/tui/ops_win.rs deleted; run.rs `with_ctx` now calls prod_win::{ProdEffects, build_ctx} with LockWait::NoWait and the TUI progress). Gate: [OK] test (407 passed, 16 ignored) / [OK] gate verde.
`grep -rn "ResidueScan::" src`:
src/cli/install_cmd.rs:493:            residue: ResidueScan::none(),   (CLI agent's file; ::none() only, not the real scan)
src/install/ops.rs:45: (doc comment)
src/install/prod_win.rs:63: ResidueScan::none()
src/install/prod_win.rs:65: ResidueScan::user_profile(&link_target, &installed_by_scoop)
Only prod_win builds the real scan.
