Status: DONE_WITH_CONCERNS

Commits:
df40385 feat(tui): native winget jobs, UAC status, orphan record removal and engine routing

Gate:
```
[OK]   rules 9s
[OK]   rules-selftest 24s
[OK]   fmt 3s
[OK]   clippy 55s
[OK]   test (1685 passed, 85 ignored) 90s
[OK]   gate verde
```
(gate run once, with the uncommitted work of Task 186 in the tree.)

Tests: `cargo test --quiet --test it wi_tui::` -> `test result: ok. 9 passed; 0 failed` (the 7 of the
brief + `upgrade_native_follows_engine` + `user_folders_ignore_planted_appdata`, which plants a fake
`APPDATA` and checks that `folders_win::user_folders` does not change). `tui_` (344 passed) and `wi_`
(237 passed) also green.

Mutation proofs:
1. Orphan item `checked: true`:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'checked: false,' -Replacement 'checked: true,' -TestCommand 'cargo test --quiet --test it wi_tui::orphan_item_never_prechecked'`
   `[OK] mutação PEGA: 'checked: false,' -> 'checked: true,' fez o teste falhar (exit 101).`
2. `ConfirmMode::Never` skips the orphan confirmation:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '    if matches!(action, Action::RemoveOrphan(_)) {' -Replacement '    if false {' -TestCommand 'cargo test --quiet --test it wi_tui::orphan_item_never_prechecked'`
   `[OK] mutação PEGA: '    if matches!(action, Action::RemoveOrphan(_)) {' -> '    if false {' fez o teste falhar (exit 101).` (fails at the `ConfirmMode::Never` part, wi_tui.rs:285)
3. "Remover só o registro" offered on the principal record:
   `pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'Some(s) => (!s.folders.is_empty(), s.registry_only_allowed),' -Replacement 'Some(s) => (!s.folders.is_empty(), true),' -TestCommand 'cargo test --quiet --test it wi_tui::kindle_items'`
   `[OK] mutação PEGA: 'Some(s) => (!s.folders.is_empty(), s.registry_only_allowed),' -> 'Some(s) => (!s.folders.is_empty(), true),' fez o teste falhar (exit 101).`
Extra (engine respected):
   `-Anchor 'match choose_engine(engine, native.map_err(Refusal)) {' -Replacement 'match choose_engine(WingetEngine::Wingetrs, native.map_err(Refusal)) {' -TestCommand '... wi_tui::engine_winget_routes_bridge'` -> `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`
   `-Anchor '        && app.settings.behavior.winget_engine == WingetEngine::Wingetrs' -Replacement '        && true' -TestCommand '... wi_tui::upgrade_native_follows_engine'` -> `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`
4. Real binary: not opened (subagent without an interactive terminal) — não verificado na TUI. Snapshot changed: `tests/it/snapshots/it__tui_view__actions_modal_snapshot.snap` (ARP row: `Remover` now enabled without the NOT_OURS note, and the `Remover (u)` button appears; header path/line updated by insta). No other snapshot changed.

Decisions:
- `Cmd::PlanWingetInstall` now answers `Msg::WingetInstallPlanned { bridge, native }` (both plans computed in `run.rs`); `App` runs `choose_engine` with the current Settings engine, so the routing is pure and testable. `Msg::WingetPlanned` kept (bridge-only path and old tests). `Engine::Bridge(Some(why))` adds the note `Pelo winget: <why>` (shown only if a confirmation opens).
- Native upgrade: the applicability fetch (already reading the manifest) also runs `plan_upgrade` and sends `Msg::NativeUpgradeLoaded`; `upgrade_target` returns `Target::Native` only with engine `wingetrs` and a plan for the row's current `latest`, else today's bridge (so the bridge tests stay as they were).
- Extra `Action::WingetRegistryOnly(UninstallPlan)` (option C chosen in the confirmation); the Kindle options are confirmation items after the app's item (`item_count` keeps `checked_actions`/`apply_groups` aligned); unchecking "Copiar pastas" clears `shared.folders`. A plan with `shared` always confirms (even with `Never`), like plans with license agreements.
- `Cmd::PlanWingetUninstall` carries `name`, `others` (other ARP app_keys) and `is_main` besides `app_key`; the menu gets `Remover o registro <local>` per other record (the only way to reach a non-principal record) and `Remover registro órfão[ (<local>)]` per stale ARP record. Uninstall/registry-only/orphan jobs take `install.lock`; phases: `WaitingElevation` when the helper runs it, else `Removing`.
- `WingetPlan` and the manifest structs it holds got `PartialEq` (needed by `Action: PartialEq`): `src/winget_install/plan.rs`, `src/sources/winget/manifest.rs` (outside Files, derive only).

