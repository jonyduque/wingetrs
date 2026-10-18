Status: DONE

Commits:
2fba60c feat(cli): --allow-scripts confirmation with gate flags and numbered scripts

Gate:
[OK]   rules 10s
[OK]   rules-selftest 24s
[OK]   fmt 3s
[OK]   clippy 2s
[OK]   test (1704 passed, 85 ignored) 51s
[OK]   gate verde

(First gate run failed only on `cli_parse::help_snapshots`, which was mine: the new flag in the help of install/upgrade/uninstall. I updated the 3 snapshots with INSTA_UPDATE and ran the gate a second time, shown above.)

Module: `cargo test --quiet --test it cli_scripts::` -> `test result: ok. 13 passed; 0 failed`

Mutation proofs:
1. `pwsh -File scripts/mutate.ps1 -Path src/cli/install_cmd.rs -Anchor 'if needs_scripts && !f.allow_scripts {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it cli_scripts::yes_without_allow_scripts_errors'`
   `[OK] mutação PEGA: 'if needs_scripts && !f.allow_scripts {' -> 'if false {' fez o teste falhar (exit 101).`
2. rustfmt splits the brief's line over 5 lines (`if/else` is longer than single_line_if_else_max_width), so I used -AnchorFile/-ReplacementFile (scratchpad t161-anchor.txt / t161-repl.txt) with the real text:
   `pwsh -File scripts/mutate.ps1 -Path src/install/ops.rs -AnchorFile <t161-anchor.txt> -ReplacementFile <t161-repl.txt> -TestCommand 'cargo test --quiet --test it cli_scripts::ops_masks_runners_without_consent'`
   `[OK] mutação PEGA: '            scripts: if allow.scripts {\n                ctx.runners.scripts.clone()\n            } else {\n                None\n            },' -> '            scripts: ctx.runners.scripts.clone(),' fez o teste falhar (exit 101).`
3. Gate: green (above).

Decisions:
- What counts as third-party code: the brief's step list plus the `third_party_uninstaller`/`has_uninstall_scripts` flags of the plan (151/159), since the executor refuses those too. The new manifest's uninstall code comes from the plan's `WriteManifest` text, resolved with `Arch::host()` like `plan_for` does. The block names it "<hook>, na remoção". An old version's hooks during an upgrade are named "<hook> da versão instalada". Uninstall reads the written manifest through the new `ops::uninstall_manifest`, which shares `recorded_manifest` with `plan_uninstall`.
- `ops::Uninstalled { residues, notes }` comes from `uninstall_with`. `uninstall` (TUI) still returns `Vec<Residue>`. The CLI prints the notes as `aviso: <nota>` on stderr after "Desinstalado:" (Task 151 review). This is tested in `uninstall_shows_skipped_uninstaller_note` through the binary with an altered manifest.json.
- The block appears only with `--dry-run` or `--allow-scripts`. Without the flag the error comes first, with nothing downloaded. The block also shows a verdict line "Análise: pede confirmação / pede atenção redobrada; revise antes de permitir". Third-party text (script, file, args, flag detail) goes through `printable` (control characters become U+FFFD), so a manifest cannot inject escape sequences into the terminal.
- Text tests (header/flags/numbering, dry-run, "segur") run the real binary through `cli_install::Lab`, since in-process stdout cannot be captured. Under `WINGETRS_TEST_ENV_KEY` the binary never runs third-party code. Host tests run in process with a counting `ScriptHost`.
- The generic UNCANCELLABLE text was already done by Task 160 (`install_cmd::uncancellable`: "um passo já começou e não pode ser interrompido (<passo>); esperando terminar", test `script_host_win::uncancellable_names_the_step`). I kept it unchanged.

Concerns:
- Files outside the list, each changed for a reason: `tests/it/cli_parse.rs` and `tests/it/wi_cli.rs` construct `Command::*` literally and needed the new field. The 3 `tests/it/snapshots/it__cli_parse__help_*.snap` gained the new flag; insta also rewrote the `source:` header of help_install. `src/install/prod_win.rs` only had a stale comment updated ("until Task 161").
- `ExecError::NeedsScriptConsent` still says "(fase 2c, Task 161)" in the user-facing text (executor.rs, not on my list). The CLI no longer reaches it, but the TUI does until Task 162.
- The `installer: if allow.scripts {...}` mask (installer runner) has no mutation proof of its own. Test 11 covers the scripts mask, and the uninstaller/installer-file cases are blocked earlier by the CLI gate.
- `.join` with external data: none written.

## Review fixes (cc010e1 fix(cli): review of task 161)
1. Installer-runner mask: new test `cli_scripts::ops_masks_installer_runner_without_consent` (`installer.file` x.exe -> `RunInstaller`, counting fake runner, `apply_with(.., &Allow::default())` -> `NeedsScriptConsent`, 0 calls).
   `pwsh -File scripts/mutate.ps1 -Path src/install/ops.rs -AnchorFile <t161-anchor3.txt> -ReplacementFile <t161-repl3.txt> -TestCommand 'cargo test --quiet --test it cli_scripts::ops_masks_installer_runner_without_consent'`
   `[OK] mutação PEGA: '            installer: if allow.scripts {\n                ctx.runners.installer\n            } else {\n                None\n            },' -> '            installer: ctx.runners.installer,' fez o teste falhar (exit 101).`
2. `printable` moved to `wingetrs::install::script_scan::printable` (pub, for the TUI to reuse). Control characters and hidden format characters (U+00AD, U+061C, U+180E, U+200B-200F, U+202A-202E, U+2060-2064, U+2066-206F, U+FEFF, U+FFF9-FFFB) are shown as `\u{xxxx}`, and tabs are kept. Test: `printable_shows_control_and_hidden_format_characters`.
3. `plan_third_party`: `RunInstaller` -> Program and `RunPs1Installer` -> Ps1, decided by the step kind. Only `uninstaller.file`/`UpgradeRunUninstaller` still go by the extension, which is how the executor decides them. Test: `installer_kind_follows_the_step`.
Module: `test result: ok. 16 passed; 0 failed`. clippy --all-targets: nothing on my paths. rustfmt --check: clean on my files.
Gate (one run): it failed only on `install_file_installer::no_consent_fails_before_anything`, which asserts the text "Task 161" in `ExecError::NeedsScriptConsent`. Task 162 changed that text in `src/install/executor.rs`, which is not committed and not mine. The rest passed: 1626 passed, 1 failed.
