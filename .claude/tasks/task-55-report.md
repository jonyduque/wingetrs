Status: DONE

Commits:
2e105cf perf(catalog): empty a map table at once when every row belongs to the replaced source
(825d893 in the range is Task 56's, not mine)

Gate:
[OK]   rules 3s
[OK]   rules-selftest 11s
[OK]   fmt 1s
[OK]   clippy 114s
[OK]   test (494 passed, 16 ignored) 62s
[OK]   gate verde

Mutation proofs (all on src/catalog/mod.rs):
1. 'if foreign {' -> 'if false {' (always truncate), `cargo test --quiet --lib truncating_owned`:
   [OK] mutação PEGA: 'if foreign {' -> 'if false {' fez o teste falhar (exit 101).
   Same mutation with `cargo test --quiet --test catalog` (tests 2 and 3): also PEGA (exit 101).
2. 'if foreign {' -> 'if true {' (never truncate):
   [OK] mutação PEGA: 'if foreign {' -> 'if true {' fez o teste falhar (exit 101).
3. EXISTS check replaced by "package of another source" JOIN:
   [OK] mutação PEGA: 'SELECT EXISTS(SELECT 1 FROM {t} WHERE package NOT IN (...))' -> 'SELECT EXISTS(SELECT 1 FROM {t} m JOIN packages p ON p.rowid = m.package WHERE p.source <> ?1)' fez o teste falhar (exit 101).

Decisions:
- clippy rejected `truncated_maps` as never read outside tests; added a `tracing::debug!(truncated_maps, ...)` in replace_source instead of an allow(dead_code).
- Test 2/3 share a small helper `pfn_map` in tests/catalog.rs (assertions unchanged).
- Unit-test module has `#[allow(clippy::unwrap_used, clippy::expect_used)]`.

Concerns: none. A/B benchmark not run (orchestrator does it). Scoop-bucket import cost (one EXISTS stopping at first foreign row): não medido.

## Review fix (Important)
Commit: fix(catalog): take the write lock before testing map ownership (see git log)
- replace_source/remove_source now use transaction_with_behavior(Immediate); truncated_maps counts only tables where the DELETE removed rows (>0).
- Test replace_waits_for_a_concurrent_writer_instead_of_failing (tests/catalog.rs): conn B holds BEGIN IMMEDIATE, worker runs replace_source, B (after a channel signal plus a 300 ms yield, not asserted) inserts and commits; worker must succeed. Ordering is not fully deterministic (the sleep lets the worker reach its first statement).
- Mutation back to .transaction(): run by hand (mutate.ps1 anchor with newline fails in pwsh arg passing): test FAILED (panic at tests/catalog.rs:482, SQLITE_BUSY), file restored.
- Mutation 'WHERE package NOT IN' -> 'WHERE package IN': [OK] mutação PEGA ... fez o teste falhar (exit 101).
- Gate: [OK]   test (495 passed, 16 ignored) / [OK]   gate verde
