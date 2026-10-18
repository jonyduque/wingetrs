Status: DONE_WITH_CONCERNS

Commits:
a3d3bf5 style(tests): rustfmt of the task 193 download test
4554624 fix(install): Ctrl+C stops a winget download mid-body (task 193 review)
1bc1c90 fix(cli,tui): review of task 193 (one removal sequence, cancellable lock wait, removal phases)
2c607b1 fix(cli,tui): one plain-removal plan for the CLI and the TUI, wording in the shared texts
2af05f4 fix(cli): winget native flows match the TUI (Ctrl+C, previous plan, Kindle, progress)

Gate (`pwsh -File scripts/verify.ps1`, last run, before the commit):
```
[OK]   rules 9s
[OK]   rules-selftest 27s
[FAIL] fmt exit 1
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\bridge\mod.rs:179:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\bridge\mod.rs:272:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\install\script_scan.rs:1228:
  (log completo: target/verify.log)
```
The fmt failure is only in other tasks' files: `src/bridge/mod.rs` (Task 190) and `src/install/script_scan.rs` (Task 164). The run before that one failed fmt only in `src/install/executor.rs` (Task 191). My own files were checked as follows:
- `rustfmt --edition 2024 --check` on my files: clean.
- `cargo clippy --all-targets`, filtered to my paths: no findings.
- Test modules, output pasted:
```
== wi_cli              test result: ok. 24 passed; 0 failed
== cli_scripts         test result: ok. 20 passed; 0 failed
== wi_tui              test result: ok. 13 passed; 0 failed
== tui_jobs            test result: ok. 40 passed; 0 failed
== wi_uninstall_flow   test result: ok. 37 passed; 0 failed
```
The orchestrator needs to run the full gate.

Follow-up (2c607b1): no full gate rerun; the gate stays red from other tasks' files. My files: `cargo clippy --all-targets`, filtered to my paths, has no findings; rustfmt is clean.
```
cargo test --quiet --test it tui_   -> test result: ok. 364 passed; 0 failed; 4 ignored
cargo test --quiet --test it wi_tui:: -> test result: ok. 14 passed; 0 failed
cargo test --quiet --test it wi_    -> 271 passed; 1 failed: wi_upgrade::native_agrees_with_winget_applicability
```
That one failure panics in `manifest::parse` (wi_upgrade.rs:359). `tests/it/wi_upgrade.rs` and `src/winget_install/engine.rs` have uncommitted edits from another agent, and I touch neither file.

Mutation proofs:
- Follow-up 1: the TUI's plain removal goes back to `nsis = false` (the shared function), test `wi_tui::tui_and_cli_plan_the_same_nullsoft_removal`:
  `[OK] mutação PEGA: 'nsis: manifest.is_some_and(manifest_nsis),' -> 'nsis: false,' fez o teste falhar (exit 101).`
- Follow-up 2: the winget-uninstaller line, now in `uninstall.rs` (`Display`), test `wi_cli::winget_wording_says_things_once`:
  `[OK] mutação PEGA: 'Some(c) if is_winget(&c.program) =>' -> 'Some(c) if false =>' fez o teste falhar (exit 101).`
- (a) The CLI goes back to `nsis = false`. The NSIS decision now exists only in `winget_install::previous::previous_plan`, which both the CLI and the TUI call, so the mutation is applied there:
  `scripts/mutate.ps1 -Path src/winget_install/previous.rs -Anchor 'nsis: plan.installer.kind == InstallerKind::Nullsoft,' -Replacement 'nsis: false,' -TestCommand 'cargo test --quiet --test it wi_cli::upgrade_previous_plan_same_in_cli_and_tui'`
  `[OK] mutação PEGA: 'nsis: plan.installer.kind == InstallerKind::Nullsoft,' -> 'nsis: false,' fez o teste falhar (exit 101).`
- (b) `--yes` with a non-empty `shared` goes ahead (the `if f.yes { bail!(SHARED_YES_REFUSED) }` block is removed, using `-AnchorFile`/`-ReplacementFile` with an empty replacement), test `wi_cli::cli_kindle_yes_refused`:
  `[OK] mutação PEGA: '    if f.yes {` (the multi-line anchor; the script printed only the first line)
