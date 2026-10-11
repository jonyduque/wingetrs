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

## Second pass (remaining timing asserts)
- tests/update.rs no_wait_returns_busy_immediately: `<100ms` -> call wrapped in tokio timeout(60 s) with the lock held; mutation NoWait=>600 s in src/update.rs: [OK] mutação PEGA.
- tests/install_ops.rs busy_lock_with_nowait_is_busy: `<2s` -> same timeout(60 s) wrapper; mutation NoWait=>600 s in src/install/ops.rs: [OK] mutação PEGA.
- tests/cache.rs acquire_timeout_waits_then_gives_up: dropped upper bound, kept `>=300ms` (cannot flake); mutation `left = ZERO` in acquire_timeout: [OK] mutação PEGA.
- tests/install_ops.rs apply_downloads_the_batch_in_parallel: server delay 400 -> 2000 ms, bound = the delay (gap expected a few ms, so >=10x margin; sequential gap >= delay by construction). Not a pure ordering proof (wiremock delay is not observable per-completion). Mutation Semaphore::new(1) in download.rs: [OK] mutação PEGA. Cost: test now ~2 s.
- tests/correlate.rs live_list_runs_under_budget: unchanged. It is #[ignore = live] (outside the gate) and its rule IS a perf budget (500 ms, release only); left as is.
5x loops of update/cache/install_ops all green; gate green (477 passed).
