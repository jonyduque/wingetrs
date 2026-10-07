Status: DONE

Commits:
e52ef17 feat(winget): MSZIP decoder and versionData parsing
(base..HEAD also lists orchestrator commits 084c7a7, 9eb0c18)

Red: `error[E0432]: unresolved import `wingetrs::sources::winget::mszip`` (before implementing)

Gate:
[OK]   fmt 0s
[OK]   clippy 3s
[OK]   test (27 passed, 0 ignored) 5s
[OK]   gate verde

Mutation proofs:
- `if !out.is_empty() {` -> `if false {` (multi_block): [OK] mutação PEGA: 'if !out.is_empty() {' -> 'if false {' fez o teste falhar (exit 101).
- `if out.len() != expected {` -> `if false {` (full suite; equals returning Ok(out) on mismatch): [OK] mutação PEGA: 'if out.len() != expected {' -> 'if false {' fez o teste falhar (exit 101).
  (Anchor `return Err(SizeMismatch...)` did not match after cargo fmt split the line; used the condition instead.)

Decisions:
- Dictionary applied when out is non-empty (2nd block onward); header size never used for preallocation.
- version_data_path takes the first 4 hash bytes (8 hex) without panicking on short input.

Concerns: none
