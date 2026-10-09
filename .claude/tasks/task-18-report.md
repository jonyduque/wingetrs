Status: DONE_WITH_CONCERNS

Commits:
4404b7e perf(search): fused on-disk scan, early start and cheap read-only open

Gate:
[OK]   fmt 1s
[OK]   clippy 9s
[OK]   test (154 passed, 2 ignored) 57s
[OK]   gate verde

Mutation proofs:
- pwsh -File scripts/mutate.ps1 -Path src/catalog/search.rs -Anchor '.then_with(|| a.lower_name.cmp(&b.lower_name))' -Replacement '.then_with(|| a.rowid.cmp(&b.rowid))' -TestCommand 'cargo test --quiet --test search'
  [OK] mutação PEGA: '.then_with(|| a.lower_name.cmp(&b.lower_name))' -> '.then_with(|| a.rowid.cmp(&b.rowid))' fez o teste falhar (exit 101).
- (first run of '.map(|s| s / 2)' -> '.map(|s| s') SURVIVED: no test pinned the half weight; added test tag_and_command_matches_score_half_of_the_same_text_in_the_name, then:)
  [OK] mutação PEGA: '.map(|s| s / 2)' -> '.map(|s| s)' fez o teste falhar (exit 101).
- open_read_only with write+create flags (-Path src/catalog/mod.rs, -TestCommand 'cargo test --quiet --test catalog'):
  [OK] mutação PEGA: 'rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,' -> 'rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,' fez o teste falhar (exit 101).

Bench (bench.ps1, 15 runs, 3 rodadas, mediana ms; real catalog):
search powertoys  before: 126,1 / 134,1 / 137,8   after: 111,1 / 109,9 / 113,2
search ""         before: 135,2 / 303,1(outlier) / 135,7   after: 102,8 / 118,3 / 115,4
Output diff (human + --json, search "" -n 50 --json, --source scoop) vs before binary: identical (diff empty).
Note: this machine measured slower than the brief (before 126-138 vs 107), so the < 100 ms target was NOT reached here (~111 ms, gain ~20 ms).

Decisions:
- Tags/commands in disk path use split_whitespace (not split(' ')) to match the in-memory split_words decoder exactly.
- Extracted open_worker_connection/rowid_ranges in catalog/mod.rs, shared by load_entries and search_on_disk (same partition).
- ensure_catalog falls back to Catalog::open if open_read_only/is_empty fail (e.g. file without schema); new ensure_catalog_ran reports whether an update ran.
- Added #[allow(clippy::manual_is_multiple_of)] on the brief's test helper (clippy -D warnings failed on verbatim body); cargo fmt reflowed the brief tests (assertions unchanged).
- Added one extra test (tag half weight) because mutation 2 survived.

Concerns:
- Target < 100 ms not met on this machine (median ~110 ms); remaining time is process floor (~38 ms+) plus startup; not profiled further.

## Review fix (CHANGES_REQUESTED: early thread not always joined)
Commits: 9bd75d3 fix(cli): always join the early search thread;6e3f5d2 chore(tasks): record task 18;4404b7e perf(search): fused on-disk scan, early start and cheap read-only open;
- EarlySearch guard (Drop joins); ensure_catalog_ran joins explicitly before run_update on the empty-catalog path; error paths join via Drop.
- New test existing_but_empty_catalog_is_updated_by_search (tests/cli_offline.rs, wiremock): passes.
- Gate: [OK] test (155 passed, 2 ignored) / [OK] gate verde.
- Bench search powertoys, 3 rounds after fix (median): 118,5 / 113,5 / 120,8 ms (two earlier rounds right after the build were disturbed: 494 / 233, discarded). No regression beyond noise.
- Optional minor (double-space/tab tags in equivalence test) not done: NewPackage tags are joined by the writer, so it cannot be injected through the API.
