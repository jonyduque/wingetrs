Status: DONE_WITH_CONCERNS

Commits:
26afa38 feat(install): script host in the executor for install hooks

Gate: (last run of `pwsh -File scripts/verify.ps1`)
```
[OK]   rules 6s
[OK]   rules-selftest 14s
[OK]   fmt 2s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (lib) due to 2 previous errors
  error: could not compile `wingetrs` (lib test) due to 2 previous errors
  (log completo: target/verify.log)
```
The 2 clippy errors are in another agent's uncommitted work, not this task's files
(`cargo clippy --quiet --all-targets --message-format short`):
```
src\winget_install\ops.rs:299:26: warning: very complex type used. Consider factoring parts into `type` definitions
src\winget_install\plan.rs:37:1: warning: large size difference between variants: the entire enum is at least 896 bytes
```
No other clippy output. The test step, run by hand the way the gate runs it (`INSTA_UPDATE=no cargo test --quiet --all-targets`): exit=0
```
test result: ok. 62 passed; 0 failed; 1 ignored; ...
test result: ok. 30 passed; 0 failed; 0 ignored; ...
test result: ok. 1381 passed; 0 failed; 81 ignored; ...
```
`install_scripts::` alone: `test result: ok. 19 passed; 0 failed`.

Mutation proofs:
1. `pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'if has_hooks && ctx.runners.scripts.is_none() {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it install_scripts::no_consent_fails_before_anything'`
   `[OK] mutação PEGA: 'if has_hooks && ctx.runners.scripts.is_none() {' -> 'if false {' fez o teste falhar (exit 101).`
   (With the mutant the install got as far as step 10 (download, extract and the rest) before the hook refused. The test failed on its `NeedsScriptConsent` match, which comes before its 0-requests check.)
2. `pwsh -File scripts/mutate.ps1 -Path src/install/runner.rs -Anchor 'HookKind::PostInstall => cx.current.clone(),' -Replacement 'HookKind::PostInstall => cx.version_dir.clone(),' -TestCommand 'cargo test --quiet --test it install_scripts::dir_per_hook'`
   `[OK] mutação PEGA: 'HookKind::PostInstall => cx.current.clone(),' -> 'HookKind::PostInstall => cx.version_dir.clone(),' fez o teste falhar (exit 101).`
3. Swapped the two planner blocks that push `RunInstaller` and `Hook(InstallerScript)` (`-AnchorFile`/`-ReplacementFile`, scratchpad `t156-anchor.txt`/`t156-repl.txt`), `-TestCommand 'cargo test --quiet --test it install_scripts::hooks_run_in_scoop_order'`
   `[OK] mutação PEGA: '        if spec.file.is_some() || !spec.args.is_empty() {` (the script prints the multi-line anchor; the line starts with `[OK] mutação PEGA`)
Extra mutations (self-review):
- `if report.verdict == Verdict::Refuse {` -> `if false {` (refused_script_fails_at_planning): `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`
- `    if written > 0 {` -> `    if written > u64::MAX {` (effects_not_yet_supported_fail): `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`
- `if !names.iter().any(|m| m.eq_ignore_ascii_case(&n)) {` -> `if true {` (env_seed_names_differing_only_in_case_appear_once): `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`

Decisions:
- `$__wr_helpers` must be a hashtable (the prelude calls `.ContainsKey`), and `VarValue` had no variant for that. I added `VarValue::Table(Vec<(String,String)>)`, rendered as `@{'k'='v';...}`, to `src/install/runner.rs` (not in Files). `$__wr_env_seed` is passed as `Str` holding the env.json text, because the prelude runs `ConvertFrom-Json -InputObject` on it.
- `installer` follows Scoop's `Invoke-Installer` (install.ps1:106): the file runs only with `file` or `args`. A script-only `installer` therefore gets no `RunInstaller`. Before this change, `installer: {}` ran the first download.
- env.json names are deduplicated ignoring case. PowerShell 5.1's `ConvertFrom-Json` rejects keys that differ only in case (`Path` and `PATH`). The `tmp` folder is created through `run.ensure_dirs`, so it is journaled. The work folder is `script-<app>-<hook key with . -> ->-<seq>`.
- The D5 env diff is also added when the host returns an io error, not only on code != 0. Any byte in `effects.jsonl` counts as "has effects": the file's length is checked and never read.
- safepath / external-data joins: `tmp.join("script-{app}-...")` uses `app`, which `check_component` (safepath) already checked. Every other `work.join(...)` and `layout.current(helper).join(exe)` takes a constant name. Manifest file names (`fnames`) and env names reach PowerShell only as quoted strings and are never joined into paths.

Concerns:
- Files outside the list that I had to touch: `src/install/prod_win.rs` (`scripts: None`, needed to compile) and `src/install/runner.rs` (`VarValue::Table`). Tests that relied on `pre_install`/`post_install`/`installer.script` being refused at planning were updated: `tests/it/install_inno.rs` (`scripts: None`), `tests/it/install_planner.rs` (match arm), `tests/it/install_manifest.rs`, `tests/it/manifest_2c.rs`, `tests/it/install_file_installer.rs` (test renamed to `ps1_still_refused_with_old_text`) and `tests/it/install_ops.rs` (now uses `pre_uninstall`).
- The fixture `unsup-innosetup__main__boost.json` is no longer refused (innosetup plus a benign `pre_install`), so the prefix-classification test failed on it. I renamed it with `git mv` to `hooks-innosetup__main__boost.json` and added its plan snapshot `tests/it/snapshots/it__install_planner__hooks-innosetup__main__boost.snap` (I checked the content: step 5 is "Rodar o script pre_install do manifesto (PowerShell)").
- The gate is not green because of clippy in `src/winget_install/*` (another agent's work). The orchestrator should rerun it once that work lands.
- `AwaitWorker` gets the work folder as its "staging". If the install is cancelled during a hook, its drop logs "cancelada durante a extração", which is misleading wording but harmless.
