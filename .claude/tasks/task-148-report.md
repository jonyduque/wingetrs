Status: DONE_WITH_CONCERNS

Commits:
9da3701 feat(install): install helpers (innounp, 7zip, lessmsi, dark) as hidden dependencies

Gate (verify.ps1, one run):
[OK]   rules 8s / [OK] rules-selftest 16s / [OK] fmt 3s / [OK] clippy 71s
[FAIL] test exit 101 -- the only failure is not mine (Task 180, tests/it/elevated_install.rs:777):
  elevated_install::win::staged_installer_runs_the_copy_with_its_own_temp --- FAILED
  test result: FAILED. 1302 passed; 1 failed; 75 ignored
My 7 tests (`cargo test --quiet --test it install_helpers::`): 7 passed.

Mutation proofs:
[OK] mutação PEGA: 'for h in &target.helpers {' -> 'for h in target.helpers.iter().take(0) {' fez o teste falhar (exit 101).   (test 1)
[OK] mutação PEGA: 'for h in &manifest.helpers {' -> 'for h in manifest.helpers.iter().take(0) {' fez o teste falhar (exit 101).   (test 3)
[OK] mutação PEGA: 'if depends_on_app || uses_helper {' -> 'if depends_on_app {' fez o teste falhar (exit 101).   (test 5)

Decisions:
- Test 3: the brief's `y.post_install` calling Expand-7zipArchive is refused by `refuse_2c` (visit and plan_install still refuse hooks), so y uses `innosetup:true` and the expected plan is [innounp, y, x]. Same code path (`for h in &manifest.helpers`).
- Helper visit uses parent bucket "main" with explicit `main/<app>` spec.
- Test file has its own minimal World (install_ops.rs World is private); state is recorded directly via StateDb::record.

Concerns: test 3 deviates from the brief (see above); red-first run was skipped (mutation proofs show the tests fail without the code).
