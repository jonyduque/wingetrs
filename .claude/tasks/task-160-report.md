# Task 160 report

Status: DONE_WITH_CONCERNS

Commits:
```
3b66092 feat(install): real PowerShell script host (Bypass child, GPO and elevation checks)
e6f7e85 feat(install): apply script effects through the phase 2a paths
```
(e6f7e85 é da Task 157: ela rodou `git commit -- src/install/executor.rs` enquanto meus 3 trechos já estavam no arquivo de trabalho compartilhado, e eles entraram no commit dela. Os trechos são meus: o `try_launch` antes de `host.run` em `run_hook`, o `tracing::warn!` quando `try_cancel` é recusado no `Drop` do `AwaitWorker` e o log "instalação cancelada durante um passo em andamento". Confira com `git show e6f7e85 -- src/install/executor.rs | grep -n "try_launch\|passo em andamento\|interrompido"`.)

Gate (rodei 2 vezes: a 1ª falhou no clippy por um `&` sobrando nos meus testes, já corrigido):
```
[OK]   rules 8s
[OK]   rules-selftest 16s
[OK]   fmt 3s
[OK]   clippy 23s
[FAIL] test exit 101
  winget_install::arp_read_win::tests::uninstall_root_picks_the_view_of_each_hive --- FAILED
  thread 'winget_install::arp_read_win::tests::uninstall_root_picks_the_view_of_each_hive' (14628) panicked at src\winget_install\arp_read_win.rs:107:9:
```
A falha é em `src/winget_install/arp_read_win.rs`, arquivo de outro agente que estava sendo editado (o log também mostra o `link.exe` 1104 de builds em paralelo). Os meus módulos, provados um por um:
```
cargo test --quiet --test it script_host     -> test result: ok. 11 passed; 0 failed; 1 ignored
cargo test --quiet --test it install_script  -> test result: ok. 55 passed; 0 failed; 0 ignored
cargo test --quiet --test it cancel          -> test result: ok. 20 passed; 0 failed; 0 ignored
cargo test --quiet --lib install::           -> test result: ok. 16 passed; 0 failed; 0 ignored
```
`cargo clippy --all-targets -- -D warnings` passou sem erro (passo clippy do gate acima, `[OK]`).

Mutation proofs:
1. `pwsh -File scripts/mutate.ps1 -Path src/install/script_host_win.rs -Anchor '"Bypass"' -Replacement '"RemoteSigned"' -TestCommand 'cargo test --quiet --test it script_host_win::args_exact'`
   `[OK] mutação PEGA: '"Bypass"' -> '"RemoteSigned"' fez o teste falhar (exit 101).`
2. `... -Path src/install/script_host_win.rs -Anchor 'const CHILD_ENV_REMOVE: [&str; 1] = ["PSModulePath"];' -Replacement 'const CHILD_ENV_REMOVE: [&str; 1] = ["PSModulePathX"];' -TestCommand 'cargo test --quiet --test it script_host_win::env_scoop_set_psmodulepath_removed'`
   `[OK] mutação PEGA: 'const CHILD_ENV_REMOVE: [&str; 1] = ["PSModulePath"];' -> 'const CHILD_ENV_REMOVE: [&str; 1] = ["PSModulePathX"];' fez o teste falhar (exit 101).`
3. `... -Path src/install/script_policy.rs -Anchor '"allsigned" | "restricted"' -Replacement '"restricted"' -TestCommand 'cargo test --quiet --test it script_host_win::gpo_blocks_cases'`
   `[OK] mutação PEGA: '"allsigned" | "restricted"' -> '"restricted"' fez o teste falhar (exit 101).`
