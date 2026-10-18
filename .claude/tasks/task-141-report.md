Status: DONE_WITH_CONCERNS

Commits:
d2694e4 feat(install): typed progress phases with live signs in the CLI and the TUI

Gate:
```
[OK]   rules 4s
[OK]   rules-selftest 11s
[OK]   fmt 1s
[OK]   clippy 23s
[OK]   test (1073 passed, 27 ignored) 55s
[OK]   gate verde
```

Mutation proofs:
1. Executor para de emitir a fase (download em cache):
   `pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'if let Some(phase) = step_phase(step, plan) {' -Replacement 'if let Some(phase) = None::<Phase> {' -TestCommand 'cargo test --quiet --test it install_progress::cached_install_emits_phases_in_order'`
   `[OK] mutação PEGA: 'if let Some(phase) = step_phase(step, plan) {' -> 'if let Some(phase) = None::<Phase> {' fez o teste falhar (exit 101).`
2. Log decodificado sem alinhar a pares:
   `pwsh -File scripts/mutate.ps1 -Path src/install/msiexec_log.rs -Anchor '    start += start % 2;' -Replacement '    start += 0;' -TestCommand 'cargo test --quiet --test it install_progress::msiexec_log_tail_reads_last_file'`
   `[OK] mutação PEGA: '    start += start % 2;' -> '    start += 0;' fez o teste falhar (exit 101).`
3. Aviso de 60 s removido:
   `pwsh -File scripts/mutate.ps1 -Path src/cli/output.rs -Anchor 'if idle >= INSTALLER_IDLE {' -Replacement 'if false {' -TestCommand 'cargo test --quiet --test it install_progress::installer_idle_warning_after_60s'`
   `[OK] mutação PEGA: 'if idle >= INSTALLER_IDLE {' -> 'if false {' fez o teste falhar (exit 101).`
4. Instalação e remoção reais (`target/release/wingetrs.exe`, stderr redirecionado, ou seja, sem TTY; o download já estava em cache):
   ```
   $ wingetrs install main/fastfetch --yes      (exit 0)
   --- stdout
   Instalar fastfetch 2.70.0  main · 64bit
     1. Comandos: fastfetch, flashfetch
     2. PATH do usuário  C:\Users\jonyd\AppData\Local\wingetrs\shims
   Instalado: fastfetch 2.70.0
   Abra um terminal novo para usar: fastfetch, flashfetch
   --- stderr
   Preparando…
   Extraindo fastfetch-windows-amd64.7z (7z)…
   Colocando os arquivos…
   Criando comandos…
   Registrando…
   $ wingetrs uninstall fastfetch --yes         (exit 0; stderr vazio)
   Desinstalar fastfetch 2.70.0
     1. Apaga a pasta do app
     2. Comandos: fastfetch, flashfetch
     3. PATH do usuário  remove C:\Users\jonyd\AppData\Local\wingetrs\shims
   Desinstalado: fastfetch 2.70.0
   ```
   Depois disso, `wingetrs list | grep -ci fastfetch` deu 0. A prova em TTY com o `rust` fica com o orquestrador.
5. Gate: verde (acima).

Decisions:
- Os tipos ficam num módulo novo, `src/install/progress.rs` (`ProgressEvent`, `Phase`, `phase_text`/`phase_title`/`clock`, `engine_of`), compartilhado pelo CLI e pela TUI. É a opção "install::progress" que o brief permite. O `install/mod.rs` também foi editado (é obrigatório para o `msiexec_log.rs`). `Progress` continua em `download.rs` e agora é `&dyn Fn(&ProgressEvent)`.
- Contador do Windows Installer: a cada ~1 s lê só os últimos 8 KiB e conta as linhas `File:` que começam depois do tamanho visto na leitura anterior. Se foi gravado mais que a janela desde então, a contagem é extrapolada pela densidade da janela, marcada com `estimated` e mostrada como `~N arquivos`. O arquivo é aberto com `File::open`, cujo compartilhamento padrão no Windows é leitura+escrita+exclusão. Erro de leitura (inclusive log ainda inexistente) ⇒ só fase e relógio.
- Mantive o texto da 139, `Windows Installer (msiexec /a) · mm:ss`, com `· não pode ser cancelado` enquanto o log ainda não foi lido. Assim os testes da 139 passam sem trocar asserções. Depois vêm `· N arquivos · <arquivo>`, com o caminho cortado à esquerda para a linha caber em 100 colunas, e a linha toda ainda é cortada na largura do terminal menos 1 (`crossterm::terminal::size`) para o `\r` não quebrar.
- O `ops::apply_with_targets` emite `Preparing` antes do lock, para haver linha desde o primeiro instante. O executor emite a fase ao começar cada passo, sem repetir fase igual (`Download`/`RemoveOldVersion` não mudam a fase). Na TUI, `Phase::Installer` continua virando `Msg::InstallerRunning`, e as outras fases viram o `Msg::InstallPhase` novo, nunca segurado pelo throttle.

