Status: DONE

Commits:
```
cbd44a1 feat(tui): apply confirm, purge, keep and residue settings
7b9c8af docs(armadilhas): mutation counted as caught by a build failure   (not mine: orchestrator commit that landed on main during the task)
```

Gate (CARGO_TARGET_DIR=target/t69, target/debug/wingetrs.exe was locked):
```
[OK]   rules 2s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 95s
[OK]   test (607 passed, 16 ignored) 107s
[OK]   gate verde
```

Mutation proofs (all with CARGO_TARGET_DIR=target/t69; mutate.ps1 reports compile failures as INCONCLUSIVO, so these are real test failures):
1. Confirmation, `needs_confirm` always true:
   `scripts/mutate.ps1 -Path src/tui/app.rs -Anchor 'pub fn needs_confirm(mode: ConfirmMode, action: &Action) -> bool {' -Replacement '... -> bool { if true { return true; }' -TestCommand 'cargo test --quiet --test tui_state never_confirm_executes_directly'`
   `[OK] mutação PEGA: 'pub fn needs_confirm(mode: ConfirmMode, action: &Action) -> bool {' -> 'pub fn needs_confirm(mode: ConfirmMode, action: &Action) -> bool { if true { return true; }' fez o teste falhar (exit 101).`
   Also `'ConfirmMode::Never => false,' -> 'ConfirmMode::Never => true,'`: `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`
2. Removals only, `Action::Remove` -> `Action::Apply` in the `Removals` arm (test removals_mode_confirms_only_removals):
   `[OK] mutação PEGA: 'ConfirmMode::Removals => matches!(action, Action::Remove(_)),' -> 'ConfirmMode::Removals => matches!(action, Action::Apply(_)),' fez o teste falhar (exit 101).`
3. Picker never skipped, extra `Choose` arm that runs the first candidate under `Never` (test ambiguous_target_still_opens_the_picker):
   `[OK] mutação PEGA: '        Ok(Planned::Choose(candidates)) => {' -> '        Ok(Planned::Choose(candidates)) if app.settings.behavior.confirm == ConfirmMode::Never && !candidates.is_empty() => { let c = &candidates[0]; return vec![Cmd::PlanInstall { ... keep: false }]; }` ... fez o teste falhar (exit 101).`
4. Purge, fixed `false` passed to `ops::plan_uninstall` (test purge_plan_says_data_will_be_deleted):
   `[OK] mutação PEGA: 'ops::plan_uninstall(c, app, purge)' -> 'ops::plan_uninstall(c, app, false)' fez o teste falhar (exit 101).`
5. Keep, fixed `keep: false` on `U` (test upgrade_carries_the_keep_setting). Two mutants:
   selected app: `[OK] mutação PEGA: '                id,\n                keep,' -> '...keep: false,' fez o teste falhar (exit 101).`
   empty selection: `[OK] mutação PEGA: 'Cmd::PlanUpgradeAll { keep })' -> 'Cmd::PlanUpgradeAll { keep: false })' fez o teste falhar (exit 101).`
6. Residues, `High` cleans everything without the `preselected` filter (test high_residue_mode_cleans_only_preselected):
   `[OK] mutação PEGA: 'residues.into_iter().filter(|r| r.preselected).collect()' -> 'residues.into_iter().collect()' fez o teste falhar (exit 101).`

Decisions:
- Plan text: I kept the brief's last line (`são mantidos.` / `serão APAGADOS (purge).`) next to the Task 48 `artifact_lines()`. It doesn't repeat anything: `artifact_lines` only covers recorded Persist artifacts, while purge deletes the whole `persist\<app>` (executor.rs, `if plan.purge`). So test 8 is as the brief wrote it.
- Candidate picker (follow-up from the coordinator): `start_planning` copies `keep` from the outgoing command into `App::planning_keep`. When a `Choose` arrives, that value is stored in `Modal::Picking { keep }`, and picking a candidate sends `PlanInstall { keep }`. So `i` gives false and `U` gives the setting as it was when the key was pressed.
- Running without confirmation uses the new `execute()` helper. That is now the same code the Enter key on Confirm runs: `Modal::Running` with `action.label()` plus `Cmd::Execute`.
- On `High` with nothing preselected, and always on `Keep`, the user gets the toast `resíduos mantidos: {n} itens`, where n is the total number of residues. `ScanInstalled` is always the first command.
- Existing tests in tui_state.rs and tui_jobs.rs updated to the new variant fields and signatures (`keep: false`, `purge: false`, `PlanUpgradeAll { keep: false }`).

Concerns: none. The picker concern is fixed in the follow-up commit (tests picker_after_upgrade_keeps_the_keep_setting and picker_after_install_never_keeps). In one gate run, the unrelated test `install_executor::failure_after_replacing_a_kept_version_restores_it` failed with "Acesso negado (os error 5)" while extracting. It passed when run alone 3 times in a row, and the next gate run passed. Probably a lock from antivirus or the indexer (not measured). target/t69 was removed at the end.

Follow-up gate:
```
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 2s
[OK]   test (609 passed, 16 ignored) 40s
[OK]   gate verde
```

Follow-up mutation proofs:
- `keep: false` fixed in the picker (test picker_after_upgrade_keeps_the_keep_setting):
  `scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '                        id: c.entry.id.clone(),
                        keep,' -Replacement '...keep: false,' -TestCommand 'cargo test --quiet --test tui_state picker_after_upgrade_keeps_the_keep_setting'`
  `[OK] mutação PEGA: '                        id: c.entry.id.clone(), ... fez o teste falhar` (the output line is cut at the line break in the multi-line anchor; exit 101)
- `keep: true` fixed when the picker opens (test picker_after_install_never_keeps):
  `[OK] mutação PEGA: 'keep: app.planning_keep,' -> 'keep: true,' fez o teste falhar (exit 101).`
