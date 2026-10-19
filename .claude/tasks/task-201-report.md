# Task 201 report

Status: DONE_WITH_CONCERNS

Commits:
```
0a029fa fix: task 201 review 2, UAC notice on registry-only HKLM removal, install bridge tested
08a940c fix: task 201 review, notice and phases on every elevated or uncancellable step
8a1bf9a feat: progress on every install path and an elevation notice before every UAC
```
(The output of `git log --format='%h %s' d16cf51..HEAD -- <files>` also lists Task 200's e185089 and 477c1ea, because they touch `src/cli/winget_cmd.rs`/`tests/it/wi_cli.rs`. Only 8a1bf9a, 08a940c and 0a029fa are mine.)

Gate:
```
[OK]   rules 10s
[OK]   rules-selftest 23s
[OK]   fmt 3s
[OK]   clippy 24s
[OK]   test (1881 passed, 87 ignored) 66s
[OK]   gate verde
```

## Matrix (path × install/upgrade/remove × CLI/TUI)

The CLI and the TUI consume the same `ProgressEvent`s: the CLI through `Phases`/`LineProgress` (spinner, clock, slow-warning), the TUI through `progress_msg` → `running_text` (spinner, clock, `phase_status`).

| Path | Op | Phases from the first instant (CLI = TUI) | Slow warning | "não pode ser cancelado" | Change |
|---|---|---|---|---|---|
| Scoop (executor) | install/upgrade | Preparando (before install.lock) → Baixando → Extraindo x (fmt) → Colocando → Criando comandos → Registrando | download "sem dados há N s" | — | none (locked by test) |
| Scoop scripts (PwshHost) | install/upgrade/remove | **Rodando o script <hook> do pacote** from PowerShell's start (removal: back to Removendo afterwards) | "rodando há N min; pode haver uma janela…" after 5 min | yes (TUI: `job_uncancellable`) | fixed (review) |
| Scoop `installer.file` / old uninstaller | install/upgrade | **Rodando o instalador do pacote (file; pode abrir janelas)** | same | yes | fixed (review) |
| Scoop `uninstaller.file` (exe) | remove | **Desinstalador de <app> rodando**, then Removendo | same | yes | fixed (review) |
| innounp | install | Extraindo x (Inno Setup) | — | — | none |
| msiexec /a reserve | install | Windows Installer (msiexec /a) + log tail | "sem atividade há N min", notice at 30 s | yes | none; **no UAC** (never `runas`, measured in Task 119) |
| Scoop | remove | Removendo app (before the lock) → Procurando resíduos | — | — | none (locked by test) |
| winget native (exe/msi) | install | **Preparando before install.lock** (was silent) → Baixando → [WaitingElevation → RunningInstaller] or RunningInstaller or **SelfElevating** → Verificando | "rodando há N min…" after 5 min | yes | fixed |
| winget native MSIX | install | same as above; RunningInstaller during `PackageManager` | same | yes | (via the same fix) |
| winget native | upgrade in place | same as install | same | yes | (via the same fix) |
| winget native | upgrade uninstallPrevious | **Preparando before the lock** → Baixando → **Desinstalador/WaitingElevation → Conferindo a remoção** → instalador → Verificando | same | yes | fixed (removal had no phase) |
| winget native | remove | Preparando (before lock) → [Copiando pastas] → WaitingElevation (helper) **→ Desinstalador rodando once the helper runs** / Desinstalador → Conferindo a remoção | same | yes | fixed (notice stayed during the whole uninstall) |
| winget native | registry-only | Preparando → Removendo; HKLM: **WaitingElevation (the helper's UAC notice) → Removendo**, in the CLI and the TUI | — | yes (TUI flag) | fixed (review 2) |
| orphan record (`cleanup --orphans`, TUI) | remove | TUI: **Preparando before install.lock** → Removendo. HKLM: **WaitingElevation (the helper's UAC notice) → Removendo**. CLI: the same phases, with the results printed after the line ends | — | yes (TUI flag) | fixed (review) |
| Bridge winget/scoop | upgrade/install | **Bridge `{tool} rodando (pode abrir janelas)` with clock** from the start (stderr); helper UAC line → WaitingElevation; the tool's own lines go to **stdout** (the progress line is erased first) and to the TUI label | TUI "ainda aguardando o instalador" after 10 min, CLI "rodando há N min" after 5 min | yes | fixed (CLI had plain stdout lines, TUI had no clock) |
| TUI batch | all | each queued job starts with the phase of its path (the bridge sets its phase in `start`) | as above | as above | fixed for the bridge |

UAC points and the notice (`progress::ELEVATION_NOTICE` = "Aguardando autorização para executar como administrador"):
- `bridge-worker` start (bridge jobs and HKLM removals): `ElevatedBridge::ensure_started` → `Transport::start_with_notice`; `SystemTransport` calls the notice after the admin-folder check, right before `ShellExecuteEx` (`elevate_win::start_noticed`); the CLI's `NoticeTransport` forwards it. Folder refused ⇒ no notice.
- `runas` per installer (Task 196): `Phase::WaitingElevation` before the launch (title starts with the constant).
- Self-elevating installer (`elevatesSelf`, or `elevationRequired` kept unelevated by a user scope): new `Phase::SelfElevating { name }`. Its title starts with the constant for at most `SELF_ELEVATION_WAIT` (2 min). After that the line reads "<name>: rodando o instalador (pode pedir autorização de administrador)". The switch lives only in `phase_text`, so the CLI and the TUI change together.
- HKLM key deletion by the helper (orphans): `ElevatedRunner::delete_arp_key_lines` passes on the notice and granted lines, and `orphans::remove_orphan_with` maps them to WaitingElevation → Removendo.
- Refused: `progress::ELEVATION_REFUSED` = "Permissão de administrador recusada." (`codes::UAC_REFUSED`, `ElevatedError::Refused`, and `REFUSED_NOTE` starts with it).

## Mutation proofs

Pasted from `scripts/mutate.ps1` (anchors of more than one line show only their first line):
```
M1  native install phase before lock (every_path):
[OK] mutação PEGA: '    phase(ctx, Phase::Preparing);
M2  helper notice before UAC:
[OK] mutação PEGA: '.start_with_notice(&mut || notice(ELEVATION_NOTICE))' -> '.start_with_notice(&mut || {})' fez o teste falhar (exit 101).
M3  self-elevating phase:
[OK] mutação PEGA: 'Verb::Open if asks_uac =>' -> 'Verb::Open if false =>' fez o teste falhar (exit 101).
M3b elevatesSelf recognised:
[OK] mutação PEGA: 'r.eq_ignore_ascii_case("elevatesSelf") || ' -> '' fez o teste falhar (exit 101).
M4  TUI bridge clock at start:
[OK] mutação PEGA: '        Action::Bridge(job) => Some(phase_progress(' -> '        Action::Bridge(job) if false => Some(phase_progress(' fez o teste falhar (exit 101).
M5  removal: granted -> uninstaller phase:
[OK] mutação PEGA: '            ELEVATION_GRANTED => Some(Phase::Uninstalling {' -> '            "x" => Some(Phase::Uninstalling {' fez o teste falhar (exit 101).
M5b removal: notice -> WaitingElevation:
[OK] mutação PEGA: '            ELEVATION_NOTICE => Some(Phase::WaitingElevation),' -> '' fez o teste falhar (exit 101).
M6  upgrade uninstallPrevious phase before lock:
[OK] mutação PEGA: '            (wctx.progress)(&ProgressEvent::Phase(Phase::Preparing));' -> '' fez o teste falhar (exit 101).
M6b upgrade removal phases:
[OK] mutação PEGA: 'let removed = run_uninstaller(uctx, &MsiQueue::of(wctx), previous, wctx.progress)' -> 'let removed = super::uninstall::uninstall_queued(uctx, Some(&MsiQueue::of(wctx)), previous)' fez o teste falhar (exit 101).
M7  CLI wrapper forwards the notice hook:
[OK] mutação PEGA: 'self.inner.start_with_notice(notice).inspect_err' -> '{ notice(); self.inner.start() }.inspect_err' fez o teste falhar (exit 101).
M8  bridge line: notice -> phase:
[OK] mutação PEGA: '        return (Some(Phase::WaitingElevation), false);' -> '        return (None, true);' fez o teste falhar (exit 101).
M8b bridge line: back to the tool phase:
[OK] mutação PEGA: '    let back = matches!(current, Some(Phase::WaitingElevation))' -> '    let back = matches!(current, Some(Phase::Verifying))' fez o teste falhar (exit 101).
M9  one constant in the title:
[OK] mutação PEGA: '            "{ELEVATION_NOTICE} (se o UAC' -> '            "Aguardando permissão (se o UAC' fez o teste falhar (exit 101).
M10 refused text:
[OK] mutação PEGA: '            ElevatedError::Refused => f.write_str(ELEVATION_REFUSED),' -> '            ElevatedError::Refused => f.write_str("sem permissão de administrador"),' fez o teste falhar (exit 101).
M11 CLI bridge first phase:
[OK] mutação PEGA: '    progress(&ProgressEvent::Phase(current.clone()));
M12 removal helper "granted" line:
[OK] mutação PEGA: '        on_line(ELEVATION_GRANTED);
M13 (addendum) shared Kindle labels:
[OK] mutação PEGA: 'pub const BACKUP_ITEM: &str = crate::winget_install::removal::BACKUP_THEN_REMOVE_LABEL;' -> 'pub const BACKUP_ITEM: &str = "Copiar pastas para backups antes";' fez o teste falhar (exit 101).
```
Review round (08a940c):
```
R1a key deletion passes lines:
[OK] mutação PEGA: 'match self.ask_elevated(&request, on_line)? {' -> 'match self.ask_elevated(&request, &mut |_| {})? {' fez o teste falhar (exit 101).
R1b orphan maps helper lines:
[OK] mutação PEGA: '                if let Some(p) = helper_line_phase(line, &removing) {' -> '                if let Some(p) = helper_line_phase("", &removing) {' fez o teste falhar (exit 101).
R1c CLI cleanup wires progress:
[OK] mutação PEGA: '        .map(|o| (o, flows.remove_orphan(o, &ts, &*progress)))' -> '        .map(|o| (o, flows.remove_orphan(o, &ts, &|_| {})))' fez o teste falhar (exit 101).
R2 TUI orphan phase before lock:
[OK] mutação PEGA: '            phase_of(env, Phase::Preparing);
R3a install hook phase:
[OK] mutação PEGA: '    // From PowerShell's start the hook runs to its end: its own phase says so (Task 201).
R3b installer.file phase:
[OK] mutação PEGA: '    (ctx.progress)(&ProgressEvent::Phase(Phase::RunningPackageInstaller {' -> '    (|_: &ProgressEvent| {})(&ProgressEvent::Phase(Phase::RunningPackageInstaller {' fez o teste falhar (exit 101).
R3c removal back to Removing:
[OK] mutação PEGA: '    let result = host.run(&prepared.job);
R3d uncancellable status:
[OK] mutação PEGA: '        | Phase::RunningScript { .. }
R3e TUI uncancellable:
[OK] mutação PEGA: '                        | crate::install::progress::Phase::RunningScript { .. }' -> '' fez o teste falhar (exit 101).
R4a CLI upgrade bridge wiring:
[OK] mutação PEGA: '        |on_line| flows.upgrade_bridge(&job, on_line),' -> '        |_| flows.upgrade_bridge(&job, &mut |line: &str| say(&format!("  {line}"))),' fez o teste falhar (exit 101).
R4b CLI upgrade bridge progress:
[OK] mutação PEGA: '    let result = super::output::bridge_progress(
R5 tool line to stdout:
[OK] mutação PEGA: '            output(&format!("  {line}"));' -> '            let _ = line;' fez o teste falhar (exit 101).
R6 self-elevating switch:
[OK] mutação PEGA: '        Phase::SelfElevating { name } if elapsed >= SELF_ELEVATION_WAIT => {' -> '        Phase::SelfElevating { name } if false => {' fez o teste falhar (exit 101).
```
New tests: `wi_cli::bridge_upgrade_progress_and_stdout`, `wi_cli::cleanup_hklm_orphan_shows_uac_notice`, `wi_orphans::hklm_orphan_shows_uac_notice_phase`, `install_scripts::scripts_and_installer_show_uncancellable_phases`, `uninstall_scripts::removal_scripts_show_uncancellable_phases`, `install_progress::package_scripts_and_installers_are_uncancellable_in_cli_and_tui`, `install_progress::self_elevating_notice_gives_way_after_two_minutes`. `every_path_emits_first_phase_immediately` gained a TUI orphan row, and `elevation_notice_before_every_uac` gained the key-deletion case.

Review round 2:
```
S1a registry-only maps helper lines:
[OK] mutação PEGA: '                    if let Some(p) = helper_line_phase(line, &removing) {' -> '                    if let Some(p) = helper_line_phase("", &removing) {' fez o teste falhar (exit 101).
S1b removal hands progress to the remover:
[OK] mutação PEGA: '            registry_only(s.progress)' -> '            registry_only(&|_| {})' fez o teste falhar (exit 101).
S2a install bridge progress wiring:
[OK] mutação PEGA: '    let result = super::output::bridge_progress(progress, &|line: &str| ui.output(line), tool, run);' -> '    let result = run(&mut |line: &str| anstream::println!("  {line}"));' fez o teste falhar (exit 101).
S2b install bridge tool lines on stdout:
[OK] mutação PEGA: 'super::output::bridge_progress(progress, &|line: &str| ui.output(line), tool, run);' -> 'super::output::bridge_progress(progress, &|line: &str| ui.notice(line), tool, run);' fez o teste falhar (exit 101).
```
New tests: `wi_uninstall_flow::registry_only_hklm_shows_uac_notice_phase`, `wi_cli::cli_registry_only_hklm_shows_uac_notice`, `wi_cli::bridge_install_progress_and_stdout`. `install_bridge` now ends in `winget_cmd::run_install_bridge`, which is the same code the fake reaches. `RecordRemover` takes the progress, as `Uninstaller` does.

Checks beyond the steps: (1) coordination with Task 200: Task 200 had committed before my commit, so the checkout held only my hunks and no `git apply --cached` was needed. There is no mutation proof for this. (2) No test opens a real UAC: fake `Transport`/`Launcher`, and the `runas` path uses `TraceLauncher`. There is no mutation proof for this either. (3) The gate passed (above).

## Decisions
- New phases are `SelfElevating { name }` and `Bridge { tool }`. Both say "não pode ser cancelado". In the TUI, `Bridge` does not set `installer_running`, so the quit behaviour is unchanged.
- The helper reports the UAC through two constant lines on the existing `on_line` channels: `ELEVATION_NOTICE` before the prompt and `ELEVATION_GRANTED` after it. `progress::bridge_line` maps them the same way in the CLI and the TUI, and `removal::run_uninstaller` maps them to WaitingElevation → Uninstalling. No public signature changed. `Transport` gained `start_with_notice`, which by default calls the notice and then `start`.
- The bridge tool's own lines stay on stdout (review decision 5). There is a new `Ui::output` (default `println!`); `TtyUi` routes it through `LineProgress::stdout_line`, which erases the stderr progress line first so the two never share a row. Phases and notices stay on stderr.
- `cleanup --orphans` now prints its per-item results after all the removals, once the progress line has ended.
- Addendum: the four Kindle labels are now in `winget_install::removal` (`BACKUP_THEN_REMOVE_LABEL`, `REMOVE_LABEL`, `REGISTRY_ONLY_LABEL`, `CANCEL_LABEL`). The TUI uses the two that are its options (`BACKUP_ITEM`, `REGISTRY_ONLY_ITEM`). The TUI checkbox therefore now reads "Backup das pastas e remover" instead of "Copiar pastas para backups antes". The test is in `wi_cli::cli_kindle_backup_then_remove`.
- `.join` with external data written in this task: none.

## Concerns
- Files outside the **Files** list: `src/bridge/elevated.rs` and `src/bridge/helper_folder.rs`. The helper's UAC moment lives in `ensure_started`, and the CLI's wrapper must forward the hook. Also `src/cli/winget_cmd.rs` (progress hunks and the shared labels, as the coordinator asked) and `src/winget_install/engine.rs`.
- Existing tests had their assertions changed, because the new behaviour requires it. `elevated_worker` (5 tests) now expects the notice and granted lines. `wi_install_flow::consumers_show_new_phases` and `wi_tui::status_bar_waiting_uac_text` now check the new title constant, and `wi_tui` checks the shared labels. `elevated_install::folder_refusal_is_not_a_refused_uac` got a fake override of `start_with_notice` for "refused before any prompt"; its assertions are unchanged.
- Removal of an HKLM key with registry-only, or through a helper that already runs, still shows WaitingElevation briefly: `removal_phase` is unchanged. The uninstaller path moves on once `ELEVATION_GRANTED` arrives.
- Self-elevating installers: review item 6 asks to keep the notice "until the first output or progress sign". An unelevated `Verb::Open` installer reports nothing to wingetrs before it exits, so only the 2-minute limit applies.
- `install_bridge` builds the real `ElevatedBridge` and `run_tool` inline before calling `run_install_bridge`. That one call (helper, job, msi lock) is still only covered by the type checker.
- Scoop `installer.file` exes that self-elevate get no notice: Scoop manifests do not declare it.
- Live UAC check not done (not measured).
- Harness: Python's text-mode `open().write()` on Windows rewrote LF files as CRLF while I edited, and I fixed it by rewriting the bytes. Consider adding to `docs/ARMADILHAS.md`: "use `newline=''` when editing with Python".
