Status: DONE_WITH_CONCERNS

Commits:
```
825d893 perf(winget): validate the index msix as an inflate and hash pipeline
```

Gate:
```
[OK]   rules 3s
[OK]   rules-selftest 9s
[OK]   fmt 1s
[OK]   clippy 9s
[OK]   test (494 passed, 16 ignored) 173s
[OK]   gate verde
```
(The first gate run failed in clippy only on `src/catalog/mod.rs:171` "field `truncated_maps` is never read", which is Task 55's in-progress file. I waited until clippy was clean and re-ran the gate.)

Mutation proofs (all with `-Path src/sources/winget/index.rs -TestCommand 'cargo test --quiet --test winget_index'`):
1. `-Anchor 'first_bad = Some(i);' -Replacement 'first_bad = None;'`
   `[OK] mutação PEGA: 'first_bad = Some(i);' -> 'first_bad = None;' fez o teste falhar (exit 101).` (corrupt_block_is_rejected + pipelined test)
2. `-Anchor 'file_hash.is_none_or(' -Replacement 'true || file_hash.is_none_or('`
   `[OK] mutação PEGA: 'file_hash.is_none_or(' -> 'true || file_hash.is_none_or(' fez o teste falhar (exit 101).` (bad_file_hash_is_rejected + pipelined test)
3. `-Anchor 'if zip.index_for_name(INDEX_ENTRY).is_none() {' -Replacement 'if false {'`
   First run, with the brief's case list: `[X] mutação SOBREVIVEU: 'if zip.index_for_name(INDEX_ENTRY).is_none() {' -> 'if false {' e os testes passaram. A regra não está coberta.`
   I added one case at the end of `cases` (see Decisions). After that:
   `[OK] mutação PEGA: 'if zip.index_for_name(INDEX_ENTRY).is_none() {' -> 'if false {' fez o teste falhar (exit 101).`
4. `-Anchor 'read_entry(&mut zip, INDEX_ENTRY)?;' -Replacement ''`
   `[OK] mutação PEGA: 'read_entry(&mut zip, INDEX_ENTRY)?;' -> '' fez o teste falhar (exit 101).`
5. Multi-line anchor `produced?;` + count `if` + Integrity `if`, replaced by count `if` + Integrity `if` + `produced?;` (so `produced?` moves after Integrity):
   `[OK] mutação PEGA: '        produced?; ...` (pipelined test, panicked at tests\winget_index.rs:339)
6. Multi-line anchor count `if` + Integrity `if`, replaced by Integrity `if` + count `if` (order swapped):
   `[OK] mutação PEGA: '        if hashes.len() != db.len().div_ceil(BLOCK_SIZE) { ...` (pipelined test, panicked at tests\winget_index.rs:339)
7. Not run, as the brief requires (removing `drop(block_tx)` would hang). By reading the code: both `Sender`s are dropped before both `join`s, and both `join`s use `unwrap_or_else(|p| std::panic::resume_unwind(p))` before `produced?`.
8. `check_rules.ps1`: `[OK]   rules 3s`. `grep -n "open_with_flags\|Connection::open" src/sources/winget/index.rs` gives only `206:        rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)`, which is the line `read_index` already had.
9. Gate green (above).

Decisions:
- The implementation is the brief's reference code, essentially verbatim.
- Mutation 3 survived the brief's cases. The fallback branch in `read_blockmap` (re-inflating index.db) already returns `MissingEntry("Public/index.db")` when both entries are missing. The `index_for_name` guard only matters when the blockmap is present and index.db is absent: without the guard, `by_name` would map to `Zip(...)`. I appended `zip_of(&[("AppxBlockMap.xml", &bm, false)])` as the last case (index 17), so the indices 10–15 used by the assertions don't move. No assertion was changed.
- The test body was transcribed from the brief, and `rustfmt` was applied to my two files only. No type adjustments were needed.

Concerns:
- The pipelined test takes about 7 s in debug (41-block synthetic inputs, SHA in debug mode). It has no wall-clock thresholds.
- End-to-end A/B: não medido (left to the orchestrator, as the brief says).
- `leftovers.ps1` shows a bash process (`sleep 240; ... verify.ps1`) started at 17:41. It belongs to another agent, not me, so I left it running.

Follow-up (review Minor):
- Commit `23e9dbb fix(winget): map thread spawn failure to an io error`
- Both workers now start with `std::thread::Builder::new().spawn_scoped(s, ..)`, and a spawn failure becomes `IndexError::Io`. If the second spawn fails, `block_tx` is dropped and the first worker is joined (`resume_unwind` on panic) before returning. No new test, as the review asked.
- Gate: `[OK]   test (495 passed, 16 ignored) 106s` / `[OK]   gate verde`
- `pwsh -File scripts/mutate.ps1 -Path src/sources/winget/index.rs -Anchor 'if n == 0 {' -Replacement 'if n < BLOCK_SIZE {' -TestCommand 'cargo test --quiet --test winget_index'`
  `[OK] mutação PEGA: 'if n == 0 {' -> 'if n < BLOCK_SIZE {' fez o teste falhar (exit 101).`
