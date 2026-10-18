Status: DONE_WITH_CONCERNS

Commits:
8f27cd2 fix(bridge): task 190 review (junction-proof orphan DACL, uninstaller wait phase, testable winget entry, handle list)
745d668 fix(examples): elevated_arp_probe follows the run_uninstaller on_line parameter
a7efb33 fix(bridge): uninstaller job wait has a deadline; winget env test reads, never sets, the process env
79e84b3 fix(bridge): phase review of the elevated side (log quoting, winget env, orphan DACL, reparse, job wait)

Gate (`pwsh -File scripts/verify.ps1`; uma execução por commit, as duas falharam só em arquivos de outros agentes):
Após 79e84b3:
```
[OK]   rules 10s
[OK]   rules-selftest 28s
[FAIL] fmt exit 1
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\tests\it\install_shim.rs:431:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\tests\it\install_shim.rs:450:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\tests\it\install_shim.rs:460:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\tests\it\script_scan.rs:701:
  (log completo: target/verify.log)
```
Após as mudanças do a7efb33 (antes do commit):
```
[OK]   rules 10s
[OK]   rules-selftest 27s
[FAIL] fmt exit 1
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\winget_install\mod.rs:23:
  Diff in \\?\C:\Users\jonyd\Projetos\wingetrs\src\winget_install\mod.rs:30:
  (log completo: target/verify.log)
```
Meus arquivos, provados à parte:
- `rustfmt --edition 2024 --check` em todos os meus arquivos .rs: sem saída (ok).
- `cargo clippy --all-targets` filtrado pelos meus caminhos: nenhum aviso.
- `pwsh -File scripts/check_rules.ps1`: rules=0.
- `cargo test --quiet --test it elevated_` -> `test result: ok. 94 passed; 0 failed; 3 ignored` (após a7efb33).
- `cargo test --quiet --test it wi_switches` -> `test result: ok. 13 passed; 0 failed`.
- `cargo test --quiet --test it spawn_env` -> `test result: ok. 5 passed`.
- `cargo test --quiet --lib bridge` -> `test result: ok. 12 passed; 0 failed; 2 ignored`.
- `cargo test --quiet --test it wi_` (após 79e84b3): 271 ok e 1 falha em `wi_upgrade::native_agrees_with_winget_applicability`. É arquivo de outro agente, que estava em edição; rodado sozinho logo depois, passou.

Após 8f27cd2 (gate não rodado de novo; o checkout tinha `tests/it/wi_fetch.rs` de outro agente quebrado por ~10 min): `cargo test --quiet --test it elevated_` -> `test result: ok. 95 passed; 0 failed; 3 ignored`; `--test it wi_` -> `test result: ok. 277 passed; 0 failed; 4 ignored`; `--lib` -> `test result: ok. 63 passed; 0 failed; 2 ignored`; clippy `--all-targets` sem aviso nos meus caminhos e sem `error`; check_rules rules=0; rustfmt nos meus arquivos.

Mutation proofs:
- Revisão (8f27cd2):
  item 1, junção acima da pasta existente: `[OK] mutação PEGA: '    let _above = super::staging_win::hold_plain_folders(path).ok()?;' -> '    let _above = ();' fez o teste falhar (exit 101).` (elevated_uninstall::job::orphan_through_junction_in_middle_is_not_orphan)
  item 2, fase de espera: `[OK] mutação PEGA: '            progress(&ProgressEvent::Phase(waiting_uninstaller_phase(&plan.name)));' -> '' fez o teste falhar (exit 101).` (wi_uninstall_flow::helper_wait_line_shows_the_waiting_phase)
  item 3, winget por `run_tool`: `[OK] mutação PEGA: '    super::staging_win::run_tool(exe, args, dir, root, sddl, sys, on_line)' -> '    let _ = (root, sddl, sys); super::spawn_win::run_in(exe, args, dir, on_line)' fez o teste falhar (exit 101).` (elevated_install::win::winget_env_is_built_from_scratch, agora chamando `elevate_win::run_winget_at`)
