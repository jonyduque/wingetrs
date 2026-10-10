Status: DONE_WITH_CONCERNS

Commits:
8a73fc2 fix(install): executor refuses foreign shortcuts, cleans upgrade leftovers, documents broadcast
5d3d84e docs(plan): task 37 owns env broadcast and in_use injection (from task 31)   (orquestrador, não meu)
4142c2b feat(install): executor with staging and full rollback

Gate:
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 3s
[OK]   test (325 passed, 15 ignored) 70s
[OK]   gate verde
(after fix 8a73fc2)
[OK]   rules 2s
[OK]   fmt 1s
[OK]   clippy 7s
[OK]   test (331 passed, 15 ignored) 71s
[OK]   gate verde

Mutation proofs (Checks beyond the steps):
1. LinkCurrent rollback
   pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'Some(target) => link::create(&target, &l).map_err(|e| io_msg(&l, e)),' -Replacement 'Some(_target) => Ok(()),' -TestCommand 'cargo test --quiet --test install_executor failure_during_upgrade_keeps_v1_current_and_user_data'
   [OK] mutação PEGA: 'Some(target) => link::create(&target, &l).map_err(|e| io_msg(&l, e)),' -> 'Some(_target) => Ok(()),' fez o teste falhar (exit 101).
2. Pre-existing persist never deleted on rollback
   pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'if created {' -Replacement 'if true {' -TestCommand 'cargo test --quiet --test install_executor failure_during_upgrade_keeps_v1_current_and_user_data'
   [OK] mutação PEGA: 'if created {' -> 'if true {' fez o teste falhar (exit 101).
3. EnvSet restores previous value
   pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'Some(v) => env.set(name, v),' -Replacement 'Some(_v) => env.remove(name),' -TestCommand 'cargo test --quiet --test install_executor env_set_over_existing_variable_is_restored_on_rollback_and_uninstall'
   [OK] mutação PEGA: 'Some(v) => env.set(name, v),' -> 'Some(_v) => env.remove(name),' fez o teste falhar (exit 101).
4. Skipping the Shim undo
   pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'shim::remove_shim(ctx.layout, &name).map_err(|e| e.to_string())?;' -Replacement 'let _ = &name;' -TestCommand 'cargo test --quiet --test install_executor failure_at_any_step_restores_the_initial_state_exactly'
   [OK] mutação PEGA: 'shim::remove_shim(ctx.layout, &name).map_err(|e| e.to_string())?;' -> 'let _ = &name;' fez o teste falhar (exit 101).

Extra (What binds rules), same script, test in parentheses:
[OK] mutação PEGA: 'if users.is_empty() {' -> 'if true {' (uninstall_keeps_shims_path_while_another_app_uses_it)
[OK] mutação PEGA: '(Some(m), _) | (None, Some(m)) => m.is_dir(),' -> '(Some(_), _) | (None, Some(_)) => spec.kind == PersistKind::Dir,' (persist_dot_dir_in_zip_becomes_junction)
[OK] mutação PEGA: 'if let Some(p) = (ctx.in_use)(&app_dir).first() {' -> 'if let Some(p) = Vec::<PathBuf>::new().first() {' (uninstall_with_file_in_use_changes_nothing)
[OK] mutação PEGA: 'if !path_edit::path_contains(existing, entry) {' -> 'if true {' (path_entry_present_before_is_not_removed_on_rollback)
[OK] mutação PEGA: 'return Err(format!("o comando {} já pertence a {}", spec.name, rec.app));' -> 'let _ = rec;' (shim_of_another_app_is_refused_and_rolled_back)
[OK] mutação PEGA: 'if plan.purge {' -> 'if false {' (purge_removes_persist)
(The surviving RecordState-undo mutant from the first commit is gone: review item 3 removed Undo::RestoreState and added check_order.)

