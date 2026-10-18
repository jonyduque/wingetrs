# Task 140: relatório

Status: DONE_WITH_CONCERNS

Commits:
```
704058f fix(cli): Ctrl+C cancels an install before msiexec starts and waits once it runs
```

Gate:
```
[OK]   rules 4s
[OK]   rules-selftest 13s
[OK]   fmt 2s
[OK]   clippy 11s
[OK]   test (1084 passed, 27 ignored) 44s
[OK]   gate verde
```

Mutation proofs:
1. Cancelamento valendo depois do `started` (teste 2):
   `pwsh -File scripts/mutate.ps1 -Path src/cli/install_cmd.rs -Anchor 'if !gate.running() {' -Replacement 'if true {' -TestCommand 'cargo test --quiet --test it cli_cancel::cancel_during_installer_waits'`
   `[OK] mutação PEGA: 'if !gate.running() {' -> 'if true {' fez o teste falhar (exit 101).` (`cli_cancel::cancel_during_installer_waits --- FAILED`, 10 s do detector de travamento do msiexec falso)
2. Não apagar o `.msiexec` velho (teste 3):
   `pwsh -File scripts/mutate.ps1 -Path src/install/executor.rs -Anchor 'remove_dead_run_leftovers(&layout.app_dir(app));' -Replacement '();' -TestCommand 'cargo test --quiet --test it cli_cancel::stale_msiexec_work_dir_is_removed'`
   `[OK] mutação PEGA: 'remove_dead_run_leftovers(&layout.app_dir(app));' -> '();' fez o teste falhar (exit 101).` (`assertion failed: !app_dir.join(".staging-1.0.msiexec").exists()`)
3. Prova manual (Ctrl+C no `install main/rust --allow-msiexec`): fica com o orquestrador e o usuário. Não rodei nenhuma instalação ao vivo. A sobra `.staging-1.99.1.msiexec` do rust ficou intacta.
4. Gate: acima.

Mutações extras (regras que eu acrescentei):
- `'            if cancelled() {' -> '            if false {'` (executor; não lançar o msiexec depois do cancelamento) → `[OK] mutação PEGA` (`cli_cancel::cancel_while_queued_never_launches_msiexec`, `assertion failed: !QUEUED_LAUNCHED...`)
- `'            let _ = fs::remove_dir(app_dir);' -> '            let _ = app_dir;'` (AwaitWorker apaga a pasta do app vazia) → `[OK] mutação PEGA` (`cli_cancel::cancel_during_extraction_waits_for_the_worker_then_removes_staging`)
- `'if only_dead_run_leftovers(dir) {' -> 'if false {'` (ops.rs, repair) → `[OK] mutação PEGA` (`install_ops::repair_removes_app_folders_holding_only_leftovers_of_a_killed_install`)
- `'    name.starts_with(".staging-") || name.starts_with(extract::TEMP_PREFIX)' -> '    name.starts_with(".staging-")'` → `[OK] mutação PEGA` (`stale_msiexec_work_dir_is_removed`, `assertion failed: !temp.exists()`)

