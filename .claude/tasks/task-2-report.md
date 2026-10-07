Status: DONE_WITH_CONCERNS

Commits:
ec75b34 feat(core): winget-style version parsing and ordering

TDD red: `cargo test --quiet --test version` -> `error[E0432]: unresolved imports wingetrs::core::version::Part, wingetrs::core::version::Version` / `error: could not compile wingetrs (test "version")`.
Green: `test result: ok. 8 passed; 0 failed`.

Gate:
[OK]   fmt 0s
[OK]   clippy 2s
[OK]   test (15 passed, 0 ignored) 4s
[OK]   gate verde

Mutation proofs (scripts/mutate.ps1 -Path src/core/version.rs, -TestCommand 'cargo test --quiet --test version'):
1. trim off (`while self.parts.last()...` -> `while false && ...`):
   [OK] mutação PEGA: ... fez o teste falhar (exit 101). -- failed in `parse_corners` (line 62), NOT compare_table.
   With `-TestCommand '... --test version compare_table'`: [X] mutação SOBREVIVEU (see Concerns).
2. empty suffix > non-empty (swap Greater/Less in Part::compare):
   [OK] mutação PEGA: ... fez o teste falhar (exit 101). (compare_table + sort_is_consistent_with_eq)
3. unknown lowest (`(true, false) => return Ordering::Less` -> Greater):
   [OK] mutação PEGA: ... fez o teste falhar (exit 101). (latest_and_unknown + sort_is_consistent_with_eq)

Decisions:
- Eq implemented as cmp()==Equal (consistent with Ord); Part derives PartialEq on raw `other` as the brief requires.
- Case folding via to_lowercase; latest/unknown detection via eq_ignore_ascii_case; `is_empty` = raw text empty.
- Digits parsed as ASCII only; u64 overflow -> integer 0, whole text in `other` (as C++ ERANGE).
- tests/version.rs: added `clippy::nonminimal_bool, clippy::double_comparisons` to the top #![allow] (clippy -D warnings rejects the literal asserts); asserts unchanged. cargo fmt reflowed the test file.

Concerns:
- Rule 1 is not observable through compare_table: ordering pads missing parts with zero parts, so trimming never changes cmp results. Trim is only observable via parts()/is_latest (parse_corners catches it). Brief's expectation that compare_table fails is inaccurate.
- .claude/tasks/task-2-base.txt is untracked (not mine), left uncommitted.

Follow-up (review): commit 9eb0c18 test(core): pin case-insensitive suffix comparison
Mutation: [OK] mutação PEGA: 'self.other.to_lowercase()' -> 'self.other.clone()' fez o teste falhar (exit 101). (suffix_comparison_is_case_insensitive)
Gate: [OK]   test (27 passed, 0 ignored) / [OK]   gate verde
