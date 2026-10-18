Status: DONE_WITH_CONCERNS

Commits:
2cc2d45 feat(install): prelude group 3 (helper extraction, external commands, persist)

Gate:
Gate NOT green because of other agents, not this task. Last run of `pwsh -File scripts/verify.ps1` (run twice; the first run was blocked by a half-edited src/install/inno.rs from another agent, `all(|c| true)`, which was reverted by them):
[OK]   rules 6s
[OK]   rules-selftest 14s
[OK]   fmt 2s
[OK]   clippy 73s
[FAIL] test exit 101
  error: linking with `link.exe` failed: exit code: 1104   (LNK1104: cannot open target\debug\deps\wingetrs-9468d8e47e96698c.exe)
leftovers.ps1 showed other agents' `cargo test --quiet --all-targets` holding that lib-test exe. I did not kill them.
Proof for my part (the `it` binary compiles again after Task 180 settled):
- `cargo test --quiet --test it prelude_`            -> test result: ok. 7 passed; 0 failed; 42 ignored (includes the pure `prelude_text_is_declared_ascii_and_clean`)
- `cargo test --quiet --test it prelude_ -- --ignored` -> test result: ok. 42 passed; 0 failed (all 15 new tests + groups 1-2 with real pwsh)

Mutation proofs (mutate.ps1, run against a temporary `tests/t154_tmp.rs` that included prelude_extract.rs because `it` did not compile at the time; file removed afterwards):
[OK] mutação PEGA: 'if ($count -eq 0) {' -> 'if ($false) {' fez o teste falhar (exit 101).     (inno_args_equal_rust_and_zero_files_throws)
[OK] mutação PEGA: 'if ($RunAs) {' -> 'if ($false) {' fez o teste falhar (exit 101).          (invoke_external_command_runas_throws)
[OK] mutação PEGA: 'ContainsKey($code)' -> 'ContainsKey(-12345)' fez o teste falhar (exit 101). (invoke_external_command_codes)
Note: the RunAs anchor is `if ($RunAs) {`, not the brief's literal line: the prelude must be ASCII, so the message is built with [char] (`'eleva' + [char]0xE7 ...`) and the brief's literal text cannot appear. The test uses a nonexistent exe so the mutant cannot raise a UAC prompt.

Decisions:
- Declared list in tests/it/prelude_core.rs (FUNCTIONS 35 -> 44) updated as the orchestrator said; forbidden-text list unchanged. Internal helpers are `__wr_*` (helper, safe_rel, run, dirof, persist_def) so they are not "declared".
- Extra guards (manifest/archive-derived paths, safepath rule), each with a test: `__wr_safe_rel` on ExtractDir, the inner tar name and Dark payload names; persist definitions all validated before anything is created (`persist_rejects_paths_outside`, `extract_dir_dotdot_throws`); `Invoke-ExternalCommand` refuses a `msi*exec` file name without the token appearing in the text (`invoke_external_command_refuses_windows_installer`).
- `Invoke-ExternalCommand` keeps the exit code in `$script:__wr_last_exit` so the throw says "(código N)"; the original msiexec log branch was dropped (token forbidden). `attrib.exe` is called by `$env:SystemRoot\System32` path, not PATH. Expand-ZipArchive uses ZipFile.ExtractToDirectory(overwrite) as the brief says (the original uses Expand-Archive).
- TDD order: tests and prelude written together; first run had one failure (typo "suportado" vs "suportada"), fixed. A careless `sed` also touched the other "suportado" lines; reverted, final diff is additions only (509 lines).

Concerns:
- Gate not green for the reason above; the orchestrator should rerun `verify.ps1` when no other cargo is running.
- All 15 new tests are `#[ignore = "live: roda pwsh real"]`; they run only with `-- --ignored`.
- Brief anchors were not honoured literally (RunAs, above); `if ($count -eq 0) {` and `ContainsKey($code)` are literal in the code.
- Helper args for `.cmd`/legacy commands get drive-letter quoting (faithful to Scoop); real `.exe` helpers go through ArgumentList. Tar flow (IsTar recursion) is ported but has no test (not in the brief list).