Decisions:
- `install_cmd::until_interrupted(op, &mut impl Interrupts, &InstallerGate, refused)`: um `select!` com `biased`. O `InstallerGate` liga no primeiro `Phase::Installer` que o callback de progresso vê (em `run_win.rs`, `gate.observe` antes de `line.update`). Em produção, `Interrupts` é `tokio::signal::windows::ctrl_c()`, criado só no `hold()` (o início da mudança no disco) e solto no `release()`. Assim, Ctrl+C no prompt "Continuar?" ou no picker continua encerrando o processo, como antes.
- Cancelado ⇒ `job.end outcome=cancelado` (detalhe `Ctrl+C`), `Cancelado.` no stdout e o erro tipado `Cancelled`. O `main.rs` (fora da lista Files, 4 linhas) troca esse erro pelo código 130, sem a linha `erro:`. O aviso `o Windows Installer não pode ser cancelado; esperando terminar` sai uma vez, numa linha acima da linha de progresso (`Ui::notice` → `LineProgress::notice`; fora de TTY é uma linha simples no stderr).
- Uninstall: até o prompt, Ctrl+C mata o processo e nada foi removido. Do `JobLog::start` até o fim de `ops::uninstall` (síncrono), o Ctrl+C fica segurado: a remoção começada nunca é cortada no meio e não há aviso na tela. Depois disso o Ctrl+C é solto.
- Item 2: o código **não** apagava `.staging-*.msiexec` numa instalação sem msiexec (só o `extract_with_msiexec` apagava o próprio) nem `.wingetrs-extract-*`. Agora o passo Extract varre `apps\<app>` (`.staging-*` e `.wingetrs-extract-*`, constante nova `extract::TEMP_PREFIX`) sem parar em erro: avisa no log e segue. O `repair` **não** fazia isso (pasta sem `install.json` virava "Pulados"). Agora ele apaga a pasta sem registro que só tem essas sobras (ou está vazia) e lista em `RepairReport.cleaned` (texto "Pastas apagadas (só sobras de instalação interrompida)" em `output.rs`).
- Achado ao implementar: soltar o future durante `WaitingInstaller`/`WaitingLock` fazia o `AwaitWorker::drop` esperar o worker, que **lançaria o msiexec depois do cancelamento** e seguraria o Ctrl+C por ~7 min. Agora o drop liga uma flag `cancel`: o `busy` para de esperar e o `launch` recusa (`extract_with_msiexec_until`; a assinatura pública de `extract_with_msiexec` não mudou). O drop também apaga a pasta do app se ela ficou vazia.

Concerns:
- Item 3, fechar o console (`CTRL_CLOSE_EVENT`): não há listener de close. O handler do tokio devolve FALSE, o Windows encerra o processo (no máximo 5 s) e nada do Rust roda (sem `job.end`, que fica sem fim; `install.lock`/`msi.lock` soltos pelo SO). O cliente `msiexec` foi criado com `CREATE_NO_WINDOW` e fora de job object, então não recebe o evento e o serviço termina a transação. `apps\<app>\.staging-<v>.msiexec` fica para trás: a próxima instalação do app o varre, e o `repair` apaga a pasta se não houver mais nada. Se esse msiexec órfão ainda estiver gravando quando a próxima instalação começar, a varredura falha nos arquivos em uso e só avisa no log. Já o `extract_with_msiexec` daquela instalação continua apagando o próprio `.msiexec` de forma estrita, antes do `msi.lock`, como antes da tarefa.
- Corrida de microssegundos: o `Phase::Installer` chega pelo canal do executor. Um Ctrl+C entre o worker emitir a fase e o loop processá-la é tratado como cancelável. Se o worker já passou da checagem `cancelled()`, o drop espera o msiexec até o fim sem mostrar o aviso. Não medido; a janela é a de uma volta do `select!`.
- A espera em `WaitingLock` (`UpdateLock::acquire_timeout(.., Duration::MAX)` em `msi_queue.rs`, fora da lista) não é interrompível. Um Ctrl+C nela só termina quando o `msi.lock` fica livre, mas aí não lança nada. Na prática o `install.lock` já serializa quem chegaria ali.
- Cancelar o lote no 2º plano deixa o 1º (dependência) instalado, sem o broadcast de ambiente do fim do `apply`. É o mesmo efeito de um `PartlyApplied`, mas sem a mensagem.
- Hold/release do uninstall e a ligação com o `CtrlC` real não têm teste automático (precisariam de sinal real); ficam para a prova manual.
- Arquivos fora da lista Files: `src/main.rs` (código 130), `src/cli/output.rs` (`repair_text` com `cleaned`), `src/install/extract.rs` (`TEMP_PREFIX`), `tests/it/install_ops.rs` (teste do repair, que precisa do `World` com `OpsCtx` de lá).