Concerns:
- O TDD não foi seguido à risca: escrevi a implementação antes dos testes, e os 9 testes passaram na primeira execução. A prova de que eles pegam o defeito certo são as 3 mutações acima, não a falha vista antes da implementação.
- A fase `Installer` na TUI usa `log: PathBuf::new()`, porque `Msg::InstallerRunning` continua sem dados e a TUI não lê o log (o brief só pede fase e relógio).
- O aviso de "sem atividade" é mais longo que 100 colunas (o texto vem do brief). Num terminal estreito ele é cortado pela largura, não quebrado.
- A animação em TTY e o aviso fixo de 30 s não foram vistos num terminal real, porque o subagente não tem TTY. O texto e o momento foram testados pelas funções puras (`installer_notice_is_shown_once_after_30s`, `phase_line_has_spinner_and_clock`).
- Fora da lista Files: `src/install/mod.rs` e `src/install/progress.rs`, como explicado em Decisions.

## Review fix (563f07e fix(install): review of task 141)

Gate: `[OK]   test (1077 passed, 27 ignored) 82s` / `[OK]   gate verde`

Mutation proofs (all `cargo test --quiet --test it install_progress::<test>`):
- `[OK] mutação PEGA: 'started(&layout.msiexec_log(app, u32::from(attempt)));' -> 'started(&layout.msiexec_log(app, 1));' fez o teste falhar (exit 101).` (comando do revisor, `install_progress::`)
- `[OK] mutação PEGA: '    (ctx.progress)(&ProgressEvent::Phase(Phase::Removing {' -> '    (|_: &ProgressEvent| {})(&ProgressEvent::Phase(Phase::Removing {' fez o teste falhar (exit 101).` (uninstall_emits_removing_then_scanning)
- `[OK] mutação PEGA: '    (ctx.progress)(&ProgressEvent::Phase(Phase::Scanning));' -> '' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: '            | msi_queue::Status::Retrying { .. } => phase(Phase::WaitingInstaller),' -> '            | msi_queue::Status::Retrying { .. } => {}' fez o teste falhar (exit 101).` (installer_retry_reports_each_attempt_log)
- `[OK] mutação PEGA: '    phase(Phase::Placing);' -> '' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: '    start = start.max(2).min(size);' -> '    start = start.max(2);' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: '(code == 1603 && tail.contains("Error 1304"))' -> '(tail.contains("Error 1304"))' fez o teste falhar (exit 101).`
- `[OK] mutação PEGA: '            message.push_str(hint);' -> '' fez o teste falhar (exit 101).` (installer_1603_error_names_the_path_limit_only_for_1304)
- `[OK] mutação PEGA: 'Phase::Installer { .. } => format!("{text} · não pode ser cancelado"),' -> 'Phase::Installer { .. } => text,' fez o teste falhar (exit 101).` (tui_status_shows_phase_and_clock)

Decisions:
- Fases novas: `Removing { app }` (emitida no `ops::uninstall` antes do lock), `Scanning` (antes de `scan_residues`) e `WaitingInstaller` (para `WaitingLock`, `WaitingInstaller` e `Retrying`). Depois que o msiexec termina, antes do `import_admin_tree`, é emitida `Placing`, sem fase nova: o passo `Promote` seguinte é deduplicado e o relógio continua. O CLI agora chama `end_progress` depois do `ops::uninstall`.
- O texto `phase_status` (fase + relógio + `· não pode ser cancelado` no Installer) é compartilhado pela TUI e pela linha do CLI antes de o log ser lido.
- Dica do 1304: lê só os últimos 64 KiB (`ERROR_WINDOW`) do log da última tentativa e só vale para o código 1603. Os outros códigos ficam iguais.
- O teste do retry 1618 espera de verdade o primeiro atraso da política (5 s), porque a `POLICY` não é injetável em `extract_with_msiexec`.
