Status: DONE

Commits:
3122b54 perf(catalog): reuse prepared statements for per-app lookups
(base 6bde38f; no spans commit, the `step{step="correlate"}` span already existed)

Gate:
[OK]   rules 2s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 100s
[OK]   test (628 passed, 16 ignored) 219s
[OK]   gate verde

Mutation proofs:
1. `pwsh -File scripts/mutate.ps1 -Path src/catalog/mod.rs -Anchor 'table.table()' -Replacement 'MapTable::ProductCode.table()' -TestCommand 'cargo test --quiet --test catalog cached_statements'`
   [OK] mutação PEGA: 'table.table()' -> 'MapTable::ProductCode.table()' fez o teste falhar (exit 101).
2. Optimization itself: no applicable mutation; the performance proof is the A/B below.

A/B (list, Span step\{step=.correlate.\}, 15 runs, base = 6bde38f, L = max(3, 10% of 39.8) = 3.98 ms):
wall base med=   298.5 max=   347.8 | nova med=   292.9 max=   347.0 | delta pareado (nova-base) med=    -1.8 ms
piso noop med=30.0 ms | sobre o piso: base 268.5, nova 262.9 ms
span base med=    39.8 max=    69.5 | nova med=    29.5 max=    48.3 | delta pareado (nova-base) med=   -10.4 ms
brutos wall base: 331.9 347.8 295.1 269.5 298.5 299.4 296.3 327.2 260.7 293.7 299.0 257.4 297.4 314.7 304.3
brutos wall nova: 347.0 291.4 294.5 298.4 286.4 291.0 293.4 315.2 292.9 291.9 244.6 286.2 237.4 325.6 316.3
brutos span base: 42.4 41.5 39.5 42.1 39.1 42.5 39.4 69.5 39.8 37.2 36.4 40.1 41.5 38.9 38.9
brutos span nova: 48.3 31.1 29.5 28.6 27.4 33.9 31.7 45.4 27.3 26.4 27.9 30.0 28.3 26.8 42.4

Decision: KEEP. (1) span delta -10.4 <= -3.98; (2) max nova 48.3 <= max base 69.5; (3) wall paired delta -1.8 <= +5 ms; tests/catalog.rs passes (full gate green, includes correlate and install_ops).

Decisions:
- Tests added to tests/catalog.rs via shared helpers (cached_fixture/replace_cached/run_cached_queries); both pass before and after the change (regression guards, not red-first).
- Gate and mutation ran with a separate CARGO_TARGET_DIR; release benchmarks used target/release.

Concerns: none. leftovers.ps1 reported a stray conhost.exe (PID 22512) mid-task; not started by me, not touched; final leftovers check clean. Cleanup done (la, tgt-base-71, perf71-*.exe, worktree removed; registry key absent). Untracked .claude/tasks/task-71-base.txt left for the orchestrator.
