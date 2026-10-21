Status: DONE_WITH_CONCERNS

Commits:
050beca feat: refuse third-party code and user-scope winget removals in an admin terminal at plan time
(`git log 10f9f30..HEAD -- <files>` also lists 573d4d7 "feat(cli): self update from the GitHub release": that one is Task 213's, it touched tests/it/main.rs.)

Gate:
[OK]   rules 16s
[OK]   rules-selftest 49s
[OK]   fmt 5s
[OK]   licenses 16s
[OK]   clippy 101s
[OK]   test (1985 passed, 87 ignored) 133s
[OK]   gate verde

Mutation proofs:
1. Scoop plan refusal removed (only the execution-time refusal left), test 2:
   `mutate.ps1 -Path src/cli/install_cmd.rs -Anchor 'let refusal = admin_gate(plans_admin_refusal(elevated, &plans), f)?;' -Replacement 'let refusal: Option<String> = None;' -TestCommand 'cargo test --quiet --test it admin_terminal::scoop_script_refused_before_confirm'`
   [OK] mutação PEGA: 'let refusal = admin_gate(plans_admin_refusal(elevated, &plans), f)?;' -> 'let refusal: Option<String> = None;' fez o teste falhar (exit 101).
2. MachineX64 refused too, test 7:
   `mutate.ps1 -Path src/winget_install/uninstall.rs -Anchor 'hive: Hive::User,\n            ..' -Replacement 'hive: Hive::User | Hive::MachineX64,\n            ..' -TestCommand 'cargo test --quiet --test it admin_terminal::winget_machine_scope_unchanged'`
   [OK] mutação PEGA: 'hive: Hive::User,   (line cut by the multi-line anchor; exit 101)
3. Token error counted as not elevated, test 9:
   `mutate.ps1 -Path src/install/script_policy.rs -Anchor 'probe().unwrap_or(true)' -Replacement 'probe().unwrap_or(false)' -TestCommand 'cargo test --quiet --test it admin_terminal::token_error_counts_as_elevated'`
   [OK] mutação PEGA: 'probe().unwrap_or(true)' -> 'probe().unwrap_or(false)' fez o teste falhar (exit 101).
Extra (one per gate path):
   [OK] mutação PEGA: 'let refusal = admin_gate(admin_code_refusal(elevated, &items, false), f)?;' -> 'let refusal: Option<String> = None;' fez o teste falhar (exit 101).   (CLI Scoop removal, scoop_removal_refused_before_confirm)
   [OK] mutação PEGA: 'removals.extend(admin_batch_item(app, &label, action));' -> 'removals.push(action);' fez o teste falhar (exit 101).   (TUI batch, batch_skips_only_the_blocked_item)
   [OK] mutação PEGA: 'Ok(plan) => match admin_filter(app, Action::WingetUninstall(plan)) {' -> 'Ok(plan) => match Some(Action::WingetUninstall(plan)) {' fez o teste falhar (exit 101).   (TUI winget, cli_and_tui_same_refusal_text)
   [OK] mutação PEGA: 'let (groups, refusals) = admin_split(groups, elevated, f)?;' -> 'let (groups, refusals) = (groups, Vec::<String>::new());' fez o teste falhar (exit 101).   (CLI upgrade --all, batch_skips_only_the_blocked_item)

Output of test 2 (`-- --nocapture`, CSI stripped):
```
Instalar x 1.0  main · sem arquitetura
  1. Comandos: x
  2. PATH do usuário -> C:\Users\jonyd\AppData\Local\Temp\.tmpCXjdWs\root\shims
["install", "x", "--allow-scripts"] => script post_install é código de terceiro e o wingetrs não o roda como administrador; abra um terminal sem administrador e repita
Instalar x 1.0  main · sem arquitetura
  1. Comandos: x
  2. PATH do usuário -> C:\Users\jonyd\AppData\Local\Temp\.tmpCXjdWs\root\shims
["install", "x", "--yes", "--allow-scripts"] => script post_install é código de terceiro e o wingetrs não o roda como administrador; abra um terminal sem administrador e repita
Instalar x 1.0  main · sem arquitetura
  1. Comandos: x
  2. PATH do usuário -> C:\Users\jonyd\AppData\Local\Temp\.tmpCXjdWs\root\shims
["install", "x"] => script post_install é código de terceiro e o wingetrs não o roda como administrador; abra um terminal sem administrador e repita
test result: ok. 1 passed
```

Gate paths and their tests: CLI install (test 2, 4, 9), CLI Scoop removal (scoop_removal_refused_before_confirm), CLI upgrade/upgrade --all with the other groups going on (test 5), CLI winget removal and its dry run (tests 6, 4, 7), TUI single plan (test 8), TUI batch (test 5), TUI winget removal (tests 7, 8, 9). Not covered by a test: the portable package path (`ProdFlows::install_portable` -> `apply_install`, production only) and the TUI portable-with-agreements branch (`on_portable_planned`, same `admin_filter` call).

Decisions:
- Injection: CLI through a new `WingetFlows::elevated() -> fn() -> io::Result<bool>` (default `script_policy::not_elevated` for the test fakes; `ProdFlows` returns `launch_win::is_elevated`); TUI through a new `App.elevated` field (default not elevated in `App::new`, set to `is_elevated` in `src/tui/run.rs`, which is outside the Files list: one line, the only place production builds the App). This avoided changing `OpsCtx`/`Runners`, which are built in 8 test files.
- The TUI check lives in `app.rs` (on_planned, on_portable_planned, on_batch_planned, on_winget_uninstall_planned), not in `jobs.rs`: jobs.rs was not changed.
- `what` = the labels of `ThirdParty::label()` joined with ", ", plus `innounp` when the plan has `Step::ExtractInno` (the execution refuses innounp elevated too).
- Dry-run warning: Scoop via `ui.output(&output::warning(..))` (the only stdout path a test can capture in-process), winget via `say`. output.rs not touched. Batch skip text: `"<apps> pulado: <refusal>"` (CLI on stderr, TUI as a warning toast). If every group is refused, the CLI fails with the first refusal.
- Tests 2-9 were written after the implementation, so the "see them fail" step is shown by the mutations above, not by a red run before the code.

Concerns:
- Item 6 (bridge upgrade of a user-scope winget package from an admin terminal): not touched, not measured. The code path (`upgrade_bridge`) has no hive check, so the same 0x8A15007D refusal by winget probably applies there.
- A user-scope winget record with shared data would also offer "remove only the registry record" (no winget). The plan-time refusal now blocks that option too in an admin terminal.
- `.join` with external data: none written.

## Review fixes (CHANGES_REQUESTED)

Commit: 3ee48ef fix: admin terminal review: innounp test, record-only removal, removal-time code, upgrade plan before refusal

Gate:
[OK]   rules 10s
[OK]   rules-selftest 32s
[OK]   fmt 3s
[OK]   licenses 6s
[OK]   clippy 40s
[OK]   test (1994 passed, 88 ignored) 105s
[OK]   gate verde

Changes:
1. innounp: new test `innounp_refused_in_plan` (manifest `innosetup: true` + `.exe`, innounp published in the catalog because the planner adds it as a dependency). CLI: the error text, `asked()==0`, no request, nothing extracted. TUI: the same text as an error toast.
2. HKCU record with the winget route and `registry_only_allowed`: CLI (interactive, no `--yes`) shows a picker with only "remove only the record" (the refusal is its note) and "cancel". `--yes` or no terminal still fails with the refusal. TUI: the confirmation opens with only `Action::WingetRegistryOnly` (no item that calls winget), and the refusal is a note. It always opens. Test: `winget_user_scope_offers_only_the_record`.
3. Install: the refusal counts only what runs now. `plans_admin_refusal` reads `plan_third_party` of a copy with `third_party_uninstaller`/`has_uninstall_scripts = false`. The installed version's removal code that an upgrade runs now (`Step::UpgradeUninstallHook`/`UpgradeRunUninstaller`, built by `planner.rs:311/320`) is still counted because it comes from the steps. Removal refuses as before. Tests: `install_ignores_code_that_runs_only_at_removal`, `upgrade_counts_the_old_versions_removal_code`.
4. Upgrade: when every group is refused, the plans are shown first, then the error. Test `upgrade_shows_plan_before_refusal` captures the plan through the new `Ui::plan` method. Its default is `emit::out_block`, so production output is unchanged; `print_plans` now goes through it.

Mutation proofs:
[OK] mutação PEGA: 'labels.chain(inno.then(|| "innounp".to_string()))' -> 'labels.chain(None::<String>)' fez o teste falhar (exit 101).   (innounp_refused_in_plan)
[OK] mutação PEGA: 'has_uninstall_scripts: false,
            ..p.clone()' fez o teste falhar (exit 101).   (-> 'has_uninstall_scripts: p.has_uninstall_scripts'; install_ignores_code_that_runs_only_at_removal)
[OK] mutação PEGA: 'has_uninstall_scripts: false,
            ..p.clone()' fez o teste falhar (exit 101).   (-> adds a filter that drops Step::UpgradeUninstallHook; upgrade_counts_the_old_versions_removal_code)
[OK] mutação PEGA: '    if let Some(r) = split.fail {
    third_party_gate' -> '    third_party_gate' fez o teste falhar (exit 101).   (upgrade_shows_plan_before_refusal)
[OK] mutação PEGA: 'if !record_only || f.yes || !ui.interactive() {' -> 'if true {' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'if !record_only || f.yes || !ui.interactive() {' -> 'if !record_only || !ui.interactive() {' fez o teste falhar (exit 101).
[OK] mutação PEGA: '.is_some_and(|s| s.registry_only_allowed)' -> '.is_some_and(|_| false)' fez o teste falhar (exit 101).   (TUI record-only)
(The upgrade mutation was INCONCLUSIVE on its first run because the test failed without the mutation, probably while another agent's build was in progress. Re-run alone: PEGA, above.)

Concern: the CLI record-only path then runs the existing `run_removal(RegistryOnly)`, which is not exercised by these fakes. The test cancels at the picker.
