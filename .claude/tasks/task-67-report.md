Status: DONE

Commits:
126ee10 feat(config): behavior section with read-modify-write updates
718968f test(config): invalid file kept intact, partial section, missing file

Gate:
[OK]   fmt 2s
[OK]   clippy 2s
[OK]   test (512 passed, 16 ignored) 34s
[OK]   gate verde
(2 earlier runs failed on flakes caused by parallel agents: install_executor "Acesso negado", index_file lock test; third run green)

Mutation proofs:
1. [OK] mutação PEGA: 'let mut cfg = Config::load(path)?;' -> 'let mut cfg = Config::default();' fez o teste falhar (exit 101).
2. [OK] mutação PEGA: '#[default] Always' -> '#[default] Never' (missing_behavior_section_gives_defaults)
3. [OK] mutação PEGA: removing rename_all from ResidueMode (behavior_values_are_read)

Decisions: test 5 also asserts the returned Config equals what is on disk.
Concerns: none

Review fixes: 3 tests added. Mutation proofs:
4. [OK] mutação PEGA: load(path)? -> load(path).unwrap_or_default() (update_file_refuses_invalid_file_and_leaves_it_intact)
5. [OK] mutação PEGA: removing #[serde(default)] from Behavior (partial_behavior_section_fills_defaults)
Gate: [OK]   test (518 passed, 16 ignored) 42s / [OK]   gate verde
