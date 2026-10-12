Status: DONE

Commits: see `git log --format='%h %s' e0e012c..HEAD -- src/tui/jobs.rs` (fix(tui): sum download progress across parallel files)

Gate:
[OK]   fmt 3s
[OK]   clippy 37s
[OK]   test (512 passed, 16 ignored) 130s
[OK]   gate verde
(two earlier runs failed on unrelated flaky tests index_file / install_state under parallel-agent load; third run green)

Mutation proofs:
[OK] mutação PEGA: 'Some((name, sum_done, sum_total))' -> 'Some((label.to_string(), done, total))' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'entries.values().map(|(_, t)| *t).sum();' -> 'Some(entries.values().filter_map(|(_, t)| *t).sum());' fez o teste falhar (exit 101).
[OK] mutação PEGA: '|| sum_total == Some(sum_done)' -> '|| total == Some(done)' fez o teste falhar (exit 101).
run.rs wiring: no mutation proof (as the brief says).

Decisions: total sum via Iterator::sum into Option<u64> (None if any None). Ran `cargo fmt` (whole project); it may have reformatted nothing outside my files.

Concerns: none

Follow-up (review Minors):
1. Completed aggregate now clears the map (entries.clear()); test finished_batch_resets_for_the_next_app.
   [OK] mutação PEGA: 'entries.clear();' -> '' fez o teste falhar (exit 101).
2. Sums use saturating_add (fold / try_fold).
Gate: [OK]   test (518 passed, 16 ignored) / [OK]   gate verde
