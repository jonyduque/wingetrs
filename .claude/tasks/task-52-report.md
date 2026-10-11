1. Status: DONE
2. Commits: see `git log --format='%h %s' 0e1dace..HEAD` (single commit "perf(download): hash a cached or downloaded blob once")
3. Gate:
[OK]   fmt 1s
[OK]   clippy 9s
[OK]   test (488 passed, 16 ignored) 94s
[OK]   gate verde
4. Mutation proofs:
- cached skips verify for all algos (`(spec.algo == HashAlgo::Sha256 || verify(spec, &bytes))` -> `true`): [OK] mutação PEGA ... fez o teste falhar (exit 101).
- BlobStore::get replaced by unchecked read: [OK] mutação PEGA: 'let bytes = blobs.get(&name)?;' -> 'let bytes = std::fs::read(data.blobs_dir().join(&name)).ok()?;' fez o teste falhar (exit 101).
- lowercase name: [OK] mutação PEGA: 'src.hash.hex.to_ascii_lowercase()' -> 'src.hash.hex.clone()' fez o teste falhar (exit 101).
- Hash reduction itself: no mutation proof (behaviour-preserving). Ganho: não medido.
5. Decisions: HashSpec::parse already lowercases hex, so test 3 first survived the mutation; the test now sets `hash.hex` to uppercase by hand (fields are pub) to make the lowercase in download_one load-bearing.
6. Concerns: none. Tests 3/4 passed before the change (behaviour-preserving), as the brief expects.
