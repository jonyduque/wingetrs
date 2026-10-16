Status: DONE_WITH_CONCERNS

Commits:
bfaccfa feat(install): msiexec /a fallback for unsupported MSI, confirmed per item and queued

Gate:
```
[OK]   rules 3s
[OK]   rules-selftest 12s
[OK]   fmt 2s
[OK]   clippy 15s
[OK]   test (980 passed, 24 ignored) 140s
[OK]   gate verde
```

Mutation proofs:
1. Fallback without authorization (Check 1):
   `pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'let Some(m) = msiexec else {' -Replacement 'let Some(m) = msiexec.or(Some(Msiexec::DISABLED)) else {' -TestCommand 'cargo test --quiet --test install_executor unsupported_msi_needs_msiexec'`
   `[OK] mutação PEGA: 'let Some(m) = msiexec else {' -> 'let Some(m) = msiexec.or(Some(Msiexec::DISABLED)) else {' fez o teste falhar (exit 101).` (unsupported_msi_needs_msiexec --- FAILED)
2. `--yes` authorizes (Check 2):
   `pwsh -File scripts/mutate.ps1 -Path src/cli/install_cmd.rs -Anchor 'if self.allow_msiexec {' -Replacement 'if self.allow_msiexec || self.yes {' -TestCommand 'cargo test --quiet --test cli_install yes_does_not_allow_msiexec'`
   `[OK] mutação PEGA: 'if self.allow_msiexec {' -> 'if self.allow_msiexec || self.yes {' fez o teste falhar (exit 101).` (yes_does_not_allow_msiexec --- FAILED)
3. Fallback offered for an integrity error (Check 3):
   `pwsh -File scripts/mutate.ps1 -Path src/install/extract.rs -Anchor 'matches!(e, ExtractError::Unsupported(_))' -Replacement 'matches!(e, ExtractError::Unsupported(_) | ExtractError::MsiLayout(_))' -TestCommand 'cargo test --quiet --test install_executor integrity_error_never_falls_back'`
   `[OK] mutação PEGA: 'matches!(e, ExtractError::Unsupported(_))' -> 'matches!(e, ExtractError::Unsupported(_) | ExtractError::MsiLayout(_))' fez o teste falhar (exit 101).` (integrity_error_never_falls_back --- FAILED)

Extra (my own, beyond the checks):
- Cancel guard: `-Path src/install/executor.rs -Anchor 'if self.handle.take().is_none() {' -Replacement 'if self.handle.take().is_some() {'` vs `cancelled_install_waits_for_the_extraction_worker` -> `[OK] mutação PEGA ...` (test FAILED)
- Admin copy skip: `-Path src/install/extract.rs -Anchor 'if comps.is_empty() && name.eq_ignore_ascii_case(admin_msi) {' -Replacement 'if false {'` vs `allowed_msiexec_extracts_via_fake` -> `[OK] mutação PEGA ...`
- Second Ctrl+C quits an uncancellable job: `-Path src/tui/app.rs -Anchor 'if ctrl_c && (quit_asked || app.quit_after_running) && !job_uncancellable(app) {' -Replacement 'if ctrl_c && (quit_asked || app.quit_after_running) {'` vs `quit_waits_for_uncancellable_job` -> `[OK] mutação PEGA ...`
- Per-item authorization: `-Path src/install/ops.rs -Anchor 'AllowMsiexec::App(a) => a.eq_ignore_ascii_case(app),' -Replacement 'AllowMsiexec::App(_) => true,'` vs `run_action_needs_msiexec_then_runs_the_allowed_item_only` -> `[OK] mutação PEGA ...`
- Eligibility, OLE: `-Anchor '&& is_ole(file).unwrap_or(false)' -Replacement ''` vs `only_an_msi_the_extractor_refuses_falls_back` -> `[OK] mutação PEGA ...`
- Eligibility, MSI kind: `-Anchor '&& matches!(ArchiveKind::of(file_name), Ok(ArchiveKind::Msi))' -Replacement ''` -> first SOBREVIVEU (the `.7z` case used a non-OLE file); fixture changed to an MSI named `.7z`, then `[OK] mutação PEGA ...`

Decisions:
- Authorization travels per call (`ops::apply_allowing`/`apply_groups_allowing(.., &AllowMsiexec)`); the launcher is `OpsCtx::msiexec` (fn pointers, so it can move into `spawn_blocking`). The TUI carries it as `Action::ApplyMsiexec { groups, app }`: the groups not applied that contain `app` wait for the confirmation; other unapplied groups are re-queued at once as `Apply`.
- "Uncancellable" = the running action is `ApplyMsiexec` (the only one that can start msiexec), not only the msiexec window; the second Ctrl+C no longer force-quits it.
- With `WINGETRS_TEST_ENV_KEY` the production launcher is `Msiexec::DISABLED` (errors, never runs msiexec), like the residue scan: the CLI test and its `--yes` mutation can never start a real msiexec.
- `--allow-msiexec` also on `upgrade` (the NeedsMsiexec message tells the user to use it). The package is hard-linked (copy as fallback) into `_msiexec\<manifest file name>`, so the admin copy has a known name to drop.
- Task 128 leftover: `AwaitWorker` wraps the `spawn_blocking` handle; if the install future is dropped, its `Drop` waits for the worker (extraction or msiexec) and only then removes the staging folder.

