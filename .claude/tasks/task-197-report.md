Status: DONE_WITH_CONCERNS

Commits:
ef339ae feat(bridge): elevated helper only from an admin-only folder; installer_mode in TUI install plan

(Task 196's commit 7b80c5e staged the whole of `tests/it/wi_tui.rs`, so it already contains my addendum test `installer_mode_same_switches_in_cli_and_tui_plans`. The functions that test calls (`tui::run::native_install_on`, `cli::winget_cmd::install_plan`) only landed in ef339ae, so the `it` tests do not compile at 7b80c5e on its own. They do compile at ef339ae.)

Gate (one full run, then a second run for the step lines):
```
[OK]   rules 12s
[OK]   rules-selftest 36s
[OK]   fmt 5s
[OK]   clippy 22s
[FAIL] test exit 101
  snapshot assertion for 'settings_screen_scrolls_to_the_last_row_in_24_lines' failed in line 1347
  snapshot assertion for 'settings_screen_snapshot' failed in line 1338
  test result: FAILED. 1721 passed; 2 failed; 85 ignored; 0 measured; 0 filtered out; finished in 53.65s
```
Both failures are `tui_view::settings_screen_*` snapshots. The new row in the diff is Task 198's `+"│   Janela do instalador (winget)  ‹ mostrar progresso ›  sem janela"`. I did not touch them. My own modules:
```
cargo test --quiet --test it elevated_worker::  -> test result: ok. 21 passed; 0 failed; 1 ignored
cargo test --quiet --test it wi_tui::           -> test result: ok. 16 passed; 0 failed; 0 ignored
```
I also ran `cargo clippy --all-targets` filtered by my paths: no findings.

Mutation proofs:
- Removing the check before runas:
  `mutate.ps1 -Path src/bridge/elevate_win.rs -Anchor 'hold_admin_chain_with(exe, sddl_of)' -Replacement 'hold_admin_chain_with(exe, sddl_of).or_else(|_| Ok::<_, std::io::Error>(Vec::new()))' -TestCommand 'cargo test --quiet --test it elevated_worker::helper_folder::win::worker_refused_from_user_writable_folder'`
  `[OK] mutação PEGA: 'hold_admin_chain_with(exe, sddl_of)' -> 'hold_admin_chain_with(exe, sddl_of).or_else(|_| Ok::<_, std::io::Error>(Vec::new()))' fez o teste falhar (exit 101).`
- Dropping the hold before runas returns:
  `mutate.ps1 -Path src/bridge/elevate_win.rs -Anchor '    let name = format!("{PIPE_PREFIX}{}", random_hex()?);' -Replacement '    drop(held); let held = (); let name = ...' -TestCommand '... worker_allowed_from_admin_folder'`
  `[OK] mutação PEGA: '    let name = format!("{PIPE_PREFIX}{}", random_hex()?);' -> '    drop(held); let held = (); let name = format!("{PIPE_PREFIX}{}", random_hex()?);' fez o teste falhar (exit 101).`
- Reparse point not refused:
  `mutate.ps1 -Path src/bridge/arp_ops_win.rs -Anchor '        return Err(not_admin(path, "ponto de reanálise"));' -Replacement '        let _ = not_admin(path, "ponto de reanálise");' -TestCommand '... worker_refused_through_junction'`
  `[OK] mutação PEGA: '        return Err(not_admin(path, "ponto de reanálise"));' -> '        let _ = not_admin(path, "ponto de reanálise");' fez o teste falhar (exit 101).`
- Notice shown only once per session:
  `mutate.ps1 -Path src/bridge/helper_folder.rs -Anchor '!self.told.swap(true, Ordering::SeqCst)' -Replacement 'true' -TestCommand '... elevation_refusal_text_same_in_cli_and_tui'`
  `[OK] mutação PEGA: '!self.told.swap(true, Ordering::SeqCst)' -> 'true' fez o teste falhar (exit 101).`
- Addendum, TUI ignoring `installer_mode`:
  `[OK] mutação PEGA: 'plan_with_mode(manifest, host, wanted, None, installer_mode)' -> 'plan_with_mode(manifest, host, wanted, None, Default::default())' fez o teste falhar (exit 101).`
- Addendum, CLI ignoring `installer_mode`:
  `[OK] mutação PEGA: '        behavior.installer_mode,' -> '        Default::default(),' fez o teste falhar (exit 101).`
- Not covered:
  `[X] mutação SOBREVIVEU: '        helper_notice.show(&mut app);' -> '' e os testes passaram.` That line is the call inside the async `event_loop`, which no test reaches. See Concerns.

Decisions:
- I reused the Task 182 chain check (`arp_ops_win::hold_admin_chain`) instead of `staging_win::hold_plain_folders`. The handles from `hold_plain_folders` have no `READ_CONTROL`, so the DACL cannot be read through them. I added an injectable `SddlOf` (`hold_admin_chain_with`, `handle_sddl` made public) so tests can simulate DACLs. `staging_win.rs` is unchanged.
- The text and the transport wrapper live in a new `src/bridge/helper_folder.rs` (one `pub mod` line in `bridge/mod.rs`). `elevated.rs` belongs to Task 196, and the code has to be cross-platform for the CLI and TUI. `NoticeTransport` passes the refusal on the first time only. The CLI prints it with `style::WARN`. The TUI stores it in a slot, wakes the loop with `Msg::Clock` and shows it as an error toast, so `app.rs` needed no change.
- `src/cli/mod.rs` ~:498 is `spawn_background_update`, which never runs `runas`, so I did not change it.
- Addendum: when I started, the upgrade call sites (TUI `native_upgrade`, CLI `plan_upgrade`) were already being changed by Task 196; it then committed them in 7b80c5e. I fixed only the TUI install path (`plan_native_install` -> `native_install_on`). The CLI install now goes through `winget_cmd::install_plan`. The "modes differ" check runs only for msi, as the coordinator asked.

Concerns:
- When the folder refusal is the reason, the native machine-scope job still fails with the generic `UAC_REFUSED`/`ElevatedError::Refused` message (in `ops.rs`/`elevated.rs`, Task 196's files). The fixed text does appear on its own, but next to that generic error.
- The `helper_notice.show` call in `event_loop` is not covered by any test (surviving mutation above). Covering it would mean changing `clock_turn` or the loop.
- Users who run wingetrs from `~\.cargo\bin` or a Scoop folder can no longer use the worker. This is the intended effect of the decision.
- No `.join` with external data was written.

## Review round 1 (commit 2a4ad77)

Commits: `2a4ad77 fix(bridge): task 197 review (grandparent DACL test, helper starts in System32 with DLL search hardened, loop_turn shows the folder refusal)`

1. **Grandparent DACL.** `worker_refused_from_user_writable_folder` gained a case: `dir.path()` gives USERS_WRITE and `bin` gives ADMIN. The test expects PermissionDenied, the refusal text and 0 launcher calls.
   `[OK] mutação PEGA: 'program.ancestors().skip(1).enumerate()' -> 'program.ancestors().skip(1).take(1).enumerate()' fez o teste falhar (exit 101).`
2. **DLL planting.**
   - (a) The launcher now receives a `HelperLaunch { exe, args, dir }`, and `launch` sets `lpDirectory`. `dir` is `SystemEnv::query()?.system32`, which comes from the KnownFolder. The test's fake launcher checks that `dir` equals System32.
     `[OK] mutação PEGA: '        dir: system32,' -> '        dir: exe.parent().unwrap_or(exe).to_path_buf(),' fez o teste falhar (exit 101).`
   - (b) and (c) `worker_main` is the first call in the `BridgeWorker` branch. It now calls `worker_main_with(pipe, parent, &harden_process)`. `harden_process` calls `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)` and then `set_current_dir(System32)`, with System32 again from the KnownFolder. I used `std::env::set_current_dir` (SetCurrentDirectoryW) so no second feature was needed.
   - Test: I chose an injectable hook. `worker_hardens_the_process_before_anything_else` checks that the hook runs before the pipe-name check, and that the helper stops when the hook fails.
     `[OK] mutação PEGA: '    harden()?;' -> '' fez o teste falhar (exit 101).`
   - Feature: I added `Win32_System_LibraryLoader` to the `windows` crate, because `SetDefaultDllDirectories` lives there. `git diff --quiet Cargo.lock` confirmed the lockfile did not change.
3. **Event-loop notice.** The loop now calls `tui::run::loop_turn` (which runs `clock_turn` and then `notice.show`). The test checks the toast right after `loop_turn`.
   `[OK] mutação PEGA: '    notice.show(app);' -> '' fez o teste falhar (exit 101).`

Gate: it stops at `[FAIL] fmt exit 1`. The only diff is in `src/install/script_scan.rs:1655`, which belongs to Task 164. The steps before it passed (`[OK] rules`, `[OK] rules-selftest`). Since the gate never got past fmt, I proved my part separately:
- `cargo clippy --all-targets`: nothing in any file.
- `elevated_worker::`: 22 passed.
- `wi_tui::`: 16 passed.
- `tui_`: 366 passed (the settings snapshots now pass after 7138199).