- Extra: Ctrl+C before the launch does not cancel. `'if gate.try_cancel() {' -> 'if false {'`, test `cli_ctrl_c_while_waiting_cancels`:
  `[OK] mutação PEGA: 'if gate.try_cancel() {' -> 'if false {' fez o teste falhar (exit 101).`
- Extra: no notice after the launch. `'ui.notice(LAUNCHED_NOTICE);' -> ''`, test `cli_ctrl_c_after_launch_prints_notice_once`:
  `[OK] mutação PEGA: 'ui.notice(LAUNCHED_NOTICE);' -> '' fez o teste falhar (exit 101).`
- Extra: removal without `install.lock`. `'let _held = lock().await?;' -> 'let _held = ();'`, test `cli_uninstall_shows_phases`:
  `[OK] mutação PEGA: 'let _held = lock().await?;' -> 'let _held = ();' fez o teste falhar (exit 101).`
- Extra (addendum b): the dry run without SHA-256, test `winget_dry_run_shows_url_and_sha256`:
  `[OK] mutação PEGA: 'text.push_str(&format!("\nSHA-256: {}", plan.installer.sha256));' -> '' fez o teste falhar (exit 101).`
- Extra (addendum from Task 194): the CLI uninstall skips the msi queue. `'uninstall_queued(&watched, Some(queue), plan)' -> 'uninstall_queued(&watched, None, plan)'`, test `cli_uninstall_of_msi_waits_for_msi_lock`:
  `[OK] mutação PEGA: 'uninstall_queued(&watched, Some(queue), plan)' -> 'uninstall_queued(&watched, None, plan)' fez o teste falhar (exit 101).`

Decisions:
- `WingetFlows`: `install`, `upgrade` and `uninstall` now return `Op<T>`, a `Send` closure that builds its future on the `spawn_blocking` thread (`Handle::block_on`). This is needed because a borrowed `&OpsCtx`/`WingetCtx` is neither `'static` nor `Send`. To support this, `ProdFlows` takes owned clones of its context, `HelperShutdown` holds an `Arc<ElevatedBridge>`, and new methods were added: `gate`, `progress`, `lock_install`, `remove_registry_only`, `free_space`. `plan_uninstall` became async so it can read the manifest for the NSIS hint.
- Ctrl+C texts: the TUI has no such text, so I used the brief's literal for both. `LAUNCHED_NOTICE` = "o instalador já começou e não pode ser interrompido; aguarde"; before the launch it is `CANCELLING_NOTICE`. While waiting on an await (download, `install.lock`), a cancelled gate drops the op (`or_cancelled`, polls every 50 ms).
- Phases are emitted from `winget_cmd`: `Removendo X` or `WaitingElevation` after `install.lock` and `try_launch`. "Conferindo a remoção" is `Phase::Removing { app: "X (conferindo a remoção)" }`; I did not add a new `Phase` variant because `progress.rs`, `output.rs` and `jobs.rs` belong to other tasks.
- Kindle: a choice is needed when `shared` has folders or allows registry-only. Options: copy and remove / remove without copying (the TUI allows this too, by unchecking the copy) / remove only the registry / cancel. The functions (`backup_folders`, `remove_registry_only`) were already in `winget_install::uninstall`, so nothing was moved out of `tui/jobs.rs`.
- The TUI `previous_plan` was removed from `tui/run.rs` and replaced by `winget_install::previous_win`. The CLI previous plan now uses `others = []` and the plan's name and version for MSIX, the same as the TUI.

