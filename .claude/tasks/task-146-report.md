Status: DONE_WITH_CONCERNS

Commits:
b0ef9c0 feat(bridge): run_spec with environment, working dir and byte decoder

Gate:
```
[OK]   rules 8s
[OK]   rules-selftest 20s
[OK]   fmt 3s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (lib) due to 3 previous errors
  error: could not compile `wingetrs` (lib test) due to 3 previous errors
  (log completo: target/verify.log)
```
The failure is only in another agent's file: `src\install\script_scan.rs:682` (E0762 unterminated character literal).
A later `cargo clippy --all-targets --message-format short` (lib compiling at that moment) failed only on
`tests\it\wi_fetch.rs:15:5: error[E0432]: unresolved import ...verified_win::VerifiedFile` (another agent); zero
lines mentioning `bridge` or `spawn_env`. My tests, run before the other agents broke the build:
```
cargo test --quiet --test it spawn_env::
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1172 filtered out; finished in 0.28s
```
Afterwards the checkout kept failing to compile in script_scan.rs (retried for ~6 min), so the orchestrator should rerun the gate.

Mutation proofs:
```
pwsh -File scripts/mutate.ps1 -Path src/bridge/spawn_win.rs -Anchor 'cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));' -Replacement 'let _ = spec.env;' -TestCommand 'cargo test --quiet --test it spawn_env::env_is_passed_and_removed'
[OK] mutação PEGA: 'cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));' -> 'let _ = spec.env;' fez o teste falhar (exit 101).
pwsh -File scripts/mutate.ps1 -Path src/bridge/mod.rs -Anchor '(self.decode)(seg)' -Replacement 'String::from_utf8_lossy(seg).into_owned()' -TestCommand 'cargo test --quiet --test it spawn_env::decoder_gets_raw_bytes'
[OK] mutação PEGA: '(self.decode)(seg)' -> 'String::from_utf8_lossy(seg).into_owned()' fez o teste falhar (exit 101).
pwsh -File scripts/mutate.ps1 -Path src/bridge/spawn_win.rs -Anchor 'if !spec.program.is_absolute() {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it spawn_env::relative_program_refused'
[OK] mutação PEGA: 'if !spec.program.is_absolute() {' -> 'if false {' fez o teste falhar (exit 101).
```

Decisions:
- `utf8_lossy` lives in `bridge/mod.rs` (so `Tail::default()` also builds outside Windows) and is re-exported with `pub use super::utf8_lossy;` in `spawn_win.rs`, so `spawn_win::utf8_lossy` works as the brief says.
- `Tail` dropped `#[derive(Default)]` (a fn pointer has no Default) and got a hand-written `Default` that calls `with_decoder(utf8_lossy)`.
- The decoder reaches `read_loop`/`spawn_reader`/`run_command` as a `fn` parameter; `run`/`run_with_grace`/`run_in` pass `utf8_lossy`, so they behave as before. The readers are still joined on every path (`Readers` and `join` untouched).
- Test helper `run` also checks that `on_line` got the same lines the tail kept.

Concerns:
- The brief's fact for test 1 is wrong: inside a `.cmd` batch file an undefined `%OS%` expands to nothing (only the interactive prompt leaves it literal). Measured: the output was `["A=1", "B="]`. The test asserts `"B="`; with `OS` inherited it would be `B=Windows_NT`, so the removal is still proved. A comment in the test says this.
- The stub `tests/it/spawn_env.rs` had no `#![cfg(windows)]`; I added it as the brief asks.
- The gate was not green because other agents' files did not compile (see Gate).
