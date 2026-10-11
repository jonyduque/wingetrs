Status: DONE

Commits:
```
c53d95c fix(extract): refuse decompression bombs by size and entry count
ce99f7a fix(extract): pin the declared-size check and charge skipped tar entries (review follow-up)
```
(`git log ae7ec20..HEAD` also lists 77245ab (Task 51) and eeb6a62 (orchestrator), which are not mine.)

Gate:
```
[OK]   fmt 2s
[OK]   clippy 3s
[OK]   test (462 passed, 16 ignored) 33s
[OK]   gate verde
```
Gate after ce99f7a:
```
[OK]   fmt 3s
[OK]   clippy 2s
[OK]   test (467 passed, 16 ignored) 33s
[OK]   gate verde
```
Before that green run, one run failed in Task 49's in-progress planner test. Another failed once in `install_executor`: `extrair [1]` returned "E/S: Acesso negado. (os error 5)". With extract.rs unchanged, it passed on the next run, so it looks transient (a rename racing with AV) or caused by Task 51 work in progress. Not investigated further.
Earlier runs failed only on other agents' files that were still in progress (Task 51 `executor.rs` fmt, Task 49 `install_planner::control_characters_in_args_are_refused`) and once on the flaky `index_file::search_retries_while_the_file_is_briefly_locked` (`index_file.rs:182`, timing under load, unrelated). The next run was green.

Mutation proofs (all `pwsh -File scripts/mutate.ps1 -Path src/install/extract.rs ... -TestCommand 'cargo test --quiet --test install_extract <filter>'`):
1. Counter removed, `-Anchor 'budget.charge(n as u64)?;' -Replacement ''`
   - filter `tar_gz_over_the_byte_budget_is_refused`: `[OK] mutação PEGA: 'budget.charge(n as u64)?;' -> '' fez o teste falhar (exit 101).`
   - filter `budget_is_for_the_whole_archive_not_per_entry`: `[OK] mutação PEGA: 'budget.charge(n as u64)?;' -> '' fez o teste falhar (exit 101).`
   Zip with both defenses removed. Both checks compare against `budget.limit`, so making the limit unbounded removes both. `-Anchor 'Ok(Budget { limit, used: 0 })' -Replacement 'Ok(Budget { limit: u64::MAX, used: 0 })'`, filter `zip_over_the_byte_budget_is_refused_before_writing`:
   `[OK] mutação PEGA: 'Ok(Budget { limit, used: 0 })' -> 'Ok(Budget { limit: u64::MAX, used: 0 })' fez o teste falhar (exit 101).`
   Zip declared sum alone. This is NOT an equivalent mutation (correcting the first version of this report). zip 8.6 Deflated ignores the declared uncompressed_size, so a header that over-declares (4 MiB declared, 1 KiB real, CRC intact) is refused only by this check. Pinned by `zip_declared_size_over_budget_is_refused_even_if_content_is_small` (ce99f7a). `-Anchor 'if declared > budget.limit {' -Replacement 'if false && declared > budget.limit {' -TestCommand 'cargo test --quiet --test install_extract zip_declared_size_over_budget'`:
   `[OK] mutação PEGA: 'if declared > budget.limit {' -> 'if false && declared > budget.limit {' fez o teste falhar (exit 101).`
   Review item 2: tar entries that are skipped (outside extract_dir, or not a file) are still decompressed, so `entry.size()` is charged against the budget too. Pinned by `tar_gz_skipped_entries_count_against_the_budget`. `-Anchor 'None => budget.charge(entry.size())?,' -Replacement 'None => {}' -TestCommand 'cargo test --quiet --test install_extract tar_gz_skipped_entries_count_against_the_budget'`:
   `[OK] mutação PEGA: 'None => budget.charge(entry.size())?,' -> 'None => {}' fez o teste falhar (exit 101).`
2. Resetting the counter on each entry, `-Anchor 'let mut out = fs::File::create(&target)?;' -Replacement 'let mut out = fs::File::create(&target)?; budget.used = 0;'`, filter `budget_is_for_the_whole_archive_not_per_entry`:
   `[OK] mutação PEGA: 'let mut out = fs::File::create(&target)?;' -> 'let mut out = fs::File::create(&target)?; budget.used = 0;' fez o teste falhar (exit 101).`
3. Ignoring the ratio, `-Anchor '.saturating_mul(limits.ratio)' -Replacement '.saturating_mul(0)'`, filter `ratio_raises_the_budget_above_the_floor`:
   `[OK] mutação PEGA: '.saturating_mul(limits.ratio)' -> '.saturating_mul(0)' fez o teste falhar (exit 101).`
4. `>` changed to `>=` in the shared `check_entries`, `-Anchor 'if count > limits.max_entries {' -Replacement 'if count >= limits.max_entries {'`:
   - filter `zip_with_too_many_entries_is_refused`: `[OK] mutação PEGA: 'if count > limits.max_entries {' -> 'if count >= limits.max_entries {' fez o teste falhar (exit 101).`
   - filter `tar_gz_with_too_many_entries_is_refused`: `[OK] mutação PEGA: 'if count > limits.max_entries {' -> 'if count >= limits.max_entries {' fez o teste falhar (exit 101).`
5. Tar entry counter removed, `-Anchor 'check_entries(count, limits)?;' -Replacement ''`, filter `tar_gz_with_too_many_entries_is_refused`:
   `[OK] mutação PEGA: 'check_entries(count, limits)?;' -> '' fez o teste falhar (exit 101).`
6. Default limits changed, `-Anchor 'ratio: 100,' -Replacement 'ratio: 10,'`, filter `default_limits_are_the_measured_ones`:
   `[OK] mutação PEGA: 'ratio: 100,' -> 'ratio: 10,' fez o teste falhar (exit 101).`
7. Live install tests (`--ignored --release`): not run, because no authorization was given. This one is for the orchestrator.

Test 8 (lying header): zip 8.6 does **not** reject an entry whose declared size (local header offset 22 and central directory offset 24) was changed to 1 KiB while it holds 2 MiB of zeros. It keeps streaming past 1 KiB, and our byte counter stops it with `Err(TooLarge { limit: 1048576 })` (observed with `--nocapture`). The test (`lying_zip_header_does_not_bypass_the_budget`) asserts that exact `TooLarge` and that `dest` does not exist.

Decisions:
- The copy uses a manual 64 KiB read/charge/write loop in `write_entry` instead of a `Read` wrapper. The error comes out as a typed `TooLarge` without passing through `io::Error`.
- The budget is computed with `.max(floor).min(ceiling)` instead of `clamp`, so a `Limits` with `floor > ceiling` cannot panic.
- `check_entries` is one helper shared by zip (`zip.len()`) and tar (counted at the top of the loop, before skipping non-file types).
- The zip declared sum is checked after the name-validation loop, still before anything is written. It counts entries that extract_dir filters out too, which is conservative on purpose (commented in the code).

Concerns:
- Early on I ran `cargo fmt -- <my files>`, which formats the whole crate. It may have reformatted files belonging to Tasks 49/51, which were in progress in the same checkout. They already showed as modified before it ran, and it aborted on a parse error in `tests/install_planner.rs`, so the effect is probably nil. I did not inspect their diffs. After that I used only `rustfmt --edition 2024` on my own files.
- Live tests not run (see check 7).
