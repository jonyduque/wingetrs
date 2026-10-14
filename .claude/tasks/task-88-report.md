Status: DONE_WITH_CONCERNS

Commits: see `git log --format='%h %s' 8594abe..HEAD` (fix(icons): serialize icons.json updates within the process)

Gate:
[OK]   fmt 2s
[OK]   clippy 17s
[OK]   test (690 passed, 17 ignored) 180s
[OK]   gate verde

Mutation proofs (removed the MAP_LOCK guard line, 3 runs, test with 2 rounds):
[OK] mutação PEGA: 'let _guard = MAP_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);' -> '' fez o teste falhar (exit 101).  (x3, identical lines)
Mutation also caught 3/3 with 5 rounds (failed in 0.14s, i.e. in the first round).

Decisions: test uses 2 rounds instead of 5: with 5 rounds it took 4.1s in debug (64 serialized writes, ~65ms each on this machine), over the 2s budget; with 2 rounds 1.65s. Mutation caught in the first round anyway.

Concerns: round count deviates from brief (2 vs 5) to meet the 2s limit. The Portuguese doc line on remember was transcribed verbatim from the brief.