4. Instrumento (`check_rules.ps1`). Plantei três violações: `src/t160_plant.rs` com uma linha só citando `PwshHost::with_program`, `tests/t160plant/x.rs` com uma linha só chamando `PwshHost::locate()` e um `#[test]` sem `ignore` no fim de `tests/it/script_host_live.rs`.
   O gate com as violações plantadas:
   ```
   [FAIL] rules exit 1
       src\t160_plant.rs:1: [PwshHost fora de script_host_win/prod_win] fn f() { let _ = wingetrs::install::script_host_win::PwshHost::with_program(p); }
     (log completo: target/verify.log)
   ```
   O resumo do verify só mostra linhas de `src`. Rodando o `check_rules.ps1` direto, com as mesmas violações, aparecem as três:
   ```
     src\t160_plant.rs:1: [PwshHost fora de script_host_win/prod_win] fn f() { let _ = wingetrs::install::script_host_win::PwshHost::with_program(p); }
     tests\it\script_host_live.rs:184: [teste do PwshHost sem ignore live] #[test]
     tests\t160plant\x.rs:1: [PwshHost real fora do teste ao vivo] fn f() { let _ = PwshHost::locate(); }
   exit=1
   ```
   Depois de restaurar: `rules-restored=0`, e no gate final `[OK]   rules 8s` e `[OK]   rules-selftest 16s`.
5. Extras:
   - `-Anchor 'env.push(("PATH".into(), child_path(&self.program)?));' -Replacement 'let _ = child_path(&self.program)?;'` com o teste `env_path_is_fixed_not_inherited` ⇒ `[OK] mutação PEGA ...` (exit 101).
   - `-Anchor 'if code == 1' -Replacement 'if code == 99'` com o teste `policy_error_mapped` ⇒ `[OK] mutação PEGA ...` (exit 101).
   - Não fiz mutação no `executor.rs`: a Task 157 editava esse arquivo ao mesmo tempo, e o `mutate.ps1` restaura o conteúdo capturado no início, o que apagaria as edições dela. A prova do portão é o passo vermelho do TDD: antes da mudança no executor, `cancel_before_launch_runs_nothing` caiu com "unwrap_err() on an Ok value" e `cancel_after_launch_waits_for_the_hook` caiu com "the launch won: the cancel was refused".

Decisions:
- Onde achar o pwsh (o orquestrador mandou não procurar pelo PATH; o brief dizia PATH): uso `SHGetKnownFolderPath(ProgramFiles)\PowerShell\7\pwsh.exe` se existir, senão `SHGetKnownFolderPath(System)\WindowsPowerShell\v1.0\powershell.exe`. A busca é feita uma vez por `build_ctx`. Se nenhum for achado, o `run` falha com NotFound.
- PATH do filho: `System32;Windows;System32\Wbem;System32\WindowsPowerShell\v1.0;<pasta do programa>`, tudo por KnownFolder. Ele é posto por último no `env`, então ganha de qualquer PATH que venha no `job.env`. O resto do ambiente é herdado.
- Regra do check_rules: o brief proíbe `PwshHost` em `tests/` fora do arquivo ao vivo, mas os próprios testes do brief usam `PwshHost::with_program(fake)`. Segui a nota do orquestrador ("o host real fora de teste ignorado"): em `src`, `PwshHost` só aparece em `script_host_win.rs`/`prod_win.rs`; em `tests`, só `PwshHost::locate` é restrito ao arquivo ao vivo, e lá todo `#[test]`/`#[tokio::test]` precisa vir seguido de `#[ignore = "live:`.
- `with_program` não lê as chaves de GPO (testes sem registro real). `locate` lê `PowerShellCore` para o pwsh 7, ou `Windows\PowerShell` quando a política diz `UseWindowsPowerShellPolicySetting=1`, e `Windows\PowerShell` para o 5.1. A checagem de elevação (`launch_win::is_elevated`) roda sempre, inclusive nos testes.
- Portão do Ctrl+C: depois que um hook começa, o portão fica "lançado" até o próximo plano (mesma regra do msiexec). Um Ctrl+C mais tarde no mesmo plano espera a operação terminar. Um `.join` com dado externo: nenhum escrito por mim.

