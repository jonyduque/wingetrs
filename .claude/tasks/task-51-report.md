Status: DONE

Commits:
```
77245ab fix(executor): reinstall a version whose folder was kept on disk
eeb6a62 docs(plan): task 53 (TUI navigation never waits for details)   <- orchestrator's, not mine
```

Gate:
```
[OK]   rules 7s
[OK]   rules-selftest 30s
[OK]   fmt 3s
[OK]   clippy 149s
[OK]   test (462 passed, 16 ignored) 198s
[OK]   gate verde
```
(Earlier runs failed on Task 50's in-progress `extract.rs` fmt, then once on `index_file::search_retries_while_the_file_is_briefly_locked` (timing, unrelated, load from parallel builds); the next run above was green.)

Mutation proofs:
1. Substituição (refuse every existing folder):
   `scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'if current_points_to(&layout.current(app), &vdir) {' -Replacement 'if true {' -TestCommand 'cargo test --quiet --test install_executor reinstalling_a_kept_version_replaces_its_folder'`
   `[OK] mutação PEGA: 'if current_points_to(&layout.current(app), &vdir) {' -> 'if true {' fez o teste falhar (exit 101).`
2. Rollback (drop the RestoreAside push):
   `scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'run.push(Undo::RestoreAside {' -Replacement 'let _ = (Undo::RestoreAside {' -TestCommand 'cargo test --quiet --test install_executor failure_after_replacing_a_kept_version_restores_it'`
   `[OK] mutação PEGA: 'run.push(Undo::RestoreAside {' -> 'let _ = (Undo::RestoreAside {' fez o teste falhar (exit 101).`
3. Limpeza (no removal of .replaced-* after success):
   `scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'remove_replaced_folders(&run);' -Replacement '' -TestCommand 'cargo test --quiet --test install_executor reinstalling_a_kept_version_replaces_its_folder'`
   `[OK] mutação PEGA: 'remove_replaced_folders(&run);' -> '' fez o teste falhar (exit 101).`
4. Proteção do alvo de current (comparison removed):
   `scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'if current_points_to(&layout.current(app), &vdir) {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test install_executor promote_still_refuses_the_folder_current_points_to'`
   `[OK] mutação PEGA: 'if current_points_to(&layout.current(app), &vdir) {' -> 'if false {' fez o teste falhar (exit 101).`
5. Rollback snapshot geral: no mutation; `failure_at_any_step_restores_the_initial_state_exactly`, `failure_during_upgrade_keeps_v1_current_and_user_data` and `upgrade_with_keep_old_keeps_v1` green in the gate (27/27 in install_executor).

Decisions:
- Normalization for "is current's target": `fs::canonicalize` of both `link::target(current)` and `vdir`; a broken/missing link never matches (so the folder is replaced).
- Cleanup scans the journal for `Undo::RestoreAside` after `remove_upgrade_leftovers` instead of adding a field to `Run`.
- Aside path built in executor.rs (`app_dir.join(".replaced-<v>-<pid>-<seq>")`, static `ASIDE_SEQ: AtomicU64`); layout.rs not touched (not in Files).
- Tests: added `v1_manifest: String` to the test `World` (set in `setup`) so the 1.0 manifest is re-planned with the same url/sha; zip bytes are not guaranteed deterministic, so recomputing the sha was avoided.
- Test 2 passes on the base too (Promote failed before replacing); it only gains meaning with the fix — mutation 2 proves it now guards the rollback.

Concerns: none.

## Review follow-up (3 Minors)

Commit: `3c29aa7 fix(executor): sweep stale replaced folders and pin the rollback step`

Gate: `[OK]   test (467 passed, 16 ignored) 36s` / `[OK]   gate verde` (earlier tries failed only on Task 49's `shim.rs` fmt and the timing test `index_file::search_retries_while_the_file_is_briefly_locked`).

Mutation proofs:
- `[OK] mutação PEGA: 'if current_points_to(&layout.current(app), &vdir) {' -> 'if true {' fez o teste falhar (exit 101).` (failure_after_replacing_a_kept_version_restores_it; panics at the `index == k` assertion)
- `[OK] mutação PEGA: 'if let Undo::RestoreAside { aside, .. } = u' -> 'if let Undo::RestoreAside { original: aside, .. } = u' fez o teste falhar (exit 101).` (reinstalling_a_kept_version_replaces_its_folder)
- `[OK] mutação PEGA: 'if fs::symlink_metadata(&aside).is_err() {' -> 'if true {' fez o teste falhar (exit 101).` (replaced_name_already_taken_picks_another)
- `[OK] mutação PEGA: 'if !stale || current_points_to(&current, &p) {' -> 'if true {' fez o teste falhar (exit 101).` (stale_replaced_folders_are_swept_after_success)
- `[OK] mutação PEGA: '.is_some_and(|n| n.starts_with(REPLACED_PREFIX));' -> '.is_some();' fez o teste falhar (exit 101).` (same test; the real version folder must stay)

Decisions:
- The stale `.replaced-*` sweep runs **after success** (with the removal of this run's own asides), not at Extract with `.staging-*`. Sweeping before Promote would make the name-collision retry unreachable, and nothing would test it. It is warn-only, matches only names starting with `.replaced-`, and skips whatever `current` points to.
- Collision: `free_aside` tries up to 64 `pid-seq` names and skips any that exist. The test pre-creates seq 0..31 for this pid, because the counter is shared by every test in the binary.
- The journal loop over `RestoreAside` is now redundant with the sweep. I kept it because it removes the exact aside even if the directory listing fails.