- (a) Tirei o `env_clear()` do caminho do winget. Ele fica em `spawn_win::run_in_env`, chamado por `staging_win::run_tool`, que é chamado por `elevate_win::run_winget`. Rodado de novo com o teste reescrito em a7efb33:
  `mutate.ps1 -Path src/bridge/spawn_win.rs -Anchor '        .env_clear()\n        .envs(env.iter().map(|(k, v)| (k, v)));' -Replacement '        .envs(env.iter().map(|(k, v)| (k, v)));' -TestCommand 'cargo test --quiet --test it elevated_install::win::winget_env_is_built_from_scratch'`
  `[OK] mutação PEGA: '        .env_clear()`
- (b) `<LOGPATH>` solto passa a ser substituído sem aspas. Âncora `-AnchorFile t190-b-anchor.txt` (as três linhas `out.push('"'); out.push_str(&p); out.push('"');`), troca `-ReplacementFile` por `out.push_str(&p);`:
  com `wi_switches::bare_log_placeholder_refused_or_quoted`: `[OK] mutação PEGA: '            out.push('"');`
  com `elevated_install::win::log_placeholder_with_space_stays_one_argument`: `[OK] mutação PEGA: '            out.push('"');`
- (c) `still_orphan` sem a checagem de DACL:
  `-Anchor '        Some(sddl) if sddl_admin_only(&sddl, AclLevel::Parent) => true,' -Replacement '        Some(_) => true,' -TestCommand '... elevated_uninstall::orphan_in_user_folder_is_not_orphan'`
  `[OK] mutação PEGA: '        Some(sddl) if sddl_admin_only(&sddl, AclLevel::Parent) => true,' -> '        Some(_) => true,' fez o teste falhar (exit 101).`
- Prazo do job (a7efb33):
  `[OK] mutação PEGA: '        if start.elapsed() >= linger {' -> '        if false {' fez o teste falhar (exit 101).` (job_deadline_leaves_the_copy_and_keeps_the_folder)
  `[OK] mutação PEGA: '        let kept = run.keep();' -> '        let kept = run.path().to_path_buf(); drop(run);' fez o teste falhar (exit 101).` (mesmo teste)
  `[OK] mutação PEGA: '            if broken.is_none()` -> `if false` (o worker nunca repassa as linhas do desinstalador; run_uninstaller_relays_progress_lines)
  Uma tentativa inválida que sobreviveu: trocar `broken = Some(e)` por nada. Ela não testa nada, porque a escrita num `Vec` nunca falha; substituí pela mutação acima.
- Extras (79e84b3):
  `[OK] mutação PEGA: '    let _folders = hold_plain_folders(src)?;' -> '    let _folders = ();' fez o teste falhar (exit 101).` (source_with_junction_in_middle_refused)
  `[OK] mutação PEGA: '    if info.nNumberOfLinks > 1 {' -> '    if info.nNumberOfLinks > 99 {' fez o teste falhar (exit 101).` (source_hard_link_refused)
  `[OK] mutação PEGA: '                t == "/S" || lower == "/norestart"' -> '                t == "/S"' fez o teste falhar (exit 101).` (adendo a)
  `[OK] mutação PEGA: '                    || BURN_PROPERTIES.contains(&lower.as_str())' -> '                    || false' fez o teste falhar (exit 101).` (adendo b)
  `[OK] mutação PEGA: '        args = dedupe_tokens(&args);' -> '        args = args.clone();' fez o teste falhar (exit 101).` (adendo c)
  `[OK] mutação PEGA: ''"', '<', '>']' -> ''"']' fez o teste falhar (exit 101).` (`<LOGPATH>` dentro de outro texto)

