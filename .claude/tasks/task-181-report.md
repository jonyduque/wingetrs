Status: DONE_WITH_CONCERNS

Commits:
4de5c43 feat(winget-install): native install flow (download, verify, launch or elevate, classify, ARP/MSIX check)

Gate:
```
[OK]   rules 7s
[OK]   rules-selftest 15s
[OK]   fmt 2s
[OK]   clippy 2s
[OK]   test (1477 passed, 82 ignored) 67s
[OK]   gate verde
```
(The gate ran on the shared checkout, which also had uncommitted changes from Tasks 156/182 in it. All green.)

Module run: `cargo test --quiet --test it wi_install_flow::` → `test result: ok. 20 passed; 0 failed`.

Mutation proofs:
1. Launch without the second hash check (the `verify` call before `msi.lock`/launch removed) → test 1:
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/ops.rs -Anchor '    verify(&mut file)?;' -Replacement '    let _ = &mut file;' -TestCommand 'cargo test --quiet --test it wi_install_flow::hash_mismatch_never_launches'`
   `[OK] mutação PEGA: '    verify(&mut file)?;' -> '    let _ = &mut file;' fez o teste falhar (exit 101).`
2. No "não apareceu" warning:
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/ops.rs -Anchor '(None, None) => Some(NOT_FOUND_WARNING.to_string()),' -Replacement '(None, None) => None,' -TestCommand 'cargo test --quiet --test it wi_install_flow::exit_zero_not_found_warns'`
   `[OK] mutação PEGA: '(None, None) => Some(NOT_FOUND_WARNING.to_string()),' -> '(None, None) => None,' fez o teste falhar (exit 101).`
3. `Helper` for scope `user`:
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/plan.rs -Anchor 'let elevation = install_elevation(&inst.kind, inst.scope.as_deref(), inst.elevation.as_deref());' -Replacement 'let elevation = if inst.scope.as_deref() == Some("user") { Elevation::Helper } else { install_elevation(&inst.kind, inst.scope.as_deref(), inst.elevation.as_deref()) };' -TestCommand 'cargo test --quiet --test it wi_install_flow::user_inno_runs_unelevated_with_filled_log'`
   `[OK] mutação PEGA: 'let elevation = install_elevation(...)' -> 'let elevation = if inst.scope.as_deref() == Some("user") { Elevation::Helper } else { ... };' fez o teste falhar (exit 101).`
4. `find_installed` tries name+publisher first:
   `pwsh -File scripts/mutate.ps1 -Path src/winget_install/post_check.rs -Anchor '    let inst = &plan.installer;' -Replacement '    if let Some(a) = by_name_and_publisher(plan, apps) { return Some(a); } let inst = &plan.installer;' -TestCommand 'cargo test --quiet --test it wi_install_flow::find_installed_order'`
   `[OK] mutação PEGA: '    let inst = &plan.installer;' -> '    if let Some(a) = by_name_and_publisher(plan, apps) { return Some(a); } let inst = &plan.installer;' fez o teste falhar (exit 101).`
5. Real binary: no command uses this flow yet (the CLI comes in Task 186), so only the gate was run.

How test 1 catches mutation 1: `fetch_installer` already rejects bytes that do not match the SHA, so the bytes≠SHA case cannot fail on the second check alone. Test 1 has a second scenario for that. `msi.lock` is held by the test. When the flow reports `WaitingInstaller` (that happens after the verify), a thread tries to write to the materialized installer. With the share-deny handle from `Materialized::verify` the write fails. In the mutant the write works, and the test fails on `!tampered`.

safepath, every `.join` with external data:
- `ops.rs installer_log`: `data.log_dir().join("installers").join(name)`. `name = <id>-<sanitized ver>-<ms>.log` must pass `safe_components` as exactly one component. The id was already checked by `fetch_installer` (`valid_target`).
- `ops.rs run_command` (Helper): `data.blobs_dir().join(sha)`. The sha is the 64-hex value `fetch` validated, and it also goes through `one_component`.
- Everything else joins constants only. `installers\<id>\<ver>` stays inside `fetch.rs` (Task 177).

Test 14, how the consumers were checked: `LineProgress::new(false, ..).update` writes `PhaseLog::line` to stderr, which a test cannot capture in-process. So `update` only has to run without panicking, and the text is checked on `PhaseLog::line` itself (the function `update` calls). The terminal path (`Phases::line_at`) and the TUI (`update` + `view::running_text`) are checked on their actual output.

Decisions:
- `gate.try_launch()` runs right before each launch (after `msi.lock`, inside the 1618 retry loop), as `LaunchGate` documents. A Ctrl+C while `run_queued` is waiting for the mutex still cancels.
- The blocking calls (`lock_msi`, `launcher.run`, `elevated.run_installer`, `msix.add`, the sqlite insert) run inline in the async fn, not in `spawn_blocking`. `WingetCtx` holds `&'a dyn` references, which `spawn_blocking` (`'static`) cannot take. The TUI runs jobs on a blocking thread with `block_on` (`tui/run.rs spawn_job`). Only `install.lock` uses `spawn_blocking`, with an owned path.
- `Helper` path: `WaitingElevation` is emitted before each elevated attempt. `ElevatedRunner` has no "UAC accepted" signal, so this path never emits `RunningInstaller`. The log lines the helper sends back are saved to `<root>\log\installers\<id>-<ver>-<ms>.log` and returned in `InstallReport.log`.
- TUI: `Msg::InstallPhase(RunningInstaller)` also sets `app.installer_running = true` in `app.rs`, so `job_uncancellable` covers the native installer just as it covers `msiexec /a`. MSIX `Deferred` puts `AddResult::user_text` in `warning`, instead of "não apareceu".
- `PlanOrBridge::Native(WingetPlan)` is not boxed, to keep the shape the brief fixes. It uses `#[allow(clippy::large_enum_variant)]`.

