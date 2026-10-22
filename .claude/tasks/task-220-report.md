Status: DONE

Commits:
7bbad77 fix: job log marks every elevated run; cancelled install leaves no new app folder

Gate:
[OK]   rules 11s
[OK]   rules-selftest 44s
[OK]   fmt 3s
[OK]   licenses 7s
[OK]   clippy 167s
[OK]   test (2029 passed, 89 ignored) 121s
[OK]   gate verde

Mutation proofs:
1. plan.rs `plan_runs_elevated` forced to false: `[OK] mutação PEGA: '    plan.elevation == Elevation::Helper ...` (test job_log_native_machine_install_is_elevated falls)
2. executor.rs drop without the `.msiexec` match: `[OK] mutação PEGA: 'name == format!("{staging_name}{MSIEXEC_SUFFIX}") || name.starts_with(&own_temp)' -> 'name.starts_with(&own_temp)' fez o teste falhar (exit 101).` (cancel_during_extract_removes_new_app_dir)
3. executor.rs filter replaced by "everything" (also deletes 0.9): `[OK] mutação PEGA: '.is_some_and(|n| is_dead_run_leftover(n) && is_own_run_leftover(n, staging_name));' -> '.is_some();' fez o teste falhar (exit 101).` (cancel_keeps_installed_versions)

Decisions:
- Decision functions: `plan_runs_elevated` (winget_install/plan.rs) and `removal_runs_elevated(plan, registry_only)` (winget_install/uninstall.rs: uninstaller by `plan.elevation`, registry-only by an HKLM ARP target); used by the CLI and the TUI (`winget_action_elevated` in tui/jobs.rs).
- "Created by this run" = the `.msiexec` folder of this staging name plus extraction temps with this process id (`.wingetrs-extract-<pid>-`), also required to pass `is_dead_run_leftover`.
- The job_log tests (1 and 2) live in tests/it/wi_cli.rs, not job_log.rs, because the fake CLI flows (`Fake`) are private there; names are as the brief says.
- Test 5 (`cancel_during_download_creates_no_app_dir`) is a stricter sibling of the existing `cancel_before_installer_rolls_back`: also asserts the blob folder holds no file.

Concerns: none. The gate ran with other agents' edits in the checkout (it was green); I committed only my files. The TUI side of `elevated` has no own test (only the shared decision functions, covered through the CLI).

Follow-up (review: upgrade `UninstallFirst`): `upgrade_runs_elevated(plan, previous)` in plan.rs, used by CLI (`run_native` takes `elevated`) and TUI (`upgrade_previous` computed before the log flag). Test `job_log_upgrade_with_helper_previous_is_elevated` (wi_cli.rs). Mutation: `[OK] mutação PEGA: ' || previous.is_some_and(|p| removal_runs_elevated(p, false))' -> '' fez o teste falhar (exit 101).`
Gate: rules/selftest OK; fmt FAILED only on another agent's src/cli/mod.rs; my files pass rustfmt --check, clippy clean on them; `cargo test` 1 failure, `transcript::appends_to_existing_file` (217's file, not mine). Gate not re-run.