Decisions:
- Revisão 1: `arp_ops_win::folder_sddl` segura todas as pastas desde a raiz do volume com `staging_win::hold_plain_folders` (a mesma regra do `open_source`). Qualquer reparse point no caminho dá `None`, e a entrada não é órfã.
- Revisão 2: o `progress.rs` estava limpo, mas não precisou mudar. A linha `WAITING_UNINSTALLER` (agora em `worker.rs`) chega ao cliente pelo novo `ElevatedRunner::run_uninstaller_lines`, cujo padrão não emite linha, e daí pelo novo `UninstallCtx::helper_line`. Ao receber essa linha, `removal::run_uninstaller` passa para `Phase::Removing { app: "<nome>: aguardando o desinstalador terminar" }`, que aparece como "Removendo <nome>: aguardando o desinstalador terminar". Por isso adicionei `helper_line: &|_| {}` nas 9 construções de `UninstallCtx` (`src/cli/run_win.rs`, `src/tui/run.rs`, `tests/it/wi_cli.rs`, `wi_uninstall_flow.rs`, `wi_upgrade.rs`, todos limpos no git antes da mudança).
- Revisão 3: `run_winget` virou busca do exe mais `run_winget_at(exe, args, root, sddl, sys, on_line)` (pub), e o teste do ambiente chama `run_winget_at`. Ficam sem teste só as três linhas de busca: `winget_exe`, `SystemEnv::query` e `staging_root`.
- Revisão 4: `run_in_job` usa `STARTUPINFOEXW` com `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` só com o handle do NUL (`HandleList` com `Drop` chamando `DeleteProcThreadAttributeList`). Os testes de job continuam passando, mas não há teste de que outro handle herdável deixou de passar: não medido.
- Item 1: `fill_log` põe as aspas; um `<LOGPATH>` que já está entre `"` não ganha outras. Isso vale para o lado elevado e para o não elevado. Além disso, `plain_value` recusa `<`/`>`.
- Item 5 (revisto em a7efb33 conforme o orquestrador):
  - O processo é criado suspenso com `CreateProcessW`, entra no job e só então é retomado.
  - O programa iniciado é esperado sem limite. Depois que ele sai, o resto do job tem até `UNINSTALL_LINGER` (10 min). O que sobra continua rodando (nada é morto) e o `RunDir` fica com `RunDir::keep` para o prune de 24 h.
  - O código de saída é o do programa iniciado.
  - O worker manda `Reply::Line` com "aguardando o desinstalador terminar" e, no prazo estourado, a linha "a pasta de execução fica para a limpeza de 24 h". `WorkerOps::run_uninstaller` ganhou `on_line` para isso.
  - Feature nova `Win32_System_JobObjects`; o `Cargo.lock` não mudou.
- Fase da UI: a remoção elevada já fica na fase existente `Phase::WaitingElevation` ("Aguardando o UAC ou o instalador elevado..."), com relógio, durante toda a chamada. Reusei essa fase. As linhas do auxiliar vão para o log via `tracing` em `src/bridge/elevated.rs`, que está fora da lista Files (mudança de 3 linhas). Mostrá-las na tela exigiria passar `on_line` por `ElevatedRunner::run_uninstaller` e por `winget_install/uninstall.rs`, arquivo que outro agente está editando; não fiz.
- Teste do ambiente do winget (pedido 2): a interpretação literal do pedido não resolve. O `env_clear` só se observa num processo filho; um mapa injetado em `installer_env` não exercitaria a herança, e a mutação (a) sobreviveria. O ambiente do filho já vem só de `installer_env(sys, tmp)`, com `sys` injetável. Por isso o teste agora só **lê** o ambiente do processo: confere que `CARGO_PKG_NAME` (o cargo define para o binário de teste) e `USERNAME` existem aqui e não chegam ao filho, e que o `TEMP` do filho difere do nosso e fica em `root\<32 hex>\tmp`. Nenhum `set_var`/`remove_var` restou.
- Item 3: a raiz do volume conta como existente; a checagem é `sddl_admin_only(.., Parent)` via o novo `sddl_of` do `DeleteCheck`, que no worker real é `arp_ops_win::folder_sddl`.
- O brief cita `split_command`, mas a função de mesmo nome do código só separa o programa do resto; o teste usa um splitter local com as regras de `argv` do C runtime.

Concerns:
- O programa iniciado continua sendo esperado sem prazo; só o resto do job tem os 10 min. Um desinstalador principal que nunca sai segura o worker, como já acontecia antes.
- `job_deadline_leaves_the_copy_and_keeps_the_folder` tem um teto de segurança de 120 s marcado `clock-ok`. Ele só serve para falhar em vez de travar e não é limiar de desempenho.
- Adendo (b): só `InstallAllUsers=1` (literal exato, Burn) foi aberto. É a única propriedade que o dry-run do Python 3.12 `--scope machine` recusou; `PrependPath` continua recusado (há teste). Adendo (a): `/norestart` só faltava no nullsoft. Adendo (c): a deduplicação vale só para Inno.
- `.join` com dado externo escrito nesta tarefa: nenhum.
- Prova ao vivo (worker elevado real com winget ou desinstalador no job): não medido.