Concerns:
- The `Helper` path never emits `RunningInstaller` (see Decisions). If the job layer needs to know when the installer starts, `ElevatedRunner` would need a "started" callback (`src/bridge/*`, owned by Task 182).
- I did not see the first tests fail before writing the code (they were written together with the implementation). The four mutations above show the tests can fail.
- A failed `winget_ops` insert only produces a `tracing::warn`; it is a hint only.

## Review fix (4c47a37 fix(winget_install): review of task 181)
1. `plan()`: when the plan goes through `Helper`, it builds the `InstallerRequest` and runs the Task 180 `check`. The source is a stand-in `C:\wingetrs\blobs\<sha>` and `program_files` is empty. If the check refuses, the result is `PlanOrBridge::Bridge("parâmetros do instalador fora da lista permitida para elevação: <token>")`. `<token>` is the first token the allowlist refuses, found by trying ever longer prefixes of the arguments. `ops` runs `req.check(&[])` again just before `run_installer`. `elevated_kind` moved to `plan.rs`. Tests: inno machine with Custom `/INVENTED` goes to the bridge with exactly that reason; `/NOCANCEL` stays native.
2. `WaitingElevation`: the title is now "Aguardando o UAC ou o instalador elevado; se o UAC não aparecer, clique no escudo...". `phase_status` adds "não pode ser cancelado". The TUI sets `installer_running = true`, which is safe because this phase is only sent after `try_launch`. The test assertion is inverted.
3. `install` has a doc comment: call it only from a dedicated thread or with `block_on` (TUI: `spawn_blocking` + `block_on`; CLI: `block_on`).
4. New test `exit_3010_is_ok_reboot`.
5. New test `msix_cancel_before_launch`.
Mutations:
- `[OK] mutação PEGA: '        None => {` (msix arm: `if !ctx.gate.try_launch()` → `if false`; test `msix_cancel_before_launch`)
- `[OK] mutação PEGA: '&& let Err(why) = helper_accepts(&inst, cmd)' -> '&& let Err(why) = Ok::<(), String>(()).and(Ok(())).map(|()| { let _ = helper_accepts; })' fez o teste falhar (exit 101).` (test `helper_params_outside_allowlist_go_to_bridge`)
Gate: did not go green, and the failures are in other agents' files: `[FAIL] fmt exit 1  Diff in ...src\install\executor.rs:2273`. Clippy fails on `src\install\executor.rs:1394` (unused `persist_dir`) and `src\bridge\worker.rs:1020` (type_complexity).
Evidence for my changes: `rustfmt --check` on my 6 files passes (FMT_OK). `cargo test --quiet --test it wi_install_flow::` → 23 passed. `install_progress::` → 13 passed.
Concern: because `program_files` is empty in the plan, `/DIR=`, `/D=` and `INSTALLDIR` in an elevated plan always go to the bridge, even when the folder is inside Program Files.
