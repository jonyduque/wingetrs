Status: DONE

Commits:
b08dc0e test(bridge): cover cancel retry and unflagged abort; settle the late-output test

Gate:
[OK]   clippy 13s
[OK]   test (860 passed, 22 ignored) 165s
[OK]   gate verde
(An earlier run failed in tests/install_archive.rs, files of another agent's task in the shared checkout; green on retry.)

Mutation proofs:
- stop() without repeating (`return;` after the sleep), `scripts/mutate.ps1 -Path src/bridge/spawn_win.rs -Anchor '            std::thread::sleep(CANCEL_RETRY);' ... -TestCommand 'cargo test --quiet --lib stop_retries_until_readers_finish'`:
  `[OK] mutação PEGA: '            std::thread::sleep(CANCEL_RETRY);' -> '            std::thread::sleep(CANCEL_RETRY);` (line cut by grep; multi-line replacement)
- 995 without flag treated as end:
  `[OK] mutação PEGA: 'if e.raw_os_error() == Some(OPERATION_ABORTED) && stop.load(Ordering::SeqCst) =>' -> 'if e.raw_os_error() == Some(OPERATION_ABORTED) =>' fez o teste falhar (exit 101).`

Late-output flakiness: 30 runs of `grace_lets_late_output_arrive` (release) while `CARGO_TARGET_DIR=target/t115-load cargo build --release` ran in parallel: pass=30/30. target/t115-load removed.

Decisions:
- Kept `ping -n 2` and the 3 s grace: 30/30 under load, no evidence for a change (the load build overlapped the start of the runs; heavier sustained load not measured).
- `stop()` delegates to `stop_with(cancel: fn(usize))`; real cancel is `cancel_io` (CancelIoEx). `read_loop` extracted as specified.
- Test 3 sends a release to the fake thread after `stop_with`, so a failing/mutated run does not hang in `Drop` (the first mutation attempt hung there before this fix).
- Test 1 covers flag-set-during-read (flag marked as the read fails) for the "Ok" half.

Concerns: none
