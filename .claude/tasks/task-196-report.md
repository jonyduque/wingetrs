# Task 196: relatório

Status: DONE

Commits:
```
7b80c5e feat(winget-install): per-installer UAC for native machine-scope installers; worker no longer runs installers
30cbee2 fix(winget-install): task 196 review (no log for elevated installers, Windows Installer prompt text, folder refusal as its own error)
d01fd69 fix(winget-install): task 196 review 2 (elevated launch by the held file's final path, no reparse point in the chain, System32 as folder)
856be83 fix(winget-install): task 196, elevated installer runs from a fresh run folder
```
(`git log --format='%h %s' 646c331..HEAD -- src/winget_install/ops.rs src/bridge/worker.rs`)

Gate: o gate não ficou verde por causa de arquivos de outros agentes. Duas rodadas, depois do meu último código:
```
[OK]   rules 13s
[OK]   rules-selftest 34s
[OK]   fmt 5s
[FAIL] clippy exit 101
  error: could not compile `wingetrs` (lib) due to 1 previous error
```
→ `src\config.rs:115:9: error: unused variable: other` (Task 198, edição em andamento). Na rodada seguinte o clippy passou e o passo de teste parou só em snapshots de Settings da Task 198:
```
  snapshot assertion for 'settings_screen_snapshot' failed in line 1338
  snapshot assertion for 'settings_screen_scrolls_to_the_last_row_in_24_lines' failed in line 1347
  test result: FAILED. 1721 passed; 2 failed; 85 ignored; 0 measured; 0 filtered out; finished in 53.62s
```
Última rodada (depois do commit): `[FAIL] clippy exit 101` → `src\install\script_scan.rs:1637:17: unused variable: after`. É um mutante da Task 164 rodando agora (`scripts/leftovers.ps1` mostra o `mutate.ps1` dela).
O meu código, provado à parte:
`cargo test --quiet --test it -- elevated_install:: wi_install_flow:: wi_tui:: wi_launch:: elevated_uninstall:: wi_cli:: wi_orphans:: wi_uninstall_flow:: wi_upgrade:: elevated_worker:: tui_jobs:: tui_state::`
```
test result: ok. 475 passed; 0 failed; 5 ignored; 0 measured; 1326 filtered out; finished in 4.36s
```
Na rodada em que o clippy rodou (`--all-targets`) não houve aviso nos meus arquivos. Nos `.rs` commitados por trecho, `git show HEAD:<arq> | rustfmt --check` passou.

Mutation proofs:
- (a) voltar a rodar instalador no worker (o braço `RunInstaller` passa a executar `source` em vez da recusa fixa):
  `pwsh -File scripts/mutate.ps1 -Path src/bridge/worker.rs -AnchorFile <scratchpad>/t196-a1-anchor.txt -ReplacementFile <scratchpad>/t196-a1-repl.txt -TestCommand 'cargo test --quiet --test it elevated_install::worker_refuses_run_installer'`
  `[OK] mutação PEGA: '            Request::RunInstaller { .. } => {`
