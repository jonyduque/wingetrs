Status: DONE_WITH_CONCERNS

Commits:
e7b560e feat(install): write manifest.json into the version folder and read it back

Gate: full `scripts/verify.ps1` did not reach green because of OTHER agents' files (first fmt: tests/it/prelude_core.rs; last run: clippy, `tests/it/wi_msix.rs:102` unresolved import `msix_win::SystemMsix`). rules, rules-selftest, fmt passed in the last run. Own paths proven instead:
- `cargo test --quiet --test it install_` -> `test result: ok. 342 passed; 0 failed; 17 ignored`
- `cargo test --quiet --lib` -> ok. 62 passed
- `cargo clippy --all-targets --message-format short` filtered by my files: no diagnostics.

Mutation proofs:
- planner (AnchorFile, rustfmt splits the literal line into 3 lines): `[OK] mutação PEGA: '    steps.push(Step::WriteManifest {` ... (tests 1/2 fail)
- executor: `[OK] mutação PEGA: 'run.push(Undo::RemovePath(dest.clone()));' -> 'let _ = &dest;' fez o teste falhar (exit 101).`
- state: `[OK] mutação PEGA: 'if crate::cache::sha256_hex(&bytes) != expected_sha256 {' -> 'if false {' fez o teste falhar (exit 101).`

Decisions:
- Executor mutation first SURVIVED: Promote's undo already removes the whole version dir, so the manifest undo is redundant for the dir check. Test 3 also asserts `undone` contains "gravar manifesto" (kills it).
- The planner anchor line is split by rustfmt (struct literal), so it is mutated via AnchorFile.
- Files beyond the list (mechanical consequences of the new Step): tests/it/install_planner.rs (match arm + expected kinds), tests/it/install_executor.rs (3 expected file lists now include manifest.json), tests/it/install_ops.rs (step span count 15 -> 17), 17 insta snapshots tests/it/snapshots/it__install_planner__*.snap (new dry-run line; insta also rewrote their metadata header).

Concerns: gate not green end-to-end due to other agents' files (see above).