Review fixes (8a73fc2), re-run after the change (anchor of check 4 changed with item 5):
[OK] mutação PEGA: 'return Err(format!("o atalho {target} já pertence a {owner}"));' -> 'let _ = owner;' (shortcut_of_another_app_is_refused_and_rolled_back)
[OK] mutação PEGA: 'if kept {' -> 'if true {' (upgrade_removes_artifacts_the_new_version_no_longer_declares)
[OK] mutação PEGA: '    remove_upgrade_leftovers(&run, ctx);
    for (i, step) in cleanup {' -> '    for (i, step) in cleanup {' (upgrade_removes_artifacts_the_new_version_no_longer_declares)
[OK] mutação PEGA: '    for (i, step) in main {' -> '    remove_upgrade_leftovers(&run, ctx);
    for (i, step) in main {' (failed_upgrade_keeps_artifacts_the_new_version_drops)
[OK] mutação PEGA: 'check_order(&plan.steps).map_err(fail_early)?;' -> '' (record_state_must_be_the_last_main_step)
[OK] mutação PEGA: 'errors.push(format!("{e}"));' -> 'return Err(format!("{e}"));' (shim_undo_failure_is_reported_and_the_rest_is_undone)
[OK] mutação PEGA: 'path_remove_entry(env, &a.target, false)' -> 'path_remove_entry(env, &a.target, true)' (uninstall_removes_path_variable_the_install_created)
[OK] mutação PEGA: 'if let Err(e) = shim::remove_shim(ctx.layout, &name) {' -> 'if let Err(e) = Ok::<(), String>(()) {' (failure_at_any_step_restores_the_initial_state_exactly)  [check 4, new anchor]
[OK] mutação PEGA: 'Some(target) => link::create(&target, &l).map_err(|e| io_msg(&l, e)),' -> 'Some(_target) => Ok(()),' (failure_during_upgrade_keeps_v1_current_and_user_data)  [check 1 re-run]
[OK] mutação PEGA: 'if created {' -> 'if true {' (failure_during_upgrade_keeps_v1_current_and_user_data)  [check 2 re-run]
[OK] mutação PEGA: 'Some(v) => env.set(name, v),' -> 'Some(_v) => env.remove(name),' (env_set_over_existing_variable_is_restored_on_rollback_and_uninstall)  [check 3 re-run]
New tests: shortcut_of_another_app_is_refused_and_rolled_back, upgrade_removes_artifacts_the_new_version_no_longer_declares, failed_upgrade_keeps_artifacts_the_new_version_drops, record_state_must_be_the_last_main_step, shim_undo_failure_is_reported_and_the_rest_is_undone (shims	ool.exe turned into a folder by the hook so removal fails), uninstall_removes_path_variable_the_install_created (24 tests in the file).

Tests (tests/install_executor.rs, 18): rollback at every step (literal), install+uninstall (literal), (a) purge_removes_persist, (b) upgrade_keeps_user_data_switches_current_and_removes_v1 + upgrade_with_keep_old_keeps_v1, (c) failure_during_upgrade_keeps_v1_current_and_user_data (every step k, full snapshot/env/shortcuts/state equality, v1 runs, user.ini intact), (d) env_set_over_existing_variable_is_restored_on_rollback_and_uninstall, remove_old_version_failure_is_only_a_warning, persist_dot_dir_in_zip_becomes_junction, persist_missing_everywhere_creates_empty_dir_and_file, existing_persist_wins_over_the_version_copy, path_entry_present_before_is_not_removed_on_rollback, path_entries_added_on_install_are_removed_on_rollback, shim_of_another_app_is_refused_and_rolled_back, uninstall_keeps_shims_path_while_another_app_uses_it, uninstall_with_file_in_use_changes_nothing, failure_message_lists_undone_steps, unsafe_version_in_plan_is_refused_before_any_change.

Decisions:
- Junctions: new `install::link` (cfg-selected mod: fs_win junction fns on Windows, symlink elsewhere) + `create_junction/junction_target/remove_junction` in fs_win.rs; the executor never names fs_win. Files outside the list: fs_win.rs, mod.rs, link.rs (junction), traits.rs (`FakeEnv::get_raw`, so the literal test compiles).
- Executor never calls `env.broadcast()`: the literal tests compare `FakeEnv::dump()` (which counts broadcasts) to the initial state after install+uninstall. The caller (CLI/TUI) must broadcast once after a successful execute.
- Shim undo restores the previous bytes of `shims\<name>.*` (not just remove), and a shortcut recorded by the app's previous install is recreated on undo; so a failed upgrade leaves v1's shim/shortcut. Env artifact `previous` is carried over from the first install on upgrade; it is stored as text and restored with `expand = contains('%')`.
- `remove_path_entry: true` (from `UninstallPlan::from_state`) means: remove each recorded PATH entry (app entries and the shims dir) when `path_users` is empty at execution time. No separate `InstallReport` type: `ExecError::Failed.undone` carries the undone step labels.
- Test harness: state.db lives outside the snapshotted root (its pages change even when content is restored); `ctx_with_hook` leaks a second StateDb connection and a clone of the hook (the literal `&FailAt(k)` temporary otherwise does not live long enough), hence `#[derive(Clone)]` on FailAt and `#![allow(clippy::drop_non_drop)]` for the literal `drop(ctx)`.

Concerns:
- `AppRecord.manifest_sha256` is written as "" — InstallPlan carries no manifest hash; a later task should add it to the plan.
- (fixed in 8a73fc2) v1 artifacts v2 no longer declares are now removed after success.
- Uninstall (and leftover cleanup) removes the Path variable when it ends up empty; it does not record whether Path existed before the first install (an empty user Path is treated as none).
- Follow-ups (not done, per review item 7): `expand` restored by `contains('%')` (EnvValue.expand is not stored in the artifact); downgrade to a version kept with keep_old (Promote refuses an existing version dir).
- App/version are validated as single safe path components at the start (the planner never validated `version`, which feeds `version_dir`).
- Python edits wrote CRLF in the working copy of executor.rs/plan.rs/traits.rs; git normalizes to LF on commit (autocrlf warning only).