Concerns:
- Arquivos fora da lista Files: os 3 trechos no `src/install/executor.rs`, exigidos pelas notas da revisão da 156. Eles saíram no commit da 157 (e6f7e85), não no meu.
- O aviso ao usuário na CLI continua sendo `UNCANCELLABLE` = "o Windows Installer não pode ser cancelado; esperando terminar" (`src/cli/install_cmd.rs:41`, pasta da Task 181). Num hook esse texto está errado. Eu só pus um `tracing::warn!` genérico no executor. Falta generalizar o texto na CLI/TUI.
- `build_ctx` agora devolve `Some(PwshHost::locate())`, mas `ops::exec_ctx` continua forçando `scripts: None` até a 161. Por isso nenhum caminho de produção roda scripts ainda.
- Não rodei o teste ao vivo `script_host_live`; ele só compila. Duas suposições dele não foram verificadas: que `$dir` no `post_install` é a pasta da versão, e que o `Set-Content` passa no `script_scan`. Ele usa `FakeEnv` e compara o `HKCU\Environment` real antes/depois, só leitura, em vez de `WINGETRS_TEST_ENV_KEY`, porque `set_var` em teste paralelo não é seguro.
- Não acrescentei casos da regra nova ao `check_rules_selftest.ps1` (fora da lista Files). A prova foi feita à mão, no item 4.

## Revisão (commit 82cd322 fix(install): review of task 160)

Gate (rodado uma vez):
```
[OK]   rules 16s
[OK]   rules-selftest 27s
[OK]   fmt 3s
[OK]   clippy 40s
[OK]   test (1558 passed, 85 ignored) 107s
[OK]   gate verde
```
Provas de mutação:
1. Nova costura de teste `with_program_and_checks(p, gpo, elevated)`. Os testes `gpo_refusal_starts_nothing` e `elevated_refusal_starts_nothing` usam o falso e conferem que `record.txt` não existe.
   - `-Anchor 'if let Some(m) = gpo_blocks(&(self.gpo)(&self.program)) {' -Replacement 'if let Some(m) = None::<String> {'` ⇒ `[OK] mutação PEGA: ... fez o teste falhar (exit 101).` (pânico do teste, sem erro de compilação)
   - `-Anchor 'refuse_if_elevated((self.elevated)()?)' -Replacement 'refuse_if_elevated(false)'` ⇒ `[OK] mutação PEGA: ... fez o teste falhar (exit 101).`
2. O `LaunchGate` agora guarda qual passo foi lançado (`try_launch_for`, `running`). O CLI troca `UNCANCELLABLE` por `uncancellable(step)`, com o texto "um passo já começou e não pode ser interrompido (<passo>); esperando terminar". Passos: "Windows Installer", "script <hook>" e "instalador <file>". O `try_launch()` do winget usa "instalador". O `RunInstaller` do Scoop agora passa pelo mesmo portão do hook.
   - `-Path src/install/executor.rs -Anchor 'try_launch_for(&format!("script {hook}"))' -Replacement 'try_launch_for("x")'` com o teste `cancel_after_launch_waits_for_the_hook` ⇒ `[OK] mutação PEGA: ... (exit 101).`
   - `-Anchor 'if !gate.try_launch_for(&step) {' -Replacement 'if false {'` com o teste `cancel_before_installer_runs_nothing` ⇒ `[OK] mutação PEGA: ... (exit 101).`
3. Comentário do `locate` corrigido: a segurança vem de recusar o processo elevado; a DACL das pastas não é conferida.
4. `check_rules`: recusa apelido (`PwshHost as`, `= PwshHost;`) e `build_ctx(` em `tests/` fora de função com `#[ignore = "live:`. Parâmetro novo `-TestsDir`. O selftest ganhou 10 casos (24 no total).
   - `-Path scripts/check_rules.ps1 -Anchor '-and -not $fnLive' -Replacement '-and $false' -TestCommand 'pwsh -NoProfile -File scripts/check_rules_selftest.ps1'` ⇒ `[OK] mutação PEGA: '-and -not $fnLive' -> '-and $false' fez o teste falhar (exit 1).`
   - Âncora do padrão de apelido trocada por `'NEVERMATCHES'` ⇒ `[OK] mutação PEGA: ... fez o teste falhar (exit 1).`
5. O teste ao vivo usa `RegEnvStore::for_test_key("Software\wingetrs-test\t160-live-<pid>")`, a mesma chave de teste que o `WINGETRS_TEST_ENV_KEY` escolhe, apagada por um guard no `Drop`. Não lê o `HKCU\Environment` real. Ele confere que a instalação terminou bem e que o `Path` da chave de teste ganhou `$dir\bin`. Esse teste não foi rodado.

Filtros: `script_host` 15 ok (1 ignorado), `cli_cancel` 9, `install_file_installer` 19, `install_script` 55, `cancel` 22.
