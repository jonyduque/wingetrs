Status: DONE_WITH_CONCERNS (commit 1 and commit 3 measured and reverted by the brief's rules; commit 2 kept)

Commits (`git log --format='%h %s' b67b96d..HEAD`):
```
094d378 Revert "perf(catalog): rebuild map indexes once per winget import"
ed913c9 Revert "perf(catalog): lower(id) index rebuilt with the map indexes"
89d7ba0 perf(catalog): lower(id) index rebuilt with the map indexes
0ae6dfd perf(catalog): larger page cache during imports
6e164d2 perf(catalog): rebuild map indexes once per winget import
```
Net diff vs base: `src/catalog/mod.rs` +42 (only the cache_size wrapper in `replace_source` plus test `import_restores_cache_size`). The revert of 6e164d2 conflicted with 0ae6dfd; I resolved it by hand (cache wrapper in `replace_source`, body moved to `import_source`). The 3 extra regression tests of commit 1 (`tests/catalog.rs`) went away with the revert.

Gate (final HEAD 094d378, own CARGO_TARGET_DIR because the user's `cargo run` TUI, pid 4144, locks target/debug/wingetrs.exe):
```
[OK]   clippy 22s
[OK]   test (639 passed, 16 ignored) 134s
[OK]   gate verde
```

Mutation proofs (all run on the commits that carried the code; real output lines):
- C1 check 1 (drop `execute_batch(MAP_INDEXES)` before commit) -> `[OK] mutação PEGA: '            tx.execute_batch(MAP_INDEXES)?;` (test replace_with_maps_keeps_the_schema_of_a_fresh_catalog)
- C1 check 2 (`rebuilds_map_indexes` always true) -> `[OK] mutação PEGA: '    !(maps.productcodes.is_empty()`
- C1 check 3 (rebuild without ix_np, via `.replace(...)` on the rebuild call only; removing it from the DROP string survives legitimately because of `IF NOT EXISTS`) -> `[OK] mutação PEGA: '            tx.execute_batch(MAP_INDEXES)?;`
- C1 check 4 (DROP moved before `transaction_with_behavior`) -> `[OK] mutação PEGA: '        // Dropping `tx` on an early return rolls the whole import back.`
- C2 check 5 (no cache_size restore) -> `[OK] mutação PEGA: '        let restored = self.conn.pragma_update(None, "cache_size", prev);' -> '        let restored: Result<(), rusqlite::Error> = Ok(());' fez o teste falhar (exit 101).` (still in HEAD)
- C3 check 6a (index out of the rebuilt set, rebuild call only) -> `[OK] mutação PEGA: '            tx.execute_batch(REBUILT_INDEXES)?;`
- C3 check 6b (find_by_id back to non-`lower(id)` form) -> `[OK] mutação PEGA: 'WHERE lower(id) = lower(?1) ORDER BY rowid' -> 'WHERE id = ?1 COLLATE NOCASE ORDER BY rowid' fez o teste falhar (exit 101).` (needed a refactor: `find_by_id_sql()` is now what the EXPLAIN test inspects; the first version only tested a const and survived.)
Checks 1-4 and 6 live on reverted commits; they are no longer in HEAD.

## Measurements

Tool: `<S>\ab-update77.ps1` as in the brief (S = `C:\Users\jonyd\AppData\Local\Temp\claude\C--Users-jonyd-Projetos-wingetrs\2e07e6fc-9d74-4bff-b5e7-d3b32963d7e7\scratchpad`), roots `<S>\ab77-root-base`, `<S>\ab77-root-new` (copies of the real catalog; the real one is only read), msix `<S>\ab-msix\source2.msix` (from 2026-10-12 19:49, fixed), `catdigest.py` after each series. Exes: base = b67b96d (worktree build), c1 = 6e164d2, c2 = 0ae6dfd, c3 = 89d7ba0, final = 094d378. Command form: `pwsh -NoProfile -File "<S>\ab-update77.ps1" -Base "<S>\ab77-<x>.exe" -New "<S>\ab77-<y>.exe" -Runs N [-ScoopLocal]`. Raw outputs: `<S>\r1-a.txt ... r11-scoop.txt`, summary script `<S>\an.py`. Cleanup done (roots, tgt dirs, exes removed; no worktree left).

Caveat on the environment: a user TUI (`cargo run`, wingetrs.exe pid 4144, plus wezterm rendering it) was running the whole time; the brief said nothing would be running and I cannot stop it. Until ~00:20 the machine was bimodal: `catalog_write` ~0.7 s or ~3 s in bursts that hit base and new alike, so the early series are noise. Series r7 onwards (00:20+) were quiet (max < 1.1 s). I therefore trust only: r9/r10/r11 (and r6/r7 partially).

Digests: in every series the content digest, schema digest and `integrity_check` were identical between the two roots, and `search.idx igual: True` (c1 vs base, c1 vs c2, base vs c2, base vs cache-only, base vs final), except c2 vs c3 where only the schema digest differs (expected: new index; I did not diff the `sqlite_master` rows line by line, unit test `rebuilt_import_equals...` covered it).

Commit 1 (rebuild indexes) vs base, `update.catalog_write{source=winget}`, paired median delta:
- r1-a (n=10, loaded): -43/-56 ms; r1-b (n=10): +64 ms; r4 (n=15, bimodal): -21 ms; r9 (n=25, quiet): **-8 ms** (max base 857, max new 815). Fails the -100 ms threshold -> reverted (094d378). Scoop (item 4) irrelevant after the revert; r8 showed `scoop_write` +0.1 ms.
- Possible reason (not measured further): on this machine the whole write is ~0.55-0.7 s, not 1.06-1.2 s of the investigation, and the base already truncates maps (Task 55).

Commit 2 (cache_size 64 MiB):
- vs c1: r5 (n=25, noisy) -220 ms; r9 (n=25, quiet) **-21 ms** (fails -40 when judged against c1).
- Cache alone vs base (c2 with `rebuild` forced false, built ad hoc, "co"): r6 (n=25) -60 ms, r9 (n=25, quiet) **-50 ms** (max base 951, max new 735). Since c1 was reverted, the last-kept reference is base: -50/-60 passes the -40 ms rule -> kept.
- Final HEAD vs base, r10 (n=25, quiet): catalog_write base med 559 / max 1030, new med 509 / max 706, **paired med -67 ms**; wall -69 ms; search_idx -0.6; parse +5. Digests equal, integrity ok, search.idx equal. Scoop series r11 (-ScoopLocal, n=15): `scoop_write` base 2.9 ms vs new 3.0 ms (delta -0.0), catalog_write -90, wall -77.
- Memory: up to 64 MiB more page cache during an import (restored afterwards).

Commit 3 (lower(id) index rebuilt with the others) vs c2 (c1 still present at the time), r7 (n=25, quiet): `catalog_write{winget}` paired med **+52 ms** (max base 945, new 949); scoop series r8 (n=15): +95 ms catalog_write, scoop_write +0.11 ms. Rule (b) is <= +20 ms -> failed -> reverted (ed913c9). Rules (a) (show.find_by_id / show wall) not measured: (b) already decided it; the Task 73 numbers (14.6 -> 0.2 ms, wall -15.8 ms) stand.

Decisions:
- Test 3 for commit 3 ("3 testes de 2348ee9"): that commit only has 2 (unit EXPLAIN test and `find_by_id_is_the_same_with_and_without_the_index`); I ported both and added `rebuilt_import_keeps_the_lower_id_index`.
- Reverts done on the final state in the order c3, then c1 (conflict resolved by hand) instead of leaving a stack that depended on c1.
- Files in the working tree were CRLF after my python edits; normalized to LF (git stores LF anyway) because `mutate.ps1` anchors failed on CRLF.

Concerns:
- User TUI running during all measurements; see caveat. The c2 verdict depends on r6/r9 (-60/-50 cache-only) and r10 (-67 final); numbers are close to the -40 threshold.
- The investigation's -213/-303 ms for the index rebuild did not reproduce here (-8 ms quiet). If the orchestrator wants it retested on a clean machine, the code is in commit 6e164d2 (reverted by 094d378) and the c3 variant in 89d7ba0.

## Audit fix
`import_restores_cache_size` now sets `cache_size` to -1234 before the import. Mutation (restore -2000 instead of prev):
`[OK] mutação PEGA: 'pragma_update(None, "cache_size", prev)' -> 'pragma_update(None, "cache_size", -2000i64)' fez o teste falhar (exit 101).` (failing test: `catalog::tests::import_restores_cache_size`). Gate: `[OK]   test (639 passed, 16 ignored) 162s` / `[OK]   gate verde`.
