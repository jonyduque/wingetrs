Status: DONE

Commits:
efa50fb feat(tui): scrollable third-party script section in the confirmation

Gate:
[OK]   rules 10s / rules-selftest 26s / fmt 3s / clippy 20s
[OK]   test (1721 passed, 86 ignored) 63s
[OK]   gate verde

Mutation proofs:
- pwsh -File scripts/mutate.ps1 -Path src/tui/app.rs -Anchor '!needs_confirm(confirm, &actions[0]) && !has_third_party_code(app, &actions)' -Replacement '!needs_confirm(confirm, &actions[0])' -TestCommand 'cargo test --quiet --test it tui_scripts::dialog_forced'
  [OK] mutação PEGA: ... fez o teste falhar (exit 101).
  (the brief's literal `if has_third_party_code(&actions) {` does not exist; the real forcing line is in `ready_one`, `confirm_batch` has the same check)
- pwsh -File scripts/mutate.ps1 -Path src/tui/jobs.rs -Anchor 'scripts: true,' -Replacement 'scripts: false,' -TestCommand 'cargo test --quiet --test it tui_scripts::apply_scripts_reaches_host'
  [OK] mutação PEGA: 'scripts: true,' -> 'scripts: false,' fez o teste falhar (exit 101).

Decisions:
- Removal code needs the recorded manifest (I/O): new `Planned::ReadyScripts { text, action, code }` from jobs::plan_remove; App.removal_code (by app) feeds confirm_modal; the section is built once in confirm_modal (Modal::Confirm.scripts).
- Section lines wrap (headers/flags) or are cut (code) at draw time; PgUp/PgDn page by lines that fit whole (test: every line is shown); wheel over the section scrolls it.
- Refused (Verdict::Refuse) script: Enter keeps the dialog open with an error toast; the executor refuses it too.
- Removal notes (skipped uninstaller) go in Done.summary ("x removido. <nota>"), test removal_notes_reach_the_user.
- Reused script_scan::printable (control + bidi/zero-width => \u{..}); test with U+202E. executor.rs: dropped "(fase 2c, Task 161)" from NeedsScriptConsent; updated tests/it/install_file_installer.rs assertion.

Concerns:
- Unchecking the script app still shows its code (section is built for all items); consent applies only to checked actions that have code.
- ApplyMsiexec after NeedsMsiexec does not carry script consent (an ApplyScripts that hits an MSI fallback is not offered msiexec).
- bench_confirm_300_lines: 6.5 ms/frame in debug (release not measured).
- cargo fmt touched nothing outside my files (others' diffs were their own).

## Review fixes (second commit)
Commits: see `git log --format='%h %s' -- src/tui/app.rs` (fix(tui): review of task 162).
Gate: [OK] test (1726 passed, 86 ignored) / [OK] gate verde.
1. ApplyMsiexec got `scripts: bool`; on_finished matches ApplyScripts; the rest is requeued as ApplyScripts. Test msiexec_in_a_scripts_batch_keeps_the_consent_of_the_rest. Mutation `let again = if scripts {` -> `if false {`: [OK] mutação PEGA.
2. Batch tests batch_shows_removal_code_and_yields_remove_scripts and batch_dialog_forced_even_when_confirm_setting_skips. Mutation (anchor rewritten: the insert is now `.insert(p.app.to_lowercase(), (manifest_sha256, code))`, old anchor file was stale; replaced by `let _ = (p, manifest_sha256, code);`): [OK] mutação PEGA.
3. Continue is refused with a Portuguese toast when the scripts area shows fewer than 2 lines (`scripts_hidden`). confirm_geom now takes `&[ScriptLine]` and counts wrapped rows (found while testing: the section was sized by logical lines, so wrapped headers cut the code). Test window_too_small_for_the_scripts_blocks_continue; mutation `if scripts_hidden(app) {` -> `if false {`: [OK] mutação PEGA.
4. RemoveScripts is now `{ plan, manifest_sha256 }`; Planned::ReadyScripts carries the record hash; jobs compares it with the record before running (not inside the install lock: uninstall_with and the executor still check version and hash under the lock). Test remove_scripts_refuses_a_manifest_other_than_the_one_shown; mutation `Ok(now) if now == shown => Ok(())` -> `Ok(_) => Ok(())`: [OK] mutação PEGA.
Extra files: tests/it/tui_jobs.rs and tests/it/tui_state.rs (added `scripts: false` to ApplyMsiexec constructions).
