# Task 191 report

Status: DONE_WITH_CONCERNS

Commits:
```
7ce872b fix(install): ps1 shim literals double typographic quotes, BOM, CRT backslashes
5564a5b fix(install): trust our own folder prefix in hook shim args
f40813a fix(install): phase 2c review (no elevation for file installers, consent order, effect paths, sha under lock)
```
(`git log e9227dd..HEAD -- <files>` also lists 2af05f4, which is another agent's commit touching `src/cli/install_cmd.rs`.)

Gate: ran once, and it failed in a file I don't own:
```
[OK]   rules 9s
[OK]   rules-selftest 28s
[FAIL] fmt exit 1
  Diff in ...\tests\it\install_shim.rs:431/450/460   (mine, fixed with rustfmt)
  Diff in ...\tests\it\script_scan.rs:701             (Task 164, not mine)
```
After rustfmt, `cargo fmt --check` complains only about `tests/it/script_scan.rs:701`. `cargo clippy --all-targets --quiet` gives 0 warnings/errors.
`cargo test --quiet --test it -- install_shim:: uninstall_scripts:: install_planner:: install_script_effects:: cli_scripts:: install_file_installer:: install_inno:: tui_scripts:: tui_jobs:: install_ops:: install_executor::`
-> `test result: ok. 320 passed; 0 failed; 3 ignored`.
Full `cargo test --quiet`: 1697 passed, 2 failed. Neither failure is mine: `cli_parse::help_snapshots` (another agent changed the `install` help text) and `wi_upgrade::native_agrees_with_winget_applicability`.

Mutation proofs (scripts/mutate.ps1, real output lines):
- (a) installer_win::run without refuse_if_elevated (AnchorFile = the 3 lines refuse/map_err/launch, Replacement `let _ = elevated; launch(...)`), test `file_installer_refused_when_elevated`:
  `[OK] mutação PEGA: '    refuse_if_elevated(elevated()?)`
- same for inno_win, test `innounp_refused_when_elevated`: `[OK] mutação PEGA: '    refuse_if_elevated(elevated()?)`
  (My first attempt here, `let _ = refuse_if_elevated(...)`, kept the `?` and so produced an equivalent mutant: SOBREVIVEU. I redid it with the AnchorFile and it was caught.)
- (b) removal_third_party looking only at `uninstaller.file` (the match on uninstaller_command replaced by `if let Some(f) = &u.file {...}`), test `removal_consent_lists_args_only_uninstaller`:
  `[OK] mutação PEGA: '        match planner::uninstaller_command(m, u, vdir) {`
- (b2) script listed before the file, test `removal_consent_order_matches_executor`: `[OK] mutação PEGA: '        match planner::uninstaller_command(m, u, vdir) {`
- (c) no canonicalization (`resolve_existing` replaced by the lexical path for both entry and base), test `effect_path_through_junction_refused`:
  `[OK] mutação PEGA: '    let real = resolve_existing(Path::new(entry))?;`
- dangling junction: `[OK] mutação PEGA: 'if fs::symlink_metadata(cur).is_ok() {' -> 'if false {' fez o teste`
- effect args `"` / control (before the allowlist): `[OK] mutação PEGA: '.filter(|a| a.chars().any(|c| c.is_control() || c == '"'))' -> '.fil` (x2)
- sha under the lock: `[OK] mutação PEGA: 'if scripts.is_some_and(|shown| shown != rec.manifest_sha256) {' -> '`
- removal note: `[OK] mutação PEGA: 'fail(index, step, format!("{message}{}", ran.note()))' -> 'fail(inde`
- allowlist widened with `&%$;{}`, test `shim_args_with_cmd_syntax_refused`: `[OK] mutação PEGA: 'c.is_ascii_alphanumeric() || " -_./\\:=,+".contains(c)' -> ...`
- `.ps1` back to raw interpolation, test `ps1_shim_uses_literal_array`: `[OK] mutação PEGA: 'write_text(&ps1, &ps1_text(&target, args))?;' -> 'write_text(&ps1, &format!("&`
- `$version`/`$app` check off: `[OK] mutação PEGA: 'if raw.contains(var) && !shim::is_plain_value(value) {' -> 'if false {' fez o teste falh`
- `.cmd` quoting removed: `[OK] mutação PEGA: 'out.push(format!("\"{t}\""));' -> 'out.push(t.clone());' fez o teste falhar (exit 101).`
- (5564a5b) hook arg remainder after a trusted folder not checked, test `shim_effect_args_under_non_ascii_profile`: `[OK] mutação PEGA: '        shim::check_args(tail)?;' -> '        let _ = tail;' fez o teste falhar (exit 10`
- (5564a5b) trusted prefix ignored: `[OK] mutação PEGA: 'let start = i + prefix.map_or(0, str::len);' -> 'let start = i + prefix.map_or(0, |_| 0)`
- (7ce872b, review 1) ps_literal back to escaping only ASCII `'`, test `ps1_shim_typographic_quotes_and_bom`: `[OK] mutação PEGA: '    crate::install::runner::ps_single_quoted(s)' -> '    format!("'{}'", s.replace('\'', "''"))' f`
- (7ce872b, review 1) .ps1 written without BOM, test `ps1_shim_dir_with_typographic_quote_and_o_double_acute`: `[OK] mutação PEGA: '[b"\xEF\xBB\xBF", text.as_bytes()].concat()' -> 'text.as_bytes()' fez o teste falhar (exit 101).`
- (7ce872b, review 2) trusted prefix not required to end at a separator (scratchpad t191-anchor.txt / t191-repl.txt), test `shim_effect_args_trusted_prefix_ends_at_separator`: `[OK] mutação PEGA: '                    .is_none_or(|c| matches!(c, '\\' | '/' | ' '))' -> '                    .is_no`
- (7ce872b, review 3) trailing backslashes not doubled, test `shim_args_trailing_backslash_doubled`: `[OK] mutação PEGA: 'out.push(format!("\"{t}{}\"", "\\".repeat(tail)));' -> 'out.push(format!("\"{t}\"")); let _ = tail`
- `%` allowed in quoted cmd args: `[OK] mutação PEGA: 'matches!(c, '"' | '%' | '!')' -> 'matches!(c, '"')' fez o teste falhar (exit 101).`

Decisions:
- Item 6: `uninstall_with(ctx, plan, scripts: Option<&str>)`. `Some(sha)` is the consent, tied to the hash that was shown. `uninstall_manifest` now returns `(manifest, sha)` read together. The TUI's `check_shown_manifest`/`recorded_sha` are removed, and the CLI passes the sha it showed. Adds `OpsError::ManifestChanged` (same text as before).
- Item 4: canonical comparison, not a refusal of every reparse point. The persist junctions and `current` are legitimate. The resolution takes the longest existing prefix plus the missing tail; a dangling link is refused; components are compared ignoring case.
- Shim args (coordinator's two notes): I chose a shared allowlist `shim::check_args` (ASCII alnum, space, `-_./\:=,+`, balanced `"`). The planner checks the manifest text with the variables standing in. `$version`/`$app`, if used, must match `[A-Za-z0-9._+-]` (no validator for this existed elsewhere; `check_component` allows `&`). Each argument is split cmd-style and substituted. In the `.cmd`/`.shim` it goes inside `"..."` when the manifest quoted it or it needs quoting; `"`, `%` and `!` are refused. The `.ps1` shim becomes `$path = '<target>'`, `$shimArgs = @('a', ...)`, `& $path @shimArgs @args`. Hook effects additionally refuse any `"` (brief item 3).
- Measured on catalog.db (read-only, 2026-10-18): 4118 Scoop manifests, 332 `bin` args. The allowlist newly refuses 1: `scoop:extras/controlprinters` (`shell:::{A8A91A66-...}`, because of `{`). No `bin` args use `$version`/`$app` (0). I did not widen the list.
- Addendum: the plan step is now "Gravar ..." (`src/install/plan.rs` plus 18 install_planner snapshots). `--dry-run` prints the MSI hint (`output::msi_hint`, shared with the summary). Test: `dry_run_msi_shows_allow_msiexec_hint_and_capitalized_steps`. Two existing tests in `install_shim.rs` now expect the new `.ps1` text.

Concerns:
- Files outside the brief's Files list (at the coordinator's request): `src/install/planner.rs`, `src/install/plan.rs`, 18 `tests/it/snapshots/*.snap`, `tests/it/install_shim.rs`.
- Resolved in 5564a5b, following the coordinator's instruction: hook shim args are split on spaces, and control characters are refused. An argument may start with the wingetrs root or the app/persist folder, matched both as written and resolved, ignoring case. That prefix is trusted: in the shim it is quoted, and it is refused only if it holds `"`, `%` or `!`. The rest of the argument goes through `check_args` without quotes. Test `shim_effect_args_under_non_ascii_profile`: a root under `João Silva\root` is accepted, with the expected text in both `.cmd` and `.ps1`, and `<data>\x & calc` is refused. `uninstall_scripts::` + `install_shim::` + `cli_scripts::` + `install_script_effects::`: 108 passed. Clippy is clean and fmt passes on both files. The full gate was not run again.
- `%` in a manifest `bin` *target* path still goes into the `.cmd` inside quotes, where cmd expands it. This was already the case before the task and is out of scope.
- `.cmd` text changes for quoted manifest tokens: `--user-data-dir="$dir\User Data"` becomes `"--user-data-dir=C:\...\User Data"`. It parses the same for the C runtime.
- `.join` with external data in this task: `real.extend(missing)` in `resolve_existing`. The names come from an entry already checked by `is_inside` (safe components) and only feed a comparison, never a write.
- No live test was added; nothing runs pwsh, innounp or an installer (`run_with` gets a fake launcher).
- Review fixes (7ce872b):
  1. `ps_literal` now reuses `runner::ps_single_quoted`, which doubles `'` and U+2018-U+201B. Every generated `.ps1` shim starts with a UTF-8 BOM. I checked the other places wingetrs writes `.ps1`: `runner.ps1` and the analysed `.ps1` copy already have a BOM; `prelude.ps1` is pure ASCII (0 bytes >= 0x80); `hook.ps1` is read with `ReadAllText` (UTF-8). All of them quote through `ps_single_quoted`.
  2. New test `shim_effect_args_trusted_prefix_ends_at_separator`; the mutation from your anchor files is now caught.
  3. `cmd_args` doubles trailing backslashes before the closing quote. The `.ps1` now splits the rendered args by C-runtime rules (`split_crt`), so it gets one backslash back. The manifest template is still split by plain quote toggling.
  4. Leftover comment in `cli_scripts.rs` removed. That commit touches only this one line; the other agent's NoWinget changes were already committed in 1bc1c90.
  Tests: install_shim + uninstall_scripts + cli_scripts + install_planner + install_script_effects + install_file_installer: 155 passed. Clippy is clean and fmt passes on the touched files. I did not re-run the full gate.
- The existing `install_shim` tests now expect a BOM (U+FEFF) at the start of the `.ps1` text.