- (a') lado do cliente, voltar a não elevar pelo próprio UAC (`Verb::RunAs` → `Verb::Open` em `run_command`):
  `pwsh -File scripts/mutate.ps1 -Path src/winget_install/ops.rs -Anchor '            Verb::RunAs\n        }' -Replacement '            Verb::Open\n        }' -TestCommand 'cargo test --quiet --test it wi_install_flow::machine_installer_uses_runas_launcher'`
  `[OK] mutação PEGA: '            Verb::RunAs`
  Um mutante literal de "mandar `RunInstaller` ao `ElevatedRunner`" não compila: o método saiu do trait. Por isso o (a) foi provado no worker e no cliente separadamente.
- (b) soltar o handle antes do fim do processo (`file.release()` movido para antes de `run_command`):
  `pwsh -File scripts/mutate.ps1 -Path src/winget_install/ops.rs -AnchorFile <scratchpad>/t196-b-anchor.txt -ReplacementFile <scratchpad>/t196-b-repl.txt -TestCommand 'cargo test --quiet --test it wi_install_flow::verified_handle_held_until_exit'`
  `[OK] mutação PEGA: '            let ran = run_command(ctx, plan, cmd, &file);`
- Lista branca conferida de novo antes do UAC:
  `-Anchor '            elevation_allowed(plan, cmd, file)?;' -Replacement '            let _ = elevation_allowed(plan, cmd, file);' -TestCommand 'cargo test --quiet --test it wi_install_flow::'`
  `[OK] mutação PEGA: '            elevation_allowed(plan, cmd, file)?;' -> '            let _ = elevation_allowed(plan, cmd, file);' fez o teste falhar (exit 101).`
  Na primeira tentativa o mutante sobreviveu. Acrescentei o teste `runas_refuses_params_outside_the_allowlist` e ele passou a falhar.
- Adendo do orquestrador (upgrade segue `installer_mode`):
  `-Path src/winget_install/engine.rs -Anchor '    plan_with_mode(&kept, host, wanted, from, installer_mode)' -Replacement '    plan_with_mode(&kept, host, wanted, from, InstallerMode::default())' -TestCommand 'cargo test --quiet --test it wi_upgrade::upgrade_follows_installer_mode'`
  `[OK] mutação PEGA: '    plan_with_mode(&kept, host, wanted, from, installer_mode)' -> '    plan_with_mode(&kept, host, wanted, from, InstallerMode::default())' fez o teste falhar (exit 101).`
- Check 1 (testes nunca usam `runas` real): só lançador falso (`FakeLauncher`). Os testes `wi_launch` que rodam `.cmd` usam `Verb::Open`. Não há prova de mutação: é regra de revisão.

Decisions:
- `Request::RunInstaller` continua no protocolo com campos tipados e `deny_unknown_fields`, mas a resposta é sempre `INSTALLER_REMOVED` e nada roda. Assim um cliente antigo recebe erro em vez de canal fechado, e o mutante (a) compila. Saíram `WorkerOps::run_installer`, `ElevatedRunner::run_installer`, `staging_win::{run_installer, stage, Staged, copy_hashed, open_source, command_for, run_staged, log_lines}` e os testes deles.
- `runas` passou a ser `LaunchSpec.verb: Verb::{Open, RunAs}`, com `Launcher::run_started` (callback quando o processo existe). Para msi/wix o alvo é o `msiexec` absoluto com `/i "<arquivo conferido>"`, porque `.msi` não tem verbo `runas`.
- Fases: `WaitingElevation` antes do UAC e `RunningInstaller` quando o processo existe, as duas "não pode ser cancelado". O texto mudou para "Aguardando permissão de administrador (UAC) ou o programa elevado; …", porque a mesma fase também serve à remoção pelo auxiliar.
- Plano (CLI/TUI): `ADMIN_PROMPT` = "pede permissão de administrador para este instalador". Fila da TUI: nada novo foi preciso. A fila já roda um job por vez e todo instalador de linha de comando lança sob `msi.lock`. O teste `tui_batch_machine_installers_one_uac_at_a_time` prova isso, e o doc em `jobs.rs` diz isso.
- O handle fica explícito: `Materialized::release()` só é chamado depois de `run_command` voltar.

Concerns:
- O instalador elevado grava o log em `%LOCALAPPDATA%\wingetrs\log\installers\` (pasta do usuário). Um processo do mesmo usuário poderia plantar uma junção ali e fazer o `msiexec` elevado criar um arquivo de log onde quiser. Antes, o log ficava no staging só de admin. Alternativas: não passar log nos instaladores elevados (perde a dica 1304) ou segurar as pastas acima abertas. Fica para o orquestrador decidir.
- Para MSI, a janela do UAC mostra "Windows Installer (Microsoft)", não o editor do MSI. Elevar o `.msi` direto não é possível, e isso não foi testado ao vivo. Exe/Inno/Nullsoft/Burn fazem `runas` no próprio arquivo e o UAC mostra o editor. A prova ao vivo não foi feita.
- Arquivos fora da lista Files: `launch.rs` (`Verb`, `run_started`), `fetch.rs` (`release`), `uninstall.rs` (campo `verb`), `install/progress.rs` (texto da fase), `examples/elevated_arp_probe.rs` (fake), testes `wi_cli`/`wi_orphans`/`wi_uninstall_flow`/`wi_upgrade`/`elevated_uninstall`/`wi_launch` (fakes e `verb`). O adendo mexeu em `engine.rs`, `cli/run_win.rs` e `tui/run.rs`; estes dois últimos foram para o commit só nos meus trechos, via `git apply --cached`.
- Coordenação: reescrevi `machine_msi_goes_through_helper` (a Task 198 tinha mudado uma linha dele, já commitada) e removi os testes de staging em `elevated_install.rs`. O teste `worker_allows_progress_switches_exactly`, da 198, foi mantido.
- Nenhum `.join` novo com dado externo (o log usa o mesmo `installer_log`, já com `safepath`).

## Segunda rodada (decisões do orquestrador; commit 30cbee2)

Gate (antes do commit 30cbee2):
```
[OK]   rules 11s
[OK]   rules-selftest 28s
[OK]   fmt 4s
[OK]   clippy 109s
[OK]   test (1820 passed, 87 ignored) 127s
[OK]   gate verde
```

O que mudou:
1. Instalador elevado sem log. `switches::install_args_unlogged` monta a linha sem o switch Log, seja o do manifesto ou o padrão. `plan::command_for` escolhe essa linha quando a elevação é `Helper`, e o `engine` faz o mesmo ao reconstruir a linha no upgrade com remoção antes. Em `<LOGPATH>` restante (num `Custom`), o `helper_accepts` manda o pacote ao winget, e `ops::elevation_allowed` recusa de novo antes do UAC. No `runas`, `run_command` não cria pasta de log nem chama `fill_log`. O resultado sai só do código de saída: `report.log = None`, sem dica 1304 para o elevado. A execução sem elevação continua com log.
2. No plano, msi/wix passam a dizer "pede permissão de administrador para o Windows Installer" (`ADMIN_PROMPT_MSI`). Os outros tipos continuam com `ADMIN_PROMPT`.
3. Recusa da pasta (vem da Task 197). Nova variante `ElevatedError::NotAdminFolder(texto)`, que exibe só o texto. `State::Refused` guarda o motivo, então os pedidos seguintes da sessão também recebem o texto. `ElevatedError::context()` evita o prefixo "falha ao apagar …" em `orphans`/`uninstall`. No job de ponte de máquina, `exec_for` mostra esse texto no lugar de `REFUSED_NOTE`. O desinstalador elevado já usava `e.to_string()`, que agora é o texto puro.

Testes novos: `wi_install_flow::runas_command_line_has_no_log` (msi, inno, burn; controle sem elevação mantém `/log`), o caso `/log "<LOGPATH>"` em `runas_refuses_params_outside_the_allowlist`, `elevated_install::folder_refusal_is_not_a_refused_uac`, `wi_orphans::folder_refusal_is_its_own_error` (runner falso que devolve a recusa) e `plan_text_is_the_confirmation` (texto do Windows Installer).

Provas de mutação (saída real):
```
[OK] mutação PEGA: 'Elevation::Helper => install_args_unlogged(' -> 'Elevation::Helper => install_args(' fez o teste falhar (exit 101).
[OK] mutação PEGA: '    if cmd.args.contains(switches::LOG_TOKEN) {' -> '    if false && cmd.args.contains(switches::LOG_TOKEN) {' fez o teste falhar (exit 101).
[OK] mutação PEGA: '            ElevatedError::NotAdminFolder(reason)' -> '            ElevatedError::Refused' fez o teste falhar (exit 101).
[OK] mutação PEGA: '.map_err(|e| e.context(format!("falha ao apagar {}", o.key)))?' -> '.map_err(|e| format!("falha ao apagar {}: {e}", o.key))?' fez o teste falhar (exit 101).
```
Testes: `runas_command_line_has_no_log`, `runas_refuses_params_outside_the_allowlist`, `folder_refusal_is_not_a_refused_uac`, `folder_refusal_is_its_own_error`.

Os concerns da primeira rodada sobre o log e o UAC do MSI foram resolvidos pelas decisões acima. Os outros concerns continuam. Mais um arquivo fora da lista: `orphans.rs`. `switches.rs` e `engine.rs` foram tocados por causa do item 1.

## Terceira rodada (revisão 2; commit d01fd69)

Gate: `[OK] rules`, `[OK] rules-selftest`, `[OK] fmt`, `[OK]   clippy 71s`. O passo de teste deu `test result: FAILED. 1730 passed; 1 failed`. A falha é só `script_scan::global_block_in_foreach_object_is_informational`, que vem do trabalho em andamento da Task 164 (`tests/it/script_scan.rs`), não deste commit.

1. Item 1 (DLL ao lado do instalador): ficou como está, a decisão foi levada ao usuário.
2. **Lançamento elevado:**
   - `verified_win::hold_chain` abre cada pasta da raiz de dados até a do arquivo sem seguir reparse point e sem compartilhar exclusão, com a mesma regra de `staging_win::hold_plain_folders`. Um reparse point é recusado.
   - `VerifiedFile::open` abre o próprio arquivo sem seguir reparse point (recusa) e guarda o `GetFinalPathNameByHandleW` do handle mantido.
   - `Materialized::verify` segura a cadeia antes de abrir o arquivo. `release`/`Drop` soltam tudo.
   - No `runas`, o programa (ou o `/i "<...>"` do msiexec) recebe `launch_path()`, que é o caminho final, e `lpDirectory` passa a ser a pasta do `msiexec` absoluto, ou seja, System32 do KnownFolder.
   - Testes: `junction_in_the_installer_chain_is_refused` (junção em `installers` e em `installers\<id>`) e `runas_gets_the_final_path_and_system32` (raiz com `..`; o spec recebe o caminho final e System32).
3. `wi_upgrade::uninstall_first_machine_install_runs_elevated_without_log`: confere `RunAs`, ausência de log e resultado Ok.
4. `wi_tui::tui_batch_refused_uac_still_runs_the_next`.
5. Removida a checagem `one_component(sha)`, que nunca era alcançada.

Provas de mutação (saída real):
```
[OK] mutação PEGA: 'command_for(&fresh.installer, fresh.mode, fresh.elevation, false)' -> 'command_for(&fresh.installer, fresh.mode, super::launch::Elevation::Never, false)' fez o teste falhar (exit 101).
[OK] mutação PEGA: '            file.launch_path(),' -> '            file.path().to_path_buf(),' fez o teste falhar (exit 101).
[OK] mutação PEGA: '            ctx.msiexec.parent().map(Path::to_path_buf),
[OK] mutação PEGA: '        self.folders = super::verified_win::hold_chain(&self.root, &self.path)?;' -> '' fez o teste falhar (exit 101).
```
O primeiro mutante (`Elevation::Never`, que não usava o caminho completo) não compilava. Com `super::launch::Elevation::Never` ele compila e é pego por `wi_upgrade::`.

Sem prova de mutação: a recusa de reparse point no próprio arquivo (`VerifiedFile::open`). O arquivo materializado é um hard link do blob, e montar um symlink de arquivo no teste exige privilégio.

## Quarta rodada (decisão do usuário sobre DLL plantada; commit 856be83)

Gate:
```
[OK]   rules 11s
[OK]   rules-selftest 27s
[OK]   fmt 5s
[OK]   clippy 74s
[OK]   test (1827 passed, 87 ignored) 102s
[OK]   gate verde
```

O que mudou:
- `verified_win::RunCopy` cria `<raiz>/installers/run-<32 hex>/`. A pasta é criada com `create_dir`, então um nome que já existe é recusado e deixado intacto.
- O instalador conferido é copiado sozinho para essa pasta (`create_new`). O hash é conferido de novo pela cópia, com `VerifiedFile::open`: o handle nega escrita, exclusão e renomeação e não segue reparse point.
- A cadeia sem junção (`hold_chain`) vai da raiz até a cópia.
- Ao soltar o `RunCopy`, o que acontece depois que o processo termina, a pasta é apagada. Pastas `run-*` com mais de 24 h são podadas antes da próxima execução elevada (`prune_runs`).
- `ops::run_command` lança, no `runas`, o caminho final da cópia, e o `RunCopy` vive até o fim da função.
- `Materialized::launch_path` saiu, porque ficou sem uso.
- `elevate_win::random_hex` passou a ser `pub(crate)`, para gerar o nome da pasta.
- O plano diz "o instalador roda numa pasta temporária do wingetrs" (`RUN_FOLDER_NOTE`).

Testes novos:
- `runas_runs_from_a_fresh_run_folder` (inno e msi): o caminho aponta para dentro de `run-<hex>`, a pasta só tem o instalador e não existe mais depois.
- `run_folder_that_exists_is_refused`: nome já existente recusado com o conteúdo plantado intacto; cópia segura contra escrita e apagada ao soltar; hash errado não deixa a pasta.
- Os testes anteriores de `runas` foram ajustados para o caminho da cópia.

Provas de mutação (saída real):
```
[OK] mutação PEGA: '            (copy, ctx.msiexec.parent().map(Path::to_path_buf), Some(run))' -> '            let _ = copy;
[OK] mutação PEGA: 'std::fs::create_dir(&dir).map_err' -> 'std::fs::create_dir_all(&dir).map_err' fez o teste falhar (exit 101).
[OK] mutação PEGA: 'std::fs::remove_dir_all(&self.dir)' -> 'std::io::Result::Ok(())' fez o teste falhar (exit 101).
```
O primeiro mutante lança o arquivo baixado em vez da cópia; o resto da linha não aparece porque é uma substituição de duas linhas.
