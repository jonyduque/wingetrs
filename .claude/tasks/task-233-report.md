Status: DONE

Commits:
561a9a6 fix(bridge): spawn_win::run never searches PATH when the process is elevated
(560eeeb in the same file list belongs to Task 205)

Gate:
[OK]   fmt 9s
[OK]   licenses 47s
[OK]   clippy 70s
[OK]   test (2096 passed, 90 ignored) 253s
[OK]   gate verde

Mutation proofs:
- `if !elevated || Path` -> `if true || Path` (equivalent to the brief's `if false` on the elevated check): [OK] mutação PEGA: ... fez o teste falhar (exit 101).
- `return winget_exe();` -> `return Ok(PathBuf::from(program));`: tests failed (3 passed; 2 failed: elevated_winget_comes_from_the_admin_folder, elevated_winget_missing_is_an_error_not_the_path). The `[OK] mutação PEGA` line was cut by `tail` in that run; not re-run.
- The tests were not seen failing before the implementation (written together with it).

Decisions:
- resolve_program lives in src/bridge/mod.rs (platform neutral); tests are in tests/it/bridge.rs (mod resolve_program), since tests/it/main.rs has no bridge_spawn line.
- Wired only in spawn_win::run_with_grace (run calls it). Other run_* take absolute paths already.
- rustfmt reordered the `use wingetrs::bridge::{...}` import in tests/it/bridge.rs (the file was not fmt-clean before; the gate demanded it).

Concerns: none. Callers checked: cli/winget_cmd.rs:147 and tui/run.rs:1718 (both via run), tui/run.rs:1704 detect_tools; uninstall.rs goes through the bridge closure.
