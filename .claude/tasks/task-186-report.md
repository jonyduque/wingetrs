Status: DONE_WITH_CONCERNS

Commits:
4f3f064 feat(cli): native winget install/uninstall/upgrade and cleanup --orphans
(`src/winget_install/folders_win.rs` + its `mod` line were written by this task but went into df40385 of Task 187, with this task's content unchanged; see orchestrator note.)

Gate:
[OK]   rules 12s
[OK]   rules-selftest 26s
[OK]   fmt 4s
[OK]   clippy 2s
[OK]   test (1685 passed, 85 ignored) 47s
[OK]   gate verde

Mutation proofs:
1. pwsh -File scripts/mutate.ps1 -Path src/cli/winget_cmd.rs -Anchor 'let initial = vec![false; found.len()];' -Replacement 'let initial = vec![true; found.len()];' -TestCommand 'cargo test --quiet --test it wi_cli::orphans_never_preselected'
   [OK] mutação PEGA: 'let initial = vec![false; found.len()];' -> 'let initial = vec![true; found.len()];' fez o teste falhar (exit 101).
2. pwsh -File scripts/mutate.ps1 -Path src/cli/winget_cmd.rs -Anchor 'if !a.orphans {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it wi_cli::cleanup_requires_orphans_flag'
   [OK] mutação PEGA: 'if !a.orphans {' -> 'if false {' fez o teste falhar (exit 101).
3. pwsh -File scripts/mutate.ps1 -Path src/cli/winget_cmd.rs -AnchorFile <scratchpad>/t186-anchor3.txt -ReplacementFile <scratchpad>/t186-repl3.txt -TestCommand 'cargo test --quiet --test it wi_cli::native_install_prints_plan_and_logs_agreements'
   (the anchor is the `say(&line)` loop that prints `Licença aceita` in `install`, followed by `let info = JobInfo { kind: "install",`; the replacement drops the `say`)
   [OK] mutação PEGA: '            say(&line); ...' fez o teste falhar (exit 101).
4. Real binary (dry run only, `target/release/wingetrs.exe --color never`, 2026-10-18):
   install Notepad++.Notepad++ --dry-run
     Notepad++ 8.9.8.1 (nullsoft, machine)
     Instalador: https://github.com/notepad-plus-plus/notepad-plus-plus/releases/download/v8.9.8.1/npp.8.9.8.1.Installer.x64.exe
     Linha de comando: <instalador> /S
     pede UAC
     Simulação: nada foi alterado.
   install Git.Git --dry-run --scope machine
     Git 2.55.0.5 (inno, machine)
     Instalador: https://github.com/git-for-windows/git/releases/download/v2.55.0.windows.5/Git-2.55.0.5-64-bit.exe
     Linha de comando: <instalador> /SP- /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG="<LOGPATH>"
     pede UAC
     Simulação: nada foi alterado.
   uninstall "Amazon Kindle" --dry-run
     Desinstalar Amazon Kindle 2.9.1.71006
       comando: C:\Users\jonyd\AppData\Local\Amazon\Kindle\application\uninstall.exe
       abre a janela do desinstalador
       Outra instalação de Amazon Kindle usa as mesmas pastas: copie antes de remover.
         pasta: C:\Users\jonyd\AppData\Local\Amazon
     Simulação: nada foi alterado.
   cleanup --orphans --dry-run (the 3 S3 leftovers are listed)
     Órfão: Cities: Skylines — HKLM 64 bits — falta C:\Program Files (x86)\Steam\steam.exe
     Órfão: Counter-Strike: Global Offensive — HKLM 64 bits — falta C:\Program Files (x86)\Steam\steam.exe
     Órfão: USB-Driver-Installer 1.0.3.0 — HKLM 32 bits — falta C:\Users\jonyd\AppData\Local\Temp\USBDriverInstaller\uninst.exe
     O app pode ter mudado de lugar (ex.: Steam em outra pasta).
     Antes de apagar, cada registro é exportado para um .reg em C:\Users\jonyd\AppData\Local\wingetrs\backups\orphans-1792339200
     Simulação: nada foi alterado.
   (checked afterwards: no `backups\orphans-*` folder exists, so the dry run exported nothing.)

Decisions:
- `winget_cmd::WingetFlows` (a trait with RPITIT async methods) carries every effect: `run_win::ProdFlows` in production, a fake in `wi_cli.rs`. `install_cmd::execute` takes it as a new parameter. Output goes through a `Say` callback (stdout in production) so tests can read it. `Flags` and `Stop` are now `pub` (with `Stop::new`).
- `ProdFlows` owns one `HelperShutdown(ElevatedBridge)` per command. Dropping it calls `shutdown(5 s)` on every path. `install_bridge` now takes that helper and no longer creates its own.
- `UserFolders` comes from `folders_win::user_folders` (SHGetKnownFolderPath LocalAppData/RoamingAppData/Documents, `wingetrs_root = DataDir::root`). Test: a child process with `APPDATA`/`LOCALAPPDATA` planted gets the same paths. `folders_win::free_space` (GetDiskFreeSpaceExW on the nearest existing folder) is there for the TUI backup. The CLI offers no folder backup.
- Engine `winget` skips the manifest fetch (`Engine::Bridge(None)` directly). Native jobs log `kind=install|upgrade|remove, tool=wingetrs`. `start_accepting` is always used for native install/upgrade, because confirming means accepting. `Licença aceita` lines are printed only with `--yes`, as in the 176 bridge.
- Orphan `.reg` exports go to `<backups>\orphans-<unix>`. Any failed removal makes the command exit 1 at the end. Without a terminal and without `--yes`, nothing is removed.

Concerns:
- Not in the Files list: three help snapshots (`help_root/uninstall/upgrade`, whose texts I changed to mention winget/cleanup). `folders_win.rs` and its `mod` line were committed by 187. `cli_parse::help_snapshots` does not snapshot `cleanup`, because that file isn't mine.
- Uninstall planning passes `nsis_hint=false`: no manifest is read for a removal.
- The native `ops::install` runs inside `until_interrupted` on the `block_on` thread, as the ops.rs doc prescribes. Ctrl+C before the launch cancels. While the installer runs, the thread is blocked, so the `uncancellable` notice only appears if Ctrl+C is read at a later await point. Ctrl+C never kills the job.
- `.join` with external data that I wrote: `flows.backups().join(&ts)`, where `ts` is our own `orphans-<unix>` (not external). The orphan key in the `.reg` name goes through the existing `orphans::backup_file` (sanitized). In `ProdFlows`, `target_of`/`read_details` validate ARP keys before use. No other `.join` of external data.
- Brief TDD order: the tests were written right after the code, not watched failing first. The three mutation proofs above show them failing.