Concerns:
- Files touched outside the **Files** list (needed to compile or wire): `src/install/extract.rs` (`needs_installer`, `import_admin_tree`, shared `through_temp`/`mapper_for`), `src/install/extract_msi.rs` (`align_case` pub(super)), `src/install/prod_win.rs`, `src/cli/install_cmd.rs`, `tests/cli_parse.rs` + 2 help snapshots, `tests/install_ops.rs` (new `OpsCtx` field). Test 3 is in `tests/cli_install.rs` (has the catalog+wiremock lab), not `tests/cli_offline.rs`.
- The live equivalence test (`tests/install_msi_live.rs`, `#[ignore = "live: msiexec"]`) compiles but was **not run** (it runs msiexec); não medido.
- No fixture for a non-UTF-8 name in the msiexec tree (refused as `UnsafePath`); NTFS names from msiexec are UTF-16, and building an unpaired surrogate name was not attempted.
- A package whose own top-level folder is named `_msiexec` would be merged into the work folder and deleted with it (theoretical; not handled).
- `scripts/leftovers.ps1` lists two processes not started by this task (a `bunx vitest` bash from another project and its python); left alone.

## Review round 1

Commit: `657e155 fix(install): msiexec job uncancellable only once msiexec launches; work dir beside staging`

Gate (the first run failed once in `tests/install_7z.rs::encoded_header_with_huge_unpack_size_refused`, a file this task does not touch; 3 isolated reruns passed, then the full gate):
```
[OK]   rules 3s
[OK]   rules-selftest 10s
[OK]   fmt 2s
[OK]   clippy 2s
[OK]   test (982 passed, 24 ignored) 73s
[OK]   gate verde
```

1. [Important] Uncancellable only from the launch. `extract_with_msiexec` calls `started` on each `msi_queue::Status::Running`. The worker passes that to the async side over a channel, and the side reports it as `progress(INSTALLER_RUNNING, 0, None)`. `jobs::progress_msg` turns that label into `Msg::InstallerRunning`, never throttled; `run.rs` now uses it. `App::installer_running` is set by that message and cleared in `on_finished`, and `job_uncancellable` reads it. New test `msiexec_job_not_launched_yet_still_force_quits`. `quit_waits_for_uncancellable_job` was kept, with `Msg::InstallerRunning` sent after the start.
   - `-Path src/tui/app.rs -Anchor 'app.installer_running && app.running.is_some()' -Replacement 'matches!(app.running.as_ref().and_then(|r| r.action.as_ref()), Some(Action::ApplyMsiexec { .. }))'` -> `[OK] mutação PEGA: ... fez o teste falhar (exit 101).` (msiexec_job_not_launched_yet_still_force_quits --- FAILED)
   - `-Path src/install/executor.rs -Anchor '            started();' -Replacement ''` -> `[OK] mutação PEGA ...` (run_action_needs_msiexec_then_runs_the_allowed_item_only --- FAILED)
   - `-Path src/tui/jobs.rs -Anchor 'if label == crate::install::executor::INSTALLER_RUNNING {' -Replacement 'if label.is_empty() {'` -> `[OK] mutação PEGA ...` (installer_launch_is_never_throttled --- FAILED)
2. [Minor] Precondition documented on `execute_install`/`execute_install_with`. The `AwaitWorker::drop` wait runs in `block_in_place` when the current runtime is multi-thread. That branch has no test (não medido).
3. [Minor] Fixture `fake_admin_non_utf8`: a lone surrogate (0xD800) in a file name refuses with `caminho inseguro no arquivo: pkg/\u{FFFD}a`. The message now has the relative path.
   - `-Path src/install/extract.rs -Anchor '.into_string()' -Replacement '.into_string().or_else(|n| Ok::<String, std::ffi::OsString>(n.to_string_lossy().into_owned()))'` -> `[OK] mutação PEGA ...` (msiexec_tree_with_unsafe_entries_is_refused_and_rolled_back --- FAILED)
4. [Minor] The work dir is now `<staging>.msiexec` beside the staging folder, inside the app folder. A stale one from a crashed run is removed first, and a drop guard removes it on every path out (success, error, panic). `fake_admin` checks the name, and the tests that check the app folder afterwards (`["1.0", "current"]` / folder gone) prove it is removed.

`src/tui/run.rs` (not in the Files list) was touched for item 1.
