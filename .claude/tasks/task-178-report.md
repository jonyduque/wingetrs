Status: DONE_WITH_CONCERNS

Commits:
a04f4cd feat(winget-install): unelevated ShellExecute launcher, elevation rule and is_elevated

Gate:
The gate failed only in other agents' files (run twice):
```
[OK]   rules 6s
[OK]   rules-selftest 15s
[OK]   fmt 2s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (lib) due to 1 previous error
  error: could not compile `wingetrs` (lib test) due to 1 previous error
```
The lib error was `src\winget_install\fetch.rs:120:17: error: unused variable: loopback` (task 177). The `it` error was `tests\it\wi_launch.rs:66:9 unwrap()`, which was mine: I had dropped the stub's `#![allow(clippy::unwrap_used, clippy::expect_used)]` and then restored it. Second run:
```
[OK]   rules 7s
[OK]   rules-selftest 15s
[FAIL] fmt exit 1
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\bridge\mod.rs:231:
```
How I proved my files instead:
- `rustfmt --edition 2024 --check` on my 3 files: no diff.
- `cargo clippy --all-targets -- -D warnings`: the only remaining error is `tests\it\wi_bridge_install.rs:410:37` (another agent's file). Nothing in `launch*`/`wi_launch`.
- `cargo test --quiet --test it wi_launch::` -> `test result: ok. 8 passed; 0 failed; ... 1245 filtered out`
- `cargo test --quiet --lib launch_win` -> `test result: ok. 2 passed; 0 failed; ... 61 filtered out`

Mutation proofs:
1. scope `user` -> `Helper`: `pwsh -File scripts/mutate.ps1 -Path src/winget_install/launch.rs -AnchorFile t178-m1a.txt -ReplacementFile t178-m1r.txt -TestCommand 'cargo test --quiet --test it wi_launch::install_elevation_truth_table'`
   `[OK] mutação PEGA: '    if is(scope, "user") {` (assertion `Msi Some("user") None`). The first attempt was INCONCLUSIVO because another agent's file did not compile; the earlier and the repeated runs both caught it.
2. `elevationRequired` checked after scope `user` (same command, files t178-m2a/m2r):
   `[OK] mutação PEGA: '    if is(requirement, "elevationRequired") {` (assertion `Exe Some("user") Some("elevationRequired")`)
3. Removing `SEE_MASK_NOCLOSEPROCESS`: `pwsh -File scripts/mutate.ps1 -Path src/winget_install/launch_win.rs -Anchor 'fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC' -Replacement 'fMask: SEE_MASK_NOASYNC' -TestCommand 'cargo test --quiet --test it wi_launch::win::cmd_exit_code_is_returned'`
   `[OK] mutação PEGA: 'fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC' -> 'fMask: SEE_MASK_NOASYNC' fez o teste falhar (exit 101).` It panics at wi_launch.rs:83:52, which is the `.unwrap()` on `run`: the mutant returns an error (null hProcess), not `Ok(0)`.
4. STA `join` with `.ok()`: not proven. As the brief says, `check_rules` blocks it.

Decisions:
- `in_sta` uses `std::thread::scope` with an explicit `join` + `resume_unwind`, so the spec can be borrowed without `'static`. `CoUninitialize` sits in a `Drop` guard, so it also runs when the closure panics.
- Probed: the missing file returns ERROR_FILE_NOT_FOUND, which `io_err` maps to `NotFound`. The `.cmd` file gets an hProcess, and the exit code of `exit /b N` comes back through GetExitCodeProcess (tests 3–6 green).
- Tests added beyond the brief: `dir_is_the_working_directory` first runs without `dir` and expects 6, which shows the 5 comes from `dir`. Extra rows in the table (appx, upper-case values, prohibited without scope, plain exe). Unit test `sta_worker_value_is_returned`.
- Exit code: the `u32` from GetExitCodeProcess is cast `as i32` (an NTSTATUS becomes negative; callers compare `as u32`).
- No new `windows` feature (SW_SHOWNORMAL/SW_HIDE come from `Win32_UI_WindowsAndMessaging`, already enabled).

Concerns:
- The full gate was not green because of other agents' files (fetch.rs, bridge/mod.rs fmt, wi_bridge_install.rs clippy, BridgeJob `op` in progress). The orchestrator needs to run the gate once those land.

## Review fix (097d8de fix(winget_install): review of task 178)

- New order: msix/appx -> prohibited -> scope `user` (Never) -> `elevationRequired` (Helper) -> `machine` (Helper) -> msi/wix/burn (Helper) -> Never. An unknown scope (`Some("")`, `Some("x")`) counts as absent. Table: `(Exe, user, elevationRequired) => Never`, plus rows `(Msi,"x")=>Helper`, `(Burn,"")=>Helper`, `(Inno,"x")=>Never`, `(Exe,"")=>Never`, `(Exe,None,ElevationRequired)=>Helper`, `(Inno,"x",elevationRequired)=>Helper`.
- `wi_launch::` -> `test result: ok. 8 passed; 0 failed; ... 1278 filtered out`
- Mutations (`-TestCommand 'cargo test --quiet --test it wi_launch::install_elevation_truth_table'`):
  - `elevationRequired` before `user` (t178-r1a/r1r): `[OK] mutação PEGA: '    if user {` (assertion `Exe Some("user") Some("elevationRequired")`)
  - unknown scope no longer absent, i.e. `scope.is_none() &&` put back in front of the msi/wix/burn rule (t178-r2a/r2r): `[OK] mutação PEGA: '    if matches!(` (assertion `Msi Some("x") None`)
  - `if user {` -> `if false {`: `[OK] mutação PEGA: '    if user {' -> '    if false {' fez o teste falhar (exit 101).`
- The literal mutation `scope.is_none() &&` -> `true &&` is equivalent under the requested rule. `user` and `machine` have already returned before that line, and unknown counts as absent, so `scope.is_none()` could only reject unknown scopes. That would break the rule. So the code has no such condition, and the proof is the reverse mutation (putting it back) above.
- Gate: `[OK] rules`, `[OK] rules-selftest`, `[OK] fmt`, `[FAIL] clippy exit 101`, failing only at `src\install\executor.rs:1463:9: unused variable: others` (another agent). Nothing in my files.
