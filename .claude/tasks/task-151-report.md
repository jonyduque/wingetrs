Status: DONE_WITH_CONCERNS

Commits:
a041be4 feat(install): run installer/uninstaller file and args (S-file), gated by consent

Gate:
[OK]   rules 6s
[OK]   rules-selftest 14s
[OK]   fmt 3s
[OK]   clippy 29s
[OK]   test (1409 passed, 81 ignored) 83s
[OK]   gate verde
(An earlier run had 2 failures in old tests that asserted the old refusal; fixed, see Decisions. The gate ran twice for that reason, plus once for a clippy collapsible-if.)

Mutation proofs:
[OK] mutação PEGA: '["$dir", "$global", "$version"]' -> '["$dir", "$global", "$versionX"]' fez o teste falhar (exit 101).   (installer_args.rs, test args_substitute_only_dir_global_version)
[OK] mutação PEGA: 'if needs_consent && ctx.runners.installer.is_none() {' -> 'if false {' fez o teste falhar (exit 101).   (executor.rs, no_consent_fails_before_anything)
[OK] mutação PEGA: 'if !keep {' -> 'if true {' fez o teste falhar (exit 101).   (executor.rs, runs_with_consent_and_deletes_file, falha em keep=true)

Joins with external data (all through safepath::safe_components):
- executor.rs `installer_program` (install and uninstall): `vdir.extend(safe_components(file))`, plus canonicalize + starts_with(vdir) (links).
- plan.rs describe `Step::RunInstaller`: `version_dir.extend(safe_components(file))`.
- planner.rs `installer_step`: rejects file with `safe_components(..).is_none_or(empty)` (UnsafePath).
- The manifest parser already rejects `..` in installer.file/uninstaller.file.
- Child PATH = System32 (KnownFolder) + the program's folder (inno::child_env); program is an absolute path inside the version folder.

Decisions:
- Args are substituted/split at plan time (Step::RunInstaller.args is final); uninstall does it at execution with the installed version dir.
- Uninstall: installed manifest missing / hash mismatch / unresolvable / uninstaller without file or with script ⇒ warn and remove as before (not an error). Reason: existing plans use manifest_sha256 "" and a blocked uninstall is worse than skipping.
- Doc-less exit text includes the last output line of the installer; AwaitWorker (staging path, nonexistent after Promote) reused for cancel safety.
- Edited tests outside the Files list because they were forced: install_inno.rs (Runners literal needs `installer: None`), install_planner.rs (exhaustive Step match), install_manifest.rs and manifest_2c.rs (fixtures `{"file":"a.exe"}` asserted the old refusal; changed to `a.ps1`, same intent).

Concerns:
- Not covered by tests: installer_win::run itself (only compiled; no real installer run), `.cmd`/.bat argument refusal path.
- Own test of "no consent" for uninstall is inside uninstaller_runs_before_removal (not a separate fixture).
- Mid-task the lib/tests briefly failed to compile from Task 180's in-flight files (bridge/*, elevated_install.rs); gate passed once they settled.

## Review fixes (commit fix(install): review of task 151)
1. `InstallPlan.third_party_uninstaller` (set by the planner when the manifest has an uninstaller); `needs_consent` looks at it. Consent error text is now neutral ("precisa da sua confirmação para rodar ... (fase 2c, Task 161)"). Test uninstaller_alone_needs_consent_to_install.
2. Test unsafe_default_file_or_empty_file_is_unsafe_path; mutation `if false {` in planner.rs: [OK] mutação PEGA (exit 101).
3. Test installer_through_junction_out_of_version_dir_refused (hook creates the junction before the step).
4. `execute_uninstall` now returns `Result<Vec<String>, ExecError>` (notes); 3 tests, one per skipped branch. ops::uninstall only logs the notes (tracing::warn): showing them in CLI/TUI needs a later change to the ops return type.
5. Test installer_win_runs_a_cmd_with_fixed_path_cwd_and_args (cfg(windows)).
6. Removed the structural CALLS == 0 asserts; refusal tests now check the message with a fake that would answer "código 3".
Gate: fmt FAIL only in src/bridge/worker.rs (Task 180's); my files are rustfmt/clippy clean and `cargo test` passes except elevated_install::exe_params_allowlist (Task 180).