## Revisão (commit 1bb219b, fix(cli): review of task 140)

Gate: não ficou verde, e não foi por esta tarefa. Outro agente (Task 142) deixou arquivos não commitados (`src/install/classify_2c.rs`, `tests/it/classify_2c.rs`, `src/install/mod.rs`), e o `verify.ps1` falhou no `[FAIL] fmt exit 1  Diff in ...src\install\classify_2c.rs:2:`. O clippy dos meus arquivos está limpo: o único erro restante é `tests\it\classify_2c.rs:7:5: used expect()`, do outro agente. `cargo test --quiet` (todos os alvos): `1005 passed; 26 ignored`, `60 passed; 1 ignored`, `30 passed`, sem falhas. O gate precisa ser rodado de novo depois que a 142 commitar.

1. O teste da fila agora tem prazo, mas não com `tokio::time::timeout`. Esse timeout não dispara: a instalação solta espera o worker dentro do próprio poll (`block_in_place`) e trava a thread. No lugar dele, `always_busy` funciona como detector de travamento: chamado de novo 5 s depois da primeira vez (2,5x o poll de 2 s), ele marca `WAITED_ON` e libera a fila.
   `-Anchor '&mut || !gate.cancelled() && (m.busy)(),' -Replacement '&mut || (m.busy)(),' -TestCommand 'cargo test --quiet --test it cli_cancel::'` → `[OK] mutação PEGA` (`cancel_while_queued_never_launches_msiexec --- FAILED`). A âncora pedida, `!cancelled()`, virou `!gate.cancelled()`.
2. `executor::LaunchGate(Arc<AtomicU8>)` ficou em `OpsCtx.launch`/`ExecCtx.launch` e substitui o `InstallerGate` e a flag do `AwaitWorker`. O worker chama `try_launch` (0→1, ou já em 1 num relançamento 1618) imediatamente antes de `(m.launch)`. O CLI chama `try_cancel` (0→2) e o drop do `AwaitWorker` também. Teste das duas ordens: `launch_gate_first_come_wins_in_both_orders`. Mutações: `'if gate.try_cancel() {' -> 'if true {'` → `[OK] mutação PEGA` (`cancel_during_installer_waits`); `'            if !gate.try_launch() {' -> '            if false {'` → `[OK] mutação PEGA` (`cancel_while_queued...`).
3. `ctx.launch.next_plan()` (volta a 0) roda no início de cada `execute_install_with`. Teste: `ctrl_c_cancels_the_next_plan_after_msiexec_ran`. Mutação `'    ctx.launch.next_plan();' -> ''` → `[OK] mutação PEGA`.
4. `cancel_during_installer_waits` agora aperta Ctrl+C duas vezes depois que o msiexec falso começou e confere `ctrl_c.0 == 3` e `refusals == 1`. Mutação `'if !told {' -> 'if true {'` → `[OK] mutação PEGA`.
5. Install, upgrade e uninstall seguram o Ctrl+C desde antes do `JobLog::start` e só soltam depois do `finish_log`/`cancelled(log)`. O `until_interrupted` segura e não solta mais.
6. Nova função `install_cmd::exit_code(&anyhow::Error) -> u8`: 130 para `Cancelled` (inclusive sob `.context()`), 1 para o resto. O `main.rs` usa essa função e só imprime `erro:` quando o código não é 130. Teste: `exit_code_is_130_only_for_ctrl_c`. Mutação `'if e.is::<Cancelled>() {' -> 'if false {'` → `[OK] mutação PEGA`.

A corrida de microssegundos citada nas Concerns acima deixou de existir. Para o campo novo, foi preciso acrescentar `launch: Default::default()` nos `OpsCtx`/`ExecCtx` literais de `tests/it/{install_executor,install_ops,install_progress,tui_jobs}.rs` e do teste unitário em `install_cmd.rs`.