Concerns:
- `src/winget_install/folders_win.rs` + its `mod` line: I created it, then Task 186 (parallel) rewrote it with the same API (`user_folders`, `free_space`, header "Task 186"); my commit includes the current (186) content because the TUI needs it to compile. Task 186's commit should not re-add it; please check when reviewing 186.
- `.join` with external data: `env.backups.join(format!("{}-pastas", env.stamp))` (stamp = `now_unix`, not external); the folder names go through `backup_folders` (safepath inside). No other `.join` with external data.
- Marked batch removal stays wingetrs-only: marked ARP/MSIX rows are "pulados" (toast), not removed in batch. MSIX removal plan has no version (not in the Cmd). The NSIS hint is off for a menu removal (no manifest read), as in the CLI.
- The `Pelo winget` reason and native install confirm use the Settings engine at Msg time (not at key time).
- TDD order: the tests were written after the code; the mutations above show they fail on the rules.
- `scripts/leftovers.ps1` lists two `bash.exe` (02:03) and a `conhost` that are not from this task (not killed).

## Review fix (94e00aa fix(tui): review of task 187)
Gate: `[OK] test (1691 passed, 85 ignored)` / `[OK] gate verde`. `wi_tui::` 13 passed.
1. Native plan with licenses always confirms (install and WingetUpgrade, even with Never): `licenses_always_confirm_native`.
   `[OK] mutação PEGA: '            !p.agreements.is_empty()' -> '            false' fez o teste falhar (exit 101).`
2. Bridge fallback with a reason and Never without licenses: toast `Pelo winget: <motivo>` (`bridge_reason_toasts_without_confirmation`).
   `[OK] mutação PEGA: '...app.push_toast(ToastKind::Info, format!("Pelo winget: {why}"));' -> '' fez o teste falhar (exit 101).`
3. `ask_orphan` always opens the modal (the enqueue branch and the orphan line in `needs_confirm` were removed).
   `[OK] mutação PEGA: '    let modal = confirm_modal(app, vec![Action::RemoveOrphan(o)], ...' -> 'if ...Never { return enqueue(...) } ...' fez o teste falhar (exit 101).`
4. Native removals (uninstall, registry only, orphan): `Phase::Removing` comes right after the job's `LaunchGate.try_launch`. From then on the job is uncancellable, and the quit dialog shows `NATIVE_QUIT` (`launched_removal_is_uncancellable`).
   `[OK] mutação PEGA: '                ) || (native_removal' -> '                ) || (false' fez o teste falhar (exit 101).`
5. Quitting (Enter in the dialog, or a second Ctrl+C) while a native job has not launched sends `Cmd::CancelNative`. `run.rs` keeps the job's gate in `Ctx.winget_gate` and calls `try_cancel` on it; `Cmd::Quit` cancels it too (`quitting_cancels_native_job_before_launch`).
   `[OK] mutação PEGA: '        vec![Cmd::CancelNative]' -> '        Vec::new()' fez o teste falhar (exit 101).`
   The test is at App level only (the job side is `ops::install`'s existing gate check plus the new `start_removal`); it does not run with a fake job.
6. `user_folders_ignore_planted_appdata` now plants APPDATA in a child process (this test run again), with no `set_var` in the shared binary.
