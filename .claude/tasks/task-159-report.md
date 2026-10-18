Status: DONE_WITH_CONCERNS

Commits:
3efa4ec feat(install): uninstall and upgrade with manifest hooks and $cmd

Gate:
[OK]   rules 10s
[OK]   rules-selftest 25s
[OK]   fmt 3s
[OK]   clippy 55s
[OK]   test (1663 passed, 85 ignored) 90s
[OK]   gate verde
(gate ran 3 times: 1st/2nd failed only on my own fmt/clippy nits, fixed; the 3rd is green)

Scoop check (scoop-update.ps1, e6aa3b3): the update DOES run the old version's uninstaller
(`Invoke-HookScript pre_uninstall` :351, `Invoke-Installer -Uninstall` :354 which runs uninstaller.file
and then uninstaller.script, `post_uninstall` :377) before installing the new one, with the OLD manifest.
Implemented as Step::UpgradeUninstallHook / Step::UpgradeRunUninstaller (old WRITTEN manifest, never the catalog).

Mutation proofs:
1. state.rs `if crate::cache::sha256_hex(&bytes) != expected_sha256 {` -> `if false {`, test
   uninstall_scripts::uninstall_manifest_sha_mismatch_refuses:
   [OK] mutação PEGA: 'if crate::cache::sha256_hex(&bytes) != expected_sha256 {' -> 'if false {' fez o teste falhar (exit 101).
2. executor.rs: the brief's literal anchor `if has_uninstall_scripts && ctx.runners.scripts.is_none() {` does not exist
   (real text: `if planner::has_uninstall_scripts(&inst.manifest) && ctx.runners.scripts.is_none() {`; not reshaped to match), test
   uninstall_scripts::uninstall_without_consent_refused_before_anything:
   [OK] mutação PEGA: 'if planner::has_uninstall_scripts(&inst.manifest) && ctx.runners.scripts.is_none() {' -> 'if false {' fez o teste falhar (exit 101).
3. executor.rs HookScope::upgrade_uninstall (multi-line AnchorFile because `version: plan.version.clone(),` also appears in
   HookScope::install): `version: plan.version.clone()` -> `plan.upgrade_from.clone().unwrap_or_default()`, test
   uninstall_scripts::upgrade_old_hooks_see_new_version_old_manifest:
   [OK] mutação PEGA: '            op: ScriptOp::Update, ... fez o teste falhar (exit 101).

Decisions:
- Altered written manifest (SHA differs) now FAILS the removal before any step (brief #2); the old behaviour (skip uninstaller with a note) is
  replaced; test skipped_uninstaller_note_when_manifest_was_altered became altered_manifest_refuses_the_removal_before_anything.
- Record with EMPTY manifest_sha256 (rebuilt by `repair`, ops.rs:1193) => installed_manifest returns None (unverifiable, not used) => removal
  without scripts + note, so repair-rebuilt apps still uninstall. Done inside state::installed_manifest.
- plan_install also scans the NEW manifest's own uninstall scripts (check_uninstall_scripts) so a refused script fails at install, not at removal.
- uninstaller.file now runs even when uninstaller.script exists (Scoop Invoke-Installer order) and defaults to the first download name like the
  installer (planner::uninstaller_command, reuses installer_step; .ps1 uninstaller => run through the script host as UninstallerScript, else runner).
- Removal runs hooks synchronously via host.run (execute_uninstall is sync); effects of uninstall hooks use apply_removal_effect (own artifacts only);
  upgrade-time effects go through apply_effect(.., uninstall=true) behind own_uninstall_effect. For a REGISTERED variable only check_env_syntax applies
  (ALLOWED_ENV is for names a script picks, TOOL_HOME-style registered vars would otherwise be refused).
- OLD_HOOKS_RAN warning is appended when at least one old step COMPLETED (a failing old hook already carries its own message).

Concerns:
- Files outside the brief's list were edited (forced by removing refuse_2c / new Step variants / InstalledInfo field): tests/it/install_ps1.rs,
  install_psmodule.rs, install_scripts.rs, install_script_effects.rs, install_written_manifest.rs (InstalledInfo literal), install_manifest.rs,
  manifest_2c.rs, install_ops.rs, install_file_installer.rs (tests that asserted the 2c refusal / altered-manifest skip), and the fixture
  unsup-script__main__7zip.json renamed to refused-script__main__7zip.json (it is no longer refused by the planner for uninstall keys; its
  post_install is still refused by the scanner, covered by scanner_refusal_of_a_real_manifest_script_stops_the_plan).
- ops::plan_uninstall now refuses (ScriptRefused / ManifestMismatch) but there is no ops-level test: ops::exec_ctx still passes installer/scripts None
  until Task 161, so only the executor-level paths are tested. Removal through ops with hook-bearing apps returns NeedsScriptConsent until then.
- Upgrade old uninstaller file is not deleted after running (Scoop deletes unless keep); the old version folder is removed/kept by RemoveOldVersion anyway.
- `.join` with external data written: none new (installer_program/scan_ps1_file reused, which go through safepath); prepare_hook joins fixed names.
- Release exe not touched; no background processes left.

## Review fixes (commit fix(install): review of task 159)
1 altered written manifest: removal => note + artifacts removed, no hook; update => fails "remova e instale de novo". 2 InstallPlan.has_uninstall_scripts + consent in execute_install. 3 rollback message extended. 4 old_manifest uses rec.arch (executor::arch_of_key now pub). 5 InstallPlan.notes (shown as "Aviso:" in Display) when the old manifest is missing. 6 recorded_path() shared. 7 test renamed hook_fixtures_resolve_and_a_missing_hash_is_refused.
Mutations (PEGA): has_hooks consent -> false (install_with_only_post_uninstall_needs_script_consent); state.rs sha check -> if false (uninstall_manifest_sha_mismatch_removes_without_scripts).
Gate: not re-run whole (others' files in flux); cargo clippy --all-targets clean, rustfmt --check clean on my files, module + install_ tests green. Touched tests/it/tui_state.rs, tui_view.rs, cli_plan_summary.rs only to add the two new InstallPlan fields.