- Follow-up: `RemovalRequest` is in `winget_install::previous`. The CLI builds it with `RemovalRequest::of_row` and the TUI carries it in `Cmd::PlanWingetUninstall(RemovalRequest)`, built in `app.rs` (it now also carries the record's version and the matched package). Both sides call `previous_win::plain_removal_plan`, which reads the other records, takes the NSIS hint from the manifest and measures the folders. The TUI fetches the manifest on the job thread (`Handle::block_on(winget_manifest)`).
- Follow-up: `name_version` and `bridge_reason` live in `plan.rs`; the winget-uninstaller line and the "version once" rule live in `UninstallPlan`'s `Display`. The TUI's "Pelo winget: …" now uses `bridge_reason`. The CLI copies (`uninstall_text`, `name_version`, `bridge_why`) were removed. `native_text` (the dry-run SHA-256) stays in the CLI because the TUI has no dry run.

Concerns:
- Files outside the Files list: `src/cli/install_cmd.rs` (3 lines: `uninstall` became async, plus `.await`), `src/cli/mod.rs` (the help text), `tests/it/cli_scripts.rs` (signatures of the `NoWinget` stub) and `src/tui/jobs.rs`. In `jobs.rs` the only changes are the `uninstall_queued`/`MsiQueue` call and its import, as asked in the addendum. All four files also contain Task 191's uncommitted edits, so I committed only my hunks (`git apply --cached` with the hunks selected).
- Untested paths: the msi queue path in the TUI (`jobs.rs:winget_uninstall`) and the manifest's NSIS hint in `ProdFlows::plan_uninstall` (Windows production) have no test of their own. The neutral parts are tested: `manifest_nsis`, `removal_plan` and `run_uninstaller`.
- Fixed in 2c607b1: the TUI now uses the same plain-removal plan as the CLI, so both get the manifest's NSIS hint. Follow-up commit touched `src/tui/app.rs` (the `Cmd` variant) and `tests/it/wi_tui.rs`: the 4 expected `Cmd::PlanWingetUninstall` literals now carry the `RemovalRequest` (adding `version` and `package`), plus the new equivalence test.
- Once the gate is launched (just before the uninstaller), a wait on `msi.lock` inside `uninstall_queued` cannot be cancelled: Ctrl+C there gets the "não pode ser interrompido" notice.
- Addendum (b): SHA-256 appears only in the native dry run. The bridge install dry run still shows only the winget command, because winget picks the installer itself. The native plan's `Instalador: <url>` line still appears outside the dry run too (it is shared with the TUI confirmation; I left it).
- Addendum (c): moved into `plan.rs`/`uninstall.rs` in 2c607b1, so the CLI and the TUI now show the same text.
- `.join` with external data in this task: only `backups().join(format!("{stamp}-pastas"))`, where the stamp is a local Unix timestamp, not external data.

## Review round (1bc1c90)

I did not rerun the full gate. My files were checked as follows:
- `cargo clippy --lib --tests`: no findings in my paths. `--all-targets` stops at `examples/elevated_arp_probe.rs`, which no longer matches the trait in `bridge/worker.rs` (Task 190).
- rustfmt `--check` on my files: clean.
- Test modules, output pasted:
```
== wi_               test result: ok. 275 passed; 0 failed; 4 ignored
== tui_              test result: ok. 364 passed; 0 failed; 4 ignored
== cli_              test result: ok. 103 passed; 0 failed; 1 ignored
== install_progress  test result: ok. 13 passed; 0 failed
```
The `cli_` run includes `cli_parse::help_snapshots`. It had been failing since 2af05f4 because of the new `install --help` wording; the two `.snap` files are now updated.

Mutation proofs (review):
- Item 1, removal gate (`src/winget_install/removal.rs`, the old winget_cmd.rs:778), test `wi_cli::cli_uninstall_ctrl_c_while_waiting_cancels`:
  `[OK] mutação PEGA: 'if !s.gate.try_launch_for("desinstalador") {' -> 'if false {' fez o teste falhar (exit 101).`
- Item 2, the `install.lock` wait in short slices, test `wi_cli::lock_install_for_gives_up_on_cancel`:
  `[OK] mutação PEGA: 'let slice = total.saturating_sub(start.elapsed()).min(LOCK_SLICE);' -> 'let slice = total.saturating_sub(start.elapsed());' fez o teste falhar (exit 101).`
- Item 3, the phase shown before the lock, test `wi_cli::cli_uninstall_shows_phases`:
  `[OK] mutação PEGA: 'phase(Phase::Preparing);' -> 'phase(Phase::Verifying);' fez o teste falhar (exit 101).`
- Item 6, the notice names the step, test `wi_cli::cli_uninstall_ctrl_c_after_launch_prints_notice_once`:
  `[OK] mutação PEGA: 'ui.notice(&uncancellable(gate.running().as_deref()));' -> 'ui.notice(&uncancellable(None));' fez o teste falhar (exit 101).`

What changed:
1. Two new uninstall tests, Ctrl+C before and after the launch. Each uses a blocking fake `lock_install`/uninstaller and a 30 s deadline.
2. `or_cancelled` is gone. The CLI now never drops an operation while it waits.
   - `ops::lock_install_for(layout, wait, gate)` waits for `install.lock` in 200 ms slices. Each slice is a `spawn_blocking` that is awaited. Between slices it reads `gate.cancelled()`. If the lock is taken just as the cancel arrives, it is let go at once.
   - `ops::install`, `engine::upgrade` (uninstallPrevious) and the CLI removal use it. The TUI keeps `lock_install` with `NoWait`.
   - Since nothing is dropped any more, a download (`fetch_all` with its `.part` guard and its hashing/rename `spawn_blocking`) always runs to its end. A Ctrl+C during the download is then refused by the gate before the launch.
   - Test `lock_install_for_gives_up_on_cancel`: with the lock held, a Ctrl+C makes it return `Cancelled` in under 5 s. After the lock is released, nobody takes it.
3. A phase is shown from the start: `Preparando` before `lock().await`, then `Copiando as pastas de dados para backups`. Copy progress is not shown: `backup_folders` has no callback, and adding one would touch the copy loop.
4. New phases `Uninstalling { name }` ("Desinstalador de X rodando (pode abrir janelas) · não pode ser cancelado") and `CheckingRemoval { name }` ("Conferindo a remoção de X (até 2 min) · não pode ser cancelado").
   - The CLI line adds "rodando há N min; pode haver uma janela esperando resposta" after `NATIVE_QUIET` (5 min). This applies to the installer, the UAC wait and the uninstaller, not to the 2-min check.
   - The TUI marks these as uncancellable (`installer_running`).
5. `winget_install::removal::{remove, run_uninstaller, removal_phase, uninstaller_phase, RemovalChoice}` is now the single sequence used by both the CLI and the TUI. In the TUI, `jobs.rs::winget_removal` replaced `winget_uninstall` and the registry-only arm, and the local `removal_phase` was removed. The `jobs.rs` hunk is necessarily larger than one line.
6. The notice is now `install_cmd::uncancellable(gate.running())`: "um passo já começou e não pode ser interrompido (desinstalador|instalador|remoção do registro); esperando terminar".
7. `run_win` passes `wctx` errors through unchanged.
- Another test changed: `wi_upgrade::upgrade_previous_cancel_before_removal_keeps_old` now expects no events instead of `["fetch"]`. Its comment already said "the download may finish"; with the cancellable lock, nothing is downloaded at all.
- `cli_scripts.rs`: only the `NoWinget` stub and its import (committed with `git apply --cached`; that file's other hunk belongs to another task).

## Download cancel (4554624, a3d3bf5)

Before this commit, a CLI download ran to its end after a Ctrl+C and the cancel took effect only at the launch gate. That is now fixed:
- `Http::get_to_file_until` reads a stop signal between chunks, and every 50 ms while waiting for the next chunk or for the response. On stop it flushes and closes the file and returns `Ok(false)`.
- `download::fetch_all_until` then returns `DownloadError::Cancelled`, and the existing `TmpGuard` deletes the `.part` file.
- `fetch::fetch_installer_until` and `ops::fetch_verified` pass `ctx.gate.cancelled()` as the stop signal and map the result to `WingetOpsError::Cancelled`.
- No future is dropped mid-await: every `spawn_blocking` is still awaited.
- The old signatures (`get_to_file`, `fetch_all`, `fetch_each`, `fetch_installer`) delegate with a stop signal that never fires, so the Scoop path behaves as before.

Test `wi_fetch::download_stops_mid_body_and_deletes_part` uses a raw TCP server that sends a 500 MB `Content-Length` and 64 KiB of body, then stalls. The test sets the stop signal once bytes have arrived and requires `Cancelled` within 10 s, with no `.part` file left. The server task is awaited.

`[OK] mutação PEGA: '() = stopped() => None,' -> '() = std::future::pending::<()>() => None,' fez o teste falhar (exit 101).`

Tests:
```
wi_ 277 ok; install_download 14 ok; http 23 ok; cli_ 103 ok; install_ops 47 ok
```
clippy `--lib --tests`: no findings in these files.

Another agent has uncommitted edits in my files from the earlier commits (`removal.rs`, `run_win.rs`, `tui/run.rs`, `wi_cli.rs`: `helper_line`). I did not touch or commit them.
