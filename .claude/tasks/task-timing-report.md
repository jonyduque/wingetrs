# Report: drop wall-clock thresholds (base 3a398a8)

## Changes
- tests/index_file.rs search_retries_while_the_file_is_briefly_locked: holder released after 5 ms (was 20 ms) with a Barrier so reader and holder start together; the reader's retry budget is ~50 ms, so a late holder thread is the only residual risk. No timing assert.
- tests/cli_offline.rs list_without_catalog...fast: bound 5 s -> 60 s (hang detector only; connection refused returns at once).
- tests/cli_offline.rs background_update_gives_up_at_once...: lock timeout knob 600000 ms; child is polled while the test still holds the lock; assertion is "child exited while lock held" (60 s hang detector, then kill). No `< 3s`.
- tests/cli_offline.rs foreground_update_still_waits_for_the_lock: child must still be running after 1.5 s with lock held (lower bound only), then lock released; child must not report "outra atualização em andamento". (Source failures exit 0, so the old `!success` was dropped.)

## Mutations (scripts/mutate.ps1)
- [OK] mutação PEGA: background uses LockWait::from_env() (src/cli/mod.rs)
- [OK] mutação PEGA: foreground uses LockWait::NoWait (src/cli/mod.rs)
- [OK] mutação PEGA: read_retrying `attempt < 5` -> `attempt < 0` (src/catalog/index_file.rs)

## Load check
5x cli_offline + index_file all pass. One cli_offline run took 47 s (others ~3.7 s); cause not measured (machine load while another agent built), still passed.

## Other timing asserts (not changed)
- tests/cache.rs:110-115 acquire_timeout 300 ms: `el >= 300ms && el < 2s` (upper bound load-fragile, 6.7x margin)
- tests/install_ops.rs:414-417 `elapsed < 2s`
- tests/install_ops.rs:1081 arrivals spread `< 400ms` (load-fragile)
- tests/update.rs:176-184 `elapsed < 100ms` (most fragile)
- tests/correlate.rs:253-274 `elapsed < 500 ms`
- tests/http.rs:91-102, install_download.rs:181-297 use server delays vs client timeouts (design-timing, margins 3-5x)
- tests/install_win_scan.rs:188-191 only records times (no assert seen)
