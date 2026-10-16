Status: DONE_WITH_CONCERNS

Commits:
`git log --format='%h %s' b78e905..HEAD -- src/install/msi_queue.rs src/install/msiexec_win.rs src/install/layout.rs src/tui/jobs.rs src/tui/run.rs tests/tui_jobs.rs tests/install_msi_queue.rs`
```
2846a85 feat(install): Windows Installer queue (msi.lock, mutex wait, 1618 retry) shared with bridge winget jobs
```

Gate (main checkout, with tasks 123/124 also in the tree):
```
[OK]   rules 5s
[OK]   rules-selftest 14s
[OK]   fmt 3s
[OK]   clippy 39s
[OK]   test (908 passed, 23 ignored) 207s
[OK]   gate verde
```
Live test (`cargo test --quiet --test install_msi_queue -- --ignored --nocapture`, debug build, not release): `installer_busy: false false`, 1 passed.

Mutation proofs:
1. Busy not re-checked after the first sleep:
   `mutate.ps1 -Path src/install/msi_queue.rs -Anchor '        while busy() {' -Replacement '        if busy() {' -TestCommand 'cargo test --quiet --test install_msi_queue waits_while_installer_busy'`
   `[OK] mutação PEGA: '        while busy() {' -> '        if busy() {' fez o teste falhar (exit 101).`
2. 1618 treated as Failed:
   `mutate.ps1 -Path src/install/msi_queue.rs -Anchor 'Attempt::Busy1618 => {}' -Replacement 'Attempt::Busy1618 => return Err(MsiQueueError::Failed(1618)),' -TestCommand 'cargo test --quiet --test install_msi_queue retries_1618_with_backoff'`
   `[OK] mutação PEGA: 'Attempt::Busy1618 => {}' -> 'Attempt::Busy1618 => return Err(MsiQueueError::Failed(1618)),' fez o teste falhar (exit 101).` (panicked at tests\install_msi_queue.rs:133)
3. Bridge doesn't take msi.lock:
   `mutate.ps1 -Path src/tui/jobs.rs -Anchor '    let _msi = match job.tool {' -Replacement '    let _msi = match Tool::Scoop {' -TestCommand 'cargo test --quiet --test tui_jobs bridge_winget_takes_msi_lock'`
   `[OK] mutação PEGA: '    let _msi = match job.tool {' -> '    let _msi = match Tool::Scoop {' fez o teste falhar (exit 101).`
   Extra (lock released before winget runs, `drop(_msi);` inserted before `let before`): it first SURVIVED; I strengthened the test so the fake exec checks the lock is still held. After that: `[OK] mutação PEGA: '        Tool::Scoop => None, ...` (panicked at tests\tui_jobs.rs:1264).
4. `grep -n "TerminateProcess\|\.kill()" src/install/msiexec_win.rs` gives no output (exit 1).

Decisions:
- `run_bridge` takes a new `msi_lock: &Path` parameter (run.rs passes `data.install_layout().msi_lock()`). `src/tui/run.rs` is outside the brief's Files list, but it is the only production caller of `run_bridge`: without this one-argument change the crate doesn't compile, and binding item 4 never reaches the real TUI. I also updated its doc comment. While waiting, `on_line` gets `jobs::WAITING_INSTALLER` = "aguardando outro instalador", which becomes the job's progress text.
- `lock_msi(path, status)` (in msi_queue.rs) reuses `UpdateLock::try_acquire`/`acquire_timeout(Duration::MAX)` and reports `WaitingLock` once.
- `wait_max/poll` = 300 busy checks in total. `InstallerBusy` comes on the 300th busy answer, so there are 299 poll sleeps. Each 1618 retry gets a fresh mutex-wait budget.
- Added pure `attempt_from_code` (0/3010 ⇒ Done, 1618 ⇒ Busy1618, other ⇒ Failed) so the mapping is testable, plus `Layout::msiexec_log(app, n)` = `<raiz>\log\msiexec-<app>-<n>.log`.
- `installer_busy`: `ERROR_ACCESS_DENIED` ⇒ true (the mutex exists). Any other error except FILE_NOT_FOUND ⇒ false plus a tracing warning, and a real conflict still shows up as 1618.

Concerns:
- TDD order: the implementation was written right after the tests, so I never saw the tests fail before it existed. The mutation proofs above stand in for that.
- I briefly built in a separate worktree (`target/wt127`) because Task 123's in-progress files broke the main build. That helped fill the disk (os error 112). The worktree and its target dirs have been removed (`git worktree remove --force`).
- I ran `cargo fmt -- tests/tui_jobs.rs` in the shared tree. That command may also format the rest of the crate, including other agents' uncommitted files. My commit only contains my files.
- The bridge tests leave small `tui_jobs-msi-<pid>-<n>.lock` files in `CARGO_TARGET_TMPDIR` (target/tmp).

## Review fixes (review: APPROVE with Minors)

1. `wait_max` is now one total budget across the 1618 retries, not a fresh one per attempt. `WaitingInstaller` is still reported once per wait. New test `installer_wait_budget_is_total_across_retries`: 200 busy answers, then a 1618, then busy again ⇒ `InstallerBusy` after 300 busy answers in total.
   Mutation (budget restarted per attempt): `mutate.ps1 -Path src/install/msi_queue.rs -Anchor '        let mut reported = false;' -Replacement '        let mut reported = false;<NL>        let mut checks = 0u128;' -TestCommand 'cargo test --quiet --test install_msi_queue installer_wait_budget_is_total_across_retries'`
   `[OK] mutação PEGA: '        let mut reported = false;' -> '        let mut reported = false;` (panicked at tests\install_msi_queue.rs:136)
2. New pure function `msi_queue::admin_args` builds the command line and strips trailing `\` and `/` from TARGETDIR. Test `admin_args_quote_paths_and_strip_trailing_separators` covers `\`, `\`, `/` and no trailing separator. Known edge: a drive root `C:\` would become `C:`. It can't happen today because the target is always a staging folder.
3. `msiexec_admin` runs `create_dir_all` on the log file's folder before launching msiexec.
4. After waiting for the lock, the bridge sends `STARTING_WINGET` = "iniciando o winget". A job that didn't wait sends nothing extra, so the existing bridge tests are unchanged. The order check in `bridge_winget_takes_msi_lock` now expects `released, line iniciando o winget, exec winget held=true`.
5. `tests/tui_jobs.rs`: `msi_lock()` returns a `MsiLock` holding a `TempDir`, which derefs to `&Path`. No lock files are left in `CARGO_TARGET_TMPDIR`.

Gate after the fixes: `scripts/verify.ps1` stopped at `[FAIL] fmt exit 1`, because of `Diff in ...\src\install\extract_7z.rs:47` (Task 123, uncommitted). So I ran the remaining stages by hand:
- `rustfmt --edition 2024 --check` on my 7 files: exit 0.
- `cargo clippy --quiet --all-targets --message-format short -- -D warnings`: exit 0.
- `cargo test --quiet --no-fail-fast`: 916 passed, 1 failed, 23 ignored. The 1 failure is `encoded_header_with_huge_unpack_size_refused` in `tests/install_7z.rs:520` (Task 123, uncommitted).

So the full gate is blocked by Task 123's in-progress files, and the orchestrator should rerun it.
